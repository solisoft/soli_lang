//! Standalone job worker (`soli jobs` / `soli worker`) and queue CLI.
//!
//! `soli serve` can run the poller next to HTTP. Operators who want to scale
//! job capacity separately set `SOLI_JOB_WORKERS=0` on the web process and run
//! this command instead — same database, same `_jobs` table, no HTTP listener.

use std::path::{Path, PathBuf};

use super::app_loader::load_jobs_in_worker;
use super::background_jobs::{self, PoolConfig};
use super::env_loader::load_env_files;
use super::server_constants;
use super::set_tokio_handle;
use super::FileTracker;
use crate::interpreter::builtins::mailer;
use crate::interpreter::Interpreter;
use crate::jobs::store;

/// Start a worker process that claims and runs jobs until SIGINT/SIGTERM.
///
/// `cli_workers` is `Some` only when `--workers` was passed; otherwise the
/// count comes from `SOLI_JOB_WORKERS` in the app's `.env`, then the serve
/// default.
pub fn run_worker(folder: &Path, cli_workers: Option<usize>) -> Result<(), String> {
    let folder = boot_app_db(folder)?;
    let workers = resolve_workers(cli_workers);
    if workers == 0 {
        return Err(
            "soli jobs needs at least 1 worker (got 0). Use `soli jobs --workers N` \
             or set SOLI_JOB_WORKERS."
                .to_string(),
        );
    }
    crate::interpreter::builtins::file::set_file_jail(folder.clone());
    crate::interpreter::builtins::image::set_image_jail(folder.clone());

    let app_dir = folder.join("app");
    let models_dir = app_dir.join("models");
    let helpers_dir = app_dir.join("helpers");
    let views_dir = app_dir.join("views");
    let jobs_dir = app_dir.join("jobs");

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("tokio runtime: {e}"))?;
    let handle = rt.handle().clone();
    set_tokio_handle(handle.clone());

    // Register `static cron` once on this process so a worker-only deploy
    // still picks up class-body schedules (web worker 0 is not running).
    if jobs_dir.exists() {
        let mut interpreter = Interpreter::new_for_serve();
        mailer::ensure_prelude(&mut interpreter);
        let mut tracker = FileTracker::new();
        load_jobs_in_worker(0, &mut interpreter, &jobs_dir, &mut tracker, true);
    }

    background_jobs::start_pool(PoolConfig {
        models_dir,
        helpers_dir,
        views_dir,
        jobs_dir,
        routes: Vec::new(),
        runtime_handle: handle,
        dev_mode: false,
        num_workers: workers,
    });
    crate::jobs::engine::start(workers, rt.handle().clone());

    println!(
        "Job worker ready ({} slot{}, {}) — Ctrl-C to stop",
        workers,
        if workers == 1 { "" } else { "s" },
        folder.display()
    );

    rt.block_on(async {
        shutdown_signal().await;
    });

    super::shutdown::begin_drain();
    println!("Job worker draining — in-flight work will finish its lease");
    // Give in-flight jobs a moment to report; the poller keeps renewing
    // leases while draining so they are not stolen mid-flight.
    std::thread::sleep(std::time::Duration::from_millis(400));
    Ok(())
}

/// Print queued jobs as a table. Optional `queue` / `state` narrow the list.
pub fn run_list(folder: &Path, queue: Option<&str>, state: Option<&str>) -> Result<(), String> {
    boot_app_db(folder)?;
    let rows = store::list(queue).map_err(|e| format!("Job.list failed: {e}"))?;
    let wanted_state = state.map(|s| s.to_ascii_lowercase());

    let mut shown = 0usize;
    println!(
        "{:<36} {:<10} {:<12} {:<24} {:>8}  RUN_AT",
        "ID", "STATE", "QUEUE", "HANDLER", "TRIES"
    );
    for row in &rows {
        let row_state = row
            .get("state")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
            .to_string();
        if let Some(want) = wanted_state.as_deref() {
            if !row_state.eq_ignore_ascii_case(want) {
                continue;
            }
        }
        let id = row.get("_key").and_then(|v| v.as_str()).unwrap_or("-");
        let queue_name = row.get("queue").and_then(|v| v.as_str()).unwrap_or("-");
        let handler = row.get("handler").and_then(|v| v.as_str()).unwrap_or("-");
        let attempts = row.get("attempts").and_then(|v| v.as_i64()).unwrap_or(0);
        let run_at = row.get("run_at").and_then(|v| v.as_str()).unwrap_or("-");
        println!(
            "{:<36} {:<10} {:<12} {:<24} {:>8}  {}",
            truncate(id, 36),
            truncate(&row_state, 10),
            truncate(queue_name, 12),
            truncate(handler, 24),
            attempts,
            run_at
        );
        shown += 1;
    }
    if shown == 0 {
        println!("(no jobs)");
    } else {
        println!("{shown} job(s)");
    }
    Ok(())
}

/// Re-queue a failed or dead job.
pub fn run_retry(folder: &Path, id: &str) -> Result<(), String> {
    boot_app_db(folder)?;
    match store::retry(id) {
        Ok(true) => {
            println!("retried {id}");
            Ok(())
        }
        Ok(false) => Err(format!("no job {id}")),
        Err(e) => Err(e),
    }
}

/// Cancel a not-yet-running job.
pub fn run_cancel(folder: &Path, id: &str) -> Result<(), String> {
    boot_app_db(folder)?;
    match store::cancel(id) {
        Ok(true) => {
            println!("cancelled {id}");
            Ok(())
        }
        Ok(false) => Err(format!("no job {id}")),
        Err(e) => Err(e),
    }
}

fn boot_app_db(folder: &Path) -> Result<PathBuf, String> {
    if !folder.exists() {
        return Err(format!("Folder '{}' does not exist", folder.display()));
    }
    if !folder.is_dir() {
        return Err(format!("'{}' is not a directory", folder.display()));
    }
    crate::module::enforce_min_soli_version(folder)?;
    let folder = folder
        .canonicalize()
        .unwrap_or_else(|_| folder.to_path_buf());
    load_env_files(&folder);
    crate::db::init_from_app_path(&folder).map_err(|e| e.message())?;
    crate::db::ensure_runtime_ready().map_err(|e| e.message())?;
    crate::interpreter::builtins::model::init_db_config();
    Ok(folder)
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    let mut out: String = value.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Default worker count: `--workers` already resolved by the CLI, else env,
/// else the same default `soli serve` uses.
pub fn resolve_workers(cli_workers: Option<usize>) -> usize {
    if let Some(n) = cli_workers {
        return n;
    }
    std::env::var("SOLI_JOB_WORKERS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(server_constants::DEFAULT_JOB_WORKERS)
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler");
        tokio::select! {
            _ = ctrl_c => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = ctrl_c.await;
    }
}
