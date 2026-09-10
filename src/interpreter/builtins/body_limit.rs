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

// ---------------------------------------------------------------------------
// Aggregate in-flight budget
// ---------------------------------------------------------------------------

/// How many times the per-request cap may be buffered at once, by default.
///
/// `MAX_BODY_SIZE` bounds *one* request. Nothing bounded the sum: bodies are
/// buffered on the async side, gated only by `SOLI_MAX_CONNECTIONS` (20 000 by
/// default), and each parsed request then rides a worker queue holding
/// `workers x 64` entries. At the 8 MiB default that is gigabytes of upload
/// sitting in RAM before a single handler runs. 16x the per-request cap
/// (128 MiB by default) leaves ordinary small-payload apps untouched.
const DEFAULT_INFLIGHT_MULTIPLE: usize = 16;

static INFLIGHT_BYTES: AtomicUsize = AtomicUsize::new(0);

/// Ceiling on request-body bytes reserved at once. `SOLI_MAX_INFLIGHT_BODY_BYTES`
/// overrides; `0` disables the budget entirely.
pub fn max_inflight_body_bytes() -> usize {
    static CAP: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *CAP.get_or_init(|| {
        std::env::var("SOLI_MAX_INFLIGHT_BODY_BYTES")
            .ok()
            .and_then(|s| parse_body_limit_env(Some(s.as_str())))
            .unwrap_or_else(|| get_max_body_size().saturating_mul(DEFAULT_INFLIGHT_MULTIPLE))
    })
}

/// Bytes currently reserved. Exposed for tests and diagnostics.
pub fn inflight_body_bytes() -> usize {
    INFLIGHT_BYTES.load(Ordering::Relaxed)
}

/// An RAII slice of the in-flight budget.
///
/// The reservation must outlive the buffered body, which is owned by the
/// `RequestData` that crosses the worker queue — so the guard rides along with
/// it and `Drop` returns the bytes on *every* exit: normal response, a 503 when
/// the queue is full, a 504 when the worker overruns, or a worker panic that
/// drops the request. A manual decrement would leak the counter on one of those
/// paths and wedge the server into refusing every subsequent upload.
#[derive(Debug)]
pub struct BodyReservation(usize);

impl BodyReservation {
    /// Reserve `bytes`, or return `None` when that would exceed the ceiling.
    ///
    /// A ceiling of `0` means "no budget": always succeeds and records nothing,
    /// so `Drop` has nothing to return.
    pub fn try_acquire(bytes: usize) -> Option<Self> {
        let cap = max_inflight_body_bytes();
        if cap == 0 {
            return Some(BodyReservation(0));
        }
        let mut current = INFLIGHT_BYTES.load(Ordering::Relaxed);
        loop {
            let next = current.saturating_add(bytes);
            if next > cap {
                return None;
            }
            match INFLIGHT_BYTES.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Some(BodyReservation(bytes)),
                Err(observed) => current = observed,
            }
        }
    }
}

impl Drop for BodyReservation {
    fn drop(&mut self) {
        if self.0 > 0 {
            INFLIGHT_BYTES.fetch_sub(self.0, Ordering::AcqRel);
        }
    }
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

    /// `INFLIGHT_BYTES` is process-global and `cargo test` runs these in
    /// parallel, so a test that samples the counter has to hold this first or
    /// it observes another test's reservation.
    static BUDGET_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// The guard exists for its `Drop`; the counter must come back down on
    /// every path, or the server refuses uploads forever after a burst.
    #[test]
    fn a_reservation_is_returned_when_it_drops() {
        let _serial = BUDGET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let before = inflight_body_bytes();
        {
            let _held = BodyReservation::try_acquire(1024).expect("under the ceiling");
            assert_eq!(inflight_body_bytes(), before + 1024);
        }
        assert_eq!(inflight_body_bytes(), before, "drop must return the bytes");
    }

    #[test]
    fn an_over_ceiling_reservation_is_refused_and_records_nothing() {
        let _serial = BUDGET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let before = inflight_body_bytes();
        let cap = max_inflight_body_bytes();
        assert!(cap > 0, "default budget should be enabled");
        assert!(BodyReservation::try_acquire(cap + 1).is_none());
        assert_eq!(
            inflight_body_bytes(),
            before,
            "a refused reservation must not charge the budget"
        );
    }

    #[test]
    fn reservations_accumulate_and_unwind_independently() {
        let _serial = BUDGET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let before = inflight_body_bytes();
        let a = BodyReservation::try_acquire(2048).expect("first");
        let b = BodyReservation::try_acquire(4096).expect("second");
        assert_eq!(inflight_body_bytes(), before + 2048 + 4096);
        drop(a);
        assert_eq!(inflight_body_bytes(), before + 4096);
        drop(b);
        assert_eq!(inflight_body_bytes(), before);
    }

    /// A zero ceiling means "no budget" — it must not refuse everything.
    #[test]
    fn a_zero_sized_reservation_is_free_and_returns_nothing() {
        let _serial = BUDGET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let before = inflight_body_bytes();
        let held = BodyReservation::try_acquire(0).expect("zero always fits");
        assert_eq!(inflight_body_bytes(), before);
        drop(held);
        assert_eq!(inflight_body_bytes(), before);
    }

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
