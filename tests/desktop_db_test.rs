//! Spawns a real database process through `desktop::db`.
//!
//! Skips itself when no database binary is available, so a checkout without one
//! (CI, a fresh clone) still passes rather than failing for the wrong reason.
//!
//! Every instance started here gets its own data directory and an ephemeral
//! port, and is stopped by pid through its own handle — so a database already
//! running on this machine is never touched.

use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::PathBuf;
use std::time::Duration;

use solilang::desktop::db::{self, DbOptions};

/// Find a database binary: an explicit override, then the sibling repo's
/// release build, then `$PATH`.
fn find_solidb() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("SOLI_TEST_SOLIDB") {
        let path = PathBuf::from(explicit);
        return path.exists().then_some(path);
    }

    let sibling = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|p| p.join("db/target/release/solidb"));
    if let Some(path) = sibling {
        if path.exists() {
            return Some(path);
        }
    }

    std::process::Command::new("which")
        .arg("solidb")
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| PathBuf::from(String::from_utf8_lossy(&out.stdout).trim().to_string()))
        .filter(|p| p.exists())
}

fn scratch(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("soli_desktop_db_{}_{}", std::process::id(), name));
    p
}

#[test]
fn starts_a_private_database_and_stops_it() {
    let Some(binary) = find_solidb() else {
        eprintln!("skipping: no solidb binary found (set SOLI_TEST_SOLIDB to override)");
        return;
    };

    let root = scratch("lifecycle");
    let _ = std::fs::remove_dir_all(&root);
    let options = DbOptions::new(binary, root.join("db"), root.join("state"));

    let mut handle = db::start(&options).expect("database should start");

    // An ephemeral port, not the database's default — two apps (or an app and
    // a developer's own instance) must be able to run side by side.
    assert_ne!(handle.port, 0);
    assert_ne!(
        handle.port, 6745,
        "must not take the database's well-known port"
    );
    assert!(handle.is_running());
    assert_eq!(
        handle.host_url(),
        format!("http://127.0.0.1:{}", handle.port)
    );

    // `start` only returns once the port accepts, so this must succeed
    // immediately rather than needing a retry loop.
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, handle.port));
    TcpStream::connect_timeout(&addr, Duration::from_secs(2))
        .expect("start() must not return before the port accepts connections");

    handle.shutdown(Duration::from_secs(10));
    assert!(!handle.is_running(), "shutdown must stop the process");

    // The port must be free again, or the next launch cannot rebind it.
    assert!(
        TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_err(),
        "port {} still accepting after shutdown",
        handle.port
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn credentials_survive_a_restart() {
    let Some(binary) = find_solidb() else {
        eprintln!("skipping: no solidb binary found");
        return;
    };

    let root = scratch("restart");
    let _ = std::fs::remove_dir_all(&root);
    let options = DbOptions::new(binary, root.join("db"), root.join("state"));

    let first = db::start(&options).expect("first start");
    let first_credentials = first.credentials.clone();
    let first_port = first.port;
    drop(first); // Drop stops the process.

    let second = db::start(&options).expect("second start");

    // The admin password is only honored when the admin user does not exist,
    // i.e. on first launch. Regenerating it per launch would lock the app out
    // of its own data on the second run.
    assert_eq!(
        second.credentials, first_credentials,
        "credentials must be stable across launches"
    );
    // A fresh port each launch is expected and fine.
    let _ = first_port;

    drop(second);
    let _ = std::fs::remove_dir_all(&root);
}

/// Helper for manually verifying orphan prevention; not part of the suite.
///
/// Starts a database, prints its pid, and holds it open. Kill *this* process
/// with SIGKILL — bypassing every cleanup path — and the database must die too.
/// On Linux that is `PR_SET_PDEATHSIG`; macOS has no equivalent, so there it is
/// expected to survive and be reaped by the next launch instead.
///
/// Run with: `cargo test --test desktop_db_test orphan_helper -- --ignored --nocapture`
#[test]
#[ignore]
fn orphan_helper_holds_a_database_open() {
    let Some(binary) = find_solidb() else {
        eprintln!("skipping: no solidb binary found");
        return;
    };
    let root = scratch("orphan");
    let _ = std::fs::remove_dir_all(&root);
    let options = DbOptions::new(binary, root.join("db"), root.join("state"));
    let handle = db::start(&options).expect("start");
    println!("DB_PORT={}", handle.port);
    std::thread::sleep(Duration::from_secs(60));
    drop(handle);
}
