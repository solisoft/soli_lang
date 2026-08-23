//! Resilience built-in classes for Soli.
//!
//! `CircuitBreaker` — a per-name failure tracker that trips open after N
//! consecutive failures and refuses calls for a cool-down before allowing
//! a probe (half-open). State lives in a bounded process-global store, the
//! same shape as the rate-limiter's, so every worker thread and engine
//! sees the same circuit. Single-process by design; cross-process
//! coordination belongs to the job system's atomic claiming.
//!
//! Callback-free on purpose: plain natives cannot invoke Soli functions,
//! so callers record outcomes explicitly:
//!
//!   if CircuitBreaker.allow("stripe") {
//!       match HTTP.post_json(url, body) rescue null {
//!           null => CircuitBreaker.record_failure("stripe"),
//!           r => { CircuitBreaker.record_success("stripe"); ... }
//!       }
//!   }

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, HashKey, HashPairs, NativeFunction, Value};

const DEFAULT_THRESHOLD: u32 = 5;
/// Seconds an open circuit waits before allowing one probe through.
const DEFAULT_RESET_AFTER_SECS: u64 = 30;
/// Cap on tracked circuits so hostile/rotating names cannot grow the
/// process-global store without bound (same defense as the rate limiter).
const MAX_CIRCUITS: usize = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Closed,
    Open { opened_at: Instant },
    HalfOpen,
}

struct Circuit {
    /// Set by `configure()`. A configured circuit is never evicted: dropping it
    /// silently restored the default threshold, so an app that tunes its
    /// breakers at boot and uses per-tenant names lost that tuning at runtime
    /// with no signal.
    configured: bool,
    threshold: u32,
    reset_after: Duration,
    consecutive_failures: u32,
    state: State,
    /// When the current half-open probe was admitted, if one is in flight.
    ///
    /// Half-open used to admit *every* caller, because nothing recorded that a
    /// probe was already running — the opposite of the documented "allows one
    /// probe", and a thundering herd onto the dependency that just failed. The
    /// timestamp (rather than a bool) means a caller that never reports back
    /// cannot wedge the circuit shut: once `reset_after` has passed, the next
    /// caller gets to probe.
    probe_started: Option<Instant>,
}

impl Circuit {
    fn new() -> Self {
        Self {
            configured: false,
            threshold: DEFAULT_THRESHOLD,
            reset_after: Duration::from_secs(DEFAULT_RESET_AFTER_SECS),
            consecutive_failures: 0,
            state: State::Closed,
            probe_started: None,
        }
    }

    /// Current state, transitioning Open → HalfOpen once the cool-down
    /// elapsed (lazy transition; no background sweeper needed).
    fn current(&mut self) -> State {
        if let State::Open { opened_at } = self.state {
            if opened_at.elapsed() >= self.reset_after {
                self.state = State::HalfOpen;
            }
        }
        self.state
    }

    fn allow(&mut self) -> bool {
        match self.current() {
            State::Closed => true,
            State::Open { .. } => false,
            State::HalfOpen => match self.probe_started {
                // A probe is already out and still within its window.
                Some(started) if started.elapsed() < self.reset_after => false,
                _ => {
                    self.probe_started = Some(Instant::now());
                    true
                }
            },
        }
    }

    fn record_success(&mut self) {
        match self.current() {
            // The probe came back healthy: close up and forget the failures.
            State::HalfOpen => {
                self.consecutive_failures = 0;
                self.state = State::Closed;
                self.probe_started = None;
            }
            State::Closed => self.consecutive_failures = 0,
            // A success reported while the circuit is OPEN did not come from
            // the admitted probe — it is a call that started before the circuit
            // tripped and finished late. Closing on it undid the trip and sent
            // the whole herd back at a dead dependency, which is exactly what
            // the half-open probe exists to prevent. Leave the circuit open and
            // let the cool-down decide.
            State::Open { .. } => {}
        }
    }

    fn record_failure(&mut self) {
        self.consecutive_failures += 1;
        match self.current() {
            // Probe failed in half-open: re-open immediately, without waiting
            // for the threshold again (which is what the code used to do,
            // despite this comment).
            State::HalfOpen => {
                self.state = State::Open {
                    opened_at: Instant::now(),
                };
                self.probe_started = None;
            }
            State::Closed => {
                if self.consecutive_failures >= self.threshold {
                    self.state = State::Open {
                        opened_at: Instant::now(),
                    };
                    self.probe_started = None;
                }
            }
            State::Open { .. } => {}
        }
    }
}

lazy_static::lazy_static! {
    static ref CIRCUITS: RwLock<HashMap<String, Circuit>> = RwLock::new(HashMap::new());
}

fn with_circuit<R>(name: &str, f: impl FnOnce(&mut Circuit) -> R) -> R {
    let mut guard = CIRCUITS.write().unwrap_or_else(|e| e.into_inner());
    if !guard.contains_key(name) && guard.len() >= MAX_CIRCUITS {
        // Bound the store so hostile/rotating names cannot grow it forever.
        //
        // Only untouched, healthy circuits are reclaimable: one that is
        // configured, tripped, or counting failures carries state a caller is
        // relying on. The old predicate reclaimed exactly the healthy ones
        // *including* configured ones, which is how boot-time tuning vanished.
        guard.retain(|_, c| {
            c.configured || c.consecutive_failures > 0 || !matches!(c.state, State::Closed)
        });
        if guard.len() >= MAX_CIRCUITS {
            // Nothing reclaimable. Refuse to grow — every tracked circuit here
            // is carrying real state, and silently exceeding the cap is how an
            // unbounded store starts. `allow()` on an untracked name therefore
            // fails OPEN (the call proceeds), which is the safe direction: a
            // full store must not start refusing healthy traffic.
            return f(&mut Circuit::new());
        }
    }
    let entry = guard.entry(name.to_string()).or_insert_with(Circuit::new);
    f(entry)
}

pub fn register_circuit_breaker_class(env: &mut Environment) {
    let mut m: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    // CircuitBreaker.allow(name) -> bool — true when a call may proceed.
    m.insert(
        "allow".to_string(),
        Rc::new(NativeFunction::new(
            "CircuitBreaker.allow",
            Some(1),
            |args| {
                let name = string_arg(&args[0], "CircuitBreaker.allow() name")?;
                Ok(Value::Bool(with_circuit(&name, |c| c.allow())))
            },
        )),
    );

    // CircuitBreaker.record_success(name) / .record_failure(name)
    for (fname, level) in [("record_success", false), ("record_failure", true)] {
        let full = format!("CircuitBreaker.{fname}");
        let err_ctx = full.clone();
        m.insert(
            fname.to_string(),
            Rc::new(NativeFunction::new(&full, Some(1), move |args| {
                let name = string_arg(&args[0], &format!("{err_ctx}() name"))?;
                with_circuit(&name, |c| {
                    if level {
                        c.record_failure();
                    } else {
                        c.record_success();
                    }
                });
                Ok(Value::Null)
            })),
        );
    }

    // CircuitBreaker.state(name) -> "closed" | "open" | "half_open"
    m.insert(
        "state".to_string(),
        Rc::new(NativeFunction::new(
            "CircuitBreaker.state",
            Some(1),
            |args| {
                let name = string_arg(&args[0], "CircuitBreaker.state() name")?;
                let s = with_circuit(&name, |c| match c.current() {
                    State::Closed => "closed",
                    State::Open { .. } => "open",
                    State::HalfOpen => "half_open",
                });
                Ok(Value::String(s.into()))
            },
        )),
    );

    // CircuitBreaker.configure(name, {"threshold": 5, "reset_after": 30})
    m.insert(
        "configure".to_string(),
        Rc::new(NativeFunction::new(
            "CircuitBreaker.configure",
            Some(2),
            |args| {
                let name = string_arg(&args[0], "CircuitBreaker.configure() name")?;
                let h = match &args[1] {
                    Value::Hash(h) => h.borrow().clone(),
                    other => {
                        return Err(format!(
                            "CircuitBreaker.configure() expects an options hash, got {}",
                            other.type_name()
                        ))
                    }
                };
                let mut threshold: Option<u32> = None;
                let mut reset_after: Option<Duration> = None;
                for (k, v) in h.iter() {
                    let key = match k {
                        HashKey::String(s) | HashKey::Symbol(s) => s.as_str().to_string(),
                        _ => continue,
                    };
                    match key.as_str() {
                        "threshold" => match v {
                            Value::Int(n) if *n > 0 => threshold = Some(*n as u32),
                            _ => return Err(
                                "CircuitBreaker.configure(): \"threshold\" expects a positive Int"
                                    .to_string(),
                            ),
                        },
                        // Seconds, Int or Float. The Float branch used to scale
                        // to milliseconds and then hand the number to
                        // `Duration::from_secs`, so `{"reset_after": 0.5}` held
                        // the circuit open for 500 *seconds*.
                        "reset_after" => match v {
                            Value::Int(n) if *n > 0 => {
                                reset_after = Some(Duration::from_secs(*n as u64))
                            }
                            Value::Float(f) if *f > 0.0 && f.is_finite() => {
                                reset_after = Some(Duration::from_secs_f64(*f))
                            }
                            _ => return Err(
                                "CircuitBreaker.configure(): \"reset_after\" expects seconds > 0"
                                    .to_string(),
                            ),
                        },
                        _ => {}
                    }
                }
                with_circuit(&name, |c| {
                    c.configured = true;
                    if let Some(t) = threshold {
                        c.threshold = t;
                    }
                    if let Some(r) = reset_after {
                        c.reset_after = r;
                    }
                });
                Ok(Value::Null)
            },
        )),
    );

    // CircuitBreaker.reset(name) — forget all state (ops/testing).
    m.insert(
        "reset".to_string(),
        Rc::new(NativeFunction::new(
            "CircuitBreaker.reset",
            Some(1),
            |args| {
                let name = string_arg(&args[0], "CircuitBreaker.reset() name")?;
                with_circuit(&name, |c| *c = Circuit::new());
                Ok(Value::Null)
            },
        )),
    );

    let class = Class {
        name: "CircuitBreaker".to_string(),
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
    env.define("CircuitBreaker".to_string(), Value::Class(Rc::new(class)));
}

fn string_arg(v: &Value, what: &str) -> Result<String, String> {
    match v {
        Value::String(s) => Ok(s.to_string()),
        other => Err(format!(
            "{what} expects a string, got {}",
            other.type_name()
        )),
    }
}

// ---------- tests ----------

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh(name: &str) {
        with_circuit(name, |c| *c = Circuit::new());
    }

    #[test]
    fn stays_closed_under_threshold_then_trips_open() {
        fresh("t1");
        with_circuit("t1", |c| {
            assert!(c.allow());
            for _ in 0..c.threshold - 1 {
                c.record_failure();
                assert_eq!(c.current(), State::Closed);
            }
            c.record_failure();
            assert!(matches!(c.current(), State::Open { .. }));
            assert!(!c.allow());
        });
    }

    #[test]
    fn success_resets_the_count() {
        fresh("t2");
        with_circuit("t2", |c| {
            for _ in 0..3 {
                c.record_failure();
            }
            c.record_success();
            assert_eq!(c.consecutive_failures, 0);
            // A single failure after success must NOT trip.
            c.record_failure();
            assert_eq!(c.current(), State::Closed);
        });
    }

    #[test]
    fn elapsed_cool_down_goes_half_open_and_probe_refail_reopens() {
        fresh("t3");
        with_circuit("t3", |c| {
            c.reset_after = Duration::from_secs(30);
            for _ in 0..c.threshold {
                c.record_failure();
            }
            assert!(matches!(c.current(), State::Open { .. }));
            // Simulate the cool-down having elapsed.
            c.state = State::Open {
                opened_at: Instant::now() - Duration::from_secs(60),
            };
            assert_eq!(c.current(), State::HalfOpen);
            // A failed probe re-opens immediately.
            c.record_failure();
            assert!(matches!(c.current(), State::Open { .. }));
        });
    }

    #[test]
    fn half_open_success_closes() {
        fresh("t4");
        with_circuit("t4", |c| {
            c.reset_after = Duration::ZERO;
            for _ in 0..c.threshold {
                c.record_failure();
            }
            let _ = c.current();
            c.record_success();
            assert_eq!(c.current(), State::Closed);
            assert_eq!(c.consecutive_failures, 0);
        });
    }
}

// ---------- Semaphore ----------

/// A named, process-global counting semaphore.
///
/// Use case: "at most N of these running at once" inside one process —
/// e.g. a cron handler that must not overlap itself. Tokens are explicit:
/// `try_acquire` returns a token id (or null when full) and `release`
/// gives the slot back. Single-process by design; cross-process mutual
/// exclusion belongs to SolidB/job claiming.
///
///   let token = Semaphore.try_acquire("nightly", 1);
///   if token.present? {
///       ... do work ...
///       Semaphore.release("nightly", token);
///   }
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_SEMAPHORES: usize = 1_000;

static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

struct SemaphoreSlot {
    limit: usize,
    held: Vec<u64>,
}

lazy_static::lazy_static! {
    static ref SEMAPHORES: RwLock<HashMap<String, SemaphoreSlot>> = RwLock::new(HashMap::new());
}

fn semaphore_arg_name(v: &Value, ctx: &str) -> Result<String, String> {
    string_arg(v, ctx)
}

pub fn register_semaphore_class(env: &mut Environment) {
    let mut m: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    // Semaphore.try_acquire(name, limit?) -> Int token | null
    m.insert(
        "try_acquire".to_string(),
        Rc::new(NativeFunction::new(
            "Semaphore.try_acquire",
            Some(2),
            |args| {
                let name = semaphore_arg_name(&args[0], "Semaphore.try_acquire() name")?;
                let limit = match &args[1] {
                    Value::Int(n) if *n > 0 => *n as usize,
                    other => {
                        return Err(format!(
                            "Semaphore.try_acquire() expects a positive Int limit, got {}",
                            other.type_name()
                        ))
                    }
                };
                let mut guard = SEMAPHORES.write().unwrap_or_else(|e| e.into_inner());
                // Drop any slot nobody holds before declaring the store full,
                // so a name that was acquired and released cannot count against
                // the cap (belt-and-braces with the reclamation in `release`,
                // which a caller may never reach).
                if !guard.contains_key(&name) && guard.len() >= MAX_SEMAPHORES {
                    guard.retain(|_, slot| !slot.held.is_empty());
                }
                if !guard.contains_key(&name) && guard.len() >= MAX_SEMAPHORES {
                    return Err(format!(
                        "Semaphore.try_acquire(): too many named semaphores (max {MAX_SEMAPHORES})"
                    ));
                }
                let slot = guard.entry(name).or_insert(SemaphoreSlot {
                    limit,
                    held: Vec::new(),
                });
                // The stored slot keeps its original limit; a later call with a
                // different limit does not resize a live semaphore.
                if slot.held.len() < slot.limit {
                    let token = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed);
                    slot.held.push(token);
                    Ok(Value::Int(token as i64))
                } else {
                    Ok(Value::Null)
                }
            },
        )),
    );

    // Semaphore.release(name, token) -> bool (true when the token was held)
    m.insert(
        "release".to_string(),
        Rc::new(NativeFunction::new("Semaphore.release", Some(2), |args| {
            let name = semaphore_arg_name(&args[0], "Semaphore.release() name")?;
            let token = match &args[1] {
                Value::Int(n) => *n as u64,
                other => {
                    return Err(format!(
                        "Semaphore.release() expects an Int token, got {}",
                        other.type_name()
                    ))
                }
            };
            let mut guard = SEMAPHORES.write().unwrap_or_else(|e| e.into_inner());
            match guard.get_mut(&name) {
                Some(slot) => {
                    let before = slot.held.len();
                    slot.held.retain(|t| *t != token);
                    let released = slot.held.len() != before;
                    // The slot stays, so the name keeps the limit its first
                    // caller fixed — dropping it here reclaimed memory but also
                    // reset the limit, letting `try_acquire("q", 10)` silently
                    // reconfigure a semaphore first created with limit 1.
                    // Reclamation instead happens in `try_acquire`, which prunes
                    // unheld slots when the store is at its cap: that is what
                    // stops a per-key pattern ("import-#{tenant}") from filling
                    // the store, without weakening the sticky limit.
                    Ok(Value::Bool(released))
                }
                None => Ok(Value::Bool(false)),
            }
        })),
    );

    // Semaphore.count(name) -> {"limit": n, "held": k} or null
    m.insert(
        "count".to_string(),
        Rc::new(NativeFunction::new("Semaphore.count", Some(1), |args| {
            let name = semaphore_arg_name(&args[0], "Semaphore.count() name")?;
            let guard = SEMAPHORES.read().unwrap_or_else(|e| e.into_inner());
            match guard.get(&name) {
                Some(slot) => {
                    let mut h = HashPairs::default();
                    h.insert(
                        HashKey::String("limit".into()),
                        Value::Int(slot.limit as i64),
                    );
                    h.insert(
                        HashKey::String("held".into()),
                        Value::Int(slot.held.len() as i64),
                    );
                    Ok(Value::Hash(Rc::new(RefCell::new(h))))
                }
                None => Ok(Value::Null),
            }
        })),
    );

    // Semaphore.reset(name) — drop the name and every token held on it.
    //
    // `release` was the only way to free a slot, so a token leaked by a job
    // that raised before releasing wedged that name for the life of the
    // process: every later `try_acquire` returned null and the nightly job
    // simply stopped running until a restart. `CircuitBreaker` already had a
    // `reset`; this is the same escape hatch for operators and tests.
    m.insert(
        "reset".to_string(),
        Rc::new(NativeFunction::new("Semaphore.reset", Some(1), |args| {
            let name = semaphore_arg_name(&args[0], "Semaphore.reset() name")?;
            let mut guard = SEMAPHORES.write().unwrap_or_else(|e| e.into_inner());
            Ok(Value::Bool(guard.remove(&name).is_some()))
        })),
    );

    let class = Class {
        name: "Semaphore".to_string(),
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
    env.define("Semaphore".to_string(), Value::Class(Rc::new(class)));
}

#[cfg(test)]
mod semaphore_tests {
    use super::*;

    #[test]
    fn acquires_up_to_limit_then_refuses_then_releases() {
        // Fresh name per run; tests may re-run on the same process in dev loops.
        const NAME: &str = "spec-acquire";
        SEMAPHORES.write().unwrap().remove(NAME);

        let t1 = acquire_token(NAME);
        assert!(t1.is_some());
        let t2 = acquire_token(NAME);
        assert!(t2.is_some());
        let t3 = acquire_token(NAME);
        assert!(t3.is_none(), "limit is 2");

        assert!(release_token(NAME, t1.unwrap()));
        let t4 = acquire_token(NAME);
        assert!(t4.is_some(), "released slot is reusable");
    }

    #[test]
    fn releasing_an_unknown_token_is_false() {
        const NAME: &str = "spec-release-unknown";
        SEMAPHORES
            .write()
            .unwrap()
            .entry(NAME.to_string())
            .or_insert(SemaphoreSlot {
                limit: 1,
                held: Vec::new(),
            });
        assert!(!release_token(NAME, 987_654));
    }

    fn acquire_token(name: &str) -> Option<u64> {
        let mut guard = SEMAPHORES.write().unwrap();
        let slot = guard.entry(name.to_string()).or_insert(SemaphoreSlot {
            limit: 2,
            held: Vec::new(),
        });
        if slot.held.len() < slot.limit {
            let token = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed);
            slot.held.push(token);
            Some(token)
        } else {
            None
        }
    }

    fn release_token(name: &str, token: u64) -> bool {
        let mut guard = SEMAPHORES.write().unwrap();
        match guard.get_mut(name) {
            Some(slot) => {
                let before = slot.held.len();
                slot.held.retain(|t| *t != token);
                slot.held.len() != before
            }
            None => false,
        }
    }
}
