mod test_runner;

use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::Path;
use std::process;

use crate::cli::args::{print_usage, DbMigrateAction, EngineAction, Options, VERSION};

#[cfg(unix)]
use daemonize::Daemonize;
#[cfg(unix)]
use nix::sys::signal::{kill, Signal};
#[cfg(unix)]
use nix::unistd::Pid;

pub fn run_build(folder: &str, output: Option<&str>, standalone: bool, target: Option<&str>) {
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

    if standalone {
        eprintln!("Standalone binary output not yet implemented");
        process::exit(1);
    }

    if let Some(t) = target {
        match t {
            "wasm32-unknown-unknown" | "wasm" => {
                return run_build_wasm(&source_dir, output);
            }
            _ => {
                eprintln!("Unknown target '{}'. Supported: wasm, wasm32-unknown-unknown", t);
                process::exit(1);
            }
        }
    }

    println!("Building bundle from {}...", source_dir.display());

    let bundle_data = match solilang::bundle::BundleBuilder::build(&source_dir) {
        Ok(data) => data,
        Err(e) => {
            eprintln!("Error building bundle: {}", e);
            process::exit(1);
        }
    };

    let output_path = match output {
        Some(path) => Path::new(path).to_path_buf(),
        None => {
            let name = source_dir
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "app".to_string());
            Path::new(&format!("{}.soli", name)).to_path_buf()
        }
    };

    match std::fs::write(&output_path, &bundle_data) {
        Ok(_) => {
            let size_kb = bundle_data.len() as f64 / 1024.0;
            println!(
                "  \x1b[32m\x1b[1m✓\x1b[0m Bundle written to {} ({:.1} KB)",
                output_path.display(),
                size_kb
            );
        }
        Err(e) => {
            eprintln!("Error writing bundle: {}", e);
            process::exit(1);
        }
    }
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

    let bundle = solilang::bundle::BundleReader::new(&bundle_data)?;

    // Extract to a temp directory
    let tmp_dir = std::env::temp_dir().join(format!("soli_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir).map_err(|e| format!("Failed to create temp dir: {}", e))?;

    for (path, content) in bundle.entries() {
        let full_path = tmp_dir.join(path);
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create dir '{}': {}", parent.display(), e))?;
        }
        std::fs::write(&full_path, content)
            .map_err(|e| format!("Failed to write '{}': {}", full_path.display(), e))?;
    }

    println!("Extracted bundle to {}", tmp_dir.display());

    // Serve from the extracted temp directory
    let folder_path = tmp_dir.to_string_lossy().to_string();
    // Create a regular DiskFS pointing at the temp dir
    let vfs = solilang::virtual_fs::DiskFS::new(&folder_path);
    solilang::serve::init_global_vfs(vfs);

    solilang::serve::serve_folder_with_options(&tmp_dir, port, dev_mode, workers)
        .map_err(|e| e.to_string())
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
            files.extend(test_runner::collect_test_files(t));
        }
    }

    if files.is_empty() {
        println!("No .sl files found.");
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

/// Build a Soli application as a WASM Cloudflare Worker.
/// Creates a deployable project with wrangler.toml and the compiled .wasm binary.
fn run_build_wasm(source_dir: &Path, output: Option<&str>) {
    let app_name = source_dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "app".to_string());

    let out_dir = match output {
        Some(p) => Path::new(p).to_path_buf(),
        None => Path::new(&format!("{}_worker", app_name)).to_path_buf(),
    };

    // Step 1: Find the wasm-worker template
    let template_dir = find_template_dir();

    // Step 2: Create output directory from template
    if out_dir.exists() {
        eprintln!("Error: Output directory '{}' already exists", out_dir.display());
        process::exit(1);
    }
    copy_template(&template_dir.join("wasm-worker"), &out_dir);

    // Step 3: Copy source files into the worker project and generate bundle module
    let soli_src_dir = out_dir.join("soli_app");
    fs::create_dir_all(&soli_src_dir).unwrap_or_else(|e| {
        eprintln!("Error creating soli_app directory: {}", e);
        process::exit(1);
    });
    copy_source_files(source_dir, &soli_src_dir);
    let bundle_content = generate_bundle_rs(&soli_src_dir);
    let bundle_rs_path = out_dir.join("src").join("bundle.rs");
    fs::write(&bundle_rs_path, &bundle_content).unwrap_or_else(|e| {
        eprintln!("Error writing bundle.rs: {}", e);
        process::exit(1);
    });

    // Step 4: Update Cargo.toml dependency path to point to the soli installation
    let cargo_toml_path = out_dir.join("Cargo.toml");
    let cargo_content = fs::read_to_string(&cargo_toml_path).unwrap_or_else(|e| {
        eprintln!("Error reading Cargo.toml: {}", e);
        process::exit(1);
    });
    let soli_path = find_soli_install_path();
    let cargo_content = cargo_content.replace("__SOLI_PATH__", &soli_path);
    fs::write(&cargo_toml_path, &cargo_content).unwrap_or_else(|e| {
        eprintln!("Error writing Cargo.toml: {}", e);
        process::exit(1);
    });

    // Step 5: Generate wrangler.toml
    let wrangler_content = format!(
        r#"name = "{}"
main = "pkg/soli_wasm_worker.wasm"
compatibility_date = "2025-01-01"

[site]
bucket = "./public"
"#,
        app_name
    );
    fs::write(out_dir.join("wrangler.toml"), &wrangler_content).unwrap_or_else(|e| {
        eprintln!("Error writing wrangler.toml: {}", e);
        process::exit(1);
    });

    println!(
        "  \x1b[32m\x1b[1m✓\x1b[0m WASM worker project created at {}",
        out_dir.display()
    );
    println!();
    println!("  To build, run:");
    println!("    cd {} && cargo build --target wasm32-unknown-unknown --release", out_dir.display());
    println!();
    println!("  To deploy to Cloudflare Workers:");
    println!("    cd {} && npx wrangler deploy", out_dir.display());
}

/// Find the template directory (relative to the binary or absolute).
fn find_template_dir() -> std::path::PathBuf {
    // Try relative to the binary's location
    if let Ok(exe) = env::current_exe() {
        // Look for template/ relative to the binary (installed case)
        let p = exe.parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("template"))
            .filter(|p| p.join("wasm-worker").exists());
        if let Some(p) = p {
            return p;
        }
    }
    // Fallback: look in the repository source tree (development case)
    let dev_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("template");
    if dev_path.join("wasm-worker").exists() {
        return dev_path;
    }
    eprintln!("Error: Could not find wasm-worker template directory");
    process::exit(1);
}

/// Copy a template directory recursively.
fn copy_template(src: &Path, dst: &Path) {
    fn copy_recursively(src: &Path, dst: &Path) -> std::io::Result<()> {
        if src.is_dir() {
            fs::create_dir_all(dst)?;
            for entry in fs::read_dir(src)? {
                let entry = entry?;
                let entry_type = entry.file_type()?;
                let src_path = entry.path();
                let dst_path = dst.join(entry.file_name());
                if entry_type.is_dir() {
                    copy_recursively(&src_path, &dst_path)?;
                } else {
                    fs::copy(&src_path, &dst_path)?;
                }
            }
        }
        Ok(())
    }
    copy_recursively(src, dst).unwrap_or_else(|e| {
        eprintln!("Error copying template: {}", e);
        process::exit(1);
    });
}

/// Copy Soli source files from the user's project into the worker project.
fn copy_source_files(src: &Path, dst: &Path) {
    fn collect_files(src: &Path, dst: &Path) {
        if let Ok(entries) = fs::read_dir(src) {
            fs::create_dir_all(dst).ok();
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name();
                let dst_path = dst.join(&name);
                if path.is_dir() {
                    collect_files(&path, &dst_path);
                } else if path.extension().map_or(false, |e| {
                    matches!(e.to_str(), Some("sl" | "slv" | "erb" | "toml" | "json" | "yml" | "yaml" | "css" | "js" | "md"))
                }) {
                    fs::copy(&path, &dst_path).ok();
                }
            }
        }
    }
    collect_files(src, dst);
}

/// Generate src/bundle.rs that embeds Soli source files via include_str!.
fn generate_bundle_rs(soli_app_dir: &Path) -> String {
    let mut lines = Vec::new();
    lines.push("// Auto-generated by `soli build --target wasm`. Do not edit.".to_string());
    lines.push("use std::collections::HashMap;".to_string());
    lines.push(String::new());
    lines.push("pub fn get_sources() -> HashMap<&'static str, &'static str> {".to_string());
    lines.push("    let mut m = HashMap::new();".to_string());

    fn collect_files(dir: &Path, prefix: &str, lines: &mut Vec<String>) {
        if let Ok(entries) = fs::read_dir(dir) {
            let mut entries: Vec<_> = entries.filter_map(|e| e.ok()).collect();
            entries.sort_by_key(|e| e.file_name());
            for entry in entries {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                let rel_path = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{}/{}", prefix, name)
                };
                if path.is_dir() {
                    collect_files(&path, &rel_path, lines);
                } else if path.extension().map_or(false, |e| {
                    matches!(e.to_str(), Some("sl" | "slv" | "erb" | "toml" | "json" | "yml" | "yaml" | "css" | "js" | "md"))
                }) {
                    // Path relative to src/bundle.rs — soli_app/ is at the project root
                    let include_path = format!("../soli_app/{}", rel_path);
                    lines.push(format!(
                        "    m.insert(\"{}\", include_str!(r\"{}\"));",
                        rel_path, include_path
                    ));
                }
            }
        }
    }
    collect_files(soli_app_dir, "", &mut lines);

    lines.push("    m".to_string());
    lines.push("}".to_string());
    lines.join("\n")
}

/// Find the local solilang install path for the template dependency.
fn find_soli_install_path() -> String {
    // When running from the repo, use CARGO_MANIFEST_DIR
    let repo_path = Path::new(env!("CARGO_MANIFEST_DIR"));
    if repo_path.join("Cargo.toml").exists() {
        return repo_path.to_string_lossy().to_string();
    }
    // Fallback: try the installed binary's location
    if let Ok(exe) = env::current_exe() {
        if let Some(dir) = exe.parent() {
            // Check for ../../ (cargo install puts binary in ~/.cargo/bin, project in ~/.cargo/bin/../)
            let candidate = dir.join("..").join("..").join("lib").join("solilang");
            if candidate.join("Cargo.toml").exists() {
                return candidate.to_string_lossy().to_string();
            }
        }
    }
    eprintln!("Error: Could not find solilang crate path for WASM build");
    eprintln!("  Set SOLI_SRC_PATH environment variable to the solilang source directory.");
    process::exit(1);
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

pub fn run_test(
    paths: &[String],
    jobs: Option<usize>,
    coverage_formats: &[String],
    coverage_min: Option<f64>,
    no_coverage: bool,
) {
    test_runner::run_test(paths, jobs, coverage_formats, coverage_min, no_coverage);
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

    if latest_tag == current_version {
        println!(
            "  You are already running the latest version: v{}",
            current_version
        );
        println!();
        return Ok(());
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

    if std::fs::rename(&current_exe, &backup_path).is_err() {
        std::fs::copy(&current_exe, &backup_path)
            .map_err(|e| format!("Failed to backup current binary: {}", e))?;
        std::fs::remove_file(&current_exe)
            .map_err(|e| format!("Failed to remove old binary: {}", e))?;
    }
    if std::fs::rename(&binary_path, &current_exe).is_err() {
        std::fs::copy(&binary_path, &current_exe)
            .map_err(|e| format!("Failed to install new binary: {}", e))?;
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
fn sha256_of_file(path: &Path) -> Result<String, String> {
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
