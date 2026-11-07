//! `TCP.check(host, port, timeout: seconds)` — is a TCP port reachable?
//!
//! ```soli
//! probe = TCP.check("db.example.com", 5432, timeout: 2)
//! probe["open"]        # true
//! probe["latency_ms"]  # 3.4 — time to complete the connect
//! probe["error"]       # nil, or why the port is not reachable
//! ```
//!
//! Connect only: no byte is written or read, and the socket is closed as soon
//! as the handshake completes. That is the whole API on purpose — a general
//! socket would let an app speak arbitrary protocols to whatever it can reach.
//!
//! The destination goes through the same SSRF policy as `HTTP`: loopback,
//! private and link-local ranges are refused unless named in
//! `SOLI_HTTP_ALLOW_HOSTS` (or the process runs under `soli test` /
//! `SOLI_DEV_ALLOW_SSRF=1`). The check runs on the host as written *and* on
//! every address it resolves to, right before connecting, so a DNS answer
//! cannot swap a public name for a private address in between.
//!
//! A refused destination or a malformed argument raises; an unreachable port
//! does not — it answers `{"open": false, "error": "…"}`, which is what a
//! health check wants to branch on.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{hash_from_pairs, Class, HashKey, NativeFunction, Value};

/// Default and ceiling for the connect timeout, in seconds.
const DEFAULT_TIMEOUT_SECS: f64 = 5.0;
const MAX_TIMEOUT_SECS: f64 = 60.0;

/// What one probe found: `Ok(latency)` when the port accepted the connection,
/// `Err(reason)` when it did not.
type ProbeOutcome = Result<f64, String>;

fn parse_timeout(value: Option<&Value>) -> Result<Duration, String> {
    let secs = match value {
        None | Some(Value::Null) => DEFAULT_TIMEOUT_SECS,
        Some(Value::Int(n)) => *n as f64,
        Some(Value::Float(f)) => *f,
        // `TCP.check(host, port, timeout: 2)` — named arguments reach a native
        // as one trailing hash.
        Some(Value::Hash(h)) => {
            let h = h.borrow();
            let mut timeout = None;
            for key in h.keys() {
                match key {
                    HashKey::String(name) | HashKey::Symbol(name) if &**name == "timeout" => {
                        timeout = h.get(key).cloned();
                    }
                    HashKey::String(name) | HashKey::Symbol(name) => {
                        return Err(format!(
                            "TCP.check(): unknown option '{name}' (expected timeout)"
                        ))
                    }
                    other => {
                        return Err(format!(
                            "TCP.check(): unknown option {other:?} (expected timeout)"
                        ))
                    }
                }
            }
            return parse_timeout(timeout.as_ref());
        }
        Some(other) => {
            return Err(format!(
                "TCP.check(): timeout must be a number of seconds, got {}",
                other.type_name()
            ))
        }
    };
    if !(secs > 0.0 && secs <= MAX_TIMEOUT_SECS) {
        return Err(format!(
            "TCP.check(): timeout must be more than 0 and at most {MAX_TIMEOUT_SECS} seconds"
        ));
    }
    Ok(Duration::from_secs_f64(secs))
}

/// The `http://host:port/` URL the HTTP SSRF guard judges this destination by.
#[cfg(not(target_arch = "wasm32"))]
fn guard_url(host: &str, port: u16) -> String {
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    if bare.contains(':') {
        format!("http://[{bare}]:{port}/")
    } else {
        format!("http://{bare}:{port}/")
    }
}

/// Connect to `host:port`, trying each resolved address in turn. `check_ssrf`
/// is off only in unit tests, which talk to a listener on loopback.
#[cfg(not(target_arch = "wasm32"))]
fn probe(
    host: &str,
    port: u16,
    timeout: Duration,
    check_ssrf: bool,
) -> Result<ProbeOutcome, String> {
    use crate::interpreter::builtins::http_class::{is_blocked_ip, validate_url_for_ssrf};
    use std::net::{TcpStream, ToSocketAddrs};
    use std::time::Instant;

    let host = host.trim();
    if host.is_empty() {
        return Err("TCP.check(): host cannot be empty".to_string());
    }
    if check_ssrf {
        validate_url_for_ssrf(&guard_url(host, port)).map_err(|e| format!("TCP.check(): {e}"))?;
    }
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    let addrs: Vec<_> = match (bare, port).to_socket_addrs() {
        Ok(addrs) => addrs.collect(),
        Err(e) => return Ok(Err(format!("DNS resolution failed for {bare}: {e}"))),
    };
    if addrs.is_empty() {
        return Ok(Err(format!("no address found for {bare}")));
    }
    let started = Instant::now();
    let mut last_error = String::new();
    for addr in &addrs {
        // Re-judge the address actually dialled. A blocked IP passes only
        // when the guard lets it through on its own terms (test runner, dev
        // override, an allowlisted literal) or the host was allowlisted by
        // name — which the check above already established for `host`.
        if check_ssrf
            && is_blocked_ip(addr.ip())
            && validate_url_for_ssrf(&guard_url(&addr.ip().to_string(), port)).is_err()
            && !crate::interpreter::builtins::http_class::host_is_allowlisted_str(bare, port)
        {
            return Err(format!(
                "TCP.check(): {bare} resolved to {}, a private/localhost address, which is not allowed",
                addr.ip()
            ));
        }
        let remaining = timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            last_error = format!("timed out after {:.1}s", timeout.as_secs_f64());
            break;
        }
        let attempt = Instant::now();
        match TcpStream::connect_timeout(addr, remaining) {
            Ok(stream) => {
                let latency_ms = attempt.elapsed().as_secs_f64() * 1000.0;
                drop(stream);
                return Ok(Ok(latency_ms));
            }
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
                last_error = format!("timed out after {:.1}s", timeout.as_secs_f64());
            }
            Err(e) => last_error = e.to_string(),
        }
    }
    Ok(Err(format!(
        "connect to {bare}:{port} failed: {last_error}"
    )))
}

#[cfg(target_arch = "wasm32")]
fn probe(
    _host: &str,
    _port: u16,
    _timeout: Duration,
    _check_ssrf: bool,
) -> Result<ProbeOutcome, String> {
    Err(crate::platform::unsupported_on_edge("TCP.check()"))
}

fn outcome_hash(outcome: ProbeOutcome) -> Value {
    let (open, latency, error) = match outcome {
        Ok(ms) => (
            true,
            Value::Float((ms * 1000.0).round() / 1000.0),
            Value::Null,
        ),
        Err(reason) => (false, Value::Null, Value::String(reason.into())),
    };
    hash_from_pairs(vec![
        ("open".to_string(), Value::Bool(open)),
        ("latency_ms".to_string(), latency),
        ("error".to_string(), error),
    ])
}

fn tcp_check(args: &[Value]) -> Result<Value, String> {
    if args.len() < 2 || args.len() > 3 {
        return Err(format!(
            "TCP.check() expects (host, port, timeout?), got {} argument(s)",
            args.len()
        ));
    }
    let host = match &args[0] {
        Value::String(s) => s.to_string(),
        other => {
            return Err(format!(
                "TCP.check() expects a string host, got {}",
                other.type_name()
            ))
        }
    };
    let port = match &args[1] {
        Value::Int(n) if (1..=65535).contains(n) => *n as u16,
        Value::Int(n) => return Err(format!("TCP.check(): port must be 1-65535, got {n}")),
        other => {
            return Err(format!(
                "TCP.check(): port must be an Int 1-65535, got {}",
                other.type_name()
            ))
        }
    };
    let timeout = parse_timeout(args.get(2))?;
    Ok(outcome_hash(probe(&host, port, timeout, true)?))
}

pub fn register_tcp_builtins(env: &mut Environment) {
    let mut methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();
    methods.insert(
        "check".to_string(),
        Rc::new(NativeFunction::new("TCP.check", None, tcp_check)),
    );
    let class = Class {
        name: "TCP".to_string(),
        superclass: None,
        methods: Rc::new(RefCell::new(HashMap::new())),
        static_methods: HashMap::new(),
        native_static_methods: methods,
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
    };
    env.define("TCP".to_string(), Value::Class(Rc::new(class)));
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    fn field(value: &Value, key: &str) -> Value {
        match value {
            Value::Hash(h) => h
                .borrow()
                .get(&HashKey::String(key.into()))
                .cloned()
                .unwrap_or(Value::Null),
            other => panic!("expected a hash, got {}", other.type_name()),
        }
    }

    #[test]
    fn an_open_port_answers_open_with_a_latency() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let outcome = probe("127.0.0.1", port, Duration::from_secs(2), false).unwrap();
        let result = outcome_hash(outcome);
        assert!(matches!(field(&result, "open"), Value::Bool(true)));
        assert!(matches!(field(&result, "latency_ms"), Value::Float(ms) if ms >= 0.0));
        assert!(matches!(field(&result, "error"), Value::Null));
    }

    #[test]
    fn a_closed_port_answers_closed_with_the_reason() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let result = outcome_hash(probe("127.0.0.1", port, Duration::from_secs(2), false).unwrap());
        assert!(matches!(field(&result, "open"), Value::Bool(false)));
        assert!(matches!(field(&result, "latency_ms"), Value::Null));
        let Value::String(error) = field(&result, "error") else {
            panic!("error should be a string");
        };
        assert!(error.contains("failed"), "{error}");
    }

    /// Loopback is refused like `HTTP` outside the test runner. The flag is
    /// process-global and other tests turn it on, so the assertion only runs
    /// while it is off.
    #[test]
    fn loopback_goes_through_the_ssrf_guard() {
        if crate::interpreter::builtins::http_class::ssrf_test_mode() {
            return;
        }
        let err = probe("127.0.0.1", 6379, Duration::from_secs(1), true).unwrap_err();
        assert!(err.contains("not allowed"), "{err}");
        let err = probe("localhost", 6379, Duration::from_secs(1), true).unwrap_err();
        assert!(err.contains("not allowed"), "{err}");
    }

    #[test]
    fn arguments_are_validated() {
        let host = Value::String("example.com".into());
        assert!(tcp_check(std::slice::from_ref(&host)).is_err());
        assert!(tcp_check(&[host.clone(), Value::Int(0)]).is_err());
        assert!(tcp_check(&[host.clone(), Value::Int(70_000)]).is_err());
        assert!(tcp_check(&[host.clone(), Value::Int(80), Value::Int(0)]).is_err());
        assert!(tcp_check(&[host, Value::Int(80), Value::Float(61.0)]).is_err());
        assert!(parse_timeout(Some(&Value::Float(0.5))).is_ok());
        let named = hash_from_pairs(vec![("timeout".to_string(), Value::Int(2))]);
        assert_eq!(parse_timeout(Some(&named)).unwrap(), Duration::from_secs(2));
        let bad = hash_from_pairs(vec![("retries".to_string(), Value::Int(2))]);
        assert!(parse_timeout(Some(&bad)).is_err());
    }
}
