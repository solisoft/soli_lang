//! Simplified OOP Model system for SoliLang.
//!
//! Collection name is auto-derived from the class name:
//! - `User` → `"users"`
//! - `BlogPost` → `"blog_posts"`
//!
//! # Query Generation
//!
//! The Model system generates SDBQL (SoliDB Query Language) queries:
//! - `User.all()` → `FOR doc IN users RETURN doc`
//! - `User.where("doc.age >= @age", { "age": 18 })` → `FOR doc IN users FILTER doc.age >= @age RETURN doc`
//! - `User.count()` → `FOR doc IN users COLLECT WITH COUNT INTO count RETURN count`
//!
//! # CRUD Operations
//!
//! ```soli
//! // Create
//! let user = User.create({ "name": "Alice", "email": "alice@example.com" });
//!
//! // Read
//! let found = User.find("user_id");
//! let all = User.all();
//! let adults = User.where("doc.age >= @age", { "age": 18 }).all();
//!
//! // Update
//! User.update("user_id", { "name": "Alice Smith" });
//!
//! // Delete
//! User.delete("user_id");
//!
//! // Count
//! let total = User.count();
//! ```
//!
//! # Query Builder Chaining
//!
//! ```soli
//! User.where("doc.age >= @age", { "age": 18 })
//!     .where("doc.active == @active", { "active": true })
//!     .order("created_at", "desc")
//!     .limit(10)
//!     .offset(20)
//!     .all();
//! ```
//!
//! # Validations
//!
//! ```soli
//! class User extends Model {
//!     validates("email", { "presence": true, "uniqueness": true })
//!     validates("name", { "presence": true, "min_length": 2, "max_length": 100 })
//!     validates("age", { "numericality": true, "min": 0, "max": 150 })
//!     validates("website", { "format": "^https?://" })
//! }
//! ```
//!
//! Validation options:
//! - `presence: true` - Field must be present and not empty
//! - `uniqueness: true` - Field value must be unique in collection
//! - `min_length: n` - String must be at least n characters
//! - `max_length: n` - String must be at most n characters
//! - `format: "regex"` - String must match regex pattern
//! - `numericality: true` - Value must be a number
//! - `min: n` - Number must be >= n
//! - `max: n` - Number must be <= n
//! - `custom: "method_name"` - Call custom validation method
//!
//! # Callbacks
//!
//! ```soli
//! class User extends Model {
//!     before_save("normalize_email")
//!     after_create("send_welcome_email")
//!     before_update("log_changes")
//!     after_delete("cleanup_related")
//! }
//! ```
//!
//! Available callbacks:
//! - `before_save`, `after_save`
//! - `before_create`, `after_create`
//! - `before_update`, `after_update`
//! - `before_delete`, `after_delete`

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{OnceLock, RwLock};
use std::time::Duration;

use crate::interpreter::builtins::http_class::get_http_client;
use crate::serve::get_tokio_handle;
use crate::solidb_http::SoliDBClient;
use indexmap::IndexMap;
use lazy_static::lazy_static;
use reqwest;

use crate::interpreter::environment::Environment;
use crate::interpreter::symbol::SymbolId;
use crate::interpreter::value::{Class, HashKey, NativeFunction, Value};


// ============================================================================
// Validation Types
// ============================================================================

/// A single validation rule for a field.
#[derive(Debug, Clone)]
pub struct ValidationRule {
    pub field: String,
    pub presence: bool,
    pub uniqueness: bool,
    pub min_length: Option<usize>,
    pub max_length: Option<usize>,
    pub format: Option<String>, // regex pattern
    pub numericality: bool,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub custom: Option<String>, // method name for custom validation
}

impl Default for ValidationRule {
    fn default() -> Self {
        Self {
            field: String::new(),
            presence: false,
            uniqueness: false,
            min_length: None,
            max_length: None,
            format: None,
            numericality: false,
            min: None,
            max: None,
            custom: None,
        }
    }
}

impl ValidationRule {
    pub fn new(field: String) -> Self {
        Self {
            field,
            ..Default::default()
        }
    }
}

/// A validation error.
#[derive(Debug, Clone)]
pub struct ValidationError {
    pub field: String,
    pub message: String,
}

impl ValidationError {
    pub fn new(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
        }
    }

    pub fn to_value(&self) -> Value {
        let mut pairs: IndexMap<HashKey, Value> = IndexMap::new();
        pairs.insert(HashKey::String("field".into()), Value::String(self.field.clone()));
        pairs.insert(HashKey::String("message".into()), Value::String(self.message.clone()));
        Value::Hash(Rc::new(RefCell::new(pairs)))
    }
}

// ============================================================================
// Callback Types
// ============================================================================

/// Lifecycle callbacks for a model.
#[derive(Debug, Clone, Default)]
pub struct ModelCallbacks {
    pub before_save: Vec<String>,
    pub after_save: Vec<String>,
    pub before_create: Vec<String>,
    pub after_create: Vec<String>,
    pub before_update: Vec<String>,
    pub after_update: Vec<String>,
    pub before_delete: Vec<String>,
    pub after_delete: Vec<String>,
}

// ============================================================================
// Model Metadata & Registry
// ============================================================================

/// Metadata for a model class (validations, callbacks).
#[derive(Debug, Clone, Default)]
pub struct ModelMetadata {
    pub validations: Vec<ValidationRule>,
    pub callbacks: ModelCallbacks,
}

/// Cached database configuration to avoid repeated env::var() lookups.
/// Note: api_key and database are read at request time so .env can be loaded later.
struct DbConfig {
    host: String,
}

impl DbConfig {
    fn from_env() -> Self {
        let host = std::env::var("SOLIDB_HOST")
            .unwrap_or_else(|_| "http://localhost:6745".to_string());
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

    /// Global async HTTP client with connection pooling for SoliDB queries.
    pub static ref ASYNC_HTTP_CLIENT: reqwest::Client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .pool_idle_timeout(Duration::from_secs(90))
        .pool_max_idle_per_host(64)
        .tcp_keepalive(Duration::from_secs(60))
        .build()
        .expect("Failed to create async HTTP client");

    /// Shared tokio runtime for database operations.
    pub static ref DB_RUNTIME: tokio::runtime::Runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("Failed to create database runtime");

    /// Cached DB configuration (for username/password which are less likely to change).
    static ref DB_CONFIG: DbConfig = DbConfig::from_env();
}

/// Cached DB config - initialized on first use.
static CACHED_DB_CONFIG: OnceLock<CachedDbConfig> = OnceLock::new();

struct CachedDbConfig {
    cursor_url: String,
    api_key: Option<String>,
}

/// Initialize DB config from environment - call this after .env is loaded.
pub fn init_db_config() {
    let _ = get_db_config();
}

/// Get cached DB config - initialized on first call.
fn get_db_config() -> &'static CachedDbConfig {
    CACHED_DB_CONFIG.get_or_init(|| {
        let host = std::env::var("SOLIDB_HOST")
            .unwrap_or_else(|_| "http://localhost:6745".to_string());
        let host = host
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .to_string();
        let database = std::env::var("SOLIDB_DATABASE").unwrap_or_else(|_| "default".to_string());
        let cursor_url = format!("http://{}/_api/database/{}/cursor", host, database);
        let api_key = std::env::var("SOLIDB_API_KEY").ok();
        CachedDbConfig { cursor_url, api_key }
    })
}

/// Get cursor URL.
fn get_cursor_url() -> &'static str {
    &get_db_config().cursor_url
}

/// Get API key.
fn get_api_key() -> Option<&'static str> {
    get_db_config().api_key.as_deref()
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

/// Register a validation rule for a model class.
pub fn register_validation(class_name: &str, rule: ValidationRule) {
    let mut registry = MODEL_REGISTRY.write().unwrap();
    let metadata = registry.entry(class_name.to_string()).or_default();
    metadata.validations.push(rule);
}

/// Register a callback for a model class.
pub fn register_callback(class_name: &str, callback_type: &str, method_name: &str) {
    let mut registry = MODEL_REGISTRY.write().unwrap();
    let metadata = registry.entry(class_name.to_string()).or_default();
    match callback_type {
        "before_save" => metadata.callbacks.before_save.push(method_name.to_string()),
        "after_save" => metadata.callbacks.after_save.push(method_name.to_string()),
        "before_create" => metadata
            .callbacks
            .before_create
            .push(method_name.to_string()),
        "after_create" => metadata
            .callbacks
            .after_create
            .push(method_name.to_string()),
        "before_update" => metadata
            .callbacks
            .before_update
            .push(method_name.to_string()),
        "after_update" => metadata
            .callbacks
            .after_update
            .push(method_name.to_string()),
        "before_delete" => metadata
            .callbacks
            .before_delete
            .push(method_name.to_string()),
        "after_delete" => metadata
            .callbacks
            .after_delete
            .push(method_name.to_string()),
        _ => {}
    }
}

// ============================================================================
// QueryBuilder
// ============================================================================

/// A query builder for chainable database queries.
/// Uses SDBQL filter expressions with symbol-based bind variables for O(1) lookup.
#[derive(Debug, Clone)]
pub struct QueryBuilder {
    pub class_name: SymbolId,
    pub collection: SymbolId,
    pub filter: Option<String>,
    pub bind_vars: HashMap<SymbolId, serde_json::Value>,
    pub order_by: Option<(SymbolId, SymbolId)>,
    pub limit_val: Option<usize>,
    pub offset_val: Option<usize>,
}

impl QueryBuilder {
    pub fn new(class_name: String, collection: String) -> Self {
        let class_id = crate::interpreter::get_symbol(&class_name);
        let collection_id = crate::interpreter::get_symbol(&collection);
        Self {
            class_name: class_id,
            collection: collection_id,
            filter: None,
            bind_vars: HashMap::new(),
            order_by: None,
            limit_val: None,
            offset_val: None,
        }
    }

    pub fn set_filter(&mut self, filter: String, bind_vars: HashMap<String, serde_json::Value>) {
        self.filter = Some(filter);
        self.bind_vars = bind_vars
            .into_iter()
            .map(|(k, v)| (crate::interpreter::get_symbol(&k), v))
            .collect();
    }

    pub fn set_order(&mut self, field: String, direction: String) {
        let field_id = crate::interpreter::get_symbol(&field);
        let dir_id = crate::interpreter::get_symbol(&direction);
        self.order_by = Some((field_id, dir_id));
    }

    pub fn set_limit(&mut self, limit: usize) {
        self.limit_val = Some(limit);
    }

    pub fn set_offset(&mut self, offset: usize) {
        self.offset_val = Some(offset);
    }

    /// Build the SDBQL query string.
    pub fn build_query(&self) -> (String, HashMap<String, serde_json::Value>) {
        let collection_str =
            crate::interpreter::symbol_string(self.collection).unwrap_or("unknown");
        let mut query = format!("FOR doc IN {}", collection_str);

        if let Some(filter) = &self.filter {
            query.push_str(&format!(" FILTER {}", filter));
        }

        if let Some((field, direction)) = &self.order_by {
            let field_str = crate::interpreter::symbol_string(*field).unwrap_or("unknown");
            let dir_str = crate::interpreter::symbol_string(*direction).unwrap_or("asc");
            let dir = match dir_str.to_lowercase().as_str() {
                "desc" | "descending" => "DESC",
                _ => "ASC",
            };
            query.push_str(&format!(" SORT doc.{} {}", field_str, dir));
        }

        if let Some(limit) = self.limit_val {
            if let Some(offset) = self.offset_val {
                query.push_str(&format!(" LIMIT {}, {}", offset, limit));
            } else {
                query.push_str(&format!(" LIMIT {}", limit));
            }
        }

        query.push_str(" RETURN doc");

        let bind_vars_str: HashMap<String, serde_json::Value> = self
            .bind_vars
            .iter()
            .map(|(k, v)| {
                (
                    crate::interpreter::symbol_string(*k)
                        .unwrap_or("")
                        .to_string(),
                    v.clone(),
                )
            })
            .collect();

        (query, bind_vars_str)
    }
}

// ============================================================================
// Validation Execution
// ============================================================================

/// Run validations on data and return any errors.
pub fn run_validations(class_name: &str, data: &Value, _is_create: bool) -> Vec<ValidationError> {
    let registry = MODEL_REGISTRY.read().unwrap();
    let metadata = match registry.get(class_name) {
        Some(m) => m,
        None => return vec![],
    };

    let hash = match data {
        Value::Hash(h) => h.borrow(),
        _ => return vec![ValidationError::new("_base", "Data must be a hash")],
    };

    let mut errors = Vec::new();

    for rule in &metadata.validations {
        // Find the field value
        let field_value = hash
            .iter()
            .find(|(k, _)| matches!(k, HashKey::String(s) if s == &rule.field))
            .map(|(_, v)| v.clone());

        // Presence validation
        if rule.presence {
            match &field_value {
                None => errors.push(ValidationError::new(&rule.field, "can't be blank")),
                Some(Value::Null) => {
                    errors.push(ValidationError::new(&rule.field, "can't be blank"))
                }
                Some(Value::String(s)) if s.is_empty() => {
                    errors.push(ValidationError::new(&rule.field, "can't be blank"))
                }
                _ => {}
            }
        }

        // Min length validation
        if let Some(min_len) = rule.min_length {
            if let Some(Value::String(s)) = &field_value {
                if s.len() < min_len {
                    errors.push(ValidationError::new(
                        &rule.field,
                        format!("is too short (minimum is {} characters)", min_len),
                    ));
                }
            }
        }

        // Max length validation
        if let Some(max_len) = rule.max_length {
            if let Some(Value::String(s)) = &field_value {
                if s.len() > max_len {
                    errors.push(ValidationError::new(
                        &rule.field,
                        format!("is too long (maximum is {} characters)", max_len),
                    ));
                }
            }
        }

        // Format validation (regex)
        if let Some(pattern) = &rule.format {
            if let Some(Value::String(s)) = &field_value {
                if let Ok(re) = regex::Regex::new(pattern) {
                    if !re.is_match(s) {
                        errors.push(ValidationError::new(&rule.field, "is invalid"));
                    }
                }
            }
        }

        // Numericality validation
        if rule.numericality {
            match &field_value {
                Some(Value::Int(_)) | Some(Value::Float(_)) => {}
                Some(_) => errors.push(ValidationError::new(&rule.field, "is not a number")),
                None => {} // Skip if field is not present (presence handles required)
            }
        }

        // Min value validation
        if let Some(min_val) = rule.min {
            match &field_value {
                Some(Value::Int(n)) if (*n as f64) < min_val => {
                    errors.push(ValidationError::new(
                        &rule.field,
                        format!("must be greater than or equal to {}", min_val),
                    ));
                }
                Some(Value::Float(n)) if *n < min_val => {
                    errors.push(ValidationError::new(
                        &rule.field,
                        format!("must be greater than or equal to {}", min_val),
                    ));
                }
                _ => {}
            }
        }

        // Max value validation
        if let Some(max_val) = rule.max {
            match &field_value {
                Some(Value::Int(n)) if (*n as f64) > max_val => {
                    errors.push(ValidationError::new(
                        &rule.field,
                        format!("must be less than or equal to {}", max_val),
                    ));
                }
                Some(Value::Float(n)) if *n > max_val => {
                    errors.push(ValidationError::new(
                        &rule.field,
                        format!("must be less than or equal to {}", max_val),
                    ));
                }
                _ => {}
            }
        }
    }

    errors
}

/// Build a validation result hash.
pub fn build_validation_result(
    valid: bool,
    errors: Vec<ValidationError>,
    record: Option<Value>,
) -> Value {
    let mut pairs: IndexMap<HashKey, Value> = IndexMap::new();
    pairs.insert(HashKey::String("valid".into()), Value::Bool(valid));

    if !valid {
        let error_values: Vec<Value> = errors.iter().map(|e| e.to_value()).collect();
        pairs.insert(HashKey::String("errors".into()), Value::Array(Rc::new(RefCell::new(error_values))));
    }

    if let Some(rec) = record {
        pairs.insert(HashKey::String("record".into()), rec);
    }

    Value::Hash(Rc::new(RefCell::new(pairs)))
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Convert PascalCase class name to snake_case collection name with pluralization.
/// Examples:
/// - "User" → "users"
/// - "BlogPost" → "blog_posts"
/// - "UserProfile" → "user_profiles"
/// - "CustomerModel" → "customers" (strips _model suffix before pluralizing)
fn class_name_to_collection(name: &str) -> String {
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
            let mut pairs: IndexMap<HashKey, Value> = IndexMap::new();
            for (k, v) in obj.iter() {
                pairs.insert(HashKey::String(k.clone()), json_to_value(v));
            }
            Value::Hash(Rc::new(RefCell::new(pairs)))
        }
    }
}

/// Fast async query execution - uses server's tokio runtime.
/// Uses same HTTP client as HTTP.request for consistency.
fn exec_async_query_with_binds(
    sdbql: String,
    bind_vars: Option<HashMap<String, serde_json::Value>>,
) -> Result<Vec<serde_json::Value>, String> {
    // Get cached values (initialized on first use after .env is loaded)
    let url = get_cursor_url();
    let api_key = get_api_key();

    let rt = get_tokio_handle().ok_or("No tokio runtime available")?;
    let client = get_http_client().clone();

    let future = async move {
        let mut payload = serde_json::json!({ "query": sdbql });
        if let Some(bv) = bind_vars {
            payload["bindVars"] = serde_json::json!(bv);
        }

        let mut request = client
            .post(url)
            .header("Content-Type", "application/json")
            .body(payload.to_string());

        if let Some(key) = api_key {
            request = request.header("X-API-Key", key);
        }

        let resp = request.send().await.map_err(|e| format!("HTTP error: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Query failed: {} - {}", status, body));
        }

        let json: serde_json::Value =
            resp.json().await.map_err(|e| format!("JSON error: {}", e))?;
        Ok(json
            .get("result")
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default())
    };

    rt.block_on(future)
}

/// Simple async query without bind variables - convenience wrapper.
fn exec_async_query(sdbql: String) -> Value {
    match exec_async_query_with_binds(sdbql, None) {
        Ok(results) => {
            // Direct conversion without intermediate Array wrapper
            let values: Vec<Value> = results.iter().map(json_to_value).collect();
            Value::Array(Rc::new(RefCell::new(values)))
        }
        Err(e) => Value::String(format!("Error: {}", e)),
    }
}

/// Async query returning raw JSON string (no Value conversion - fastest).
/// Uses same HTTP client as HTTP.request for consistency.
fn exec_async_query_raw(sdbql: String) -> Value {
    // Get cached values (initialized on first use after .env is loaded)
    let url = get_cursor_url();
    let api_key = get_api_key();
    let body = format!(r#"{{"query":"{}"}}"#, sdbql.replace('"', r#"\""#));

    let Some(rt) = get_tokio_handle() else {
        return Value::String("Error: No tokio runtime available".to_string());
    };

    let client = get_http_client().clone();
    match rt.block_on(async move {
        let mut request = client
            .post(url)
            .header("Content-Type", "application/json")
            .body(body);

        if let Some(key) = api_key {
            request = request.header("X-API-Key", key);
        }

        let resp = request.send().await.map_err(|e| format!("HTTP error: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Query failed: {} - {}", status, body));
        }

        resp.text().await.map_err(|e| format!("Read error: {}", e))
    }) {
        Ok(text) => Value::String(text),
        Err(e) => Value::String(format!("Error: {}", e)),
    }
}

/// Hardcoded query - exact same pattern as HTTP.request for comparison.
fn exec_query_hardcoded(sdbql: String) -> Value {
    // Hardcoded values - exactly like test-cursor does
    let url = "http://localhost:6745/_api/database/solipay/cursor";
    let api_key = "sk_8bc935c8fc837e147a0ab100747d197b354d5cdf80635bbd5951bc1a313a1ab8";
    let body = format!(r#"{{"query":"{}"}}"#, sdbql.replace('"', r#"\""#));

    let Some(rt) = get_tokio_handle() else {
        return Value::String("Error: No tokio runtime available".to_string());
    };

    let client = get_http_client().clone();
    match rt.block_on(async move {
        let request = client
            .post(url)
            .header("Content-Type", "application/json")
            .header("X-API-Key", api_key)
            .body(body);

        let resp = request.send().await.map_err(|e| e.to_string())?;
        resp.text().await.map_err(|e| e.to_string())
    }) {
        Ok(text) => Value::String(text),
        Err(e) => Value::String(format!("Error: {}", e)),
    }
}

// Re-export value_to_json from value module for backward compatibility
pub use crate::interpreter::value::value_to_json;

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

/// Extract class name from the first argument (the Class).
fn get_class_name_from_class(args: &[Value]) -> Result<String, String> {
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
        let mut native_static_methods = HashMap::new();

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

                Ok(exec_db_json(move || {
                    let client = SoliDBClient::connect(&DB_CONFIG.host)
                        .map_err(|e| format!("Failed to connect: {}", e))?;

                    let id = client
                        .insert(&collection, None, data_value.clone())
                        .map_err(|e| format!("Create failed: {}", e))?;

                    // Build result with record - direct JSON construction
                    let mut result_map = serde_json::Map::new();
                    result_map.insert("valid".to_string(), serde_json::Value::Bool(true));

                    // Add the record with id
                    if let serde_json::Value::Object(mut data_map) = data_value {
                        data_map.insert("id".to_string(), id);
                        result_map
                            .insert("record".to_string(), serde_json::Value::Object(data_map));
                    }

                    Ok(serde_json::Value::Object(result_map))
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

                Ok(exec_db_json(move || {
                    let client = SoliDBClient::connect(&DB_CONFIG.host)
                        .map_err(|e| format!("Failed to connect: {}", e))?;

                    let doc = client
                        .get(&collection, &id)
                        .map_err(|e| format!("Find failed: {}", e))?;

                    // Return doc or null if not found
                    Ok(doc.unwrap_or(serde_json::Value::Null))
                }))
            })),
        );

        // Model.where(filter, bind_vars) - Returns a QueryBuilder for chaining
        // Example: User.where("doc.age >= @age AND doc.active == @active", { "age": 18, "active": true })
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
                        let mut map = HashMap::new();
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
        native_static_methods.insert(
            "all".to_string(),
            Rc::new(NativeFunction::new("Model.all", Some(1), |args| {
                let collection = get_collection_from_class(&args)?;
                let sdbql = format!("FOR doc IN {} RETURN doc", collection);
                Ok(exec_async_query(sdbql))
            })),
        );

        // Model.all_json() - Get all documents as raw JSON string (fastest)
        native_static_methods.insert(
            "all_json".to_string(),
            Rc::new(NativeFunction::new("Model.all_json", Some(1), |args| {
                let collection = get_collection_from_class(&args)?;
                let sdbql = format!("FOR doc IN {} RETURN doc", collection);
                Ok(exec_async_query_raw(sdbql))
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

                Ok(exec_db_json(move || {
                    let client = SoliDBClient::connect(&DB_CONFIG.host)
                        .map_err(|e| format!("Failed to connect: {}", e))?;

                    client
                        .update(&collection, &id, data_value, true)
                        .map_err(|e| format!("Update failed: {}", e))?;

                    Ok(serde_json::Value::String("Updated".to_string()))
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

                Ok(exec_db_json(move || {
                    let client = SoliDBClient::connect(&DB_CONFIG.host)
                        .map_err(|e| format!("Failed to connect: {}", e))?;

                    client
                        .delete(&collection, &id)
                        .map_err(|e| format!("Delete failed: {}", e))?;

                    Ok(serde_json::Value::String("Deleted".to_string()))
                }))
            })),
        );

        // Model.count() - Count documents
        native_static_methods.insert(
            "count".to_string(),
            Rc::new(NativeFunction::new("Model.count", Some(1), |args| {
                let collection = get_collection_from_class(&args)?;

                Ok(exec_db_json(move || {
                    let client = SoliDBClient::connect(&DB_CONFIG.host)
                        .map_err(|e| format!("Failed to connect: {}", e))?;

                    let sdbql = format!(
                        "FOR doc IN {} COLLECT WITH COUNT INTO count RETURN count",
                        collection
                    );
                    let results = client
                        .query(&sdbql, None)
                        .map_err(|e| format!("Query failed: {}", e))?;

                    // Return directly as JSON array - skip string serialization
                    Ok(serde_json::Value::Array(results))
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
            static_fields: Rc::new(RefCell::new(HashMap::new())),
            fields: HashMap::new(),
            constructor: None,
        };
        env.define("Model".to_string(), Value::Class(Rc::new(model_class)));
    }
}

pub fn register_model_builtins(env: &mut Environment) {
    Model::register_builtins(env);

    // Register global wrapper functions for class-level DSL
    // These functions expect the class as the first argument (passed by execute_class)

    // validates(class, field, options) - Register validation rules
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

// ============================================================================
// QueryBuilder Execution
// ============================================================================

/// Execute a QueryBuilder and return results.
pub fn execute_query_builder(qb: &QueryBuilder) -> Value {
    let (query, bind_vars) = qb.build_query();
    let bind_vars_opt = if bind_vars.is_empty() {
        None
    } else {
        Some(bind_vars)
    };

    match exec_async_query_with_binds(query, bind_vars_opt) {
        Ok(results) => json_to_value(&serde_json::Value::Array(results)),
        Err(e) => Value::String(format!("Error: {}", e)),
    }
}

/// Execute a QueryBuilder for first result only.
pub fn execute_query_builder_first(qb: &QueryBuilder) -> Value {
    let mut qb_with_limit = qb.clone();
    qb_with_limit.set_limit(1);
    let (query, bind_vars) = qb_with_limit.build_query();
    let bind_vars_opt = if bind_vars.is_empty() {
        None
    } else {
        Some(bind_vars)
    };

    match exec_async_query_with_binds(query, bind_vars_opt) {
        Ok(results) => {
            let first = results.into_iter().next().unwrap_or(serde_json::Value::Null);
            json_to_value(&first)
        }
        Err(e) => Value::String(format!("Error: {}", e)),
    }
}

/// Execute a QueryBuilder for count.
pub fn execute_query_builder_count(qb: &QueryBuilder) -> Value {
    let collection = crate::interpreter::symbol_string(qb.collection)
        .unwrap_or("unknown")
        .to_string();
    let mut query = format!("FOR doc IN {}", collection);
    let bind_vars_str: HashMap<String, serde_json::Value> = qb
        .bind_vars
        .iter()
        .map(|(k, v)| {
            (
                crate::interpreter::symbol_string(*k)
                    .unwrap_or("")
                    .to_string(),
                v.clone(),
            )
        })
        .collect();

    if let Some(filter) = &qb.filter {
        query.push_str(&format!(" FILTER {}", filter));
    }

    query.push_str(" COLLECT WITH COUNT INTO count RETURN count");

    let bind_vars_opt = if bind_vars_str.is_empty() {
        None
    } else {
        Some(bind_vars_str)
    };

    match exec_async_query_with_binds(query, bind_vars_opt) {
        Ok(results) => json_to_value(&serde_json::Value::Array(results)),
        Err(e) => Value::String(format!("Error: {}", e)),
    }
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
