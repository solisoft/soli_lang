use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, HashKey, HashPairs, Instance, NativeFunction, Value};
use lazy_static::lazy_static;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::RwLock;
use std::time::{Duration, Instant};

lazy_static! {
    static ref RATE_LIMIT_STORE: RwLock<RateLimitStore> = RwLock::new(RateLimitStore::new());
}

/// Cap on distinct rate-limit keys. Without this, an attacker minting a
/// fresh key per request (rotating IP spoof, random token, …) grows the
/// process-global `HashMap` without bound. Once full, inserting a new key
/// evicts an existing bucket (after reclaiming expired ones) so the store
/// stays bounded while every live key keeps its own independent counter.
const MAX_BUCKETS: usize = 10_000;

/// Run expiry cleanup every N write ops so empty buckets are reclaimed
/// without requiring apps to call `RateLimiter.cleanup()` manually.
const CLEANUP_EVERY_OPS: u64 = 64;

struct RateLimitBucket {
    requests: Vec<Instant>,
    limit: usize,
    window: Duration,
}

impl RateLimitBucket {
    fn new(limit: usize, window: Duration) -> Self {
        Self {
            requests: Vec::new(),
            limit,
            window,
        }
    }

    /// When this bucket last saw a request, for least-recently-used eviction.
    /// An empty bucket is the best possible victim.
    fn last_activity(&self) -> Option<Instant> {
        self.requests.last().copied()
    }

    fn is_allowed(&mut self) -> (bool, usize, Duration) {
        let now = Instant::now();
        let window_start = now - self.window;

        self.requests.retain(|t| *t > window_start);

        let remaining = if self.requests.len() < self.limit {
            self.limit - self.requests.len()
        } else {
            0
        };

        if self.requests.len() < self.limit {
            self.requests.push(now);
            let wait_time = if self.requests.len() == 1 {
                Duration::ZERO
            } else {
                let oldest_in_window = &self.requests[0];
                if self.requests.len() == self.limit {
                    (*oldest_in_window + self.window).saturating_duration_since(now)
                } else {
                    Duration::ZERO
                }
            };
            (true, remaining.saturating_sub(1), wait_time)
        } else {
            let reset_time = self.requests[0] + self.window - now;
            (false, 0, reset_time)
        }
    }

    fn status(&self) -> (usize, usize, Duration) {
        let now = Instant::now();
        let window_start = now - self.window;
        let valid_requests: Vec<&Instant> = self
            .requests
            .iter()
            .filter(|t| **t > window_start)
            .collect();
        let remaining = self.limit.saturating_sub(valid_requests.len());
        let reset_time = if let Some(oldest) = valid_requests.first() {
            **oldest + self.window - now
        } else {
            Duration::ZERO
        };
        (self.limit, remaining, reset_time)
    }
}

struct RateLimitStore {
    buckets: HashMap<String, RateLimitBucket>,
    ops_since_cleanup: u64,
}

impl RateLimitStore {
    fn new() -> Self {
        Self {
            buckets: HashMap::new(),
            ops_since_cleanup: 0,
        }
    }

    fn maybe_auto_cleanup(&mut self) {
        self.ops_since_cleanup = self.ops_since_cleanup.saturating_add(1);
        if self.ops_since_cleanup >= CLEANUP_EVERY_OPS {
            self.ops_since_cleanup = 0;
            self.cleanup();
        }
    }

    fn get_or_create(&mut self, key: &str, limit: usize, window: Duration) -> &mut RateLimitBucket {
        self.maybe_auto_cleanup();

        if !self.buckets.contains_key(key) && self.buckets.len() >= MAX_BUCKETS {
            // Reclaim expired buckets first — cheap and usually enough.
            self.cleanup();
            // Still full ⇒ a genuine unique-key flood. Evict existing buckets
            // to make room instead of collapsing every over-cap key into one
            // shared bucket. Each surviving key keeps its own (limit, window),
            // so counts are never mixed across keys or rate-limit rules. The
            // evicted key just restarts its window (fail-open for that one key
            // — the safe direction), and memory stays bounded at MAX_BUCKETS.
            // Evict the least recently active bucket rather than whichever key
            // the map happens to hand back first. An arbitrary victim meant a
            // key-flood could reset a *live* limiter — the attacker's own, or
            // another client's — so filling the map was itself a way past the
            // throttle.
            while !self.buckets.contains_key(key) && self.buckets.len() >= MAX_BUCKETS {
                let Some(victim) = self
                    .buckets
                    .iter()
                    .min_by_key(|(_, bucket)| bucket.last_activity())
                    .map(|(k, _)| k.clone())
                else {
                    break;
                };
                self.buckets.remove(&victim);
            }
        }

        self.buckets
            .entry(key.to_string())
            .or_insert_with(|| RateLimitBucket::new(limit, window))
    }

    fn status(&self, key: &str, limit: usize, _window: Duration) -> (bool, usize, Duration) {
        if let Some(bucket) = self.buckets.get(key) {
            let (_total, remaining, reset) = bucket.status();
            (remaining > 0, remaining, reset)
        } else {
            (true, limit, Duration::ZERO)
        }
    }

    fn cleanup(&mut self) {
        let now = Instant::now();
        self.buckets.retain(|_, bucket| {
            let window_start = now - bucket.window;
            bucket.requests.retain(|t| *t > window_start);
            !bucket.requests.is_empty() || bucket.limit == 0
        });
    }
}

pub fn register_rate_limit_builtins(env: &mut Environment) {
    let mut class_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    class_methods.insert(
        "allowed".to_string(),
        Rc::new(NativeFunction::new(
            "RateLimiter.allowed",
            Some(0),
            |args| {
                let this = match args.first() {
                    Some(Value::Instance(inst)) => inst,
                    _ => {
                        return Err(
                            "RateLimiter.allowed() must be called on an instance".to_string()
                        )
                    }
                };
                let key = match this.borrow().fields.get("key").cloned() {
                    Some(Value::String(s)) => s.clone(),
                    _ => return Err("RateLimiter instance missing key".to_string()),
                };
                let limit = match this.borrow().fields.get("limit").cloned() {
                    Some(Value::Int(i)) => i as usize,
                    _ => return Err("RateLimiter instance missing limit".to_string()),
                };
                let window = match this.borrow().fields.get("window").cloned() {
                    Some(Value::Int(i)) => i as u64,
                    _ => return Err("RateLimiter instance missing window".to_string()),
                };

                if limit == 0 {
                    return Ok(Value::Bool(true));
                }

                let mut store = RATE_LIMIT_STORE
                    .write()
                    .map_err(|e| format!("Rate limiter error: {}", e))?;
                let bucket = store.get_or_create(&key, limit, Duration::from_secs(window));
                let (allowed, _, _) = bucket.is_allowed();
                Ok(Value::Bool(allowed))
            },
        )),
    );

    class_methods.insert(
        "throttle".to_string(),
        Rc::new(NativeFunction::new(
            "RateLimiter.throttle",
            Some(0),
            |args| {
                let this = match args.first() {
                    Some(Value::Instance(inst)) => inst,
                    _ => {
                        return Err(
                            "RateLimiter.throttle() must be called on an instance".to_string()
                        )
                    }
                };
                let key = match this.borrow().fields.get("key").cloned() {
                    Some(Value::String(s)) => s.clone(),
                    _ => return Err("RateLimiter instance missing key".to_string()),
                };
                let limit = match this.borrow().fields.get("limit").cloned() {
                    Some(Value::Int(i)) => i as usize,
                    _ => return Err("RateLimiter instance missing limit".to_string()),
                };
                let window = match this.borrow().fields.get("window").cloned() {
                    Some(Value::Int(i)) => i as u64,
                    _ => return Err("RateLimiter instance missing window".to_string()),
                };

                let mut store = RATE_LIMIT_STORE
                    .write()
                    .map_err(|e| format!("Rate limiter error: {}", e))?;
                let bucket = store.get_or_create(&key, limit, Duration::from_secs(window));
                let (_, _, wait_time) = bucket.is_allowed();
                Ok(Value::Int(wait_time.as_secs() as i64))
            },
        )),
    );

    class_methods.insert(
        "status".to_string(),
        Rc::new(NativeFunction::new("RateLimiter.status", Some(0), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("RateLimiter.status() must be called on an instance".to_string()),
            };
            let key = match this.borrow().fields.get("key").cloned() {
                Some(Value::String(s)) => s.clone(),
                _ => return Err("RateLimiter instance missing key".to_string()),
            };
            let limit = match this.borrow().fields.get("limit").cloned() {
                Some(Value::Int(i)) => i as usize,
                _ => return Err("RateLimiter instance missing limit".to_string()),
            };
            let window = match this.borrow().fields.get("window").cloned() {
                Some(Value::Int(i)) => i as u64,
                _ => return Err("RateLimiter instance missing window".to_string()),
            };

            let store = RATE_LIMIT_STORE
                .read()
                .map_err(|e| format!("Rate limiter error: {}", e))?;
            let (allowed, remaining, reset) =
                store.status(&key, limit, Duration::from_secs(window));

            let mut result: HashPairs = HashPairs::default();
            result.insert(HashKey::String("allowed".into()), Value::Bool(allowed));
            result.insert(
                HashKey::String("remaining".into()),
                Value::Int(remaining as i64),
            );
            result.insert(
                HashKey::String("reset_in".into()),
                Value::Int(reset.as_secs() as i64),
            );
            result.insert(HashKey::String("limit".into()), Value::Int(limit as i64));
            result.insert(HashKey::String("window".into()), Value::Int(window as i64));

            Ok(Value::Hash(Rc::new(RefCell::new(result))))
        })),
    );

    class_methods.insert(
        "headers".to_string(),
        Rc::new(NativeFunction::new(
            "RateLimiter.headers",
            Some(0),
            |args| {
                let this = match args.first() {
                    Some(Value::Instance(inst)) => inst,
                    _ => {
                        return Err(
                            "RateLimiter.headers() must be called on an instance".to_string()
                        )
                    }
                };
                let limit = match this.borrow().fields.get("limit").cloned() {
                    Some(Value::Int(i)) => i as usize,
                    _ => return Err("RateLimiter instance missing limit".to_string()),
                };
                let remaining = match this.borrow().fields.get("remaining").cloned() {
                    Some(Value::Int(i)) => i as usize,
                    _ => {
                        let key = match this.borrow().fields.get("key").cloned() {
                            Some(Value::String(s)) => s.clone(),
                            _ => return Err("RateLimiter instance missing key".to_string()),
                        };
                        let window = match this.borrow().fields.get("window").cloned() {
                            Some(Value::Int(i)) => i as u64,
                            _ => return Err("RateLimiter instance missing window".to_string()),
                        };
                        let store = RATE_LIMIT_STORE
                            .read()
                            .map_err(|e| format!("Rate limiter error: {}", e))?;
                        let (_, rem, _) = store.status(&key, limit, Duration::from_secs(window));
                        rem
                    }
                };
                let reset = match this.borrow().fields.get("reset").cloned() {
                    Some(Value::Int(i)) => i,
                    _ => {
                        let key = match this.borrow().fields.get("key").cloned() {
                            Some(Value::String(s)) => s.clone(),
                            _ => return Err("RateLimiter instance missing key".to_string()),
                        };
                        let window = match this.borrow().fields.get("window").cloned() {
                            Some(Value::Int(i)) => i as u64,
                            _ => return Err("RateLimiter instance missing window".to_string()),
                        };
                        let store = RATE_LIMIT_STORE
                            .read()
                            .map_err(|e| format!("Rate limiter error: {}", e))?;
                        let (_, _, reset_time) =
                            store.status(&key, limit, Duration::from_secs(window));
                        reset_time.as_secs() as i64
                    }
                };

                let mut headers: HashPairs = HashPairs::default();
                headers.insert(
                    HashKey::String("X-RateLimit-Limit".into()),
                    Value::String(limit.to_string().into()),
                );
                headers.insert(
                    HashKey::String("X-RateLimit-Remaining".into()),
                    Value::String(remaining.to_string().into()),
                );
                headers.insert(
                    HashKey::String("X-RateLimit-Reset".into()),
                    Value::String(reset.to_string().into()),
                );

                Ok(Value::Hash(Rc::new(RefCell::new(headers))))
            },
        )),
    );

    class_methods.insert(
        "reset".to_string(),
        Rc::new(NativeFunction::new("RateLimiter.reset", Some(0), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("RateLimiter.reset() must be called on an instance".to_string()),
            };
            let key = match this.borrow().fields.get("key").cloned() {
                Some(Value::String(s)) => s.clone(),
                _ => return Err("RateLimiter instance missing key".to_string()),
            };

            let mut store = RATE_LIMIT_STORE
                .write()
                .map_err(|e| format!("Rate limiter error: {}", e))?;
            store.buckets.remove(&*key);
            Ok(Value::Bool(true))
        })),
    );

    let mut static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    static_methods.insert(
        "reset_all".to_string(),
        Rc::new(NativeFunction::new(
            "RateLimiter.reset_all",
            Some(0),
            |_args| {
                let mut store = RATE_LIMIT_STORE
                    .write()
                    .map_err(|e| format!("Rate limiter error: {}", e))?;
                store.buckets.clear();
                Ok(Value::Bool(true))
            },
        )),
    );

    static_methods.insert(
        "cleanup".to_string(),
        Rc::new(NativeFunction::new(
            "RateLimiter.cleanup",
            Some(0),
            |_args| {
                let mut store = RATE_LIMIT_STORE
                    .write()
                    .map_err(|e| format!("Rate limiter error: {}", e))?;
                store.cleanup();
                Ok(Value::Bool(true))
            },
        )),
    );

    let class = Class {
        name: "RateLimiter".to_string(),
        superclass: None,
        methods: Rc::new(RefCell::new(HashMap::new())),
        static_methods: HashMap::new(),
        native_static_methods: static_methods,
        native_methods: class_methods,
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
    };

    let rate_limiter_class = Rc::new(class);
    env.define(
        "RateLimiter".to_string(),
        Value::Class(Rc::clone(&rate_limiter_class)),
    );

    let class_for_from_ip = Rc::clone(&rate_limiter_class);
    env.define(
        "rate_limiter_from_ip".to_string(),
        // Variadic, not `Some(2)`: the window is optional and the body has
        // always read `args.get(2)`, but an exact arity of 2 made the
        // three-argument form the docs describe fail with "Wrong number of
        // arguments: expected 2, got 3" before the body ever ran. The
        // argument count is checked here instead.
        Value::NativeFunction(NativeFunction::new(
            "rate_limiter_from_ip",
            None,
            move |args| {
                if args.len() < 2 || args.len() > 3 {
                    return Err(format!(
                        "rate_limiter_from_ip(req, limit, window_seconds?) expects 2 or 3 arguments, got {}",
                        args.len()
                    ));
                }
                let req = match &args[0] {
                    Value::Hash(_) => &args[0],
                    other => {
                        return Err(format!(
                            "rate_limiter_from_ip() expects hash req, got {}",
                            other.type_name()
                        ))
                    }
                };
                let limit = match &args[1] {
                    Value::Int(i) => *i as usize,
                    other => {
                        return Err(format!(
                            "rate_limiter_from_ip() expects int limit, got {}",
                            other.type_name()
                        ))
                    }
                };
                // A non-integer window used to fall back to 60 silently, so a
                // typo produced a limiter with a window nobody asked for.
                let window = match args.get(2) {
                    None | Some(Value::Null) => 60,
                    Some(Value::Int(i)) => *i as u64,
                    Some(other) => {
                        return Err(format!(
                            "rate_limiter_from_ip() expects int window_seconds, got {}",
                            other.type_name()
                        ))
                    }
                };

                let ip = extract_client_ip(req).unwrap_or_default();
                let key = format!("ip:{}", rate_limit_ip_key(&ip));

                let mut inst = Instance::new(Rc::clone(&class_for_from_ip));
                inst.set("key", Value::String(key.into()));
                inst.set("limit", Value::Int(limit as i64));
                inst.set("window", Value::Int(window as i64));
                inst.set("reset", Value::Int(0));
                Ok(Value::Instance(Rc::new(RefCell::new(inst))))
            },
        )),
    );

    let deprecated_function = |name: String| {
        Value::NativeFunction(NativeFunction::new(name.clone(), Some(0), move |_args| {
            Ok(Value::String(
                format!(
                    "{}() has been removed. Use RateLimiter class instead.",
                    name
                )
                .into(),
            ))
        }))
    };

    env.define(
        "rate_limit".to_string(),
        deprecated_function("rate_limit".to_string()),
    );
    env.define(
        "throttle".to_string(),
        deprecated_function("throttle".to_string()),
    );
    env.define(
        "rate_limit_status".to_string(),
        deprecated_function("rate_limit_status".to_string()),
    );
    env.define(
        "rate_limit_headers".to_string(),
        deprecated_function("rate_limit_headers".to_string()),
    );
    env.define(
        "rate_limit_reset".to_string(),
        deprecated_function("rate_limit_reset".to_string()),
    );
    env.define(
        "rate_limit_reset_all".to_string(),
        deprecated_function("rate_limit_reset_all".to_string()),
    );
    env.define(
        "rate_limit_cleanup".to_string(),
        deprecated_function("rate_limit_cleanup".to_string()),
    );
    env.define(
        "rate_limit_ip".to_string(),
        deprecated_function("rate_limit_ip".to_string()),
    );
}

/// Prefix length an IPv6 client is aggregated to for rate limiting.
///
/// A residential IPv6 allocation is normally a 64-bit prefix, giving one
/// household 2^64 addresses it can use freely. Keying on the full address
/// therefore meant a single host could take a fresh bucket for every request,
/// so the login and password-reset throttles in the auth scaffold simply never
/// tripped. IPv4 has no equivalent (an address is the unit), so it is keyed
/// whole. `SOLI_RATE_LIMIT_IPV6_PREFIX` widens or narrows this: 56 or 48
/// aggregates a whole site.
fn ipv6_prefix_len() -> u8 {
    std::env::var("SOLI_RATE_LIMIT_IPV6_PREFIX")
        .ok()
        .and_then(|v| v.trim().parse::<u8>().ok())
        .filter(|n| (1..=128).contains(n))
        .unwrap_or(64)
}

/// Bucket key for a client address: IPv4 verbatim, IPv6 aggregated to a prefix.
pub(crate) fn rate_limit_ip_key(ip: &str) -> String {
    let trimmed = ip.trim();
    let Ok(addr) = trimmed.parse::<std::net::IpAddr>() else {
        // Not an address (an empty string from a non-server context, or a
        // hostname): key it verbatim rather than silently collapsing callers.
        return trimmed.to_string();
    };
    let addr = match addr {
        // A dual-stack listener reports IPv4 peers as ::ffff:a.b.c.d; those are
        // IPv4 clients and must not be aggregated to a /64.
        std::net::IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => std::net::IpAddr::V4(v4),
            None => std::net::IpAddr::V6(v6),
        },
        other => other,
    };
    match addr {
        std::net::IpAddr::V4(v4) => v4.to_string(),
        std::net::IpAddr::V6(v6) => {
            let prefix_len = ipv6_prefix_len();
            let mut octets = v6.octets();
            let full_bytes = (prefix_len / 8) as usize;
            let remaining_bits = prefix_len % 8;
            if remaining_bits > 0 {
                octets[full_bytes] &= 0xffu8 << (8 - remaining_bits);
            }
            for byte in octets
                .iter_mut()
                .skip(full_bytes + usize::from(remaining_bits > 0))
            {
                *byte = 0;
            }
            format!("{}/{}", std::net::Ipv6Addr::from(octets), prefix_len)
        }
    }
}

/// SEC-030: derive a stable rate-limit key from the request.
///
/// Previously this function unconditionally trusted `X-Forwarded-For` from
/// the request headers — on a directly-exposed app, an attacker rotating
/// `X-Forwarded-For: <random>` per request would mint a fresh bucket each
/// time and bypass the limiter (including for login endpoints).
///
/// Behavior:
/// * `enable_trust_proxy()` ON: read `X-Forwarded-For` and take the
///   right-most entry. With a standard nginx-style
///   `proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for`,
///   the rightmost entry is the actual client as seen by the trusted
///   proxy; whatever the client tried to spoof in the header sits to
///   the left and is ignored.
/// * trust-proxy OFF (default): use the request hash's `remote_addr`
///   (the actual TCP peer IP populated by `serve/mod.rs`). Falls back
///   to `None` only when called from a non-server context.
fn extract_client_ip(req: &Value) -> Option<String> {
    let hash = match req {
        Value::Hash(h) => h,
        _ => return None,
    };
    let borrowed = hash.borrow();

    if super::trust_proxy::is_trust_proxy_enabled() {
        if let Some((_, Value::Hash(headers))) = borrowed
            .iter()
            .find(|(k, _)| matches!(k, HashKey::String(s) if **s == *"headers"))
        {
            let h_borrowed = headers.borrow();
            let xff = h_borrowed
                .iter()
                .find(|(k, _)| {
                    matches!(k, HashKey::String(s) if **s == *"x-forwarded-for" || **s == *"X-Forwarded-For")
                })
                .and_then(|(_, v)| {
                    if let Value::String(s) = v {
                        Some(s.as_ref())
                    } else {
                        None
                    }
                });
            if let Some(xff) = xff {
                // Take the rightmost non-empty entry — that's the address
                // the trusted proxy chose to record.
                if let Some(ip) = xff.rsplit(',').map(|s| s.trim()).find(|s| !s.is_empty()) {
                    return Some(ip.to_string());
                }
            }
        }
    }

    borrowed
        .iter()
        .find(|(k, _)| matches!(k, HashKey::String(s) if **s == *"remote_addr"))
        .and_then(|(_, v)| {
            if let Value::String(s) = v {
                Some(s.clone())
            } else {
                None
            }
        })
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::value::HashPairs;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn rate_limit_store_caps_bucket_count() {
        let mut store = RateLimitStore::new();
        let window = Duration::from_secs(60);
        // Fill to the cap with unique keys.
        for i in 0..MAX_BUCKETS {
            let key = format!("k{i}");
            let bucket = store.get_or_create(&key, 10, window);
            let _ = bucket.is_allowed();
        }
        assert!(
            store.buckets.len() <= MAX_BUCKETS,
            "bucket count {} exceeded cap",
            store.buckets.len()
        );
        // Flooding more unique keys evicts existing buckets rather than
        // growing the store or collapsing into a shared bucket. The store
        // stays capped, and the just-requested key always gets its own
        // bucket with the caller's own limit/window (no cross-key mixing).
        for i in 0..100 {
            let key = format!("flood{i}");
            let _ = store.get_or_create(&key, 10, window).is_allowed();
            assert!(
                store.buckets.len() <= MAX_BUCKETS,
                "flood should evict, not grow past cap: {}",
                store.buckets.len()
            );
            assert!(
                store.buckets.contains_key(&key),
                "the just-requested key must keep its own dedicated bucket"
            );
        }
    }

    #[test]
    fn rate_limit_auto_cleanup_drops_empty_buckets() {
        let mut store = RateLimitStore::new();
        // Zero window → timestamps immediately expire on next retain.
        let bucket = store.get_or_create("ephemeral", 5, Duration::from_secs(0));
        let _ = bucket.is_allowed();
        // Force enough ops to trip auto-cleanup.
        for i in 0..CLEANUP_EVERY_OPS {
            let _ = store.get_or_create(&format!("t{i}"), 1, Duration::from_secs(0));
        }
        // Expired / empty buckets should have been reclaimed.
        assert!(
            !store.buckets.contains_key("ephemeral")
                || store
                    .buckets
                    .get("ephemeral")
                    .map(|b| b.requests.is_empty())
                    .unwrap_or(true),
            "auto-cleanup should reclaim idle buckets"
        );
    }

    /// Bundled into one test because `TRUST_PROXY_ENABLED` is process-global —
    /// running cases as separate `#[test]` items would race them under
    /// cargo's parallel runner.
    #[test]
    fn extract_client_ip_honours_trust_proxy_gate() {
        // Helper to build a request hash with the given headers and remote.
        let make_req = |xff: Option<&str>, remote: Option<&str>| -> Value {
            let mut req: HashPairs = HashPairs::default();
            if let Some(xff) = xff {
                let mut headers: HashPairs = HashPairs::default();
                headers.insert(
                    HashKey::String("x-forwarded-for".into()),
                    Value::String(xff.to_string().into()),
                );
                req.insert(
                    HashKey::String("headers".into()),
                    Value::Hash(Rc::new(RefCell::new(headers))),
                );
            }
            if let Some(remote) = remote {
                req.insert(
                    HashKey::String("remote_addr".into()),
                    Value::String(remote.to_string().into()),
                );
            }
            Value::Hash(Rc::new(RefCell::new(req)))
        };

        // Save and restore the global flag.
        let prev = super::super::trust_proxy::is_trust_proxy_enabled();

        // Trust-proxy OFF: XFF is ignored, remote_addr wins.
        // SEC-030 attack scenario: client rotates `X-Forwarded-For: <random>` per
        // request; without the gate, the limiter is bypassed.
        super::super::trust_proxy::TRUST_PROXY_ENABLED
            .store(false, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(
            extract_client_ip(&make_req(Some("1.1.1.1"), Some("203.0.113.5"))),
            Some("203.0.113.5".to_string()),
            "trust_proxy OFF must ignore X-Forwarded-For"
        );
        // Even if XFF rotates, the bucket key stays the peer IP.
        assert_eq!(
            extract_client_ip(&make_req(Some("99.99.99.99"), Some("203.0.113.5"))),
            Some("203.0.113.5".to_string()),
            "rotating XFF must not change the bucket when trust_proxy is off"
        );
        // No remote_addr → None (limiter falls back to a sane default).
        assert!(extract_client_ip(&make_req(Some("1.1.1.1"), None)).is_none());

        // Trust-proxy ON: take the rightmost XFF entry — the address the
        // trusted proxy chose to record.
        super::super::trust_proxy::TRUST_PROXY_ENABLED
            .store(true, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(
            extract_client_ip(&make_req(Some("1.2.3.4"), Some("203.0.113.5"))),
            Some("1.2.3.4".to_string()),
            "single-hop XFF should resolve to the recorded client"
        );
        // `X-Forwarded-For: <spoofed>, <real>` — nginx-style append puts
        // the real client at the right.
        assert_eq!(
            extract_client_ip(&make_req(Some("99.99.99.99, 1.2.3.4"), Some("203.0.113.5"))),
            Some("1.2.3.4".to_string()),
            "rightmost XFF entry wins under trust_proxy"
        );
        // Trailing/leading whitespace tolerated.
        assert_eq!(
            extract_client_ip(&make_req(Some("a, b ,  1.2.3.4   "), None)),
            Some("1.2.3.4".to_string())
        );
        // Empty XFF falls back to remote_addr.
        assert_eq!(
            extract_client_ip(&make_req(Some(""), Some("203.0.113.5"))),
            Some("203.0.113.5".to_string())
        );

        // Restore.
        super::super::trust_proxy::TRUST_PROXY_ENABLED
            .store(prev, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(test)]
mod ip_key_tests {
    use super::*;

    /// An IPv4 address is the unit of identity and is keyed whole.
    #[test]
    fn ipv4_addresses_key_individually() {
        assert_eq!(rate_limit_ip_key("203.0.113.7"), "203.0.113.7");
        assert_ne!(
            rate_limit_ip_key("203.0.113.7"),
            rate_limit_ip_key("203.0.113.8")
        );
    }

    /// The bypass: a residential IPv6 allocation is a /64, so one host could
    /// take a fresh bucket per request and never trip the login throttle.
    /// Every address in a /64 must now share one bucket.
    #[test]
    fn ipv6_addresses_in_one_slash_64_share_a_bucket() {
        let a = rate_limit_ip_key("2001:db8:1:2::1");
        let b = rate_limit_ip_key("2001:db8:1:2:ffff:ffff:ffff:ffff");
        assert_eq!(a, b, "one /64 must be one bucket");
        // A different /64 is still a different client.
        assert_ne!(a, rate_limit_ip_key("2001:db8:1:3::1"));
    }

    /// A dual-stack listener reports IPv4 peers as ::ffff:a.b.c.d. Those are
    /// IPv4 clients and must not be collapsed into a /64 with their neighbours.
    #[test]
    fn ipv4_mapped_addresses_are_treated_as_ipv4() {
        assert_eq!(rate_limit_ip_key("::ffff:203.0.113.7"), "203.0.113.7");
        assert_ne!(
            rate_limit_ip_key("::ffff:203.0.113.7"),
            rate_limit_ip_key("::ffff:203.0.113.8")
        );
    }

    /// Anything that is not an address (no server context, a hostname) is keyed
    /// verbatim rather than collapsed into one shared bucket.
    #[test]
    fn non_addresses_are_kept_verbatim() {
        assert_eq!(rate_limit_ip_key(""), "");
        assert_eq!(rate_limit_ip_key("not-an-ip"), "not-an-ip");
    }

    /// Eviction under a key flood must take the least recently active bucket,
    /// not an arbitrary one — evicting a live limiter is itself a bypass.
    #[test]
    fn eviction_prefers_the_least_recently_active_bucket() {
        let mut store = RateLimitStore::new();
        let window = Duration::from_secs(60);

        // Fill the store, touching each key once as it is created.
        for i in 0..MAX_BUCKETS {
            let key = format!("k{i}");
            store.get_or_create(&key, 100, window).is_allowed();
        }
        // Re-touch one early key so it becomes the most recently active.
        store.get_or_create("k0", 100, window).is_allowed();

        // One more key forces an eviction.
        store.get_or_create("flood", 100, window).is_allowed();

        assert!(
            store.buckets.contains_key("k0"),
            "the freshly used bucket must survive an eviction"
        );
        assert!(store.buckets.len() <= MAX_BUCKETS);
    }
}
