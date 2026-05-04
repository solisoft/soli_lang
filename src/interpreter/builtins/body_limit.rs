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

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

/// 8 MiB. Big enough for typical JSON/form posts; small enough that an
/// abusive client cannot trivially blow out worker memory. Apps that
/// legitimately accept large uploads override via `set_max_body_size`.
const DEFAULT_MAX_BODY_SIZE: usize = 8 * 1024 * 1024;

static MAX_BODY_SIZE: AtomicUsize = AtomicUsize::new(DEFAULT_MAX_BODY_SIZE);

pub fn get_max_body_size() -> usize {
    MAX_BODY_SIZE.load(Ordering::Relaxed)
}

pub fn register_body_limit_builtins(env: &mut Environment) {
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
}
