//! Background jobs and cron scheduling, backed by SolidB.
//!
//! Exposes two static-method-only classes:
//! - `Job` — enqueue, schedule, list, cancel queue jobs.
//! - `Cron` — manage recurring jobs and build cron expressions.
//!
//! Job handlers are user-defined classes in `app/jobs/*.sl`. SolidB triggers
//! a handler by POSTing to `/_jobs/run/:name` on the Soli app; the callback
//! route invokes the class's `static fn perform(args)`.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::OnceLock;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{empty_hash, value_to_json, Class, NativeFunction, Value};
use crate::solidb_http::SoliDBClient;

/// Static configuration for the jobs system, sourced from env vars on first use.
struct JobsConfig {
    database: String,
    default_queue: String,
    callback_url: String,
}

impl JobsConfig {
    fn from_env() -> Self {
        let database = std::env::var("SOLI_JOBS_DATABASE")
            .or_else(|_| std::env::var("SOLIDB_DATABASE"))
            .unwrap_or_else(|_| "default".to_string());
        let default_queue =
            std::env::var("SOLI_JOBS_DEFAULT_QUEUE").unwrap_or_else(|_| "default".to_string());
        let callback_url = std::env::var("SOLI_JOBS_CALLBACK_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:3000/_jobs/run".to_string());
        Self {
            database,
            default_queue,
            callback_url,
        }
    }
}

fn jobs_config() -> &'static JobsConfig {
    static CFG: OnceLock<JobsConfig> = OnceLock::new();
    CFG.get_or_init(JobsConfig::from_env)
}

/// Build a SolidB client wired up with the configured database and credentials
/// from the existing model DB config (so users don't have to set up auth twice).
fn make_client() -> Result<SoliDBClient, String> {
    use crate::interpreter::builtins::model::core::{
        get_api_key, get_basic_auth, get_jwt_token, DB_CONFIG,
    };
    let host = &DB_CONFIG.host;
    let mut client =
        SoliDBClient::connect(host).map_err(|e| format!("SolidB connect failed: {}", e))?;
    if let Some(jwt) = get_jwt_token() {
        client = client.with_jwt_token(jwt);
    } else if let Some(key) = get_api_key() {
        client = client.with_api_key(key);
    } else if let Some(basic) = get_basic_auth() {
        // The cached basic auth header is already "Basic <base64>". Decode to
        // recover username/password so we can hand them to the client builder.
        if let Some(rest) = basic.strip_prefix("Basic ") {
            use base64::{engine::general_purpose::STANDARD, Engine as _};
            if let Ok(decoded_bytes) = STANDARD.decode(rest) {
                if let Ok(s) = String::from_utf8(decoded_bytes) {
                    if let Some((u, p)) = s.split_once(':') {
                        client = client.with_basic_auth(u, p);
                    }
                }
            }
        }
    }
    client.set_database(&jobs_config().database);
    Ok(client)
}

fn callback_for(handler: &str) -> String {
    let base = jobs_config().callback_url.trim_end_matches('/');
    format!("{}/{}", base, handler)
}

fn arg_string(args: &[Value], idx: usize, fn_name: &str) -> Result<String, String> {
    match args.get(idx) {
        Some(Value::String(s)) => Ok(s.clone()),
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

fn cron_every(arg: &Value) -> Result<String, String> {
    let secs = parse_duration(arg)?;
    if secs < 60 {
        return Err("Cron.every() minimum granularity is 1 minute".to_string());
    }
    let mins = secs / 60;
    let hours = mins / 60;
    let days = hours / 24;
    if days > 0 && hours % 24 == 0 {
        return Ok(format!("0 0 */{} * *", days));
    }
    if hours > 0 && mins % 60 == 0 {
        if hours == 1 {
            return Ok("0 * * * *".to_string());
        }
        return Ok(format!("0 */{} * * *", hours));
    }
    if mins == 1 {
        return Ok("* * * * *".to_string());
    }
    Ok(format!("*/{} * * * *", mins))
}

fn cron_daily_at(time: &str) -> Result<String, String> {
    let (h, m) = parse_hhmm(time)?;
    Ok(format!("{} {} * * *", m, h))
}

fn cron_hourly() -> String {
    "0 * * * *".to_string()
}

fn cron_weekly_at(day: &str, time: &str) -> Result<String, String> {
    let (h, m) = parse_hhmm(time)?;
    let dow = match day.to_lowercase().as_str() {
        "sun" | "sunday" | "0" => 0,
        "mon" | "monday" | "1" => 1,
        "tue" | "tues" | "tuesday" | "2" => 2,
        "wed" | "wednesday" | "3" => 3,
        "thu" | "thurs" | "thursday" | "4" => 4,
        "fri" | "friday" | "5" => 5,
        "sat" | "saturday" | "6" => 6,
        other => return Err(format!("Unknown weekday: {}", other)),
    };
    Ok(format!("{} {} * * {}", m, h, dow))
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

fn job_enqueue(args: Vec<Value>) -> Result<Value, String> {
    if args.len() < 2 {
        return Err("Job.enqueue(handler, args, queue?) requires at least 2 arguments".to_string());
    }
    let handler = arg_string(&args, 0, "Job.enqueue")?;
    let payload = arg_hash_as_json(&args, 1)?;
    let queue = match args.get(2) {
        Some(Value::String(s)) => s.clone(),
        _ => jobs_config().default_queue.clone(),
    };
    let client = make_client()?;
    let callback = callback_for(&handler);
    let id = client
        .enqueue_job(&queue, &handler, payload, &callback, None)
        .map_err(|e| format!("Job.enqueue failed: {}", e))?;
    Ok(Value::String(id))
}

fn job_enqueue_in(args: Vec<Value>) -> Result<Value, String> {
    if args.len() < 3 {
        return Err(
            "Job.enqueue_in(handler, duration, args, queue?) requires at least 3 arguments"
                .to_string(),
        );
    }
    let handler = arg_string(&args, 0, "Job.enqueue_in")?;
    let secs = parse_duration(&args[1])?;
    let payload = arg_hash_as_json(&args, 2)?;
    let queue = match args.get(3) {
        Some(Value::String(s)) => s.clone(),
        _ => jobs_config().default_queue.clone(),
    };
    let when = iso_now_plus_seconds(secs);
    let client = make_client()?;
    let callback = callback_for(&handler);
    let id = client
        .enqueue_job(&queue, &handler, payload, &callback, Some(&when))
        .map_err(|e| format!("Job.enqueue_in failed: {}", e))?;
    Ok(Value::String(id))
}

fn job_enqueue_at(args: Vec<Value>) -> Result<Value, String> {
    if args.len() < 3 {
        return Err(
            "Job.enqueue_at(handler, datetime, args, queue?) requires at least 3 arguments"
                .to_string(),
        );
    }
    let handler = arg_string(&args, 0, "Job.enqueue_at")?;
    let when = arg_string(&args, 1, "Job.enqueue_at")?;
    let payload = arg_hash_as_json(&args, 2)?;
    let queue = match args.get(3) {
        Some(Value::String(s)) => s.clone(),
        _ => jobs_config().default_queue.clone(),
    };
    let client = make_client()?;
    let callback = callback_for(&handler);
    let id = client
        .enqueue_job(&queue, &handler, payload, &callback, Some(&when))
        .map_err(|e| format!("Job.enqueue_at failed: {}", e))?;
    Ok(Value::String(id))
}

fn job_cancel(args: Vec<Value>) -> Result<Value, String> {
    let id = arg_string(&args, 0, "Job.cancel")?;
    let client = make_client()?;
    client
        .cancel_job(&id)
        .map_err(|e| format!("Job.cancel failed: {}", e))?;
    Ok(Value::Bool(true))
}

fn job_list(args: Vec<Value>) -> Result<Value, String> {
    let queue = match args.first() {
        Some(Value::String(s)) => s.clone(),
        _ => jobs_config().default_queue.clone(),
    };
    let client = make_client()?;
    let jobs = client
        .list_jobs(&queue)
        .map_err(|e| format!("Job.list failed: {}", e))?;
    Ok(json_to_value_or_null(serde_json::Value::Array(jobs)))
}

fn job_queues(_args: Vec<Value>) -> Result<Value, String> {
    let client = make_client()?;
    let queues = client
        .list_queues()
        .map_err(|e| format!("Job.queues failed: {}", e))?;
    Ok(json_to_value_or_null(serde_json::Value::Array(queues)))
}

// ===== Cron class methods =====

fn cron_schedule(args: Vec<Value>) -> Result<Value, String> {
    if args.len() < 3 {
        return Err(
            "Cron.schedule(name, expr, handler, args?) requires at least 3 arguments".to_string(),
        );
    }
    let name = arg_string(&args, 0, "Cron.schedule")?;
    let expr = arg_string(&args, 1, "Cron.schedule")?;
    let handler = arg_string(&args, 2, "Cron.schedule")?;
    let payload = arg_hash_as_json(&args, 3)?;

    let client = make_client()?;
    let callback = callback_for(&handler);

    // Upsert by name: look up existing entry, update if found else create.
    let existing = client
        .list_crons()
        .map_err(|e| format!("Cron.schedule list failed: {}", e))?;
    let existing_id = existing.iter().find_map(|entry| {
        let entry_name = entry.get("name").and_then(|v| v.as_str())?;
        if entry_name == name {
            entry
                .get("id")
                .or_else(|| entry.get("_key"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        } else {
            None
        }
    });

    let id = match existing_id {
        Some(id) => {
            let fields = serde_json::json!({
                "schedule": expr,
                "handler": handler,
                "args": payload,
                "callback_url": callback,
            });
            client
                .update_cron(&id, fields)
                .map_err(|e| format!("Cron.schedule update failed: {}", e))?;
            id
        }
        None => client
            .create_cron(&name, &expr, &handler, payload, &callback)
            .map_err(|e| format!("Cron.schedule create failed: {}", e))?,
    };
    Ok(Value::String(id))
}

fn cron_list(_args: Vec<Value>) -> Result<Value, String> {
    let client = make_client()?;
    let crons = client
        .list_crons()
        .map_err(|e| format!("Cron.list failed: {}", e))?;
    Ok(json_to_value_or_null(serde_json::Value::Array(crons)))
}

fn cron_update_method(args: Vec<Value>) -> Result<Value, String> {
    if args.len() < 2 {
        return Err("Cron.update(id, fields_hash) requires 2 arguments".to_string());
    }
    let id = arg_string(&args, 0, "Cron.update")?;
    let fields = match &args[1] {
        Value::Hash(_) => value_to_json(&args[1])?,
        other => {
            return Err(format!(
                "Cron.update() expects hash for fields, got {}",
                other.type_name()
            ))
        }
    };
    let client = make_client()?;
    client
        .update_cron(&id, fields)
        .map_err(|e| format!("Cron.update failed: {}", e))?;
    Ok(Value::Bool(true))
}

fn cron_delete(args: Vec<Value>) -> Result<Value, String> {
    let id = arg_string(&args, 0, "Cron.delete")?;
    let client = make_client()?;
    client
        .delete_cron(&id)
        .map_err(|e| format!("Cron.delete failed: {}", e))?;
    Ok(Value::Bool(true))
}

// ===== Class registration =====

pub fn register_jobs_builtins(env: &mut Environment) {
    register_job_class(env);
    register_cron_class(env);

    // Internal: look up a class by name from the current execution env.
    // Used by the SolidB-webhook callback handler to dispatch to a job class
    // discovered at request time.
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
            Ok(current_env_lookup(&name).unwrap_or(Value::Null))
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
            cron_every(&args[0]).map(Value::String)
        })),
    );
    statics.insert(
        "daily_at".to_string(),
        Rc::new(NativeFunction::new("Cron.daily_at", Some(1), |args| {
            let s = arg_string(&args, 0, "Cron.daily_at")?;
            cron_daily_at(&s).map(Value::String)
        })),
    );
    statics.insert(
        "hourly".to_string(),
        Rc::new(NativeFunction::new("Cron.hourly", Some(0), |_| {
            Ok(Value::String(cron_hourly()))
        })),
    );
    statics.insert(
        "weekly_at".to_string(),
        Rc::new(NativeFunction::new("Cron.weekly_at", Some(2), |args| {
            let day = arg_string(&args, 0, "Cron.weekly_at")?;
            let time = arg_string(&args, 1, "Cron.weekly_at")?;
            cron_weekly_at(&day, &time).map(Value::String)
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

/// Inject perform_now / perform_later / perform_in / perform_at / set / schedule_cron
/// static methods into a user-defined `XJob` class, returning a fresh `Rc<Class>`
/// that the caller should re-define in the environment.
///
/// User-defined methods on the class take precedence — facade methods are only
/// added when the corresponding name is not already present.
pub fn inject_facade_methods(class: &Class) -> Class {
    let class_name = class.name.clone();
    let mut native_statics = class.native_static_methods.clone();

    let already_defined = |name: &str| {
        class.native_static_methods.contains_key(name) || class.static_methods.contains_key(name)
    };

    if !already_defined("perform_later") {
        let cn = class_name.clone();
        native_statics.insert(
            "perform_later".to_string(),
            Rc::new(NativeFunction::new(
                format!("{}.perform_later", class_name),
                None,
                move |args| {
                    let mut a = vec![Value::String(cn.clone())];
                    a.extend(args);
                    job_enqueue(a)
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
                            "{}.perform_in(duration, args, queue?) requires duration",
                            cn
                        ));
                    }
                    let mut a = vec![Value::String(cn.clone()), args[0].clone()];
                    if args.len() > 1 {
                        a.push(args[1].clone());
                    } else {
                        a.push(empty_hash());
                    }
                    if args.len() > 2 {
                        a.push(args[2].clone());
                    }
                    job_enqueue_in(a)
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
                            "{}.perform_at(datetime, args, queue?) requires datetime",
                            cn
                        ));
                    }
                    let mut a = vec![Value::String(cn.clone()), args[0].clone()];
                    if args.len() > 1 {
                        a.push(args[1].clone());
                    } else {
                        a.push(empty_hash());
                    }
                    if args.len() > 2 {
                        a.push(args[2].clone());
                    }
                    job_enqueue_at(a)
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
                    let mut a = vec![args[0].clone(), args[1].clone(), Value::String(cn.clone())];
                    if args.len() > 2 {
                        a.push(args[2].clone());
                    }
                    cron_schedule(a)
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

/// Read a `static cron` field from a class; returns the string if present.
pub fn read_static_cron(class: &Class) -> Option<String> {
    let fields = class.static_fields.borrow();
    match fields.get("cron") {
        Some(Value::String(s)) => Some(s.clone()),
        _ => None,
    }
}

/// Idempotently register a `static cron`-declared schedule against SolidB.
/// Equivalent to `Cron.schedule(name, expr, handler, {})` but callable from
/// Rust during worker boot.
pub fn register_static_cron(name: &str, expr: &str, handler: &str) -> Result<String, String> {
    let args = vec![
        Value::String(name.to_string()),
        Value::String(expr.to_string()),
        Value::String(handler.to_string()),
    ];
    match cron_schedule(args)? {
        Value::String(id) => Ok(id),
        _ => Ok(String::new()),
    }
}

/// Soli prelude that defines the SolidB-webhook callback handler. Loaded once
/// per worker. Looks up the matching XJob class by name, calls its `perform`
/// method with the supplied args, and returns 200/503/500.
pub const JOBS_CALLBACK_PRELUDE: &str = r#"
fn __soli_jobs_run(req) {
    let name = req["params"]["name"];
    let cls = __soli_get_class(name);
    if cls == null {
        return {"status": 503, "body": "Job class not loaded: " + str(name)};
    }
    let payload = req["json"];
    let job_args = {};
    if payload != null {
        let candidate = payload["args"];
        if candidate != null {
            job_args = candidate;
        }
    }
    try {
        cls.perform(job_args);
        return {"status": 200, "body": "ok"};
    } catch err {
        print("Job " + str(name) + " failed: " + str(err));
        return {"status": 500, "body": "job error: " + str(err)};
    }
}
"#;

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
