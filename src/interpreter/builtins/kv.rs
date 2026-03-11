use super::solikv::{solikv_cmd, solikv_configure, solikv_del, solikv_get, solikv_set};
use crate::interpreter::environment::Environment;
use crate::interpreter::value::{json_to_value, Class, HashKey, HashPairs, NativeFunction, Value};

/// Convert a Value to a raw string for Redis storage (no JSON encoding).
fn value_to_raw(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => format!("{}", other),
    }
}
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

fn extract_string(
    args: &[Value],
    idx: usize,
    fn_name: &str,
    param: &str,
) -> Result<String, String> {
    match args.get(idx) {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(other) => Err(format!(
            "{}() expects string {}, got {}",
            fn_name,
            param,
            other.type_name()
        )),
        None => Err(format!("{}() missing argument: {}", fn_name, param)),
    }
}

fn extract_int(args: &[Value], idx: usize, fn_name: &str, param: &str) -> Result<i64, String> {
    match args.get(idx) {
        Some(Value::Int(i)) => Ok(*i),
        Some(other) => Err(format!(
            "{}() expects int {}, got {}",
            fn_name,
            param,
            other.type_name()
        )),
        None => Err(format!("{}() missing argument: {}", fn_name, param)),
    }
}

/// Convert a serde_json::Value from solikv_cmd to a Soli Value.
fn solikv_result_to_value(result: &serde_json::Value) -> Result<Value, String> {
    match result {
        serde_json::Value::Null => Ok(Value::Null),
        serde_json::Value::Bool(b) => Ok(Value::Bool(*b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(Value::Int(i))
            } else if let Some(f) = n.as_f64() {
                Ok(Value::Float(f))
            } else {
                Ok(Value::Null)
            }
        }
        serde_json::Value::String(s) => Ok(Value::String(s.clone())),
        serde_json::Value::Array(arr) => {
            let vals: Result<Vec<Value>, String> = arr.iter().map(solikv_result_to_value).collect();
            Ok(Value::Array(Rc::new(RefCell::new(vals?))))
        }
        other => json_to_value(other.clone()),
    }
}

pub fn register_kv_builtins(env: &mut Environment) {
    let mut kv_static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    // KV.set(key, value, ttl?)
    kv_static_methods.insert(
        "set".to_string(),
        Rc::new(NativeFunction::new("KV.set", None, |args| {
            if args.len() < 2 || args.len() > 3 {
                return Err("KV.set() expects 2 or 3 arguments (key, value, ttl?)".to_string());
            }
            let key = extract_string(&args, 0, "KV.set", "key")?;
            let value = &args[1];
            let ttl = args.get(2).and_then(|v| match v {
                Value::Int(i) => Some(*i as u64),
                _ => None,
            });

            let raw_str = value_to_raw(value);

            solikv_set(&key, &raw_str, ttl)?;
            Ok(Value::Null)
        })),
    );

    // KV.get(key)
    kv_static_methods.insert(
        "get".to_string(),
        Rc::new(NativeFunction::new("KV.get", Some(1), |args| {
            let key = extract_string(&args, 0, "KV.get", "key")?;
            match solikv_get(&key)? {
                None => Ok(Value::Null),
                Some(s) => Ok(Value::String(s)),
            }
        })),
    );

    // KV.delete(key)
    kv_static_methods.insert(
        "delete".to_string(),
        Rc::new(NativeFunction::new("KV.delete", Some(1), |args| {
            let key = extract_string(&args, 0, "KV.delete", "key")?;
            let count = solikv_del(&key)?;
            Ok(Value::Bool(count > 0))
        })),
    );

    // KV.exists(key)
    kv_static_methods.insert(
        "exists".to_string(),
        Rc::new(NativeFunction::new("KV.exists", Some(1), |args| {
            let key = extract_string(&args, 0, "KV.exists", "key")?;
            let result = solikv_cmd(&["EXISTS", &key])?;
            Ok(Value::Bool(result.as_i64().unwrap_or(0) > 0))
        })),
    );

    // KV.keys(pattern?)
    kv_static_methods.insert(
        "keys".to_string(),
        Rc::new(NativeFunction::new("KV.keys", None, |args| {
            if args.len() > 1 {
                return Err("KV.keys() expects 0 or 1 arguments (pattern?)".to_string());
            }
            let pattern = args
                .first()
                .and_then(|v| match v {
                    Value::String(s) => Some(s.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| "*".to_string());

            let result = solikv_cmd(&["KEYS", &pattern])?;
            let keys: Vec<Value> = result
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| Value::String(s.to_string())))
                        .collect()
                })
                .unwrap_or_default();
            Ok(Value::Array(Rc::new(RefCell::new(keys))))
        })),
    );

    // KV.ttl(key)
    kv_static_methods.insert(
        "ttl".to_string(),
        Rc::new(NativeFunction::new("KV.ttl", Some(1), |args| {
            let key = extract_string(&args, 0, "KV.ttl", "key")?;
            let result = solikv_cmd(&["TTL", &key])?;
            match result.as_i64() {
                Some(ttl) if ttl >= 0 => Ok(Value::Int(ttl)),
                _ => Ok(Value::Null),
            }
        })),
    );

    // KV.expire(key, seconds)
    kv_static_methods.insert(
        "expire".to_string(),
        Rc::new(NativeFunction::new("KV.expire", Some(2), |args| {
            let key = extract_string(&args, 0, "KV.expire", "key")?;
            let ttl = extract_int(&args, 1, "KV.expire", "seconds")?;
            let ttl_str = ttl.to_string();
            let result = solikv_cmd(&["EXPIRE", &key, &ttl_str])?;
            Ok(Value::Bool(result.as_i64().unwrap_or(0) > 0))
        })),
    );

    // KV.persist(key)
    kv_static_methods.insert(
        "persist".to_string(),
        Rc::new(NativeFunction::new("KV.persist", Some(1), |args| {
            let key = extract_string(&args, 0, "KV.persist", "key")?;
            let result = solikv_cmd(&["PERSIST", &key])?;
            Ok(Value::Bool(result.as_i64().unwrap_or(0) > 0))
        })),
    );

    // KV.rename(key, newkey)
    kv_static_methods.insert(
        "rename".to_string(),
        Rc::new(NativeFunction::new("KV.rename", Some(2), |args| {
            let key = extract_string(&args, 0, "KV.rename", "key")?;
            let newkey = extract_string(&args, 1, "KV.rename", "newkey")?;
            solikv_cmd(&["RENAME", &key, &newkey])?;
            Ok(Value::Null)
        })),
    );

    // KV.type(key)
    kv_static_methods.insert(
        "type".to_string(),
        Rc::new(NativeFunction::new("KV.type", Some(1), |args| {
            let key = extract_string(&args, 0, "KV.type", "key")?;
            let result = solikv_cmd(&["TYPE", &key])?;
            match result.as_str() {
                Some(s) => Ok(Value::String(s.to_string())),
                None => Ok(Value::Null),
            }
        })),
    );

    // --- Numeric operations ---

    kv_static_methods.insert(
        "incr".to_string(),
        Rc::new(NativeFunction::new("KV.incr", Some(1), |args| {
            let key = extract_string(&args, 0, "KV.incr", "key")?;
            let result = solikv_cmd(&["INCR", &key])?;
            Ok(Value::Int(result.as_i64().unwrap_or(0)))
        })),
    );

    kv_static_methods.insert(
        "decr".to_string(),
        Rc::new(NativeFunction::new("KV.decr", Some(1), |args| {
            let key = extract_string(&args, 0, "KV.decr", "key")?;
            let result = solikv_cmd(&["DECR", &key])?;
            Ok(Value::Int(result.as_i64().unwrap_or(0)))
        })),
    );

    kv_static_methods.insert(
        "incrby".to_string(),
        Rc::new(NativeFunction::new("KV.incrby", Some(2), |args| {
            let key = extract_string(&args, 0, "KV.incrby", "key")?;
            let amount = extract_int(&args, 1, "KV.incrby", "amount")?;
            let amount_str = amount.to_string();
            let result = solikv_cmd(&["INCRBY", &key, &amount_str])?;
            Ok(Value::Int(result.as_i64().unwrap_or(0)))
        })),
    );

    kv_static_methods.insert(
        "decrby".to_string(),
        Rc::new(NativeFunction::new("KV.decrby", Some(2), |args| {
            let key = extract_string(&args, 0, "KV.decrby", "key")?;
            let amount = extract_int(&args, 1, "KV.decrby", "amount")?;
            let amount_str = amount.to_string();
            let result = solikv_cmd(&["DECRBY", &key, &amount_str])?;
            Ok(Value::Int(result.as_i64().unwrap_or(0)))
        })),
    );

    // --- List operations ---

    kv_static_methods.insert(
        "lpush".to_string(),
        Rc::new(NativeFunction::new("KV.lpush", None, |args| {
            if args.len() < 2 {
                return Err(
                    "KV.lpush() expects at least 2 arguments (key, value, ...values)".to_string(),
                );
            }
            let key = extract_string(&args, 0, "KV.lpush", "key")?;
            let mut cmd_args: Vec<String> = vec!["LPUSH".to_string(), key];
            for v in &args[1..] {
                cmd_args.push(value_to_raw(v));
            }
            let refs: Vec<&str> = cmd_args.iter().map(|s| s.as_str()).collect();
            let result = solikv_cmd(&refs)?;
            Ok(Value::Int(result.as_i64().unwrap_or(0)))
        })),
    );

    kv_static_methods.insert(
        "rpush".to_string(),
        Rc::new(NativeFunction::new("KV.rpush", None, |args| {
            if args.len() < 2 {
                return Err(
                    "KV.rpush() expects at least 2 arguments (key, value, ...values)".to_string(),
                );
            }
            let key = extract_string(&args, 0, "KV.rpush", "key")?;
            let mut cmd_args: Vec<String> = vec!["RPUSH".to_string(), key];
            for v in &args[1..] {
                cmd_args.push(value_to_raw(v));
            }
            let refs: Vec<&str> = cmd_args.iter().map(|s| s.as_str()).collect();
            let result = solikv_cmd(&refs)?;
            Ok(Value::Int(result.as_i64().unwrap_or(0)))
        })),
    );

    kv_static_methods.insert(
        "lpop".to_string(),
        Rc::new(NativeFunction::new("KV.lpop", Some(1), |args| {
            let key = extract_string(&args, 0, "KV.lpop", "key")?;
            let result = solikv_cmd(&["LPOP", &key])?;
            solikv_result_to_value(&result)
        })),
    );

    kv_static_methods.insert(
        "rpop".to_string(),
        Rc::new(NativeFunction::new("KV.rpop", Some(1), |args| {
            let key = extract_string(&args, 0, "KV.rpop", "key")?;
            let result = solikv_cmd(&["RPOP", &key])?;
            solikv_result_to_value(&result)
        })),
    );

    kv_static_methods.insert(
        "lrange".to_string(),
        Rc::new(NativeFunction::new("KV.lrange", Some(3), |args| {
            let key = extract_string(&args, 0, "KV.lrange", "key")?;
            let start = extract_int(&args, 1, "KV.lrange", "start")?;
            let stop = extract_int(&args, 2, "KV.lrange", "stop")?;
            let start_str = start.to_string();
            let stop_str = stop.to_string();
            let result = solikv_cmd(&["LRANGE", &key, &start_str, &stop_str])?;
            solikv_result_to_value(&result)
        })),
    );

    kv_static_methods.insert(
        "llen".to_string(),
        Rc::new(NativeFunction::new("KV.llen", Some(1), |args| {
            let key = extract_string(&args, 0, "KV.llen", "key")?;
            let result = solikv_cmd(&["LLEN", &key])?;
            Ok(Value::Int(result.as_i64().unwrap_or(0)))
        })),
    );

    // --- Set operations ---

    kv_static_methods.insert(
        "sadd".to_string(),
        Rc::new(NativeFunction::new("KV.sadd", None, |args| {
            if args.len() < 2 {
                return Err(
                    "KV.sadd() expects at least 2 arguments (key, member, ...members)".to_string(),
                );
            }
            let key = extract_string(&args, 0, "KV.sadd", "key")?;
            let mut cmd_args: Vec<String> = vec!["SADD".to_string(), key];
            for v in &args[1..] {
                cmd_args.push(value_to_raw(v));
            }
            let refs: Vec<&str> = cmd_args.iter().map(|s| s.as_str()).collect();
            let result = solikv_cmd(&refs)?;
            Ok(Value::Int(result.as_i64().unwrap_or(0)))
        })),
    );

    kv_static_methods.insert(
        "srem".to_string(),
        Rc::new(NativeFunction::new("KV.srem", None, |args| {
            if args.len() < 2 {
                return Err(
                    "KV.srem() expects at least 2 arguments (key, member, ...members)".to_string(),
                );
            }
            let key = extract_string(&args, 0, "KV.srem", "key")?;
            let mut cmd_args: Vec<String> = vec!["SREM".to_string(), key];
            for v in &args[1..] {
                cmd_args.push(value_to_raw(v));
            }
            let refs: Vec<&str> = cmd_args.iter().map(|s| s.as_str()).collect();
            let result = solikv_cmd(&refs)?;
            Ok(Value::Int(result.as_i64().unwrap_or(0)))
        })),
    );

    kv_static_methods.insert(
        "smembers".to_string(),
        Rc::new(NativeFunction::new("KV.smembers", Some(1), |args| {
            let key = extract_string(&args, 0, "KV.smembers", "key")?;
            let result = solikv_cmd(&["SMEMBERS", &key])?;
            solikv_result_to_value(&result)
        })),
    );

    kv_static_methods.insert(
        "sismember".to_string(),
        Rc::new(NativeFunction::new("KV.sismember", Some(2), |args| {
            let key = extract_string(&args, 0, "KV.sismember", "key")?;
            let member = value_to_raw(&args[1]);
            let result = solikv_cmd(&["SISMEMBER", &key, &member])?;
            Ok(Value::Bool(result.as_i64().unwrap_or(0) > 0))
        })),
    );

    kv_static_methods.insert(
        "scard".to_string(),
        Rc::new(NativeFunction::new("KV.scard", Some(1), |args| {
            let key = extract_string(&args, 0, "KV.scard", "key")?;
            let result = solikv_cmd(&["SCARD", &key])?;
            Ok(Value::Int(result.as_i64().unwrap_or(0)))
        })),
    );

    // --- Hash operations ---

    kv_static_methods.insert(
        "hset".to_string(),
        Rc::new(NativeFunction::new("KV.hset", Some(3), |args| {
            let key = extract_string(&args, 0, "KV.hset", "key")?;
            let field = extract_string(&args, 1, "KV.hset", "field")?;
            let val = value_to_raw(&args[2]);
            let result = solikv_cmd(&["HSET", &key, &field, &val])?;
            Ok(Value::Int(result.as_i64().unwrap_or(0)))
        })),
    );

    kv_static_methods.insert(
        "hget".to_string(),
        Rc::new(NativeFunction::new("KV.hget", Some(2), |args| {
            let key = extract_string(&args, 0, "KV.hget", "key")?;
            let field = extract_string(&args, 1, "KV.hget", "field")?;
            let result = solikv_cmd(&["HGET", &key, &field])?;
            solikv_result_to_value(&result)
        })),
    );

    kv_static_methods.insert(
        "hdel".to_string(),
        Rc::new(NativeFunction::new("KV.hdel", None, |args| {
            if args.len() < 2 {
                return Err(
                    "KV.hdel() expects at least 2 arguments (key, field, ...fields)".to_string(),
                );
            }
            let key = extract_string(&args, 0, "KV.hdel", "key")?;
            let mut cmd_args: Vec<String> = vec!["HDEL".to_string(), key];
            for v in &args[1..] {
                match v {
                    Value::String(s) => cmd_args.push(s.clone()),
                    other => {
                        return Err(format!(
                            "KV.hdel() expects string fields, got {}",
                            other.type_name()
                        ))
                    }
                }
            }
            let refs: Vec<&str> = cmd_args.iter().map(|s| s.as_str()).collect();
            let result = solikv_cmd(&refs)?;
            Ok(Value::Int(result.as_i64().unwrap_or(0)))
        })),
    );

    kv_static_methods.insert(
        "hgetall".to_string(),
        Rc::new(NativeFunction::new("KV.hgetall", Some(1), |args| {
            let key = extract_string(&args, 0, "KV.hgetall", "key")?;
            let result = solikv_cmd(&["HGETALL", &key])?;

            // HGETALL returns alternating key/value pairs as an array,
            // or an object — handle both
            if let Some(obj) = result.as_object() {
                let mut hash = HashPairs::default();
                for (k, v) in obj {
                    hash.insert(HashKey::String(k.clone()), solikv_result_to_value(v)?);
                }
                return Ok(Value::Hash(Rc::new(RefCell::new(hash))));
            }

            if let Some(arr) = result.as_array() {
                let mut hash = HashPairs::default();
                let mut i = 0;
                while i + 1 < arr.len() {
                    if let Some(k) = arr[i].as_str() {
                        hash.insert(
                            HashKey::String(k.to_string()),
                            solikv_result_to_value(&arr[i + 1])?,
                        );
                    }
                    i += 2;
                }
                return Ok(Value::Hash(Rc::new(RefCell::new(hash))));
            }

            Ok(Value::Hash(Rc::new(RefCell::new(HashPairs::default()))))
        })),
    );

    kv_static_methods.insert(
        "hexists".to_string(),
        Rc::new(NativeFunction::new("KV.hexists", Some(2), |args| {
            let key = extract_string(&args, 0, "KV.hexists", "key")?;
            let field = extract_string(&args, 1, "KV.hexists", "field")?;
            let result = solikv_cmd(&["HEXISTS", &key, &field])?;
            Ok(Value::Bool(result.as_i64().unwrap_or(0) > 0))
        })),
    );

    kv_static_methods.insert(
        "hkeys".to_string(),
        Rc::new(NativeFunction::new("KV.hkeys", Some(1), |args| {
            let key = extract_string(&args, 0, "KV.hkeys", "key")?;
            let result = solikv_cmd(&["HKEYS", &key])?;
            solikv_result_to_value(&result)
        })),
    );

    kv_static_methods.insert(
        "hlen".to_string(),
        Rc::new(NativeFunction::new("KV.hlen", Some(1), |args| {
            let key = extract_string(&args, 0, "KV.hlen", "key")?;
            let result = solikv_cmd(&["HLEN", &key])?;
            Ok(Value::Int(result.as_i64().unwrap_or(0)))
        })),
    );

    // --- Server / utility ---

    kv_static_methods.insert(
        "ping".to_string(),
        Rc::new(NativeFunction::new("KV.ping", Some(0), |_args| {
            let result = solikv_cmd(&["PING"])?;
            match result.as_str() {
                Some(s) => Ok(Value::String(s.to_string())),
                None => Ok(Value::Bool(true)),
            }
        })),
    );

    kv_static_methods.insert(
        "dbsize".to_string(),
        Rc::new(NativeFunction::new("KV.dbsize", Some(0), |_args| {
            let result = solikv_cmd(&["DBSIZE"])?;
            Ok(Value::Int(result.as_i64().unwrap_or(0)))
        })),
    );

    kv_static_methods.insert(
        "flushdb".to_string(),
        Rc::new(NativeFunction::new("KV.flushdb", Some(0), |_args| {
            solikv_cmd(&["FLUSHDB"])?;
            Ok(Value::Null)
        })),
    );

    // KV.cmd(...args) — run any raw command
    kv_static_methods.insert(
        "cmd".to_string(),
        Rc::new(NativeFunction::new("KV.cmd", None, |args| {
            if args.is_empty() {
                return Err("KV.cmd() requires at least one argument".to_string());
            }
            let str_args: Vec<String> = args.iter().map(value_to_raw).collect();
            let refs: Vec<&str> = str_args.iter().map(|s| s.as_str()).collect();
            let result = solikv_cmd(&refs)?;
            solikv_result_to_value(&result)
        })),
    );

    // KV.configure(host, token?)
    kv_static_methods.insert(
        "configure".to_string(),
        Rc::new(NativeFunction::new("KV.configure", None, |args| {
            if args.is_empty() || args.len() > 2 {
                return Err("KV.configure() expects 1 or 2 arguments (host, token?)".to_string());
            }
            let host = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "KV.configure() expects string host, got {}",
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

    let kv_class_rc = Rc::new(Class {
        name: "KV".to_string(),
        superclass: None,
        methods: HashMap::new(),
        static_methods: HashMap::new(),
        native_static_methods: kv_static_methods,
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
    });

    env.define("KV".to_string(), Value::Class(kv_class_rc));
}
