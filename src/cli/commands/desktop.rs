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
