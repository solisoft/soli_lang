//! Model class and Model/ORM built-ins for SoliLang.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::AtomicUsize;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

use solidb_client::SoliDBClient;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, FutureState, HttpFutureKind, NativeFunction, Value};

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
    static ref MODEL_STATES: Mutex<HashMap<usize, ModelConnectionState>> = Mutex::new(HashMap::new());
    static ref MODEL_NEXT_ID: AtomicUsize = AtomicUsize::new(1);
}

struct ModelConnectionState {
    host: String,
    database: String,
    auth_username: Option<String>,
    auth_password: Option<String>,
}

impl ModelConnectionState {
    fn new(host: String, database: String) -> Self {
        Self {
            host,
            database,
            auth_username: None,
            auth_password: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ModelDefinition {
    pub name: String,
    pub collection: String,
    pub fields: Vec<String>,
    pub soft_deletes: bool,
}

#[derive(Debug)]
pub struct ModelState {
    pub name: String,
    pub collection: String,
    pub database: String,
    pub connected: bool,
}

impl ModelState {
    pub fn new(name: &str, collection: &str, database: &str) -> Self {
        Self {
            name: name.to_string(),
            collection: collection.to_string(),
            database: database.to_string(),
            connected: false,
        }
    }
}

pub struct Model;

impl Model {
    pub fn register_builtins(env: &mut Environment) {
        Self::register_model_class(env);
        Self::register_model_functions(env);
        Self::register_field_functions(env);
        Self::register_migration_functions(env);
    }

    fn register_model_class(env: &mut Environment) {
        let model_class = Class {
            name: "Model".to_string(),
            superclass: None,
            methods: HashMap::new(),
            static_methods: HashMap::new(),
            native_static_methods: HashMap::new(),
            constructor: None,
        };
        env.define("Model".to_string(), Value::Class(Rc::new(model_class)));
    }

    fn register_model_functions(env: &mut Environment) {
        env.define(
            "model_define".to_string(),
            Value::NativeFunction(NativeFunction::new("model_define", Some(2), |_args| {
                println!("Model definition registered");
                Ok(Value::Null)
            })),
        );

        env.define(
            "model_connect".to_string(),
            Value::NativeFunction(NativeFunction::new("model_connect", Some(2), |args| {
                let host = match &args[0] {
                    Value::String(s) => s.clone(),
                    other => {
                        return Err(format!(
                            "model_connect() expects string host as first argument, got {}",
                            other.type_name()
                        ))
                    }
                };

                let database = match &args[1] {
                    Value::String(s) => s.clone(),
                    other => {
                        return Err(format!(
                            "model_connect() expects string database as second argument, got {}",
                            other.type_name()
                        ))
                    }
                };

                Ok(spawn_db_future(move || {
                    tokio::runtime::Runtime::new()
                        .map_err(|e| format!("Failed to create async runtime: {}", e))?
                        .block_on(async {
                            let mut client = SoliDBClient::connect(&host)
                                .await
                                .map_err(|e| format!("Failed to connect to SoliDB: {}", e))?;
                            let version = client
                                .ping()
                                .await
                                .map_err(|e| format!("Ping failed: {}", e))?;
                            Ok(format!("Connected to SoliDB (version: {})", version))
                        })
                }))
            })),
        );

        env.define(
            "model_create".to_string(),
            Value::NativeFunction(NativeFunction::new("model_create", Some(3), |args| {
                let collection = match &args[0] {
                    Value::String(s) => s.clone(),
                    other => {
                        return Err(format!(
                            "model_create() expects string collection as first argument, got {}",
                            other.type_name()
                        ))
                    }
                };

                let data_value: Result<serde_json::Value, String> = match &args[1] {
                    Value::Hash(hash) => {
                        let mut map = serde_json::Map::new();
                        for (k, v) in hash.borrow().iter() {
                            if let Value::String(key) = k {
                                map.insert(key.clone(), value_to_json(v)?);
                            }
                        }
                        Ok(serde_json::Value::Object(map))
                    }
                    _ => {
                        return Err(format!(
                            "model_create() expects hash data as second argument, got {}",
                            args[1].type_name()
                        ))
                    }
                };
                let data_value = data_value?;

                Ok(spawn_db_future(move || {
                    tokio::runtime::Runtime::new()
                        .map_err(|e| format!("Failed to create async runtime: {}", e))?
                        .block_on(async {
                            let host = "http://localhost:8080".to_string();
                            let database = "default".to_string();
                            let mut client = SoliDBClient::connect(&host)
                                .await
                                .map_err(|e| format!("Failed to connect: {}", e))?;

                            let id = client
                                .insert(&database, &collection, None, data_value)
                                .await
                                .map_err(|e| format!("Create failed: {}", e))?;

                            Ok(serde_json::to_string(&id).unwrap_or_default())
                        })
                }))
            })),
        );

        env.define(
            "model_find".to_string(),
            Value::NativeFunction(NativeFunction::new("model_find", Some(2), |args| {
                let collection = match &args[0] {
                    Value::String(s) => s.clone(),
                    other => {
                        return Err(format!(
                            "model_find() expects string collection as first argument, got {}",
                            other.type_name()
                        ))
                    }
                };

                let id = match &args[1] {
                    Value::String(s) => s.clone(),
                    other => {
                        return Err(format!(
                            "model_find() expects string id as second argument, got {}",
                            other.type_name()
                        ))
                    }
                };

                Ok(spawn_db_future(move || {
                    tokio::runtime::Runtime::new()
                        .map_err(|e| format!("Failed to create async runtime: {}", e))?
                        .block_on(async {
                            let host = "http://localhost:8080".to_string();
                            let database = "default".to_string();
                            let mut client = SoliDBClient::connect(&host)
                                .await
                                .map_err(|e| format!("Failed to connect: {}", e))?;

                            let doc = client
                                .get(&database, &collection, &id)
                                .await
                                .map_err(|e| format!("Get failed: {}", e))?;

                            Ok(serde_json::to_string(&doc).unwrap_or_default())
                        })
                }))
            })),
        );

        env.define(
            "model_where".to_string(),
            Value::NativeFunction(NativeFunction::new("model_where", Some(4), |args| {
                let collection = match &args[0] {
                    Value::String(s) => s.clone(),
                    other => {
                        return Err(format!(
                            "model_where() expects string collection as first argument, got {}",
                            other.type_name()
                        ))
                    }
                };

                let field = match &args[1] {
                    Value::String(s) => s.clone(),
                    other => {
                        return Err(format!(
                            "model_where() expects string field as second argument, got {}",
                            other.type_name()
                        ))
                    }
                };

                let operator = match &args[2] {
                    Value::String(s) => s.clone(),
                    other => {
                        return Err(format!(
                            "model_where() expects string operator as third argument, got {}",
                            other.type_name()
                        ))
                    }
                };

                let value = args[3].clone();
                let json_value = value_to_json(&value)?;

                Ok(spawn_db_future(move || {
                    tokio::runtime::Runtime::new()
                        .map_err(|e| format!("Failed to create async runtime: {}", e))?
                        .block_on(async {
                            let host = "http://localhost:8080".to_string();
                            let database = "default".to_string();
                            let mut client = SoliDBClient::connect(&host)
                                .await
                                .map_err(|e| format!("Failed to connect: {}", e))?;

                            let sdbql = format!(
                                "SELECT * FROM {} WHERE {} {} ?",
                                collection, field, operator
                            );
                            let mut bind_vars = HashMap::new();
                            bind_vars.insert("value".to_string(), json_value);

                            let results = client
                                .query(&database, &sdbql, Some(bind_vars))
                                .await
                                .map_err(|e| format!("Query failed: {}", e))?;

                            Ok(serde_json::to_string(&results).unwrap_or_default())
                        })
                }))
            })),
        );

        env.define(
            "model_get".to_string(),
            Value::NativeFunction(NativeFunction::new("model_get", Some(2), |args| {
                let collection = match &args[0] {
                    Value::String(s) => s.clone(),
                    other => {
                        return Err(format!(
                            "model_get() expects string collection as first argument, got {}",
                            other.type_name()
                        ))
                    }
                };

                let id = match &args[1] {
                    Value::String(s) => s.clone(),
                    other => {
                        return Err(format!(
                            "model_get() expects string id as second argument, got {}",
                            other.type_name()
                        ))
                    }
                };

                Ok(spawn_db_future(move || {
                    tokio::runtime::Runtime::new()
                        .map_err(|e| format!("Failed to create async runtime: {}", e))?
                        .block_on(async {
                            let host = "http://localhost:8080".to_string();
                            let database = "default".to_string();
                            let mut client = SoliDBClient::connect(&host)
                                .await
                                .map_err(|e| format!("Failed to connect: {}", e))?;

                            let doc = client
                                .get(&database, &collection, &id)
                                .await
                                .map_err(|e| format!("Get failed: {}", e))?;

                            Ok(serde_json::to_string(&doc).unwrap_or_default())
                        })
                }))
            })),
        );

        env.define(
            "model_update".to_string(),
            Value::NativeFunction(NativeFunction::new("model_update", Some(3), |args| {
                let collection = match &args[0] {
                    Value::String(s) => s.clone(),
                    other => {
                        return Err(format!(
                            "model_update() expects string collection as first argument, got {}",
                            other.type_name()
                        ))
                    }
                };

                let id = match &args[1] {
                    Value::String(s) => s.clone(),
                    other => {
                        return Err(format!(
                            "model_update() expects string id as second argument, got {}",
                            other.type_name()
                        ))
                    }
                };

                let data_value: Result<serde_json::Value, String> = match &args[2] {
                    Value::Hash(hash) => {
                        let mut map = serde_json::Map::new();
                        for (k, v) in hash.borrow().iter() {
                            if let Value::String(key) = k {
                                map.insert(key.clone(), value_to_json(v)?);
                            }
                        }
                        Ok(serde_json::Value::Object(map))
                    }
                    _ => {
                        return Err(format!(
                            "model_update() expects hash data as third argument, got {}",
                            args[2].type_name()
                        ))
                    }
                };
                let data_value = data_value?;

                Ok(spawn_db_future(move || {
                    tokio::runtime::Runtime::new()
                        .map_err(|e| format!("Failed to create async runtime: {}", e))?
                        .block_on(async {
                            let host = "http://localhost:8080".to_string();
                            let database = "default".to_string();
                            let mut client = SoliDBClient::connect(&host)
                                .await
                                .map_err(|e| format!("Failed to connect: {}", e))?;

                            client
                                .update(&database, &collection, &id, data_value, true)
                                .await
                                .map_err(|e| format!("Update failed: {}", e))?;

                            Ok("Updated".to_string())
                        })
                }))
            })),
        );

        env.define(
            "model_delete".to_string(),
            Value::NativeFunction(NativeFunction::new("model_delete", Some(2), |args| {
                let collection = match &args[0] {
                    Value::String(s) => s.clone(),
                    other => {
                        return Err(format!(
                            "model_delete() expects string collection as first argument, got {}",
                            other.type_name()
                        ))
                    }
                };

                let id = match &args[1] {
                    Value::String(s) => s.clone(),
                    other => {
                        return Err(format!(
                            "model_delete() expects string id as second argument, got {}",
                            other.type_name()
                        ))
                    }
                };

                Ok(spawn_db_future(move || {
                    tokio::runtime::Runtime::new()
                        .map_err(|e| format!("Failed to create async runtime: {}", e))?
                        .block_on(async {
                            let host = "http://localhost:8080".to_string();
                            let database = "default".to_string();
                            let mut client = SoliDBClient::connect(&host)
                                .await
                                .map_err(|e| format!("Failed to connect: {}", e))?;

                            client
                                .delete(&database, &collection, &id)
                                .await
                                .map_err(|e| format!("Delete failed: {}", e))?;

                            Ok("Deleted".to_string())
                        })
                }))
            })),
        );

        env.define(
            "model_count".to_string(),
            Value::NativeFunction(NativeFunction::new("model_count", Some(1), |args| {
                let collection = match &args[0] {
                    Value::String(s) => s.clone(),
                    other => {
                        return Err(format!(
                            "model_count() expects string collection as first argument, got {}",
                            other.type_name()
                        ))
                    }
                };

                Ok(spawn_db_future(move || {
                    tokio::runtime::Runtime::new()
                        .map_err(|e| format!("Failed to create async runtime: {}", e))?
                        .block_on(async {
                            let host = "http://localhost:8080".to_string();
                            let database = "default".to_string();
                            let mut client = SoliDBClient::connect(&host)
                                .await
                                .map_err(|e| format!("Failed to connect: {}", e))?;

                            let sdbql = format!("SELECT COUNT(*) FROM {}", collection);
                            let results = client
                                .query(&database, &sdbql, None)
                                .await
                                .map_err(|e| format!("Query failed: {}", e))?;

                            Ok(serde_json::to_string(&results).unwrap_or_default())
                        })
                }))
            })),
        );
    }

    fn register_field_functions(env: &mut Environment) {
        let mut native_static_methods = HashMap::new();

        native_static_methods.insert(
            "string".to_string(),
            Rc::new(NativeFunction::new("Field.string", Some(1), |_args| {
                Ok(Value::Null)
            })),
        );

        native_static_methods.insert(
            "int".to_string(),
            Rc::new(NativeFunction::new("Field.int", Some(1), |_args| {
                Ok(Value::Null)
            })),
        );

        native_static_methods.insert(
            "float".to_string(),
            Rc::new(NativeFunction::new("Field.float", Some(1), |_args| {
                Ok(Value::Null)
            })),
        );

        native_static_methods.insert(
            "bool".to_string(),
            Rc::new(NativeFunction::new("Field.bool", Some(1), |_args| {
                Ok(Value::Null)
            })),
        );

        native_static_methods.insert(
            "datetime".to_string(),
            Rc::new(NativeFunction::new("Field.datetime", Some(1), |_args| {
                Ok(Value::Null)
            })),
        );

        native_static_methods.insert(
            "reference".to_string(),
            Rc::new(NativeFunction::new("Field.reference", Some(2), |_args| {
                Ok(Value::Null)
            })),
        );

        let field_class = Class {
            name: "Field".to_string(),
            superclass: None,
            methods: HashMap::new(),
            static_methods: HashMap::new(),
            native_static_methods,
            constructor: None,
        };
        env.define("Field".to_string(), Value::Class(Rc::new(field_class)));
    }

    fn register_migration_functions(env: &mut Environment) {
        env.define(
            "Migration".to_string(),
            Value::NativeFunction(NativeFunction::new("Migration", Some(3), |_args| {
                println!("Migration registered");
                Ok(Value::Null)
            })),
        );

        env.define(
            "model_migrate".to_string(),
            Value::NativeFunction(NativeFunction::new("model_migrate", None, |_args| {
                println!("Running migrations...");
                Ok(Value::Null)
            })),
        );

        env.define(
            "model_rollback".to_string(),
            Value::NativeFunction(NativeFunction::new("model_rollback", None, |_args| {
                println!("Rolling back last migration...");
                Ok(Value::Null)
            })),
        );

        env.define(
            "model_status".to_string(),
            Value::NativeFunction(NativeFunction::new("model_status", None, |_args| {
                println!("Migration status:");
                Ok(Value::Null)
            })),
        );
    }
}

pub fn register_model_builtins(env: &mut Environment) {
    Model::register_builtins(env);
}

fn value_to_json(value: &Value) -> Result<serde_json::Value, String> {
    match value {
        Value::Int(n) => Ok(serde_json::Value::Number(serde_json::Number::from(*n))),
        Value::Float(f) => Ok(serde_json::Value::Number(
            serde_json::Number::from_f64(*f).ok_or_else(|| "Invalid float".to_string())?,
        )),
        Value::String(s) => Ok(serde_json::Value::String(s.clone())),
        Value::Bool(b) => Ok(serde_json::Value::Bool(*b)),
        Value::Null => Ok(serde_json::Value::Null),
        Value::Array(arr) => {
            let borrow = arr.borrow();
            let vec: Result<Vec<serde_json::Value>, String> = borrow
                .iter()
                .map(|v| value_to_json(v))
                .collect();
            vec.map(serde_json::Value::Array)
        }
        Value::Hash(hash) => {
            let borrow = hash.borrow();
            let mut map = serde_json::Map::new();
            for (k, v) in borrow.iter() {
                if let Value::String(key) = k {
                    map.insert(key.clone(), value_to_json(v)?);
                }
            }
            Ok(serde_json::Value::Object(map))
        }
        _ => Err(format!("Cannot convert {} to JSON", value.type_name())),
    }
}
