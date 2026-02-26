//! Core Model types, registry, and database configuration.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{OnceLock, RwLock};

use lazy_static::lazy_static;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, NativeFunction, Value};

use super::callbacks::{register_callback, ModelCallbacks};
use super::validation::{register_validation, ValidationRule};

/// Metadata for a model class (validations, callbacks).
#[derive(Debug, Clone, Default)]
pub struct ModelMetadata {
    pub validations: Vec<ValidationRule>,
    pub callbacks: ModelCallbacks,
}

/// Cached database configuration to avoid repeated env::var() lookups.
/// Note: api_key and database are read at request time so .env can be loaded later.
pub struct DbConfig {
    pub host: String,
}

impl DbConfig {
    fn from_env() -> Self {
        let host =
            std::env::var("SOLIDB_HOST").unwrap_or_else(|_| "http://localhost:6745".to_string());
        let host = host
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .to_string();
        Self { host }
    }
}

lazy_static! {
    /// Global registry mapping class names to their metadata.
    pub static ref MODEL_REGISTRY: RwLock<HashMap<String, ModelMetadata>> =
        RwLock::new(HashMap::new());

    /// Cached DB configuration (for username/password which are less likely to change).
    pub static ref DB_CONFIG: DbConfig = DbConfig::from_env();
}

/// Cached DB config - initialized on first use.
static CACHED_DB_CONFIG: OnceLock<CachedDbConfig> = OnceLock::new();

struct CachedDbConfig {
    cursor_url: String,
    database: String,
    api_key: Option<String>,
    basic_auth: Option<String>,
}

/// Initialize DB config from environment - call this after .env is loaded.
pub fn init_db_config() {
    let _ = get_db_config();
}

/// Get cached DB config - initialized on first call.
fn get_db_config() -> &'static CachedDbConfig {
    CACHED_DB_CONFIG.get_or_init(|| {
        let host =
            std::env::var("SOLIDB_HOST").unwrap_or_else(|_| "http://localhost:6745".to_string());
        let host = host
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .to_string();
        let database = std::env::var("SOLIDB_DATABASE").unwrap_or_else(|_| "default".to_string());
        let cursor_url = format!("http://{}/_api/database/{}/cursor", host, database);
        let api_key = std::env::var("SOLIDB_API_KEY").ok();
        let basic_auth = match (
            std::env::var("SOLIDB_USERNAME").ok(),
            std::env::var("SOLIDB_PASSWORD").ok(),
        ) {
            (Some(u), Some(p)) => {
                use base64::Engine;
                Some(format!(
                    "Basic {}",
                    base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", u, p))
                ))
            }
            _ => None,
        };
        CachedDbConfig {
            cursor_url,
            database,
            api_key,
            basic_auth,
        }
    })
}

/// Get database name.
pub fn get_database_name() -> &'static str {
    &get_db_config().database
}

/// Get cursor URL.
pub fn get_cursor_url() -> &'static str {
    &get_db_config().cursor_url
}

/// Get API key.
pub fn get_api_key() -> Option<&'static str> {
    get_db_config().api_key.as_deref()
}

/// Get Basic auth header value.
pub fn get_basic_auth() -> Option<&'static str> {
    get_db_config().basic_auth.as_deref()
}

/// Get or create metadata for a model class.
pub fn get_or_create_metadata(class_name: &str) -> ModelMetadata {
    let registry = MODEL_REGISTRY.read().unwrap();
    registry.get(class_name).cloned().unwrap_or_default()
}

/// Update metadata for a model class.
pub fn update_metadata(class_name: &str, metadata: ModelMetadata) {
    let mut registry = MODEL_REGISTRY.write().unwrap();
    registry.insert(class_name.to_string(), metadata);
}

/// Convert PascalCase class name to snake_case collection name with pluralization.
/// Examples:
/// - "User" → "users"
/// - "BlogPost" → "blog_posts"
/// - "UserProfile" → "user_profiles"
/// - "CustomerModel" → "customers" (strips _model suffix before pluralizing)
pub fn class_name_to_collection(name: &str) -> String {
    // Strip _model suffix if present for cleaner collection names
    let name = name.strip_suffix("Model").unwrap_or(name);

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

/// Extract collection name from the first argument (the Class).
pub fn get_collection_from_class(args: &[Value]) -> Result<String, String> {
    match args.first() {
        Some(Value::Class(class)) => Ok(class_name_to_collection(&class.name)),
        Some(other) => Err(format!(
            "Expected class as first argument, got {}",
            other.type_name()
        )),
        None => Err("Missing class argument".to_string()),
    }
}

/// Extract class name from the first argument (the Class).
pub fn get_class_name_from_class(args: &[Value]) -> Result<String, String> {
    match args.first() {
        Some(Value::Class(class)) => Ok(class.name.clone()),
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

        // Direct query function for benchmarking (bypasses all class dispatch)
        use super::crud::exec_async_query_raw;
        env.define(
            "db_query_raw".to_string(),
            Value::NativeFunction(NativeFunction::new("db_query_raw", Some(1), |args| {
                let query = match args.first() {
                    Some(Value::String(s)) => s.clone(),
                    _ => return Err("db_query_raw requires a query string".to_string()),
                };
                Ok(exec_async_query_raw(query))
            })),
        );

        // Debug: show the cursor URL
        env.define(
            "db_cursor_url".to_string(),
            Value::NativeFunction(NativeFunction::new("db_cursor_url", Some(0), |_args| {
                Ok(Value::String(get_cursor_url().to_string()))
            })),
        );

        // Test function with hardcoded values - mirrors HTTP.request exactly
        use super::crud::exec_query_hardcoded;
        env.define(
            "db_query_hardcoded".to_string(),
            Value::NativeFunction(NativeFunction::new("db_query_hardcoded", Some(1), |args| {
                let query = match args.first() {
                    Some(Value::String(s)) => s.clone(),
                    _ => return Err("db_query_hardcoded requires a query string".to_string()),
                };
                Ok(exec_query_hardcoded(query))
            })),
        );
    }

    fn register_model_class(env: &mut Environment) {
        let mut native_static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

        // ====================================================================
        // Validation & Callback Registration Methods
        // ====================================================================

        // validates(field, options) - Register validation rules
        native_static_methods.insert(
            "validates".to_string(),
            Rc::new(NativeFunction::new("Model.validates", Some(3), |args| {
                let class_name = get_class_name_from_class(&args)?;

                let field = match args.get(1) {
                    Some(Value::String(s)) => s.clone(),
                    Some(other) => {
                        return Err(format!(
                            "validates() expects string field name, got {}",
                            other.type_name()
                        ))
                    }
                    None => return Err("validates() requires field argument".to_string()),
                };

                let options = match args.get(2) {
                    Some(Value::Hash(hash)) => hash.borrow().clone(),
                    Some(other) => {
                        return Err(format!(
                            "validates() expects hash options, got {}",
                            other.type_name()
                        ))
                    }
                    None => return Err("validates() requires options argument".to_string()),
                };

                let mut rule = ValidationRule::new(field);

                // Parse options
                use crate::interpreter::value::HashKey;
                for (key, value) in options {
                    if let HashKey::String(key_str) = key {
                        match key_str.as_str() {
                            "presence" => {
                                if let Value::Bool(b) = value {
                                    rule.presence = b;
                                }
                            }
                            "uniqueness" => {
                                if let Value::Bool(b) = value {
                                    rule.uniqueness = b;
                                }
                            }
                            "min_length" => {
                                if let Value::Int(n) = value {
                                    rule.min_length = Some(n as usize);
                                }
                            }
                            "max_length" => {
                                if let Value::Int(n) = value {
                                    rule.max_length = Some(n as usize);
                                }
                            }
                            "format" => {
                                if let Value::String(s) = value {
                                    rule.format = Some(s);
                                }
                            }
                            "numericality" => {
                                if let Value::Bool(b) = value {
                                    rule.numericality = b;
                                }
                            }
                            "min" => match value {
                                Value::Int(n) => rule.min = Some(n as f64),
                                Value::Float(n) => rule.min = Some(n),
                                _ => {}
                            },
                            "max" => match value {
                                Value::Int(n) => rule.max = Some(n as f64),
                                Value::Float(n) => rule.max = Some(n),
                                _ => {}
                            },
                            "custom" => {
                                if let Value::String(s) = value {
                                    rule.custom = Some(s);
                                }
                            }
                            _ => {}
                        }
                    }
                }

                register_validation(&class_name, rule);
                Ok(Value::Null)
            })),
        );

        // Callback registration methods
        for callback_type in &[
            "before_save",
            "after_save",
            "before_create",
            "after_create",
            "before_update",
            "after_update",
            "before_delete",
            "after_delete",
        ] {
            let callback_name = callback_type.to_string();
            let method_name = format!("Model.{}", callback_type);
            native_static_methods.insert(
                callback_name.clone(),
                Rc::new(NativeFunction::new(&method_name, Some(2), move |args| {
                    let class_name = get_class_name_from_class(&args)?;
                    let method_name = match args.get(1) {
                        Some(Value::String(s)) => s.clone(),
                        Some(other) => {
                            return Err(format!(
                                "{}() expects string method name, got {}",
                                callback_name,
                                other.type_name()
                            ))
                        }
                        None => {
                            return Err(format!(
                                "{}() requires method name argument",
                                callback_name
                            ))
                        }
                    };
                    register_callback(&class_name, &callback_name, &method_name);
                    Ok(Value::Null)
                })),
            );
        }

        // ====================================================================
        // CRUD Methods
        // ====================================================================

        // Model.create(data) - Insert document with validation
        use super::crud::{exec_insert, json_to_value};
        use super::validation::{build_validation_result, run_validations};
        use crate::interpreter::value::value_to_json;
        use crate::interpreter::value::HashKey;
        native_static_methods.insert(
            "create".to_string(),
            Rc::new(NativeFunction::new("Model.create", Some(2), |args| {
                let class_name = get_class_name_from_class(&args)?;
                let collection = class_name_to_collection(&class_name);

                let data = args
                    .get(1)
                    .cloned()
                    .ok_or_else(|| "Model.create() requires data argument".to_string())?;

                // Run validations
                let errors = run_validations(&class_name, &data, true);
                if !errors.is_empty() {
                    return Ok(build_validation_result(false, errors, None));
                }

                let data_value: Result<serde_json::Value, String> = match &data {
                    Value::Hash(hash) => {
                        let mut map = serde_json::Map::new();
                        for (k, v) in hash.borrow().iter() {
                            if let HashKey::String(key) = k {
                                map.insert(key.clone(), value_to_json(v)?);
                            }
                        }
                        Ok(serde_json::Value::Object(map))
                    }
                    other => Err(format!(
                        "Model.create() expects hash data, got {}",
                        other.type_name()
                    )),
                };
                let data_value = data_value?;

                let result = exec_insert(&collection, None, data_value.clone());

                match result {
                    Ok(id) => {
                        let mut result_map = serde_json::Map::new();
                        result_map.insert("valid".to_string(), serde_json::Value::Bool(true));

                        if let serde_json::Value::Object(mut data_map) = data_value {
                            data_map.insert("id".to_string(), id);
                            result_map
                                .insert("record".to_string(), serde_json::Value::Object(data_map));
                        }

                        Ok(Value::Hash(Rc::new(RefCell::new(
                            result_map
                                .into_iter()
                                .map(|(k, v)| (HashKey::String(k), json_to_value(&v)))
                                .collect(),
                        ))))
                    }
                    Err(e) => Ok(Value::String(format!("Error: {}", e))),
                }
            })),
        );

        // Model.find(id) - Get by ID
        use super::crud::exec_get;
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

                match exec_get(&collection, &id) {
                    Ok(doc) => Ok(json_to_value(&doc)),
                    Err(e) => Ok(Value::String(format!("Error: {}", e))),
                }
            })),
        );

        // Model.where(filter, bind_vars) - Returns a QueryBuilder for chaining
        use super::query::QueryBuilder;
        use std::collections::HashMap as StdHashMap;
        native_static_methods.insert(
            "where".to_string(),
            Rc::new(NativeFunction::new("Model.where", Some(3), |args| {
                let class_name = get_class_name_from_class(&args)?;
                let collection = class_name_to_collection(&class_name);

                let filter = match args.get(1) {
                    Some(Value::String(s)) => s.clone(),
                    Some(other) => {
                        return Err(format!(
                            "Model.where() expects string filter expression, got {}",
                            other.type_name()
                        ))
                    }
                    None => return Err("Model.where() requires filter expression".to_string()),
                };

                let bind_vars = match args.get(2) {
                    Some(Value::Hash(hash)) => {
                        let mut map = StdHashMap::new();
                        for (k, v) in hash.borrow().iter() {
                            if let HashKey::String(key) = k {
                                map.insert(key.clone(), value_to_json(v)?);
                            }
                        }
                        map
                    }
                    Some(other) => {
                        return Err(format!(
                            "Model.where() expects hash for bind variables, got {}",
                            other.type_name()
                        ))
                    }
                    None => return Err("Model.where() requires bind variables hash".to_string()),
                };

                // Create a QueryBuilder and set the filter
                let mut qb = QueryBuilder::new(class_name, collection);
                qb.set_filter(filter, bind_vars);

                Ok(Value::QueryBuilder(Rc::new(RefCell::new(qb))))
            })),
        );

        // Model.all() - Get all documents (uses async HTTP for high performance)
        use super::crud::exec_auto_collection;
        native_static_methods.insert(
            "all".to_string(),
            Rc::new(NativeFunction::new("Model.all", Some(1), |args| {
                let collection = get_collection_from_class(&args)?;
                let sdbql = format!("FOR doc IN {} RETURN doc", collection);
                Ok(exec_auto_collection(sdbql, &collection))
            })),
        );

        // Model.all_json() - Get all documents as raw JSON string (fastest)
        use super::crud::exec_async_query_raw;
        native_static_methods.insert(
            "all_json".to_string(),
            Rc::new(NativeFunction::new("Model.all_json", Some(1), |args| {
                let collection = get_collection_from_class(&args)?;
                let sdbql = format!("FOR doc IN {} RETURN doc", collection);
                Ok(exec_async_query_raw(sdbql))
            })),
        );

        // Model.update(id, data) - Update document
        use super::crud::exec_update;
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
                            if let HashKey::String(key) = k {
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

                match exec_update(&collection, &id, data_value, true) {
                    Ok(result) => Ok(json_to_value(&result)),
                    Err(e) => Ok(Value::String(format!("Error: {}", e))),
                }
            })),
        );

        // Model.delete(id) - Delete document
        use super::crud::exec_delete;
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

                match exec_delete(&collection, &id) {
                    Ok(result) => Ok(json_to_value(&result)),
                    Err(e) => Ok(Value::String(format!("Error: {}", e))),
                }
            })),
        );

        // Model.count() - Count documents
        use super::crud::exec_query;
        native_static_methods.insert(
            "count".to_string(),
            Rc::new(NativeFunction::new("Model.count", Some(1), |args| {
                let collection = get_collection_from_class(&args)?;

                let sdbql = format!(
                    "FOR doc IN {} COLLECT WITH COUNT INTO count RETURN count",
                    collection
                );

                match exec_query(&collection, sdbql) {
                    Ok(results) => Ok(Value::Array(Rc::new(RefCell::new(
                        results.iter().map(json_to_value).collect(),
                    )))),
                    Err(e) => Ok(Value::String(format!("Error: {}", e))),
                }
            })),
        );

        let model_class = Class {
            name: "Model".to_string(),
            superclass: None,
            methods: HashMap::new(),
            static_methods: HashMap::new(),
            native_static_methods,
            native_methods: HashMap::new(),
            static_fields: Rc::new(RefCell::new(HashMap::new())),
            fields: HashMap::new(),
            constructor: None,
            nested_classes: Rc::new(RefCell::new(HashMap::new())),
            ..Default::default()
        };
        env.define("Model".to_string(), Value::Class(Rc::new(model_class)));
    }
}

pub fn register_model_builtins(env: &mut Environment) {
    Model::register_builtins(env);

    // Register global wrapper functions for class-level DSL
    // These functions expect the class as the first argument (passed by execute_class)

    // validates(class, field, options) - Register validation rules
    use crate::interpreter::value::HashKey;
    env.define(
        "validates".to_string(),
        Value::NativeFunction(NativeFunction::new("validates", Some(3), |args| {
            let class_name = get_class_name_from_class(&args)?;

            let field = match args.get(1) {
                Some(Value::String(s)) => s.clone(),
                Some(other) => {
                    return Err(format!(
                        "validates() expects string field name, got {}",
                        other.type_name()
                    ))
                }
                None => return Err("validates() requires field argument".to_string()),
            };

            let options = match args.get(2) {
                Some(Value::Hash(hash)) => hash.borrow().clone(),
                Some(other) => {
                    return Err(format!(
                        "validates() expects hash options, got {}",
                        other.type_name()
                    ))
                }
                None => return Err("validates() requires options argument".to_string()),
            };

            let mut rule = ValidationRule::new(field);

            // Parse options
            for (key, value) in options {
                if let HashKey::String(key_str) = key {
                    match key_str.as_str() {
                        "presence" => {
                            if let Value::Bool(b) = value {
                                rule.presence = b;
                            }
                        }
                        "uniqueness" => {
                            if let Value::Bool(b) = value {
                                rule.uniqueness = b;
                            }
                        }
                        "min_length" => {
                            if let Value::Int(n) = value {
                                rule.min_length = Some(n as usize);
                            }
                        }
                        "max_length" => {
                            if let Value::Int(n) = value {
                                rule.max_length = Some(n as usize);
                            }
                        }
                        "format" => {
                            if let Value::String(s) = value {
                                rule.format = Some(s);
                            }
                        }
                        "numericality" => {
                            if let Value::Bool(b) = value {
                                rule.numericality = b;
                            }
                        }
                        "min" => match value {
                            Value::Int(n) => rule.min = Some(n as f64),
                            Value::Float(n) => rule.min = Some(n),
                            _ => {}
                        },
                        "max" => match value {
                            Value::Int(n) => rule.max = Some(n as f64),
                            Value::Float(n) => rule.max = Some(n),
                            _ => {}
                        },
                        "custom" => {
                            if let Value::String(s) = value {
                                rule.custom = Some(s);
                            }
                        }
                        _ => {}
                    }
                }
            }

            register_validation(&class_name, rule);
            Ok(Value::Null)
        })),
    );

    // Callback registration global functions
    for callback_type in &[
        "before_save",
        "after_save",
        "before_create",
        "after_create",
        "before_update",
        "after_update",
        "before_delete",
        "after_delete",
    ] {
        let callback_name = callback_type.to_string();
        let callback_name_for_fn = callback_name.clone();
        let callback_name_for_closure = callback_name.clone();
        env.define(
            callback_name,
            Value::NativeFunction(NativeFunction::new(
                &callback_name_for_fn,
                Some(2),
                move |args| {
                    let class_name = get_class_name_from_class(&args)?;
                    let method_name = match args.get(1) {
                        Some(Value::String(s)) => s.clone(),
                        Some(other) => {
                            return Err(format!(
                                "{}() expects string method name, got {}",
                                callback_name_for_closure,
                                other.type_name()
                            ))
                        }
                        None => {
                            return Err(format!(
                                "{}() requires method name argument",
                                callback_name_for_closure
                            ))
                        }
                    };
                    register_callback(&class_name, &callback_name_for_closure, &method_name);
                    Ok(Value::Null)
                },
            )),
        );
    }
}
