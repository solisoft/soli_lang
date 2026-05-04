//! Per-request body size cap.
//!
//! Without a cap, an attacker can stream an arbitrarily large request body
//! (or open many concurrent uploads) and exhaust server memory before the
//! handler even runs — `BodyExt::collect` buffers the entire body. We
//! enforce a limit on every non-GET/HEAD body read in `handle_hyper_request`
//! and short-circuit to 413 either via the `Content-Length` header
//! (cheap, no read) or via `http_body_util::Limited` (catches chunked
//! uploads that don't declare a length).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Once;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

/// 8 MiB. Big enough for typical JSON/form posts; small enough that an
/// abusive client cannot trivially blow out worker memory. Apps that
/// legitimately accept large uploads override via `set_max_body_size`.
const DEFAULT_MAX_BODY_SIZE: usize = 8 * 1024 * 1024;

static MAX_BODY_SIZE: AtomicUsize = AtomicUsize::new(DEFAULT_MAX_BODY_SIZE);
static ENV_INIT: Once = Once::new();

pub fn get_max_body_size() -> usize {
    MAX_BODY_SIZE.load(Ordering::Relaxed)
}

/// Parse a `SOLI_MAX_BODY_SIZE` value. Returns `Some(bytes)` for a clean
/// non-negative integer, `None` for missing/empty/non-numeric input.
/// Factored out so tests don't race on `std::env::var` or the
/// `Once`-protected init.
fn parse_body_limit_env(raw: Option<&str>) -> Option<usize> {
    raw?.trim().parse::<usize>().ok()
}

/// Read `SOLI_MAX_BODY_SIZE` once and seed the cap from it. Value is the
/// limit in bytes (e.g. `33554432` for 32 MiB). Non-numeric or negative
/// values are ignored and the default stands. `set_max_body_size(...)`
/// still overrides at runtime.
fn init_from_env() {
    ENV_INIT.call_once(|| {
        let raw = std::env::var("SOLI_MAX_BODY_SIZE").ok();
        if let Some(n) = parse_body_limit_env(raw.as_deref()) {
            MAX_BODY_SIZE.store(n, Ordering::Relaxed);
        }
    });
}

pub fn register_body_limit_builtins(env: &mut Environment) {
    init_from_env();

    env.define(
        "set_max_body_size".to_string(),
        Value::NativeFunction(NativeFunction::new("set_max_body_size", Some(1), |args| {
            let bytes = match &args[0] {
                Value::Int(n) if *n >= 0 => *n as usize,
                Value::Int(_) => {
                    return Err("set_max_body_size: bytes must be non-negative".to_string())
                }
                other => {
                    return Err(format!(
                        "set_max_body_size expects Int, got {}",
                        other.type_name()
                    ))
                }
            };
            MAX_BODY_SIZE.store(bytes, Ordering::Relaxed);
            Ok(Value::Int(bytes as i64))
        })),
    );

    env.define(
        "max_body_size".to_string(),
        Value::NativeFunction(NativeFunction::new("max_body_size", Some(0), |_args| {
            Ok(Value::Int(get_max_body_size() as i64))
        })),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bundled because `MAX_BODY_SIZE` is process-global; running these
    /// cases as separate tests would race them under cargo's parallel
    /// runner.
    #[test]
    fn body_limit_round_trip() {
        // Reset to default so we don't depend on test ordering.
        MAX_BODY_SIZE.store(DEFAULT_MAX_BODY_SIZE, Ordering::Relaxed);
        assert_eq!(get_max_body_size(), DEFAULT_MAX_BODY_SIZE);

        MAX_BODY_SIZE.store(1024, Ordering::Relaxed);
        assert_eq!(get_max_body_size(), 1024);

        MAX_BODY_SIZE.store(DEFAULT_MAX_BODY_SIZE, Ordering::Relaxed);
    }

    #[test]
    fn env_parser_accepts_non_negative_int_only() {
        assert_eq!(parse_body_limit_env(Some("0")), Some(0));
        assert_eq!(parse_body_limit_env(Some("65536")), Some(65536));
        assert_eq!(parse_body_limit_env(Some(" 33554432 ")), Some(33554432));
        // Junk and missing values fall through.
        assert_eq!(parse_body_limit_env(Some("")), None);
        assert_eq!(parse_body_limit_env(Some("abc")), None);
        assert_eq!(parse_body_limit_env(Some("-1")), None);
        assert_eq!(parse_body_limit_env(Some("1MB")), None);
        assert_eq!(parse_body_limit_env(None), None);
    }
}
