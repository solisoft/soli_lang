//! In-process background job worker pool.
//!
//! Job code runs here, never on a web-worker thread, so a slow handler cannot
//! delay request serving. The job engine's poller (`crate::jobs::engine`) claims
//! due rows from the queue and hands each one to this pool; the worker reports
//! the outcome back so the row is completed or retried.
//!
//! Jobs handed over without a queue row (`enqueue`) are fire-and-forget: nothing
//! records their outcome, so they own their idempotency and error handling.
//!
//! `Value` is `Rc`-based and not `Send`, so job args cannot cross the thread
//! boundary as a `Value`. They travel as a JSON string and are re-parsed on the
//! pool thread, which runs its own fully-loaded interpreter (job classes live in
//! a `thread_local!` registry, so each pool thread must load them itself).

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::thread;
use std::time::Instant;

use crossbeam::channel;

use super::app_loader::{load_jobs_in_worker, load_models};
use super::set_tokio_handle;
use super::uploads_prelude;
use super::FileTracker;
use crate::interpreter::builtins::server::{set_worker_routes, WorkerRoute};
use crate::interpreter::builtins::{mailer, named_routes, template};
use crate::interpreter::value::Value;
use crate::interpreter::Interpreter;
use crate::span::Span;

/// A unit of work handed to the background pool. Every field is `Send`; args
/// are carried as JSON (see module docs).
struct BackgroundJob {
    class_name: String,
    args_json: String,
    /// The queue row this work came from, when the job engine dispatched it.
    /// `None` for legacy fire-and-forget hand-offs, which have no row to
    /// update and therefore no retry.
    record: Option<crate::jobs::JobDoc>,
}

/// Set once when the pool starts. `enqueue` returns `false` while unset so
/// callers fall back to running the job inline (pool disabled / not serving).
static BG_SENDER: OnceLock<channel::Sender<BackgroundJob>> = OnceLock::new();

/// Everything a pool thread needs to build its own interpreter — the same
/// job-relevant subset a web worker loads (models/services/policies/mailers/
/// jobs/templates), minus routing/controllers/VM.
#[derive(Clone)]
pub struct PoolConfig {
    pub models_dir: PathBuf,
    pub helpers_dir: PathBuf,
    pub views_dir: PathBuf,
    pub jobs_dir: PathBuf,
    pub routes: Vec<WorkerRoute>,
    pub runtime_handle: tokio::runtime::Handle,
    pub dev_mode: bool,
    pub num_workers: usize,
}

/// The Soli runner defined in each pool interpreter.
///
/// Returns `null` on success and an error string on failure, so the Rust side
/// can complete or retry the queue row. A missing class is an error too — the
/// job must not be marked done when nothing ran (that happens during a deploy
/// where a handler was renamed; the retry gives the new code a chance).
const JOBS_BACKGROUND_RUNNER: &str = r#"
fn __soli_run_job_bg(name, args) {
    let cls = __soli_get_class(name);
    if cls == null {
        return "job class not loaded: " + str(name);
    }
    try {
        cls.perform(args);
        return null;
    } catch err {
        return str(err);
    }
}
"#;

/// Hand a job to the pool without a queue row: fire-and-forget, no retry.
/// Returns `false` if the pool isn't running, so the caller can fall back to
/// running the job inline.
pub fn enqueue(class_name: String, args_json: String) -> bool {
    send(BackgroundJob {
        class_name,
        args_json,
        record: None,
    })
}

/// Hand a claimed queue row to the pool. The worker reports its outcome back to
/// the job store, so the row is completed or retried. Returns `false` when the
/// pool isn't running — the caller must then release its claim.
pub fn enqueue_with_id(class_name: String, args_json: String, record: crate::jobs::JobDoc) -> bool {
    send(BackgroundJob {
        class_name,
        args_json,
        record: Some(record),
    })
}

fn send(job: BackgroundJob) -> bool {
    match BG_SENDER.get() {
        Some(tx) => tx.send(job).is_ok(),
        None => false,
    }
}

/// Spawn the background job worker pool. No-op when `num_workers == 0`, or if
/// called twice (the sender is set once). Called from `serve()` after routes are
/// resolved and only when the jobs callback route is active.
pub fn start_pool(config: PoolConfig) {
    if config.num_workers == 0 {
        return;
    }
    let (tx, rx) = channel::unbounded::<BackgroundJob>();
    if BG_SENDER.set(tx).is_err() {
        // Already started — leave the existing pool in place.
        return;
    }

    for id in 0..config.num_workers {
        let rx = rx.clone();
        let config = config.clone();
        let builder = thread::Builder::new().name(format!("bg-job-{}", id));
        if let Err(e) = builder.spawn(move || run_pool_worker(id, rx, config)) {
            eprintln!("Failed to spawn background job worker {}: {}", id, e);
        }
    }
    println!("Started {} background job worker(s)", config.num_workers);
}

/// One pool thread: build a loaded interpreter, then drain the channel forever.
/// Mirrors the web-worker restart-on-panic loop so a panicking job recreates the
/// interpreter and keeps the pool alive.
fn run_pool_worker(id: usize, rx: channel::Receiver<BackgroundJob>, config: PoolConfig) {
    // Job code uses Model.*/HTTP.* which need the server's tokio handle.
    set_tokio_handle(config.runtime_handle.clone());

    loop {
        let rx = rx.clone();
        let config = config.clone();
        let result = catch_unwind(AssertUnwindSafe(|| {
            let mut interpreter = Interpreter::new_for_serve();
            build_job_interpreter(id, &mut interpreter, &config);

            let runner = interpreter.global_env().borrow().get("__soli_run_job_bg");
            let Some(runner) = runner else {
                eprintln!("Background job worker {}: runner not defined; exiting", id);
                return;
            };
            worker_recv_loop(&rx, &mut interpreter, runner);
        }));

        match result {
            Ok(_) => break, // channel closed — normal shutdown
            Err(_) => eprintln!("Background job worker {} panicked, restarting...", id),
        }
    }
}

/// Whether background-job interpreters should load view helpers (incl. i18n
/// locale tables). Defaults to `true`; set `SOLI_JOB_VIEW_HELPERS=0` to skip
/// them when no job renders a helper-using template — a per-job-interpreter
/// memory saving.
fn job_view_helpers_enabled() -> bool {
    !matches!(
        std::env::var("SOLI_JOB_VIEW_HELPERS").ok().as_deref(),
        Some("0") | Some("false") | Some("no")
    )
}

/// Load the job-relevant slice of the app into a pool interpreter: templates and
/// view helpers (jobs may render), the Mailer prelude, models + sibling
/// services/policies/mailers, the uploads prelude, named-route URL helpers, and
/// the job classes themselves. Then define the background runner.
fn build_job_interpreter(id: usize, interpreter: &mut Interpreter, config: &PoolConfig) {
    // URL/context parity with web workers (named-route helpers read these).
    set_worker_routes(config.routes.clone());

    if config.views_dir.exists() {
        template::init_templates(config.views_dir.clone());
    }
    // View helpers (which include an app's i18n locale tables — often the
    // single largest per-interpreter cost) are only needed if a job renders a
    // template that calls a helper. Most jobs don't, so this is opt-out via
    // `SOLI_JOB_VIEW_HELPERS=0` to drop the helper/locale ASTs from every job
    // interpreter. Default keeps the previous behavior for apps whose mailers
    // render helper-using templates in a job.
    if config.helpers_dir.exists() && job_view_helpers_enabled() {
        if let Err(e) = template::load_view_helpers(&config.helpers_dir) {
            eprintln!(
                "Background job worker {}: error loading view helpers: {}",
                id, e
            );
        }
    }
    template::set_dev_mode(config.dev_mode);

    // Mailer/Message/__MailDelivery base classes (used by `deliver_later` jobs).
    mailer::ensure_prelude(interpreter);

    if let Err(e) = load_models(interpreter, &config.models_dir) {
        eprintln!("Background job worker {}: error loading models: {}", id, e);
    }
    if let Some(parent) = config.models_dir.parent() {
        for sub in ["services", "policies", "mailers"] {
            let dir = parent.join(sub);
            if dir.exists() {
                if let Err(e) = load_models(interpreter, &dir) {
                    eprintln!("Background job worker {}: error loading {}: {}", id, sub, e);
                }
            }
        }
    }

    if let Err(e) = uploads_prelude::define_uploads_prelude(interpreter) {
        eprintln!(
            "Background job worker {}: error loading uploads prelude: {}",
            id, e
        );
    }

    {
        let mut env = interpreter.environment.borrow_mut();
        named_routes::register_named_route_helpers(&mut env);
    }

    // Job classes + `__soli_get_class`/callback prelude registration.
    if config.jobs_dir.exists() {
        let mut tracker = FileTracker::new();
        // `false`: never sync `static cron` — web worker 0 already did.
        load_jobs_in_worker(id, interpreter, &config.jobs_dir, &mut tracker, false);
    }

    define_bg_runner(id, interpreter);
}

/// Lex/parse/execute `JOBS_BACKGROUND_RUNNER` into the interpreter, mirroring
/// `mailer::ensure_prelude`.
fn define_bg_runner(id: usize, interpreter: &mut Interpreter) {
    let tokens = match crate::lexer::Scanner::new(JOBS_BACKGROUND_RUNNER).scan_tokens() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Background job worker {}: runner lex error: {}", id, e);
            return;
        }
    };
    let program = match crate::parser::Parser::new(tokens).parse() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Background job worker {}: runner parse error: {}", id, e);
            return;
        }
    };
    if let Err(e) = interpreter.interpret(&program) {
        eprintln!("Background job worker {}: runner execute error: {}", id, e);
    }
}

/// Drain the channel: parse args JSON, run the job via the Soli runner, then
/// report the outcome to the job store (when the work came from a queue row).
/// Returns when the channel is closed.
fn worker_recv_loop(
    rx: &channel::Receiver<BackgroundJob>,
    interpreter: &mut Interpreter,
    runner: Value,
) {
    while let Ok(job) = rx.recv() {
        if let Some(record) = &job.record {
            crate::jobs::engine::mark_started(&record.key);
        }
        // Every exit path below must clear the in-flight marker, or the poller
        // keeps renewing a lease for work that already finished.
        let outcome = run_one(&job, interpreter, &runner);
        if let Some(record) = &job.record {
            crate::jobs::engine::mark_finished(&record.key);
            crate::jobs::engine::report_outcome(record, outcome.as_deref());
        } else if let Some(err) = &outcome {
            eprintln!("Background job {} error: {}", job.class_name, err);
        }
    }
}

/// Execute one job. Returns `None` on success, `Some(error)` otherwise.
///
/// A panicking handler is caught here (not just at the thread level) so the
/// queue row is failed and retried rather than lost when the worker restarts.
fn run_one(job: &BackgroundJob, interpreter: &mut Interpreter, runner: &Value) -> Option<String> {
    let args = match crate::interpreter::value::parse_json(&job.args_json) {
        Ok(value) => value,
        Err(e) => return Some(format!("invalid args JSON: {e}")),
    };

    let start = Instant::now();
    let call_args = vec![Value::String(job.class_name.clone().into()), args];
    let result = catch_unwind(AssertUnwindSafe(|| {
        interpreter.call_value(runner.clone(), call_args, Span::default())
    }));

    let error = match result {
        // The runner returns null on success, an error string on failure.
        Ok(Ok(Value::String(err))) => Some(err.to_string()),
        Ok(Ok(_)) => None,
        Ok(Err(e)) => Some(e.to_string()),
        Err(_) => Some("job panicked".to_string()),
    };

    let ms = start.elapsed().as_millis();
    if error.is_none() {
        println!("[bg-job] {} finished in {}ms", job.class_name, ms);
    }
    error
}
