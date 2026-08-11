//! Background jobs and cron scheduling — the Soli-side API surface.
//!
//! Exposes three static-method-only classes:
//! - `Job` — enqueue, schedule, list, cancel queue jobs.
//! - `Webhook` — enqueue an outbound HTTP delivery as a job.
//! - `Cron` — manage recurring jobs and build cron expressions.
//!
//! Job handlers are user-defined classes in `app/jobs/*_job.sl` with a
//! `static def perform(args)`. Everything here writes rows into the `_jobs` /
//! `_cron_jobs` collections through [`crate::jobs::store`]; the engine
//! (`crate::jobs::engine`) claims and runs them inside the Soli process. No
//! database calls back into the app.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::OnceLock;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{empty_hash, value_to_json, Class, NativeFunction, Value};
use crate::jobs::{scheduler, store, JobDoc};

use std::cell::RefCell;

thread_local! {
    /// Per-worker registry of loaded `app/jobs/*_job.sl` classes, keyed by
    /// class name. Populated by `load_jobs_in_worker` after facade injection.
    ///
    /// The job runner resolves a handler name to its class through this
    /// registry. It exists because the prod execution path runs requests through
    /// the bytecode VM, which never populates the thread-local `CURRENT_ENV`
    /// that `current_env_lookup` reads — so an env-only lookup returns Null in
    /// prod even though the class loaded fine at boot. This registry is
    /// populated in both modes on the worker thread, so dispatch works either
    /// way.
    static JOB_CLASSES: RefCell<HashMap<String, Value>> = RefCell::new(HashMap::new());
}

/// Register a loaded job class so the job runner can find it independently of
/// the (interpreter-only) `CURRENT_ENV` thread-local.
pub fn register_job_class_in_registry(name: &str, class: Value) {
    JOB_CLASSES.with(|registry| {
        registry.borrow_mut().insert(name.to_string(), class);
    });
}

/// Look up a job class previously registered via
/// `register_job_class_in_registry`. Returns `None` if unknown.
pub fn lookup_job_class(name: &str) -> Option<Value> {
    JOB_CLASSES.with(|registry| registry.borrow().get(name).cloned())
}

/// Default queue for enqueues that don't name one.
fn default_queue() -> &'static str {
    static Q: OnceLock<String> = OnceLock::new();
    Q.get_or_init(|| crate::jobs::config().default_queue.clone())
}

fn arg_string(args: &[Value], idx: usize, fn_name: &str) -> Result<String, String> {
    match args.get(idx) {
        Some(Value::String(s)) => Ok(s.clone().to_string()),
        Some(other) => Err(format!(
            "{}() expects string at position {}, got {}",
            fn_name,
            idx + 1,
            other.type_name()
        )),
        None => Err(format!(
            "{}() missing required argument at position {}",
            fn_name,
            idx + 1
        )),
    }
}

fn arg_hash_as_json(args: &[Value], idx: usize) -> Result<serde_json::Value, String> {
    match args.get(idx) {
        Some(Value::Hash(_)) | Some(Value::Array(_)) => value_to_json(&args[idx]),
        Some(Value::Null) | None => Ok(serde_json::Value::Object(serde_json::Map::new())),
        Some(other) => Err(format!(
            "expected hash/array/null at position {}, got {}",
            idx + 1,
            other.type_name()
        )),
    }
}

fn json_to_value_or_null(json: serde_json::Value) -> Value {
    crate::interpreter::value::json_to_value(json).unwrap_or(Value::Null)
}

// ===== Duration parser (for perform_in) =====

/// Parse a "5 minutes" / "1 hour" / "2 days" / "30 seconds" string, or accept
/// a number of seconds directly. Returns seconds.
fn parse_duration(value: &Value) -> Result<i64, String> {
    match value {
        Value::Int(n) => Ok(*n),
        Value::Float(f) => Ok(*f as i64),
        Value::String(s) => parse_duration_str(s),
        other => Err(format!(
            "expected duration string or seconds, got {}",
            other.type_name()
        )),
    }
}

fn parse_duration_str(s: &str) -> Result<i64, String> {
    let trimmed = s.trim();
    let mut split = trimmed.splitn(2, char::is_whitespace);
    let n_part = split.next().ok_or("empty duration")?;
    let unit = split.next().unwrap_or("seconds").trim().to_lowercase();
    let n: i64 = n_part
        .parse()
        .map_err(|_| format!("invalid duration number: {}", n_part))?;
    let multiplier = match unit.as_str() {
        "s" | "sec" | "secs" | "second" | "seconds" => 1,
        "m" | "min" | "mins" | "minute" | "minutes" => 60,
        "h" | "hr" | "hrs" | "hour" | "hours" => 3600,
        "d" | "day" | "days" => 86_400,
        "w" | "wk" | "week" | "weeks" => 604_800,
        other => return Err(format!("unknown duration unit: {}", other)),
    };
    Ok(n * multiplier)
}

fn iso_now_plus_seconds(secs: i64) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    format_iso_utc(now + secs)
}

fn format_iso_utc(unix_seconds: i64) -> String {
    // Minimal RFC 3339 formatter: YYYY-MM-DDTHH:MM:SSZ (UTC).
    // Avoids pulling chrono if the project hasn't already.
    use chrono::{DateTime, Utc};
    let dt = DateTime::<Utc>::from_timestamp(unix_seconds, 0).unwrap_or_else(Utc::now);
    dt.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

// ===== Cron expression helpers =====

// All builders below emit SIX-field cron expressions
// (`sec min hour day-of-month month day-of-week`). The scheduler parses them
// with the `cron` crate, which requires the leading seconds field and rejects
// the 5-field Unix form — `Cron.schedule` surfaces that as an error naming the
// six-field shape rather than accepting a schedule that would never fire.
fn cron_every(arg: &Value) -> Result<String, String> {
    let secs = parse_duration(arg)?;
    if secs < 60 {
        return Err("Cron.every() minimum granularity is 1 minute".to_string());
    }
    let mins = secs / 60;
    let hours = mins / 60;
    let days = hours / 24;
    if days > 0 && hours % 24 == 0 {
        return Ok(format!("0 0 0 */{} * *", days));
    }
    if hours > 0 && mins % 60 == 0 {
        if hours == 1 {
            return Ok("0 0 * * * *".to_string());
        }
        return Ok(format!("0 0 */{} * * *", hours));
    }
    if mins == 1 {
        return Ok("0 * * * * *".to_string());
    }
    Ok(format!("0 */{} * * * *", mins))
}

fn cron_daily_at(time: &str) -> Result<String, String> {
    let (h, m) = parse_hhmm(time)?;
    Ok(format!("0 {} {} * * *", m, h))
}

fn cron_hourly() -> String {
    "0 0 * * * *".to_string()
}

fn cron_weekly_at(day: &str, time: &str) -> Result<String, String> {
    let (h, m) = parse_hhmm(time)?;
    // `cron` numbers day-of-week 1-7 and rejects 0, so emit the three-letter
    // name instead — unambiguous, and Sunday (which `0` would 400 on) is "Sun".
    let dow = match day.to_lowercase().as_str() {
        "sun" | "sunday" | "0" | "7" => "Sun",
        "mon" | "monday" | "1" => "Mon",
        "tue" | "tues" | "tuesday" | "2" => "Tue",
        "wed" | "wednesday" | "3" => "Wed",
        "thu" | "thurs" | "thursday" | "4" => "Thu",
        "fri" | "friday" | "5" => "Fri",
        "sat" | "saturday" | "6" => "Sat",
        other => return Err(format!("Unknown weekday: {}", other)),
    };
    Ok(format!("0 {} {} * * {}", m, h, dow))
}

fn parse_hhmm(time: &str) -> Result<(u32, u32), String> {
    let (h, m) = time
        .split_once(':')
        .ok_or_else(|| format!("expected HH:MM, got {}", time))?;
    let h: u32 = h
        .trim()
        .parse()
        .map_err(|_| format!("invalid hour: {}", h))?;
    let m: u32 = m
        .trim()
        .parse()
        .map_err(|_| format!("invalid minute: {}", m))?;
    if h > 23 || m > 59 {
        return Err(format!("HH:MM out of range: {}", time));
    }
    Ok((h, m))
}

// ===== Job class methods =====

/// Resolve the trailing queue/options argument shared by the job enqueue
/// methods. The argument may be:
///
/// - omitted / `null` → default queue, no extra options
/// - a `String` → queue name (the original positional form)
/// - a `Hash` → `{ queue?, priority?, max_retries? }`; the `queue` key selects
///   the queue (default when absent) and every other key reaches the engine
///   as-is. `priority` is an Int — higher runs first.
///
/// Returns `(queue_name, opts_json_object)` where the opts object carries the
/// scheduling knobs the engine understands (everything but `queue`).
fn job_queue_and_opts(arg: Option<&Value>) -> Result<(String, serde_json::Value), String> {
    let mut queue = default_queue().to_string();
    let mut out = serde_json::Map::new();
    match arg {
        None | Some(Value::Null) => {}
        Some(Value::String(s)) => queue = s.to_string(),
        Some(hash @ Value::Hash(_)) => {
            if let serde_json::Value::Object(map) = value_to_json(hash)? {
                for (k, v) in map {
                    if k == "queue" {
                        if let Some(s) = v.as_str() {
                            queue = s.to_string();
                        }
                    } else {
                        out.insert(k, v);
                    }
                }
            }
        }
        Some(other) => {
            return Err(format!(
                "queue argument must be a queue-name string or an options hash \
                 ({{ queue, priority, max_retries }}), got {}",
                other.type_name()
            ));
        }
    }
    Ok((queue, serde_json::Value::Object(out)))
}

/// Enqueue a named job class with a payload — the same path as `Job.enqueue`,
/// exposed for built-in callers (e.g. `Mailer` `deliver_later`).
pub(crate) fn enqueue(args: &[Value]) -> Result<Value, String> {
    job_enqueue(args)
}

/// Write a job row and log the enqueue for the dev bar.
fn enqueue_doc(mut doc: JobDoc, opts: &serde_json::Value, what: &str) -> Result<Value, String> {
    doc.apply_opts(opts);
    super::job_log::record(&doc);
    let id = store::enqueue(&doc).map_err(|e| format!("{what} failed: {e}"))?;
    Ok(Value::String(id.into()))
}

fn job_enqueue(args: &[Value]) -> Result<Value, String> {
    if args.len() < 2 {
        return Err(
            "Job.enqueue(handler, args, queue_or_opts?) requires at least 2 arguments".to_string(),
        );
    }
    let handler = arg_string(args, 0, "Job.enqueue")?;
    let payload = arg_hash_as_json(args, 1)?;
    let (queue, opts) = job_queue_and_opts(args.get(2))?;
    let doc = JobDoc::new(&handler, payload, &queue, crate::jobs::now_iso());
    enqueue_doc(doc, &opts, "Job.enqueue")
}

fn job_enqueue_in(args: &[Value]) -> Result<Value, String> {
    if args.len() < 3 {
        return Err(
            "Job.enqueue_in(handler, duration, args, queue_or_opts?) requires at least 3 arguments"
                .to_string(),
        );
    }
    let handler = arg_string(args, 0, "Job.enqueue_in")?;
    let secs = parse_duration(&args[1])?;
    let payload = arg_hash_as_json(args, 2)?;
    let (queue, opts) = job_queue_and_opts(args.get(3))?;
    let doc = JobDoc::new(&handler, payload, &queue, iso_now_plus_seconds(secs));
    enqueue_doc(doc, &opts, "Job.enqueue_in")
}

fn job_enqueue_at(args: &[Value]) -> Result<Value, String> {
    if args.len() < 3 {
        return Err(
            "Job.enqueue_at(handler, datetime, args, queue_or_opts?) requires at least 3 arguments"
                .to_string(),
        );
    }
    let handler = arg_string(args, 0, "Job.enqueue_at")?;
    let when = arg_string(args, 1, "Job.enqueue_at")?;
    let payload = arg_hash_as_json(args, 2)?;
    let (queue, opts) = job_queue_and_opts(args.get(3))?;
    let when = normalize_datetime(&when)?;
    let doc = JobDoc::new(&handler, payload, &queue, when);
    enqueue_doc(doc, &opts, "Job.enqueue_at")
}

/// Accept an RFC 3339 timestamp (with or without a `Z`/offset) and re-emit it in
/// the fixed-width UTC form the engine compares against. Validating here means a
/// typo fails at the enqueue call instead of silently never running.
fn normalize_datetime(value: &str) -> Result<String, String> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(value) {
        return Ok(crate::jobs::iso_from_unix(dt.timestamp()));
    }
    // Accept a naive "YYYY-MM-DDTHH:MM:SS" / "YYYY-MM-DD HH:MM:SS" as UTC.
    let candidate = value.trim().replace(' ', "T");
    for fmt in ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M"] {
        if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(&candidate, fmt) {
            return Ok(crate::jobs::iso_from_unix(naive.and_utc().timestamp()));
        }
    }
    Err(format!(
        "invalid datetime {value:?}: expected an ISO-8601 timestamp \
         like \"2026-08-11T03:00:00Z\""
    ))
}

fn job_cancel(args: &[Value]) -> Result<Value, String> {
    let id = arg_string(args, 0, "Job.cancel")?;
    let cancelled = store::cancel(&id).map_err(|e| format!("Job.cancel failed: {e}"))?;
    Ok(Value::Bool(cancelled))
}

fn job_list(args: &[Value]) -> Result<Value, String> {
    // No argument lists every queue; a string narrows to one.
    let queue = match args.first() {
        Some(Value::String(s)) => Some(s.to_string()),
        _ => None,
    };
    let jobs = store::list(queue.as_deref()).map_err(|e| format!("Job.list failed: {e}"))?;
    Ok(json_to_value_or_null(serde_json::Value::Array(jobs)))
}

fn job_queues(_args: &[Value]) -> Result<Value, String> {
    let queues = store::queues().map_err(|e| format!("Job.queues failed: {e}"))?;
    Ok(json_to_value_or_null(serde_json::Value::Array(queues)))
}

// ===== Webhook class methods =====
//
// `Webhook.enqueue(url, payload, opts?)` enqueues a job whose target is the
// given URL rather than a Soli job class. The job engine POSTs the payload with
// `X-Webhook-Signature` (HMAC-SHA256 of the body keyed with `opts["secret"]` or
// `SOLI_WEBHOOK_SECRET`), `X-Webhook-Event: job`, and
// `X-Webhook-Delivery: <job_id>` — the same headers receivers verified before,
// now sent by Soli itself so this works on every database adapter.
//
// `opts` may include:
//   - queue:        String  — queue name (defaults to the engine default)
//   - priority:     Int     — higher first
//   - max_retries:  Int
//   - secret:       String  — per-job HMAC key
//   - headers:      Hash    — extra outgoing HTTP headers

/// Split webhook options into `(queue, engine_opts, secret, headers)`.
type WebhookOpts = (
    String,
    serde_json::Value,
    Option<String>,
    Option<serde_json::Value>,
);

fn webhook_build_opts(opts_arg: Option<&Value>) -> Result<WebhookOpts, String> {
    let mut queue = default_queue().to_string();
    let mut out = serde_json::Map::new();
    let mut secret = None;
    let mut headers = None;

    if let Some(hash @ Value::Hash(_)) = opts_arg {
        if let serde_json::Value::Object(map) = value_to_json(hash)? {
            for (k, v) in map {
                match k.as_str() {
                    "queue" => {
                        if let Some(s) = v.as_str() {
                            queue = s.to_string();
                        }
                    }
                    "secret" => secret = v.as_str().map(str::to_string),
                    "headers" => headers = Some(v),
                    // priority / max_retries reach the engine unchanged.
                    _ => {
                        out.insert(k, v);
                    }
                }
            }
        }
    }

    Ok((queue, serde_json::Value::Object(out), secret, headers))
}

fn webhook_enqueue(args: &[Value]) -> Result<Value, String> {
    if args.len() < 2 {
        return Err(
            "Webhook.enqueue(url, payload, opts?) requires at least 2 arguments".to_string(),
        );
    }
    let url = arg_string(args, 0, "Webhook.enqueue")?;
    let payload = arg_hash_as_json(args, 1)?;
    let (queue, opts, secret, headers) = webhook_build_opts(args.get(2))?;
    let doc = crate::jobs::engine::webhook_job(
        &url,
        payload,
        &queue,
        crate::jobs::now_iso(),
        (secret, headers),
    );
    enqueue_doc(doc, &opts, "Webhook.enqueue")
}

fn webhook_enqueue_in(args: &[Value]) -> Result<Value, String> {
    if args.len() < 3 {
        return Err(
            "Webhook.enqueue_in(url, duration, payload, opts?) requires at least 3 arguments"
                .to_string(),
        );
    }
    let url = arg_string(args, 0, "Webhook.enqueue_in")?;
    let secs = parse_duration(&args[1])?;
    let payload = arg_hash_as_json(args, 2)?;
    let (queue, opts, secret, headers) = webhook_build_opts(args.get(3))?;
    let doc = crate::jobs::engine::webhook_job(
        &url,
        payload,
        &queue,
        iso_now_plus_seconds(secs),
        (secret, headers),
    );
    enqueue_doc(doc, &opts, "Webhook.enqueue_in")
}

fn webhook_enqueue_at(args: &[Value]) -> Result<Value, String> {
    if args.len() < 3 {
        return Err(
            "Webhook.enqueue_at(url, datetime, payload, opts?) requires at least 3 arguments"
                .to_string(),
        );
    }
    let url = arg_string(args, 0, "Webhook.enqueue_at")?;
    let when = normalize_datetime(&arg_string(args, 1, "Webhook.enqueue_at")?)?;
    let payload = arg_hash_as_json(args, 2)?;
    let (queue, opts, secret, headers) = webhook_build_opts(args.get(3))?;
    let doc = crate::jobs::engine::webhook_job(&url, payload, &queue, when, (secret, headers));
    enqueue_doc(doc, &opts, "Webhook.enqueue_at")
}

// ===== Cron class methods =====

fn cron_schedule(args: &[Value]) -> Result<Value, String> {
    if args.len() < 3 {
        return Err(
            "Cron.schedule(name, expr, handler, args?) requires at least 3 arguments".to_string(),
        );
    }
    let name = arg_string(args, 0, "Cron.schedule")?;
    let expr = arg_string(args, 1, "Cron.schedule")?;
    let handler = arg_string(args, 2, "Cron.schedule")?;
    let payload = arg_hash_as_json(args, 3)?;
    // Validated here (not silently at fire time) so a bad expression fails the
    // call that declared it.
    let id = scheduler::upsert(&name, &expr, &handler, payload)
        .map_err(|e| format!("Cron.schedule failed: {e}"))?;
    Ok(Value::String(id.into()))
}

fn cron_list(_args: &[Value]) -> Result<Value, String> {
    let crons = store::list_crons().map_err(|e| format!("Cron.list failed: {e}"))?;
    Ok(json_to_value_or_null(serde_json::Value::Array(crons)))
}

fn cron_update_method(args: &[Value]) -> Result<Value, String> {
    if args.len() < 2 {
        return Err("Cron.update(name, fields_hash) requires 2 arguments".to_string());
    }
    let name = arg_string(args, 0, "Cron.update")?;
    let fields = match &args[1] {
        Value::Hash(_) => value_to_json(&args[1])?,
        other => {
            return Err(format!(
                "Cron.update() expects hash for fields, got {}",
                other.type_name()
            ))
        }
    };
    // A new expression must be valid and must move the schedule position, or
    // the row would keep firing on the old cadence.
    let mut patch = fields.clone();
    if let Some(expr) = fields.get("cron_expression").and_then(|v| v.as_str()) {
        let next = scheduler::next_run_after(expr, crate::jobs::unix_now())
            .map_err(|e| format!("Cron.update failed: {e}"))?;
        if let Some(map) = patch.as_object_mut() {
            map.insert("next_run_at".to_string(), serde_json::json!(next));
        }
    }
    store::update_cron(&name, patch).map_err(|e| format!("Cron.update failed: {e}"))?;
    Ok(Value::Bool(true))
}

fn cron_delete(args: &[Value]) -> Result<Value, String> {
    let name = arg_string(args, 0, "Cron.delete")?;
    store::delete_cron(&name).map_err(|e| format!("Cron.delete failed: {e}"))?;
    Ok(Value::Bool(true))
}

// ===== Class registration =====

pub fn register_jobs_builtins(env: &mut Environment) {
    register_job_class(env);
    register_webhook_class(env);
    register_cron_class(env);

    // Internal: look up a class by name from the current execution env. The
    // pool's job runner uses this to resolve a handler name to its class.
    env.define(
        "__soli_get_class".to_string(),
        Value::NativeFunction(NativeFunction::new("__soli_get_class", Some(1), |args| {
            let name = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "__soli_get_class() expects string, got {}",
                        other.type_name()
                    ))
                }
            };
            use crate::interpreter::executor::current_env_lookup;
            // Prefer the mode-independent job registry (populated for both the
            // interpreter and VM paths); fall back to the interpreter's
            // CURRENT_ENV for any non-job class the caller might request.
            let resolved = lookup_job_class(&name).or_else(|| current_env_lookup(&name));
            Ok(resolved.unwrap_or(Value::Null))
        })),
    );
}

fn register_job_class(env: &mut Environment) {
    let mut statics: HashMap<String, Rc<NativeFunction>> = HashMap::new();
    statics.insert(
        "enqueue".to_string(),
        Rc::new(NativeFunction::new("Job.enqueue", None, job_enqueue)),
    );
    statics.insert(
        "enqueue_in".to_string(),
        Rc::new(NativeFunction::new("Job.enqueue_in", None, job_enqueue_in)),
    );
    statics.insert(
        "enqueue_at".to_string(),
        Rc::new(NativeFunction::new("Job.enqueue_at", None, job_enqueue_at)),
    );
    statics.insert(
        "cancel".to_string(),
        Rc::new(NativeFunction::new("Job.cancel", Some(1), job_cancel)),
    );
    statics.insert(
        "list".to_string(),
        Rc::new(NativeFunction::new("Job.list", None, job_list)),
    );
    statics.insert(
        "queues".to_string(),
        Rc::new(NativeFunction::new("Job.queues", Some(0), job_queues)),
    );

    let class = Class {
        name: "Job".to_string(),
        native_static_methods: statics,
        ..Default::default()
    };
    env.define("Job".to_string(), Value::Class(Rc::new(class)));
}

fn register_webhook_class(env: &mut Environment) {
    let mut statics: HashMap<String, Rc<NativeFunction>> = HashMap::new();
    statics.insert(
        "enqueue".to_string(),
        Rc::new(NativeFunction::new(
            "Webhook.enqueue",
            None,
            webhook_enqueue,
        )),
    );
    statics.insert(
        "enqueue_in".to_string(),
        Rc::new(NativeFunction::new(
            "Webhook.enqueue_in",
            None,
            webhook_enqueue_in,
        )),
    );
    statics.insert(
        "enqueue_at".to_string(),
        Rc::new(NativeFunction::new(
            "Webhook.enqueue_at",
            None,
            webhook_enqueue_at,
        )),
    );
    statics.insert(
        "cancel".to_string(),
        Rc::new(NativeFunction::new("Webhook.cancel", Some(1), job_cancel)),
    );
    statics.insert(
        "list".to_string(),
        Rc::new(NativeFunction::new("Webhook.list", None, job_list)),
    );

    let class = Class {
        name: "Webhook".to_string(),
        native_static_methods: statics,
        ..Default::default()
    };
    env.define("Webhook".to_string(), Value::Class(Rc::new(class)));
}

fn register_cron_class(env: &mut Environment) {
    let mut statics: HashMap<String, Rc<NativeFunction>> = HashMap::new();
    statics.insert(
        "schedule".to_string(),
        Rc::new(NativeFunction::new("Cron.schedule", None, cron_schedule)),
    );
    statics.insert(
        "list".to_string(),
        Rc::new(NativeFunction::new("Cron.list", Some(0), cron_list)),
    );
    statics.insert(
        "update".to_string(),
        Rc::new(NativeFunction::new(
            "Cron.update",
            Some(2),
            cron_update_method,
        )),
    );
    statics.insert(
        "delete".to_string(),
        Rc::new(NativeFunction::new("Cron.delete", Some(1), cron_delete)),
    );
    statics.insert(
        "every".to_string(),
        Rc::new(NativeFunction::new("Cron.every", Some(1), |args| {
            cron_every(&args[0]).map(|s| Value::String(s.into()))
        })),
    );
    statics.insert(
        "daily_at".to_string(),
        Rc::new(NativeFunction::new("Cron.daily_at", Some(1), |args| {
            let s = arg_string(args, 0, "Cron.daily_at")?;
            cron_daily_at(&s).map(|s| Value::String(s.into()))
        })),
    );
    statics.insert(
        "hourly".to_string(),
        Rc::new(NativeFunction::new("Cron.hourly", Some(0), |_| {
            Ok(Value::String(cron_hourly().into()))
        })),
    );
    statics.insert(
        "weekly_at".to_string(),
        Rc::new(NativeFunction::new("Cron.weekly_at", Some(2), |args| {
            let day = arg_string(args, 0, "Cron.weekly_at")?;
            let time = arg_string(args, 1, "Cron.weekly_at")?;
            cron_weekly_at(&day, &time).map(|s| Value::String(s.into()))
        })),
    );

    let class = Class {
        name: "Cron".to_string(),
        native_static_methods: statics,
        ..Default::default()
    };
    env.define("Cron".to_string(), Value::Class(Rc::new(class)));
}

// ===== Facade-method injection =====

/// Inject perform_later / perform_in / perform_at / perform_now / schedule_cron
/// static methods into a user-defined `XJob` class, returning a fresh
/// `Rc<Class>` that the caller should re-define in the environment. Each enqueue
/// facade accepts an optional trailing queue-name string or
/// `{ queue, priority, max_retries }` options hash (see `job_queue_and_opts`).
///
/// User-defined methods on the class take precedence — facade methods are only
/// added when the corresponding name is not already present.
pub fn inject_facade_methods(class: &Class) -> Class {
    let class_name = class.name.clone();
    let mut native_statics = class.native_static_methods.clone();

    let already_defined = |name: &str| {
        class.native_static_methods.contains_key(name) || class.static_methods.contains_key(name)
    };

    // `perform_now` runs the handler inline, in this process, right now — no
    // queue row, no worker. It is how a spec exercises a job, and how a caller
    // opts out of asynchrony.
    if !already_defined("perform_now") {
        let target = Rc::new(class.clone());
        native_statics.insert(
            "perform_now".to_string(),
            Rc::new(NativeFunction::new(
                format!("{}.perform_now", class_name),
                None,
                move |args| perform_now_inline(&target, args),
            )),
        );
    }

    if !already_defined("perform_later") {
        let cn = class_name.clone();
        native_statics.insert(
            "perform_later".to_string(),
            Rc::new(NativeFunction::new(
                format!("{}.perform_later", class_name),
                None,
                move |args| {
                    let mut a = vec![Value::String(cn.clone().into())];
                    a.extend_from_slice(args);
                    job_enqueue(&a)
                },
            )),
        );
    }

    if !already_defined("perform_in") {
        let cn = class_name.clone();
        native_statics.insert(
            "perform_in".to_string(),
            Rc::new(NativeFunction::new(
                format!("{}.perform_in", class_name),
                None,
                move |args| {
                    if args.is_empty() {
                        return Err(format!(
                            "{}.perform_in(duration, args, queue_or_opts?) requires duration",
                            cn
                        ));
                    }
                    let mut a = vec![Value::String(cn.clone().into()), args[0].clone()];
                    if args.len() > 1 {
                        a.push(args[1].clone());
                    } else {
                        a.push(empty_hash());
                    }
                    if args.len() > 2 {
                        a.push(args[2].clone());
                    }
                    job_enqueue_in(&a)
                },
            )),
        );
    }

    if !already_defined("perform_at") {
        let cn = class_name.clone();
        native_statics.insert(
            "perform_at".to_string(),
            Rc::new(NativeFunction::new(
                format!("{}.perform_at", class_name),
                None,
                move |args| {
                    if args.is_empty() {
                        return Err(format!(
                            "{}.perform_at(datetime, args, queue_or_opts?) requires datetime",
                            cn
                        ));
                    }
                    let mut a = vec![Value::String(cn.clone().into()), args[0].clone()];
                    if args.len() > 1 {
                        a.push(args[1].clone());
                    } else {
                        a.push(empty_hash());
                    }
                    if args.len() > 2 {
                        a.push(args[2].clone());
                    }
                    job_enqueue_at(&a)
                },
            )),
        );
    }

    if !already_defined("schedule_cron") {
        let cn = class_name.clone();
        native_statics.insert(
            "schedule_cron".to_string(),
            Rc::new(NativeFunction::new(
                format!("{}.schedule_cron", class_name),
                None,
                move |args| {
                    if args.len() < 2 {
                        return Err(format!(
                            "{}.schedule_cron(name, expr, args?) requires name and expr",
                            cn
                        ));
                    }
                    let mut a = vec![
                        args[0].clone(),
                        args[1].clone(),
                        Value::String(cn.clone().into()),
                    ];
                    if args.len() > 2 {
                        a.push(args[2].clone());
                    }
                    cron_schedule(&a)
                },
            )),
        );
    }

    Class::new(
        class.name.clone(),
        class.superclass.clone(),
        class.methods.borrow().clone(),
        class.static_methods.clone(),
        native_statics,
        class.native_methods.clone(),
        class.static_fields.clone(),
        class.fields.clone(),
        class.constructor.clone(),
        class.nested_classes.clone(),
    )
}

/// Run a job class's `perform` in the calling process, synchronously.
///
/// Backs `XJob.perform_now(args)` and the engine's dispatch of a claimed row on
/// interpreters that have the class loaded. Handles all three shapes a static
/// method can take (native, tree-walking, bytecode) so it works in dev and prod.
fn perform_now_inline(class: &Rc<Class>, args: &[Value]) -> Result<Value, String> {
    let call_args: Vec<Value> = if args.is_empty() {
        vec![empty_hash()]
    } else {
        args.to_vec()
    };

    if let Some(native) = class.native_static_methods.get("perform") {
        return (native.func)(&call_args);
    }
    if let Some(closure) = class.find_vm_static_method("perform") {
        let mut interpreter = crate::interpreter::Interpreter::default();
        return interpreter
            .call_value(
                Value::VmClosure(closure),
                call_args,
                crate::span::Span::default(),
            )
            .map_err(|e| e.to_string());
    }
    if let Some(method) = class.find_static_method("perform") {
        let mut interpreter = crate::interpreter::Interpreter::default();
        return interpreter
            .call_function_with_this(&method, Some(Value::Class(class.clone())), call_args)
            .map_err(|e| e.to_string());
    }
    Err(format!(
        "{} has no `static def perform(args)` — a job class must define one",
        class.name
    ))
}

/// Read a `static cron` field from a class; returns the string if present.
pub fn read_static_cron(class: &Class) -> Option<String> {
    let fields = class.static_fields.borrow();
    match fields.get("cron") {
        Some(Value::String(s)) => Some(s.clone().to_string()),
        _ => None,
    }
}

/// Whether a job class declares `static background: Bool = true`.
///
/// Retained for compatibility: under the Soli job engine every job already runs
/// on the worker pool rather than a request thread, so this flag no longer
/// changes behavior.
pub fn read_static_background(class: &Class) -> bool {
    matches!(
        class.static_fields.borrow().get("background"),
        Some(Value::Bool(true))
    )
}

/// Idempotently register a `static cron`-declared schedule. Equivalent to
/// `Cron.schedule(name, expr, handler, {})` but callable from Rust during
/// worker boot.
pub fn register_static_cron(name: &str, expr: &str, handler: &str) -> Result<String, String> {
    scheduler::upsert(name, expr, handler, serde_json::json!({}))
}

/// Convert a `EmailJob` class name to a snake-case cron name like
/// `email_job` (matches the file naming convention).
pub fn class_name_to_snake(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (i, ch) in name.chars().enumerate() {
        if ch.is_uppercase() && i > 0 {
            out.push('_');
        }
        out.extend(ch.to_lowercase());
    }
    out
}

#[cfg(test)]
mod job_opts_tests {
    //! `job_queue_and_opts` is the single point where the trailing enqueue
    //! argument is interpreted, so these tests pin all four shapes: omitted,
    //! a bare queue-name string (the original form), and an options hash with
    //! or without an explicit `queue`. The `priority` / `max_retries` knobs
    //! must survive into the forwarded opts object unchanged.
    use super::{default_queue, job_queue_and_opts};
    use crate::interpreter::value::{HashKey, HashPairs, Value};
    use std::cell::RefCell;
    use std::rc::Rc;

    fn hash(pairs: &[(&str, Value)]) -> Value {
        let mut hp = HashPairs::default();
        for (k, v) in pairs {
            hp.insert(HashKey::String((*k).into()), v.clone());
        }
        Value::Hash(Rc::new(RefCell::new(hp)))
    }

    #[test]
    fn omitted_uses_default_queue_and_no_opts() {
        let (queue, opts) = job_queue_and_opts(None).unwrap();
        assert_eq!(queue, default_queue());
        assert_eq!(opts, serde_json::json!({}));
    }

    #[test]
    fn null_is_treated_as_omitted() {
        let (queue, opts) = job_queue_and_opts(Some(&Value::Null)).unwrap();
        assert_eq!(queue, default_queue());
        assert_eq!(opts, serde_json::json!({}));
    }

    #[test]
    fn bare_string_selects_queue_with_no_opts() {
        let arg = Value::String("mailers".into());
        let (queue, opts) = job_queue_and_opts(Some(&arg)).unwrap();
        assert_eq!(queue, "mailers");
        assert_eq!(opts, serde_json::json!({}));
    }

    #[test]
    fn hash_selects_queue_and_forwards_priority() {
        let arg = hash(&[
            ("queue", Value::String("high".into())),
            ("priority", Value::Int(10)),
            ("max_retries", Value::Int(3)),
        ]);
        let (queue, opts) = job_queue_and_opts(Some(&arg)).unwrap();
        assert_eq!(queue, "high");
        assert_eq!(
            opts,
            serde_json::json!({ "priority": 10, "max_retries": 3 })
        );
    }

    #[test]
    fn hash_without_queue_keeps_default_but_keeps_priority() {
        let arg = hash(&[("priority", Value::Int(5))]);
        let (queue, opts) = job_queue_and_opts(Some(&arg)).unwrap();
        assert_eq!(queue, default_queue());
        assert_eq!(opts, serde_json::json!({ "priority": 5 }));
    }

    #[test]
    fn wrong_type_is_rejected() {
        assert!(job_queue_and_opts(Some(&Value::Int(7))).is_err());
    }
}

#[cfg(test)]
mod cron_expr_tests {
    //! The `Cron.*` expression builders must emit SIX-field expressions
    //! (`sec min hour day-of-month month day-of-week`). The scheduler parses
    //! them with the `cron` crate, which rejects the 5-field Unix form — a
    //! 5-field expression would be refused at declaration time. These tests pin
    //! the exact output AND parse it with the same crate so that regression
    //! can't come back.
    use super::{cron_daily_at, cron_every, cron_hourly, cron_weekly_at};
    use crate::interpreter::value::Value;
    use cron::Schedule;
    use std::str::FromStr;

    fn assert_valid(expr: &str) {
        assert_eq!(
            expr.split_whitespace().count(),
            6,
            "expression must have 6 fields: {:?}",
            expr
        );
        assert!(
            Schedule::from_str(expr).is_ok(),
            "expression must parse with the `cron` crate the scheduler uses: {:?}",
            expr
        );
    }

    fn every(spec: &str) -> String {
        cron_every(&Value::String(spec.to_string().into())).expect("cron_every")
    }

    #[test]
    fn every_emits_valid_minute_schedules() {
        assert_eq!(every("5 minutes"), "0 */5 * * * *");
        assert_eq!(every("15 minutes"), "0 */15 * * * *");
        assert_eq!(every("1 minute"), "0 * * * * *");
        for spec in ["5 minutes", "15 minutes", "1 minute"] {
            assert_valid(&every(spec));
        }
    }

    #[test]
    fn every_emits_valid_hour_and_day_schedules() {
        assert_eq!(every("1 hour"), "0 0 * * * *");
        assert_eq!(every("2 hours"), "0 0 */2 * * *");
        assert_eq!(every("1 day"), "0 0 0 */1 * *");
        assert_eq!(every("3 days"), "0 0 0 */3 * *");
        for spec in ["1 hour", "2 hours", "1 day", "3 days"] {
            assert_valid(&every(spec));
        }
    }

    #[test]
    fn every_rejects_sub_minute_granularity() {
        assert!(cron_every(&Value::String("30 seconds".to_string().into())).is_err());
    }

    #[test]
    fn daily_at_and_hourly_emit_valid_expressions() {
        assert_eq!(cron_daily_at("03:00").unwrap(), "0 0 3 * * *");
        assert_eq!(cron_daily_at("23:45").unwrap(), "0 45 23 * * *");
        assert_eq!(cron_hourly(), "0 0 * * * *");
        assert_valid(&cron_daily_at("03:00").unwrap());
        assert_valid(&cron_daily_at("23:45").unwrap());
        assert_valid(&cron_hourly());
    }

    #[test]
    fn weekly_at_uses_weekday_names_so_sunday_is_valid() {
        assert_eq!(cron_weekly_at("monday", "09:00").unwrap(), "0 0 9 * * Mon");
        // `cron` rejects day-of-week 0, so Sunday is emitted as the name "Sun".
        assert_eq!(cron_weekly_at("sunday", "00:00").unwrap(), "0 0 0 * * Sun");
        assert_eq!(cron_weekly_at("fri", "17:30").unwrap(), "0 30 17 * * Fri");
        for (day, time) in [("monday", "09:00"), ("sunday", "00:00"), ("fri", "17:30")] {
            assert_valid(&cron_weekly_at(day, time).unwrap());
        }
    }
}

#[cfg(test)]
mod facade_tests {
    //! `perform_now` runs a handler inline, with no queue and no database — it
    //! is how a spec exercises job logic. These tests also pin the facade
    //! injection contract: user-defined methods always win over injected ones.
    use super::*;
    use std::cell::Cell;

    thread_local! {
        // Per-thread so the parallel test runner can't cross-count calls.
        static CALLS: Cell<usize> = const { Cell::new(0) };
    }

    /// A job class whose `perform` is a native static, so the test needs no
    /// interpreter setup.
    fn job_class(name: &str) -> Class {
        let mut statics: HashMap<String, Rc<NativeFunction>> = HashMap::new();
        statics.insert(
            "perform".to_string(),
            Rc::new(NativeFunction::new("perform", None, |args| {
                CALLS.with(|c| c.set(c.get() + 1));
                // Echo the args back so the caller can assert they arrived.
                Ok(args.first().cloned().unwrap_or(Value::Null))
            })),
        );
        Class {
            name: name.to_string(),
            native_static_methods: statics,
            ..Default::default()
        }
    }

    fn call_static(class: &Class, method: &str, args: &[Value]) -> Result<Value, String> {
        let f = class
            .native_static_methods
            .get(method)
            .unwrap_or_else(|| panic!("{method} should be injected"));
        (f.func)(args)
    }

    #[test]
    fn injection_adds_the_full_facade_set() {
        let injected = inject_facade_methods(&job_class("EmailJob"));
        for name in [
            "perform_now",
            "perform_later",
            "perform_in",
            "perform_at",
            "schedule_cron",
        ] {
            assert!(
                injected.native_static_methods.contains_key(name),
                "{name} should be injected"
            );
        }
    }

    #[test]
    fn perform_now_runs_the_handler_immediately_and_returns_its_value() {
        let injected = inject_facade_methods(&job_class("EmailJob"));
        let before = CALLS.with(|c| c.get());

        let args = crate::interpreter::value::json_to_value(serde_json::json!({"to": "a@b.c"}))
            .expect("args");
        let result = call_static(&injected, "perform_now", &[args]).expect("perform_now");

        assert_eq!(
            CALLS.with(|c| c.get()),
            before + 1,
            "perform_now must call perform exactly once, inline"
        );
        // The handler's return value reaches the caller (so a spec can assert it).
        match result {
            Value::Hash(h) => {
                let borrowed = h.borrow();
                let to = borrowed
                    .get(&crate::interpreter::value::HashKey::String("to".into()))
                    .cloned();
                assert!(matches!(to, Some(Value::String(s)) if s.as_ref() == "a@b.c"));
            }
            other => panic!("expected the handler's hash back, got {other:?}"),
        }
    }

    #[test]
    fn perform_now_defaults_missing_args_to_an_empty_hash() {
        // `Job.perform_now()` with no argument must still call perform(args)
        // rather than failing an arity check.
        let injected = inject_facade_methods(&job_class("NoArgJob"));
        let result = call_static(&injected, "perform_now", &[]).expect("perform_now");
        assert!(matches!(result, Value::Hash(_)));
    }

    #[test]
    fn perform_now_reports_a_missing_perform_clearly() {
        // A class with no `perform` is a user error worth naming precisely.
        let bare = Class {
            name: "BrokenJob".to_string(),
            ..Default::default()
        };
        let injected = inject_facade_methods(&bare);
        let err = call_static(&injected, "perform_now", &[]).expect_err("must error");
        assert!(err.contains("BrokenJob"), "{err}");
        assert!(err.contains("perform"), "{err}");
    }

    #[test]
    fn user_defined_methods_are_not_overwritten() {
        // If an app defines its own perform_now/perform_later, injection must
        // leave it alone.
        let mut class = job_class("CustomJob");
        class.native_static_methods.insert(
            "perform_now".to_string(),
            Rc::new(NativeFunction::new("custom", None, |_| {
                Ok(Value::String("custom".into()))
            })),
        );
        let injected = inject_facade_methods(&class);
        let result = call_static(&injected, "perform_now", &[]).expect("custom perform_now");
        assert!(matches!(result, Value::String(s) if s.as_ref() == "custom"));
    }
}
