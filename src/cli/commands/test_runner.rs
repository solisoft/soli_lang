use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::{Arc, Mutex};
use std::time::Duration;

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
    coverage: bool,
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

    let models_dir = app_dir.join("app").join("models");
    let model_preamble = if models_dir.is_dir() {
        let mut preamble = String::new();
        if let Ok(entries) = fs::read_dir(&models_dir) {
            let mut model_files: Vec<_> = entries
                .flatten()
                .filter(|e| e.path().extension().is_some_and(|ext| ext == "sl"))
                .collect();
            model_files.sort_by_key(|e| e.path());
            for entry in model_files {
                if let Ok(content) = fs::read_to_string(entry.path()) {
                    preamble.push_str(&content);
                    preamble.push('\n');
                }
            }
        }
        preamble
    } else {
        String::new()
    };

    let test_dir = if test_path.is_file() {
        test_path.parent().unwrap_or(&test_path).to_path_buf()
    } else {
        test_path.clone()
    };

    let enable_coverage = coverage && !no_coverage;
    let tracker = if enable_coverage {
        let config = CoverageConfig {
            enabled: true,
            output_dir: PathBuf::from("coverage"),
            formats: vec![OutputFormat::Console],
            threshold: coverage_min.or(Some(80.0)),
            exclude_patterns: Vec::new(),
            exclude_lines: Vec::new(),
            show_uncovered: true,
            per_test: false,
        };
        let tracker = CoverageTracker::new(config);
        let tracker = Arc::new(Mutex::new(tracker));
        {
            let mut tracker_guard = tracker.lock().unwrap();
            register_app_source_lines(&mut tracker_guard, app_dir);
            collect_and_register_sources(&mut tracker_guard, &test_dir);
        }
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

    let needs_test_server = test_files.iter().any(|f| {
        f.file_name()
            .map(|n| n.to_string_lossy().contains("integration"))
            .unwrap_or(false)
    });

    if needs_test_server {
        println!("Starting test server...");

        let port = {
            let listener =
                std::net::TcpListener::bind("127.0.0.1:0").expect("Failed to bind to random port");
            listener.local_addr().unwrap().port()
        };

        solilang::interpreter::builtins::test_server::start_test_server_on_port(port);

        let app_dir_owned = app_dir.to_path_buf();
        std::thread::spawn(move || {
            if let Err(e) =
                solilang::serve::serve_folder_with_options(&app_dir_owned, port, false, 1)
            {
                eprintln!("Test server error: {}", e);
            }
        });

        let base_url = format!("http://127.0.0.1:{}", port);
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        let max_attempts = 50;
        let mut ready = false;
        for _ in 0..max_attempts {
            std::thread::sleep(Duration::from_millis(100));
            if client.get(format!("{}/health", base_url)).send().is_ok() {
                ready = true;
                break;
            }
        }
        if !ready {
            eprintln!("Error: Test server failed to start on port {}", port);
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
            let preamble = model_preamble.clone();
            let tracker_clone = tracker.clone();

            handles.push(s.spawn(move || {
                let mut results: Vec<(PathBuf, bool, String, Duration, i64)> = Vec::new();

                for file in chunk {
                    let start = std::time::Instant::now();
                    let result = fs::read_to_string(&file).map_err(|e| e.to_string());

                    let (passed, error, assertions) = match result {
                        Ok(source) => {
                            let full_source = if !preamble.is_empty()
                                && !file
                                    .file_name()
                                    .map(|n| n.to_string_lossy().contains("integration"))
                                    .unwrap_or(false)
                            {
                                format!("{}\n{}", preamble, source)
                            } else {
                                source
                            };
                            if let Some(ref tracker) = tracker_clone {
                                let tracker_guard = tracker.lock().unwrap();
                                tracker_guard.start_test(file.to_string_lossy().as_ref());
                            }
                            let tracker_for_run = tracker_clone.clone();
                            let panic_result =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    solilang::run_with_path_and_coverage(
                                        &full_source,
                                        Some(&file),
                                        false,
                                        tracker_for_run.as_ref(),
                                        Some(&file),
                                    )
                                }));
                            if let Some(ref tracker) = tracker_clone {
                                let mut tracker_guard = tracker.lock().unwrap();
                                tracker_guard.end_test();
                            }
                            match panic_result {
                                Ok(Ok(count)) => (true, String::new(), count),
                                Ok(Err(e)) => (false, e.to_string(), 0),
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

    let mut current_dir: Option<PathBuf> = None;
    for (path, passed_test, error, duration, assertions) in &all_results {
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

        let file_name = path.file_name().unwrap_or_default().to_string_lossy();
        let display_path = if parent_str == "." {
            file_name.to_string()
        } else {
            format!("{}/{}", parent_str, file_name)
        };

        let duration_str = format_duration(*duration);
        if *passed_test {
            passed += 1;
            total_assertions += *assertions;
            println!(
                "  {:40} {:>8} {:>6} ✓",
                display_path, duration_str, assertions
            );
        } else {
            failed += 1;
            println!(
                "  {:40} {:>8} {:>6} ✗ {}",
                display_path, duration_str, assertions, error
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
        if let Some(ref tracker_rc) = tracker {
            let coverage = tracker_rc.lock().unwrap().get_aggregated_coverage();
            let config = CoverageConfig {
                enabled: true,
                output_dir: PathBuf::from("coverage"),
                formats: vec![OutputFormat::Console],
                threshold: coverage_min.or(Some(80.0)),
                exclude_patterns: Vec::new(),
                exclude_lines: Vec::new(),
                show_uncovered: true,
                per_test: false,
            };
            let reporter = CoverageReporter::new(config);
            reporter.generate_reports(&coverage);

            if let Some(min) = coverage_min {
                if coverage.total_line_coverage_percent() < min {
                    eprintln!(
                        "\n❌ Coverage {:.1}% is below threshold {:.0}%",
                        coverage.total_line_coverage_percent(),
                        min
                    );
                    process::exit(1);
                }
            }
        }
    }

    if failed > 0 {
        process::exit(1);
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
