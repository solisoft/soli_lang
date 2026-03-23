use super::solikv::{
    get_solikv_config, solikv_cmd, solikv_configure, solikv_del, solikv_get, solikv_set,
};
use crate::interpreter::environment::Environment;
use crate::interpreter::value::{
    json_to_value, stringify_to_string, Class, Instance, NativeFunction, Value,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

const DEFAULT_TTL_SECONDS: u64 = 3600;

fn prefixed_key(key: &str) -> String {
    let cfg = get_solikv_config().read().unwrap();
    format!("{}{}", cfg.prefix, key)
}

fn strip_prefix(full_key: &str) -> String {
    let cfg = get_solikv_config().read().unwrap();
    full_key
        .strip_prefix(cfg.prefix.as_str())
        .unwrap_or(full_key)
        .to_string()
}

pub(crate) fn cache_set_impl(key: &str, value: &Value, ttl: Option<u64>) -> Result<Value, String> {
    let pkey = prefixed_key(key);
    let ttl = ttl.unwrap_or_else(|| {
        get_solikv_config()
            .read()
            .map(|c| c.default_ttl)
            .unwrap_or(DEFAULT_TTL_SECONDS)
    });

    let json_str = stringify_to_string(value)
        .map_err(|e| format!("Cache.set() failed to serialize value: {}", e))?;

    solikv_set(&pkey, &json_str, Some(ttl))?;
    Ok(Value::Null)
}

pub(crate) fn cache_get_impl(key: &str) -> Result<Value, String> {
    let pkey = prefixed_key(key);
    match solikv_get(&pkey)? {
        None => Ok(Value::Null),
        Some(s) => {
            let parsed: serde_json::Value = serde_json::from_str(&s)
                .map_err(|e| format!("Cache deserialization error: {}", e))?;
            json_to_value(parsed)
        }
    }
}

fn cache_has_impl(key: &str) -> Result<Value, String> {
    let pkey = prefixed_key(key);
    let result = solikv_cmd(&["EXISTS", &pkey])?;
    let exists = result.as_i64().unwrap_or(0) > 0;
    Ok(Value::Bool(exists))
}

fn cache_delete_impl(key: &str) -> Result<Value, String> {
    let pkey = prefixed_key(key);
    let count = solikv_del(&pkey)?;
    Ok(Value::Bool(count > 0))
}

fn cache_clear_impl() -> Result<Value, String> {
    let pattern = {
        let cfg = get_solikv_config().read().map_err(|e| e.to_string())?;
        format!("{}*", cfg.prefix)
    };
    let keys_result = solikv_cmd(&["KEYS", &pattern])?;

    if let Some(arr) = keys_result.as_array() {
        for key in arr {
            if let Some(k) = key.as_str() {
                solikv_cmd(&["DEL", k])?;
            }
        }
    }
    Ok(Value::Null)
}

fn cache_keys_impl() -> Result<Value, String> {
    let pattern = {
        let cfg = get_solikv_config().read().map_err(|e| e.to_string())?;
        format!("{}*", cfg.prefix)
    };
    let keys_result = solikv_cmd(&["KEYS", &pattern])?;

    let keys: Vec<Value> = keys_result
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| Value::String(strip_prefix(s))))
                .collect()
        })
        .unwrap_or_default();
    Ok(Value::Array(Rc::new(RefCell::new(keys))))
}

fn cache_size_impl() -> Result<Value, String> {
    let pattern = {
        let cfg = get_solikv_config().read().map_err(|e| e.to_string())?;
        format!("{}*", cfg.prefix)
    };
    let keys_result = solikv_cmd(&["KEYS", &pattern])?;
    let count = keys_result.as_array().map(|a| a.len()).unwrap_or(0);
    Ok(Value::Int(count as i64))
}

fn cache_ttl_impl(key: &str) -> Result<Value, String> {
    let pkey = prefixed_key(key);
    let result = solikv_cmd(&["TTL", &pkey])?;
    match result.as_i64() {
        Some(ttl) if ttl >= 0 => Ok(Value::Int(ttl)),
        _ => Ok(Value::Null),
    }
}

fn cache_touch_impl(key: &str, ttl: u64) -> Result<Value, String> {
    let pkey = prefixed_key(key);
    let ttl_str = ttl.to_string();
    let result = solikv_cmd(&["EXPIRE", &pkey, &ttl_str])?;
    let ok = result.as_i64().unwrap_or(0) > 0;
    Ok(Value::Bool(ok))
}

fn extract_string_key(args: &[Value], fn_name: &str) -> Result<String, String> {
    match &args[0] {
        Value::String(s) => Ok(s.clone()),
        other => Err(format!(
            "{}() expects string key, got {}",
            fn_name,
            other.type_name()
        )),
    }
}

pub fn register_cache_builtins(env: &mut Environment) {
    let mut cache_static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    cache_static_methods.insert(
        "set".to_string(),
        Rc::new(NativeFunction::new("Cache.set", None, |args| {
            if args.len() < 2 || args.len() > 3 {
                return Err(format!(
                    "Cache.set() expects 2-3 arguments, got {}",
                    args.len()
                ));
            }
            let key = extract_string_key(&args, "Cache.set")?;
            let value = &args[1];
            let ttl = args.get(2).and_then(|v| match v {
                Value::Int(i) => Some(*i as u64),
                _ => None,
            });
            cache_set_impl(&key, value, ttl)
        })),
    );

    cache_static_methods.insert(
        "get".to_string(),
        Rc::new(NativeFunction::new("Cache.get", Some(1), |args| {
            let key = extract_string_key(&args, "Cache.get")?;
            cache_get_impl(&key)
        })),
    );

    cache_static_methods.insert(
        "delete".to_string(),
        Rc::new(NativeFunction::new("Cache.delete", Some(1), |args| {
            let key = extract_string_key(&args, "Cache.delete")?;
            cache_delete_impl(&key)
        })),
    );

    cache_static_methods.insert(
        "has".to_string(),
        Rc::new(NativeFunction::new("Cache.has", Some(1), |args| {
            let key = extract_string_key(&args, "Cache.has")?;
            cache_has_impl(&key)
        })),
    );

    cache_static_methods.insert(
        "clear".to_string(),
        Rc::new(NativeFunction::new("Cache.clear", Some(0), |_args| {
            cache_clear_impl()
        })),
    );

    cache_static_methods.insert(
        "clear_expired".to_string(),
        Rc::new(NativeFunction::new(
            "Cache.clear_expired",
            Some(0),
            |_args| {
                // No-op: SoliKV handles TTL expiration automatically
                Ok(Value::Null)
            },
        )),
    );

    cache_static_methods.insert(
        "keys".to_string(),
        Rc::new(NativeFunction::new("Cache.keys", Some(0), |_args| {
            cache_keys_impl()
        })),
    );

    cache_static_methods.insert(
        "size".to_string(),
        Rc::new(NativeFunction::new("Cache.size", Some(0), |_args| {
            cache_size_impl()
        })),
    );

    cache_static_methods.insert(
        "ttl".to_string(),
        Rc::new(NativeFunction::new("Cache.ttl", Some(1), |args| {
            let key = extract_string_key(&args, "Cache.ttl")?;
            cache_ttl_impl(&key)
        })),
    );

    cache_static_methods.insert(
        "touch".to_string(),
        Rc::new(NativeFunction::new("Cache.touch", Some(2), |args| {
            let key = extract_string_key(&args, "Cache.touch")?;
            let ttl = match &args[1] {
                Value::Int(i) => *i as u64,
                other => {
                    return Err(format!(
                        "Cache.touch() expects int ttl, got {}",
                        other.type_name()
                    ))
                }
            };
            cache_touch_impl(&key, ttl)
        })),
    );

    cache_static_methods.insert(
        "configure".to_string(),
        Rc::new(NativeFunction::new("Cache.configure", None, |args| {
            if args.is_empty() || args.len() > 2 {
                return Err(format!(
                    "Cache.configure() expects 1-2 arguments, got {}",
                    args.len()
                ));
            }
            let host = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "Cache.configure() expects string host, got {}",
                        other.type_name()
                    ))
                }
            };
            let token = args.get(1).and_then(|v| match v {
                Value::String(s) => Some(s.clone()),
                _ => None,
            });
            solikv_configure(&host, token);
            Ok(Value::Null)
        })),
    );

    let cache_class_rc = Rc::new(Class {
        name: "Cache".to_string(),
        superclass: None,
        methods: Rc::new(RefCell::new(HashMap::new())),
        static_methods: HashMap::new(),
        native_static_methods: cache_static_methods,
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
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

    // Global function wrappers

    env.define(
        "cache_set".to_string(),
        Value::NativeFunction(NativeFunction::new("cache_set", None, |args| {
            if args.len() < 2 || args.len() > 3 {
                return Err(format!(
                    "cache_set() expects 2-3 arguments, got {}",
                    args.len()
                ));
            }
            let key = extract_string_key(&args, "cache_set")?;
            let value = &args[1];
            let ttl = args.get(2).and_then(|v| match v {
                Value::Int(i) => Some(*i as u64),
                _ => None,
            });
            cache_set_impl(&key, value, ttl)
        })),
    );

    env.define(
        "cache_get".to_string(),
        Value::NativeFunction(NativeFunction::new("cache_get", Some(1), |args| {
            let key = extract_string_key(&args, "cache_get")?;
            cache_get_impl(&key)
        })),
    );

    env.define(
        "cache_delete".to_string(),
        Value::NativeFunction(NativeFunction::new("cache_delete", Some(1), |args| {
            let key = extract_string_key(&args, "cache_delete")?;
            cache_delete_impl(&key)
        })),
    );

    env.define(
        "cache_has".to_string(),
        Value::NativeFunction(NativeFunction::new("cache_has", Some(1), |args| {
            let key = extract_string_key(&args, "cache_has")?;
            cache_has_impl(&key)
        })),
    );

    env.define(
        "cache_clear".to_string(),
        Value::NativeFunction(NativeFunction::new("cache_clear", Some(0), |_args| {
            cache_clear_impl()
        })),
    );

    env.define(
        "cache_clear_expired".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "cache_clear_expired",
            Some(0),
            |_args| Ok(Value::Null),
        )),
    );

    env.define(
        "cache_keys".to_string(),
        Value::NativeFunction(NativeFunction::new("cache_keys", Some(0), |_args| {
            cache_keys_impl()
        })),
    );

    env.define(
        "cache_ttl".to_string(),
        Value::NativeFunction(NativeFunction::new("cache_ttl", Some(1), |args| {
            let key = extract_string_key(&args, "cache_ttl")?;
            cache_ttl_impl(&key)
        })),
    );

    env.define(
        "cache_touch".to_string(),
        Value::NativeFunction(NativeFunction::new("cache_touch", Some(2), |args| {
            let key = extract_string_key(&args, "cache_touch")?;
            let ttl = match &args[1] {
                Value::Int(i) => *i as u64,
                other => {
                    return Err(format!(
                        "cache_touch() expects int ttl, got {}",
                        other.type_name()
                    ))
                }
            };
            cache_touch_impl(&key, ttl)
        })),
    );

    env.define(
        "cache_size".to_string(),
        Value::NativeFunction(NativeFunction::new("cache_size", Some(0), |_args| {
            cache_size_impl()
        })),
    );

    env.define(
        "cache_config".to_string(),
        Value::NativeFunction(NativeFunction::new("cache_config", Some(1), |args| {
            let ttl = match &args[0] {
                Value::Int(i) => *i as u64,
                Value::Null => return Ok(Value::Null),
                other => {
                    return Err(format!(
                        "cache_config() expects int or null ttl, got {}",
                        other.type_name()
                    ))
                }
            };
            let cfg = get_solikv_config();
            let mut w = cfg.write().map_err(|e| e.to_string())?;
            w.default_ttl = ttl;
            Ok(Value::Null)
        })),
    );
}
