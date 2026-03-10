//! Core Model types, registry, and database configuration.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{OnceLock, RwLock};

use lazy_static::lazy_static;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, NativeFunction, Value};

use super::callbacks::{register_callback, ModelCallbacks};
use super::relations::{
    build_relation, get_relation, register_relation, RelationDef, RelationType,
};
use super::validation::{register_validation, ValidationRule};

/// Metadata for a model class (validations, callbacks).
#[derive(Debug, Clone, Default)]
pub struct ModelMetadata {
    pub validations: Vec<ValidationRule>,
    pub callbacks: ModelCallbacks,
    pub relations: Vec<RelationDef>,
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

/// Cached JWT token obtained by logging in with Basic auth credentials.
/// This avoids Argon2 password verification on every DB request.
static CACHED_JWT: OnceLock<Option<String>> = OnceLock::new();

/// Initialize JWT token by logging in to SoliDB. Call this at startup (outside tokio).
pub fn init_jwt_token() {
    let _ = get_jwt_token();
}

/// Get cached JWT token, logging in with Basic auth credentials (via ureq, no tokio).
/// JWT is faster than both API key and Basic auth for subsequent requests.
pub fn get_jwt_token() -> Option<&'static str> {
    CACHED_JWT
        .get_or_init(|| {
            let (username, password) = match (
                std::env::var("SOLIDB_USERNAME").ok(),
                std::env::var("SOLIDB_PASSWORD").ok(),
            ) {
                (Some(u), Some(p)) => (u, p),
                _ => return None,
            };
            let host = std::env::var("SOLIDB_HOST")
                .unwrap_or_else(|_| "http://localhost:6745".to_string());
            let login_url = format!("{}/auth/login", host);
            let payload = serde_json::json!({
                "username": username,
                "password": password,
            });
            // Use ureq (synchronous) to avoid tokio runtime conflicts
            match ureq::post(&login_url)
                .set("Content-Type", "application/json")
                .send_string(&payload.to_string())
            {
                Ok(resp) => match resp.into_string() {
                    Ok(body) => match serde_json::from_str::<serde_json::Value>(&body) {
                        Ok(json) => json
                            .get("token")
                            .and_then(|t| t.as_str())
                            .map(|t| t.to_string()),
                        Err(_) => None,
                    },
                    Err(_) => None,
                },
                Err(e) => {
                    eprintln!("Warning: JWT login failed ({}), falling back", e);
                    None
                }
            }
        })
        .as_deref()
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

/// Extract Rc<Class> from the first argument.
fn get_class_rc_from_args(args: &[Value]) -> Result<Rc<Class>, String> {
    match args.first() {
        Some(Value::Class(class)) => Ok(class.clone()),
        _ => Err("Expected class as first argument".to_string()),
    }
}

/// Convert instance fields to a Value::Hash suitable for validation.
fn instance_fields_to_hash(
    inst: &std::cell::Ref<'_, crate::interpreter::value::Instance>,
) -> Value {
    use crate::interpreter::value::{HashKey, HashPairs};
    let mut pairs = HashPairs::default();
    for (k, v) in &inst.fields {
        if !k.starts_with('_') {
            pairs.insert(HashKey::String(k.clone()), v.clone());
        }
    }
    Value::Hash(Rc::new(RefCell::new(pairs)))
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
        // Relation DSL Methods
        // ====================================================================

        // has_many(name) or has_many(name, options)
        for (rel_method, rel_type) in &[
            ("has_many", RelationType::HasMany),
            ("has_one", RelationType::HasOne),
            ("belongs_to", RelationType::BelongsTo),
        ] {
            let method_label = format!("Model.{}", rel_method);
            let rel_type = rel_type.clone();
            native_static_methods.insert(
                rel_method.to_string(),
                Rc::new(NativeFunction::new(&method_label, None, move |args| {
                    let class_name = get_class_name_from_class(&args)?;
                    let name = match args.get(1) {
                        Some(Value::String(s)) => s.clone(),
                        Some(other) => {
                            return Err(format!(
                                "relation expects string name, got {}",
                                other.type_name()
                            ))
                        }
                        None => return Err("relation requires a name argument".to_string()),
                    };

                    // Optional config hash for overrides
                    let mut class_override: Option<String> = None;
                    let mut fk_override: Option<String> = None;
                    if let Some(Value::Hash(hash)) = args.get(2) {
                        use crate::interpreter::value::HashKey;
                        for (k, v) in hash.borrow().iter() {
                            if let HashKey::String(key) = k {
                                match key.as_str() {
                                    "class_name" => {
                                        if let Value::String(s) = v {
                                            class_override = Some(s.clone());
                                        }
                                    }
                                    "foreign_key" => {
                                        if let Value::String(s) = v {
                                            fk_override = Some(s.clone());
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }

                    let relation = build_relation(
                        &class_name,
                        &name,
                        rel_type.clone(),
                        class_override.as_deref(),
                        fk_override.as_deref(),
                    );
                    register_relation(&class_name, relation);
                    Ok(Value::Null)
                })),
            );
        }

        // ====================================================================
        // Query Chain Starters: includes, join
        // ====================================================================

        // Model.includes("posts", "profile") - eager load relations
        use super::query::QueryBuilder;
        use crate::interpreter::value::HashKey;
        native_static_methods.insert(
            "includes".to_string(),
            Rc::new(NativeFunction::new("Model.includes", None, |args| {
                let class = get_class_rc_from_args(&args)?;
                let class_name = class.name.clone();
                let collection = class_name_to_collection(&class_name);

                let mut qb = QueryBuilder::new_with_class(class_name.clone(), collection, class);
                let arguments = &args[1..];

                if arguments.len() == 1 && matches!(&arguments[0], Value::Hash(_)) {
                    // Pattern B: hash arg → { "posts": ["title", "body"] }
                    if let Value::Hash(hash) = &arguments[0] {
                        for (k, v) in hash.borrow().iter() {
                            let rel_name = match k {
                                HashKey::String(s) => s.clone(),
                                _ => continue,
                            };
                            let rel = get_relation(&class_name, &rel_name).ok_or_else(|| {
                                format!("No relation '{}' defined on {}", rel_name, class_name)
                            })?;
                            let fields = match v {
                                Value::Array(arr) => {
                                    let names: Vec<String> = arr
                                        .borrow()
                                        .iter()
                                        .filter_map(|v| {
                                            if let Value::String(s) = v {
                                                Some(s.clone())
                                            } else {
                                                None
                                            }
                                        })
                                        .collect();
                                    if names.is_empty() {
                                        None
                                    } else {
                                        Some(names)
                                    }
                                }
                                _ => None,
                            };
                            qb.add_include(
                                rel_name,
                                rel,
                                None,
                                std::collections::HashMap::new(),
                                fields,
                            );
                        }
                    }
                } else if arguments.len() >= 2 && matches!(arguments.last(), Some(Value::Hash(_))) {
                    // Pattern C: filtered include
                    let rel_name = match &arguments[0] {
                        Value::String(s) => s.clone(),
                        other => {
                            return Err(format!(
                                "includes() expects string relation name, got {}",
                                other.type_name()
                            ))
                        }
                    };
                    let rel = get_relation(&class_name, &rel_name).ok_or_else(|| {
                        format!("No relation '{}' defined on {}", rel_name, class_name)
                    })?;

                    let filter = if arguments.len() >= 3 {
                        match &arguments[1] {
                            Value::String(s) => Some(s.clone()),
                            _ => None,
                        }
                    } else {
                        None
                    };

                    let options_hash = match arguments.last() {
                        Some(Value::Hash(h)) => h.borrow(),
                        _ => unreachable!(),
                    };

                    let mut bind_vars = std::collections::HashMap::new();
                    let mut fields: Option<Vec<String>> = None;

                    for (k, v) in options_hash.iter() {
                        if let HashKey::String(key) = k {
                            if key == "fields" {
                                if let Value::Array(arr) = v {
                                    let names: Vec<String> = arr
                                        .borrow()
                                        .iter()
                                        .filter_map(|v| {
                                            if let Value::String(s) = v {
                                                Some(s.clone())
                                            } else {
                                                None
                                            }
                                        })
                                        .collect();
                                    if !names.is_empty() {
                                        fields = Some(names);
                                    }
                                }
                            } else {
                                bind_vars.insert(
                                    key.clone(),
                                    crate::interpreter::value::value_to_json(v)?,
                                );
                            }
                        }
                    }

                    qb.add_include(rel_name, rel, filter, bind_vars, fields);
                } else {
                    // Pattern A: all strings → multi-relation unfiltered
                    for arg in arguments {
                        let rel_name = match arg {
                            Value::String(s) => s.clone(),
                            other => {
                                return Err(format!(
                                    "includes() expects string relation names, got {}",
                                    other.type_name()
                                ))
                            }
                        };
                        let rel = get_relation(&class_name, &rel_name).ok_or_else(|| {
                            format!("No relation '{}' defined on {}", rel_name, class_name)
                        })?;
                        qb.add_include(rel_name, rel, None, std::collections::HashMap::new(), None);
                    }
                }

                Ok(Value::QueryBuilder(Rc::new(RefCell::new(qb))))
            })),
        );

        // Model.select("name", "email") / Model.fields("name", "email") - field selection
        let select_fn = Rc::new(NativeFunction::new("Model.select", None, |args| {
            let class = get_class_rc_from_args(&args)?;
            let class_name = class.name.clone();
            let collection = class_name_to_collection(&class_name);

            let mut qb = QueryBuilder::new_with_class(class_name, collection, class);
            let mut fields = Vec::new();
            for arg in &args[1..] {
                match arg {
                    Value::String(s) => fields.push(s.clone()),
                    other => {
                        return Err(format!(
                            "select() expects string field names, got {}",
                            other.type_name()
                        ))
                    }
                }
            }
            qb.set_select(fields);
            Ok(Value::QueryBuilder(Rc::new(RefCell::new(qb))))
        }));
        native_static_methods.insert("select".to_string(), select_fn.clone());
        native_static_methods.insert("fields".to_string(), select_fn);

        // Model.join("posts") or Model.join("posts", "published = @p", { p: true })
        native_static_methods.insert(
            "join".to_string(),
            Rc::new(NativeFunction::new("Model.join", None, |args| {
                let class = get_class_rc_from_args(&args)?;
                let class_name = class.name.clone();
                let collection = class_name_to_collection(&class_name);

                let rel_name = match args.get(1) {
                    Some(Value::String(s)) => s.clone(),
                    Some(other) => {
                        return Err(format!(
                            "join() expects string relation name, got {}",
                            other.type_name()
                        ))
                    }
                    None => return Err("join() requires a relation name".to_string()),
                };

                let rel = get_relation(&class_name, &rel_name).ok_or_else(|| {
                    format!("No relation '{}' defined on {}", rel_name, class_name)
                })?;

                let filter = match args.get(2) {
                    Some(Value::String(s)) => Some(s.clone()),
                    _ => None,
                };

                let bind_vars = match args.get(3) {
                    Some(Value::Hash(hash)) => {
                        use crate::interpreter::value::HashKey;
                        let mut map = std::collections::HashMap::new();
                        for (k, v) in hash.borrow().iter() {
                            if let HashKey::String(key) = k {
                                map.insert(
                                    key.clone(),
                                    crate::interpreter::value::value_to_json(v)?,
                                );
                            }
                        }
                        map
                    }
                    _ => std::collections::HashMap::new(),
                };

                let mut qb = QueryBuilder::new_with_class(class_name, collection, class);
                qb.add_join(rel_name, rel, filter, bind_vars);

                Ok(Value::QueryBuilder(Rc::new(RefCell::new(qb))))
            })),
        );

        // ====================================================================
        // CRUD Methods
        // ====================================================================

        // Model.create(data) - Insert document with validation, returns instance in record
        use super::crud::{exec_insert, json_to_value};
        use super::validation::{build_validation_result, run_validations};
        use crate::interpreter::value::value_to_json;
        native_static_methods.insert(
            "create".to_string(),
            Rc::new(NativeFunction::new("Model.create", Some(2), |args| {
                let class = get_class_rc_from_args(&args)?;
                let class_name = class.name.clone();
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
                        if let serde_json::Value::Object(mut data_map) = data_value {
                            // Flatten DB metadata fields from the id response
                            if let serde_json::Value::Object(ref id_map) = id {
                                for field in &["_key", "_id", "_rev", "_created_at", "_updated_at"]
                                {
                                    if let Some(val) = id_map.get(*field) {
                                        data_map.insert(field.to_string(), val.clone());
                                    }
                                }
                            }
                            data_map.insert("id".to_string(), id);
                            let record =
                                json_doc_to_instance(&class, &serde_json::Value::Object(data_map));

                            let mut result_pairs = crate::interpreter::value::HashPairs::default();
                            result_pairs
                                .insert(HashKey::String("valid".to_string()), Value::Bool(true));
                            result_pairs.insert(HashKey::String("record".to_string()), record);
                            Ok(Value::Hash(Rc::new(RefCell::new(result_pairs))))
                        } else {
                            let mut result_pairs = crate::interpreter::value::HashPairs::default();
                            result_pairs
                                .insert(HashKey::String("valid".to_string()), Value::Bool(true));
                            Ok(Value::Hash(Rc::new(RefCell::new(result_pairs))))
                        }
                    }
                    Err(e) => Ok(Value::String(format!("Error: {}", e))),
                }
            })),
        );

        // Model.find(id) - Get by ID, returns a class instance
        use super::crud::{exec_get, json_doc_to_instance};
        native_static_methods.insert(
            "find".to_string(),
            Rc::new(NativeFunction::new("Model.find", Some(2), |args| {
                let class = get_class_rc_from_args(&args)?;
                let collection = class_name_to_collection(&class.name);

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
                    Ok(doc) => Ok(json_doc_to_instance(&class, &doc)),
                    // Not found or collection error → null (not an application error)
                    Err(_) => Ok(Value::Null),
                }
            })),
        );

        // Model.where(filter, bind_vars) - Returns a QueryBuilder for chaining
        use std::collections::HashMap as StdHashMap;
        native_static_methods.insert(
            "where".to_string(),
            Rc::new(NativeFunction::new("Model.where", Some(3), |args| {
                let class = get_class_rc_from_args(&args)?;
                let class_name = class.name.clone();
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
                let mut qb = QueryBuilder::new_with_class(class_name, collection, class);
                qb.set_filter(filter, bind_vars);

                Ok(Value::QueryBuilder(Rc::new(RefCell::new(qb))))
            })),
        );

        // Model.all() - Get all documents as class instances
        use super::crud::exec_auto_collection_as_instances;
        native_static_methods.insert(
            "all".to_string(),
            Rc::new(NativeFunction::new("Model.all", Some(1), |args| {
                let class = get_class_rc_from_args(&args)?;
                let collection = class_name_to_collection(&class.name);
                let sdbql = format!("FOR doc IN {} RETURN doc", collection);
                Ok(exec_auto_collection_as_instances(
                    sdbql,
                    &collection,
                    &class,
                ))
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

        // Model.order(field, direction?) - Returns a QueryBuilder with ordering (no filter)
        native_static_methods.insert(
            "order".to_string(),
            Rc::new(NativeFunction::new("Model.order", Some(3), |args| {
                let class = get_class_rc_from_args(&args)?;
                let class_name = class.name.clone();
                let collection = class_name_to_collection(&class_name);

                let field = match args.get(1) {
                    Some(Value::String(s)) => s.clone(),
                    Some(other) => {
                        return Err(format!(
                            "Model.order() expects string field name, got {}",
                            other.type_name()
                        ))
                    }
                    None => return Err("Model.order() requires a field name".to_string()),
                };

                let direction = match args.get(2) {
                    Some(Value::String(s)) => s.clone(),
                    _ => "asc".to_string(),
                };

                let mut qb = QueryBuilder::new_with_class(class_name, collection, class);
                qb.set_order(field, direction);

                Ok(Value::QueryBuilder(Rc::new(RefCell::new(qb))))
            })),
        );

        // Model.limit(n) - Returns a QueryBuilder with a limit (no filter)
        native_static_methods.insert(
            "limit".to_string(),
            Rc::new(NativeFunction::new("Model.limit", Some(2), |args| {
                let class = get_class_rc_from_args(&args)?;
                let class_name = class.name.clone();
                let collection = class_name_to_collection(&class_name);

                let limit = match args.get(1) {
                    Some(Value::Int(n)) => *n as usize,
                    Some(Value::Float(f)) => *f as usize,
                    Some(other) => {
                        return Err(format!(
                            "Model.limit() expects integer, got {}",
                            other.type_name()
                        ))
                    }
                    None => return Err("Model.limit() requires a number".to_string()),
                };

                let mut qb = QueryBuilder::new_with_class(class_name, collection, class);
                qb.set_limit(limit);

                Ok(Value::QueryBuilder(Rc::new(RefCell::new(qb))))
            })),
        );

        // Model.update(id, data) - Update document (accepts hash or instance as data)
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
                    Some(Value::Instance(inst)) => {
                        let inst_ref = inst.borrow();
                        let mut map = serde_json::Map::new();
                        for (k, v) in &inst_ref.fields {
                            if !k.starts_with('_') {
                                map.insert(k.clone(), value_to_json(v)?);
                            }
                        }
                        Ok(serde_json::Value::Object(map))
                    }
                    Some(other) => Err(format!(
                        "Model.update() expects hash or instance data, got {}",
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
                    "FOR doc IN {} COLLECT WITH COUNT INTO cnt RETURN cnt",
                    collection
                );

                match exec_query(&collection, sdbql) {
                    Ok(results) => {
                        // COUNT query returns [N] - extract the integer
                        if let Some(count) = results.first() {
                            Ok(json_to_value(count))
                        } else {
                            Ok(Value::Int(0))
                        }
                    }
                    // Collection doesn't exist yet → count is 0
                    Err(_) => Ok(Value::Int(0)),
                }
            })),
        );

        // ====================================================================
        // Instance Methods (called on model instances: user.update(), user.delete())
        // ====================================================================
        let mut native_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

        // instance.update() - Persist current instance fields to DB
        // Returns true on success, false on validation/DB error (errors stored in _errors)
        native_methods.insert(
            "update".to_string(),
            Rc::new(NativeFunction::new("Model#update", Some(0), |args| {
                use super::validation::run_validations;

                let instance = match &args[0] {
                    Value::Instance(inst) => inst.clone(),
                    _ => return Err("Expected instance".to_string()),
                };
                let inst_ref = instance.borrow();
                let class_name = inst_ref.class.name.clone();
                let collection = class_name_to_collection(&class_name);
                let key = inst_ref
                    .get("_key")
                    .ok_or_else(|| "Instance has no _key field".to_string())?;
                let key_str = match key {
                    Value::String(s) => s,
                    _ => return Err("_key is not a string".to_string()),
                };

                // Run validations
                let data_hash = instance_fields_to_hash(&inst_ref);
                let errors = run_validations(&class_name, &data_hash, false);
                if !errors.is_empty() {
                    let error_values: Vec<Value> = errors.iter().map(|e| e.to_value()).collect();
                    drop(inst_ref);
                    instance.borrow_mut().set(
                        "_errors".to_string(),
                        Value::Array(Rc::new(RefCell::new(error_values))),
                    );
                    return Ok(Value::Bool(false));
                }

                let mut map = serde_json::Map::new();
                for (k, v) in &inst_ref.fields {
                    if !k.starts_with('_') {
                        map.insert(k.clone(), value_to_json(v)?);
                    }
                }
                drop(inst_ref);
                match exec_update(&collection, &key_str, serde_json::Value::Object(map), true) {
                    Ok(result) => {
                        let mut inst_mut = instance.borrow_mut();
                        if let serde_json::Value::Object(ref res_map) = result {
                            if let Some(rev) = res_map.get("_rev") {
                                inst_mut.set("_rev".to_string(), json_to_value(rev));
                            }
                        }
                        inst_mut.set(
                            "_errors".to_string(),
                            Value::Array(Rc::new(RefCell::new(vec![]))),
                        );
                        Ok(Value::Bool(true))
                    }
                    Err(e) => {
                        instance.borrow_mut().set(
                            "_errors".to_string(),
                            Value::Array(Rc::new(RefCell::new(vec![Value::String(e.to_string())]))),
                        );
                        Ok(Value::Bool(false))
                    }
                }
            })),
        );

        // instance.save() - Insert or update depending on whether _key exists
        // Returns true on success, false on validation/DB error (errors stored in _errors)
        native_methods.insert(
            "save".to_string(),
            Rc::new(NativeFunction::new("Model#save", Some(0), |args| {
                use super::validation::run_validations;

                let instance = match &args[0] {
                    Value::Instance(inst) => inst.clone(),
                    _ => return Err("Expected instance".to_string()),
                };
                let inst_ref = instance.borrow();
                let class_name = inst_ref.class.name.clone();
                let collection = class_name_to_collection(&class_name);
                let has_key = matches!(inst_ref.get("_key"), Some(Value::String(_)));

                // Run validations
                let data_hash = instance_fields_to_hash(&inst_ref);
                let errors = run_validations(&class_name, &data_hash, !has_key);
                if !errors.is_empty() {
                    let error_values: Vec<Value> = errors.iter().map(|e| e.to_value()).collect();
                    drop(inst_ref);
                    instance.borrow_mut().set(
                        "_errors".to_string(),
                        Value::Array(Rc::new(RefCell::new(error_values))),
                    );
                    return Ok(Value::Bool(false));
                }

                let mut map = serde_json::Map::new();
                for (k, v) in &inst_ref.fields {
                    if !k.starts_with('_') {
                        map.insert(k.clone(), value_to_json(v)?);
                    }
                }

                if has_key {
                    // Update existing document
                    let key_str = match inst_ref.get("_key").unwrap() {
                        Value::String(s) => s,
                        _ => unreachable!(),
                    };
                    drop(inst_ref);
                    match exec_update(&collection, &key_str, serde_json::Value::Object(map), true) {
                        Ok(result) => {
                            let mut inst_mut = instance.borrow_mut();
                            if let serde_json::Value::Object(ref res_map) = result {
                                if let Some(rev) = res_map.get("_rev") {
                                    inst_mut.set("_rev".to_string(), json_to_value(rev));
                                }
                            }
                            inst_mut.set(
                                "_errors".to_string(),
                                Value::Array(Rc::new(RefCell::new(vec![]))),
                            );
                            Ok(Value::Bool(true))
                        }
                        Err(e) => {
                            instance.borrow_mut().set(
                                "_errors".to_string(),
                                Value::Array(Rc::new(RefCell::new(vec![Value::String(
                                    e.to_string(),
                                )]))),
                            );
                            Ok(Value::Bool(false))
                        }
                    }
                } else {
                    // Insert new document
                    drop(inst_ref);
                    match exec_insert(&collection, None, serde_json::Value::Object(map)) {
                        Ok(result) => {
                            let mut inst_mut = instance.borrow_mut();
                            if let serde_json::Value::Object(ref res_map) = result {
                                for field in &["_key", "_id", "_rev", "_created_at", "_updated_at"]
                                {
                                    if let Some(val) = res_map.get(*field) {
                                        inst_mut.set(field.to_string(), json_to_value(val));
                                    }
                                }
                            }
                            inst_mut.set(
                                "_errors".to_string(),
                                Value::Array(Rc::new(RefCell::new(vec![]))),
                            );
                            Ok(Value::Bool(true))
                        }
                        Err(e) => {
                            instance.borrow_mut().set(
                                "_errors".to_string(),
                                Value::Array(Rc::new(RefCell::new(vec![Value::String(
                                    e.to_string(),
                                )]))),
                            );
                            Ok(Value::Bool(false))
                        }
                    }
                }
            })),
        );

        // instance.delete() - Delete the document from DB
        native_methods.insert(
            "delete".to_string(),
            Rc::new(NativeFunction::new("Model#delete", Some(0), |args| {
                let instance = match &args[0] {
                    Value::Instance(inst) => inst.clone(),
                    _ => return Err("Expected instance".to_string()),
                };
                let inst_ref = instance.borrow();
                let collection = class_name_to_collection(&inst_ref.class.name);
                let key = inst_ref
                    .get("_key")
                    .ok_or_else(|| "Instance has no _key field".to_string())?;
                let key_str = match key {
                    Value::String(s) => s,
                    _ => return Err("_key is not a string".to_string()),
                };
                drop(inst_ref);
                match exec_delete(&collection, &key_str) {
                    Ok(result) => Ok(json_to_value(&result)),
                    Err(e) => Ok(Value::String(format!("Error: {}", e))),
                }
            })),
        );

        // instance.errors - Return the list of errors from last save/update
        native_methods.insert(
            "errors".to_string(),
            Rc::new(NativeFunction::new("Model#errors", Some(0), |args| {
                let instance = match &args[0] {
                    Value::Instance(inst) => inst.clone(),
                    _ => return Err("Expected instance".to_string()),
                };
                let inst_ref = instance.borrow();
                match inst_ref.get("_errors") {
                    Some(errors) => Ok(errors),
                    None => Ok(Value::Array(Rc::new(RefCell::new(vec![])))),
                }
            })),
        );

        // instance.reload - Re-fetch from DB and refresh all fields
        native_methods.insert(
            "reload".to_string(),
            Rc::new(NativeFunction::new("Model#reload", Some(0), |args| {
                let instance = match &args[0] {
                    Value::Instance(inst) => inst.clone(),
                    _ => return Err("Expected instance".to_string()),
                };
                let inst_ref = instance.borrow();
                let collection = class_name_to_collection(&inst_ref.class.name);
                let key = inst_ref
                    .get("_key")
                    .ok_or_else(|| "Instance has no _key field, cannot reload".to_string())?;
                let key_str = match key {
                    Value::String(s) => s,
                    _ => return Err("_key is not a string".to_string()),
                };
                drop(inst_ref);
                match exec_get(&collection, &key_str) {
                    Ok(doc) => {
                        if let serde_json::Value::Object(map) = &doc {
                            let mut inst_mut = instance.borrow_mut();
                            for (k, v) in map {
                                inst_mut.set(k.clone(), json_to_value(v));
                            }
                        }
                        Ok(Value::Instance(instance))
                    }
                    Err(e) => Err(format!("reload failed: {}", e)),
                }
            })),
        );

        let model_class = Class {
            name: "Model".to_string(),
            superclass: None,
            methods: HashMap::new(),
            static_methods: HashMap::new(),
            native_static_methods,
            native_methods,
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

    // Relation DSL global functions: has_many, has_one, belongs_to
    for (rel_method, rel_type) in &[
        ("has_many", RelationType::HasMany),
        ("has_one", RelationType::HasOne),
        ("belongs_to", RelationType::BelongsTo),
    ] {
        let method_name = rel_method.to_string();
        let rel_type = rel_type.clone();
        env.define(
            method_name.clone(),
            Value::NativeFunction(NativeFunction::new(&method_name, None, move |args| {
                let class_name = get_class_name_from_class(&args)?;
                let name = match args.get(1) {
                    Some(Value::String(s)) => s.clone(),
                    Some(other) => {
                        return Err(format!(
                            "relation expects string name, got {}",
                            other.type_name()
                        ))
                    }
                    None => return Err("relation requires a name argument".to_string()),
                };

                let mut class_override: Option<String> = None;
                let mut fk_override: Option<String> = None;
                if let Some(Value::Hash(hash)) = args.get(2) {
                    use crate::interpreter::value::HashKey;
                    for (k, v) in hash.borrow().iter() {
                        if let HashKey::String(key) = k {
                            match key.as_str() {
                                "class_name" => {
                                    if let Value::String(s) = v {
                                        class_override = Some(s.clone());
                                    }
                                }
                                "foreign_key" => {
                                    if let Value::String(s) = v {
                                        fk_override = Some(s.clone());
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }

                let relation = build_relation(
                    &class_name,
                    &name,
                    rel_type.clone(),
                    class_override.as_deref(),
                    fk_override.as_deref(),
                );
                register_relation(&class_name, relation);
                Ok(Value::Null)
            })),
        );
    }
}
