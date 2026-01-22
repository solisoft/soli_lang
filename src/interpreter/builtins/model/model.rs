//! Simplified OOP Model system for SoliLang.
//!
//! Collection name is auto-derived from the class name:
//! - `User` → `"users"`
//! - `BlogPost` → `"blog_posts"`
//!
//! # Example Usage
//!
//! ```soli
//! class User extends Model { }
//!
//! let user = User.create({ "name": "Alice" });
//! let found = User.find(user.id);
//! let adults = User.where("age", ">=", 18);
//! let all = User.all();
//! User.update(user.id, { "name": "Bob" });
//! User.delete(user.id);
//! ```

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

use solidb_client::SoliDBClient;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, FutureState, HttpFutureKind, NativeFunction, Value};

/// Convert PascalCase class name to snake_case collection name with pluralization.
/// Examples:
/// - "User" → "users"
/// - "BlogPost" → "blog_posts"
/// - "UserProfile" → "user_profiles"
fn class_name_to_collection(name: &str) -> String {
    let mut result = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            result.push('_');
        }
        result.push(c.to_lowercase().next().unwrap());
    }
    result.push('s'); // simple pluralization
    result
}

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
            let vec: Result<Vec<serde_json::Value>, String> =
                borrow.iter().map(|v| value_to_json(v)).collect();
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

/// Extract collection name from the first argument (the Class).
fn get_collection_from_class(args: &[Value]) -> Result<String, String> {
    match args.first() {
        Some(Value::Class(class)) => Ok(class_name_to_collection(&class.name)),
        Some(other) => Err(format!(
            "Expected class as first argument, got {}",
            other.type_name()
        )),
        None => Err("Missing class argument".to_string()),
    }
}

pub struct Model;

impl Model {
    pub fn register_builtins(env: &mut Environment) {
        Self::register_model_class(env);
    }

    fn register_model_class(env: &mut Environment) {
        let mut native_static_methods = HashMap::new();

        // Model.create(data) - Insert document
        native_static_methods.insert(
            "create".to_string(),
            Rc::new(NativeFunction::new("Model.create", Some(2), |args| {
                let collection = get_collection_from_class(&args)?;

                let data_value: Result<serde_json::Value, String> = match args.get(1) {
                    Some(Value::Hash(hash)) => {
                        let mut map = serde_json::Map::new();
                        for (k, v) in hash.borrow().iter() {
                            if let Value::String(key) = k {
                                map.insert(key.clone(), value_to_json(v)?);
                            }
                        }
                        Ok(serde_json::Value::Object(map))
                    }
                    Some(other) => Err(format!(
                        "Model.create() expects hash data, got {}",
                        other.type_name()
                    )),
                    None => Err("Model.create() requires data argument".to_string()),
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

        // Model.find(id) - Get by ID
        native_static_methods.insert(
            "find".to_string(),
            Rc::new(NativeFunction::new("Model.find", Some(2), |args| {
                let collection = get_collection_from_class(&args)?;

                let id = match args.get(1) {
                    Some(Value::String(s)) => s.clone(),
                    Some(other) => {
                        return Err(format!(
                            "Model.find() expects string id, got {}",
                            other.type_name()
                        ))
                    }
                    None => return Err("Model.find() requires id argument".to_string()),
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
                                .map_err(|e| format!("Find failed: {}", e))?;

                            Ok(serde_json::to_string(&doc).unwrap_or_default())
                        })
                }))
            })),
        );

        // Model.where(field, op, value) - Query with filter
        native_static_methods.insert(
            "where".to_string(),
            Rc::new(NativeFunction::new("Model.where", Some(4), |args| {
                let collection = get_collection_from_class(&args)?;

                let field = match args.get(1) {
                    Some(Value::String(s)) => s.clone(),
                    Some(other) => {
                        return Err(format!(
                            "Model.where() expects string field, got {}",
                            other.type_name()
                        ))
                    }
                    None => return Err("Model.where() requires field argument".to_string()),
                };

                let operator = match args.get(2) {
                    Some(Value::String(s)) => s.clone(),
                    Some(other) => {
                        return Err(format!(
                            "Model.where() expects string operator, got {}",
                            other.type_name()
                        ))
                    }
                    None => return Err("Model.where() requires operator argument".to_string()),
                };

                let value = args
                    .get(3)
                    .ok_or_else(|| "Model.where() requires value argument".to_string())?
                    .clone();
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

        // Model.all() - Get all documents
        native_static_methods.insert(
            "all".to_string(),
            Rc::new(NativeFunction::new("Model.all", Some(1), |args| {
                let collection = get_collection_from_class(&args)?;

                Ok(spawn_db_future(move || {
                    tokio::runtime::Runtime::new()
                        .map_err(|e| format!("Failed to create async runtime: {}", e))?
                        .block_on(async {
                            let host = "http://localhost:8080".to_string();
                            let database = "default".to_string();
                            let mut client = SoliDBClient::connect(&host)
                                .await
                                .map_err(|e| format!("Failed to connect: {}", e))?;

                            let sdbql = format!("SELECT * FROM {}", collection);
                            let results = client
                                .query(&database, &sdbql, None)
                                .await
                                .map_err(|e| format!("Query failed: {}", e))?;

                            Ok(serde_json::to_string(&results).unwrap_or_default())
                        })
                }))
            })),
        );

        // Model.update(id, data) - Update document
        native_static_methods.insert(
            "update".to_string(),
            Rc::new(NativeFunction::new("Model.update", Some(3), |args| {
                let collection = get_collection_from_class(&args)?;

                let id = match args.get(1) {
                    Some(Value::String(s)) => s.clone(),
                    Some(other) => {
                        return Err(format!(
                            "Model.update() expects string id, got {}",
                            other.type_name()
                        ))
                    }
                    None => return Err("Model.update() requires id argument".to_string()),
                };

                let data_value: Result<serde_json::Value, String> = match args.get(2) {
                    Some(Value::Hash(hash)) => {
                        let mut map = serde_json::Map::new();
                        for (k, v) in hash.borrow().iter() {
                            if let Value::String(key) = k {
                                map.insert(key.clone(), value_to_json(v)?);
                            }
                        }
                        Ok(serde_json::Value::Object(map))
                    }
                    Some(other) => Err(format!(
                        "Model.update() expects hash data, got {}",
                        other.type_name()
                    )),
                    None => Err("Model.update() requires data argument".to_string()),
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

        // Model.delete(id) - Delete document
        native_static_methods.insert(
            "delete".to_string(),
            Rc::new(NativeFunction::new("Model.delete", Some(2), |args| {
                let collection = get_collection_from_class(&args)?;

                let id = match args.get(1) {
                    Some(Value::String(s)) => s.clone(),
                    Some(other) => {
                        return Err(format!(
                            "Model.delete() expects string id, got {}",
                            other.type_name()
                        ))
                    }
                    None => return Err("Model.delete() requires id argument".to_string()),
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

        // Model.count() - Count documents
        native_static_methods.insert(
            "count".to_string(),
            Rc::new(NativeFunction::new("Model.count", Some(1), |args| {
                let collection = get_collection_from_class(&args)?;

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

        let model_class = Class {
            name: "Model".to_string(),
            superclass: None,
            methods: HashMap::new(),
            static_methods: HashMap::new(),
            native_static_methods,
            native_methods: HashMap::new(),
            constructor: None,
        };
        env.define("Model".to_string(), Value::Class(Rc::new(model_class)));
    }
}

pub fn register_model_builtins(env: &mut Environment) {
    Model::register_builtins(env);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_class_name_to_collection() {
        assert_eq!(class_name_to_collection("User"), "users");
        assert_eq!(class_name_to_collection("BlogPost"), "blog_posts");
        assert_eq!(class_name_to_collection("UserProfile"), "user_profiles");
        assert_eq!(class_name_to_collection("A"), "as");
        assert_eq!(class_name_to_collection("ABC"), "a_b_cs");
    }
}
