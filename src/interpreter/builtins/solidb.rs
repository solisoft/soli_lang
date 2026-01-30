//! SoliDB client built-in functions.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::RwLock;

use indexmap::IndexMap;

use crate::solidb_http::SoliDBClient;
use base64::{engine::general_purpose::STANDARD, Engine as _};

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{HashKey, Instance, NativeFunction, Value};

// Execute DB operation synchronously and return result directly
fn exec_db_sync<F>(f: F) -> Value
where
    F: FnOnce() -> Result<String, String>,
{
    match f() {
        Ok(json_str) => {
            // Parse JSON and convert to Value
            match serde_json::from_str::<serde_json::Value>(&json_str) {
                Ok(json) => json_to_value(&json),
                Err(_) => Value::String(json_str),
            }
        }
        Err(e) => Value::String(format!("Error: {}", e)),
    }
}

/// Execute DB operation that returns serde_json::Value directly.
/// This skips the double JSON conversion (Value -> String -> Value).
fn exec_db_json<F>(f: F) -> Value
where
    F: FnOnce() -> Result<serde_json::Value, String>,
{
    match f() {
        Ok(json) => json_to_value(&json),
        Err(e) => Value::String(format!("Error: {}", e)),
    }
}

fn json_to_value(json: &serde_json::Value) -> Value {
    match json {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else if let Some(f) = n.as_f64() {
                Value::Float(f)
            } else {
                Value::String(n.to_string())
            }
        }
        serde_json::Value::String(s) => Value::String(s.clone()),
        serde_json::Value::Array(arr) => {
            let values: Vec<Value> = arr.iter().map(json_to_value).collect();
            Value::Array(Rc::new(RefCell::new(values)))
        }
        serde_json::Value::Object(obj) => {
            let pairs: IndexMap<HashKey, Value> = obj
                .iter()
                .map(|(k, v)| (HashKey::String(k.clone()), json_to_value(v)))
                .collect();
            Value::Hash(Rc::new(RefCell::new(pairs)))
        }
    }
}

lazy_static::lazy_static! {
    static ref SOLIDB_STATES: RwLock<HashMap<usize, SolidbState>> = RwLock::new(HashMap::new());
    static ref SOLIDB_NEXT_ID: AtomicUsize = AtomicUsize::new(1);
}

struct SolidbState {
    host: String,
    database: String,
    auth_username: Option<String>,
    auth_password: Option<String>,
    connected: bool,
}

impl SolidbState {
    fn new(host: String, database: String) -> Self {
        Self {
            host,
            database,
            auth_username: None,
            auth_password: None,
            connected: false,
        }
    }
}

pub fn register_solidb_builtins(env: &mut Environment) {
    register_global_solidb_functions(env);
    register_solidb_class(env);
}

fn register_global_solidb_functions(env: &mut Environment) {
    env.define(
        "solidb_connect".to_string(),
        Value::NativeFunction(NativeFunction::new("solidb_connect", Some(1), |args| {
            let addr = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "solidb_connect() expects string address, got {}",
                        other.type_name()
                    ))
                }
            };

            Ok(exec_db_sync(move || {
                let client = SoliDBClient::connect(&addr)
                    .map_err(|e| format!("Failed to connect to SoliDB: {}", e))?;
                let version = client.ping().map_err(|e| format!("Ping failed: {}", e))?;
                Ok(format!("Connected (ping: {})", version))
            }))
        })),
    );

    env.define(
        "solidb_ping".to_string(),
        Value::NativeFunction(NativeFunction::new("solidb_ping", Some(1), |args| {
            let addr = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "solidb_ping() expects string address, got {}",
                        other.type_name()
                    ))
                }
            };

            Ok(exec_db_sync(move || {
                let client = SoliDBClient::connect(&addr)
                    .map_err(|e| format!("Failed to connect: {}", e))?;
                let timestamp = client.ping().map_err(|e| format!("Ping failed: {}", e))?;
                Ok(timestamp.to_string())
            }))
        })),
    );

    env.define(
        "solidb_auth".to_string(),
        Value::NativeFunction(NativeFunction::new("solidb_auth", Some(4), |args| {
            let addr = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "solidb_auth() expects string address as first argument, got {}",
                        other.type_name()
                    ))
                }
            };

            let _database = match &args[1] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "solidb_auth() expects string database as second argument, got {}",
                        other.type_name()
                    ))
                }
            };

            let username = match &args[2] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "solidb_auth() expects string username as third argument, got {}",
                        other.type_name()
                    ))
                }
            };

            let password = match &args[3] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "solidb_auth() expects string password as fourth argument, got {}",
                        other.type_name()
                    ))
                }
            };

            Ok(exec_db_sync(move || {
                let _client = SoliDBClient::connect(&addr)
                    .map_err(|e| format!("Failed to connect: {}", e))?
                    .with_basic_auth(&username, &password);
                Ok("Authenticated".to_string())
            }))
        })),
    );

    env.define(
        "solidb_query".to_string(),
        Value::NativeFunction(NativeFunction::new("solidb_query", Some(3), |args| {
            let addr = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "solidb_query() expects string address as first argument, got {}",
                        other.type_name()
                    ))
                }
            };

            let database = match &args[1] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "solidb_query() expects string database as second argument, got {}",
                        other.type_name()
                    ))
                }
            };

            let sdbql = match &args[2] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "solidb_query() expects string SDBQL query as third argument, got {}",
                        other.type_name()
                    ))
                }
            };

            let bind_vars = if args.len() > 3 {
                match &args[3] {
                    Value::Hash(hash) => {
                        let mut map = std::collections::HashMap::new();
                        for (k, v) in hash.borrow().iter() {
                            if let HashKey::String(key) = k {
                                map.insert(key.clone(), value_to_json(v)?);
                            }
                        }
                        Some(map)
                    }
                    _ => None,
                }
            } else {
                None
            };

            Ok(exec_db_json(move || {
                let mut client = SoliDBClient::connect(&addr)
                    .map_err(|e| format!("Failed to connect: {}", e))?;
                client.set_database(&database);
                let results = client
                    .query(&sdbql, bind_vars)
                    .map_err(|e| format!("Query failed: {}", e))?;
                // Return directly as JSON array - skip string serialization
                Ok(serde_json::Value::Array(results))
            }))
        })),
    );
}

fn register_solidb_class(env: &mut Environment) {
    let solidb_class = Rc::new(crate::interpreter::value::Class {
        name: "Solidb".to_string(),
        superclass: None,
        methods: std::collections::HashMap::new(),
        static_methods: std::collections::HashMap::new(),
        native_static_methods: std::collections::HashMap::new(),
        native_methods: std::collections::HashMap::new(),
        static_fields: Rc::new(RefCell::new(std::collections::HashMap::new())),
        fields: std::collections::HashMap::new(),
        constructor: None,
        all_methods_cache: RefCell::new(None),
        all_native_methods_cache: RefCell::new(None),
    });

    env.define("Solidb".to_string(), Value::Class(solidb_class.clone()));

    env.define(
        "Solidb".to_string(),
        Value::NativeFunction(NativeFunction::new("Solidb", Some(2), move |args| {
            let host = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "Solidb() expects string host as first argument, got {}",
                        other.type_name()
                    ))
                }
            };

            let database = match &args[1] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "Solidb() expects string database as second argument, got {}",
                        other.type_name()
                    ))
                }
            };

            let instance_id = SOLIDB_NEXT_ID.fetch_add(1, Ordering::SeqCst);

            let mut states = SOLIDB_STATES.write().map_err(|e| e.to_string())?;
            states.insert(instance_id, SolidbState::new(host, database));
            drop(states);

            let mut inner = Instance::new(solidb_class.clone());
            inner.set("_id".to_string(), Value::Int(instance_id as i64));
            let instance = Rc::new(RefCell::new(inner));

            Ok(Value::Instance(instance))
        })),
    );

    let method_definitions = vec![
        ("auth", 2),
        ("query", 1),
        ("get", 2),
        ("insert", 3),
        ("update", 3),
        ("upsert", 3),
        ("delete", 2),
        ("list", 1),
        ("explain", 1),
        ("ping", 0),
        ("connected", 0),
        ("close", 0),
        ("create_collection", 1),
        ("drop_collection", 1),
        ("list_collections", 0),
        ("collection_stats", 1),
        ("create_index", 4),
        ("drop_index", 2),
        ("list_indexes", 1),
        ("store_blob", 4),
        ("get_blob", 2),
        ("get_blob_metadata", 2),
        ("delete_blob", 2),
    ];

    for (method_name, min_args) in method_definitions {
        let method = method_name.to_string();
        let arity = min_args + 1;

        env.define(
            format!("solidb_{}", method_name),
            Value::NativeFunction(NativeFunction::new(
                format!("solidb_{}", method_name),
                Some(arity),
                move |args| {
                    if args.len() < arity {
                        return Err(format!(
                            "solidb_{}() requires at least {} argument(s)",
                            method, arity
                        ));
                    }

                    let instance_rc = match &args[0] {
                        Value::Instance(inst) => inst.clone(),
                        other => {
                            return Err(format!(
                                "solidb_{}() must be called on a Solidb instance, got {}",
                                method,
                                other.type_name()
                            ))
                        }
                    };

                    let instance_id = {
                        let inst_guard = instance_rc.borrow();
                        match inst_guard.get("_id") {
                            Some(Value::Int(id)) => id as usize,
                            _ => return Err("Solidb instance missing _id".to_string()),
                        }
                    };

                    let states = SOLIDB_STATES.read().map_err(|e| e.to_string())?;
                    let state = states
                        .get(&instance_id)
                        .ok_or_else(|| "Solidb instance not found".to_string())?;

                    let host = state.host.clone();
                    let database = state.database.clone();
                    let auth_username = state.auth_username.clone();
                    let auth_password = state.auth_password.clone();
                    let state_connected = state.connected;
                    drop(states);

                    match method.as_str() {
                        "auth" => {
                            let username = match &args[1] {
                                Value::String(s) => s.clone(),
                                other => {
                                    return Err(format!(
                                        "auth() expects string username, got {}",
                                        other.type_name()
                                    ))
                                }
                            };
                            let password = match &args[2] {
                                Value::String(s) => s.clone(),
                                other => {
                                    return Err(format!(
                                        "auth() expects string password, got {}",
                                        other.type_name()
                                    ))
                                }
                            };

                            let mut states = SOLIDB_STATES.write().map_err(|e| e.to_string())?;
                            let state = states
                                .get_mut(&instance_id)
                                .ok_or_else(|| "Solidb instance not found".to_string())?;
                            state.auth_username = Some(username.clone());
                            state.auth_password = Some(password.clone());
                            state.connected = true;
                            drop(states);

                            Ok(exec_db_sync(move || {
                                let _client = SoliDBClient::connect(&host)
                                    .map_err(|e| format!("Failed to connect: {}", e))?
                                    .with_basic_auth(&username, &password);
                                Ok("Authenticated".to_string())
                            }))
                        }
                        "query" => {
                            let sdbql = match &args[1] {
                                Value::String(s) => s.clone(),
                                other => {
                                    return Err(format!(
                                        "query() expects string SDBQL, got {}",
                                        other.type_name()
                                    ))
                                }
                            };
                            let bind_vars = if args.len() > 2 {
                                match &args[2] {
                                    Value::Hash(hash) => {
                                        let mut map = std::collections::HashMap::new();
                                        for (k, v) in hash.borrow().iter() {
                                            if let HashKey::String(key) = k {
                                                map.insert(key.clone(), value_to_json(v)?);
                                            }
                                        }
                                        Some(map)
                                    }
                                    _ => None,
                                }
                            } else {
                                None
                            };
                            let auth_username = auth_username.clone();
                            let auth_password = auth_password.clone();
                            Ok(exec_db_json(move || {
                                let mut client = SoliDBClient::connect(&host)
                                    .map_err(|e| format!("Failed to connect: {}", e))?;
                                client.set_database(&database);
                                if let (Some(u), Some(p)) =
                                    (auth_username.as_deref(), auth_password.as_deref())
                                {
                                    client = client.with_basic_auth(u, p);
                                }
                                let results = client.query(&sdbql, bind_vars)
                                    .map_err(|e| format!("Query failed: {}", e))?;
                                // Return directly as JSON array - skip string serialization
                                Ok(serde_json::Value::Array(results))
                            }))
                        }
                        "get" => {
                            let collection = match &args[1] {
                                Value::String(s) => s.clone(),
                                other => {
                                    return Err(format!(
                                        "get() expects string collection, got {}",
                                        other.type_name()
                                    ))
                                }
                            };
                            let key = match &args[2] {
                                Value::String(s) => s.clone(),
                                other => {
                                    return Err(format!(
                                        "get() expects string key, got {}",
                                        other.type_name()
                                    ))
                                }
                            };
                            let auth_username = auth_username.clone();
                            let auth_password = auth_password.clone();
                            Ok(exec_db_json(move || {
                                let mut client = SoliDBClient::connect(&host)
                                    .map_err(|e| format!("Failed to connect: {}", e))?;
                                client.set_database(&database);
                                if let (Some(u), Some(p)) =
                                    (auth_username.as_deref(), auth_password.as_deref())
                                {
                                    client = client.with_basic_auth(u, p);
                                }
                                let doc = client.get(&collection, &key)
                                    .map_err(|e| format!("Get failed: {}", e))?;
                                // Return doc or null if not found
                                Ok(doc.unwrap_or(serde_json::Value::Null))
                            }))
                        }
                        "insert" => {
                            let collection = match &args[1] {
                                Value::String(s) => s.clone(),
                                other => {
                                    return Err(format!(
                                        "insert() expects string collection, got {}",
                                        other.type_name()
                                    ))
                                }
                            };
                            let key = match &args[2] {
                                Value::String(s) => Some(s.clone()),
                                Value::Null => None,
                                other => {
                                    return Err(format!(
                                        "insert() expects string or null key, got {}",
                                        other.type_name()
                                    ))
                                }
                            };
                            let document = value_to_json(&args[3])?;
                            let auth_username = auth_username.clone();
                            let auth_password = auth_password.clone();
                            Ok(exec_db_json(move || {
                                let mut client = SoliDBClient::connect(&host)
                                    .map_err(|e| format!("Failed to connect: {}", e))?;
                                client.set_database(&database);
                                if let (Some(u), Some(p)) =
                                    (auth_username.as_deref(), auth_password.as_deref())
                                {
                                    client = client.with_basic_auth(u, p);
                                }
                                let result = client.insert(&collection, key.as_deref(), document)
                                    .map_err(|e| format!("Insert failed: {}", e))?;
                                Ok(result)
                            }))
                        }
                        "update" | "upsert" => {
                            let collection = match &args[1] {
                                Value::String(s) => s.clone(),
                                other => {
                                    return Err(format!(
                                        "{}() expects string collection, got {}",
                                        method,
                                        other.type_name()
                                    ))
                                }
                            };
                            let key = match &args[2] {
                                Value::String(s) => s.clone(),
                                other => {
                                    return Err(format!(
                                        "{}() expects string key, got {}",
                                        method,
                                        other.type_name()
                                    ))
                                }
                            };
                            let document = value_to_json(&args[3])?;
                            let merge = method == "upsert";
                            let auth_username = auth_username.clone();
                            let auth_password = auth_password.clone();
                            Ok(exec_db_json(move || {
                                let mut client = SoliDBClient::connect(&host)
                                    .map_err(|e| format!("Failed to connect: {}", e))?;
                                client.set_database(&database);
                                if let (Some(u), Some(p)) =
                                    (auth_username.as_deref(), auth_password.as_deref())
                                {
                                    client = client.with_basic_auth(u, p);
                                }
                                let result = client.update(&collection, &key, document, merge)
                                    .map_err(|e| format!("Update failed: {}", e))?;
                                Ok(result)
                            }))
                        }
                        "delete" => {
                            let collection = match &args[1] {
                                Value::String(s) => s.clone(),
                                other => {
                                    return Err(format!(
                                        "delete() expects string collection, got {}",
                                        other.type_name()
                                    ))
                                }
                            };
                            let key = match &args[2] {
                                Value::String(s) => s.clone(),
                                other => {
                                    return Err(format!(
                                        "delete() expects string key, got {}",
                                        other.type_name()
                                    ))
                                }
                            };
                            let auth_username = auth_username.clone();
                            let auth_password = auth_password.clone();
                            Ok(exec_db_sync(move || {
                                let mut client = SoliDBClient::connect(&host)
                                    .map_err(|e| format!("Failed to connect: {}", e))?;
                                client.set_database(&database);
                                if let (Some(u), Some(p)) =
                                    (auth_username.as_deref(), auth_password.as_deref())
                                {
                                    client = client.with_basic_auth(u, p);
                                }
                                client.delete(&collection, &key)
                                    .map_err(|e| format!("Delete failed: {}", e))?;
                                Ok("OK".to_string())
                            }))
                        }
                        "list" => {
                            let collection = match &args[1] {
                                Value::String(s) => s.clone(),
                                other => {
                                    return Err(format!(
                                        "list() expects string collection, got {}",
                                        other.type_name()
                                    ))
                                }
                            };
                            let auth_username = auth_username.clone();
                            let auth_password = auth_password.clone();
                            Ok(exec_db_json(move || {
                                let mut client = SoliDBClient::connect(&host)
                                    .map_err(|e| format!("Failed to connect: {}", e))?;
                                client.set_database(&database);
                                if let (Some(u), Some(p)) =
                                    (auth_username.as_deref(), auth_password.as_deref())
                                {
                                    client = client.with_basic_auth(u, p);
                                }
                                let docs = client.list(&collection, 100, 0)
                                    .map_err(|e| format!("List failed: {}", e))?;
                                // Return directly as JSON array - skip string serialization
                                Ok(serde_json::Value::Array(docs))
                            }))
                        }
                        "explain" => {
                            let sdbql = match &args[1] {
                                Value::String(s) => s.clone(),
                                other => {
                                    return Err(format!(
                                        "explain() expects string SDBQL, got {}",
                                        other.type_name()
                                    ))
                                }
                            };
                            let bind_vars = if args.len() > 2 {
                                match &args[2] {
                                    Value::Hash(hash) => {
                                        let mut map = std::collections::HashMap::new();
                                        for (k, v) in hash.borrow().iter() {
                                            if let HashKey::String(key) = k {
                                                map.insert(key.clone(), value_to_json(v)?);
                                            }
                                        }
                                        Some(map)
                                    }
                                    _ => None,
                                }
                            } else {
                                None
                            };
                            let auth_username = auth_username.clone();
                            let auth_password = auth_password.clone();
                            Ok(exec_db_json(move || {
                                let mut client = SoliDBClient::connect(&host)
                                    .map_err(|e| format!("Failed to connect: {}", e))?;
                                client.set_database(&database);
                                if let (Some(u), Some(p)) =
                                    (auth_username.as_deref(), auth_password.as_deref())
                                {
                                    client = client.with_basic_auth(u, p);
                                }
                                let explanation = client.explain(&sdbql, bind_vars)
                                    .map_err(|e| format!("Explain failed: {}", e))?;
                                Ok(explanation)
                            }))
                        }
                        "ping" => {
                            let auth_username = auth_username.clone();
                            let auth_password = auth_password.clone();
                            Ok(exec_db_sync(move || {
                                let mut client = SoliDBClient::connect(&host)
                                    .map_err(|e| format!("Failed to connect: {}", e))?;
                                client.set_database(&database);
                                if let (Some(u), Some(p)) =
                                    (auth_username.as_deref(), auth_password.as_deref())
                                {
                                    client = client.with_basic_auth(u, p);
                                }
                                let timestamp = client.ping()
                                    .map_err(|e| format!("Ping failed: {}", e))?;
                                Ok(timestamp.to_string())
                            }))
                        }
                        "connected" => Ok(Value::Bool(state_connected)),
                        "close" => {
                            // Remove state from global HashMap to free memory
                            let mut states = SOLIDB_STATES.write().map_err(|e| e.to_string())?;
                            states.remove(&instance_id);
                            Ok(Value::Bool(true))
                        }
                        "create_collection" => {
                            let name = match &args[1] {
                                Value::String(s) => s.clone(),
                                other => {
                                    return Err(format!(
                                        "create_collection() expects string name, got {}",
                                        other.type_name()
                                    ))
                                }
                            };
                            let _collection_type = if args.len() > 2 {
                                match &args[2] {
                                    Value::String(s) => Some(s.clone()),
                                    Value::Null => None,
                                    other => {
                                        return Err(format!(
                                            "create_collection() expects string or null type, got {}",
                                            other.type_name()
                                        ))
                                    }
                                }
                            } else {
                                None
                            };
                            let auth_username = auth_username.clone();
                            let auth_password = auth_password.clone();
                            Ok(exec_db_sync(move || {
                                let mut client = SoliDBClient::connect(&host)
                                    .map_err(|e| format!("Failed to connect: {}", e))?;
                                client.set_database(&database);
                                if let (Some(u), Some(p)) =
                                    (auth_username.as_deref(), auth_password.as_deref())
                                {
                                    client = client.with_basic_auth(u, p);
                                }
                                client.create_collection(&name)
                                    .map_err(|e| format!("Create collection failed: {}", e))?;
                                Ok(format!("Created collection: {}", name))
                            }))
                        }
                        "drop_collection" => {
                            let name = match &args[1] {
                                Value::String(s) => s.clone(),
                                other => {
                                    return Err(format!(
                                        "drop_collection() expects string name, got {}",
                                        other.type_name()
                                    ))
                                }
                            };
                            let auth_username = auth_username.clone();
                            let auth_password = auth_password.clone();
                            Ok(exec_db_sync(move || {
                                let mut client = SoliDBClient::connect(&host)
                                    .map_err(|e| format!("Failed to connect: {}", e))?;
                                client.set_database(&database);
                                if let (Some(u), Some(p)) =
                                    (auth_username.as_deref(), auth_password.as_deref())
                                {
                                    client = client.with_basic_auth(u, p);
                                }
                                client.drop_collection(&name)
                                    .map_err(|e| format!("Drop collection failed: {}", e))?;
                                Ok(format!("Dropped collection: {}", name))
                            }))
                        }
                        "list_collections" => {
                            let auth_username = auth_username.clone();
                            let auth_password = auth_password.clone();
                            Ok(exec_db_json(move || {
                                let mut client = SoliDBClient::connect(&host)
                                    .map_err(|e| format!("Failed to connect: {}", e))?;
                                client.set_database(&database);
                                if let (Some(u), Some(p)) =
                                    (auth_username.as_deref(), auth_password.as_deref())
                                {
                                    client = client.with_basic_auth(u, p);
                                }
                                let collections = client.list_collections()
                                    .map_err(|e| format!("List collections failed: {}", e))?;
                                // Return directly as JSON array - skip string serialization
                                Ok(serde_json::Value::Array(collections))
                            }))
                        }
                        "collection_stats" => {
                            let collection = match &args[1] {
                                Value::String(s) => s.clone(),
                                other => {
                                    return Err(format!(
                                        "collection_stats() expects string collection, got {}",
                                        other.type_name()
                                    ))
                                }
                            };
                            let auth_username = auth_username.clone();
                            let auth_password = auth_password.clone();
                            Ok(exec_db_json(move || {
                                let mut client = SoliDBClient::connect(&host)
                                    .map_err(|e| format!("Failed to connect: {}", e))?;
                                client.set_database(&database);
                                if let (Some(u), Some(p)) =
                                    (auth_username.as_deref(), auth_password.as_deref())
                                {
                                    client = client.with_basic_auth(u, p);
                                }
                                let stats = client.collection_stats(&collection)
                                    .map_err(|e| format!("Collection stats failed: {}", e))?;
                                Ok(stats)
                            }))
                        }
                        "create_index" => {
                            let collection = match &args[1] {
                                Value::String(s) => s.clone(),
                                other => {
                                    return Err(format!(
                                        "create_index() expects string collection, got {}",
                                        other.type_name()
                                    ))
                                }
                            };
                            let name = match &args[2] {
                                Value::String(s) => s.clone(),
                                other => {
                                    return Err(format!(
                                        "create_index() expects string index name, got {}",
                                        other.type_name()
                                    ))
                                }
                            };
                            let fields: Vec<String> = match &args[3] {
                                Value::Array(arr) => {
                                    let borrowed = arr.borrow();
                                    borrowed
                                        .iter()
                                        .filter_map(|v| {
                                            if let Value::String(s) = v {
                                                Some(s.clone())
                                            } else {
                                                None
                                            }
                                        })
                                        .collect()
                                }
                                Value::String(s) => vec![s.clone()],
                                other => {
                                    return Err(format!(
                                        "create_index() expects array or string fields, got {}",
                                        other.type_name()
                                    ))
                                }
                            };
                            let (unique, sparse) = if args.len() > 4 {
                                match &args[4] {
                                    Value::Bool(b) => (*b, false),
                                    Value::Hash(hash) => {
                                        let borrowed = hash.borrow();
                                        let unique = borrowed
                                            .iter()
                                            .find(|(k, _)| matches!(k, HashKey::String(s) if s == "unique"))
                                            .and_then(|(_, v)| if let Value::Bool(b) = v { Some(*b) } else { None })
                                            .unwrap_or(false);
                                        let sparse = borrowed
                                            .iter()
                                            .find(|(k, _)| matches!(k, HashKey::String(s) if s == "sparse"))
                                            .and_then(|(_, v)| if let Value::Bool(b) = v { Some(*b) } else { None })
                                            .unwrap_or(false);
                                        (unique, sparse)
                                    }
                                    _ => (false, false),
                                }
                            } else {
                                (false, false)
                            };
                            let auth_username = auth_username.clone();
                            let auth_password = auth_password.clone();
                            Ok(exec_db_sync(move || {
                                let mut client = SoliDBClient::connect(&host)
                                    .map_err(|e| format!("Failed to connect: {}", e))?;
                                client.set_database(&database);
                                if let (Some(u), Some(p)) =
                                    (auth_username.as_deref(), auth_password.as_deref())
                                {
                                    client = client.with_basic_auth(u, p);
                                }
                                client.create_index(&collection, &name, fields, unique, sparse)
                                    .map_err(|e| format!("Create index failed: {}", e))?;
                                Ok(format!("Created index: {} on {}", name, collection))
                            }))
                        }
                        "drop_index" => {
                            let collection = match &args[1] {
                                Value::String(s) => s.clone(),
                                other => {
                                    return Err(format!(
                                        "drop_index() expects string collection, got {}",
                                        other.type_name()
                                    ))
                                }
                            };
                            let name = match &args[2] {
                                Value::String(s) => s.clone(),
                                other => {
                                    return Err(format!(
                                        "drop_index() expects string index name, got {}",
                                        other.type_name()
                                    ))
                                }
                            };
                            let auth_username = auth_username.clone();
                            let auth_password = auth_password.clone();
                            Ok(exec_db_sync(move || {
                                let mut client = SoliDBClient::connect(&host)
                                    .map_err(|e| format!("Failed to connect: {}", e))?;
                                client.set_database(&database);
                                if let (Some(u), Some(p)) =
                                    (auth_username.as_deref(), auth_password.as_deref())
                                {
                                    client = client.with_basic_auth(u, p);
                                }
                                client.drop_index(&collection, &name)
                                    .map_err(|e| format!("Drop index failed: {}", e))?;
                                Ok(format!("Dropped index: {} from {}", name, collection))
                            }))
                        }
                        "list_indexes" => {
                            let collection = match &args[1] {
                                Value::String(s) => s.clone(),
                                other => {
                                    return Err(format!(
                                        "list_indexes() expects string collection, got {}",
                                        other.type_name()
                                    ))
                                }
                            };
                            let auth_username = auth_username.clone();
                            let auth_password = auth_password.clone();
                            Ok(exec_db_json(move || {
                                let mut client = SoliDBClient::connect(&host)
                                    .map_err(|e| format!("Failed to connect: {}", e))?;
                                client.set_database(&database);
                                if let (Some(u), Some(p)) =
                                    (auth_username.as_deref(), auth_password.as_deref())
                                {
                                    client = client.with_basic_auth(u, p);
                                }
                                let indexes = client.list_indexes(&collection)
                                    .map_err(|e| format!("List indexes failed: {}", e))?;
                                Ok(serde_json::Value::Array(indexes))
                            }))
                        }
                        "store_blob" => {
                            let collection = match &args[1] {
                                Value::String(s) => s.clone(),
                                other => {
                                    return Err(format!(
                                        "store_blob() expects string collection, got {}",
                                        other.type_name()
                                    ))
                                }
                            };
                            let data_base64 = match &args[2] {
                                Value::String(s) => s.clone(),
                                other => {
                                    return Err(format!(
                                        "store_blob() expects base64 string data, got {}",
                                        other.type_name()
                                    ))
                                }
                            };
                            let filename = match &args[3] {
                                Value::String(s) => s.clone(),
                                other => {
                                    return Err(format!(
                                        "store_blob() expects string filename, got {}",
                                        other.type_name()
                                    ))
                                }
                            };
                            let content_type = match &args[4] {
                                Value::String(s) => s.clone(),
                                other => {
                                    return Err(format!(
                                        "store_blob() expects string content_type, got {}",
                                        other.type_name()
                                    ))
                                }
                            };
                            let data = STANDARD.decode(&data_base64)
                                .map_err(|e| format!("Failed to decode base64: {}", e))?;
                            let auth_username = auth_username.clone();
                            let auth_password = auth_password.clone();
                            Ok(exec_db_sync(move || {
                                let mut client = SoliDBClient::connect(&host)
                                    .map_err(|e| format!("Failed to connect: {}", e))?;
                                client.set_database(&database);
                                if let (Some(u), Some(p)) =
                                    (auth_username.as_deref(), auth_password.as_deref())
                                {
                                    client = client.with_basic_auth(u, p);
                                }
                                let blob_id = client.store_blob(&collection, &data, &filename, &content_type)
                                    .map_err(|e| format!("Store blob failed: {}", e))?;
                                Ok(blob_id)
                            }))
                        }
                        "get_blob" => {
                            let collection = match &args[1] {
                                Value::String(s) => s.clone(),
                                other => {
                                    return Err(format!(
                                        "get_blob() expects string collection, got {}",
                                        other.type_name()
                                    ))
                                }
                            };
                            let blob_id = match &args[2] {
                                Value::String(s) => s.clone(),
                                other => {
                                    return Err(format!(
                                        "get_blob() expects string blob_id, got {}",
                                        other.type_name()
                                    ))
                                }
                            };
                            let auth_username = auth_username.clone();
                            let auth_password = auth_password.clone();
                            Ok(exec_db_sync(move || {
                                let mut client = SoliDBClient::connect(&host)
                                    .map_err(|e| format!("Failed to connect: {}", e))?;
                                client.set_database(&database);
                                if let (Some(u), Some(p)) =
                                    (auth_username.as_deref(), auth_password.as_deref())
                                {
                                    client = client.with_basic_auth(u, p);
                                }
                                let data = client.get_blob(&collection, &blob_id)
                                    .map_err(|e| format!("Get blob failed: {}", e))?;
                                Ok(STANDARD.encode(&data))
                            }))
                        }
                        "get_blob_metadata" => {
                            let collection = match &args[1] {
                                Value::String(s) => s.clone(),
                                other => {
                                    return Err(format!(
                                        "get_blob_metadata() expects string collection, got {}",
                                        other.type_name()
                                    ))
                                }
                            };
                            let blob_id = match &args[2] {
                                Value::String(s) => s.clone(),
                                other => {
                                    return Err(format!(
                                        "get_blob_metadata() expects string blob_id, got {}",
                                        other.type_name()
                                    ))
                                }
                            };
                            let auth_username = auth_username.clone();
                            let auth_password = auth_password.clone();
                            Ok(exec_db_json(move || {
                                let mut client = SoliDBClient::connect(&host)
                                    .map_err(|e| format!("Failed to connect: {}", e))?;
                                client.set_database(&database);
                                if let (Some(u), Some(p)) =
                                    (auth_username.as_deref(), auth_password.as_deref())
                                {
                                    client = client.with_basic_auth(u, p);
                                }
                                let metadata = client.get_blob_metadata(&collection, &blob_id)
                                    .map_err(|e| format!("Get blob metadata failed: {}", e))?;
                                Ok(metadata)
                            }))
                        }
                        "delete_blob" => {
                            let collection = match &args[1] {
                                Value::String(s) => s.clone(),
                                other => {
                                    return Err(format!(
                                        "delete_blob() expects string collection, got {}",
                                        other.type_name()
                                    ))
                                }
                            };
                            let blob_id = match &args[2] {
                                Value::String(s) => s.clone(),
                                other => {
                                    return Err(format!(
                                        "delete_blob() expects string blob_id, got {}",
                                        other.type_name()
                                    ))
                                }
                            };
                            let auth_username = auth_username.clone();
                            let auth_password = auth_password.clone();
                            Ok(exec_db_sync(move || {
                                let mut client = SoliDBClient::connect(&host)
                                    .map_err(|e| format!("Failed to connect: {}", e))?;
                                client.set_database(&database);
                                if let (Some(u), Some(p)) =
                                    (auth_username.as_deref(), auth_password.as_deref())
                                {
                                    client = client.with_basic_auth(u, p);
                                }
                                client.delete_blob(&collection, &blob_id)
                                    .map_err(|e| format!("Delete blob failed: {}", e))?;
                                Ok("OK".to_string())
                            }))
                        }
                        _ => Err(format!("Unknown method: {}", method)),
                    }
                },
            )),
        );
    }
}

// Use centralized value_to_json from value module
use crate::interpreter::value::value_to_json;
