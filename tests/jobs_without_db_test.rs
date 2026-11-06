//! `soli serve` on an app whose database does not answer.
//!
//! `soli new` scaffolds an empty `app/jobs/`, and the job engine used to start
//! on the directory alone, so every scaffolded app ran a poller that printed
//! `[cron] tick failed` and `[jobs] poll failed` on every tick when no database
//! was there. These tests run the real binary against a throwaway app with the
//! database on a closed port and read what it prints.

use std::io::Read;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

fn pick_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .map(|addr| addr.port())
        .expect("a free local port")
}

/// A minimal app: one route, an `app/jobs/` directory, and `job_files` in it.
fn write_app(root: &Path, job_files: &[(&str, &str)]) {
    std::fs::create_dir_all(root.join("config")).expect("config dir");
    std::fs::create_dir_all(root.join("app/controllers")).expect("controllers dir");
    std::fs::create_dir_all(root.join("app/jobs")).expect("jobs dir");
    std::fs::write(
        root.join("config/routes.sl"),
        "get(\"/ping\", \"home#ping\")\n",
    )
    .expect("routes");
    std::fs::write(
        root.join("app/controllers/home_controller.sl"),
        "class HomeController < Controller\n  # GET /ping\n  def ping(req)\n    \
         {\"status\": 200, \"body\": \"pong\"}\n  end\nend\n",
    )
    .expect("controller");
    for (name, source) in job_files {
        std::fs::write(root.join("app/jobs").join(name), source).expect("job file");
    }
}

/// Collect a child stream into `sink` on a thread of its own.
fn drain(mut stream: impl Read + Send + 'static, sink: Arc<Mutex<String>>) {
    thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = stream.read(&mut buf) {
            if n == 0 {
                break;
            }
            if let Ok(mut out) = sink.lock() {
                out.push_str(&String::from_utf8_lossy(&buf[..n]));
            }
        }
    });
}

/// Serve `app` in production with the database on a closed port, wait until it
/// answers, let the poller tick for `observe`, and return everything printed.
fn serve_and_capture(app: &Path, observe: Duration) -> String {
    let port = pick_port();
    let mut child: Child = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_soli")))
        .arg("serve")
        .arg(app)
        .arg("--port")
        .arg(port.to_string())
        .arg("--workers")
        .arg("1")
        .env("SOLI_HOST", "127.0.0.1")
        .env("SOLIDB_HOST", "http://127.0.0.1:1")
        .env("SOLIDB_USERNAME", "admin")
        .env("SOLIDB_PASSWORD", "admin")
        .env_remove("SOLI_SMTP_HOST")
        .env_remove("SOLI_JOB_WORKERS")
        // A fast tick, so a few seconds hold many of them.
        .env("SOLI_JOBS_POLL_MS", "100")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn soli serve");
    let output = Arc::new(Mutex::new(String::new()));
    drain(child.stdout.take().expect("stdout"), output.clone());
    drain(child.stderr.take().expect("stderr"), output.clone());

    let deadline = Instant::now() + Duration::from_secs(60);
    let mut ready = false;
    while Instant::now() < deadline {
        if ureq::get(&format!("http://127.0.0.1:{port}/ping"))
            .timeout(Duration::from_millis(500))
            .call()
            .is_ok()
        {
            ready = true;
            break;
        }
        thread::sleep(Duration::from_millis(200));
    }
    thread::sleep(observe);
    let _ = child.kill();
    let _ = child.wait();
    thread::sleep(Duration::from_millis(200));
    let printed = output.lock().map(|out| out.clone()).unwrap_or_default();
    assert!(ready, "server never answered /ping; it printed:\n{printed}");
    printed
}

#[test]
fn an_empty_jobs_directory_starts_no_engine_and_prints_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_app(dir.path(), &[]);
    let printed = serve_and_capture(dir.path(), Duration::from_secs(3));
    assert!(
        !printed.contains("Job engine:"),
        "no job file, no engine:\n{printed}"
    );
    assert!(!printed.contains("[jobs]"), "{printed}");
    assert!(!printed.contains("[cron]"), "{printed}");
}

#[test]
fn an_unreachable_database_is_reported_once_not_per_tick() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_app(
        dir.path(),
        &[(
            "tick_job.sl",
            "class TickJob\n  static def perform(args)\n    nil\n  end\nend\n",
        )],
    );
    let printed = serve_and_capture(dir.path(), Duration::from_secs(4));
    assert!(
        printed.contains("Job engine:"),
        "a job file starts the engine:\n{printed}"
    );
    assert_eq!(
        printed.matches("[jobs] database unreachable").count(),
        1,
        "one line per outage:\n{printed}"
    );
    assert!(!printed.contains("[cron] tick failed"), "{printed}");
    assert!(!printed.contains("[jobs] poll failed"), "{printed}");
    assert!(
        printed.matches("JWT login failed").count() <= 1,
        "the login warning prints once per failure streak:\n{printed}"
    );
}
