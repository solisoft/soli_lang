use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use solilang::coverage::data::AggregatedCoverage;
use solilang::coverage::tracker::{clear_global_coverage_tracker, set_global_coverage_tracker};
use solilang::coverage::{CoverageConfig, CoverageReporter, CoverageTracker, OutputFormat};

fn format_duration(duration: Duration) -> String {
    let micros = duration.as_micros();
    if micros < 1000 {
        format!("{}µs", micros)
    } else if micros < 1_000_000 {
        format!("{}ms", (micros + 500) / 1000)
    } else {
        format!("{}.{}s", micros / 1_000_000, (micros % 1_000_000) / 10000)
    }
}

pub fn run_test(
    path: Option<&str>,
    jobs: usize,
    coverage_formats: &[String],
    coverage_min: Option<f64>,
    no_coverage: bool,
) {
    use std::sync::mpsc;

    let test_path = match path {
        Some(p) => PathBuf::from(p),
        None => std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("tests"),
    };

    if !test_path.exists() {
        eprintln!("Error: Test path '{}' does not exist", test_path.display());
        process::exit(1);
    }

    std::env::set_var("APP_ENV", "test");
    let app_dir = if test_path.is_file() {
        test_path
            .parent()
            .and_then(|p| p.parent())
            .unwrap_or_else(|| Path::new("."))
    } else {
        test_path.parent().unwrap_or_else(|| Path::new("."))
    };
    let env_test_path = app_dir.join(".env.test");
    if !env_test_path.exists() {
        eprintln!(
            "Error: .env.test file not found at '{}'. Create one with test database configuration.",
            env_test_path.display()
        );
        process::exit(1);
    }
    solilang::serve::env_loader::load_env_files(app_dir);

    ensure_test_database();
    solilang::interpreter::builtins::model::init_db_config();

    let test_files = if test_path.is_file() {
        vec![test_path.clone()]
    } else {
        collect_test_files(&test_path)
    };

    if test_files.is_empty() {
        println!("No test files found.");
        return;
    }

    let mut model_preamble_files: Vec<(PathBuf, String)> = Vec::new();

    // Test helpers expected to exist by scaffold-generated tests but not shipped
    // as builtins. Defined at Soli level so they can call user lambdas.
    let helpers_src = "fn with_transaction(block) { block() }\n".to_string();
    model_preamble_files.push((PathBuf::from("<test-helpers>"), helpers_src));

    // Load every `.sl` in app/models, app/helpers, app/middleware into the
    // test interpreter. Models define classes used in tests; helpers and
    // middleware define top-level `def` functions that unit tests can call
    // directly (without going through an HTTP request) — e.g.
    // `authorize_admin(req)` or `active_link(path, current)`.
    for sub in ["models", "helpers", "middleware"] {
        let dir = app_dir.join("app").join(sub);
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

    let test_dir = if test_path.is_file() {
        test_path.parent().unwrap_or(&test_path).to_path_buf()
    } else {
        test_path.clone()
    };

    let enable_coverage = !no_coverage;
    // Console is always present; every other accepted name adds its format.
    let output_formats = {
        let mut formats = vec![OutputFormat::Console];
        for name in coverage_formats {
            let variant = match name.as_str() {
                "console" => continue, // already added
                "html" => OutputFormat::Html,
                "json" => OutputFormat::Json,
                "xml" => OutputFormat::Xml,
                other => {
                    eprintln!("Unknown --coverage format '{}'.", other);
                    process::exit(64);
                }
            };
            if !formats.contains(&variant) {
                formats.push(variant);
            }
        }
        formats
    };
    let tracker = if enable_coverage {
        let config = CoverageConfig {
            enabled: true,
            output_dir: PathBuf::from("coverage"),
            formats: output_formats.clone(),
            threshold: coverage_min.or(Some(80.0)),
            exclude_patterns: Vec::new(),
            exclude_lines: Vec::new(),
            show_uncovered: true,
            per_test: false,
            root_dir: Some(app_dir.to_path_buf()),
        };
        let tracker = CoverageTracker::new(config);
        let tracker = Arc::new(Mutex::new(tracker));
        {
            let mut tracker_guard = tracker.lock().unwrap();
            register_app_source_lines(&mut tracker_guard, app_dir);
        }
        set_global_coverage_tracker(tracker.clone());
        Some(tracker)
    } else {
        None
    };

    let num_workers = jobs.max(1);
    println!(
        "Running {} test(s) with {} worker(s)...",
        test_files.len(),
        num_workers
    );
    println!();

    // Start the test server whenever the app has controllers — any test that
    // calls get()/post()/login()/etc needs it, not just files named *integration*.
    let needs_test_server = app_dir.join("app").join("controllers").is_dir();

    struct ChildGuard(Option<std::process::Child>);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            if let Some(mut c) = self.0.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
    }
    let mut test_server_child = ChildGuard(None);
    if needs_test_server {
        println!("Starting test server...");

        let port = {
            let listener =
                std::net::TcpListener::bind("127.0.0.1:0").expect("Failed to bind to random port");
            listener.local_addr().unwrap().port()
        };

        solilang::interpreter::builtins::test_server::start_test_server_on_port(port);

        // Run the test server in a separate process. When it lives in the
        // same process as the test runner, the server's tokio runtime and
        // the test runner's thread-local runtimes deadlock against each other
        // on the shared reqwest HTTP_CLIENT — requests hang even though the
        // OS socket is fine. A subprocess has its own address space and
        // runtime, so HTTP calls always go over a real TCP connection.
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("soli"));
        // Run the test server in dev mode so request handlers execute in the
        // tree-walking interpreter. Class-based controllers (e.g. `class
        // SessionsController extends Controller`) don't currently work through
        // the VM/production path and return JSON `null` instead of rendering.
        let mut cmd = std::process::Command::new(&exe);
        cmd.arg("serve")
            .arg(app_dir)
            .arg("--dev")
            .arg("--port")
            .arg(port.to_string())
            .arg("--workers")
            .arg("4")
            .env("APP_ENV", "test")
            .env(
                "SOLIDB_HOST",
                std::env::var("SOLIDB_HOST")
                    .unwrap_or_else(|_| "http://localhost:6745".to_string()),
            )
            .env(
                "SOLIDB_DATABASE",
                std::env::var("SOLIDB_DATABASE").unwrap_or_else(|_| "test".to_string()),
            )
            .env(
                "SOLIDB_USERNAME",
                std::env::var("SOLIDB_USERNAME").unwrap_or_default(),
            )
            .env(
                "SOLIDB_PASSWORD",
                std::env::var("SOLIDB_PASSWORD").unwrap_or_default(),
            )
            .stdout(
                std::fs::File::create("/tmp/soli_test_server.log")
                    .map(std::process::Stdio::from)
                    .unwrap_or(std::process::Stdio::null()),
            )
            .stderr(
                std::fs::File::options()
                    .append(true)
                    .create(true)
                    .open("/tmp/soli_test_server.log")
                    .map(std::process::Stdio::from)
                    .unwrap_or(std::process::Stdio::null()),
            );
if enable_coverage {
            cmd.env("SOLI_COVERAGE_ENABLED", "1");
        }
        let child = cmd.spawn().expect("Failed to spawn test server subprocess");
        test_server_child.0 = Some(child);

        let base_url = format!("http://127.0.0.1:{}", port);
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        let max_attempts = 50;
        let mut ready = false;
        for _ in 0..max_attempts {
            std::thread::sleep(Duration::from_millis(200));
            if client.get(format!("{}/health", base_url)).send().is_ok() {
                ready = true;
                break;
            }
        }
        if !ready {
            eprintln!("Error: Test server failed to start on port {}", port);
            drop(test_server_child);
            process::exit(1);
        }

        println!("Test server running on port {}", port);
        println!();
        std::io::stdout().flush().unwrap();
    }

    let (tx, rx) = mpsc::channel();

    std::thread::scope(|s| {
        let mut handles = Vec::new();
        let chunk_size = test_files.len().div_ceil(num_workers);
        for chunk in test_files.chunks(chunk_size) {
            let tx = tx.clone();
            let chunk: Vec<PathBuf> = chunk.to_vec();
            let preamble_files = model_preamble_files.clone();
            let tracker_clone = tracker.clone();

            handles.push(s.spawn(move || {
                let mut results: Vec<(PathBuf, bool, String, Duration, i64)> = Vec::new();

                for file in chunk {
                    let start = std::time::Instant::now();
                    let result = fs::read_to_string(&file).map_err(|e| e.to_string());

                    let (passed, error, assertions) = match result {
                        Ok(source) => {
                            let is_integration = file
                                .file_name()
                                .map(|n| n.to_string_lossy().contains("integration"))
                                .unwrap_or(false);
                            let preamble_slice: &[(PathBuf, String)] =
                                if is_integration { &[] } else { &preamble_files };
                            if let Some(ref tracker) = tracker_clone {
                                let tracker_guard = tracker.lock().unwrap();
                                tracker_guard.start_test(file.to_string_lossy().as_ref());
                            }
                            let tracker_for_run = tracker_clone.clone();
                            let panic_result =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    solilang::run_with_path_and_coverage(
                                        &source,
                                        Some(&file),
                                        false,
                                        tracker_for_run.as_ref(),
                                        Some(&file),
                                        preamble_slice,
                                    )
                                }));
                            if let Some(ref tracker) = tracker_clone {
                                let mut tracker_guard = tracker.lock().unwrap();
                                tracker_guard.end_test();
                            }
                            match panic_result {
                                Ok((count, Ok(()))) => (true, String::new(), count),
                                Ok((count, Err(e))) => (false, e.to_string(), count),
                                Err(_) => (
                                    false,
                                    "Test panicked (may require async runtime)".to_string(),
                                    0,
                                ),
                            }
                        }
                        Err(e) => (false, e, 0),
                    };

                    let duration = start.elapsed();
                    results.push((file, passed, error, duration, assertions));
                }

                let _ = tx.send(results);
            }));
        }

        drop(tx);

        for handle in handles {
            handle.join().unwrap();
        }
    });

    let mut all_results: Vec<(PathBuf, bool, String, Duration, i64)> = Vec::new();
    for received in rx {
        all_results.extend(received);
    }

    let mut passed = 0;
    let mut failed = 0;
    let mut total_assertions = 0;

    // Pre-compute display path for every result so we know the longest name
    // and can align the duration/assertion columns across rows, even when
    // some filenames are longer than the default 40-char padding.
    let display_rows: Vec<String> = all_results
        .iter()
        .map(|(path, _, _, _, _)| {
            let relative_to_test_dir = path.strip_prefix(&test_dir).unwrap_or(path);
            let parent_str = relative_to_test_dir
                .parent()
                .and_then(|p| p.to_str())
                .unwrap_or(".");
            let file_name = path.file_name().unwrap_or_default().to_string_lossy();
            if parent_str == "." {
                file_name.to_string()
            } else {
                format!("{}/{}", parent_str, file_name)
            }
        })
        .collect();
    let name_width = display_rows
        .iter()
        .map(|s| s.chars().count())
        .max()
        .unwrap_or(40)
        .max(40);

    let mut current_dir: Option<PathBuf> = None;
    for ((path, passed_test, error, duration, assertions), display_path) in
        all_results.iter().zip(display_rows.iter())
    {
        let parent = path.parent().unwrap_or(path).to_path_buf();
        let relative_to_test_dir = path.strip_prefix(&test_dir).unwrap_or(path);
        let parent_str = relative_to_test_dir
            .parent()
            .and_then(|p| p.to_str())
            .unwrap_or(".");

        if current_dir.as_ref().map(|d| d != &parent).unwrap_or(true) {
            if current_dir.is_some() {
                println!();
            }
            current_dir = Some(parent.clone());
            if parent_str != "." {
                println!("{}", parent_str);
            }
        }

        let pad = name_width.saturating_sub(display_path.chars().count());
        let duration_str = format_duration(*duration);
        total_assertions += *assertions;
        if *passed_test {
            passed += 1;
            println!(
                "  {}{} {:>8} {:>6} ✓",
                display_path,
                " ".repeat(pad),
                duration_str,
                assertions
            );
        } else {
            failed += 1;
            println!(
                "  {}{} {:>8} {:>6} ✗ {}",
                display_path,
                " ".repeat(pad),
                duration_str,
                assertions,
                error
            );
        }
        std::io::stdout().flush().unwrap();
    }

    println!();
    println!("{}", if failed > 0 { "❌ " } else { "✓ " });
    println!(
        "  {} passed, {} failed ({} total)",
        passed,
        failed,
        passed + failed
    );
    println!("  {} assertions", total_assertions);

    if enable_coverage {
        // Fetch coverage from the subprocess test server (controllers,
        // middleware, helpers, routes all execute there — not in this
        // process — so their hits are only visible to the subprocess's
        // global tracker). Merge those hits into the parent tracker so
        // they show up in the combined report. Do this BEFORE killing the
        // child via ChildGuard::drop.
        if needs_test_server {
            if let Some(ref tracker_rc) = tracker {
                if let Some(port) =
                    solilang::interpreter::builtins::test_server::get_test_server_port()
                {
                    let url = format!("http://127.0.0.1:{}/__coverage__", port);
                    if let Ok(resp) = ureq::get(&url).timeout(Duration::from_secs(5)).call() {
                        if let Ok(text) = resp.into_string() {
                            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                                if let Some(files) = json.get("files").and_then(|v| v.as_array()) {
                                    let mut guard = tracker_rc.lock().unwrap();
                                    for f in files {
                                        let path = match f.get("path").and_then(|v| v.as_str()) {
                                            Some(p) => PathBuf::from(p),
                                            None => continue,
                                        };
                                        if let Some(hits) = f.get("hits").and_then(|v| v.as_array())
                                        {
                                            for pair in hits {
                                                if let Some(arr) = pair.as_array() {
                                                    let line = arr
                                                        .first()
                                                        .and_then(|v| v.as_u64())
                                                        .unwrap_or(0)
                                                        as usize;
                                                    let count = arr
                                                        .get(1)
                                                        .and_then(|v| v.as_u64())
                                                        .unwrap_or(0);
                                                    for _ in 0..count {
                                                        guard
                                                            .record_line_hit_to_global(&path, line);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
if let Some(ref tracker_rc) = tracker {
            let coverage = tracker_rc.lock().unwrap().get_aggregated_coverage();

            let app_coverage = AggregatedCoverage {
                file_coverages: coverage
                    .file_coverages
                    .iter()
                    .filter(|(path, _)| {
                        let s = path.to_string_lossy();
                        // Hide synthetic preamble files (e.g. "<test-helpers>")
                        // and any file inside the tests directory, regardless of
                        // whether the path is absolute or relative.
                        !s.starts_with('<') && !s.contains("/tests/") && !s.starts_with("tests/")
                    })
                    .map(|(path, cov)| (path.clone(), cov.clone()))
                    .collect(),
                test_count: coverage.test_count,
                passed_count: coverage.passed_count,
                failed_count: coverage.failed_count,
                pending_count: coverage.pending_count,
            };

            if app_coverage.file_coverages.is_empty() && coverage.total_lines() == 0 {
                let reporter = CoverageReporter::new(CoverageConfig {
                    enabled: true,
                    output_dir: PathBuf::from("coverage"),
                    formats: vec![OutputFormat::Console],
                    threshold: coverage_min.or(Some(80.0)),
                    exclude_patterns: Vec::new(),
                    exclude_lines: Vec::new(),
                    show_uncovered: true,
                    per_test: false,
                    root_dir: Some(app_dir.to_path_buf()),
                });
                println!("\nCoverage: N/A (no source files found in app/, config/, lib/)");
            } else {
                let config = CoverageConfig {
                    enabled: true,
                    output_dir: PathBuf::from("coverage"),
                    formats: output_formats.clone(),
                    threshold: coverage_min.or(Some(80.0)),
                    exclude_patterns: Vec::new(),
                    exclude_lines: Vec::new(),
                    show_uncovered: true,
                    per_test: false,
                    root_dir: Some(app_dir.to_path_buf()),
                };
                let reporter = CoverageReporter::new(config);
                reporter.generate_reports(&app_coverage);
                for fmt in &output_formats {
                    match fmt {
                        OutputFormat::Html => {
                            println!("  HTML coverage report: coverage/index.html");
                        }
                        OutputFormat::Json => {
                            println!("  JSON coverage report: coverage/coverage.json");
                        }
                        OutputFormat::Xml => {
                            println!("  Cobertura XML report: coverage/cobertura.xml");
                        }
                        OutputFormat::Console => {}
                    }
                }

                if let Some(min) = coverage_min {
                    if app_coverage.total_line_coverage_percent() < min {
                        eprintln!(
                            "\n❌ Coverage {:.1}% is below threshold {:.0}%",
                            app_coverage.total_line_coverage_percent(),
                            min
                        );
                        process::exit(1);
                    }
                }
            }
        }
    }

    if failed > 0 {
        process::exit(1);
    }

    if enable_coverage {
        clear_global_coverage_tracker();
    }

    println!();
}

fn ensure_test_database() {
    let host = std::env::var("SOLIDB_HOST").unwrap_or_else(|_| "http://localhost:6745".to_string());
    let database = std::env::var("SOLIDB_DATABASE").unwrap_or_else(|_| "default".to_string());

    if database == "default" {
        return;
    }

    let auth_header = match (
        std::env::var("SOLIDB_USERNAME"),
        std::env::var("SOLIDB_PASSWORD"),
    ) {
        (Ok(user), Ok(pass)) => {
            use base64::Engine;
            let encoded =
                base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", user, pass));
            Some(format!("Basic {}", encoded))
        }
        _ => None,
    };

    let drop_url = format!("{}/_api/database/{}", host, database);
    let mut drop_req = ureq::delete(&drop_url);
    if let Some(ref auth) = auth_header {
        drop_req = drop_req.set("Authorization", auth);
    }
    let _ = drop_req.call();

    let create_url = format!("{}/_api/database", host);
    let payload = format!(r#"{{"name":"{}"}}"#, database);
    let mut create_req = ureq::post(&create_url).set("Content-Type", "application/json");
    if let Some(ref auth) = auth_header {
        create_req = create_req.set("Authorization", auth);
    }
    let _ = create_req.send_string(&payload);
}

pub fn collect_test_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().map(|e| e == "sl").unwrap_or(false) {
                files.push(path);
            } else if path.is_dir() {
                files.extend(collect_test_files(&path));
            }
        }
    }

    files
}

fn register_app_source_lines(tracker: &mut CoverageTracker, app_dir: &Path) {
    let source_dirs = [
        app_dir.join("app"),
        app_dir.join("config"),
        app_dir.join("lib"),
    ];

    for source_dir in &source_dirs {
        if source_dir.is_dir() {
            collect_and_register_sources(tracker, source_dir);
        }
    }
}

fn collect_and_register_sources(tracker: &mut CoverageTracker, dir: &Path) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_and_register_sources(tracker, &path);
            } else if path.extension().map(|e| e == "sl").unwrap_or(false) {
                if let Ok(source) = fs::read_to_string(&path) {
                    tracker.register_executable_lines_from_source(&path, &source);
                }
            }
        }
    }
}
