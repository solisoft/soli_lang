use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, Instance, NativeFunction, Value};
use lazy_static::lazy_static;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::RwLock;
use std::time::{Duration, Instant};

lazy_static! {
    static ref RATE_LIMIT_STORE: RwLock<RateLimitStore> = RwLock::new(RateLimitStore::new());
}

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
}

impl RateLimitStore {
    fn new() -> Self {
        Self {
            buckets: HashMap::new(),
        }
    }

    fn get_or_create(&mut self, key: &str, limit: usize, window: Duration) -> &mut RateLimitBucket {
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

            let result: Vec<(Value, Value)> = vec![
                (Value::String("allowed".to_string()), Value::Bool(allowed)),
                (
                    Value::String("remaining".to_string()),
                    Value::Int(remaining as i64),
                ),
                (
                    Value::String("reset_in".to_string()),
                    Value::Int(reset.as_secs() as i64),
                ),
                (Value::String("limit".to_string()), Value::Int(limit as i64)),
                (
                    Value::String("window".to_string()),
                    Value::Int(window as i64),
                ),
            ];

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

                let headers: Vec<(Value, Value)> = vec![
                    (
                        Value::String("X-RateLimit-Limit".to_string()),
                        Value::String(limit.to_string()),
                    ),
                    (
                        Value::String("X-RateLimit-Remaining".to_string()),
                        Value::String(remaining.to_string()),
                    ),
                    (
                        Value::String("X-RateLimit-Reset".to_string()),
                        Value::String(reset.to_string()),
                    ),
                ];

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
            store.buckets.remove(&key);
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
        methods: HashMap::new(),
        static_methods: HashMap::new(),
        native_static_methods: static_methods,
        native_methods: class_methods,
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
    };

    let rate_limiter_class = Rc::new(class);
    env.define(
        "RateLimiter".to_string(),
        Value::Class(Rc::clone(&rate_limiter_class)),
    );

    let class_for_from_ip = Rc::clone(&rate_limiter_class);
    env.define(
        "rate_limiter_from_ip".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "rate_limiter_from_ip",
            Some(2),
            move |args| {
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
                let window = args
                    .get(2)
                    .and_then(|v| match v {
                        Value::Int(i) => Some(*i as u64),
                        _ => None,
                    })
                    .unwrap_or(60);

                let ip = extract_client_ip(req).unwrap_or_default();
                let key = format!("ip:{}", ip);

                let mut inst = Instance::new(Rc::clone(&class_for_from_ip));
                inst.set("key".to_string(), Value::String(key));
                inst.set("limit".to_string(), Value::Int(limit as i64));
                inst.set("window".to_string(), Value::Int(window as i64));
                inst.set("reset".to_string(), Value::Int(0));
                Ok(Value::Instance(Rc::new(RefCell::new(inst))))
            },
        )),
    );

    let deprecated_function = |name: String| {
        Value::NativeFunction(NativeFunction::new(name.clone(), Some(0), move |_args| {
            Ok(Value::String(format!(
                "{}() has been removed. Use RateLimiter class instead.",
                name
            )))
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

fn extract_client_ip(req: &Value) -> Option<String> {
    match req {
        Value::Hash(hash) => {
            let borrowed = hash.borrow();
            borrowed.iter()
                .find(|(k, _)| matches!(k, Value::String(s) if s == "headers"))
                .and_then(|(_, h)| {
                    if let Value::Hash(headers) = h {
                        let h_borrowed = headers.borrow();
                        h_borrowed.iter()
                            .find(|(k, _)| matches!(k, Value::String(s) if s == "x-forwarded-for" || s == "X-Forwarded-For"))
                            .map(|(_, v)| {
                                if let Value::String(s) = v { s.clone() } else { String::new() }
                            })
                            .or_else(|| {
                                h_borrowed.iter()
                                    .find(|(k, _)| matches!(k, Value::String(s) if s == "remote_addr" || s == "Remote-Addr"))
                                    .map(|(_, v)| {
                                        if let Value::String(s) = v { s.clone() } else { String::new() }
                                    })
                            })
                    } else { None }
                })
        }
        _ => None,
    }
}
