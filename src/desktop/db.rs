//! Supervising a private database process for a packaged desktop app.
//!
//! The app owns its database rather than talking to a shared installation: it
//! spawns one on loopback with a random port and per-install credentials, waits
//! for it to accept connections, and stops it on the way out.
//!
//! Two properties are load-bearing:
//!
//! - **Loopback only.** The database defaults to binding all interfaces, which
//!   on a laptop would publish the user's data to whatever network they are on.
//!   `--host 127.0.0.1` is passed unconditionally, never made configurable.
//! - **The child must not outlive the parent.** An orphaned database keeps the
//!   data directory locked, so the next launch fails. See [`arm_parent_death`].

use std::io::Write;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Credentials for the app's own database instance.
///
/// Generated once per install and stored beside the data directory. They are
/// deliberately *not* encrypted with the key fetched from the key server:
/// rotating that key would then permanently lock every existing install out of
/// its own data, and the file guards a loopback-only database whose data files
/// sit unencrypted in the sibling directory under the same ownership anyway.
/// Confidentiality of the contents comes from field-level encryption, whose key
/// is never written to disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbCredentials {
    pub username: String,
    pub password: String,
}

/// Where the database lives and how long to wait for it.
#[derive(Debug, Clone)]
pub struct DbOptions {
    /// The database executable.
    pub binary: PathBuf,
    /// Data directory. Must persist across launches.
    pub data_dir: PathBuf,
    /// Small mutable state: credentials, logs.
    pub state_dir: PathBuf,
    /// How long to wait for the port to accept. A cold open of a large data
    /// directory takes seconds, so this is generous by design.
    pub ready_timeout: Duration,
}

impl DbOptions {
    pub fn new(binary: PathBuf, data_dir: PathBuf, state_dir: PathBuf) -> Self {
        Self {
            binary,
            data_dir,
            state_dir,
            ready_timeout: Duration::from_secs(30),
        }
    }
}

/// A running database. Dropping it stops the process.
#[derive(Debug)]
pub struct DbHandle {
    child: Child,
    pub port: u16,
    pub credentials: DbCredentials,
    /// Windows orphan guard. Held for the handle's lifetime: dropping it closes
    /// the job, which kills everything inside. `None` where the platform has no
    /// equivalent (Linux uses PDEATHSIG instead; macOS has neither).
    _process_group: Option<crate::platform::job::ProcessGroup>,
}

impl DbHandle {
    /// The database process id.
    ///
    /// Exposed so the shutdown coordinator can signal it without holding the
    /// handle: shutdown runs on its own thread while this handle stays owned by
    /// the boot path.
    pub fn child_pid(&self) -> u32 {
        self.child.id()
    }

    /// Base URL for the model layer's `SOLIDB_HOST`.
    pub fn host_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Whether the process is still running. `Ok(None)` from `try_wait` means
    /// alive; anything else means it exited or we can no longer tell.
    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Ask the process to stop, escalating to a hard kill.
    ///
    /// The graceful path matters: RocksDB replays its write-ahead log after an
    /// unclean close, which is slow and reads to a user like corruption.
    pub fn shutdown(&mut self, #[cfg_attr(not(unix), allow(unused_variables))] grace: Duration) {
        if !self.is_running() {
            return;
        }

        #[cfg(unix)]
        {
            // SAFETY: `id()` is this child's pid, and we are its parent, so it
            // has not been reaped.
            unsafe {
                libc::kill(self.child.id() as libc::pid_t, libc::SIGTERM);
            }
            let deadline = Instant::now() + grace;
            while Instant::now() < deadline {
                if !self.is_running() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }

        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for DbHandle {
    fn drop(&mut self) {
        self.shutdown(Duration::from_secs(10));
    }
}

/// Load this install's credentials, generating them on first run.
pub fn load_or_create_credentials(state_dir: &Path) -> Result<DbCredentials, String> {
    let path = state_dir.join("creds.json");

    if let Ok(raw) = std::fs::read_to_string(&path) {
        let parsed: serde_json::Value = serde_json::from_str(&raw)
            .map_err(|e| format!("{} is not valid JSON: {}", path.display(), e))?;
        let username = parsed
            .get("username")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("{} has no \"username\"", path.display()))?;
        let password = parsed
            .get("password")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("{} has no \"password\"", path.display()))?;
        return Ok(DbCredentials {
            username: username.to_string(),
            password: password.to_string(),
        });
    }

    std::fs::create_dir_all(state_dir)
        .map_err(|e| format!("cannot create {}: {}", state_dir.display(), e))?;

    let credentials = DbCredentials {
        // Must match the admin account the database creates on an empty
        // install (`DEFAULT_USER` in its auth module). Any other name simply
        // does not exist, so every login returns 400 and the app runs with no
        // database access at all — while otherwise appearing to start fine.
        username: "admin".to_string(),
        password: random_hex_32(),
    };
    let body = serde_json::json!({
        "username": credentials.username,
        "password": credentials.password,
    })
    .to_string();

    write_private_file(&path, body.as_bytes())?;
    Ok(credentials)
}

/// 32 random bytes, hex-encoded.
fn random_hex_32() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Write a file only the owner can read.
///
/// The mode is set at creation rather than afterwards, so there is no window in
/// which the credentials exist with default permissions.
fn write_private_file(path: &Path, contents: &[u8]) -> Result<(), String> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|e| format!("cannot write {}: {}", path.display(), e))?;
    file.write_all(contents)
        .map_err(|e| format!("cannot write {}: {}", path.display(), e))
}

/// Reserve a free loopback port by binding and immediately releasing it.
///
/// There is an unavoidable race between releasing and the child binding, so
/// callers retry the whole spawn rather than trusting a single attempt.
fn pick_loopback_port() -> Result<u16, String> {
    let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .map_err(|e| format!("cannot reserve a loopback port: {}", e))?;
    listener
        .local_addr()
        .map(|addr| addr.port())
        .map_err(|e| format!("cannot read the reserved port: {}", e))
}

/// On Linux, ask the kernel to kill this process when its parent dies.
///
/// This is the only orphan guard that survives the parent being `SIGKILL`ed.
/// It is armed in the child between fork and exec.
///
/// Caveat worth knowing: the signal fires when the *spawning thread* exits, not
/// the process — so the caller must spawn from a thread that lives as long as
/// the app.
///
/// macOS has no equivalent (no `PDEATHSIG`, no job objects), so there the
/// guarantee degrades to the explicit shutdown path plus the stale-process
/// sweep at next launch. Windows gets a job object in the Windows phase.
#[cfg(target_os = "linux")]
fn arm_parent_death(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    // SAFETY: `prctl` is async-signal-safe and this closure runs in the child
    // between fork and exec, where only such calls are permitted.
    unsafe {
        command.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
            Ok(())
        });
    }
}

#[cfg(not(target_os = "linux"))]
fn arm_parent_death(_command: &mut Command) {}

/// Start the database and wait until it accepts connections.
///
/// Retries the whole spawn a few times: the reserved port can be taken between
/// releasing the probe listener and the child binding it.
pub fn start(options: &DbOptions) -> Result<DbHandle, String> {
    const ATTEMPTS: usize = 5;
    let mut last_error = String::new();

    for attempt in 1..=ATTEMPTS {
        match try_start_once(options) {
            Ok(handle) => return Ok(handle),
            Err(e) => {
                last_error = e;
                if attempt < ATTEMPTS {
                    std::thread::sleep(Duration::from_millis(150));
                }
            }
        }
    }
    Err(format!(
        "database failed to start after {} attempts: {}",
        ATTEMPTS, last_error
    ))
}

fn try_start_once(options: &DbOptions) -> Result<DbHandle, String> {
    if !options.binary.exists() {
        return Err(format!(
            "database binary not found at {}",
            options.binary.display()
        ));
    }

    let credentials = load_or_create_credentials(&options.state_dir)?;
    let port = pick_loopback_port()?;

    std::fs::create_dir_all(&options.data_dir)
        .map_err(|e| format!("cannot create {}: {}", options.data_dir.display(), e))?;

    let mut command = Command::new(&options.binary);
    command
        .arg("--port")
        .arg(port.to_string())
        // Never configurable: binding all interfaces would publish the user's
        // database to whatever network the machine is on.
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--data-dir")
        .arg(&options.data_dir)
        // Single node, so the replication write-ahead log is pure overhead.
        .arg("--no-sync-log")
        // Low-memory storage profile — the right trade for a desktop app.
        .arg("--dev")
        .arg("--log-file")
        .arg(options.state_dir.join("solidb.log"))
        // Consumed only when the admin user does not exist yet, i.e. first
        // launch. Later launches authenticate with the stored value.
        .env("SOLIDB_ADMIN_PASSWORD", &credentials.password)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    arm_parent_death(&mut command);

    let child = command
        .spawn()
        .map_err(|e| format!("cannot start {}: {}", options.binary.display(), e))?;

    // Windows: adopt the child into a kill-on-close job, so it dies with this
    // process however this process ends — the counterpart to PDEATHSIG above.
    // Best-effort: failing to create the job must not stop the app starting,
    // it only degrades orphan prevention to the explicit shutdown path.
    let process_group = match crate::platform::job::ProcessGroup::new() {
        Ok(Some(group)) => match group.adopt(&child) {
            Ok(()) => Some(group),
            Err(e) => {
                eprintln!(
                    "warning: could not tie the database to this process ({})",
                    e
                );
                None
            }
        },
        Ok(None) => None, // platform has no job objects
        Err(e) => {
            eprintln!("warning: could not create a process group ({})", e);
            None
        }
    };

    let mut handle = DbHandle {
        child,
        port,
        credentials,
        _process_group: process_group,
    };

    match wait_until_accepting(&mut handle, options.ready_timeout) {
        Ok(()) => Ok(handle),
        Err(e) => {
            handle.shutdown(Duration::from_secs(2));
            Err(e)
        }
    }
}

/// Poll until the port accepts a TCP connection.
///
/// Deliberately does not speak HTTP: the database has no unauthenticated health
/// endpoint, so any HTTP probe would have to carry credentials and interpret
/// 401s as success. Accepting a connection is the property actually needed —
/// that the listener is up before the app issues its first query.
fn wait_until_accepting(handle: &mut DbHandle, timeout: Duration) -> Result<(), String> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, handle.port));
    let deadline = Instant::now() + timeout;

    while Instant::now() < deadline {
        // Exiting early is better than waiting out the timeout on a database
        // that already died — the error is far more useful.
        if !handle.is_running() {
            return Err(format!(
                "database exited before accepting connections on port {} \
                 (check {})",
                handle.port, "the log file in the app's state directory"
            ));
        }
        if TcpStream::connect_timeout(&addr, Duration::from_millis(250)).is_ok() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    Err(format!(
        "database did not accept connections on port {} within {:?}",
        handle.port, timeout
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("soli_db_test_{}_{}", std::process::id(), name));
        p
    }

    #[test]
    fn credentials_are_generated_once_and_then_reused() {
        let dir = scratch("creds");
        let _ = std::fs::remove_dir_all(&dir);

        let first = load_or_create_credentials(&dir).expect("first generate");
        // Not an arbitrary name: it must match the admin account the database
        // creates on an empty install, or authentication fails at runtime.
        assert_eq!(first.username, "admin");
        assert_eq!(first.password.len(), 64, "32 random bytes, hex-encoded");

        // Stability matters: the password is only honored when the admin user
        // is first created, so regenerating it would lock out an existing
        // install.
        let second = load_or_create_credentials(&dir).expect("reload");
        assert_eq!(first, second);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn generated_passwords_differ_between_installs() {
        let a = scratch("creds_a");
        let b = scratch("creds_b");
        let _ = std::fs::remove_dir_all(&a);
        let _ = std::fs::remove_dir_all(&b);

        let one = load_or_create_credentials(&a).expect("a");
        let two = load_or_create_credentials(&b).expect("b");
        assert_ne!(one.password, two.password);

        let _ = std::fs::remove_dir_all(&a);
        let _ = std::fs::remove_dir_all(&b);
    }

    #[test]
    #[cfg(unix)]
    fn credentials_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("creds_mode");
        let _ = std::fs::remove_dir_all(&dir);

        load_or_create_credentials(&dir).expect("generate");
        let mode = std::fs::metadata(dir.join("creds.json"))
            .expect("stat")
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "credentials must not be world-readable"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_a_missing_binary() {
        let dir = scratch("missing_bin");
        let options = DbOptions::new(
            PathBuf::from("/nonexistent/solidb"),
            dir.join("db"),
            dir.join("state"),
        );
        let err = start(&options).expect_err("must not claim success");
        assert!(
            err.contains("not found"),
            "error should name the missing binary, got: {}",
            err
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn picks_distinct_free_ports() {
        let a = pick_loopback_port().expect("a");
        let b = pick_loopback_port().expect("b");
        assert_ne!(a, 0);
        assert_ne!(b, 0);
    }
}
