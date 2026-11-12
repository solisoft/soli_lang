//! Metrics — the app metrics API (`Metrics.increment`, `gauge`, `histogram`,
//! `distribution`, `time`, `value`, `enabled?`, `reset`).
//!
//! The class is Soli (`metrics.sl`) because `Metrics.time(name) do … end`
//! runs a block, which a plain native cannot. It is evaluated once per thread
//! into an environment of its own, like `Retry` (see `retry.rs` for why not
//! into the environment receiving it), and calls the `__metrics_*` natives
//! that `register_builtins` installs — globally, because the VM compiles the
//! class's methods against its own globals. The storage and the sinks are
//! `crate::metrics`.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::OnceLock;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{hash_from_pairs, HashKey, NativeFunction, Value};
use crate::metrics::registry::{Kind, SeriesValue};
use crate::metrics::statsd::StatsdType;

pub(crate) const METRICS_SOURCE: &str = include_str!("metrics.sl");

thread_local! {
    /// The evaluated Metrics class, once per thread.
    static METRICS_CLASS: RefCell<Option<Value>> = const { RefCell::new(None) };
}

/// Define the embedded `Metrics` class in `env`.
pub fn register_metrics_class(env: &Rc<RefCell<Environment>>) -> Result<(), String> {
    let class = metrics_class()?;
    env.borrow_mut().define("Metrics".to_string(), class);
    Ok(())
}

fn metrics_class() -> Result<Value, String> {
    if let Some(class) = METRICS_CLASS.with(|slot| slot.borrow().clone()) {
        return Ok(class);
    }
    let tokens = crate::lexer::Scanner::new(METRICS_SOURCE)
        .scan_tokens()
        .map_err(|e| format!("metrics stdlib lexer error: {}", e))?;
    let program = crate::parser::Parser::new(tokens)
        .parse()
        .map_err(|e| format!("metrics stdlib parser error: {}", e))?;
    let home = Rc::new(RefCell::new(Environment::with_builtins_capacity()));
    crate::interpreter::builtins::register_builtins(&mut home.borrow_mut(), false);
    let mut interpreter = crate::interpreter::Interpreter::with_environment(home.clone());
    for stmt in &program.statements {
        interpreter
            .execute(stmt)
            .map_err(|e| format!("metrics stdlib eval error: {}", e))?;
    }
    let class = home
        .borrow()
        .get("Metrics")
        .ok_or_else(|| "metrics stdlib did not define Metrics".to_string())?;
    METRICS_CLASS.with(|slot| *slot.borrow_mut() = Some(class.clone()));
    Ok(class)
}

/// A sample value: an Int or a Float.
fn number(value: &Value, method: &str) -> Result<f64, String> {
    match value {
        Value::Int(n) => Ok(*n as f64),
        Value::Float(f) => Ok(*f),
        other => Err(format!(
            "Metrics.{method}() expects a number, got {}",
            other.type_name()
        )),
    }
}

fn tag_value(value: &Value, method: &str, key: &str) -> Result<String, String> {
    Ok(match value {
        Value::String(s) => s.to_string(),
        Value::Int(n) => n.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
        other => {
            return Err(format!(
                "Metrics.{method}(): tag \"{key}\" must be a String, number, Bool or nil, got {}",
                other.type_name()
            ))
        }
    })
}

/// The `tags` hash as `(key, value)` strings; nil is no tags.
fn tags(value: &Value, method: &str) -> Result<Vec<(String, String)>, String> {
    match value {
        Value::Null => Ok(Vec::new()),
        Value::Hash(hash) => hash
            .borrow()
            .iter()
            .map(|(k, v)| {
                let key = match k {
                    HashKey::String(s) | HashKey::Symbol(s) => s.to_string(),
                    other => {
                        return Err(format!(
                            "Metrics.{method}(): tag keys must be strings, got {other}"
                        ))
                    }
                };
                let value = tag_value(v, method, &key)?;
                Ok((key, value))
            })
            .collect(),
        other => Err(format!(
            "Metrics.{method}() expects tags as a Hash, got {}",
            other.type_name()
        )),
    }
}

fn name(value: &Value, method: &str) -> Result<String, String> {
    match value {
        Value::String(s) => Ok(s.to_string()),
        other => Err(format!(
            "Metrics.{method}() expects the metric name as a String, got {}",
            other.type_name()
        )),
    }
}

fn method_of(kind: Kind, statsd: StatsdType) -> &'static str {
    match (kind, statsd) {
        (Kind::Counter, _) => "increment",
        (Kind::Gauge, _) => "gauge",
        (Kind::Histogram, StatsdType::Distribution) => "distribution",
        (Kind::Histogram, _) => "histogram",
    }
}

/// A number as Soli shows it: an Int when it is one.
fn soli_number(v: f64) -> Value {
    if v.fract() == 0.0 && v.abs() < 9.0e15 {
        Value::Int(v as i64)
    } else {
        Value::Float(v)
    }
}

/// The `__metrics_*` natives `metrics.sl` calls, registered with every
/// builtin (see `register_builtins`).
pub fn register_metrics_natives(env: &mut Environment) {
    // __metrics_record(kind, statsd_type, name, value, tags)
    let record = NativeFunction::new("__metrics_record", Some(5), |args| {
        let (Value::String(kind), Value::String(statsd)) = (&args[0], &args[1]) else {
            return Err("__metrics_record: bad kind".to_string());
        };
        let kind = Kind::parse(kind.as_str()).ok_or("__metrics_record: bad kind")?;
        let statsd = StatsdType::parse(statsd.as_str()).ok_or("__metrics_record: bad type")?;
        let method = method_of(kind, statsd);
        let metric = name(&args[2], method)?;
        // `Metrics.increment("x", {"plan": "pro"})`: a hash where the value
        // goes is the tags, and the count is 1.
        let (value, tag_arg) = match (&args[3], &args[4]) {
            (Value::Hash(_), Value::Hash(h)) if kind == Kind::Counter && h.borrow().is_empty() => {
                (1.0, &args[3])
            }
            (v, t) => (number(v, method)?, t),
        };
        let tags = tags(tag_arg, method)?;
        crate::metrics::record_app(kind, statsd, &metric, value, &tags)
            .map_err(|e| format!("Metrics.{method}(): {e}"))?;
        Ok(Value::Null)
    });
    let value = NativeFunction::new("__metrics_value", Some(2), |args| {
        let metric = name(&args[0], "value")?;
        let tags = tags(&args[1], "value")?;
        let found = crate::metrics::Metrics::global()
            .app
            .value(&metric, &tags)
            .map_err(|e| format!("Metrics.value(): {e}"))?;
        Ok(match found {
            None => Value::Null,
            Some(SeriesValue::Number(n)) => soli_number(n),
            Some(SeriesValue::Histogram(h)) => {
                let cumulative = h.cumulative();
                let buckets = hash_from_pairs(cumulative.iter().enumerate().map(|(i, c)| {
                    let le = match h.bounds.get(i) {
                        Some(b) => crate::metrics::format_prom_value(*b),
                        None => "+Inf".to_string(),
                    };
                    (le, Value::Int(*c as i64))
                }));
                hash_from_pairs([
                    ("count", Value::Int(h.count() as i64)),
                    ("sum", Value::Float(h.sum)),
                    ("buckets", buckets),
                ])
            }
        })
    });
    let enabled = NativeFunction::new("__metrics_enabled", Some(0), |_| {
        Ok(Value::Bool(crate::metrics::app_metrics_enabled()))
    });
    let reset = NativeFunction::new("__metrics_reset", Some(0), |_| {
        if crate::platform::env::var("APP_ENV").as_deref() != Ok("test") {
            return Err("Metrics.reset is for tests only (APP_ENV=test)".to_string());
        }
        crate::metrics::Metrics::global().app.reset();
        Ok(Value::Null)
    });
    let clock = NativeFunction::new("__metrics_clock", Some(0), |_| {
        static ANCHOR: OnceLock<web_time::Instant> = OnceLock::new();
        let anchor = *ANCHOR.get_or_init(web_time::Instant::now);
        Ok(Value::Float(anchor.elapsed().as_secs_f64()))
    });
    for (name, native) in [
        ("__metrics_record", record),
        ("__metrics_value", value),
        ("__metrics_enabled", enabled),
        ("__metrics_reset", reset),
        ("__metrics_clock", clock),
    ] {
        env.define(name.to_string(), Value::NativeFunction(native));
    }
}
