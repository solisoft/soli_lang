use super::solikv::{solikv_cmd, solikv_configure, solikv_del, solikv_get, solikv_set};
use crate::interpreter::environment::Environment;
use crate::interpreter::value::{json_to_value, Class, HashKey, HashPairs, NativeFunction, Value};

/// Convert a Value to a raw string for Redis storage (no JSON encoding).
fn value_to_raw(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone().to_string(),
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
        Some(Value::String(s)) => Ok(s.clone().to_string()),
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

/// SEC-037: Commands that wipe state, enumerate the entire keyspace, mutate
/// server config, or run arbitrary Lua. Denied by default for `KV.cmd`,
/// `KV.flushdb`, and `KV.keys` — opt back in with `SOLI_KV_ALLOW_ADMIN=1`
/// (and only do so on a privately-deployed admin process, not on a worker
/// reachable from user traffic).
const KV_DENYLIST: &[&str] = &[
    // Wipes / mass-mutation
    "FLUSHALL",
    "FLUSHDB",
    // Bulk key enumeration (also O(N) blocking on Redis)
    "KEYS",
    "SCAN",
    // Server / cluster control
    "CONFIG",
    "DEBUG",
    "SHUTDOWN",
    "MONITOR",
    "CLIENT",
    "SLAVEOF",
    "REPLICAOF",
    "BGREWRITEAOF",
    "BGSAVE",
    "SAVE",
    "CLUSTER",
    "FAILOVER",
    "RESET",
    "ACL",
    // Arbitrary Lua / scripting surface
    "SCRIPT",
    "EVAL",
    "EVALSHA",
    "FUNCTION",
];

fn kv_admin_allowed() -> bool {
    matches!(
        std::env::var("SOLI_KV_ALLOW_ADMIN").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

fn check_kv_command(verb: &str) -> Result<(), String> {
    let upper = verb.to_uppercase();
    if KV_DENYLIST.iter().any(|c| *c == upper) && !kv_admin_allowed() {
        return Err(format!(
            "KV: '{}' is denylisted as a destructive or admin command. \
             Set SOLI_KV_ALLOW_ADMIN=1 to enable raw admin commands \
             (only on a trusted, non-user-facing process).",
            upper
        ));
    }
    Ok(())
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
        serde_json::Value::String(s) => Ok(Value::String(s.clone().into())),
        serde_json::Value::Array(arr) => {
            let vals: Result<Vec<Value>, String> = arr.iter().map(solikv_result_to_value).collect();
            Ok(Value::Array(Rc::new(RefCell::new(vals?))))
        }
        other => json_to_value(other.clone()),
    }
}

/// Convert a RESP reply into a Soli Float. Float-returning Redis commands
/// (ZSCORE, ZINCRBY, INCRBYFLOAT, HINCRBYFLOAT) reply with a bulk string like
/// "1.5", so parse strings as well as JSON numbers. A null reply maps to nil.
fn solikv_result_to_float(result: &serde_json::Value) -> Value {
    match result {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Number(n) => n.as_f64().map(Value::Float).unwrap_or(Value::Null),
        serde_json::Value::String(s) => s.parse::<f64>().map(Value::Float).unwrap_or(Value::Null),
        _ => Value::Null,
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
                Some(s) => Ok(Value::String(s.into())),
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
                .unwrap_or_else(|| "*".into());

            // SEC-037: KEYS is O(N) and exposes the entire keyspace —
            // denylisted unless SOLI_KV_ALLOW_ADMIN=1.
            check_kv_command("KEYS")?;
            let result = solikv_cmd(&["KEYS", &pattern])?;
            let keys: Vec<Value> = result
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| Value::String(s.to_string().into())))
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
                Some(s) => Ok(Value::String(s.to_string().into())),
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

    kv_static_methods.insert(
        "incrbyfloat".to_string(),
        Rc::new(NativeFunction::new("KV.incrbyfloat", Some(2), |args| {
            let key = extract_string(&args, 0, "KV.incrbyfloat", "key")?;
            let amount = match args.get(1) {
                Some(Value::Float(f)) => *f,
                Some(Value::Int(i)) => *i as f64,
                other => {
                    return Err(format!(
                        "KV.incrbyfloat() expects number amount, got {}",
                        other
                            .map(|v| v.type_name())
                            .unwrap_or_else(|| "nil".to_string())
                    ))
                }
            };
            let amount_str = amount.to_string();
            let result = solikv_cmd(&["INCRBYFLOAT", &key, &amount_str])?;
            Ok(solikv_result_to_float(&result))
        })),
    );

    // --- String extras ---

    // KV.setnx(key, value) -> Bool (true if the key was set, false if it existed)
    kv_static_methods.insert(
        "setnx".to_string(),
        Rc::new(NativeFunction::new("KV.setnx", Some(2), |args| {
            let key = extract_string(&args, 0, "KV.setnx", "key")?;
            let value = value_to_raw(&args[1]);
            let result = solikv_cmd(&["SETNX", &key, &value])?;
            Ok(Value::Bool(result.as_i64().unwrap_or(0) > 0))
        })),
    );

    // KV.getset(key, value) -> previous value (or nil)
    kv_static_methods.insert(
        "getset".to_string(),
        Rc::new(NativeFunction::new("KV.getset", Some(2), |args| {
            let key = extract_string(&args, 0, "KV.getset", "key")?;
            let value = value_to_raw(&args[1]);
            let result = solikv_cmd(&["GETSET", &key, &value])?;
            solikv_result_to_value(&result)
        })),
    );

    // KV.getdel(key) -> value (or nil); deletes the key after reading
    kv_static_methods.insert(
        "getdel".to_string(),
        Rc::new(NativeFunction::new("KV.getdel", Some(1), |args| {
            let key = extract_string(&args, 0, "KV.getdel", "key")?;
            let result = solikv_cmd(&["GETDEL", &key])?;
            solikv_result_to_value(&result)
        })),
    );

    // KV.append(key, value) -> Int (new string length)
    kv_static_methods.insert(
        "append".to_string(),
        Rc::new(NativeFunction::new("KV.append", Some(2), |args| {
            let key = extract_string(&args, 0, "KV.append", "key")?;
            let value = value_to_raw(&args[1]);
            let result = solikv_cmd(&["APPEND", &key, &value])?;
            Ok(Value::Int(result.as_i64().unwrap_or(0)))
        })),
    );

    // KV.strlen(key) -> Int
    kv_static_methods.insert(
        "strlen".to_string(),
        Rc::new(NativeFunction::new("KV.strlen", Some(1), |args| {
            let key = extract_string(&args, 0, "KV.strlen", "key")?;
            let result = solikv_cmd(&["STRLEN", &key])?;
            Ok(Value::Int(result.as_i64().unwrap_or(0)))
        })),
    );

    // KV.mget(...keys) -> Array (nil for missing keys)
    kv_static_methods.insert(
        "mget".to_string(),
        Rc::new(NativeFunction::new("KV.mget", None, |args| {
            if args.is_empty() {
                return Err("KV.mget() expects at least 1 argument (key, ...keys)".to_string());
            }
            let mut cmd_args: Vec<String> = vec!["MGET".to_string()];
            for (idx, _) in args.iter().enumerate() {
                cmd_args.push(extract_string(&args, idx, "KV.mget", "key")?);
            }
            let refs: Vec<&str> = cmd_args.iter().map(|s| s.as_str()).collect();
            let result = solikv_cmd(&refs)?;
            solikv_result_to_value(&result)
        })),
    );

    // KV.mset(key, value, ...) -> nil (set many key/value pairs atomically)
    kv_static_methods.insert(
        "mset".to_string(),
        Rc::new(NativeFunction::new("KV.mset", None, |args| {
            if args.len() < 2 || args.len() % 2 != 0 {
                return Err(
                    "KV.mset() expects an even number of arguments (key, value, ...)".to_string(),
                );
            }
            let mut cmd_args: Vec<String> = vec!["MSET".to_string()];
            for v in &args {
                cmd_args.push(value_to_raw(v));
            }
            let refs: Vec<&str> = cmd_args.iter().map(|s| s.as_str()).collect();
            solikv_cmd(&refs)?;
            Ok(Value::Null)
        })),
    );

    // --- Expiry / generic extras ---

    // KV.pexpire(key, milliseconds) -> Bool
    kv_static_methods.insert(
        "pexpire".to_string(),
        Rc::new(NativeFunction::new("KV.pexpire", Some(2), |args| {
            let key = extract_string(&args, 0, "KV.pexpire", "key")?;
            let ms = extract_int(&args, 1, "KV.pexpire", "milliseconds")?;
            let ms_str = ms.to_string();
            let result = solikv_cmd(&["PEXPIRE", &key, &ms_str])?;
            Ok(Value::Bool(result.as_i64().unwrap_or(0) > 0))
        })),
    );

    // KV.pttl(key) -> Int milliseconds (or nil if no expiry / missing)
    kv_static_methods.insert(
        "pttl".to_string(),
        Rc::new(NativeFunction::new("KV.pttl", Some(1), |args| {
            let key = extract_string(&args, 0, "KV.pttl", "key")?;
            let result = solikv_cmd(&["PTTL", &key])?;
            match result.as_i64() {
                Some(ms) if ms >= 0 => Ok(Value::Int(ms)),
                _ => Ok(Value::Null),
            }
        })),
    );

    // KV.expireat(key, unix_timestamp) -> Bool
    kv_static_methods.insert(
        "expireat".to_string(),
        Rc::new(NativeFunction::new("KV.expireat", Some(2), |args| {
            let key = extract_string(&args, 0, "KV.expireat", "key")?;
            let ts = extract_int(&args, 1, "KV.expireat", "timestamp")?;
            let ts_str = ts.to_string();
            let result = solikv_cmd(&["EXPIREAT", &key, &ts_str])?;
            Ok(Value::Bool(result.as_i64().unwrap_or(0) > 0))
        })),
    );

    // KV.touch(...keys) -> Int (number of existing keys touched)
    kv_static_methods.insert(
        "touch".to_string(),
        Rc::new(NativeFunction::new("KV.touch", None, |args| {
            if args.is_empty() {
                return Err("KV.touch() expects at least 1 argument (key, ...keys)".to_string());
            }
            let mut cmd_args: Vec<String> = vec!["TOUCH".to_string()];
            for (idx, _) in args.iter().enumerate() {
                cmd_args.push(extract_string(&args, idx, "KV.touch", "key")?);
            }
            let refs: Vec<&str> = cmd_args.iter().map(|s| s.as_str()).collect();
            let result = solikv_cmd(&refs)?;
            Ok(Value::Int(result.as_i64().unwrap_or(0)))
        })),
    );

    // KV.unlink(...keys) -> Int (non-blocking delete)
    kv_static_methods.insert(
        "unlink".to_string(),
        Rc::new(NativeFunction::new("KV.unlink", None, |args| {
            if args.is_empty() {
                return Err("KV.unlink() expects at least 1 argument (key, ...keys)".to_string());
            }
            let mut cmd_args: Vec<String> = vec!["UNLINK".to_string()];
            for (idx, _) in args.iter().enumerate() {
                cmd_args.push(extract_string(&args, idx, "KV.unlink", "key")?);
            }
            let refs: Vec<&str> = cmd_args.iter().map(|s| s.as_str()).collect();
            let result = solikv_cmd(&refs)?;
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

    // KV.lindex(key, index) -> element at index (or nil); negative index counts from tail
    kv_static_methods.insert(
        "lindex".to_string(),
        Rc::new(NativeFunction::new("KV.lindex", Some(2), |args| {
            let key = extract_string(&args, 0, "KV.lindex", "key")?;
            let index = extract_int(&args, 1, "KV.lindex", "index")?;
            let index_str = index.to_string();
            let result = solikv_cmd(&["LINDEX", &key, &index_str])?;
            solikv_result_to_value(&result)
        })),
    );

    // KV.lset(key, index, value) -> nil
    kv_static_methods.insert(
        "lset".to_string(),
        Rc::new(NativeFunction::new("KV.lset", Some(3), |args| {
            let key = extract_string(&args, 0, "KV.lset", "key")?;
            let index = extract_int(&args, 1, "KV.lset", "index")?;
            let index_str = index.to_string();
            let value = value_to_raw(&args[2]);
            solikv_cmd(&["LSET", &key, &index_str, &value])?;
            Ok(Value::Null)
        })),
    );

    // KV.lrem(key, count, value) -> Int (number of elements removed)
    kv_static_methods.insert(
        "lrem".to_string(),
        Rc::new(NativeFunction::new("KV.lrem", Some(3), |args| {
            let key = extract_string(&args, 0, "KV.lrem", "key")?;
            let count = extract_int(&args, 1, "KV.lrem", "count")?;
            let count_str = count.to_string();
            let value = value_to_raw(&args[2]);
            let result = solikv_cmd(&["LREM", &key, &count_str, &value])?;
            Ok(Value::Int(result.as_i64().unwrap_or(0)))
        })),
    );

    // KV.ltrim(key, start, stop) -> nil (keep only the given range)
    kv_static_methods.insert(
        "ltrim".to_string(),
        Rc::new(NativeFunction::new("KV.ltrim", Some(3), |args| {
            let key = extract_string(&args, 0, "KV.ltrim", "key")?;
            let start = extract_int(&args, 1, "KV.ltrim", "start")?;
            let stop = extract_int(&args, 2, "KV.ltrim", "stop")?;
            let start_str = start.to_string();
            let stop_str = stop.to_string();
            solikv_cmd(&["LTRIM", &key, &start_str, &stop_str])?;
            Ok(Value::Null)
        })),
    );

    // KV.rpoplpush(source, dest) -> moved element (or nil)
    kv_static_methods.insert(
        "rpoplpush".to_string(),
        Rc::new(NativeFunction::new("KV.rpoplpush", Some(2), |args| {
            let source = extract_string(&args, 0, "KV.rpoplpush", "source")?;
            let dest = extract_string(&args, 1, "KV.rpoplpush", "dest")?;
            let result = solikv_cmd(&["RPOPLPUSH", &source, &dest])?;
            solikv_result_to_value(&result)
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

    // KV.spop(key, count?) -> a random member removed (or an array if count given)
    kv_static_methods.insert(
        "spop".to_string(),
        Rc::new(NativeFunction::new("KV.spop", None, |args| {
            if args.is_empty() || args.len() > 2 {
                return Err("KV.spop() expects 1 or 2 arguments (key, count?)".to_string());
            }
            let key = extract_string(&args, 0, "KV.spop", "key")?;
            let result = match args.get(1) {
                Some(_) => {
                    let count = extract_int(&args, 1, "KV.spop", "count")?;
                    let count_str = count.to_string();
                    solikv_cmd(&["SPOP", &key, &count_str])?
                }
                None => solikv_cmd(&["SPOP", &key])?,
            };
            solikv_result_to_value(&result)
        })),
    );

    // KV.srandmember(key, count?) -> a random member (or an array if count given); non-destructive
    kv_static_methods.insert(
        "srandmember".to_string(),
        Rc::new(NativeFunction::new("KV.srandmember", None, |args| {
            if args.is_empty() || args.len() > 2 {
                return Err("KV.srandmember() expects 1 or 2 arguments (key, count?)".to_string());
            }
            let key = extract_string(&args, 0, "KV.srandmember", "key")?;
            let result = match args.get(1) {
                Some(_) => {
                    let count = extract_int(&args, 1, "KV.srandmember", "count")?;
                    let count_str = count.to_string();
                    solikv_cmd(&["SRANDMEMBER", &key, &count_str])?
                }
                None => solikv_cmd(&["SRANDMEMBER", &key])?,
            };
            solikv_result_to_value(&result)
        })),
    );

    // KV.sinter(...keys) / KV.sunion(...keys) / KV.sdiff(...keys) -> Array
    for (method, verb) in [
        ("sinter", "SINTER"),
        ("sunion", "SUNION"),
        ("sdiff", "SDIFF"),
    ] {
        kv_static_methods.insert(
            method.to_string(),
            Rc::new(NativeFunction::new(
                format!("KV.{}", method),
                None,
                move |args| {
                    if args.is_empty() {
                        return Err(format!(
                            "KV.{}() expects at least 1 argument (key, ...keys)",
                            method
                        ));
                    }
                    let fn_name = format!("KV.{}", method);
                    let mut cmd_args: Vec<String> = vec![verb.to_string()];
                    for (idx, _) in args.iter().enumerate() {
                        cmd_args.push(extract_string(&args, idx, &fn_name, "key")?);
                    }
                    let refs: Vec<&str> = cmd_args.iter().map(|s| s.as_str()).collect();
                    let result = solikv_cmd(&refs)?;
                    solikv_result_to_value(&result)
                },
            )),
        );
    }

    // KV.smismember(key, ...members) -> Array of Bool (membership of each)
    kv_static_methods.insert(
        "smismember".to_string(),
        Rc::new(NativeFunction::new("KV.smismember", None, |args| {
            if args.len() < 2 {
                return Err(
                    "KV.smismember() expects at least 2 arguments (key, member, ...members)"
                        .to_string(),
                );
            }
            let key = extract_string(&args, 0, "KV.smismember", "key")?;
            let mut cmd_args: Vec<String> = vec!["SMISMEMBER".to_string(), key];
            for v in &args[1..] {
                cmd_args.push(value_to_raw(v));
            }
            let refs: Vec<&str> = cmd_args.iter().map(|s| s.as_str()).collect();
            let result = solikv_cmd(&refs)?;
            // Server replies with an array of 0/1; surface as Bools.
            let bools: Vec<Value> = result
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .map(|v| Value::Bool(v.as_i64().unwrap_or(0) > 0))
                        .collect()
                })
                .unwrap_or_default();
            Ok(Value::Array(Rc::new(RefCell::new(bools))))
        })),
    );

    // KV.smove(source, dest, member) -> Bool
    kv_static_methods.insert(
        "smove".to_string(),
        Rc::new(NativeFunction::new("KV.smove", Some(3), |args| {
            let source = extract_string(&args, 0, "KV.smove", "source")?;
            let dest = extract_string(&args, 1, "KV.smove", "dest")?;
            let member = value_to_raw(&args[2]);
            let result = solikv_cmd(&["SMOVE", &source, &dest, &member])?;
            Ok(Value::Bool(result.as_i64().unwrap_or(0) > 0))
        })),
    );

    // --- HyperLogLog operations ---
    // Probabilistic cardinality estimation: count distinct items using a
    // fixed ~12 KB sketch instead of storing every member. Standard error
    // ~0.81% (Redis-compatible p=14). Proxies PFADD / PFCOUNT / PFMERGE.

    // KV.pfadd(key, ...elements) -> Int (1 if the HLL was modified, else 0)
    kv_static_methods.insert(
        "pfadd".to_string(),
        Rc::new(NativeFunction::new("KV.pfadd", None, |args| {
            if args.is_empty() {
                return Err("KV.pfadd() expects at least 1 argument (key, ...elements)".to_string());
            }
            let key = extract_string(&args, 0, "KV.pfadd", "key")?;
            let mut cmd_args: Vec<String> = vec!["PFADD".to_string(), key];
            for v in &args[1..] {
                cmd_args.push(value_to_raw(v));
            }
            let refs: Vec<&str> = cmd_args.iter().map(|s| s.as_str()).collect();
            let result = solikv_cmd(&refs)?;
            Ok(Value::Int(result.as_i64().unwrap_or(0)))
        })),
    );

    // KV.pfcount(key, ...keys) -> Int (estimated cardinality; union if multiple keys)
    kv_static_methods.insert(
        "pfcount".to_string(),
        Rc::new(NativeFunction::new("KV.pfcount", None, |args| {
            if args.is_empty() {
                return Err("KV.pfcount() expects at least 1 argument (key, ...keys)".to_string());
            }
            let mut cmd_args: Vec<String> = vec!["PFCOUNT".to_string()];
            for (idx, _) in args.iter().enumerate() {
                cmd_args.push(extract_string(&args, idx, "KV.pfcount", "key")?);
            }
            let refs: Vec<&str> = cmd_args.iter().map(|s| s.as_str()).collect();
            let result = solikv_cmd(&refs)?;
            Ok(Value::Int(result.as_i64().unwrap_or(0)))
        })),
    );

    // KV.pfmerge(destkey, ...sourcekeys) -> nil (merges source HLLs into dest)
    kv_static_methods.insert(
        "pfmerge".to_string(),
        Rc::new(NativeFunction::new("KV.pfmerge", None, |args| {
            if args.is_empty() {
                return Err(
                    "KV.pfmerge() expects at least 1 argument (destkey, ...sourcekeys)".to_string(),
                );
            }
            let mut cmd_args: Vec<String> = vec!["PFMERGE".to_string()];
            for (idx, _) in args.iter().enumerate() {
                cmd_args.push(extract_string(&args, idx, "KV.pfmerge", "key")?);
            }
            let refs: Vec<&str> = cmd_args.iter().map(|s| s.as_str()).collect();
            solikv_cmd(&refs)?;
            Ok(Value::Null)
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
                    Value::String(s) => cmd_args.push(s.clone().to_string()),
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
                    hash.insert(
                        HashKey::String(k.clone().into()),
                        solikv_result_to_value(v)?,
                    );
                }
                return Ok(Value::Hash(Rc::new(RefCell::new(hash))));
            }

            if let Some(arr) = result.as_array() {
                let mut hash = HashPairs::default();
                let mut i = 0;
                while i + 1 < arr.len() {
                    if let Some(k) = arr[i].as_str() {
                        hash.insert(
                            HashKey::String(k.to_string().into()),
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

    // KV.hsetnx(key, field, value) -> Bool (true only if the field was created)
    kv_static_methods.insert(
        "hsetnx".to_string(),
        Rc::new(NativeFunction::new("KV.hsetnx", Some(3), |args| {
            let key = extract_string(&args, 0, "KV.hsetnx", "key")?;
            let field = extract_string(&args, 1, "KV.hsetnx", "field")?;
            let value = value_to_raw(&args[2]);
            let result = solikv_cmd(&["HSETNX", &key, &field, &value])?;
            Ok(Value::Bool(result.as_i64().unwrap_or(0) > 0))
        })),
    );

    // KV.hincrby(key, field, amount) -> Int (new value)
    kv_static_methods.insert(
        "hincrby".to_string(),
        Rc::new(NativeFunction::new("KV.hincrby", Some(3), |args| {
            let key = extract_string(&args, 0, "KV.hincrby", "key")?;
            let field = extract_string(&args, 1, "KV.hincrby", "field")?;
            let amount = extract_int(&args, 2, "KV.hincrby", "amount")?;
            let amount_str = amount.to_string();
            let result = solikv_cmd(&["HINCRBY", &key, &field, &amount_str])?;
            Ok(Value::Int(result.as_i64().unwrap_or(0)))
        })),
    );

    // KV.hincrbyfloat(key, field, amount) -> Float (new value)
    kv_static_methods.insert(
        "hincrbyfloat".to_string(),
        Rc::new(NativeFunction::new("KV.hincrbyfloat", Some(3), |args| {
            let key = extract_string(&args, 0, "KV.hincrbyfloat", "key")?;
            let field = extract_string(&args, 1, "KV.hincrbyfloat", "field")?;
            let amount = match args.get(2) {
                Some(Value::Float(f)) => *f,
                Some(Value::Int(i)) => *i as f64,
                other => {
                    return Err(format!(
                        "KV.hincrbyfloat() expects number amount, got {}",
                        other
                            .map(|v| v.type_name())
                            .unwrap_or_else(|| "nil".to_string())
                    ))
                }
            };
            let amount_str = amount.to_string();
            let result = solikv_cmd(&["HINCRBYFLOAT", &key, &field, &amount_str])?;
            Ok(solikv_result_to_float(&result))
        })),
    );

    // KV.hmget(key, ...fields) -> Array (nil for missing fields)
    kv_static_methods.insert(
        "hmget".to_string(),
        Rc::new(NativeFunction::new("KV.hmget", None, |args| {
            if args.len() < 2 {
                return Err(
                    "KV.hmget() expects at least 2 arguments (key, field, ...fields)".to_string(),
                );
            }
            let key = extract_string(&args, 0, "KV.hmget", "key")?;
            let mut cmd_args: Vec<String> = vec!["HMGET".to_string(), key];
            for (idx, _) in args.iter().enumerate().skip(1) {
                cmd_args.push(extract_string(&args, idx, "KV.hmget", "field")?);
            }
            let refs: Vec<&str> = cmd_args.iter().map(|s| s.as_str()).collect();
            let result = solikv_cmd(&refs)?;
            solikv_result_to_value(&result)
        })),
    );

    // KV.hvals(key) -> Array of values
    kv_static_methods.insert(
        "hvals".to_string(),
        Rc::new(NativeFunction::new("KV.hvals", Some(1), |args| {
            let key = extract_string(&args, 0, "KV.hvals", "key")?;
            let result = solikv_cmd(&["HVALS", &key])?;
            solikv_result_to_value(&result)
        })),
    );

    // --- Sorted set (ZSET) operations ---

    // KV.zadd(key, score, member, ...) -> Int (number of new members added)
    kv_static_methods.insert(
        "zadd".to_string(),
        Rc::new(NativeFunction::new("KV.zadd", None, |args| {
            if args.len() < 3 || (args.len() - 1) % 2 != 0 {
                return Err(
                    "KV.zadd() expects a key followed by score/member pairs (key, score, member, ...)"
                        .to_string(),
                );
            }
            let key = extract_string(&args, 0, "KV.zadd", "key")?;
            let mut cmd_args: Vec<String> = vec!["ZADD".to_string(), key];
            for v in &args[1..] {
                cmd_args.push(value_to_raw(v));
            }
            let refs: Vec<&str> = cmd_args.iter().map(|s| s.as_str()).collect();
            let result = solikv_cmd(&refs)?;
            Ok(Value::Int(result.as_i64().unwrap_or(0)))
        })),
    );

    // KV.zrem(key, ...members) -> Int (number removed)
    kv_static_methods.insert(
        "zrem".to_string(),
        Rc::new(NativeFunction::new("KV.zrem", None, |args| {
            if args.len() < 2 {
                return Err(
                    "KV.zrem() expects at least 2 arguments (key, member, ...members)".to_string(),
                );
            }
            let key = extract_string(&args, 0, "KV.zrem", "key")?;
            let mut cmd_args: Vec<String> = vec!["ZREM".to_string(), key];
            for v in &args[1..] {
                cmd_args.push(value_to_raw(v));
            }
            let refs: Vec<&str> = cmd_args.iter().map(|s| s.as_str()).collect();
            let result = solikv_cmd(&refs)?;
            Ok(Value::Int(result.as_i64().unwrap_or(0)))
        })),
    );

    // KV.zscore(key, member) -> Float (or nil)
    kv_static_methods.insert(
        "zscore".to_string(),
        Rc::new(NativeFunction::new("KV.zscore", Some(2), |args| {
            let key = extract_string(&args, 0, "KV.zscore", "key")?;
            let member = value_to_raw(&args[1]);
            let result = solikv_cmd(&["ZSCORE", &key, &member])?;
            Ok(solikv_result_to_float(&result))
        })),
    );

    // KV.zincrby(key, amount, member) -> Float (new score)
    kv_static_methods.insert(
        "zincrby".to_string(),
        Rc::new(NativeFunction::new("KV.zincrby", Some(3), |args| {
            let key = extract_string(&args, 0, "KV.zincrby", "key")?;
            let amount = match args.get(1) {
                Some(Value::Float(f)) => *f,
                Some(Value::Int(i)) => *i as f64,
                other => {
                    return Err(format!(
                        "KV.zincrby() expects number amount, got {}",
                        other
                            .map(|v| v.type_name())
                            .unwrap_or_else(|| "nil".to_string())
                    ))
                }
            };
            let amount_str = amount.to_string();
            let member = value_to_raw(&args[2]);
            let result = solikv_cmd(&["ZINCRBY", &key, &amount_str, &member])?;
            Ok(solikv_result_to_float(&result))
        })),
    );

    // KV.zrank(key, member) / KV.zrevrank(key, member) -> Int rank (or nil)
    for (method, verb) in [("zrank", "ZRANK"), ("zrevrank", "ZREVRANK")] {
        kv_static_methods.insert(
            method.to_string(),
            Rc::new(NativeFunction::new(
                format!("KV.{}", method),
                Some(2),
                move |args| {
                    let fn_name = format!("KV.{}", method);
                    let key = extract_string(&args, 0, &fn_name, "key")?;
                    let member = value_to_raw(&args[1]);
                    let result = solikv_cmd(&[verb, &key, &member])?;
                    match result.as_i64() {
                        Some(rank) => Ok(Value::Int(rank)),
                        None => Ok(Value::Null),
                    }
                },
            )),
        );
    }

    // KV.zcard(key) -> Int
    kv_static_methods.insert(
        "zcard".to_string(),
        Rc::new(NativeFunction::new("KV.zcard", Some(1), |args| {
            let key = extract_string(&args, 0, "KV.zcard", "key")?;
            let result = solikv_cmd(&["ZCARD", &key])?;
            Ok(Value::Int(result.as_i64().unwrap_or(0)))
        })),
    );

    // KV.zcount(key, min, max) -> Int (members with score in [min, max])
    kv_static_methods.insert(
        "zcount".to_string(),
        Rc::new(NativeFunction::new("KV.zcount", Some(3), |args| {
            let key = extract_string(&args, 0, "KV.zcount", "key")?;
            let min = value_to_raw(&args[1]);
            let max = value_to_raw(&args[2]);
            let result = solikv_cmd(&["ZCOUNT", &key, &min, &max])?;
            Ok(Value::Int(result.as_i64().unwrap_or(0)))
        })),
    );

    // KV.zrange(key, start, stop, with_scores?) / KV.zrevrange(...) -> Array
    for (method, verb) in [("zrange", "ZRANGE"), ("zrevrange", "ZREVRANGE")] {
        kv_static_methods.insert(
            method.to_string(),
            Rc::new(NativeFunction::new(
                format!("KV.{}", method),
                None,
                move |args| {
                    if args.len() < 3 || args.len() > 4 {
                        return Err(format!(
                            "KV.{}() expects 3 or 4 arguments (key, start, stop, with_scores?)",
                            method
                        ));
                    }
                    let fn_name = format!("KV.{}", method);
                    let key = extract_string(&args, 0, &fn_name, "key")?;
                    let start = extract_int(&args, 1, &fn_name, "start")?.to_string();
                    let stop = extract_int(&args, 2, &fn_name, "stop")?.to_string();
                    let with_scores = matches!(args.get(3), Some(v) if v.is_truthy());
                    let mut cmd_args = vec![verb, &key, &start, &stop];
                    if with_scores {
                        cmd_args.push("WITHSCORES");
                    }
                    let result = solikv_cmd(&cmd_args)?;
                    solikv_result_to_value(&result)
                },
            )),
        );
    }

    // KV.zrangebyscore(key, min, max) -> Array (members with score in [min, max])
    kv_static_methods.insert(
        "zrangebyscore".to_string(),
        Rc::new(NativeFunction::new("KV.zrangebyscore", Some(3), |args| {
            let key = extract_string(&args, 0, "KV.zrangebyscore", "key")?;
            let min = value_to_raw(&args[1]);
            let max = value_to_raw(&args[2]);
            let result = solikv_cmd(&["ZRANGEBYSCORE", &key, &min, &max])?;
            solikv_result_to_value(&result)
        })),
    );

    // --- Bitmap operations ---

    // KV.setbit(key, offset, value) -> Int (the previous bit value)
    kv_static_methods.insert(
        "setbit".to_string(),
        Rc::new(NativeFunction::new("KV.setbit", Some(3), |args| {
            let key = extract_string(&args, 0, "KV.setbit", "key")?;
            let offset = extract_int(&args, 1, "KV.setbit", "offset")?.to_string();
            let bit = extract_int(&args, 2, "KV.setbit", "value")?;
            if bit != 0 && bit != 1 {
                return Err("KV.setbit() value must be 0 or 1".to_string());
            }
            let bit_str = bit.to_string();
            let result = solikv_cmd(&["SETBIT", &key, &offset, &bit_str])?;
            Ok(Value::Int(result.as_i64().unwrap_or(0)))
        })),
    );

    // KV.getbit(key, offset) -> Int (0 or 1)
    kv_static_methods.insert(
        "getbit".to_string(),
        Rc::new(NativeFunction::new("KV.getbit", Some(2), |args| {
            let key = extract_string(&args, 0, "KV.getbit", "key")?;
            let offset = extract_int(&args, 1, "KV.getbit", "offset")?.to_string();
            let result = solikv_cmd(&["GETBIT", &key, &offset])?;
            Ok(Value::Int(result.as_i64().unwrap_or(0)))
        })),
    );

    // KV.bitcount(key) -> Int (number of set bits)
    kv_static_methods.insert(
        "bitcount".to_string(),
        Rc::new(NativeFunction::new("KV.bitcount", Some(1), |args| {
            let key = extract_string(&args, 0, "KV.bitcount", "key")?;
            let result = solikv_cmd(&["BITCOUNT", &key])?;
            Ok(Value::Int(result.as_i64().unwrap_or(0)))
        })),
    );

    // --- Server / utility ---

    kv_static_methods.insert(
        "ping".to_string(),
        Rc::new(NativeFunction::new("KV.ping", Some(0), |_args| {
            let result = solikv_cmd(&["PING"])?;
            match result.as_str() {
                Some(s) => Ok(Value::String(s.to_string().into())),
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
            // SEC-037: wipes the entire database — denylisted unless
            // SOLI_KV_ALLOW_ADMIN=1.
            check_kv_command("FLUSHDB")?;
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
            // SEC-037: filter the verb against the destructive/admin
            // denylist before issuing the raw command. The check is on the
            // first positional argument (the command verb).
            check_kv_command(&str_args[0])?;
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
            solikv_configure(&host, token.map(|s| s.to_string()));
            Ok(Value::Null)
        })),
    );

    let kv_class_rc = Rc::new(Class {
        name: "KV".to_string(),
        superclass: None,
        methods: Rc::new(RefCell::new(HashMap::new())),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// SOLI_KV_ALLOW_ADMIN is read from process env, so the env-mutating
    /// tests must not interleave. Cargo runs tests in this module in
    /// parallel by default — serialize them through this mutex.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_admin_env<F: FnOnce()>(value: Option<&str>, body: F) {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let prev = std::env::var("SOLI_KV_ALLOW_ADMIN").ok();
        // SAFETY: ENV_LOCK serializes all SOLI_KV_ALLOW_ADMIN access in
        // this module, so no other thread is reading the variable while
        // we mutate it.
        unsafe {
            match value {
                Some(v) => std::env::set_var("SOLI_KV_ALLOW_ADMIN", v),
                None => std::env::remove_var("SOLI_KV_ALLOW_ADMIN"),
            }
        }
        body();
        unsafe {
            match prev {
                Some(v) => std::env::set_var("SOLI_KV_ALLOW_ADMIN", v),
                None => std::env::remove_var("SOLI_KV_ALLOW_ADMIN"),
            }
        }
    }

    #[test]
    fn denylist_blocks_destructive_verbs_by_default() {
        with_admin_env(None, || {
            for verb in ["FLUSHALL", "FLUSHDB", "KEYS", "CONFIG", "DEBUG", "EVAL"] {
                let err = check_kv_command(verb).unwrap_err();
                assert!(
                    err.contains(verb) && err.contains("SOLI_KV_ALLOW_ADMIN"),
                    "expected denylist error for {}, got: {}",
                    verb,
                    err
                );
            }
        });
    }

    #[test]
    fn denylist_is_case_insensitive() {
        with_admin_env(None, || {
            assert!(check_kv_command("flushdb").is_err());
            assert!(check_kv_command("FlushAll").is_err());
            assert!(check_kv_command("keys").is_err());
        });
    }

    #[test]
    fn allows_normal_commands() {
        with_admin_env(None, || {
            for verb in [
                "GET", "SET", "INCR", "HGETALL", "LRANGE", "EXPIRE", "PFADD", "PFCOUNT", "PFMERGE",
            ] {
                check_kv_command(verb)
                    .unwrap_or_else(|e| panic!("expected {} to be allowed, got: {}", verb, e));
            }
        });
    }

    #[test]
    fn admin_env_unlocks_denylist() {
        with_admin_env(Some("1"), || {
            check_kv_command("FLUSHDB").unwrap();
            check_kv_command("KEYS").unwrap();
            check_kv_command("CONFIG").unwrap();
        });
        with_admin_env(Some("true"), || {
            check_kv_command("EVAL").unwrap();
        });
        with_admin_env(Some("yes"), || {
            check_kv_command("DEBUG").unwrap();
        });
        with_admin_env(Some("0"), || {
            assert!(check_kv_command("FLUSHDB").is_err());
        });
    }
}
