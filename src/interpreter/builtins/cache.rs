use crate::interpreter::environment::Environment;
use crate::interpreter::value::value_to_json;
use crate::interpreter::value::{Class, HashKey, Instance, NativeFunction, Value};
use indexmap::IndexMap;
use lazy_static::lazy_static;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::RwLock;
use std::time::{Duration, Instant};

lazy_static! {
    static ref CACHE_STORE: RwLock<CacheStore> = RwLock::new(CacheStore::new());
    static ref CACHE_CONFIG: RwLock<CacheConfig> = RwLock::new(CacheConfig::default());
}

const DEFAULT_TTL_SECONDS: u64 = 3600;
const DEFAULT_MAX_SIZE: usize = 10000;

struct CacheEntry {
    value: String,
    expires_at: Instant,
}

struct CacheStore {
    entries: HashMap<String, CacheEntry>,
}

impl CacheStore {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }
}

struct CacheConfig {
    default_ttl: Duration,
    max_size: usize,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            default_ttl: Duration::from_secs(DEFAULT_TTL_SECONDS),
            max_size: DEFAULT_MAX_SIZE,
        }
    }
}

fn evict_expired_or_oldest(store: &mut CacheStore) {
    let now = Instant::now();
    let mut expired: Vec<String> = store
        .entries
        .iter()
        .filter(|(_, entry)| entry.expires_at <= now)
        .map(|(k, _)| k.clone())
        .collect();

    if expired.is_empty() {
        if let Some((oldest_key, _)) = store
            .entries
            .iter()
            .min_by_key(|(_, entry)| entry.expires_at)
        {
            expired.push(oldest_key.clone());
        }
    }

    for key in expired {
        store.entries.remove(&key);
    }
}

fn json_to_value(json: &serde_json::Value) -> Result<Value, String> {
    match json {
        serde_json::Value::Null => Ok(Value::Null),
        serde_json::Value::Bool(b) => Ok(Value::Bool(*b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(Value::Int(i))
            } else if let Some(f) = n.as_f64() {
                Ok(Value::Float(f))
            } else {
                Ok(Value::String(n.to_string()))
            }
        }
        serde_json::Value::String(s) => Ok(Value::String(s.clone())),
        serde_json::Value::Array(arr) => {
            let values: Result<Vec<Value>, String> = arr.iter().map(json_to_value).collect();
            Ok(Value::Array(Rc::new(RefCell::new(values?))))
        }
        serde_json::Value::Object(obj) => {
            let mut result: IndexMap<HashKey, Value> = IndexMap::new();
            for (k, v) in obj.iter() {
                result.insert(HashKey::String(k.clone()), json_to_value(v)?);
            }
            Ok(Value::Hash(Rc::new(RefCell::new(result))))
        }
    }
}

pub fn register_cache_builtins(env: &mut Environment) {
    let mut cache_static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    cache_static_methods.insert(
        "set".to_string(),
        Rc::new(NativeFunction::new("Cache.set", Some(2), |args| {
            let key = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "Cache.set() expects string key, got {}",
                        other.type_name()
                    ))
                }
            };
            let value = &args[1];
            let ttl = args
                .get(2)
                .and_then(|v| match v {
                    Value::Int(i) => Some(*i as u64),
                    _ => None,
                })
                .unwrap_or(DEFAULT_TTL_SECONDS);

            let json = value_to_json(value)
                .map_err(|e| format!("Cache.set() failed to serialize value: {}", e))?;
            let json_str = json.to_string();

            let mut store = CACHE_STORE
                .write()
                .map_err(|e| format!("Cache error: {}", e))?;

            let config = CACHE_CONFIG
                .read()
                .map_err(|e| format!("Cache config error: {}", e))?;
            if store.entries.len() >= config.max_size {
                evict_expired_or_oldest(&mut store);
            }

            store.entries.insert(
                key,
                CacheEntry {
                    value: json_str,
                    expires_at: Instant::now() + Duration::from_secs(ttl),
                },
            );

            Ok(Value::Null)
        })),
    );

    cache_static_methods.insert(
        "get".to_string(),
        Rc::new(NativeFunction::new("Cache.get", Some(1), |args| {
            let key = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "Cache.get() expects string key, got {}",
                        other.type_name()
                    ))
                }
            };

            let store_try = CACHE_STORE.try_read();
            let store = match store_try {
                Ok(s) => s,
                Err(_) => return Err("Cache is busy".to_string()),
            };

            if let Some(entry) = store.entries.get(&key) {
                if entry.expires_at > Instant::now() {
                    drop(store);
                    let store = CACHE_STORE
                        .read()
                        .map_err(|e| format!("Cache error: {}", e))?;
                    if let Some(entry) = store.entries.get(&key) {
                        if entry.expires_at > Instant::now() {
                            let json: serde_json::Value = serde_json::from_str(&entry.value)
                                .map_err(|e| format!("Cache deserialization error: {}", e))?;
                            return json_to_value(&json);
                        }
                    }
                }
            }

            Ok(Value::Null)
        })),
    );

    cache_static_methods.insert(
        "delete".to_string(),
        Rc::new(NativeFunction::new("Cache.delete", Some(1), |args| {
            let key = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "Cache.delete() expects string key, got {}",
                        other.type_name()
                    ))
                }
            };

            let mut store = CACHE_STORE
                .write()
                .map_err(|e| format!("Cache error: {}", e))?;
            let removed = store.entries.remove(&key);
            Ok(Value::Bool(removed.is_some()))
        })),
    );

    cache_static_methods.insert(
        "has".to_string(),
        Rc::new(NativeFunction::new("Cache.has", Some(1), |args| {
            let key = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "Cache.has() expects string key, got {}",
                        other.type_name()
                    ))
                }
            };

            let store = CACHE_STORE
                .read()
                .map_err(|e| format!("Cache error: {}", e))?;
            if let Some(entry) = store.entries.get(&key) {
                return Ok(Value::Bool(entry.expires_at > Instant::now()));
            }
            Ok(Value::Bool(false))
        })),
    );

    cache_static_methods.insert(
        "clear".to_string(),
        Rc::new(NativeFunction::new("Cache.clear", Some(0), |_args| {
            let mut store = CACHE_STORE
                .write()
                .map_err(|e| format!("Cache error: {}", e))?;
            store.entries.clear();
            Ok(Value::Null)
        })),
    );

    cache_static_methods.insert(
        "clear_expired".to_string(),
        Rc::new(NativeFunction::new(
            "Cache.clear_expired",
            Some(0),
            |_args| {
                let mut store = CACHE_STORE
                    .write()
                    .map_err(|e| format!("Cache error: {}", e))?;
                let now = Instant::now();
                store.entries.retain(|_, entry| entry.expires_at > now);
                Ok(Value::Null)
            },
        )),
    );

    cache_static_methods.insert(
        "keys".to_string(),
        Rc::new(NativeFunction::new("Cache.keys", Some(0), |_args| {
            let store = CACHE_STORE
                .read()
                .map_err(|e| format!("Cache error: {}", e))?;
            let now = Instant::now();
            let keys: Vec<Value> = store
                .entries
                .iter()
                .filter(|(_, entry)| entry.expires_at > now)
                .map(|(k, _)| Value::String(k.clone()))
                .collect();
            Ok(Value::Array(Rc::new(RefCell::new(keys))))
        })),
    );

    cache_static_methods.insert(
        "size".to_string(),
        Rc::new(NativeFunction::new("Cache.size", Some(0), |_args| {
            let store = CACHE_STORE
                .read()
                .map_err(|e| format!("Cache error: {}", e))?;
            Ok(Value::Int(store.entries.len() as i64))
        })),
    );

    cache_static_methods.insert(
        "ttl".to_string(),
        Rc::new(NativeFunction::new("Cache.ttl", Some(1), |args| {
            let key = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "Cache.ttl() expects string key, got {}",
                        other.type_name()
                    ))
                }
            };

            let store = CACHE_STORE
                .read()
                .map_err(|e| format!("Cache error: {}", e))?;
            if let Some(entry) = store.entries.get(&key) {
                if entry.expires_at > Instant::now() {
                    let remaining = entry.expires_at.duration_since(Instant::now());
                    return Ok(Value::Int(remaining.as_secs() as i64));
                }
            }
            Ok(Value::Null)
        })),
    );

    cache_static_methods.insert(
        "touch".to_string(),
        Rc::new(NativeFunction::new("Cache.touch", Some(2), |args| {
            let key = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "Cache.touch() expects string key, got {}",
                        other.type_name()
                    ))
                }
            };
            let ttl = match &args[1] {
                Value::Int(i) => *i as u64,
                other => {
                    return Err(format!(
                        "Cache.touch() expects int ttl, got {}",
                        other.type_name()
                    ))
                }
            };

            let mut store = CACHE_STORE
                .write()
                .map_err(|e| format!("Cache error: {}", e))?;
            if let Some(entry) = store.entries.get_mut(&key) {
                if entry.expires_at > Instant::now() {
                    entry.expires_at = Instant::now() + Duration::from_secs(ttl);
                    return Ok(Value::Bool(true));
                }
            }
            Ok(Value::Bool(false))
        })),
    );

    let cache_class_rc = Rc::new(Class {
        name: "Cache".to_string(),
        superclass: None,
        methods: HashMap::new(),
        static_methods: HashMap::new(),
        native_static_methods: cache_static_methods,
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        all_methods_cache: RefCell::new(None),
        all_native_methods_cache: RefCell::new(None),
    });

    env.define(
        "Cache".to_string(),
        Value::Class(Rc::clone(&cache_class_rc)),
    );

    let cache_class_for_closure = Rc::clone(&cache_class_rc);
    env.define(
        "cache".to_string(),
        Value::NativeFunction(NativeFunction::new("cache", Some(0), move |_args| {
            let inst = Instance::new(Rc::clone(&cache_class_for_closure));
            Ok(Value::Instance(Rc::new(RefCell::new(inst))))
        })),
    );

    env.define(
        "cache_set".to_string(),
        Value::NativeFunction(NativeFunction::new("cache_set", Some(2), |args| {
            let key = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "cache_set() expects string key, got {}",
                        other.type_name()
                    ))
                }
            };
            let value = &args[1];
            let ttl = args
                .get(2)
                .and_then(|v| match v {
                    Value::Int(i) => Some(*i as u64),
                    _ => None,
                })
                .unwrap_or(DEFAULT_TTL_SECONDS);

            let json = value_to_json(value)
                .map_err(|e| format!("cache_set() failed to serialize value: {}", e))?;
            let json_str = json.to_string();

            let mut store = CACHE_STORE
                .write()
                .map_err(|e| format!("Cache error: {}", e))?;

            let config = CACHE_CONFIG
                .read()
                .map_err(|e| format!("Cache config error: {}", e))?;
            if store.entries.len() >= config.max_size {
                evict_expired_or_oldest(&mut store);
            }

            store.entries.insert(
                key,
                CacheEntry {
                    value: json_str,
                    expires_at: Instant::now() + Duration::from_secs(ttl),
                },
            );

            Ok(Value::Null)
        })),
    );

    env.define(
        "cache_get".to_string(),
        Value::NativeFunction(NativeFunction::new("cache_get", Some(1), |args| {
            let key = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "cache_get() expects string key, got {}",
                        other.type_name()
                    ))
                }
            };

            let store_try = CACHE_STORE.try_read();
            let store = match store_try {
                Ok(s) => s,
                Err(_) => return Err("Cache is busy".to_string()),
            };

            if let Some(entry) = store.entries.get(&key) {
                if entry.expires_at > Instant::now() {
                    drop(store);
                    let store = CACHE_STORE
                        .read()
                        .map_err(|e| format!("Cache error: {}", e))?;
                    if let Some(entry) = store.entries.get(&key) {
                        if entry.expires_at > Instant::now() {
                            let json: serde_json::Value = serde_json::from_str(&entry.value)
                                .map_err(|e| format!("Cache deserialization error: {}", e))?;
                            return json_to_value(&json);
                        }
                    }
                }
            }

            Ok(Value::Null)
        })),
    );

    env.define(
        "cache_delete".to_string(),
        Value::NativeFunction(NativeFunction::new("cache_delete", Some(1), |args| {
            let key = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "cache_delete() expects string key, got {}",
                        other.type_name()
                    ))
                }
            };

            let mut store = CACHE_STORE
                .write()
                .map_err(|e| format!("Cache error: {}", e))?;
            let removed = store.entries.remove(&key);
            Ok(Value::Bool(removed.is_some()))
        })),
    );

    env.define(
        "cache_has".to_string(),
        Value::NativeFunction(NativeFunction::new("cache_has", Some(1), |args| {
            let key = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "cache_has() expects string key, got {}",
                        other.type_name()
                    ))
                }
            };

            let store = CACHE_STORE
                .read()
                .map_err(|e| format!("Cache error: {}", e))?;
            if let Some(entry) = store.entries.get(&key) {
                return Ok(Value::Bool(entry.expires_at > Instant::now()));
            }
            Ok(Value::Bool(false))
        })),
    );

    env.define(
        "cache_clear".to_string(),
        Value::NativeFunction(NativeFunction::new("cache_clear", Some(0), |_args| {
            let mut store = CACHE_STORE
                .write()
                .map_err(|e| format!("Cache error: {}", e))?;
            store.entries.clear();
            Ok(Value::Null)
        })),
    );

    env.define(
        "cache_clear_expired".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "cache_clear_expired",
            Some(0),
            |_args| {
                let mut store = CACHE_STORE
                    .write()
                    .map_err(|e| format!("Cache error: {}", e))?;
                let now = Instant::now();
                store.entries.retain(|_, entry| entry.expires_at > now);
                Ok(Value::Null)
            },
        )),
    );

    env.define(
        "cache_keys".to_string(),
        Value::NativeFunction(NativeFunction::new("cache_keys", Some(0), |_args| {
            let store = CACHE_STORE
                .read()
                .map_err(|e| format!("Cache error: {}", e))?;
            let now = Instant::now();
            let keys: Vec<Value> = store
                .entries
                .iter()
                .filter(|(_, entry)| entry.expires_at > now)
                .map(|(k, _)| Value::String(k.clone()))
                .collect();
            Ok(Value::Array(Rc::new(RefCell::new(keys))))
        })),
    );

    env.define(
        "cache_ttl".to_string(),
        Value::NativeFunction(NativeFunction::new("cache_ttl", Some(1), |args| {
            let key = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "cache_ttl() expects string key, got {}",
                        other.type_name()
                    ))
                }
            };

            let store = CACHE_STORE
                .read()
                .map_err(|e| format!("Cache error: {}", e))?;
            if let Some(entry) = store.entries.get(&key) {
                if entry.expires_at > Instant::now() {
                    let remaining = entry.expires_at.duration_since(Instant::now());
                    return Ok(Value::Int(remaining.as_secs() as i64));
                }
            }
            Ok(Value::Null)
        })),
    );

    env.define(
        "cache_touch".to_string(),
        Value::NativeFunction(NativeFunction::new("cache_touch", Some(2), |args| {
            let key = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "cache_touch() expects string key, got {}",
                        other.type_name()
                    ))
                }
            };
            let ttl = match &args[1] {
                Value::Int(i) => *i as u64,
                other => {
                    return Err(format!(
                        "cache_touch() expects int ttl, got {}",
                        other.type_name()
                    ))
                }
            };

            let mut store = CACHE_STORE
                .write()
                .map_err(|e| format!("Cache error: {}", e))?;
            if let Some(entry) = store.entries.get_mut(&key) {
                if entry.expires_at > Instant::now() {
                    entry.expires_at = Instant::now() + Duration::from_secs(ttl);
                    return Ok(Value::Bool(true));
                }
            }
            Ok(Value::Bool(false))
        })),
    );

    env.define(
        "cache_size".to_string(),
        Value::NativeFunction(NativeFunction::new("cache_size", Some(0), |_args| {
            let store = CACHE_STORE
                .read()
                .map_err(|e| format!("Cache error: {}", e))?;
            Ok(Value::Int(store.entries.len() as i64))
        })),
    );

    env.define(
        "cache_config".to_string(),
        Value::NativeFunction(NativeFunction::new("cache_config", Some(2), |args| {
            let ttl = match &args[0] {
                Value::Int(i) => Some(*i as u64),
                Value::Null => None,
                other => {
                    return Err(format!(
                        "cache_config() expects int or null ttl, got {}",
                        other.type_name()
                    ))
                }
            };
            let max_size = match &args[1] {
                Value::Int(i) => Some(*i as usize),
                Value::Null => None,
                other => {
                    return Err(format!(
                        "cache_config() expects int or null max_size, got {}",
                        other.type_name()
                    ))
                }
            };

            let mut config = CACHE_CONFIG
                .write()
                .map_err(|e| format!("Cache config error: {}", e))?;
            if let Some(t) = ttl {
                config.default_ttl = Duration::from_secs(t);
            }
            if let Some(m) = max_size {
                config.max_size = m;
            }
            Ok(Value::Null)
        })),
    );
}
