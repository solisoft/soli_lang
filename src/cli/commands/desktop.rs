//! `soli desktop build` — package an app as a self-contained desktop artifact.

use std::path::{Path, PathBuf};
use std::process;

use solilang::desktop::container::{self, ContainerInputs};
use solilang::desktop::manifest::{DesktopManifest, MANIFEST_VERSION};

use super::resolve_bundle_key;

/// Everything `soli desktop build` needs.
pub struct DesktopBuildArgs<'a> {
    pub folder: &'a str,
    pub app_id: &'a str,
    pub app_name: Option<&'a str>,
    pub output: Option<&'a str>,
    /// Path to the database binary to embed. Once per-target database releases
    /// are published this becomes optional, resolved by download like the soli
    /// runtime; until then it is required and supplied explicitly.
    pub db_binary: &'a str,
    /// Directory of `<collection>.ndjson` files shipped as reference data.
    pub seed: Option<&'a str>,
    pub protect: bool,
    pub target: Option<&'a str>,
}

pub fn run(args: DesktopBuildArgs<'_>) {
    let source_dir = resolve_source_dir(args.folder);

    if let Err(e) = solilang::desktop::paths::validate_app_id(args.app_id) {
        eprintln!("Error: {}", e);
        process::exit(1);
    }

    if let Some(t) = args.target {
        if let Err(e) = crate::cli::standalone::validate_target(t) {
            eprintln!("Error: {}", e);
            process::exit(1);
        }
    }

    let db_binary_path = Path::new(args.db_binary);
    if !db_binary_path.is_file() {
        eprintln!(
            "Error: database binary '{}' not found. Build it (cargo build --release in the \
             solidb repo) and pass --solidb <path>.",
            db_binary_path.display()
        );
        process::exit(1);
    }

    // The key resolves through the same chain as `soli build --encrypt` and as
    // serving, and may live in the app's .env.
    solilang::serve::env_loader::load_env_files(&source_dir);

    println!("Building desktop app from {}...", source_dir.display());

    // 1. The application itself, encrypted. Always encrypted: an unprotected
    //    desktop artifact would ship its source in the clear to every user.
    let app_bundle = if args.protect {
        solilang::bundle::BundleBuilder::build_protected(&source_dir)
    } else {
        solilang::bundle::BundleBuilder::build(&source_dir)
    }
    .unwrap_or_else(|e| {
        eprintln!("Error building app bundle: {}", e);
        process::exit(1);
    });

    let (key, key_source) = resolve_bundle_key().unwrap_or_else(|e| {
        eprintln!("Error: {}", e);
        process::exit(1);
    });
    println!("  Encrypting application ({})", key_source);
    let encrypted_app = solilang::bundle::encrypt_bundle(&app_bundle, &key).unwrap_or_else(|e| {
        eprintln!("Error encrypting application: {}", e);
        process::exit(1);
    });

    // 2. The database binary.
    let db_binary = std::fs::read(db_binary_path).unwrap_or_else(|e| {
        eprintln!("Error reading {}: {}", db_binary_path.display(), e);
        process::exit(1);
    });
    println!(
        "  Embedding database binary ({:.1} MB)",
        db_binary.len() as f64 / (1024.0 * 1024.0)
    );

    // 3. Reference data.
    let seed = args
        .seed
        .map(|dir| collect_seed(Path::new(dir)))
        .unwrap_or_default();
    if !seed.is_empty() {
        println!("  Embedding {} reference collection(s)", seed.len());
    }

    let app_name = args
        .app_name
        .map(|s| s.to_string())
        .unwrap_or_else(|| default_app_name(&source_dir));

    let manifest = DesktopManifest {
        manifest_version: MANIFEST_VERSION,
        app_id: args.app_id.to_string(),
        app_name,
        soli_version: env!("CARGO_PKG_VERSION").to_string(),
        solidb_version: read_db_version(db_binary_path),
        // Filled in by `container::build` from the embedded bytes.
        solidb_sha256: String::new(),
        seed_version: (!seed.is_empty()).then(|| container::seed_digest(&seed)[..16].to_string()),
        seed_sha256: None,
    };

    let payload = container::build(ContainerInputs {
        encrypted_app,
        db_binary,
        seed,
        manifest,
    })
    .unwrap_or_else(|e| {
        eprintln!("Error assembling desktop container: {}", e);
        process::exit(1);
    });

    // 4. Staple onto a runtime, exactly as a standalone build does.
    let output_path = resolve_output_path(args.output, &source_dir, args.target);
    if output_path.is_dir() {
        eprintln!(
            "Error: output path '{}' is a directory — pass --output <file>",
            output_path.display()
        );
        process::exit(1);
    }

    crate::cli::standalone::write_standalone_exe(&payload, &output_path, args.target)
        .unwrap_or_else(|e| {
            eprintln!("Error writing desktop executable: {}", e);
            process::exit(1);
        });

    let size = std::fs::metadata(&output_path)
        .map(|m| m.len())
        .unwrap_or(0);
    println!(
        "\nWrote {} ({:.1} MB)",
        output_path.display(),
        size as f64 / (1024.0 * 1024.0)
    );
    println!("Run it directly; it starts its own database and opens the app.");
}

fn resolve_source_dir(folder: &str) -> PathBuf {
    let dir = if folder == "." {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    } else {
        PathBuf::from(folder)
    };
    if !dir.is_dir() {
        eprintln!("Error: '{}' is not a directory", folder);
        process::exit(1);
    }
    dir
}

fn default_app_name(source_dir: &Path) -> String {
    source_dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "app".to_string())
}

fn resolve_output_path(output: Option<&str>, source_dir: &Path, target: Option<&str>) -> PathBuf {
    match output {
        Some(path) => PathBuf::from(path),
        None => {
            let base = default_app_name(source_dir);
            let name = match target {
                Some(t) => format!("{}-{}", base, crate::cli::standalone::target_label(Some(t))),
                None => base,
            };
            PathBuf::from(name)
        }
    }
}

/// Ask the database binary for its version, for the manifest's diagnostics.
/// Best-effort: a binary for another architecture cannot be run here.
fn read_db_version(binary: &Path) -> String {
    std::process::Command::new(binary)
        .arg("--version")
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Read `<collection>.ndjson` files from a directory.
fn collect_seed(dir: &Path) -> Vec<(String, Vec<u8>)> {
    if !dir.is_dir() {
        eprintln!("Error: seed directory '{}' does not exist", dir.display());
        process::exit(1);
    }

    let mut seed = Vec::new();
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| {
        eprintln!("Error reading {}: {}", dir.display(), e);
        process::exit(1);
    });

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("ndjson") {
            continue;
        }
        let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let bytes = std::fs::read(&path).unwrap_or_else(|e| {
            eprintln!("Error reading {}: {}", path.display(), e);
            process::exit(1);
        });
        seed.push((name.to_string(), bytes));
    }

    seed.sort_by(|a, b| a.0.cmp(&b.0));
    seed
}

/// Database the app uses. Matches the model layer's own default, so an app that
/// never sets SOLIDB_DATABASE finds its reference data where it expects it.
const DEFAULT_DATABASE: &str = "default";

// ---------------------------------------------------------------------------
// Boot
// ---------------------------------------------------------------------------

/// Launch a desktop artifact.
///
/// Ordering is not incidental. The model layer latches `SOLIDB_HOST` the first
/// time it touches the database and caches it for the process lifetime, so the
/// database must be running and its address exported *before* the server
/// starts. And the key server is consulted before any of that, so a revoked
/// install fails fast instead of after paying for a database boot.
pub fn boot(
    payload: &[u8],
    port: u16,
    dev_mode: bool,
    workers: usize,
    origin: &str,
) -> Result<(), String> {
    let container = solilang::desktop::container::open(payload)?;
    let manifest = container.manifest.clone();

    println!("Starting {}...", manifest.app_name);

    // 1. One instance per install: two servers over one database directory
    //    would fail deep inside the storage engine with an unhelpful error.
    let paths = solilang::desktop::paths::for_app(&manifest.app_id)?;
    paths.ensure()?;
    let lock_path = paths.instance_lock();
    let _instance_lock = match solilang::platform::lock::try_acquire(&lock_path)? {
        Some(lock) => lock,
        None => {
            return Err(format!(
                "{} is already running (lock held at {})",
                manifest.app_name,
                lock_path.display()
            ))
        }
    };

    // 2. The key, before anything expensive. No key, no app — by design: the
    //    key is never persisted, so there is nothing to fall back to offline.
    let (key, key_source) =
        resolve_bundle_key().map_err(|e| format!("'{}' could not be unlocked — {}", origin, e))?;
    println!("  Unlocked ({})", key_source);

    // 3. The database binary, extracted once and reused. `container::open`
    //    already verified it against the manifest, which is what makes reusing
    //    a copy from a user-writable cache safe.
    let db_binary_path = extract_db_binary(&paths.cache, container.db_binary, &manifest)?;

    // 4. Start the database and point the model layer at it.
    let options = solilang::desktop::db::DbOptions::new(
        db_binary_path,
        paths.data.clone(),
        paths.state.clone(),
    );
    let db = solilang::desktop::db::start(&options)?;
    println!("  Database ready on port {}", db.port);
    export_db_environment(&db);

    // 4b. Reference data, only when it differs from what is installed. This
    //     replaces collections wholesale, so it runs before the app can serve
    //     a request against half-imported data.
    if solilang::desktop::seed::needs_import(&paths.state, &manifest) {
        let owned: Vec<(String, Vec<u8>)> = container
            .seed
            .iter()
            .map(|(name, bytes)| (name.clone(), bytes.to_vec()))
            .collect();
        println!("  Importing reference data...");
        solilang::desktop::seed::import(&db.host_url(), DEFAULT_DATABASE, &db.credentials, &owned)?;
        // Recorded only after success, so a failure part-way through leaves
        // the watermark stale and the next launch retries.
        solilang::desktop::seed::record_watermark(&paths.state, &manifest)?;
    }

    // 5. Decrypt and extract the application itself.
    let app_bundle = solilang::bundle::decrypt_bundle(container.encrypted_app, &key)?;
    let bundle = solilang::bundle::BundleReader::new(&app_bundle)?;
    solilang::bundle::check_bundle_meta(bundle.entries())?;

    let tmp_dir = super::encrypted_extraction_dir()?;
    solilang::cleanup::register_cleanup_dir(&tmp_dir);
    for (path, content) in bundle.entries() {
        let full_path = tmp_dir.join(path);
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create '{}': {}", parent.display(), e))?;
        }
        std::fs::write(&full_path, content)
            .map_err(|e| format!("cannot write '{}': {}", full_path.display(), e))?;
    }

    let vfs = solilang::virtual_fs::DiskFS::new(&tmp_dir.to_string_lossy());
    solilang::serve::init_global_vfs(vfs);

    // 6. Loopback only. A desktop app has no business listening on the
    //    network, and pinning this also skips the outbound probe the
    //    all-interfaces path makes just to print a LAN URL.
    std::env::set_var("SOLI_HOST", "127.0.0.1");

    // Keep the database alive for the whole run: dropping the handle stops the
    // process, and serving blocks until shutdown.
    let db_guard = db;
    let app_name = manifest.app_name.clone();
    let result = solilang::serve::serve_folder_with_options_and_hooks(
        &tmp_dir,
        port,
        dev_mode,
        workers,
        Some(Box::new(move |bound| {
            // Arm the gate before opening anything: the launch URL carries the
            // one-shot token, so it cannot be minted any earlier (the port is
            // not known) or any later (the browser would race the gate).
            let launch_url = solilang::desktop::token::arm(bound);
            let opened = solilang::desktop::shell::open(&launch_url);
            match opened {
                solilang::desktop::shell::Opened::AppWindow => {
                    println!("\n{} opened in an app window.", app_name)
                }
                solilang::desktop::shell::Opened::BrowserTab => {
                    println!("\n{} opened in your browser.", app_name)
                }
                solilang::desktop::shell::Opened::Nothing => {
                    println!("\nCould not open a browser automatically.")
                }
            }

            // Always print the link, even when a window supposedly opened.
            // Launching only tells us a process *started*, not that a window
            // appeared — a browser that forwards to an existing instance and
            // exits looks identical to success. Staying quiet on that path
            // would leave the user with an armed gate and no way through it.
            println!("If it did not appear, open:\n  {}", launch_url);
        })),
    )
    .map_err(|e| e.to_string());

    drop(db_guard);
    result
}

/// Export the database address and credentials.
///
/// Must happen before the server starts: the model layer caches its connection
/// configuration on first use and never re-reads it.
fn export_db_environment(db: &solilang::desktop::db::DbHandle) {
    let host = db.host_url();
    std::env::set_var("SOLIDB_HOST", &host);
    std::env::set_var("SOLIDB_USERNAME", &db.credentials.username);
    std::env::set_var("SOLIDB_PASSWORD", &db.credentials.password);

    // The session store reads its own variable and does *not* fall back to
    // SOLIDB_HOST, so setting only the model-layer one would leave sessions
    // pointing at a default address where nothing is listening.
    std::env::set_var("SOLI_SOLIDB_HOST", &host);
    std::env::set_var("SOLIDB_DATABASE", DEFAULT_DATABASE);

    // A `.env` shipped inside the app must not be able to redirect the
    // database somewhere else.
    std::env::set_var(
        "SOLI_PROTECT_ENV",
        "SOLIDB_HOST,SOLIDB_USERNAME,SOLIDB_PASSWORD,SOLIDB_DATABASE,SOLI_SOLIDB_HOST",
    );
}

/// Write the database binary to the cache, reusing an identical existing copy.
///
/// Named by content hash so a new app version lands beside the old one rather
/// than racing to overwrite a binary a running instance may be executing.
fn extract_db_binary(
    cache_dir: &Path,
    bytes: &[u8],
    manifest: &DesktopManifest,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(cache_dir)
        .map_err(|e| format!("cannot create {}: {}", cache_dir.display(), e))?;

    let short = &manifest.solidb_sha256[..16.min(manifest.solidb_sha256.len())];
    let path = cache_dir.join(format!("solidb-{}", short));

    // Reuse only if the bytes on disk still match — the cache is user-writable
    // and may have been altered or truncated since the last launch.
    if let Ok(existing) = std::fs::read(&path) {
        if container::sha256_hex(&existing) == manifest.solidb_sha256 {
            return Ok(path);
        }
    }

    std::fs::write(&path, bytes).map_err(|e| format!("cannot write {}: {}", path.display(), e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("cannot make {} executable: {}", path.display(), e))?;
    }
    Ok(path)
}
