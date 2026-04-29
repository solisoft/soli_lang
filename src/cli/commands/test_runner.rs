use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use solilang::coverage::data::AggregatedCoverage;
use solilang::coverage::tracker::{clear_global_coverage_tracker, set_global_coverage_tracker};
use solilang::coverage::{CoverageConfig, CoverageReporter, CoverageTracker, OutputFormat};

struct ProgressState {
    passed: usize,
    failed: usize,
    total_assertions: i64,
}

fn draw_progress_bar(state: &ProgressState, total_files: usize, icon: &str) {
    let done = state.passed + state.failed;
    let bar_len = 30;
    let filled = if total_files == 0 {
        0
    } else {
        ((bar_len as f64) * (done as f64) / (total_files as f64)) as usize
    };
    let filled = filled.min(bar_len);
    let empty = bar_len - filled;
    let color = if state.failed > 0 { "31" } else { "32" };
    eprint!(
        "\r\x1b[{color}m\x1b[1m[\x1b[{color}m{}\x1b[0m\x1b[90m{}\x1b[0m\x1b[{color}m\x1b[1m] {} {}/{} \x1b[90m{} assertions\x1b[0m\x1b[K",
        "█".repeat(filled),
        "░".repeat(empty),
        icon,
        done,
        total_files,
        state.total_assertions,
    );
    let _ = io::stderr().flush();
}

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

    // Load every `.sl` in app/models, app/services, app/helpers,
    // app/middleware into the test interpreter. Models and services define
    // classes used in tests; helpers and middleware define top-level `def`
    // functions that unit tests can call directly (without going through
    // an HTTP request) — e.g. `authorize_admin(req)` or
    // `active_link(path, current)`.
    for sub in ["models", "services", "helpers", "middleware"] {
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

    // Start one test server per worker (each with its own DB) whenever the
    // app has controllers — any test that calls get()/post()/login()/etc
    // needs the server, not just files named *integration*. Per-worker
    // isolation: tests on worker `i` write to `{base}_w{i}` and hit
    // `127.0.0.1:{port_i}`, so concurrent workers don't trample each
    // other's rows or sessions.
    let needs_test_server = app_dir.join("app").join("controllers").is_dir();

    let worker_databases = worker_database_names(num_workers);
    ensure_test_databases(&worker_databases);

    #[derive(Clone)]
    struct WorkerEnv {
        port: Option<u16>,
        database: String,
    }

    struct ChildGuard(Vec<std::process::Child>);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            for mut c in self.0.drain(..) {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
    }
    let mut test_server_children = ChildGuard(Vec::new());
    let mut worker_envs: Vec<WorkerEnv> = worker_databases
        .iter()
        .map(|db| WorkerEnv {
            port: None,
            database: db.clone(),
        })
        .collect();

    if needs_test_server {
        println!("Starting {} test server(s)...", num_workers);

        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("soli"));
        let solidb_host =
            std::env::var("SOLIDB_HOST").unwrap_or_else(|_| "http://localhost:6745".to_string());
        let solidb_user = std::env::var("SOLIDB_USERNAME").unwrap_or_default();
        let solidb_pass = std::env::var("SOLIDB_PASSWORD").unwrap_or_default();

        for (i, env) in worker_envs.iter_mut().enumerate() {
            let port = {
                let listener = std::net::TcpListener::bind("127.0.0.1:0")
                    .expect("Failed to bind to random port");
                listener.local_addr().unwrap().port()
            };
            env.port = Some(port);

            // Run the server in dev mode so handlers execute via the
            // tree-walking interpreter. Class-based controllers don't
            // currently work through the VM path. Subprocess isolation
            // also avoids the reqwest+block_on cross-runtime deadlock
            // that bites when server and runner share a process.
            let log_path = format!("/tmp/soli_test_server_w{}.log", i);
            let mut cmd = std::process::Command::new(&exe);
            cmd.arg("serve")
                .arg(app_dir)
                .arg("--dev")
                .arg("--port")
                .arg(port.to_string())
                .arg("--workers")
                .arg("2")
                .env("APP_ENV", "test")
                .env("SOLIDB_HOST", &solidb_host)
                .env("SOLIDB_DATABASE", &env.database)
                .env("SOLIDB_USERNAME", &solidb_user)
                .env("SOLIDB_PASSWORD", &solidb_pass)
                // Pin SOLIDB_DATABASE so the server's `.env.test` reload
                // (override_existing=true) doesn't clobber the per-worker
                // value we just set.
                .env("SOLI_PROTECT_ENV", "SOLIDB_DATABASE")
                .stdout(
                    std::fs::File::create(&log_path)
                        .map(std::process::Stdio::from)
                        .unwrap_or(std::process::Stdio::null()),
                )
                .stderr(
                    std::fs::File::options()
                        .append(true)
                        .create(true)
                        .open(&log_path)
                        .map(std::process::Stdio::from)
                        .unwrap_or(std::process::Stdio::null()),
                );
            if enable_coverage {
                cmd.env("SOLI_COVERAGE_ENABLED", "1");
            }
            let child = cmd.spawn().expect("Failed to spawn test server subprocess");
            test_server_children.0.push(child);
        }

        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        // Probe every server concurrently (one thread per worker port). Each
        // probe checks immediately, then sleeps 200ms between retries — the
        // old code paid 200ms × N just to start probing.
        let failed_port = std::thread::scope(|s| {
            let mut handles = Vec::new();
            for env in &worker_envs {
                let port = env.port.unwrap();
                let client = &client;
                handles.push(s.spawn(move || {
                    let base_url = format!("http://127.0.0.1:{}", port);
                    for attempt in 0..50 {
                        if attempt > 0 {
                            std::thread::sleep(Duration::from_millis(200));
                        }
                        if client.get(format!("{}/health", base_url)).send().is_ok() {
                            return None;
                        }
                    }
                    Some(port)
                }));
            }
            handles
                .into_iter()
                .find_map(|h| h.join().unwrap())
        });
        if let Some(port) = failed_port {
            eprintln!("Error: Test server failed to start on port {}", port);
            drop(test_server_children);
            process::exit(1);
        }

        // Mark the global "test server is running" flag so that code paths
        // reading the old global atomic (e.g. coverage code on the main
        // thread before per-thread overrides exist) see a sane port.
        solilang::interpreter::builtins::test_server::start_test_server_on_port(
            worker_envs[0].port.unwrap(),
        );

        let ports: Vec<String> = worker_envs
            .iter()
            .map(|e| e.port.unwrap().to_string())
            .collect();
        println!("Test servers running on ports {}", ports.join(", "));
        println!();
        std::io::stdout().flush().unwrap();
    }

    let total_files = test_files.len();
    let progress = Arc::new(Mutex::new(ProgressState {
        passed: 0,
        failed: 0,
        total_assertions: 0,
    }));
    type TestResult = (PathBuf, bool, String, Duration, i64);
    let all_results_shared: Arc<Mutex<Vec<TestResult>>> = Arc::new(Mutex::new(Vec::new()));

    let stop_animation = Arc::new(AtomicBool::new(false));
    let animate = std::io::stderr().is_terminal();

    let anim_handle = if animate {
        let progress = progress.clone();
        let stop = stop_animation.clone();
        Some(std::thread::spawn(move || {
            let spinner = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
            let mut frame = 0usize;
            while !stop.load(Ordering::Relaxed) {
                let snapshot = {
                    let p = progress.lock().unwrap();
                    ProgressState {
                        passed: p.passed,
                        failed: p.failed,
                        total_assertions: p.total_assertions,
                    }
                };
                let icon = spinner[frame].to_string();
                draw_progress_bar(&snapshot, total_files, &icon);
                std::thread::sleep(Duration::from_millis(80));
                frame = (frame + 1) % spinner.len();
            }
        }))
    } else {
        None
    };

    // Share a single multi-threaded tokio runtime across all worker threads.
    // Without this, each worker uses its own thread-local current-thread
    // runtime to drive the *shared* reqwest HTTP_CLIENT — and connections
    // in the pool get bound to whichever thread's runtime first opened
    // them. When another worker reuses such a connection through its own
    // runtime, the I/O driver isn't running and the future deadlocks.
    // Same root cause as the test-server subprocess workaround above; that
    // workaround alone doesn't help the runner's *own* parallel workers.
    let shared_rt = if num_workers > 1 {
        Some(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(num_workers)
                .enable_all()
                .build()
                .expect("Failed to build shared tokio runtime for test workers"),
        )
    } else {
        None
    };
    let shared_rt_handle = shared_rt.as_ref().map(|rt| rt.handle().clone());

    // Shared work queue, ordered largest-first (LPT scheduling). Workers pop
    // one file at a time so a heavy file can't trap a chunk while peers
    // idle — replaces the old static `test_files.chunks(N)` partition.
    let work_queue: Arc<Mutex<Vec<PathBuf>>> = {
        let mut files = test_files.clone();
        files.sort_by_key(|p| {
            std::cmp::Reverse(fs::metadata(p).map(|m| m.len()).unwrap_or(0))
        });
        // Workers pop from the end, so reverse so the largest is popped first.
        files.reverse();
        Arc::new(Mutex::new(files))
    };

    let suite_start = std::time::Instant::now();
    std::thread::scope(|s| {
        let mut handles = Vec::new();
        for env in worker_envs.iter().take(num_workers) {
            let queue = work_queue.clone();
            let preamble_files = model_preamble_files.clone();
            let tracker_clone = tracker.clone();
            let progress = progress.clone();
            let all_results_shared = all_results_shared.clone();
            let rt_handle = shared_rt_handle.clone();
            let env = env.clone();

            handles.push(s.spawn(move || {
                if let Some(handle) = rt_handle {
                    solilang::serve::set_tokio_handle(handle);
                }
                solilang::interpreter::builtins::model::db_config::set_database_override(
                    env.database.clone(),
                );
                if let Some(port) = env.port {
                    solilang::interpreter::builtins::test_server::set_thread_test_server_port(port);
                }
                loop {
                    let file = {
                        let mut q = queue.lock().unwrap();
                        match q.pop() {
                            Some(f) => f,
                            None => break,
                        }
                    };
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
                            // Per-thread stdout capture — fd-level redirection
                            // (e.g. `gag::BufferRedirect`) deadlocks under
                            // `--jobs > 1` because every worker fights over
                            // the same process stdout fd and the OS pipe
                            // backing the redirect fills.
                            let print_guard =
                                solilang::interpreter::builtins::StdoutCaptureGuard::start();
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
                            let _ = print_guard.finish();
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

                    {
                        let mut p = progress.lock().unwrap();
                        if passed {
                            p.passed += 1;
                        } else {
                            p.failed += 1;
                        }
                        p.total_assertions += assertions;
                    }

                    all_results_shared
                        .lock()
                        .unwrap()
                        .push((file, passed, error, duration, assertions));
                }
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }
    });
    let suite_duration = suite_start.elapsed();

    stop_animation.store(true, Ordering::Relaxed);
    if let Some(handle) = anim_handle {
        handle.join().unwrap();
    }

    let (passed, failed, total_assertions_val) = {
        let p = progress.lock().unwrap();
        (p.passed, p.failed, p.total_assertions)
    };
    let final_state = ProgressState {
        passed,
        failed,
        total_assertions: total_assertions_val,
    };
    let final_icon = if failed > 0 { "✗" } else { "✓" };
    draw_progress_bar(&final_state, total_files, final_icon);
    eprintln!();

    let mut all_results: Vec<TestResult> = match Arc::try_unwrap(all_results_shared) {
        Ok(mutex) => mutex.into_inner().unwrap(),
        Err(arc) => arc.lock().unwrap().clone(),
    };
    all_results.sort_by(|a, b| a.0.cmp(&b.0));

    println!();
    println!();

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
        if *passed_test {
            println!(
                "  {}{} {:>8} {:>6} ✓",
                display_path,
                " ".repeat(pad),
                duration_str,
                assertions
            );
        } else {
            println!(
                "  {}{} {:>8} {:>6} ✗",
                display_path,
                " ".repeat(pad),
                duration_str,
                assertions
            );

            let error_msg = error.trim();
            if error_msg.starts_with("Runtime error:") {
                let body = error_msg.strip_prefix("Runtime error:").unwrap().trim();
                if body.starts_with("Test failed:") {
                    let test_info = body.strip_prefix("Test failed:").unwrap().trim();
                    println!("  ┌─ ✗ {}", test_info);
                    println!("  └─");
                } else if body.contains("tests failed:") {
                    let parts: Vec<&str> = body.splitn(2, "tests failed:").collect();
                    let count = parts[0].trim();
                    let failures = parts[1].trim();
                    println!(
                        "  ┌─ {} test failure{}",
                        count,
                        if count == "1" { "" } else { "s" }
                    );
                    for line in failures.lines() {
                        let line = line.trim();
                        if line.starts_with("- ") {
                            if let Some(rest) = line.strip_prefix("- ") {
                                if let Some(at_pos) = rest.find(": ") {
                                    let test_name = &rest[..at_pos];
                                    let error_detail = &rest[at_pos + 2..];
                                    println!("  │");
                                    println!("  │  ✗ {}", test_name);
                                    println!("  │     → {}", error_detail);
                                } else {
                                    println!("  │  • {}", rest);
                                }
                            }
                        } else {
                            println!("  │ {}", line);
                        }
                    }
                    println!("  └─");
                } else {
                    println!("  ┌─ {}", body);
                    println!("  └─");
                }
            } else {
                let first_line = error_msg.lines().next().unwrap_or(error_msg);
                println!("  ┌─ ✗ {}", first_line);
                for line in error_msg.lines().skip(1) {
                    println!("  │ {}", line);
                }
                println!("  └─");
            }
            println!();
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
    println!("  {} assertions", total_assertions_val);
    println!("  Time: {}", format_duration(suite_duration));

    if enable_coverage {
        // Fetch coverage from the subprocess test server (controllers,
        // middleware, helpers, routes all execute there — not in this
        // process — so their hits are only visible to the subprocess's
        // global tracker). Merge those hits into the parent tracker so
        // they show up in the combined report. Do this BEFORE killing the
        // child via ChildGuard::drop.
        if needs_test_server {
            if let Some(ref tracker_rc) = tracker {
                for env in &worker_envs {
                    let Some(port) = env.port else { continue };
                    let url = format!("http://127.0.0.1:{}/__coverage__", port);
                    let Ok(resp) = ureq::get(&url).timeout(Duration::from_secs(5)).call() else {
                        continue;
                    };
                    let Ok(text) = resp.into_string() else {
                        continue;
                    };
                    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
                        continue;
                    };
                    let Some(files) = json.get("files").and_then(|v| v.as_array()) else {
                        continue;
                    };
                    let mut guard = tracker_rc.lock().unwrap();
                    for f in files {
                        let path = match f.get("path").and_then(|v| v.as_str()) {
                            Some(p) => PathBuf::from(p),
                            None => continue,
                        };
                        if let Some(hits) = f.get("hits").and_then(|v| v.as_array()) {
                            for pair in hits {
                                if let Some(arr) = pair.as_array() {
                                    let line =
                                        arr.first().and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                                    let count = arr.get(1).and_then(|v| v.as_u64()).unwrap_or(0);
                                    for _ in 0..count {
                                        guard.record_line_hit_to_global(&path, line);
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

fn base_test_database() -> String {
    std::env::var("SOLIDB_DATABASE").unwrap_or_else(|_| "default".to_string())
}

/// Returns one DB name per worker, derived from `SOLIDB_DATABASE`. With a
/// single worker the base name is used as-is; with multiple workers each
/// worker gets `{base}_w{i}` so parallel tests don't share rows.
fn worker_database_names(num_workers: usize) -> Vec<String> {
    let base = base_test_database();
    if num_workers <= 1 {
        return vec![base];
    }
    (0..num_workers)
        .map(|i| format!("{}_w{}", base, i))
        .collect()
}

fn ensure_test_databases(db_names: &[String]) {
    let host = std::env::var("SOLIDB_HOST").unwrap_or_else(|_| "http://localhost:6745".to_string());
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

    // Drop+create each worker DB in parallel — independent operations on
    // distinct names, no shared state. Saves ~Nx round-trip latency before
    // any test runs.
    std::thread::scope(|s| {
        for database in db_names {
            if database == "default" {
                continue;
            }
            let host = &host;
            let auth_header = &auth_header;
            s.spawn(move || {
                let drop_url = format!("{}/_api/database/{}", host, database);
                let mut drop_req = ureq::delete(&drop_url);
                if let Some(auth) = auth_header {
                    drop_req = drop_req.set("Authorization", auth);
                }
                let _ = drop_req.call();

                let create_url = format!("{}/_api/database", host);
                let payload = format!(r#"{{"name":"{}"}}"#, database);
                let mut create_req =
                    ureq::post(&create_url).set("Content-Type", "application/json");
                if let Some(auth) = auth_header {
                    create_req = create_req.set("Authorization", auth);
                }
                let _ = create_req.send_string(&payload);
            });
        }
    });
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
