//! SoliDB client built-in functions.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

use solidb_client::SoliDBClient;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{FutureState, HttpFutureKind, Instance, NativeFunction, Value};

fn spawn_db_future<F>(f: F) -> Value
where
    F: FnOnce() -> Result<String, String> + Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let result = f();
        let _ = tx.send(result);
    });
    Value::Future(Arc::new(Mutex::new(FutureState::Pending {
        receiver: rx,
        kind: HttpFutureKind::Json,
    })))
}

lazy_static::lazy_static! {
    static ref SOLIDB_STATES: Mutex<HashMap<usize, SolidbState>> = Mutex::new(HashMap::new());
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

            Ok(spawn_db_future(move || {
                tokio::runtime::Runtime::new()
                    .map_err(|e| format!("Failed to create async runtime: {}", e))?
                    .block_on(async {
                        let mut client = SoliDBClient::connect(&addr)
                            .await
                            .map_err(|e| format!("Failed to connect to SoliDB: {}", e))?;
                        let version = client
                            .ping()
                            .await
                            .map_err(|e| format!("Ping failed: {}", e))?;
                        Ok(format!("Connected (ping: {})", version))
                    })
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

            Ok(spawn_db_future(move || {
                tokio::runtime::Runtime::new()
                    .map_err(|e| format!("Failed to create async runtime: {}", e))?
                    .block_on(async {
                        let mut client = SoliDBClient::connect(&addr)
                            .await
                            .map_err(|e| format!("Failed to connect: {}", e))?;
                        let timestamp = client
                            .ping()
                            .await
                            .map_err(|e| format!("Ping failed: {}", e))?;
                        Ok(timestamp.to_string())
                    })
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

            let database = match &args[1] {
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

            Ok(spawn_db_future(move || {
                tokio::runtime::Runtime::new()
                    .map_err(|e| format!("Failed to create async runtime: {}", e))?
                    .block_on(async {
                        let mut client = SoliDBClient::connect(&addr)
                            .await
                            .map_err(|e| format!("Failed to connect: {}", e))?;
                        client
                            .auth(&database, &username, &password)
                            .await
                            .map_err(|e| format!("Auth failed: {}", e))?;
                        Ok("Authenticated".to_string())
                    })
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
                            if let Value::String(key) = k {
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

            Ok(spawn_db_future(move || {
                tokio::runtime::Runtime::new()
                    .map_err(|e| format!("Failed to create async runtime: {}", e))?
                    .block_on(async {
                        let mut client = SoliDBClient::connect(&addr)
                            .await
                            .map_err(|e| format!("Failed to connect: {}", e))?;
                        let results = client
                            .query(&database, &sdbql, bind_vars)
                            .await
                            .map_err(|e| format!("Query failed: {}", e))?;
                        Ok(serde_json::to_string(&results).unwrap_or_default())
                    })
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
        constructor: None,
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

            let mut states = SOLIDB_STATES.lock().map_err(|e| e.to_string())?;
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
        // Collection management
        ("create_collection", 1),
        ("drop_collection", 1),
        ("list_collections", 0),
        ("collection_stats", 1),
        // Index management
        ("create_index", 4),
        ("drop_index", 2),
        ("list_indexes", 1),
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

                    let mut states = SOLIDB_STATES.lock().map_err(|e| e.to_string())?;
                    let state = states
                        .get_mut(&instance_id)
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

                            let mut states = SOLIDB_STATES.lock().map_err(|e| e.to_string())?;
                            let state = states
                                .get_mut(&instance_id)
                                .ok_or_else(|| "Solidb instance not found".to_string())?;
                            state.auth_username = Some(username.clone());
                            state.auth_password = Some(password.clone());
                            state.connected = true;
                            drop(states);

                            Ok(spawn_db_future(move || {
                                tokio::runtime::Runtime::new()
                                    .map_err(|e| format!("Failed to create async runtime: {}", e))?
                                    .block_on(async {
                                        let mut client = SoliDBClient::connect(&host)
                                            .await
                                            .map_err(|e| format!("Failed to connect: {}", e))?;
                                        client
                                            .auth(&database, &username, &password)
                                            .await
                                            .map_err(|e| format!("Auth failed: {}", e))?;
                                        Ok("Authenticated".to_string())
                                    })
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
                                            if let Value::String(key) = k {
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
                            Ok(spawn_db_future(move || {
                                tokio::runtime::Runtime::new()
                                    .map_err(|e| format!("Failed to create async runtime: {}", e))?
                                    .block_on(async {
                                        let mut client = SoliDBClient::connect(&host)
                                            .await
                                            .map_err(|e| format!("Failed to connect: {}", e))?;
                                        if let (Some(u), Some(p)) =
                                            (auth_username.as_deref(), auth_password.as_deref())
                                        {
                                            client
                                                .auth(&database, u, p)
                                                .await
                                                .map_err(|e| format!("Auth failed: {}", e))?;
                                        }
                                        let results = client
                                            .query(&database, &sdbql, bind_vars)
                                            .await
                                            .map_err(|e| format!("Query failed: {}", e))?;
                                        Ok(serde_json::to_string(&results).unwrap_or_default())
                                    })
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
                            Ok(spawn_db_future(move || {
                                tokio::runtime::Runtime::new()
                                    .map_err(|e| format!("Failed to create async runtime: {}", e))?
                                    .block_on(async {
                                        let mut client = SoliDBClient::connect(&host)
                                            .await
                                            .map_err(|e| format!("Failed to connect: {}", e))?;
                                        if let (Some(u), Some(p)) =
                                            (auth_username.as_deref(), auth_password.as_deref())
                                        {
                                            client
                                                .auth(&database, u, p)
                                                .await
                                                .map_err(|e| format!("Auth failed: {}", e))?;
                                        }
                                        let doc = client
                                            .get(&database, &collection, &key)
                                            .await
                                            .map_err(|e| format!("Get failed: {}", e))?;
                                        Ok(serde_json::to_string(&doc).unwrap_or_default())
                                    })
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
                            Ok(spawn_db_future(move || {
                                tokio::runtime::Runtime::new()
                                    .map_err(|e| format!("Failed to create async runtime: {}", e))?
                                    .block_on(async {
                                        let mut client = SoliDBClient::connect(&host)
                                            .await
                                            .map_err(|e| format!("Failed to connect: {}", e))?;
                                        if let (Some(u), Some(p)) =
                                            (auth_username.as_deref(), auth_password.as_deref())
                                        {
                                            client
                                                .auth(&database, u, p)
                                                .await
                                                .map_err(|e| format!("Auth failed: {}", e))?;
                                        }
                                        let result = client
                                            .insert(
                                                &database,
                                                &collection,
                                                key.as_deref(),
                                                document,
                                            )
                                            .await
                                            .map_err(|e| format!("Insert failed: {}", e))?;
                                        Ok(serde_json::to_string(&result).unwrap_or_default())
                                    })
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
                            Ok(spawn_db_future(move || {
                                tokio::runtime::Runtime::new()
                                    .map_err(|e| format!("Failed to create async runtime: {}", e))?
                                    .block_on(async {
                                        let mut client = SoliDBClient::connect(&host)
                                            .await
                                            .map_err(|e| format!("Failed to connect: {}", e))?;
                                        if let (Some(u), Some(p)) =
                                            (auth_username.as_deref(), auth_password.as_deref())
                                        {
                                            client
                                                .auth(&database, u, p)
                                                .await
                                                .map_err(|e| format!("Auth failed: {}", e))?;
                                        }
                                        let result = client
                                            .update(&database, &collection, &key, document, merge)
                                            .await
                                            .map_err(|e| format!("Update failed: {}", e))?;
                                        Ok(serde_json::to_string(&result).unwrap_or_default())
                                    })
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
                            Ok(spawn_db_future(move || {
                                tokio::runtime::Runtime::new()
                                    .map_err(|e| format!("Failed to create async runtime: {}", e))?
                                    .block_on(async {
                                        let mut client = SoliDBClient::connect(&host)
                                            .await
                                            .map_err(|e| format!("Failed to connect: {}", e))?;
                                        if let (Some(u), Some(p)) =
                                            (auth_username.as_deref(), auth_password.as_deref())
                                        {
                                            client
                                                .auth(&database, u, p)
                                                .await
                                                .map_err(|e| format!("Auth failed: {}", e))?;
                                        }
                                        client
                                            .delete(&database, &collection, &key)
                                            .await
                                            .map_err(|e| format!("Delete failed: {}", e))?;
                                        Ok("OK".to_string())
                                    })
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
                            Ok(spawn_db_future(move || {
                                tokio::runtime::Runtime::new()
                                    .map_err(|e| format!("Failed to create async runtime: {}", e))?
                                    .block_on(async {
                                        let mut client = SoliDBClient::connect(&host)
                                            .await
                                            .map_err(|e| format!("Failed to connect: {}", e))?;
                                        if let (Some(u), Some(p)) =
                                            (auth_username.as_deref(), auth_password.as_deref())
                                        {
                                            client
                                                .auth(&database, u, p)
                                                .await
                                                .map_err(|e| format!("Auth failed: {}", e))?;
                                        }
                                        let (docs, _) = client
                                            .list(&database, &collection, None, None)
                                            .await
                                            .map_err(|e| format!("List failed: {}", e))?;
                                        Ok(serde_json::to_string(&docs).unwrap_or_default())
                                    })
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
                                            if let Value::String(key) = k {
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
                            Ok(spawn_db_future(move || {
                                tokio::runtime::Runtime::new()
                                    .map_err(|e| format!("Failed to create async runtime: {}", e))?
                                    .block_on(async {
                                        let mut client = SoliDBClient::connect(&host)
                                            .await
                                            .map_err(|e| format!("Failed to connect: {}", e))?;
                                        if let (Some(u), Some(p)) =
                                            (auth_username.as_deref(), auth_password.as_deref())
                                        {
                                            client
                                                .auth(&database, u, p)
                                                .await
                                                .map_err(|e| format!("Auth failed: {}", e))?;
                                        }
                                        let explanation = client
                                            .explain(&database, &sdbql, bind_vars)
                                            .await
                                            .map_err(|e| format!("Explain failed: {}", e))?;
                                        Ok(serde_json::to_string(&explanation).unwrap_or_default())
                                    })
                            }))
                        }
                        "ping" => {
                            let auth_username = auth_username.clone();
                            let auth_password = auth_password.clone();
                            Ok(spawn_db_future(move || {
                                tokio::runtime::Runtime::new()
                                    .map_err(|e| format!("Failed to create async runtime: {}", e))?
                                    .block_on(async {
                                        let mut client = SoliDBClient::connect(&host)
                                            .await
                                            .map_err(|e| format!("Failed to connect: {}", e))?;
                                        if let (Some(u), Some(p)) =
                                            (auth_username.as_deref(), auth_password.as_deref())
                                        {
                                            client
                                                .auth(&database, u, p)
                                                .await
                                                .map_err(|e| format!("Auth failed: {}", e))?;
                                        }
                                        let timestamp = client
                                            .ping()
                                            .await
                                            .map_err(|e| format!("Ping failed: {}", e))?;
                                        Ok(timestamp.to_string())
                                    })
                            }))
                        }
                        "connected" => Ok(Value::Bool(state_connected)),
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
                            let collection_type = if args.len() > 2 {
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
                            Ok(spawn_db_future(move || {
                                tokio::runtime::Runtime::new()
                                    .map_err(|e| format!("Failed to create async runtime: {}", e))?
                                    .block_on(async {
                                        let mut client = SoliDBClient::connect(&host)
                                            .await
                                            .map_err(|e| format!("Failed to connect: {}", e))?;
                                        if let (Some(u), Some(p)) =
                                            (auth_username.as_deref(), auth_password.as_deref())
                                        {
                                            client
                                                .auth(&database, u, p)
                                                .await
                                                .map_err(|e| format!("Auth failed: {}", e))?;
                                        }
                                        client
                                            .create_collection(&database, &name, collection_type.as_deref())
                                            .await
                                            .map_err(|e| format!("Create collection failed: {}", e))?;
                                        Ok(format!("Created collection: {}", name))
                                    })
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
                            Ok(spawn_db_future(move || {
                                tokio::runtime::Runtime::new()
                                    .map_err(|e| format!("Failed to create async runtime: {}", e))?
                                    .block_on(async {
                                        let mut client = SoliDBClient::connect(&host)
                                            .await
                                            .map_err(|e| format!("Failed to connect: {}", e))?;
                                        if let (Some(u), Some(p)) =
                                            (auth_username.as_deref(), auth_password.as_deref())
                                        {
                                            client
                                                .auth(&database, u, p)
                                                .await
                                                .map_err(|e| format!("Auth failed: {}", e))?;
                                        }
                                        client
                                            .delete_collection(&database, &name)
                                            .await
                                            .map_err(|e| format!("Drop collection failed: {}", e))?;
                                        Ok(format!("Dropped collection: {}", name))
                                    })
                            }))
                        }
                        "list_collections" => {
                            let auth_username = auth_username.clone();
                            let auth_password = auth_password.clone();
                            Ok(spawn_db_future(move || {
                                tokio::runtime::Runtime::new()
                                    .map_err(|e| format!("Failed to create async runtime: {}", e))?
                                    .block_on(async {
                                        let mut client = SoliDBClient::connect(&host)
                                            .await
                                            .map_err(|e| format!("Failed to connect: {}", e))?;
                                        if let (Some(u), Some(p)) =
                                            (auth_username.as_deref(), auth_password.as_deref())
                                        {
                                            client
                                                .auth(&database, u, p)
                                                .await
                                                .map_err(|e| format!("Auth failed: {}", e))?;
                                        }
                                        let collections = client
                                            .list_collections(&database)
                                            .await
                                            .map_err(|e| format!("List collections failed: {}", e))?;
                                        Ok(serde_json::to_string(&collections).unwrap_or_default())
                                    })
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
                            Ok(spawn_db_future(move || {
                                tokio::runtime::Runtime::new()
                                    .map_err(|e| format!("Failed to create async runtime: {}", e))?
                                    .block_on(async {
                                        let mut client = SoliDBClient::connect(&host)
                                            .await
                                            .map_err(|e| format!("Failed to connect: {}", e))?;
                                        if let (Some(u), Some(p)) =
                                            (auth_username.as_deref(), auth_password.as_deref())
                                        {
                                            client
                                                .auth(&database, u, p)
                                                .await
                                                .map_err(|e| format!("Auth failed: {}", e))?;
                                        }
                                        let stats = client
                                            .collection_stats(&database, &collection)
                                            .await
                                            .map_err(|e| format!("Collection stats failed: {}", e))?;
                                        Ok(serde_json::to_string(&stats).unwrap_or_default())
                                    })
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
                                            .find(|(k, _)| matches!(k, Value::String(s) if s == "unique"))
                                            .and_then(|(_, v)| if let Value::Bool(b) = v { Some(*b) } else { None })
                                            .unwrap_or(false);
                                        let sparse = borrowed
                                            .iter()
                                            .find(|(k, _)| matches!(k, Value::String(s) if s == "sparse"))
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
                            Ok(spawn_db_future(move || {
                                tokio::runtime::Runtime::new()
                                    .map_err(|e| format!("Failed to create async runtime: {}", e))?
                                    .block_on(async {
                                        let mut client = SoliDBClient::connect(&host)
                                            .await
                                            .map_err(|e| format!("Failed to connect: {}", e))?;
                                        if let (Some(u), Some(p)) =
                                            (auth_username.as_deref(), auth_password.as_deref())
                                        {
                                            client
                                                .auth(&database, u, p)
                                                .await
                                                .map_err(|e| format!("Auth failed: {}", e))?;
                                        }
                                        client
                                            .create_index(&database, &collection, &name, fields, unique, sparse)
                                            .await
                                            .map_err(|e| format!("Create index failed: {}", e))?;
                                        Ok(format!("Created index: {} on {}", name, collection))
                                    })
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
                            Ok(spawn_db_future(move || {
                                tokio::runtime::Runtime::new()
                                    .map_err(|e| format!("Failed to create async runtime: {}", e))?
                                    .block_on(async {
                                        let mut client = SoliDBClient::connect(&host)
                                            .await
                                            .map_err(|e| format!("Failed to connect: {}", e))?;
                                        if let (Some(u), Some(p)) =
                                            (auth_username.as_deref(), auth_password.as_deref())
                                        {
                                            client
                                                .auth(&database, u, p)
                                                .await
                                                .map_err(|e| format!("Auth failed: {}", e))?;
                                        }
                                        client
                                            .delete_index(&database, &collection, &name)
                                            .await
                                            .map_err(|e| format!("Drop index failed: {}", e))?;
                                        Ok(format!("Dropped index: {} from {}", name, collection))
                                    })
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
                            Ok(spawn_db_future(move || {
                                tokio::runtime::Runtime::new()
                                    .map_err(|e| format!("Failed to create async runtime: {}", e))?
                                    .block_on(async {
                                        let mut client = SoliDBClient::connect(&host)
                                            .await
                                            .map_err(|e| format!("Failed to connect: {}", e))?;
                                        if let (Some(u), Some(p)) =
                                            (auth_username.as_deref(), auth_password.as_deref())
                                        {
                                            client
                                                .auth(&database, u, p)
                                                .await
                                                .map_err(|e| format!("Auth failed: {}", e))?;
                                        }
                                        let indexes = client
                                            .list_indexes(&database, &collection)
                                            .await
                                            .map_err(|e| format!("List indexes failed: {}", e))?;
                                        Ok(serde_json::to_string(&indexes).unwrap_or_default())
                                    })
                            }))
                        }
                        _ => Err(format!("Unknown method: {}", method)),
                    }
                },
            )),
        );
    }
}

fn value_to_json(value: &Value) -> Result<serde_json::Value, String> {
    match value {
        Value::Null => Ok(serde_json::Value::Null),
        Value::Bool(b) => Ok(serde_json::Value::Bool(*b)),
        Value::Int(n) => Ok(serde_json::Value::Number((*n).into())),
        Value::Float(n) => serde_json::Number::from_f64(*n)
            .map(serde_json::Value::Number)
            .ok_or_else(|| "Cannot convert float to JSON".to_string()),
        Value::String(s) => Ok(serde_json::Value::String(s.clone())),
        Value::Array(arr) => {
            let items: Result<Vec<serde_json::Value>, String> =
                arr.borrow().iter().map(value_to_json).collect();
            Ok(serde_json::Value::Array(items?))
        }
        Value::Hash(hash) => {
            let mut map = serde_json::Map::new();
            for (k, v) in hash.borrow().iter() {
                let key = match k {
                    Value::String(s) => s.clone(),
                    _ => format!("{}", k),
                };
                map.insert(key, value_to_json(v)?);
            }
            Ok(serde_json::Value::Object(map))
        }
        other => Err(format!("Cannot convert {} to JSON", other.type_name())),
    }
}
