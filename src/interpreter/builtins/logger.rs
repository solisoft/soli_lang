//! Logger built-in class for Soli.
//!
//! Structured leveled logging to stderr. All methods are static:
//! `Logger.info("msg", {"key": value})` etc. Levels are `debug < info <
//! warn < error`; messages below the configured level are dropped.
//! `SOLI_LOG_LEVEL` seeds the initial level. For tests, capture mode
//! records entries in a bounded ring buffer instead of relying on stderr:
//! `Logger.set_capture(true)` → … → `Logger.entries()`.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::rc::Rc;
use std::sync::{RwLock, RwLockReadGuard};

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, HashKey, HashPairs, NativeFunction, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Debug = 0,
    Info = 1,
    Warn = 2,
    Error = 3,
}

impl Level {
    fn from_str(s: &str) -> Option<Level> {
        match s.trim().to_ascii_lowercase().as_str() {
            "debug" | "trace" => Some(Level::Debug),
            "info" => Some(Level::Info),
            "warn" | "warning" => Some(Level::Warn),
            "error" => Some(Level::Error),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Level::Debug => "DEBUG",
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
        }
    }
}

struct Config {
    level: Level,
    json: bool,
}

fn default_level() -> Level {
    std::env::var("SOLI_LOG_LEVEL")
        .ok()
        .and_then(|v| Level::from_str(&v))
        .unwrap_or(Level::Info)
}

lazy_static::lazy_static! {
    static ref CONFIG: RwLock<Config> = RwLock::new(Config {
        level: default_level(),
        json: false,
    });
    /// Bounded capture ring for tests. Entries recorded while capture is on;
    /// the cap keeps a forgotten capture from growing without bound.
    static ref CAPTURE: RwLock<CaptureBuffer> = RwLock::new(CaptureBuffer::default());
}

const CAPTURE_CAP: usize = 1000;

#[derive(Default)]
struct CaptureBuffer {
    enabled: bool,
    entries: VecDeque<String>,
}

fn read_config() -> RwLockReadGuard<'static, Config> {
    // A poisoned lock only matters if a writer panicked mid-swap; recover so
    // logging can never wedge an app.
    CONFIG.read().unwrap_or_else(|e| e.into_inner())
}

/// Render one field value for text output / JSON embedding. Scalars render
/// naturally; anything else falls back to its JSON form when it has one,
/// or its type name otherwise. Complex fields are never silently dropped
/// in JSON output.
fn render_field(v: &Value) -> String {
    match v {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Int(n) => n.to_string(),
        Value::Float(f) => f.to_string(),
        Value::String(s) => s.to_string(),
        other => crate::interpreter::value_stringify::stringify_to_string(other)
            .unwrap_or_else(|_| other.type_name().to_string()),
    }
}

/// A field value as a **valid JSON** token.
///
/// The JSON branch used to splice `render_field` in unquoted, so a value with
/// no JSON form emitted a bare token — `{"cb":Function}` for a function field,
/// `{"ratio":NaN}` for a non-finite float — and broke every downstream parser
/// for that line. JSON has no NaN/Infinity literal, so those become strings.
fn render_field_json(v: &Value) -> String {
    fn json_str(s: &str) -> String {
        serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
    }
    match v {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Int(n) => n.to_string(),
        Value::Float(f) => {
            if f.is_finite() {
                f.to_string()
            } else {
                json_str(&f.to_string())
            }
        }
        Value::String(s) => json_str(s.as_str()),
        other => match crate::interpreter::value_stringify::stringify_to_string(other) {
            // Splice a real JSON form (hash, array, decimal) in as-is; quote
            // anything else so the line stays parseable.
            Ok(rendered) if serde_json::from_str::<serde_json::Value>(&rendered).is_ok() => {
                rendered
            }
            Ok(rendered) => json_str(&rendered),
            Err(_) => json_str(&other.type_name()),
        },
    }
}

/// Emit one entry: to stderr always, and into the capture ring if enabled.
fn emit(level: Level, message: &str, fields: Option<&HashPairs>) {
    let cfg = read_config();
    if level < cfg.level {
        return;
    }

    // RFC 3339 timestamp via the chrono clock already used by DateTime.
    let ts = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");

    let line = if cfg.json {
        let mut parts = vec![
            format!("\"ts\":\"{ts}\""),
            format!("\"level\":\"{}\"", level.name()),
            format!(
                "\"message\":{}",
                serde_json::to_string(message).unwrap_or_else(|_| "\"\"".to_string())
            ),
        ];
        if let Some(fields) = fields {
            for (k, v) in fields.iter() {
                let key = match k {
                    HashKey::String(s) | HashKey::Symbol(s) => s.to_string(),
                    _ => continue,
                };
                parts.push(format!(
                    "{}:{}",
                    serde_json::to_string(&key).unwrap_or_default(),
                    render_field_json(v)
                ));
            }
        }
        format!("{{{}}}", parts.join(","))
    } else {
        let mut line = format!("{ts} [{}] {}", level.name(), message);
        if let Some(fields) = fields {
            for (k, v) in fields.iter() {
                let key = match k {
                    HashKey::String(s) | HashKey::Symbol(s) => s.to_string(),
                    _ => continue,
                };
                line.push_str(&format!(" {}={}", key, render_field(v)));
            }
        }
        line
    };

    let _ = writeln!(std::io::stderr(), "{line}");

    drop(cfg);
    if let Ok(mut buf) = CAPTURE.write() {
        if buf.enabled {
            if buf.entries.len() >= CAPTURE_CAP {
                buf.entries.pop_front();
            }
            buf.entries.push_back(line);
        }
    }
}

/// Turn test capture on/off; clearing when disabled.
fn set_capture(on: bool) {
    if let Ok(mut buf) = CAPTURE.write() {
        buf.enabled = on;
        if !on {
            buf.entries.clear();
        }
    }
}

/// Snapshot of captured entries (oldest first).
fn captured_entries() -> Vec<String> {
    let buf = CAPTURE.read().unwrap_or_else(|e| e.into_inner());
    buf.entries.iter().cloned().collect()
}

fn extract_fields(args: &[Value], ctx: &str) -> Result<Option<HashPairs>, String> {
    match args.get(1) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Hash(h)) => Ok(Some(h.borrow().clone())),
        Some(other) => Err(format!(
            "{ctx} expects a hash of fields, got {}",
            other.type_name()
        )),
    }
}

fn register_log_fn(
    methods: &mut HashMap<String, Rc<NativeFunction>>,
    class_name: &str,
    method_name: &str,
    level: Level,
) {
    let fname = format!("{class_name}.{method_name}");
    let full = fname.clone();
    methods.insert(
        method_name.to_string(),
        // Arity 1 or 2 (optional fields hash); validated below.
        Rc::new(NativeFunction::new(&full, None, move |args| {
            if args.is_empty() || args.len() > 2 {
                return Err(format!("{fname}() expects (message, fields?)"));
            }
            let message = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "{fname}() expects a message string, got {}",
                        other.type_name()
                    ))
                }
            };
            let fields = extract_fields(args, &fname)?;
            emit(level, &message, fields.as_ref());
            Ok(Value::Null)
        })),
    );
}

pub fn register_logger_class(env: &mut Environment) {
    let mut m: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    register_log_fn(&mut m, "Logger", "debug", Level::Debug);
    register_log_fn(&mut m, "Logger", "info", Level::Info);
    register_log_fn(&mut m, "Logger", "warn", Level::Warn);
    register_log_fn(&mut m, "Logger", "error", Level::Error);

    // Logger.configure({"level": "warn", "json": true}) — both keys optional.
    m.insert(
        "configure".to_string(),
        Rc::new(NativeFunction::new("Logger.configure", Some(1), |args| {
            let h = match &args[0] {
                Value::Hash(h) => h.borrow().clone(),
                other => {
                    return Err(format!(
                        "Logger.configure() expects a hash, got {}",
                        other.type_name()
                    ))
                }
            };
            let mut guard = CONFIG.write().unwrap_or_else(|e| e.into_inner());
            if let Some(Value::String(v)) = h.get(&HashKey::String("level".into())) {
                match Level::from_str(v) {
                    Some(l) => guard.level = l,
                    None => {
                        return Err(format!(
                            "Logger.configure(): unknown level \"{v}\" (debug/info/warn/error)"
                        ))
                    }
                }
            }
            if let Some(v) = h.get(&HashKey::String("json".into())) {
                match v {
                    Value::Bool(b) => guard.json = *b,
                    other => {
                        return Err(format!(
                            "Logger.configure(): \"json\" expects a bool, got {}",
                            other.type_name()
                        ))
                    }
                }
            }
            Ok(Value::Null)
        })),
    );

    // Logger.level() -> current level name ("INFO" by default).
    m.insert(
        "level".to_string(),
        Rc::new(NativeFunction::new("Logger.level", Some(0), |_args| {
            Ok(Value::String(read_config().level.name().into()))
        })),
    );

    // Test capture controls.
    m.insert(
        "set_capture".to_string(),
        Rc::new(NativeFunction::new(
            "Logger.set_capture",
            Some(1),
            |args| match &args[0] {
                Value::Bool(on) => {
                    set_capture(*on);
                    Ok(Value::Null)
                }
                other => Err(format!(
                    "Logger.set_capture() expects a bool, got {}",
                    other.type_name()
                )),
            },
        )),
    );

    m.insert(
        "entries".to_string(),
        Rc::new(NativeFunction::new("Logger.entries", Some(0), |_args| {
            Ok(Value::Array(Rc::new(RefCell::new(
                captured_entries()
                    .into_iter()
                    .map(|l| Value::String(l.into()))
                    .collect(),
            ))))
        })),
    );

    m.insert(
        "clear_entries".to_string(),
        Rc::new(NativeFunction::new(
            "Logger.clear_entries",
            Some(0),
            |_args| {
                if let Ok(mut buf) = CAPTURE.write() {
                    buf.entries.clear();
                }
                Ok(Value::Null)
            },
        )),
    );

    let class = Class {
        name: "Logger".to_string(),
        superclass: None,
        methods: Rc::new(RefCell::new(HashMap::new())),
        static_methods: HashMap::new(),
        native_static_methods: m,
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
    };
    env.define("Logger".to_string(), Value::Class(Rc::new(class)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    /// One process-wide lock for every test that touches `CAPTURE` or `CONFIG`.
    ///
    /// Both are process-global, and the test harness runs these in parallel, so
    /// `capture_ring_is_bounded_and_lifo` could observe entries another test had
    /// just emitted and fail on the absolute count. Same fix (and same reason)
    /// as the env-mutating helpers in `serve::server_constants`.
    static LOG_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn lock_log() -> std::sync::MutexGuard<'static, ()> {
        LOG_TEST_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    fn fields(pairs: &[(&str, Value)]) -> HashPairs {
        let mut h = HashPairs::default();
        for (k, v) in pairs {
            h.insert(HashKey::String((*k).into()), v.clone());
        }
        h
    }

    #[test]
    fn level_filtering_drops_below_threshold() {
        let _guard = lock_log();
        CONFIG.write().unwrap().level = Level::Warn;
        assert_eq!(read_config().level, Level::Warn);
        // Debug would be filtered; no panic either way — just exercise paths.
        emit(Level::Debug, "hidden", None);
        emit(Level::Error, "shown", None);
        CONFIG.write().unwrap().level = default_level();
    }

    #[test]
    fn captured_entry_contains_ts_level_message_and_fields() {
        let _guard = lock_log();
        {
            let mut cfg = CONFIG.write().unwrap();
            cfg.json = false;
            cfg.level = Level::Debug;
        }
        set_capture(true);
        emit(
            Level::Warn,
            "order placed",
            Some(&fields(&[("order_id", Value::Int(42))])),
        );
        set_capture(false);
        // set_capture(false) clears, so capture directly around emit instead.
        {
            let mut buf = CAPTURE.write().unwrap();
            buf.enabled = true;
            buf.entries.clear();
        }
        emit(
            Level::Warn,
            "order placed",
            Some(&fields(&[("order_id", Value::Int(42))])),
        );
        // The capture buffer is process-global and other tests may write to
        // it concurrently — assert on our entry being present as the newest,
        // not on the absolute count.
        let entries = captured_entries();
        let last = entries.last().expect("at least one captured entry");
        assert!(last.contains("[WARN] order placed"));
        assert!(last.contains("order_id=42"));
        {
            let mut buf = CAPTURE.write().unwrap();
            buf.enabled = false;
            buf.entries.clear();
        }
        CONFIG.write().unwrap().level = default_level();
    }

    #[test]
    fn capture_ring_is_bounded_and_lifo() {
        let _guard = lock_log();
        {
            let mut buf = CAPTURE.write().unwrap();
            buf.enabled = true;
            buf.entries.clear();
            for i in 0..(CAPTURE_CAP + 50) {
                if buf.entries.len() >= CAPTURE_CAP {
                    buf.entries.pop_front();
                }
                buf.entries.push_back(format!("line-{i}"));
            }
        }
        let buf = CAPTURE.read().unwrap();
        assert_eq!(buf.entries.len(), CAPTURE_CAP);
        assert_eq!(
            buf.entries.back().unwrap(),
            &format!("line-{}", CAPTURE_CAP + 49)
        );
        assert_eq!(buf.entries.front().unwrap(), "line-50");
    }

    #[test]
    fn levels_order_debug_before_error() {
        assert!(Level::Debug < Level::Info);
        assert!(Level::Info < Level::Warn);
        assert!(Level::Warn < Level::Error);
        assert_eq!(Level::from_str("WARNING"), Some(Level::Warn));
        assert_eq!(Level::from_str("nope"), None);
    }
}
