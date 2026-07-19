pub mod desktop;
mod progress;
mod test_runner;

use std::env;
#[cfg(unix)]
use std::fs::OpenOptions;
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;
use std::process;

use crate::cli::args::{
    print_usage, DbMigrateAction, DbSeedAction, EngineAction, Options, VERSION,
};

#[cfg(unix)]
use daemonize::Daemonize;
#[cfg(unix)]
use nix::sys::signal::{kill, Signal};
#[cfg(unix)]
use nix::unistd::Pid;

pub fn run_build(
    folder: &str,
    output: Option<&str>,
    standalone: bool,
    encrypt: bool,
    protect: bool,
    target: Option<&str>,
) {
    // Resolve "." to current directory so file_name() works properly
    let source_dir = if folder == "." {
        std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf())
    } else {
        Path::new(folder).to_path_buf()
    };

    if !source_dir.exists() {
        eprintln!("Error: Folder '{}' does not exist", folder);
        process::exit(1);
    }

    if !source_dir.is_dir() {
        eprintln!("Error: '{}' is not a directory", folder);
        process::exit(1);
    }

    // Catch a --target typo before doing any build work.
    if let Some(t) = target {
        if let Err(e) = crate::cli::standalone::validate_target(t) {
            eprintln!("Error: {}", e);
            process::exit(1);
        }
    }

    // The key comes from the same resolution chain as serve (SOLI_BUNDLE_KEY
    // or the key server), and the vars may live in the app's .env — load it
    // so `soli build --encrypt` works from a plain shell.
    if encrypt {
        solilang::serve::env_loader::load_env_files(&source_dir);
    }

    println!("Building bundle from {}...", source_dir.display());

    let bundle_data = if protect {
        solilang::bundle::BundleBuilder::build_protected(&source_dir)
    } else {
        solilang::bundle::BundleBuilder::build(&source_dir)
    };
    let bundle_data = match bundle_data {
        Ok(data) => data,
        Err(e) => {
            eprintln!("Error building bundle: {}", e);
            process::exit(1);
        }
    };

    let bundle_data = if encrypt {
        let (key, source) = resolve_bundle_key().unwrap_or_else(|e| {
            eprintln!("Error: {}", e);
            process::exit(1);
        });
        println!("  Encrypting bundle ({})", source);
        solilang::bundle::encrypt_bundle(&bundle_data, &key).unwrap_or_else(|e| {
            eprintln!("Error encrypting bundle: {}", e);
            process::exit(1);
        })
    } else {
        bundle_data
    };

    let app_name = source_dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "app".to_string());
    let output_path = match output {
        Some(path) => Path::new(path).to_path_buf(),
        None if standalone => {
            // `<name>` for host builds, `<name>-<target>` for cross builds.
            let name = match target {
                Some(t) => format!(
                    "{}-{}",
                    app_name,
                    crate::cli::standalone::target_label(Some(t))
                ),
                None => app_name.clone(),
            };
            Path::new(&name).to_path_buf()
        }
        None => Path::new(&format!("{}.soli", app_name)).to_path_buf(),
    };

    // The extensionless standalone default collides with the source dir when
    // building from the parent directory (`soli build my_app --standalone`).
    if standalone && output_path.is_dir() {
        eprintln!(
            "Error: output path '{}' is a directory — pass --output <file>",
            output_path.display()
        );
        process::exit(1);
    }

    let mode = match (protect, encrypt) {
        (true, _) => "protected: binary AST, encrypted",
        (false, true) => "encrypted",
        _ => "",
    };

    if standalone {
        // A Windows artifact needs `.exe` even when cross-built from Linux.
        let output_path = crate::cli::standalone::apply_exe_suffix(&output_path, target);
        if let Err(e) =
            crate::cli::standalone::write_standalone_exe(&bundle_data, &output_path, target)
        {
            eprintln!("Error building standalone executable: {}", e);
            process::exit(1);
        }
        let total = std::fs::metadata(&output_path)
            .map(|m| m.len())
            .unwrap_or(0);
        let mode_suffix = if mode.is_empty() {
            String::new()
        } else {
            format!(", {}", mode)
        };
        println!(
            "  \x1b[32m\x1b[1m✓\x1b[0m Standalone executable written to {} ({:.1} MB, {:.1} KB app bundle{})",
            output_path.display(),
            total as f64 / (1024.0 * 1024.0),
            bundle_data.len() as f64 / 1024.0,
            mode_suffix
        );
        let run_name = output_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| output_path.display().to_string());
        println!(
            "    Platform: {} — run it with: ./{} --port 8080",
            crate::cli::standalone::target_label(target),
            run_name
        );
        return;
    }

    match std::fs::write(&output_path, &bundle_data) {
        Ok(_) => {
            let size_kb = bundle_data.len() as f64 / 1024.0;
            let mode_suffix = if mode.is_empty() {
                String::new()
            } else {
                format!(" ({})", mode)
            };
            println!(
                "  \x1b[32m\x1b[1m✓\x1b[0m Bundle written to {} ({:.1} KB){}",
                output_path.display(),
                size_kb,
                mode_suffix
            );
        }
        Err(e) => {
            eprintln!("Error writing bundle: {}", e);
            process::exit(1);
        }
    }
}

/// Resolve the bundle encryption/decryption key. Order: `SOLI_BUNDLE_KEY`
/// (the key itself), then `SOLI_BUNDLE_AUTH_URL` (a key server queried with
/// an optional `SOLI_BUNDLE_API_KEY` sent as `x-api-key`). Returns the key
/// material plus a human label of where it came from.
fn resolve_bundle_key() -> Result<(String, String), String> {
    if let Ok(key) = std::env::var("SOLI_BUNDLE_KEY") {
        if !key.trim().is_empty() {
            return Ok((
                key.trim().to_string(),
                "key from SOLI_BUNDLE_KEY".to_string(),
            ));
        }
    }
    if let Ok(url) = std::env::var("SOLI_BUNDLE_AUTH_URL") {
        if !url.trim().is_empty() {
            let api_key = std::env::var("SOLI_BUNDLE_API_KEY")
                .ok()
                .filter(|k| !k.trim().is_empty());
            let key = fetch_bundle_key(url.trim(), api_key.as_deref())?;
            return Ok((key, format!("key fetched from {}", url.trim())));
        }
    }
    Err("no bundle key configured. Provide it via:\n  \
         SOLI_BUNDLE_KEY        the key material itself, or\n  \
         SOLI_BUNDLE_AUTH_URL   URL returning the key (optional SOLI_BUNDLE_API_KEY sent as x-api-key)\n\
         Set them in the environment or in a .env file next to the .soli bundle."
        .to_string())
}

/// GET the key material from the key server. The response body (≤ 4 KB,
/// trimmed) is the key. Runs at single-threaded boot — blocking client, no
/// tokio runtime involved.
fn fetch_bundle_key(url: &str, api_key: Option<&str>) -> Result<String, String> {
    use std::io::Read;

    let client = reqwest::blocking::Client::builder()
        .user_agent("soli-lang-cli")
        .min_tls_version(reqwest::tls::Version::TLS_1_2)
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let mut request = client.get(url);
    if let Some(key) = api_key {
        request = request.header("x-api-key", key);
    }

    let response = request
        .send()
        .map_err(|e| format!("key server request failed ({}): {}", url, e))?;

    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(format!(
            "key server rejected the API key (HTTP {}) — was the key revoked?",
            status.as_u16()
        ));
    }
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err("key not found on key server (HTTP 404) — was it revoked?".to_string());
    }
    if !status.is_success() {
        return Err(format!("key server returned HTTP {}", status.as_u16()));
    }

    let mut body = String::new();
    response
        .take(4096)
        .read_to_string(&mut body)
        .map_err(|e| format!("failed to read key server response: {}", e))?;
    let key = body.trim().to_string();
    if key.is_empty() {
        return Err("key server returned an empty body".to_string());
    }
    Ok(key)
}

pub fn run_serve(folder: &str, port: u16, dev_mode: bool, workers: usize, daemonize: bool) {
    let path = Path::new(folder);

    if !path.exists() {
        eprintln!("Error: Folder '{}' does not exist", folder);
        process::exit(1);
    }

    if !path.is_dir() {
        // Check if it's a .soli bundle file
        if folder.ends_with(".soli") && path.is_file() {
            // It's a bundle file - serve from the bundle
            if let Err(e) = serve_from_bundle(folder, port, dev_mode, workers) {
                eprintln!("Error: {}", e);
                process::exit(70);
            }
            return;
        }
        eprintln!("Error: '{}' is not a directory", folder);
        process::exit(1);
    }

    if let Err(msg) = solilang::module::enforce_min_soli_version(path) {
        eprintln!("{}", msg);
        process::exit(1);
    }

    #[cfg(unix)]
    if daemonize {
        let pid_file = path.join("soli.pid");
        let log_file = path.join("soli.log");
        kill_previous_process(&pid_file);

        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_file)
            .unwrap_or_else(|e| {
                eprintln!("Error: Cannot create log file: {}", e);
                process::exit(1);
            });

        let daemon = Daemonize::new()
            .pid_file(&pid_file)
            .chown_pid_file(true)
            .working_directory(path)
            .stdout(log.try_clone().unwrap())
            .stderr(log);

        println!("Starting soli daemon...");
        println!("  PID file: {}", pid_file.display());
        println!("  Log file: {}", log_file.display());

        match daemon.start() {
            Ok(_) => println!("Daemon started successfully"),
            Err(e) => {
                eprintln!("Error: Failed to daemonize: {}", e);
                process::exit(1);
            }
        }
    }

    #[cfg(not(unix))]
    if daemonize {
        eprintln!("Error: Daemonization is only supported on Unix systems");
        process::exit(1);
    }

    if let Err(e) = solilang::serve::serve_folder_with_options(path, port, dev_mode, workers) {
        eprintln!("Error: {}", e);
        process::exit(70);
    }
}

fn serve_from_bundle(
    bundle_path: &str,
    port: u16,
    dev_mode: bool,
    workers: usize,
) -> Result<(), String> {
    let bundle_data = std::fs::read(bundle_path)
        .map_err(|e| format!("Failed to read bundle '{}': {}", bundle_path, e))?;

    // The `.env` convention: config lives in the directory containing the
    // bundle file (dotfiles are deliberately excluded from bundles).
    let bundle_dir = Path::new(bundle_path)
        .canonicalize()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    serve_bundle_bytes(
        &bundle_data,
        &bundle_dir,
        port,
        dev_mode,
        workers,
        bundle_path,
    )
}

/// Serve a bundle from its raw bytes — the shared tail of `soli serve
/// app.soli` and standalone executables (`cli::standalone`), so both inherit
/// the same key resolution, /dev/shm extraction contract, and atexit cleanup.
/// `origin` is the human label used in error messages (a bundle path or the
/// standalone executable's name).
pub(crate) fn serve_bundle_bytes(
    bundle_data: &[u8],
    env_dir: &Path,
    port: u16,
    dev_mode: bool,
    workers: usize,
    origin: &str,
) -> Result<(), String> {
    // Load `.env` (and `.env.{APP_ENV}`) BEFORE anything else: the bundle
    // key config (SOLI_BUNDLE_KEY / SOLI_BUNDLE_AUTH_URL) may live there.
    // Secrets don't belong in a distributable artifact, so the operator
    // drops a `.env` next to it — the extracted temp dir that
    // `serve_folder` later scans never has one.
    solilang::serve::env_loader::load_env_files(env_dir);

    // A desktop artifact carries its own database and an encrypted app inside
    // a plain outer bundle, so it needs a different boot sequence: start the
    // database first, point the model layer at it, then serve the inner app.
    if solilang::desktop::container::is_desktop_payload(bundle_data) {
        return desktop::boot(bundle_data, port, dev_mode, workers, origin);
    }

    let encrypted = solilang::bundle::is_encrypted_bundle(bundle_data);
    let bundle_data = if encrypted {
        let (key, source) =
            resolve_bundle_key().map_err(|e| format!("'{}' is encrypted — {}", origin, e))?;
        println!("Decrypting bundle ({})", source);
        solilang::bundle::decrypt_bundle(bundle_data, &key)?
    } else {
        bundle_data.to_vec()
    };

    let bundle = solilang::bundle::BundleReader::new(&bundle_data)?;
    solilang::bundle::check_bundle_meta(bundle.entries())?;

    // Extraction dir. Decrypted app trees go to RAM-backed tmpfs (/dev/shm)
    // with 0700 perms so plaintext never lands on persistent disk; plain
    // bundles keep the historical temp-dir behavior.
    let tmp_dir = if encrypted {
        encrypted_extraction_dir()?
    } else {
        let dir = std::env::temp_dir().join(format!("soli_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create temp dir: {}", e))?;
        dir
    };

    if encrypted {
        // The decrypted tree must not outlive the server.
        solilang::cleanup::register_cleanup_dir(&tmp_dir);
    }

    for (path, content) in bundle.entries() {
        let full_path = tmp_dir.join(path);
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create dir '{}': {}", parent.display(), e))?;
        }
        std::fs::write(&full_path, content)
            .map_err(|e| format!("Failed to write '{}': {}", full_path.display(), e))?;
    }

    if encrypted {
        println!(
            "Extracted bundle to {} (RAM-backed, removed on shutdown)",
            tmp_dir.display()
        );
    } else {
        println!("Extracted bundle to {}", tmp_dir.display());
    }

    // Serve from the extracted temp directory
    let folder_path = tmp_dir.to_string_lossy().to_string();
    // Create a regular DiskFS pointing at the temp dir
    let vfs = solilang::virtual_fs::DiskFS::new(&folder_path);
    solilang::serve::init_global_vfs(vfs);

    solilang::serve::serve_folder_with_options(&tmp_dir, port, dev_mode, workers)
        .map_err(|e| e.to_string())
}

/// Pick the extraction directory for a DECRYPTED bundle: `/dev/shm/soli_<pid>`
/// (RAM-backed tmpfs) with mode 0700. Without /dev/shm the boot is refused —
/// silently writing decrypted source to persistent disk would defeat the
/// encryption without the operator knowing. `SOLI_BUNDLE_ALLOW_DISK=1` is the
/// explicit opt-out (temp dir, still 0700, loud warning).
fn encrypted_extraction_dir() -> Result<std::path::PathBuf, String> {
    let shm = Path::new("/dev/shm");
    let base = if shm.is_dir() {
        shm.to_path_buf()
    } else if std::env::var("SOLI_BUNDLE_ALLOW_DISK").ok().as_deref() == Some("1") {
        eprintln!(
            "\x1b[33mWarning:\x1b[0m /dev/shm is not available — extracting the DECRYPTED \
             bundle to the temp dir on persistent disk (SOLI_BUNDLE_ALLOW_DISK=1)."
        );
        std::env::temp_dir()
    } else {
        return Err(
            "/dev/shm is not available on this system, so the decrypted bundle would be \
             written to persistent disk. Refusing to start. Set SOLI_BUNDLE_ALLOW_DISK=1 \
             to allow extraction to the temp dir anyway."
                .to_string(),
        );
    };

    sweep_stale_extractions(&base);

    let dir = base.join(format!("soli_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    create_private_dir(&dir)?;
    Ok(dir)
}

/// Create a directory with mode 0700 (owner-only) on unix.
fn create_private_dir(dir: &Path) -> Result<(), String> {
    // Delegates to the platform layer, which applies owner-only permissions at
    // creation on both Unix (mode 0700) and Windows (a protected DACL). The
    // non-unix branch here used to be a bare `create_dir_all` with no hardening
    // at all — decrypted application source in a world-readable directory.
    solilang::platform::dirs::create_private_dir(dir)
}

/// Best-effort removal of `soli_<pid>` extraction dirs left behind by dead
/// processes (`kill -9`, crash). Never touches a dir whose pid is alive.
fn sweep_stale_extractions(base: &Path) {
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    let own_pid = std::process::id();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid_str) = name
            .to_string_lossy()
            .strip_prefix("soli_")
            .map(String::from)
        else {
            continue;
        };
        let Ok(pid) = pid_str.parse::<u32>() else {
            continue;
        };
        if pid == own_pid {
            continue;
        }
        // Only sweep a directory whose owning process is gone. This used to
        // call `nix::kill` directly with no cfg gate, which made the whole
        // module fail to compile off Unix.
        if !solilang::platform::process::is_alive(pid) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

pub fn run_new(name: &str, template: Option<&str>) {
    use solilang::scaffold::app_generator::print_success_message;

    match solilang::scaffold::create_app(name, template) {
        Ok(()) => print_success_message(name),
        Err(e) => {
            eprintln!("Error: {}", e);
            process::exit(1);
        }
    }
}

pub fn run_generate(scaffold_name: &str, fields: &[String], folder: &str) {
    match solilang::scaffold::create_scaffold_with_fields(folder, scaffold_name, fields) {
        Ok(()) => solilang::scaffold::print_scaffold_success_message(scaffold_name),
        Err(e) => {
            eprintln!("Error: {}", e);
            process::exit(1);
        }
    }
}

pub fn run_generate_auth(folder: &str) {
    match solilang::scaffold::create_auth(folder) {
        Ok(()) => solilang::scaffold::print_auth_success_message(),
        Err(e) => {
            eprintln!("Error: {}", e);
            process::exit(1);
        }
    }
}

pub fn run_generate_mailer(name: &str, actions: &[String], folder: &str) {
    match solilang::scaffold::create_mailer(folder, name, actions) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("Error: {}", e);
            process::exit(1);
        }
    }
}

pub fn run_generate_component(name: &str, folder: &str) {
    match solilang::scaffold::create_component(folder, name) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("Error: {}", e);
            process::exit(1);
        }
    }
}

pub fn run_engine(action: &EngineAction) {
    match action {
        EngineAction::Create { name } => match solilang::scaffold::create_engine(name) {
            Ok(()) => {}
            Err(e) => {
                eprintln!("Error: {}", e);
                process::exit(1);
            }
        },
        EngineAction::DbMigrate { engine_name } => {
            let app_path = std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());

            if let Err(e) = solilang::serve::engine_loader::run_engine_migrations(
                &app_path,
                engine_name.as_deref(),
            ) {
                eprintln!("Error: {}", e);
                process::exit(1);
            }
        }
        EngineAction::DbRollback { engine_name } => {
            let app_path = std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());

            if let Err(e) = solilang::serve::engine_loader::run_engine_rollback(
                &app_path,
                engine_name.as_deref(),
            ) {
                eprintln!("Error: {}", e);
                process::exit(1);
            }
        }
    }
}

#[cfg(unix)]
fn kill_previous_process(pid_file: &Path) {
    if !pid_file.exists() {
        return;
    }

    let mut file = match File::open(pid_file) {
        Ok(f) => f,
        Err(_) => return,
    };

    let mut pid_str = String::new();
    if file.read_to_string(&mut pid_str).is_err() {
        return;
    }

    let pid: i32 = match pid_str.trim().parse() {
        Ok(p) => p,
        Err(_) => {
            let _ = fs::remove_file(pid_file);
            return;
        }
    };

    let cmdline_path = format!("/proc/{}/cmdline", pid);
    if let Ok(mut cmdline_file) = File::open(&cmdline_path) {
        let mut cmdline = String::new();
        if cmdline_file.read_to_string(&mut cmdline).is_ok() {
            let is_soli = cmdline.split('\0').any(|arg| {
                if arg.is_empty() {
                    return false;
                }
                if arg == "soli" {
                    return true;
                }
                Path::new(arg)
                    .file_name()
                    .map(|name| name == "soli")
                    .unwrap_or(false)
            });

            if is_soli {
                println!("Killing previous soli process (PID: {})", pid);
                if let Err(e) = kill(Pid::from_raw(pid), Signal::SIGTERM) {
                    eprintln!("Warning: Failed to kill process {}: {}", pid, e);
                } else {
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    if Path::new(&cmdline_path).exists() {
                        println!("Process still running, sending SIGKILL...");
                        let _ = kill(Pid::from_raw(pid), Signal::SIGKILL);
                        std::thread::sleep(std::time::Duration::from_millis(200));
                    }
                }
            }
        }
    }

    let _ = fs::remove_file(pid_file);
}

pub fn run_file(path: &str, options: &Options) {
    let path = Path::new(path);

    // Enforce a `soli_version` floor if this script lives inside a project.
    // A standalone script with no enclosing soli.toml is unaffected.
    let start_dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if let Err(msg) = solilang::module::enforce_min_soli_version(start_dir) {
        eprintln!("{}", msg);
        process::exit(1);
    }

    let result = if options.use_vm {
        solilang::run_file_vm(path, !options.no_type_check)
    } else {
        solilang::run_file(path, !options.no_type_check)
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        process::exit(70);
    }
}

pub fn run_eval(code: &str, options: &Options) {
    let result = solilang::run_with_type_check(code, !options.no_type_check);

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        process::exit(70);
    }
}

pub fn run_repl() {
    solilang::repl_tui::run_tui_repl().unwrap();
}

pub fn run_lsp() {
    // Logs go to stderr so they don't corrupt the LSP stdio framing.
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .target(env_logger::Target::Stderr)
        .try_init();
    solilang::lsp::start_lsp();
}

pub fn run_lint(paths: &[String]) {
    let targets: Vec<std::path::PathBuf> = if paths.is_empty() {
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        vec![cwd]
    } else {
        paths.iter().map(std::path::PathBuf::from).collect()
    };

    let mut files: Vec<std::path::PathBuf> = Vec::new();
    for t in &targets {
        if !t.exists() {
            eprintln!("Error: Path '{}' does not exist", t.display());
            process::exit(1);
        }
        if t.is_file() {
            files.push(t.clone());
        } else {
            files.extend(test_runner::collect_lint_files(t));
        }
    }

    if files.is_empty() {
        println!("No .sl or .slv files found.");
        return;
    }

    let mut total_issues = 0;
    let mut files_with_issues = 0;

    for file in &files {
        let source = match fs::read_to_string(file) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}: error reading file: {}", file.display(), e);
                continue;
            }
        };

        let diagnostics = match solilang::lint_file(&source, &file.display().to_string()) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("{}: parse error: {}", file.display(), e);
                continue;
            }
        };

        if !diagnostics.is_empty() {
            files_with_issues += 1;
            for d in &diagnostics {
                println!(
                    "{}:{}:{} - [{}] {}",
                    file.display(),
                    d.span.line,
                    d.span.column,
                    d.rule,
                    d.message
                );
            }
            total_issues += diagnostics.len();
        }
    }

    if total_issues > 0 {
        println!();
        println!(
            "{} issue(s) found in {} file(s)",
            total_issues, files_with_issues
        );
        process::exit(1);
    }

    println!("No issues found.");
}

pub fn run_check(paths: &[String]) {
    let targets: Vec<std::path::PathBuf> = if paths.is_empty() {
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        vec![cwd]
    } else {
        paths.iter().map(std::path::PathBuf::from).collect()
    };

    let mut files: Vec<std::path::PathBuf> = Vec::new();
    for t in &targets {
        if !t.exists() {
            eprintln!("Error: Path '{}' does not exist", t.display());
            process::exit(1);
        }
        if t.is_file() {
            files.push(t.clone());
        } else {
            // Type-check Soli programs only (skip .slv templates).
            files.extend(
                test_runner::collect_lint_files(t)
                    .into_iter()
                    .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("sl")),
            );
        }
    }

    if files.is_empty() {
        println!("No .sl files found.");
        return;
    }

    let checked = files.len();
    let mut total_errors = 0;
    let mut total_warnings = 0;
    let mut files_with_errors = 0;

    for file in &files {
        let source = match fs::read_to_string(file) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}: error reading file: {}", file.display(), e);
                continue;
            }
        };
        match solilang::type_check_source(&source, Some(file.as_path())) {
            Ok(warnings) => {
                total_warnings += warnings.len();
                for warning in &warnings {
                    println!("{}: {}", file.display(), warning);
                }
            }
            Err(errors) => {
                files_with_errors += 1;
                total_errors += errors.len();
                for err in &errors {
                    println!("{}: {}", file.display(), err);
                }
            }
        }
    }

    if total_errors > 0 {
        println!();
        println!(
            "{} error(s){} in {} of {} file(s)",
            total_errors,
            if total_warnings > 0 {
                format!(", {} warning(s)", total_warnings)
            } else {
                String::new()
            },
            files_with_errors,
            checked
        );
        process::exit(1);
    }

    if total_warnings > 0 {
        println!();
        println!(
            "No type errors, {} warning(s). Checked {} file(s).",
            total_warnings, checked
        );
        return;
    }

    println!("No type errors. Checked {} file(s).", checked);
}

pub fn run_fmt(paths: &[String], check: bool, stdin: bool) {
    use std::io::Read;
    if stdin {
        let mut source = String::new();
        if let Err(e) = std::io::stdin().read_to_string(&mut source) {
            eprintln!("Error reading stdin: {}", e);
            process::exit(1);
        }
        match solilang::fmt::format_source(&source) {
            Ok(formatted) => print!("{}", formatted),
            Err(e) => {
                eprintln!("fmt: {}", e);
                process::exit(1);
            }
        }
        return;
    }

    let targets: Vec<std::path::PathBuf> = if paths.is_empty() {
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        vec![cwd]
    } else {
        paths.iter().map(std::path::PathBuf::from).collect()
    };

    let mut files: Vec<std::path::PathBuf> = Vec::new();
    for t in &targets {
        if !t.exists() {
            eprintln!("Error: Path '{}' does not exist", t.display());
            process::exit(1);
        }
        if t.is_file() {
            files.push(t.clone());
        } else {
            files.extend(test_runner::collect_test_files(t));
        }
    }

    if files.is_empty() {
        println!("No .sl files found.");
        return;
    }

    let mut changed = 0usize;
    let mut errors = 0usize;
    for file in &files {
        let source = match fs::read_to_string(file) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}: error reading file: {}", file.display(), e);
                errors += 1;
                continue;
            }
        };
        let formatted = match solilang::fmt::format_source(&source) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}: {}", file.display(), e);
                errors += 1;
                continue;
            }
        };
        if formatted == source {
            continue;
        }
        changed += 1;
        if check {
            println!("would reformat: {}", file.display());
            print_unified_diff(&source, &formatted);
        } else if let Err(e) = fs::write(file, &formatted) {
            eprintln!("{}: error writing: {}", file.display(), e);
            errors += 1;
        } else {
            println!("formatted: {}", file.display());
        }
    }

    if changed == 0 && errors == 0 {
        println!("All files are already formatted.");
    }
    if errors > 0 {
        process::exit(1);
    }
    if check && changed > 0 {
        process::exit(1);
    }
}

/// Print a minimal unified-style diff so `soli fmt --check` shows what
/// would change (and on which lines), not just which files would change.
/// Groups runs of consecutive differences into hunks with line numbers.
fn print_unified_diff(a: &str, b: &str) {
    let a_lines: Vec<&str> = a.lines().collect();
    let b_lines: Vec<&str> = b.lines().collect();
    let (mut i, mut j) = (0usize, 0usize);
    while i < a_lines.len() || j < b_lines.len() {
        // Skip matching prefix.
        while i < a_lines.len() && j < b_lines.len() && a_lines[i] == b_lines[j] {
            i += 1;
            j += 1;
        }
        if i >= a_lines.len() && j >= b_lines.len() {
            break;
        }
        // Find the next sync point: the shortest pair (di, dj) such that
        // a[i+di..] starts with b[j+dj..] (or vice versa). Bounded search.
        let mut di = 0usize;
        let mut dj = 0usize;
        let max_look = 200usize;
        'outer: for d in 1..=max_look {
            for k in 0..=d {
                let ai = k;
                let bj = d - k;
                if i + ai <= a_lines.len() && j + bj <= b_lines.len() {
                    let a_after = a_lines.get(i + ai);
                    let b_after = b_lines.get(j + bj);
                    if a_after == b_after && a_after.is_some() {
                        di = ai;
                        dj = bj;
                        break 'outer;
                    }
                }
            }
        }
        if di == 0 && dj == 0 {
            // No sync within bounds — flush the rest.
            di = a_lines.len() - i;
            dj = b_lines.len() - j;
        }
        // Print the hunk header (1-indexed line numbers).
        println!("  @@ -{},{} +{},{} @@", i + 1, di, j + 1, dj);
        for k in 0..di {
            println!("  - {}", a_lines[i + k]);
        }
        for k in 0..dj {
            println!("  + {}", b_lines[j + k]);
        }
        i += di;
        j += dj;
    }
}

/// Deploy to a remote server over SSH.
///
/// Unix-only, because it is built on ssh2 (see Cargo.toml). The non-unix arm
/// reports that rather than failing the build, so the rest of the CLI stays
/// available on platforms that cannot deploy.
#[cfg(not(unix))]
pub fn run_deploy(_folder: Option<&str>) {
    eprintln!("`soli deploy` is only available on Unix systems.");
    std::process::exit(1);
}

#[cfg(unix)]
pub fn run_deploy(folder: Option<&str>) {
    let path = if let Some(f) = folder {
        Path::new(f).to_path_buf()
    } else {
        std::env::current_dir().expect("Failed to get current directory")
    };

    println!("Deploying from {}...", path.display());

    let config = match solilang::module::deploy::load_deploy_config(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    };

    if config.servers.is_empty() {
        eprintln!("Error: No servers configured in deploy.toml");
        std::process::exit(1);
    }

    println!(
        "Deploying to {} server(s) in parallel...",
        config.servers.len()
    );
    println!();

    let rt = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
    let results = match rt.block_on(solilang::module::deploy::deploy(config)) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    };

    solilang::module::deploy::print_summary(&results);

    let all_success = results.iter().all(|r| r.success);
    if !all_success {
        std::process::exit(1);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run_test(
    paths: &[String],
    jobs: Option<usize>,
    coverage_formats: &[String],
    coverage_min: Option<f64>,
    no_coverage: bool,
    show_uncovered: bool,
    fail_on_n1: bool,
) {
    test_runner::run_test(
        paths,
        jobs,
        coverage_formats,
        coverage_min,
        no_coverage,
        show_uncovered,
        fail_on_n1,
    );
}

pub fn run_init() {
    use solilang::module::Package;

    let toml_path = Path::new("soli.toml");
    if toml_path.exists() {
        eprintln!("soli.toml already exists in this directory");
        process::exit(1);
    }

    let name = env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| "my-package".to_string());

    let pkg = Package::new(&name);
    let content = pkg.to_toml();

    fs::write(toml_path, content).unwrap_or_else(|e| {
        eprintln!("Error: Failed to write soli.toml: {}", e);
        process::exit(1);
    });

    println!();
    println!("  \x1b[32m\x1b[1m✓\x1b[0m Created soli.toml");
    println!();
}

pub fn run_add(
    name: &str,
    git: &Option<String>,
    path: &Option<String>,
    tag: &Option<String>,
    branch: &Option<String>,
    rev: &Option<String>,
    version: &Option<String>,
) {
    use solilang::module::{installer, lockfile::LockFile, Dependency, Package};

    let toml_path = match Package::find(Path::new(".")) {
        Some(p) => p,
        None => {
            eprintln!("No soli.toml found. Run 'soli init' first.");
            process::exit(1);
        }
    };

    let mut pkg = Package::load(&toml_path).unwrap_or_else(|e| {
        eprintln!("Error: Failed to load soli.toml: {}", e);
        process::exit(1);
    });

    let dep = if let Some(git_url) = git {
        Dependency::Git {
            url: git_url.clone(),
            tag: tag.clone(),
            branch: branch.clone(),
            rev: rev.clone(),
        }
    } else if let Some(dep_path) = path {
        Dependency::Path(dep_path.clone())
    } else if let Some(ver) = version {
        Dependency::Version(ver.clone())
    } else {
        eprintln!("Error: Must specify --git, --path, or --version");
        print_usage();
        process::exit(64);
    };

    installer::add_dependency(&mut pkg, name, dep.clone());

    fs::write(&toml_path, pkg.to_toml()).unwrap_or_else(|e| {
        eprintln!("Error: Failed to write soli.toml: {}", e);
        process::exit(1);
    });

    println!();
    println!("  \x1b[32m\x1b[1m✓\x1b[0m Added dependency '{}'", name);

    if matches!(dep, Dependency::Git { .. } | Dependency::Version(_)) {
        let lock_path = toml_path.with_file_name("soli.lock");
        let mut lock = LockFile::load(&lock_path).unwrap_or_default();

        println!();
        println!("  Installing...");
        if let Err(e) = installer::install_all(&pkg, &mut lock, &lock_path) {
            eprintln!("  \x1b[31mError:\x1b[0m {}", e);
            process::exit(1);
        }
    }

    println!();
}

pub fn run_remove(name: &str) {
    use solilang::module::{installer, lockfile::LockFile, Package};

    let toml_path = match Package::find(Path::new(".")) {
        Some(p) => p,
        None => {
            eprintln!("No soli.toml found.");
            process::exit(1);
        }
    };

    let mut pkg = Package::load(&toml_path).unwrap_or_else(|e| {
        eprintln!("Error: Failed to load soli.toml: {}", e);
        process::exit(1);
    });

    if !pkg.dependencies.contains_key(name) {
        eprintln!("Error: Dependency '{}' not found in soli.toml", name);
        process::exit(1);
    }

    let lock_path = toml_path.with_file_name("soli.lock");
    let mut lock = LockFile::load(&lock_path).unwrap_or_default();

    installer::remove_dependency(&mut pkg, name, &mut lock);

    fs::write(&toml_path, pkg.to_toml()).unwrap_or_else(|e| {
        eprintln!("Error: Failed to write soli.toml: {}", e);
        process::exit(1);
    });

    if let Err(e) = lock.save(&lock_path) {
        eprintln!("Warning: Failed to update lock file: {}", e);
    }

    println!();
    println!("  \x1b[32m\x1b[1m✓\x1b[0m Removed dependency '{}'", name);
    println!();
}

pub fn run_install() {
    use solilang::module::{installer, lockfile::LockFile, Package};

    let toml_path = match Package::find(Path::new(".")) {
        Some(p) => p,
        None => {
            eprintln!("No soli.toml found. Run 'soli init' first.");
            process::exit(1);
        }
    };

    let pkg = Package::load(&toml_path).unwrap_or_else(|e| {
        eprintln!("Error: Failed to load soli.toml: {}", e);
        process::exit(1);
    });

    let lock_path = toml_path.with_file_name("soli.lock");
    let mut lock = LockFile::load(&lock_path).unwrap_or_default();

    let remote_deps: Vec<_> = pkg
        .dependencies
        .iter()
        .filter(|(_, d)| {
            matches!(
                d,
                solilang::module::Dependency::Git { .. } | solilang::module::Dependency::Version(_)
            )
        })
        .collect();

    if remote_deps.is_empty() {
        println!();
        println!("  No remote dependencies to install.");
        println!();
        return;
    }

    println!();
    println!("  \x1b[1mInstalling dependencies...\x1b[0m");
    println!();

    if let Err(e) = installer::install_all(&pkg, &mut lock, &lock_path) {
        eprintln!("  \x1b[31mError:\x1b[0m {}", e);
        process::exit(1);
    }

    let summary = installer::installed_summary(&lock);
    if !summary.is_empty() {
        println!();
        println!(
            "  \x1b[32m\x1b[1m✓\x1b[0m {} package(s) installed",
            summary.len()
        );
        for (name, rev, _) in &summary {
            println!("    {} @ {}", name, rev);
        }
    }
    println!();
}

pub fn run_update(name: Option<&str>) {
    use solilang::module::{installer, lockfile::LockFile, Package};

    let toml_path = match Package::find(Path::new(".")) {
        Some(p) => p,
        None => {
            eprintln!("No soli.toml found. Run 'soli init' first.");
            process::exit(1);
        }
    };

    let pkg = Package::load(&toml_path).unwrap_or_else(|e| {
        eprintln!("Error: Failed to load soli.toml: {}", e);
        process::exit(1);
    });

    let lock_path = toml_path.with_file_name("soli.lock");
    let mut lock = LockFile::load(&lock_path).unwrap_or_default();

    println!();
    println!("  \x1b[1mUpdating dependencies...\x1b[0m");
    println!();

    let result = if let Some(pkg_name) = name {
        installer::update_package(pkg_name, &pkg, &mut lock, &lock_path)
    } else {
        installer::update_all(&pkg, &mut lock, &lock_path)
    };

    if let Err(e) = result {
        eprintln!("  \x1b[31mError:\x1b[0m {}", e);
        process::exit(1);
    }

    let summary = installer::installed_summary(&lock);
    if !summary.is_empty() {
        println!();
        println!(
            "  \x1b[32m\x1b[1m✓\x1b[0m {} package(s) up to date",
            summary.len()
        );
        for (name, rev, _) in &summary {
            println!("    {} @ {}", name, rev);
        }
    }
    println!();
}

pub fn run_self_update() -> Result<(), Box<dyn std::error::Error>> {
    let repo = "solisoft/soli_lang";
    let current_version = VERSION;

    println!();
    println!("  \x1b[1mChecking for updates...\x1b[0m");
    println!();

    let os = match std::env::consts::OS {
        "linux" => "linux",
        "macos" => "darwin",
        _ => {
            eprintln!("  \x1b[31mError:\x1b[0m Unsupported operating system");
            process::exit(1);
        }
    };

    let arch = match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" | "arm64" => "arm64",
        _ => {
            eprintln!(
                "  \x1b[31mError:\x1b[0m Unsupported architecture: {}",
                std::env::consts::ARCH
            );
            process::exit(1);
        }
    };

    let tag_url = format!("https://api.github.com/repos/{}/releases/latest", repo);
    // SEC-042a: same TLS-1.2 floor as the shared runtime clients in
    // `http_class.rs`. The `rustls-tls` backend already refuses 1.0/1.1
    // today; the explicit setting prevents a silent regain of a
    // downgrade-prone handshake on a future backend swap.
    let client = reqwest::blocking::Client::builder()
        .user_agent("soli-lang-cli")
        .min_tls_version(reqwest::tls::Version::TLS_1_2)
        .build()
        .expect("Failed to create HTTP client");

    let response = client
        .get(&tag_url)
        .send()
        .map_err(|e| format!("Failed to fetch latest release: {}", e))
        .and_then(|resp| {
            if resp.status() == reqwest::StatusCode::NOT_FOUND {
                Err("Release not found".into())
            } else {
                resp.error_for_status()
                    .map_err(|e| format!("GitHub API error: {}", e))
            }
        });

    let response = match response {
        Ok(r) => r,
        Err(e) => {
            eprintln!("  \x1b[31mError:\x1b[0m {}", e);
            process::exit(1);
        }
    };

    let json: serde_json::Value = response
        .json()
        .map_err(|e| format!("Failed to parse GitHub response: {}", e))
        .expect("Failed to parse release info");

    let latest_tag = json["tag_name"]
        .as_str()
        .unwrap_or("v0.0.0")
        .trim_start_matches('v');

    // Gate on a *numeric* comparison so we never downgrade. The previous
    // guard only short-circuited the exactly-equal case, so a machine running
    // a newer build than the latest published release (e.g. a locally built
    // or pre-release binary) would silently install the older release.
    match solilang::module::compare_versions(latest_tag, current_version) {
        std::cmp::Ordering::Equal => {
            println!(
                "  You are already running the latest version: v{}",
                current_version
            );
            println!();
            return Ok(());
        }
        std::cmp::Ordering::Less => {
            println!("  Current version: v{}", current_version);
            println!("  Latest release:  v{}", latest_tag);
            println!();
            println!("  Your installed version is newer than the latest published release —");
            println!("  nothing to do (refusing to downgrade).");
            println!();
            return Ok(());
        }
        std::cmp::Ordering::Greater => {}
    }

    println!("  Current version: v{}", current_version);
    println!("  Latest version: v{}", latest_tag);
    println!();

    let tarball = format!("soli-{}-{}.tar.gz", os, arch);
    let download_url = format!(
        "https://github.com/{}/releases/download/v{}/{}",
        repo, latest_tag, tarball
    );

    println!("  Downloading {}...", tarball);

    // SEC-041: stage the tarball + extracted binary in a fresh, mode-0700,
    // randomly-named directory. The previous flow used `/tmp/<tarball>`
    // and `/tmp/soli`, both predictable shared-tmp paths — a co-tenant
    // could pre-create `/tmp/soli` as a symlink to `~/.cargo/bin/soli`
    // (or any privileged file) and the `tar::unpack` + `fs::rename` +
    // `chmod 0755` sequence would clobber the target. `tempfile::tempdir`
    // returns a unique directory whose name no other user can predict
    // and whose mode is 0700 on Unix. The dir is dropped (and its
    // contents wiped) when `_temp_dir` goes out of scope at the end of
    // this function, so cleanup is automatic on every exit path.
    let temp_dir = tempfile::Builder::new()
        .prefix("soli-update-")
        .tempdir()
        .map_err(|e| format!("Failed to create temp directory: {}", e))?;
    let tarball_path = temp_dir.path().join(&tarball);
    let binary_path = temp_dir.path().join("soli");

    let response = client
        .get(&download_url)
        .send()
        .map_err(|e| format!("Failed to download: {}", e))
        .and_then(|resp| {
            if resp.status() == reqwest::StatusCode::NOT_FOUND {
                Err(format!(
                    "Release asset not found: {} - may not be available for {}-{}",
                    latest_tag, os, arch
                ))
            } else {
                resp.error_for_status()
                    .map_err(|e| format!("Download error: {}", e))
            }
        });

    let mut response = match response {
        Ok(r) => r,
        Err(e) => {
            eprintln!("  \x1b[31mError:\x1b[0m {}", e);
            process::exit(1);
        }
    };

    let mut file = std::fs::File::create(&tarball_path)
        .map_err(|e| format!("Failed to create temp file: {}", e))
        .expect("Failed to create temp file");

    response
        .copy_to(&mut file)
        .map_err(|e| format!("Failed to write download: {}", e))
        .expect("Failed to write download");
    drop(file);

    // SEC-041: verify the tarball against the published SHA256 before any
    // extraction or privileged install step. Releases publish a
    // `<tarball>.sha256` sibling asset. If the asset is missing we warn
    // and continue (older releases predating this requirement won't have
    // it); a present-but-mismatched checksum hard-fails so a tampered
    // download cannot reach `tar::unpack` or `chmod 0755`.
    let actual_sha = match sha256_of_file(&tarball_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("  \x1b[31mError:\x1b[0m hashing download: {}", e);
            process::exit(1);
        }
    };
    let sha_url = format!("{}.sha256", download_url);
    match client.get(&sha_url).send() {
        Ok(resp) if resp.status().is_success() => {
            let body = resp
                .text()
                .map_err(|e| format!("Failed to read .sha256: {}", e))?;
            let expected = body.split_whitespace().next().unwrap_or("").to_lowercase();
            if expected.is_empty() {
                eprintln!("  \x1b[31mError:\x1b[0m empty .sha256 file at {}", sha_url);
                process::exit(1);
            }
            if expected != actual_sha {
                eprintln!(
                    "  \x1b[31mError:\x1b[0m checksum mismatch: expected {}, got {}",
                    expected, actual_sha
                );
                process::exit(1);
            }
            println!("  Checksum verified.");
        }
        Ok(resp) if resp.status() == reqwest::StatusCode::NOT_FOUND => {
            eprintln!(
                "  \x1b[33mWarning:\x1b[0m no .sha256 published for v{} — \
                 skipping checksum verification.",
                latest_tag
            );
        }
        Ok(resp) => {
            eprintln!(
                "  \x1b[31mError:\x1b[0m fetching .sha256: HTTP {}",
                resp.status()
            );
            process::exit(1);
        }
        Err(e) => {
            eprintln!("  \x1b[31mError:\x1b[0m fetching .sha256: {}", e);
            process::exit(1);
        }
    }

    println!("  Extracting...");
    let tf = std::fs::File::open(&tarball_path).expect("Failed to open tarball");
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(tf));
    archive
        .unpack(temp_dir.path())
        .expect("Failed to extract tarball");

    let current_exe = std::env::current_exe().expect("Failed to get current executable path");
    let backup_path = current_exe.parent().unwrap().join("soli.backup");

    // Map a permission-denied error into a friendly "re-run with sudo" hint —
    // soli is likely installed in a root-owned location (e.g. /usr/local/bin).
    let permission_hint = |e: &std::io::Error| {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            eprintln!();
            eprintln!(
                "  \x1b[31mError:\x1b[0m cannot write to {} (permission denied).",
                current_exe.display()
            );
            eprintln!(
                "  This soli is installed in a root-owned location. Re-run with: \x1b[1msudo soli update\x1b[0m"
            );
            eprintln!();
            process::exit(1);
        }
    };

    // Back up the current binary (rename if possible, else copy + remove).
    if std::fs::rename(&current_exe, &backup_path).is_err() {
        if let Err(e) = std::fs::copy(&current_exe, &backup_path) {
            permission_hint(&e);
            return Err(format!("Failed to backup current binary: {}", e).into());
        }
        if let Err(e) = std::fs::remove_file(&current_exe) {
            permission_hint(&e);
            return Err(format!("Failed to remove old binary: {}", e).into());
        }
    }
    // Move the freshly downloaded binary into place.
    if std::fs::rename(&binary_path, &current_exe).is_err() {
        if let Err(e) = std::fs::copy(&binary_path, &current_exe) {
            permission_hint(&e);
            return Err(format!("Failed to install new binary: {}", e).into());
        }
        std::fs::remove_file(&binary_path).ok();
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&current_exe)
            .expect("Failed to get permissions")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&current_exe, perms)
            .expect("Failed to set executable permissions");
    }

    std::fs::remove_file(&backup_path).ok();
    // `temp_dir` (and its contents — tarball + any extras) is removed when
    // the `TempDir` handle drops at the end of this scope.
    drop(temp_dir);

    println!();
    println!("  \x1b[32m\x1b[1m✓\x1b[0m Updated to v{}", latest_tag);
    println!();
    Ok(())
}

/// SEC-041: stream-hash a file with SHA-256, returning the lowercase hex
/// digest. Used to verify the downloaded release tarball against the
/// `.sha256` sibling asset before any extraction or `chmod 0755` step.
pub(crate) fn sha256_of_file(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut file = File::open(path).map_err(|e| format!("open {}: {}", path.display(), e))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("read {}: {}", path.display(), e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect())
}

pub fn run_login(registry: Option<&str>, token: Option<&str>) {
    use solilang::module::credentials::{save_credentials, Credentials};
    use solilang::module::registry::DEFAULT_REGISTRY;

    let registry_url = registry.unwrap_or(DEFAULT_REGISTRY);

    let token_value = if let Some(t) = token {
        t.to_string()
    } else {
        eprint!("  Enter API token: ");
        let mut input = String::new();
        std::io::stdin().read_line(&mut input).unwrap_or_else(|e| {
            eprintln!("Error reading input: {}", e);
            process::exit(1);
        });
        let input = input.trim().to_string();
        if input.is_empty() {
            eprintln!("Error: Token cannot be empty");
            process::exit(1);
        }
        input
    };

    let creds = Credentials {
        url: registry_url.to_string(),
        token: token_value,
    };

    save_credentials(&creds).unwrap_or_else(|e| {
        eprintln!("Error: {}", e);
        process::exit(1);
    });

    println!();
    println!("  \x1b[32m\x1b[1m✓\x1b[0m Logged in to {}", registry_url);
    println!();
}

pub fn run_publish(registry: Option<&str>) {
    use solilang::module::credentials::load_credentials;
    use solilang::module::registry::{self, DEFAULT_REGISTRY};
    use solilang::module::Package;

    let toml_path = match Package::find(Path::new(".")) {
        Some(p) => p,
        None => {
            eprintln!("No soli.toml found. Run 'soli init' first.");
            process::exit(1);
        }
    };

    let pkg = Package::load(&toml_path).unwrap_or_else(|e| {
        eprintln!("Error: Failed to load soli.toml: {}", e);
        process::exit(1);
    });

    if pkg.name.is_empty() {
        eprintln!("Error: package.name is required in soli.toml");
        process::exit(1);
    }
    if pkg.version.is_empty() {
        eprintln!("Error: package.version is required in soli.toml");
        process::exit(1);
    }

    let creds = load_credentials().unwrap_or_else(|| {
        eprintln!("Error: Not logged in. Run 'soli login' first.");
        process::exit(1);
    });

    let registry_url = match registry {
        Some(url) => url.to_string(),
        None if !creds.url.is_empty() => creds.url.clone(),
        None => DEFAULT_REGISTRY.to_string(),
    };

    let project_dir = toml_path.parent().unwrap_or(Path::new("."));
    let description = pkg.description.as_deref().unwrap_or("");

    println!();
    println!("  \x1b[1mPackaging {}@{}...\x1b[0m", pkg.name, pkg.version);

    let tarball_path = std::env::temp_dir().join(format!("{}-{}.tar.gz", pkg.name, pkg.version));
    create_tarball(project_dir, &tarball_path).unwrap_or_else(|e| {
        eprintln!("Error: Failed to create tarball: {}", e);
        process::exit(1);
    });

    println!("  \x1b[1mPublishing to {}...\x1b[0m", registry_url);

    registry::publish_package(
        &registry_url,
        &creds.token,
        &pkg.name,
        &pkg.version,
        description,
        &tarball_path,
    )
    .unwrap_or_else(|e| {
        let _ = fs::remove_file(&tarball_path);
        eprintln!("  \x1b[31mError:\x1b[0m {}", e);
        process::exit(1);
    });

    let _ = fs::remove_file(&tarball_path);

    println!();
    println!(
        "  \x1b[32m\x1b[1m✓\x1b[0m Published {}@{}",
        pkg.name, pkg.version
    );
    println!();
}

fn create_tarball(project_dir: &Path, dest: &std::path::Path) -> Result<(), String> {
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use walkdir::WalkDir;

    let file = fs::File::create(dest).map_err(|e| format!("Failed to create tarball: {}", e))?;
    let encoder = GzEncoder::new(file, Compression::default());
    let mut archive = tar::Builder::new(encoder);

    let skip_dirs = [".git", "node_modules", "target", ".soli"];

    for entry in WalkDir::new(project_dir).into_iter().filter_entry(|e| {
        let name = e.file_name().to_string_lossy();
        !skip_dirs.iter().any(|d| name == *d)
    }) {
        let entry = entry.map_err(|e| format!("Failed to walk directory: {}", e))?;
        let path = entry.path();
        let relative = path
            .strip_prefix(project_dir)
            .map_err(|e| format!("Failed to compute relative path: {}", e))?;

        if relative.as_os_str().is_empty() {
            continue;
        }

        if path.is_file() {
            archive
                .append_path_with_name(path, relative)
                .map_err(|e| format!("Failed to add file to tarball: {}", e))?;
        } else if path.is_dir() {
            archive
                .append_dir(relative, path)
                .map_err(|e| format!("Failed to add directory to tarball: {}", e))?;
        }
    }

    archive
        .finish()
        .map_err(|e| format!("Failed to finalize tarball: {}", e))?;

    Ok(())
}

pub fn run_db_migrate(action: &DbMigrateAction, folder: &str) {
    use solilang::migration::{DbConfig, MigrationRunner};

    let app_path = Path::new(folder);

    if !app_path.exists() {
        eprintln!("Error: Folder '{}' does not exist", folder);
        process::exit(1);
    }

    let config = DbConfig::from_env(app_path);

    match action {
        DbMigrateAction::Up => {
            println!();
            println!("  \x1b[1mRunning migrations...\x1b[0m");
            println!();

            let runner = MigrationRunner::new(config, app_path);
            match runner.migrate_up() {
                Ok(result) => {
                    println!();
                    println!("  \x1b[32m{}\x1b[0m", result.message);
                    println!();
                }
                Err(e) => {
                    eprintln!("  \x1b[31mError:\x1b[0m {}", e);
                    process::exit(1);
                }
            }
        }
        DbMigrateAction::Down => {
            println!();
            println!("  \x1b[1mRolling back migration...\x1b[0m");
            println!();

            let runner = MigrationRunner::new(config, app_path);
            match runner.migrate_down() {
                Ok(result) => {
                    println!();
                    println!("  \x1b[32m{}\x1b[0m", result.message);
                    println!();
                }
                Err(e) => {
                    eprintln!("  \x1b[31mError:\x1b[0m {}", e);
                    process::exit(1);
                }
            }
        }
        DbMigrateAction::Status => {
            let runner = MigrationRunner::new(config, app_path);
            match runner.status() {
                Ok(status) => {
                    solilang::migration::print_status(&status);
                }
                Err(e) => {
                    eprintln!("  \x1b[31mError:\x1b[0m {}", e);
                    process::exit(1);
                }
            }
        }
        DbMigrateAction::Generate { name } => {
            match solilang::migration::generate_migration(app_path, name) {
                Ok(path) => {
                    println!();
                    println!("  \x1b[32mCreated migration:\x1b[0m {}", path.display());
                    println!();
                }
                Err(e) => {
                    eprintln!("  \x1b[31mError:\x1b[0m {}", e);
                    process::exit(1);
                }
            }
        }
    }
}

/// `soli db:indexes [folder]` — load the app's models (so the class-body
/// index DSL registers), then create any missing declared indexes. The
/// production counterpart of the dev-boot auto-sync; safe to run repeatedly.
pub fn run_db_indexes(folder: &str) {
    let app_path = Path::new(folder);

    if !app_path.exists() {
        eprintln!("Error: Folder '{}' does not exist", folder);
        process::exit(1);
    }

    // Same bootstrap as db:seed: env + DB config, then the model files so
    // every `index`/`vector_index`/`fulltext_index`/`geo_index` declaration
    // lands in the model registry.
    solilang::serve::env_loader::load_env_files(app_path);
    solilang::interpreter::builtins::model::init_db_config();

    let models_dir = app_path.join("app").join("models");
    if models_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(&models_dir) {
            let mut sorted: Vec<_> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|ext| ext == "sl"))
                .collect();
            sorted.sort();
            for path in sorted {
                let Ok(source) = fs::read_to_string(&path) else {
                    continue;
                };
                if let Err(e) = solilang::run_with_options(&source, false) {
                    eprintln!(
                        "  \x1b[33mWarning:\x1b[0m {} failed to load: {}",
                        path.display(),
                        e
                    );
                }
            }
        }
    } else {
        eprintln!(
            "  \x1b[33mWarning:\x1b[0m no app/models directory under '{}'",
            folder
        );
    }

    println!();
    println!("  \x1b[1mSyncing declared indexes...\x1b[0m");
    println!();
    let report = solilang::interpreter::builtins::model::index_sync::sync_declared_indexes();
    if report.is_empty() {
        println!("  Nothing to do — no index declarations found (or all exist).");
    } else {
        for line in &report {
            println!("  {}", line);
        }
    }
    println!();
}

/// `soli routes [folder]` — print the app's expanded route table without
/// starting the server. Routes load in the exact server-boot order
/// (middleware → engine mounts → routes DSL → config/routes.sl → engine
/// routes), so the listing and its row order match what production registers.
pub fn run_routes(folder: &str, grep: Option<&str>, json: bool) {
    use solilang::serve::route_listing;

    let listing = match route_listing::collect_routes(Path::new(folder)) {
        Ok(listing) => listing,
        Err(e) => {
            eprintln!("\x1b[31mError:\x1b[0m {}", e);
            process::exit(1);
        }
    };

    if json {
        println!("{}", route_listing::format_json(&listing, grep));
    } else {
        print!("{}", route_listing::format_table(&listing, grep));
    }
}

/// `soli graph build [folder]` — extract the project's code graph and store it
/// in SolidB (nodes = files/classes/methods/routes/views, edges = imports/
/// inherits/calls/renders/routes_to/relates), embedding node text so agents can
/// retrieve code by semantic search and traverse relationships (graph RAG).
/// `--dry-run` prints the graph as JSON and touches neither SolidB nor the
/// embedding API.
#[allow(clippy::too_many_arguments)]
pub fn run_graph(
    folder: &str,
    no_embed: bool,
    database: Option<&str>,
    dry_run: bool,
    fresh: bool,
    ext: Option<&str>,
    exclude: Option<&str>,
    config_path: Option<&str>,
) {
    use solilang::graph;

    let app_path = Path::new(folder);

    // Choose the extractor: the generic multi-language path when the user asked
    // for extensions (via --ext or a .soligraph.toml), otherwise the Soli-app
    // extractor.
    let config = graph::GraphConfig::load(app_path, ext, exclude, config_path);
    let generic = config.has_extensions();

    // Pass 1 (parse) — the CPU-bound phase that scales with project size.
    let mut build_bar = progress::ProgressBar::new("Building graph");
    let build_result = if generic {
        graph::build_generic_graph(app_path, &config, &mut |done, total| {
            build_bar.set(done, total)
        })
    } else {
        graph::build_graph_with_progress(app_path, &mut |done, total| build_bar.set(done, total))
    };
    build_bar.finish();
    let mut project_graph = match build_result {
        Ok(g) => g,
        Err(e) => {
            eprintln!("\x1b[31mError:\x1b[0m {}", e);
            process::exit(1);
        }
    };

    if dry_run {
        println!("{}", project_graph.to_pretty_json());
        return;
    }

    // Load `.env` (+ `.env.{APP_ENV}`) and cache the DB config so the graph can
    // reach SoliDB — same preamble as `db:seed`. JWT is acquired lazily on the
    // first authenticated call.
    solilang::serve::env_loader::load_env_files(app_path);
    solilang::interpreter::builtins::model::init_db_config();

    let embed = !no_embed;
    let opts = graph::SyncOptions {
        database: database.map(str::to_string),
        embed,
    };

    // Default is an incremental, non-destructive sync: skip when nothing
    // changed, reuse embeddings for unchanged nodes, and upsert/prune instead
    // of dropping. `--fresh` forces a full clean rebuild.
    if !fresh && graph::is_up_to_date(&project_graph.file_hashes, database) {
        println!();
        println!(
            "  \x1b[32m✓\x1b[0m code graph already up to date \x1b[2m({} files unchanged)\x1b[0m",
            project_graph.file_hashes.len()
        );
        println!();
        return;
    }

    let mut reused = 0usize;
    let mut reembedded = 0usize;

    // Embedding — the slow, network-bound phase (sequential API round-trips).
    if fresh {
        if embed {
            let mut embed_bar = progress::ProgressBar::new(&format!(
                "Embedding {} nodes",
                project_graph.nodes.len()
            ));
            let result = graph::embed_graph(&mut project_graph, &mut |done, total| {
                embed_bar.set(done, total)
            });
            embed_bar.finish();
            if let Err(e) = result {
                eprintln!("  \x1b[31mError:\x1b[0m {}", e);
                process::exit(1);
            }
        }
    } else {
        // Reuse cached vectors for unchanged nodes; embed only the deltas.
        let mut embed_bar = progress::ProgressBar::new("Embedding changed nodes");
        let result = graph::embed_incremental(&mut project_graph, &opts, &mut |done, total| {
            embed_bar.set(done, total)
        });
        embed_bar.finish();
        match result {
            Ok((r, e)) => {
                reused = r;
                reembedded = e;
            }
            Err(e) => {
                eprintln!("  \x1b[31mError:\x1b[0m {}", e);
                process::exit(1);
            }
        }
    }

    // Write to SolidB: clean rebuild for --fresh, non-destructive sync otherwise.
    let mut write_bar = progress::ProgressBar::new(if fresh {
        "Writing to SolidB"
    } else {
        "Syncing to SolidB"
    });
    let write_result = if fresh {
        graph::write_graph(&project_graph, &opts, &mut |done, total| {
            write_bar.set(done, total)
        })
    } else {
        graph::sync_graph(&project_graph, &opts, &mut |done, total| {
            write_bar.set(done, total)
        })
    };
    write_bar.finish();
    let report = match write_result {
        Ok(r) => r,
        Err(e) => {
            eprintln!("  \x1b[31mError:\x1b[0m {}", e);
            process::exit(1);
        }
    };

    let embed_note = if report.embedded {
        if fresh {
            format!(", embedded (dim {})", report.dimension)
        } else {
            format!(", embeddings: {} reused / {} refreshed", reused, reembedded)
        }
    } else {
        ", no embeddings".to_string()
    };
    let verb = if fresh { "rebuilt" } else { "synced" };
    println!();
    println!(
        "  \x1b[32m→\x1b[0m {} {} nodes, {} edges{} → SolidB \x1b[1m{}\x1b[0m",
        verb, report.nodes, report.edges, embed_note, report.database
    );
    if project_graph.unresolved_calls > 0 {
        println!(
            "  \x1b[2m({} ambiguous function call(s) left unlinked)\x1b[0m",
            project_graph.unresolved_calls
        );
    }
    println!();
}

/// `soli graph query "<question>" [folder]` — retrieve the code most relevant
/// to a task from the graph in SolidB (semantic seed + graph expansion), for
/// agents. `--json` emits a structured result; otherwise a scannable summary.
#[allow(clippy::too_many_arguments)]
pub fn run_graph_query(
    question: &str,
    folder: &str,
    database: Option<&str>,
    limit: usize,
    hops: usize,
    path: Option<&str>,
    kind: Option<&str>,
    json: bool,
) {
    use solilang::graph;

    let app_path = Path::new(folder);
    solilang::serve::env_loader::load_env_files(app_path);
    solilang::interpreter::builtins::model::init_db_config();

    let kinds = kind.map(graph::parse_kinds).unwrap_or_default();
    let opts = graph::QueryOptions {
        database: database.map(str::to_string),
        limit,
        hops,
        path: path.map(str::to_string),
        kinds,
    };
    let result = match graph::run_query(question, &opts) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("\x1b[31mError:\x1b[0m {}", e);
            process::exit(1);
        }
    };

    if json {
        println!("{}", result.to_pretty_json());
        return;
    }

    let noun = if result.results.len() == 1 {
        "result"
    } else {
        "results"
    };
    let mut scope_parts = Vec::new();
    if let Some(p) = path {
        if !p.is_empty() {
            scope_parts.push(format!("path {}", p));
        }
    }
    if let Some(k) = kind {
        if !k.is_empty() {
            scope_parts.push(format!("kind {}", k));
        }
    }
    let scope = if scope_parts.is_empty() {
        String::new()
    } else {
        format!(", {}", scope_parts.join(", "))
    };
    println!();
    println!(
        "  \x1b[1mQuery\x1b[0m \"{}\"  \x1b[2m({}, {} {}{})\x1b[0m",
        result.query,
        result.mode,
        result.results.len(),
        noun,
        scope
    );
    if result.results.is_empty() {
        println!();
        println!("  No matching code found. Has the graph been built? (soli graph build)");
        println!();
        return;
    }
    for (idx, hit) in result.results.iter().enumerate() {
        println!();
        let loc = if hit.file.is_empty() {
            String::new()
        } else {
            format!("  \x1b[2m{}:{}\x1b[0m", hit.file, hit.line)
        };
        println!(
            "  \x1b[1m{}.\x1b[0m \x1b[36m{}\x1b[0m  {}{}  \x1b[2m[{:.2}]\x1b[0m",
            idx + 1,
            hit.kind,
            hit.qualified_name,
            loc,
            hit.score
        );
        if !hit.signature.is_empty() {
            println!("     \x1b[2m{}\x1b[0m", hit.signature);
        }
        if !hit.snippet.is_empty() {
            // One line of context for humans; full snippet is in --json. The
            // composed snippet begins with the synthetic `<kind> <qualified>`
            // header and signature already printed above, so skip those and
            // surface the doc/body instead of duplicated metadata.
            let header = format!("{} {}", hit.kind, hit.qualified_name);
            let context: String = hit
                .snippet
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && *line != header && *line != hit.signature)
                .take(2)
                .collect::<Vec<_>>()
                .join(" · ");
            if !context.is_empty() {
                let context = if context.chars().count() > 120 {
                    let mut end = 117;
                    while end > 0 && !context.is_char_boundary(end) {
                        end -= 1;
                    }
                    format!("{}…", &context[..end])
                } else {
                    context
                };
                println!("     \x1b[2m{}\x1b[0m", context);
            }
        }
        // Show a bounded set of relationships; the rest stay in --json.
        let shown = hit.neighbors.iter().take(12);
        for nb in shown {
            let arrow = if nb.direction == "out" { "→" } else { "←" };
            println!(
                "       {} \x1b[33m{:<12}\x1b[0m {}:{}",
                arrow, nb.edge_kind, nb.kind, nb.name
            );
        }
        if hit.neighbors.len() > 12 {
            println!(
                "       \x1b[2m(+{} more relationships)\x1b[0m",
                hit.neighbors.len() - 12
            );
        }
    }
    println!();
}

pub fn run_db_seed(action: &DbSeedAction, folder: &str) {
    let app_path = Path::new(folder);

    if !app_path.exists() {
        eprintln!("Error: Folder '{}' does not exist", folder);
        process::exit(1);
    }

    match action {
        DbSeedAction::Generate { name } => {
            match solilang::scaffold::seed_generator::generate_seed(app_path, name) {
                Ok(path) => {
                    println!();
                    println!("  \x1b[32mCreated seed:\x1b[0m {}", path.display());
                    println!();
                }
                Err(e) => {
                    eprintln!("  \x1b[31mError:\x1b[0m {}", e);
                    process::exit(1);
                }
            }
        }
        DbSeedAction::Run { file } => {
            // Collect the seed files to run. When a specific file is given,
            // run only that one (resolved relative to the project folder).
            // Otherwise discover them in order: the single-file `db/seeds.sl`
            // first, then every `db/seeds/*.sl` sorted by name (timestamp
            // prefixes give a deterministic order). Each file runs every
            // invocation — seeds are not tracked, so authors make them
            // idempotent themselves (e.g. find_or_create).
            let mut seed_files: Vec<std::path::PathBuf> = Vec::new();
            if let Some(file) = file {
                let path = app_path.join(file);
                if !path.is_file() {
                    eprintln!();
                    eprintln!(
                        "  \x1b[31mError:\x1b[0m seed file not found: {}",
                        path.display()
                    );
                    process::exit(1);
                }
                seed_files.push(path);
            } else {
                let single = app_path.join("db/seeds.sl");
                if single.is_file() {
                    seed_files.push(single);
                }
                let seeds_dir = app_path.join("db/seeds");
                if seeds_dir.is_dir() {
                    if let Ok(entries) = fs::read_dir(&seeds_dir) {
                        let mut dir_files: Vec<std::path::PathBuf> = entries
                            .flatten()
                            .map(|e| e.path())
                            .filter(|p| p.extension().is_some_and(|ext| ext == "sl"))
                            .collect();
                        dir_files.sort();
                        seed_files.extend(dir_files);
                    }
                }
            }

            if seed_files.is_empty() {
                println!();
                println!(
                    "  No seed files found. Create \x1b[1mdb/seeds.sl\x1b[0m or run \x1b[1msoli db:seed generate <name>\x1b[0m."
                );
                println!();
                return;
            }

            // Load `.env` (+ `.env.{APP_ENV}`) and cache the DB config so the
            // Model layer can reach SoliDB. JWT auth is acquired lazily by the
            // CRUD layer on the first authenticated call — same as the test
            // runner — so no explicit `init_jwt_token()` is needed here.
            solilang::serve::env_loader::load_env_files(app_path);
            solilang::interpreter::builtins::model::init_db_config();

            // Auto-load `app/models` and `app/services` as a preamble so seed
            // scripts can call `User.create({...})` without explicit imports,
            // mirroring how the test runner preloads them.
            let mut model_preamble_files: Vec<(std::path::PathBuf, String)> = Vec::new();
            for sub in ["models", "services"] {
                let dir = app_path.join("app").join(sub);
                if !dir.is_dir() {
                    continue;
                }
                let Ok(entries) = fs::read_dir(&dir) else {
                    continue;
                };
                let mut sorted: Vec<_> = entries
                    .flatten()
                    .filter(|e| e.path().extension().is_some_and(|ext| ext == "sl"))
                    .collect();
                sorted.sort_by_key(|e| e.path());
                for entry in sorted {
                    let path = entry.path();
                    if let Ok(content) = fs::read_to_string(&path) {
                        let absolute = path.canonicalize().unwrap_or(path);
                        model_preamble_files.push((absolute, content));
                    }
                }
            }

            println!();
            println!("  \x1b[1mSeeding database...\x1b[0m");
            println!();

            for path in &seed_files {
                let display = path
                    .strip_prefix(app_path)
                    .unwrap_or(path.as_path())
                    .display();
                let source = match fs::read_to_string(path) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("  \x1b[31mError:\x1b[0m failed to read {}: {}", display, e);
                        process::exit(1);
                    }
                };

                println!("  \x1b[2mSeeding\x1b[0m {}", display);

                let (_assertions, result) = solilang::run_with_path_and_coverage(
                    &source,
                    Some(path),
                    false,
                    None,
                    None,
                    &model_preamble_files,
                );
                if let Err(e) = result {
                    eprintln!();
                    eprintln!("  \x1b[31mError seeding {}:\x1b[0m {}", display, e);
                    process::exit(1);
                }
            }

            println!();
            println!("  \x1b[32mSeeding complete.\x1b[0m");
            println!();
        }
    }
}

#[cfg(test)]
mod tests {
    use solilang::module::compare_versions;
    use std::cmp::Ordering;

    #[test]
    fn newer_release_is_greater() {
        assert_eq!(compare_versions("1.9.0", "1.8.28"), Ordering::Greater);
        assert_eq!(compare_versions("2.0.0", "1.9.0"), Ordering::Greater);
    }

    #[test]
    fn equal_versions() {
        assert_eq!(compare_versions("1.9.0", "1.9.0"), Ordering::Equal);
        assert_eq!(compare_versions("v1.9.0", "1.9.0"), Ordering::Equal);
    }

    #[test]
    fn downgrade_is_less() {
        // The exact regression: installed 1.9.0, "latest" published 1.8.28.
        assert_eq!(compare_versions("1.8.28", "1.9.0"), Ordering::Less);
    }

    #[test]
    fn numeric_not_lexicographic() {
        // A string compare would put "1.10.0" before "1.9.0"; numeric must not.
        assert_eq!(compare_versions("1.10.0", "1.9.0"), Ordering::Greater);
        assert_eq!(compare_versions("1.8.28", "1.8.9"), Ordering::Greater);
    }

    #[test]
    fn prerelease_suffix_ignored_for_ordering() {
        assert_eq!(compare_versions("1.10.0-rc1", "1.10.0"), Ordering::Equal);
        assert_eq!(compare_versions("1.10.0-rc1", "1.9.0"), Ordering::Greater);
    }
}
