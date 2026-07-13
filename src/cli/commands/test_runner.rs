use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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

#[derive(Clone)]
struct WorkerSlot {
    worker_id: usize,
    /// Basename (no extension) of the file the worker is currently running.
    /// `None` when the worker is between files (idle / done).
    current_file: Option<String>,
    /// When the current file started running. Used for live elapsed display.
    started_at: Option<std::time::Instant>,
    files_done: usize,
    files_failed: usize,
    /// Last terminal status: '✓', '✗', or ' ' (no file finished yet).
    last_status: char,
}

impl WorkerSlot {
    fn new(worker_id: usize) -> Self {
        Self {
            worker_id,
            current_file: None,
            started_at: None,
            files_done: 0,
            files_failed: 0,
            last_status: ' ',
        }
    }
}

fn truncate_chars(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let count = s.chars().count();
    if count <= max {
        return s.to_string();
    }
    let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
    t.push('…');
    t
}

fn pad_chars(s: &str, width: usize) -> String {
    let count = s.chars().count();
    if count >= width {
        return s.to_string();
    }
    let mut t = String::with_capacity(s.len() + (width - count));
    t.push_str(s);
    for _ in count..width {
        t.push(' ');
    }
    t
}

/// One full-width row for a single worker: label, per-worker progress bar,
/// status icon, current file (truncated to fit), elapsed, and `done/share`
/// counter. The "share" denominator is an even split of the total file
/// count across workers — workers don't have a fixed quota (the queue is
/// shared LPT) so this is an estimate, capped at 100%.
fn render_worker_row(
    slot: &WorkerSlot,
    total_files: usize,
    num_workers: usize,
    term_width: usize,
    spinner_char: char,
    all_done: bool,
) -> String {
    let label = format!("W{:<2}", slot.worker_id); // 3 visible chars

    let icon = if slot.current_file.is_some() {
        spinner_char
    } else if slot.last_status != ' ' {
        slot.last_status
    } else {
        '·'
    };
    let icon_color = if slot.files_failed > 0 || slot.last_status == '✗' {
        31 // red
    } else if slot.current_file.is_some() {
        36 // cyan (running)
    } else if slot.last_status == '✓' {
        32 // green
    } else {
        90 // dim
    };

    let elapsed_str = match slot.started_at {
        Some(t) => {
            let secs = t.elapsed().as_secs_f64();
            if secs < 10.0 {
                format!("{:>4.1}s", secs)
            } else if secs < 100.0 {
                format!("{:>4}s", secs as u64)
            } else {
                ">99s".to_string()
            }
        }
        None => "     ".to_string(),
    };

    let expected_share = total_files.div_ceil(num_workers.max(1)).max(1);
    let bar_len: usize = 14;
    // Once the suite's done, every worker's bar shows 100% regardless of
    // how many files they actually pulled — LPT scheduling produces uneven
    // splits, so a fast worker may have processed more than its even share
    // while a slow one processed less. The aggregate bar at the bottom is
    // the authoritative total.
    let filled = if all_done {
        bar_len
    } else {
        let raw = ((bar_len as f64) * (slot.files_done as f64) / (expected_share as f64)) as usize;
        raw.min(bar_len)
    };
    let empty = bar_len - filled;
    let bar_color = if slot.files_failed > 0 { "31" } else { "32" };

    let counter = slot.files_done.to_string();
    let counter_visible = counter.chars().count();

    let file_text = slot.current_file.clone().unwrap_or_else(|| {
        if slot.files_done > 0 || slot.files_failed > 0 {
            "idle".to_string()
        } else {
            "—".to_string()
        }
    });

    // Visible widths used: 1(lead) + 3(label) + 1 + 1([) + bar_len + 1(])
    //                     + 1 + 1(icon) + 1 + file_w + 1 + 5(elapsed)
    //                     + 1 + counter + 1(trail) = 17 + bar_len + counter + file_w
    let fixed = 17 + bar_len + counter_visible;
    let file_w = term_width.saturating_sub(1).saturating_sub(fixed).max(4);

    let file_truncated = truncate_chars(&file_text, file_w);
    let file_padded = pad_chars(&file_truncated, file_w);

    format!(
        " {label} \x1b[{bar_color}m[\x1b[{bar_color}m{bar_filled}\x1b[0m\x1b[90m{bar_empty}\x1b[0m\x1b[{bar_color}m]\x1b[0m \x1b[{icon_color}m{icon}\x1b[0m {file_padded} \x1b[90m{elapsed_str}\x1b[0m \x1b[{bar_color}m{counter}\x1b[0m ",
        bar_filled = "█".repeat(filled),
        bar_empty = "░".repeat(empty),
    )
}

fn render_progress_bar(state: &ProgressState, total_files: usize, icon: &str) -> String {
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
    format!(
        "\x1b[{color}m\x1b[1m[\x1b[{color}m{}\x1b[0m\x1b[90m{}\x1b[0m\x1b[{color}m\x1b[1m] {} {}/{} \x1b[90m{} assertions\x1b[0m",
        "█".repeat(filled),
        "░".repeat(empty),
        icon,
        done,
        total_files,
        state.total_assertions,
    )
}

/// Redraw one row per worker (with its own progress bar) plus the aggregate
/// bar at the bottom, in place. Single-column layout — no terminal-width
/// arithmetic, no auto-wrap traps. Returns the number of newlines emitted
/// so the next call can rewind by exactly that count.
fn redraw_grid(
    slots: &[WorkerSlot],
    state: &ProgressState,
    total_files: usize,
    spinner_char: char,
    last_lines: usize,
) -> usize {
    use std::fmt::Write as _;

    let term_width = crossterm::terminal::size()
        .map(|(c, _)| c as usize)
        .unwrap_or(80);
    let num_workers = slots.len();
    let all_done = total_files > 0 && state.passed + state.failed >= total_files;

    let mut buf = String::new();
    if last_lines > 0 {
        // Rewind to col 1 of the line `last_lines` above (CPL = Cursor
        // Previous Line), then erase everything from there to end of
        // screen. The `\x1b[J` makes us robust against any extra newlines
        // a test worker might have leaked to stderr — without it, one
        // off-count line causes the display to drift each tick and stack.
        write!(buf, "\x1b[{last_lines}F\x1b[J").unwrap();
    }

    for slot in slots {
        buf.push_str(&render_worker_row(
            slot,
            total_files,
            num_workers,
            term_width,
            spinner_char,
            all_done,
        ));
        buf.push_str("\x1b[K\n");
    }

    // Blank separator line between the per-worker rows and the aggregate.
    buf.push_str("\x1b[K\n");

    buf.push_str(&render_progress_bar(
        state,
        total_files,
        &spinner_char.to_string(),
    ));
    buf.push_str("\x1b[K");

    eprint!("{buf}");
    let _ = io::stderr().flush();

    // Newlines just emitted: one per worker row + the blank separator.
    // The aggregate bar leaves the cursor on the same line it was drawn on
    // (no trailing `\n`), so the count matches the number of `\n`s written.
    num_workers + 1
}

/// Compute the app/project root for a given test invocation.
///
/// `test_path` is what the user passed (or the implicit `tests/` default);
/// `is_file` is whether it points at a single spec rather than a directory.
/// File specs need to walk up two parents (`tests/foo_spec.sl` → `.`),
/// directory specs need one (`tests/` → `.`).
///
/// `Path::parent()` returns `Some("")` rather than `None` when there's
/// nothing above the current component (e.g. `Path::new("tests")`'s parent
/// is the empty path). The previous `unwrap_or_else(|| Path::new("."))`
/// only fired on `None`, so a relative spec like `tests/foo_spec.sl` left
/// `app_dir` as `""` and downstream `soli serve ""` could never serve
/// `/health` — the test runner then sat in its 200×50ms probe loop and
/// looked like it hung. This helper treats empty paths the same as `None`.
fn resolve_app_dir(test_path: &Path, is_file: bool) -> PathBuf {
    let parent_chain = if is_file {
        test_path.parent().and_then(|p| p.parent())
    } else {
        test_path.parent()
    };
    parent_chain
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
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
    let test_paths: Vec<PathBuf> = if paths.is_empty() {
        vec![std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("tests")]
    } else {
        paths.iter().map(PathBuf::from).collect()
    };

    for p in &test_paths {
        if !p.exists() {
            eprintln!("Error: Test path '{}' does not exist", p.display());
            process::exit(1);
        }
    }

    // The first target anchors app_dir resolution and the display root; we
    // assume all targets live within the same Soli project.
    let test_path = test_paths[0].clone();

    std::env::set_var("APP_ENV", "test");

    // SEC-017: tell the SSRF blocklist this process is a test runner so
    // loopback/private addresses are reachable for spec fixtures. The
    // flag is process-local (an `AtomicBool` in `http_class.rs`) — env
    // vars are explicitly NOT trusted for this decision so a production
    // deployment that happens to have `APP_ENV=test` set can't lose
    // the guardrail. Children spawned below get a separate signal via
    // `SOLI_INTERNAL_TEST_RUNNER` which `main.rs` translates into the
    // same in-process flag.
    solilang::interpreter::builtins::http_class::enable_ssrf_test_mode();

    // `--fail-on-n1`: arm the process-local guard so every request spec that
    // triggers an N+1 fails without a per-test `assert_no_n_plus_one`. The
    // check lives at the response-building choke point in `request_helpers`;
    // test files run in worker threads of this same runner process, so a
    // process-global flag reaches them (same shape as the SSRF flag above).
    if fail_on_n1 {
        solilang::interpreter::builtins::request_helpers::enable_fail_on_n1();
    }
    let app_dir = resolve_app_dir(&test_path, test_path.is_file());

    if let Err(msg) = solilang::module::enforce_min_soli_version(&app_dir) {
        eprintln!("{}", msg);
        process::exit(1);
    }

    let env_test_path = app_dir.join(".env.test");
    if !env_test_path.exists() {
        eprintln!(
            "Error: .env.test file not found at '{}'. Create one with test database configuration.",
            env_test_path.display()
        );
        process::exit(1);
    }
    solilang::serve::env_loader::load_env_files(&app_dir);

    solilang::interpreter::builtins::model::init_db_config();

    let mut test_files: Vec<PathBuf> = Vec::new();
    for p in &test_paths {
        if p.is_file() {
            test_files.push(p.clone());
        } else {
            test_files.extend(collect_test_files(p));
        }
    }
    // De-duplicate in case overlapping paths were passed (e.g. `tests/` and
    // `tests/foo_spec.sl`). Preserve discovery order.
    {
        let mut seen = std::collections::HashSet::new();
        test_files.retain(|p| seen.insert(p.clone()));
    }

    if test_files.is_empty() {
        println!("No test files found.");
        return;
    }

    let mut model_preamble_files: Vec<(PathBuf, String)> = Vec::new();

    // Load every `.sl` in app/models, app/policies, app/services, app/helpers,
    // app/middleware, app/jobs into the test interpreter. Models and services
    // define classes used in tests; helpers and middleware define top-level
    // `def` functions that unit tests can call directly (without going through
    // an HTTP request) — e.g. `authorize_admin(req)` or
    // `active_link(path, current)`. Jobs are also classes that specs invoke
    // directly (`EmailJob.perform(...)`); without preloading them, the call
    // throws "undefined" and a `try/catch` in the spec can pass vacuously.
    //
    // Policies are classes too (`ApplicationPolicy`, `<Model>Policy`), so a spec
    // can exercise the predicates directly instead of only through an HTTP
    // request — the base-class defaults and the fail-closed `policy_for` branch
    // are otherwise unreachable, since every concrete policy overrides them.
    // They sort before `helpers`, whose `current_user` / `signed_in?` must stay
    // the ones a spec sees (`app/policies/application_policy.sl` defines
    // identical globals; last write wins).
    for sub in [
        "models",
        "policies",
        "services",
        "helpers",
        "middleware",
        "jobs",
    ] {
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
            root_dir: Some(app_dir.clone()),
        };
        let tracker = CoverageTracker::new(config);
        let tracker = Arc::new(Mutex::new(tracker));
        {
            let mut tracker_guard = tracker.lock().unwrap();
            register_app_source_lines(&mut tracker_guard, &app_dir);
        }
        set_global_coverage_tracker(tracker.clone());
        Some(tracker)
    } else {
        None
    };

    // Start one test server per worker (each with its own DB) whenever the
    // app has controllers — any test that calls get()/post()/login()/etc
    // needs the server, not just files named *integration*. Per-worker
    // isolation: tests on worker `i` write to `{base}_w{i}` and hit
    // `127.0.0.1:{port_i}`, so concurrent workers don't trample each
    // other's rows or sessions.
    let needs_test_server = app_dir.join("app").join("controllers").is_dir();

    // Spawning a `soli serve` subprocess per worker is the dominant cost
    // when controllers are present: 8 parallel boots take ~12× a single
    // boot's time on a 16-core box, so high `--jobs` reliably regresses
    // wall time. Default to 3 in that case (sweet spot in benches), and
    // 1 otherwise (no subprocesses → linear scaling, but `lang/`-style
    // suites are short enough that a single worker is fine by default).
    // Cap at the number of test files — a single-file run pays no benefit
    // from spawning spare workers, and each spare test-server subprocess is
    // ~80-120ms of boot+probe before it sits idle for the entire suite.
    let num_workers = jobs
        .unwrap_or(if needs_test_server { 3 } else { 1 })
        .max(1)
        .min(test_files.len())
        .max(1);
    println!(
        "Running {} test(s) with {} worker(s)...",
        test_files.len(),
        num_workers
    );
    println!();

    let worker_databases = worker_database_names(num_workers, &base_test_database());
    ensure_test_databases(&worker_databases);

    #[derive(Clone)]
    struct WorkerEnv {
        port: Option<u16>,
        database: String,
    }

    struct ChildGuard(Vec<std::process::Child>);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            // Kill all first, THEN wait. The previous loop did
            // kill+wait per child sequentially, which (for 8 children
            // taking ~400ms each to fully exit) added ~3s of post-suite
            // wall time on `--jobs 8`. Sending SIGKILL to all up front
            // overlaps their kernel-side cleanup; we then just wait for
            // the slowest.
            for c in &mut self.0 {
                let _ = c.kill();
            }
            for mut c in self.0.drain(..) {
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

    // SEC-080: per-process random token gating the `/__coverage__` dump.
    // Minted once and shared across the spawned children + the runner's
    // collection request so an accidentally-enabled coverage endpoint can
    // never be scraped without proving knowledge of this token.
    let coverage_token = if needs_test_server && enable_coverage {
        Some(uuid::Uuid::new_v4().to_string())
    } else {
        None
    };

    // SEC-084: per-run UUID v4 the test runner hands children via
    // `SOLI_INTERNAL_TEST_RUNNER`. Children's `main.rs` only honours the
    // signal when the value parses as a UUID v4 — replacing the previous
    // `=1` token, which was easy for an operator to set accidentally
    // (or to inherit from a shell that happened to export it) and would
    // silently disable the SSRF guardrail.
    let internal_test_runner_token = if needs_test_server {
        uuid::Uuid::new_v4().to_string()
    } else {
        String::new()
    };

    if needs_test_server {
        println!("Starting {} test server(s)...", num_workers);

        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("soli"));
        let solidb_host =
            std::env::var("SOLIDB_HOST").unwrap_or_else(|_| "http://localhost:6745".to_string());
        let solidb_user = std::env::var("SOLIDB_USERNAME").unwrap_or_default();
        let solidb_pass = std::env::var("SOLIDB_PASSWORD").unwrap_or_default();

        // One `/auth/login` for the whole run: children inherit the token
        // via SOLIDB_JWT instead of each logging in at boot. N parallel
        // boots from one IP trip SoliDB's per-IP login rate limit
        // (20/min), and a process that boots into a poisoned window falls
        // back to basic auth with a login retry storm that keeps the
        // bucket full for every subsequent run.
        let shared_jwt = solilang::interpreter::builtins::model::db_config::get_jwt_token();

        for (i, env) in worker_envs.iter_mut().enumerate() {
            let port = {
                let listener = std::net::TcpListener::bind("127.0.0.1:0")
                    .expect("Failed to bind to random port");
                listener.local_addr().unwrap().port()
            };
            env.port = Some(port);

            // Run the server in dev mode so handlers execute via the
            // tree-walking interpreter. Class-based controllers don't
            // fully work through the VM path — a quick `/bench/*` spot
            // test passed, but running the full crm controller suite
            // without --dev caused 2-11 of 31 tests to fail (something
            // the VM doesn't reproduce: probably session middleware,
            // ERB-via-VM, or a class-feature edge case). Subprocess
            // isolation also avoids the reqwest+block_on cross-runtime
            // deadlock that bites when server and runner share a process.
            // `--workers 1` because each test server only serves a single
            // runner worker — extra hyper threads were dead weight that
            // contended for cores during the suite.
            let log_path = format!("/tmp/soli_test_server_w{}.log", i);
            let mut cmd = std::process::Command::new(&exe);
            cmd.arg("serve")
                .arg(&app_dir)
                .arg("--dev")
                .arg("--port")
                .arg(port.to_string())
                .arg("--workers")
                .arg("1")
                .env("APP_ENV", "test")
                // SEC-017 / SEC-084: hidden internal-only env signal for
                // the SSRF bypass on test-server children. The value is
                // a fresh UUID v4 minted per `soli test` run; `main.rs`
                // only honours the signal when the value parses as v4
                // so an operator who accidentally sets `=1` (legacy
                // shape) doesn't silently lose the SSRF guardrail.
                .env("SOLI_INTERNAL_TEST_RUNNER", &internal_test_runner_token)
                .env("SOLIDB_JWT", shared_jwt.as_deref().unwrap_or(""))
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
                if let Some(ref token) = coverage_token {
                    // SEC-080: child server gates `/__coverage__` on this token.
                    cmd.env("SOLI_COVERAGE_TOKEN", token);
                }
            }
            let child = cmd.spawn().expect("Failed to spawn test server subprocess");
            test_server_children.0.push(child);
        }

        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        // Probe every server concurrently. 50ms retry interval — an
        // isolated boot is ~80-120ms (now that Tailwind is skipped in
        // test mode via APP_ENV), so this typically converges in 1-3
        // attempts even under N-way parallel boot.
        let failed_port = std::thread::scope(|s| {
            let mut handles = Vec::new();
            for env in &worker_envs {
                let port = env.port.unwrap();
                let client = &client;
                handles.push(s.spawn(move || {
                    let base_url = format!("http://127.0.0.1:{}", port);
                    for attempt in 0..200 {
                        if attempt > 0 {
                            std::thread::sleep(Duration::from_millis(50));
                        }
                        if client.get(format!("{}/health", base_url)).send().is_ok() {
                            return None;
                        }
                    }
                    Some(port)
                }));
            }
            handles.into_iter().find_map(|h| h.join().unwrap())
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

    // Per-worker live state for the grid display. Each worker writes only
    // to its own slot (independent Mutex per slot) so contention is nil
    // even at high `--jobs`.
    let worker_slots: Arc<Vec<Mutex<WorkerSlot>>> = Arc::new(
        (0..num_workers)
            .map(|i| Mutex::new(WorkerSlot::new(i)))
            .collect(),
    );

    let stop_animation = Arc::new(AtomicBool::new(false));
    let animate = std::io::stderr().is_terminal();
    // Shared with the animation thread so the main thread's final-state
    // repaint knows how many lines to rewind over.
    let last_lines_drawn = Arc::new(AtomicUsize::new(0));

    let anim_handle = if animate {
        let progress = progress.clone();
        let stop = stop_animation.clone();
        let slots = worker_slots.clone();
        let last_lines_drawn = last_lines_drawn.clone();
        Some(std::thread::spawn(move || {
            let spinner = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
            let mut frame = 0usize;
            let mut last_lines = 0usize;
            while !stop.load(Ordering::Relaxed) {
                let snapshot = {
                    let p = progress.lock().unwrap();
                    ProgressState {
                        passed: p.passed,
                        failed: p.failed,
                        total_assertions: p.total_assertions,
                    }
                };
                let slot_snapshot: Vec<WorkerSlot> =
                    slots.iter().map(|m| m.lock().unwrap().clone()).collect();
                last_lines = redraw_grid(
                    &slot_snapshot,
                    &snapshot,
                    total_files,
                    spinner[frame],
                    last_lines,
                );
                last_lines_drawn.store(last_lines, Ordering::Relaxed);
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
        files.sort_by_key(|p| std::cmp::Reverse(fs::metadata(p).map(|m| m.len()).unwrap_or(0)));
        // Workers pop from the end, so reverse so the largest is popped first.
        files.reverse();
        Arc::new(Mutex::new(files))
    };

    let suite_start = std::time::Instant::now();
    std::thread::scope(|s| {
        let mut handles = Vec::new();
        for (worker_idx, env) in worker_envs.iter().take(num_workers).enumerate() {
            let queue = work_queue.clone();
            let preamble_files = model_preamble_files.clone();
            let tracker_clone = tracker.clone();
            let progress = progress.clone();
            let all_results_shared = all_results_shared.clone();
            let rt_handle = shared_rt_handle.clone();
            let env = env.clone();
            let slots = worker_slots.clone();

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
                    // Mark this worker as running `file`. Strip the `_test.sl`
                    // suffix when present so the cell shows what's under
                    // test, not the suffix.
                    let display_name = file
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| file.to_string_lossy().to_string());
                    let display_name = display_name
                        .strip_suffix("_test")
                        .map(|s| s.to_string())
                        .unwrap_or(display_name);
                    {
                        let mut slot = slots[worker_idx].lock().unwrap();
                        slot.current_file = Some(display_name);
                        slot.started_at = Some(std::time::Instant::now());
                    }
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

                    {
                        let mut slot = slots[worker_idx].lock().unwrap();
                        slot.current_file = None;
                        slot.started_at = None;
                        slot.files_done += 1;
                        if !passed {
                            slot.files_failed += 1;
                        }
                        slot.last_status = if passed { '✓' } else { '✗' };
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
    let final_icon = if failed > 0 { '✗' } else { '✓' };
    if animate {
        // Final repaint over the animator's last frame. We rewind by the
        // exact line count the animator stored before exiting so the grid
        // doesn't double-print.
        let final_slots: Vec<WorkerSlot> = worker_slots
            .iter()
            .map(|m| m.lock().unwrap().clone())
            .collect();
        let rewind = last_lines_drawn.load(Ordering::Relaxed);
        redraw_grid(&final_slots, &final_state, total_files, final_icon, rewind);
    } else {
        // Non-TTY: just print the bar inline so logs stay readable.
        eprint!(
            "{}\x1b[K",
            render_progress_bar(&final_state, total_files, &final_icon.to_string())
        );
    }
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
                    // SEC-080: present the per-process token the runner
                    // handed each child via SOLI_COVERAGE_TOKEN. Without
                    // it the server returns 403, by design — scrapers
                    // outside this process must not be able to read the
                    // coverage dump even if `SOLI_COVERAGE_ENABLED` is set.
                    let mut req = solilang::interpreter::builtins::http_class::ureq_agent()
                        .get(&url)
                        .timeout(Duration::from_secs(5));
                    if let Some(ref token) = coverage_token {
                        req = req.set("X-Coverage-Token", token);
                    }
                    let Ok(resp) = req.call() else {
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
                    show_uncovered,
                    per_test: false,
                    root_dir: Some(app_dir.clone()),
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

    if enable_coverage {
        clear_global_coverage_tracker();
    }

    // Truncate the worker DBs now that the suite is done so they're left
    // row-free for the next run and for any manual DB inspection in between.
    // We truncate rather than drop: dropping is ~500ms/DB serialised on the
    // server side (~4s at `--jobs 8`), whereas truncating collections is a
    // cheap parallel range tombstone. The collections (schema) survive, which
    // is what `ensure_test_databases` expects on the next run anyway.
    truncate_test_databases(&worker_databases);

    println!();

    drop(test_server_children);

    if failed > 0 {
        process::exit(1);
    }
}

fn base_test_database() -> String {
    std::env::var("SOLIDB_DATABASE").unwrap_or_else(|_| "default".to_string())
}

/// Returns one DB name per worker, derived from `SOLIDB_DATABASE`. With a
/// single worker the base name is used as-is; with multiple workers each
/// worker gets `{stem}_w{i}{suffix}` so the `_test` / `_spec` marker stays
/// at the end. App code commonly gates test-only behaviour on
/// `SOLIDB_DATABASE.ends_with("_test")`; the previous `{base}_w{i}` shape
/// (e.g. `tasks_test_w1`) silently masked the suffix on workers 1+ and
/// produced wildly different timings between `--jobs 1` and parallel runs.
fn worker_database_names(num_workers: usize, base_database_name: &str) -> Vec<String> {
    let base = if base_database_name.ends_with("_spec") || base_database_name.ends_with("_test") {
        base_database_name.to_string()
    } else {
        format!("{}_spec", base_database_name)
    };
    if num_workers <= 1 {
        return vec![base];
    }
    let suffix = if base.ends_with("_test") {
        "_test"
    } else {
        "_spec"
    };
    let stem = &base[..base.len() - suffix.len()];
    let mut names = vec![base.clone()];
    names.extend((1..num_workers).map(|i| format!("{}_w{}{}", stem, i, suffix)));
    names
}

fn test_db_host() -> String {
    std::env::var("SOLIDB_HOST").unwrap_or_else(|_| "http://localhost:6745".to_string())
}

fn test_db_auth_header() -> Option<String> {
    match (
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
    }
}

fn ensure_test_databases(db_names: &[String]) {
    let databases: Vec<&String> = db_names.iter().filter(|name| *name != "default").collect();
    if databases.is_empty() {
        return;
    }

    let host = test_db_host();
    println!(
        "Resetting {} test database(s) on {}...",
        databases.len(),
        host
    );
    let auth_header = test_db_auth_header();

    // Reset the base worker DB first: its collection list becomes the
    // template for newly-created sibling worker DBs. Pre-creating the
    // collections here — before any spec runs — matters because SoliDB
    // serializes collection creation engine-wide (~180ms each, an OPTIONS
    // file rewrite per RocksDB column family op). Left to the lazy
    // auto-create path, a first `--jobs 16` run creates hundreds of
    // collections in the middle of requests and random specs blow past
    // the 10s HTTP timeouts.
    let base_database = databases[0];
    let base_started = std::time::Instant::now();
    let base_outcome = prepare_test_database(&host, &auth_header, base_database, &[]);
    print_reset_outcome(base_database, base_started.elapsed(), &base_outcome);

    let template_collections: Vec<(String, String)> =
        list_collections(&host, &auth_header, base_database)
            .ok()
            .flatten()
            .unwrap_or_default();

    // Remaining worker DBs in parallel — independent operations on
    // distinct names, no shared state. The reset guarantees a row-free
    // baseline because `before_each` only clears collections the test
    // file knows about; without it, rows written by past runs (or by a
    // freshly added test) would leak across runs and cause
    // non-deterministic failures.
    let mut pending_creates: Vec<(String, Vec<(String, String)>)> = Vec::new();
    if databases.len() > 1 {
        std::thread::scope(|s| {
            let handles: Vec<_> = databases[1..]
                .iter()
                .map(|database| {
                    let host = &host;
                    let auth_header = &auth_header;
                    let template_collections = &template_collections;
                    s.spawn(move || {
                        let started = std::time::Instant::now();
                        let outcome = prepare_test_database(
                            host,
                            auth_header,
                            database,
                            template_collections,
                        );
                        (*database, started.elapsed(), outcome)
                    })
                })
                .collect();

            for handle in handles {
                let (database, elapsed, outcome) = handle.join().unwrap();
                print_reset_outcome(database, elapsed, &outcome);
                if let Ok((_, missing)) = outcome {
                    if !missing.is_empty() {
                        pending_creates.push((database.clone(), missing));
                    }
                }
            }
        });
    }

    // Create missing collections through ONE sequential queue. SoliDB
    // serializes column-family creation engine-wide (each is an OPTIONS
    // file rewrite, ~200ms+), so parallel creates only pile requests up
    // behind the lock until they time out client-side. Sequential is the
    // same total server time without the queueing failures. This is a
    // one-time cost per new worker DB; later runs truncate instead.
    let total_creates: usize = pending_creates.iter().map(|(_, names)| names.len()).sum();
    if total_creates > 0 {
        println!(
            "Creating {} missing collection(s) (one-time setup for new worker DBs)...",
            total_creates
        );
        let create_started = std::time::Instant::now();
        let mut created = 0usize;
        for (database, names) in &pending_creates {
            for (name, collection_type) in names {
                match create_collection(&host, &auth_header, database, name, collection_type) {
                    Ok(()) => created += 1,
                    // Not fatal: the lazy auto-create path covers any
                    // collection we fail to pre-create here.
                    Err(err) => println!("  ⚠ {}/{}: {}", database, name, err),
                }
            }
        }
        println!(
            "  ✓ {}/{} collection(s) created ({}ms)",
            created,
            total_creates,
            create_started.elapsed().as_millis()
        );
    }
    println!();
}

/// Outcome of preparing one worker DB: human-readable detail plus the
/// `(name, type)` collections still to create through the sequential queue.
type PrepareOutcome = Result<(String, Vec<(String, String)>), String>;

fn print_reset_outcome(database: &str, elapsed: std::time::Duration, outcome: &PrepareOutcome) {
    match outcome {
        Ok((detail, missing)) if missing.is_empty() => {
            println!("  ✓ {} {} ({}ms)", database, detail, elapsed.as_millis())
        }
        Ok((detail, missing)) => println!(
            "  ✓ {} {} ({}ms, {} collection(s) to create)",
            database,
            detail,
            elapsed.as_millis(),
            missing.len()
        ),
        // 401 without configured credentials is the normal state for
        // projects that don't use SoliDB (e.g. pure-language suites) —
        // note it dimly instead of flagging a failure.
        Err(err)
            if err.contains("status code 401") && std::env::var("SOLIDB_USERNAME").is_err() =>
        {
            println!(
                "  - {} skipped (SoliDB credentials not configured)",
                database
            )
        }
        Err(err) => println!(
            "  ✗ {} — {} (tests against this DB may fail)",
            database, err
        ),
    }
}

/// Reset one worker database to an empty baseline.
///
/// Truncating every collection (a RocksDB range tombstone, ~1-25ms each,
/// issued in parallel) is dramatically cheaper than dropping the database:
/// SoliDB drops the underlying column families one at a time with a manifest
/// fsync each (~175ms per collection — 7+ seconds on a 41-collection app
/// DB). So when the database already exists we truncate its collections and
/// only create the database when it's missing. Collections left behind by
/// removed tests survive as empty shells, which is harmless — the baseline
/// we need is "no rows", not "no collections". Set `SOLI_TEST_FRESH_DB=1`
/// to force the old drop+recreate when a schema-level reset is wanted
/// (e.g. after reworking migrations or indexes).
fn prepare_test_database(
    host: &str,
    auth_header: &Option<String>,
    database: &str,
    template_collections: &[(String, String)],
) -> PrepareOutcome {
    let agent = solilang::interpreter::builtins::http_class::ureq_agent();
    let with_auth = |mut req: ureq::Request| {
        if let Some(auth) = auth_header {
            req = req.set("Authorization", auth);
        }
        req
    };

    if std::env::var("SOLI_TEST_FRESH_DB").as_deref() == Ok("1") {
        // Drop errors are expected (404 when the DB doesn't exist yet) —
        // only the create result matters.
        let drop_url = format!("{}/_api/database/{}", host, database);
        let _ = with_auth(agent.delete(&drop_url)).call();
        create_test_database(host, &with_auth, database)?;
        return Ok((
            "recreated (SOLI_TEST_FRESH_DB)".to_string(),
            template_collections.to_vec(),
        ));
    }

    let Some(existing) = list_collections(host, auth_header, database)? else {
        // First run: the database doesn't exist yet. The template
        // collections get created afterwards through the sequential
        // create queue, so they don't get auto-created one by one in
        // the middle of specs.
        create_test_database(host, &with_auth, database)?;
        return Ok(("created".to_string(), template_collections.to_vec()));
    };

    // A collection whose type diverges from the base DB's (e.g. a "blob"
    // collection that was pre-created as plain "document" by an older
    // runner) can't be repaired by truncate — drop it and queue a
    // correctly-typed recreate.
    let template_types: std::collections::HashMap<&str, &str> = template_collections
        .iter()
        .map(|(name, collection_type)| (name.as_str(), collection_type.as_str()))
        .collect();
    let (mismatched, keep): (Vec<_>, Vec<_>) = existing.iter().partition(|(name, actual_type)| {
        template_types
            .get(name.as_str())
            .is_some_and(|expected| *expected != actual_type.as_str())
    });

    for (name, _) in &mismatched {
        let delete_url = format!("{}/_api/database/{}/collection/{}", host, database, name);
        with_auth(agent.delete(&delete_url))
            .call()
            .map_err(|err| format!("drop mistyped {} failed: {}", name, err))?;
    }

    let keep_collections: Vec<(String, String)> = keep
        .iter()
        .map(|collection| (*collection).clone())
        .collect();
    let truncate_errors = truncate_collections(host, auth_header, database, &keep_collections);

    if let Some(first_error) = truncate_errors.first() {
        return Err(format!(
            "{} ({} of {} truncates failed)",
            first_error,
            truncate_errors.len(),
            keep.len()
        ));
    }

    // Converge on the base DB's schema: a worker DB that predates a newly
    // added model would otherwise auto-create the collection mid-spec.
    let existing_names: std::collections::HashSet<&str> =
        keep.iter().map(|(name, _)| name.as_str()).collect();
    let missing: Vec<(String, String)> = template_collections
        .iter()
        .filter(|(name, _)| !existing_names.contains(name.as_str()))
        .cloned()
        .collect();

    let mut detail = if keep.is_empty() {
        "already empty".to_string()
    } else {
        format!("truncated {} collection(s)", keep.len())
    };
    if !mismatched.is_empty() {
        detail.push_str(&format!(", dropped {} mistyped", mismatched.len()));
    }
    Ok((detail, missing))
}

/// Truncate the given collections of one database in parallel. A truncate is
/// a cheap RocksDB range tombstone (~1-25ms each), so issuing them
/// concurrently keeps the wall time at roughly the slowest single collection.
/// Returns the per-collection error strings (empty on full success).
fn truncate_collections(
    host: &str,
    auth_header: &Option<String>,
    database: &str,
    collections: &[(String, String)],
) -> Vec<String> {
    let agent = solilang::interpreter::builtins::http_class::ureq_agent();
    let with_auth = |mut req: ureq::Request| {
        if let Some(auth) = auth_header {
            req = req.set("Authorization", auth);
        }
        req
    };
    std::thread::scope(|s| {
        let handles: Vec<_> = collections
            .iter()
            .map(|(collection, _)| {
                let with_auth = &with_auth;
                s.spawn(move || {
                    let truncate_url = format!(
                        "{}/_api/database/{}/collection/{}/truncate",
                        host, database, collection
                    );
                    with_auth(agent.put(&truncate_url))
                        .call()
                        .map(|_| ())
                        .map_err(|err| format!("truncate {} failed: {}", collection, err))
                })
            })
            .collect();
        handles
            .into_iter()
            .filter_map(|handle| handle.join().unwrap().err())
            .collect()
    })
}

/// Truncate every collection in each worker DB once the suite finishes,
/// leaving them row-free for the next run and for any manual DB inspection
/// in between. Unlike a drop (~500ms/DB serialised on the server), truncate
/// is a cheap parallel range tombstone, so the post-suite tail stays small.
fn truncate_test_databases(db_names: &[String]) {
    let databases: Vec<&String> = db_names.iter().filter(|name| *name != "default").collect();
    if databases.is_empty() {
        return;
    }
    let host = test_db_host();
    let auth_header = test_db_auth_header();
    let started = std::time::Instant::now();

    let errors: Vec<String> = std::thread::scope(|s| {
        let handles: Vec<_> = databases
            .iter()
            .map(|database| {
                let host = &host;
                let auth_header = &auth_header;
                s.spawn(move || {
                    // A missing DB (None) or a list error means nothing to
                    // truncate — surface only real list errors.
                    match list_collections(host, auth_header, database) {
                        Ok(Some(collections)) => {
                            truncate_collections(host, auth_header, database, &collections)
                        }
                        Ok(None) => Vec::new(),
                        Err(err) => vec![format!("{}: {}", database, err)],
                    }
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap())
            .collect()
    });

    if let Some(first_error) = errors.first() {
        // Non-fatal: the next run's `ensure_test_databases` resets anyway.
        println!(
            "⚠ post-suite truncate had {} error(s); first: {}",
            errors.len(),
            first_error
        );
    } else {
        println!(
            "Truncated {} test database(s) ({}ms)",
            databases.len(),
            started.elapsed().as_millis()
        );
    }
}

/// List a database's collections as `(name, type)` pairs — type is
/// "document", "edge", or "blob" and MUST survive into pre-created
/// worker-DB collections (a blob upload against a "document"-typed
/// collection is a 400). `Ok(None)` means the database doesn't exist.
fn list_collections(
    host: &str,
    auth_header: &Option<String>,
    database: &str,
) -> Result<Option<Vec<(String, String)>>, String> {
    let agent = solilang::interpreter::builtins::http_class::ureq_agent();
    let list_url = format!("{}/_api/database/{}/collection", host, database);
    let mut req = agent.get(&list_url);
    if let Some(auth) = auth_header {
        req = req.set("Authorization", auth);
    }
    let response = match req.call() {
        Ok(response) => response,
        Err(ureq::Error::Status(404, _)) => return Ok(None),
        Err(err) => return Err(format!("list collections failed: {}", err)),
    };
    let body = response
        .into_string()
        .map_err(|err| format!("list collections failed: {}", err))?;
    let json: serde_json::Value =
        serde_json::from_str(&body).map_err(|err| format!("list collections failed: {}", err))?;
    Ok(Some(
        json["collections"]
            .as_array()
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|entry| {
                        let name = entry["name"].as_str()?.to_string();
                        let collection_type =
                            entry["type"].as_str().unwrap_or("document").to_string();
                        Some((name, collection_type))
                    })
                    .collect()
            })
            .unwrap_or_default(),
    ))
}

/// Create one collection with an explicit type. 409 (already exists)
/// counts as success.
fn create_collection(
    host: &str,
    auth_header: &Option<String>,
    database: &str,
    collection: &str,
    collection_type: &str,
) -> Result<(), String> {
    let agent = solilang::interpreter::builtins::http_class::ureq_agent();
    let create_url = format!("{}/_api/database/{}/collection", host, database);
    let payload = format!(
        r#"{{"name":"{}","type":"{}"}}"#,
        collection, collection_type
    );
    let mut req = agent
        .post(&create_url)
        .set("Content-Type", "application/json");
    if let Some(auth) = auth_header.as_ref() {
        req = req.set("Authorization", auth);
    }
    match req.send_string(&payload) {
        Ok(_) | Err(ureq::Error::Status(409, _)) => Ok(()),
        Err(err) => Err(format!("create failed: {}", err)),
    }
}

fn create_test_database(
    host: &str,
    with_auth: &dyn Fn(ureq::Request) -> ureq::Request,
    database: &str,
) -> Result<(), String> {
    let agent = solilang::interpreter::builtins::http_class::ureq_agent();
    let create_url = format!("{}/_api/database", host);
    let payload = format!(r#"{{"name":"{}"}}"#, database);
    with_auth(
        agent
            .post(&create_url)
            .set("Content-Type", "application/json"),
    )
    .send_string(&payload)
    .map_err(|err| format!("create failed: {}", err))?;
    Ok(())
}

pub fn collect_test_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().map(|e| e == "sl").unwrap_or(false) {
                files.push(path);
            } else if path.is_dir() {
                let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if dir_name.starts_with('.') || dir_name.starts_with('_') {
                    continue;
                }
                files.extend(collect_test_files(&path));
            }
        }
    }

    files
}

/// Like `collect_test_files`, but also picks up `.slv` view templates. Used by
/// `soli lint` so that directory linting covers views, not just `.sl` sources.
pub fn collect_lint_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                let lintable = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|ext| ext == "sl" || ext == "slv")
                    .unwrap_or(false);
                if lintable {
                    files.push(path);
                }
            } else if path.is_dir() {
                let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if dir_name.starts_with('.') || dir_name.starts_with('_') {
                    continue;
                }
                files.extend(collect_lint_files(&path));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_app_dir_bare_filename() {
        // `foo.sl` — no parent components, falls through to ".".
        assert_eq!(
            resolve_app_dir(Path::new("foo.sl"), true),
            PathBuf::from(".")
        );
    }

    #[test]
    fn resolve_app_dir_relative_spec_in_tests_dir() {
        // The bug case: `tests/foo.sl` — `parent().and_then(parent)` yields
        // `Some("")` (empty path), not `None`. Pre-fix this short-circuited
        // `unwrap_or_else` and `app_dir` was the empty path.
        assert_eq!(
            resolve_app_dir(Path::new("tests/foo.sl"), true),
            PathBuf::from(".")
        );
    }

    #[test]
    fn resolve_app_dir_dot_prefixed_relative_spec() {
        // `./tests/foo.sl` — parent chain produces `Some(".")`, kept as-is.
        assert_eq!(
            resolve_app_dir(Path::new("./tests/foo.sl"), true),
            PathBuf::from(".")
        );
    }

    #[test]
    fn resolve_app_dir_absolute_spec() {
        // `/path/to/tests/foo.sl` — parent chain produces `/path/to`.
        assert_eq!(
            resolve_app_dir(Path::new("/path/to/tests/foo.sl"), true),
            PathBuf::from("/path/to")
        );
    }

    #[test]
    fn resolve_app_dir_directory_arg() {
        // Directory case: `tests/` — one level up should resolve to ".".
        assert_eq!(
            resolve_app_dir(Path::new("tests"), false),
            PathBuf::from(".")
        );
    }

    #[test]
    fn worker_database_names_single_bare_default() {
        assert_eq!(worker_database_names(1, "default"), vec!["default_spec"]);
    }

    #[test]
    fn worker_database_names_single_explicit_test_suffix() {
        assert_eq!(worker_database_names(1, "myapp_test"), vec!["myapp_test"]);
    }

    #[test]
    fn worker_database_names_multiple_default_base() {
        assert_eq!(
            worker_database_names(3, "default"),
            vec!["default_spec", "default_w1_spec", "default_w2_spec"]
        );
    }

    #[test]
    fn worker_database_names_multiple_test_base_preserves_suffix() {
        let names = worker_database_names(3, "myapp_test");
        assert_eq!(names, vec!["myapp_test", "myapp_w1_test", "myapp_w2_test"]);
        for n in &names {
            assert!(
                n.ends_with("_test"),
                "{} should end with _test so app-level test gates work",
                n
            );
        }
    }

    #[test]
    fn worker_database_names_existing_spec_base() {
        assert_eq!(worker_database_names(1, "foo_spec"), vec!["foo_spec"]);
    }

    #[test]
    fn worker_database_names_existing_test_base() {
        assert_eq!(worker_database_names(1, "foo_test"), vec!["foo_test"]);
    }

    #[test]
    fn worker_database_names_explicit_spec_no_double_suffix() {
        assert_eq!(
            worker_database_names(3, "foo_spec"),
            vec!["foo_spec", "foo_w1_spec", "foo_w2_spec"]
        );
    }
}
