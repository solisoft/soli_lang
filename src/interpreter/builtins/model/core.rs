//! Core Model types, registry, and database configuration.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, Function, NativeFunction, Value};

pub use super::db_config::{
    get_api_key, get_basic_auth, get_cursor_url, get_database_name, get_jwt_token, init_db_config,
    init_jwt_token, DbConfig, DB_CONFIG,
};
pub use super::engine_context::{
    get_model_engine_context, set_model_engine_context, EngineContextGuard,
};
pub use super::registry::{
    get_or_create_metadata, get_translated_fields, is_soft_delete, is_translated_field,
    register_translation, update_metadata, ModelMetadata, MODEL_REGISTRY,
};

use super::callbacks::register_callback;
use super::relations::{build_relation, get_relation, register_relation, RelationType};
use super::validation::{register_validation, ValidationRule};

/// Get a Transaction class for a specific model.
/// Creates a new class each time (not cached due to Class not being Sync).
pub fn get_or_create_transaction_class(model_name: &str) -> Rc<Class> {
    let _model_name_owned = model_name.to_string();
    let collection = class_name_to_collection(model_name);

    let methods: std::collections::HashMap<String, Rc<Function>> = std::collections::HashMap::new();
    let mut native_methods: std::collections::HashMap<String, Rc<NativeFunction>> =
        std::collections::HashMap::new();

    native_methods.insert(
        "get".to_string(),
        Rc::new(NativeFunction::new("Transaction#get", Some(2), {
            let collection = collection.clone();
            move |args| {
                let key = match args.get(1) {
                    Some(Value::String(s)) => s.clone(),
                    Some(other) => {
                        return Err(format!(
                            "tx.get() expects string key, got {}",
                            other.type_name()
                        ))
                    }
                    None => return Err("tx.get() requires key argument".to_string()),
                };
                use super::crud::{exec_get_tx, json_to_value};
                match exec_get_tx(&collection, &key) {
                    Ok(result) => Ok(json_to_value(&result)),
                    Err(e) => Ok(Value::String(format!("Error: {}", e))),
                }
            }
        })),
    );

    native_methods.insert(
        "create".to_string(),
        Rc::new(NativeFunction::new("Transaction#create", Some(2), {
            let collection = collection.clone();
            move |args| {
                let doc = match args.get(1) {
                    Some(v) => crate::interpreter::value::value_to_json(v)
                        .map_err(|e| format!("Failed to convert document: {}", e))?,
                    None => return Err("tx.create() requires document argument".to_string()),
                };
                use super::crud::{exec_insert_tx, json_to_value};
                match exec_insert_tx(&collection, None, doc) {
                    Ok(result) => Ok(json_to_value(&result)),
                    Err(e) => Ok(Value::String(format!("Error: {}", e))),
                }
            }
        })),
    );

    native_methods.insert(
        "update".to_string(),
        Rc::new(NativeFunction::new("Transaction#update", Some(3), {
            let collection = collection.clone();
            move |args| {
                let key = match args.get(1) {
                    Some(Value::String(s)) => s.clone(),
                    Some(other) => {
                        return Err(format!(
                            "tx.update() expects string key, got {}",
                            other.type_name()
                        ))
                    }
                    None => return Err("tx.update() requires key argument".to_string()),
                };
                let doc = match args.get(2) {
                    Some(v) => crate::interpreter::value::value_to_json(v)
                        .map_err(|e| format!("Failed to convert document: {}", e))?,
                    None => return Err("tx.update() requires document argument".to_string()),
                };
                use super::crud::{exec_update_tx, json_to_value};
                match exec_update_tx(&collection, &key, doc) {
                    Ok(result) => Ok(json_to_value(&result)),
                    Err(e) => Ok(Value::String(format!("Error: {}", e))),
                }
            }
        })),
    );

    native_methods.insert(
        "delete".to_string(),
        Rc::new(NativeFunction::new("Transaction#delete", Some(2), {
            let collection = collection.clone();
            move |args| {
                let key = match args.get(1) {
                    Some(Value::String(s)) => s.clone(),
                    Some(other) => {
                        return Err(format!(
                            "tx.delete() expects string key, got {}",
                            other.type_name()
                        ))
                    }
                    None => return Err("tx.delete() requires key argument".to_string()),
                };
                use super::crud::{exec_delete_tx, json_to_value};
                match exec_delete_tx(&collection, &key) {
                    Ok(result) => Ok(json_to_value(&result)),
                    Err(e) => Ok(Value::String(format!("Error: {}", e))),
                }
            }
        })),
    );

    native_methods.insert(
        "commit".to_string(),
        Rc::new(NativeFunction::new(
            "Transaction#commit",
            Some(1),
            |_args| {
                use super::crud::commit_transaction;
                match commit_transaction() {
                    Ok(()) => Ok(Value::Bool(true)),
                    Err(e) => Ok(Value::String(format!("Error: {}", e))),
                }
            },
        )),
    );

    native_methods.insert(
        "rollback".to_string(),
        Rc::new(NativeFunction::new(
            "Transaction#rollback",
            Some(1),
            |_args| {
                use super::crud::rollback_transaction;
                match rollback_transaction() {
                    Ok(()) => Ok(Value::Bool(true)),
                    Err(e) => Ok(Value::String(format!("Error: {}", e))),
                }
            },
        )),
    );

    Rc::new(Class {
        name: format!("{}Transaction", model_name),
        superclass: None,
        methods: Rc::new(RefCell::new(methods)),
        static_methods: std::collections::HashMap::new(),
        native_static_methods: std::collections::HashMap::new(),
        native_methods,
        static_fields: Rc::new(RefCell::new(std::collections::HashMap::new())),
        fields: std::collections::HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(std::collections::HashMap::new())),
        ..Default::default()
    })
}

/// Convert PascalCase class name to snake_case collection name with pluralization.
/// Examples:
/// - "User" → "users"
/// - "BlogPost" → "blog_posts"
/// - "UserProfile" → "user_profiles"
/// - "CustomerModel" → "customers" (strips _model suffix before pluralizing)
///
/// If engine context is set, collection name is prefixed: "User" + engine "shop" → "shop_users"
pub fn class_name_to_collection(name: &str) -> String {
    class_name_to_collection_with_engine(name, get_model_engine_context().as_deref())
}

pub fn class_name_to_collection_with_engine(name: &str, engine: Option<&str>) -> String {
    let base = compute_base_collection_name(name);

    match engine {
        Some(e) => format!("{}_{}", e, base),
        None => base,
    }
}

fn compute_base_collection_name(name: &str) -> String {
    let name = name.strip_suffix("Model").unwrap_or(name);

    let mut result = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            result.push('_');
        }
        result.push(c.to_lowercase().next().unwrap());
    }
    result.push('s');
    result
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_base_collection_name() {
        assert_eq!(compute_base_collection_name("User"), "users");
        assert_eq!(compute_base_collection_name("BlogPost"), "blog_posts");
        assert_eq!(compute_base_collection_name("UserProfile"), "user_profiles");
        assert_eq!(compute_base_collection_name("CustomerModel"), "customers");
    }

    #[test]
    fn test_collection_with_engine_none() {
        assert_eq!(class_name_to_collection_with_engine("User", None), "users");
        assert_eq!(
            class_name_to_collection_with_engine("BlogPost", None),
            "blog_posts"
        );
    }

    #[test]
    fn test_collection_with_engine_prefix() {
        assert_eq!(
            class_name_to_collection_with_engine("User", Some("shop")),
            "shop_users"
        );
        assert_eq!(
            class_name_to_collection_with_engine("BlogPost", Some("admin")),
            "admin_blog_posts"
        );
        assert_eq!(
            class_name_to_collection_with_engine("CustomerModel", Some("billing")),
            "billing_customers"
        );
    }

    #[test]
    fn test_parse_count_result_scalar() {
        // Primary shape: [N]
        let results = vec![serde_json::json!(5)];
        assert_eq!(parse_count_result(&results), Value::Int(5));
    }

    #[test]
    fn test_parse_count_result_scalar_zero() {
        let results = vec![serde_json::json!(0)];
        assert_eq!(parse_count_result(&results), Value::Int(0));
    }

    #[test]
    fn test_parse_count_result_object_cnt() {
        // Alt shape: [{"cnt": N}]
        let results = vec![serde_json::json!({ "cnt": 42 })];
        assert_eq!(parse_count_result(&results), Value::Int(42));
    }

    #[test]
    fn test_parse_count_result_object_count() {
        // Alt shape: [{"count": N}]
        let results = vec![serde_json::json!({ "count": 7 })];
        assert_eq!(parse_count_result(&results), Value::Int(7));
    }

    #[test]
    fn test_parse_count_result_empty_is_zero() {
        // An empty result array means "no rows" — genuine zero.
        let results: Vec<serde_json::Value> = vec![];
        assert_eq!(parse_count_result(&results), Value::Int(0));
    }

    #[test]
    fn test_parse_count_result_object_without_known_key() {
        // Unknown object shape falls back to Value::Int(0) rather than
        // leaking a Value::Hash into callers expecting a number.
        let results = vec![serde_json::json!({ "total": 9 })];
        assert_eq!(parse_count_result(&results), Value::Int(0));
    }

    #[test]
    fn test_parse_count_result_large_number() {
        // Ensure i64 path handles values > i32::MAX.
        let big = (i32::MAX as i64) + 1;
        let results = vec![serde_json::json!(big)];
        assert_eq!(parse_count_result(&results), Value::Int(big));
    }
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

/// Parse the result of a count query into a `Value::Int`.
///
/// SolidB's primary return shape is a scalar: `[N]` (emitted by
/// `RETURN COLLECTION_COUNT(...)` and `RETURN LENGTH(...)`). Some
/// drivers / dialects emit an object wrapper instead: `[{"cnt": N}]`
/// or `[{"count": N}]`. Both are accepted here. Anything unrecognised
/// falls through to `json_to_value` so the caller can at least see the
/// raw payload rather than a silent zero.
pub fn parse_count_result(results: &[serde_json::Value]) -> Value {
    match results.first() {
        Some(serde_json::Value::Number(n)) => n.as_i64().map(Value::Int).unwrap_or(Value::Int(0)),
        Some(serde_json::Value::Object(map)) => map
            .get("cnt")
            .or_else(|| map.get("count"))
            .and_then(|v| v.as_i64())
            .map(Value::Int)
            .unwrap_or(Value::Int(0)),
        Some(other) => super::crud::json_to_value(other),
        None => Value::Int(0),
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

/// Apply every entry of a `Value::Hash` onto an instance's fields,
/// matching direct assignment (`inst.field = ...`). Framework-internal
/// `_`-prefixed keys (`_key`, `_errors`, `_pending_translations`) are
/// skipped — callers must never overwrite those via bulk update.
///
/// Non-Hash argument returns an error; non-String hash keys are silently
/// skipped (instances only have string-keyed fields).
fn apply_hash_to_instance(
    inst: &Rc<RefCell<crate::interpreter::value::Instance>>,
    hash: &Value,
) -> Result<(), String> {
    use crate::interpreter::value::HashKey;
    let pairs = match hash {
        Value::Hash(p) => p,
        other => {
            return Err(format!(
                "expected a Hash of attributes, got {}",
                other.type_name()
            ))
        }
    };
    let mut inst_mut = inst.borrow_mut();
    for (k, v) in pairs.borrow().iter() {
        if let HashKey::String(field) = k {
            if field.starts_with('_') {
                continue;
            }
            inst_mut.set(field.clone(), v.clone());
        }
    }
    Ok(())
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
                        None,
                        None,
                    );
                    register_relation(&class_name, relation);
                    Ok(Value::Null)
                })),
            );
        }

        // ====================================================================
        // Translation DSL: translate("field1", "field2", ...)
        // ====================================================================

        // Model.translate("title", "description") - declare translatable fields
        native_static_methods.insert(
            "translate".to_string(),
            Rc::new(NativeFunction::new("Model.translate", None, |args| {
                let class_name = get_class_name_from_class(&args)?;

                // Accept one or more field names as arguments
                let field_names: Vec<String> = args[1..]
                    .iter()
                    .filter_map(|arg| {
                        if let Value::String(s) = arg {
                            Some(s.clone())
                        } else {
                            None
                        }
                    })
                    .collect();

                if field_names.is_empty() {
                    return Err("translate() requires at least one field name".to_string());
                }

                for field_name in &field_names {
                    register_translation(&class_name, field_name);
                }
                Ok(Value::Null)
            })),
        );

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
                let errors = run_validations(&class_name, &data, None)?;
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

        // Model.mock_query_result(query, results_array) - Register mock DB response for testing
        use super::crud::register_query_mock;
        native_static_methods.insert(
            "mock_query_result".to_string(),
            Rc::new(NativeFunction::new(
                "Model.mock_query_result",
                Some(2),
                |args| {
                    let query = match args.get(1) {
                        Some(Value::String(s)) => s.clone(),
                        _ => {
                            return Err("mock_query_result expects query string as second argument"
                                .to_string())
                        }
                    };
                    let results_array = match args.get(2) {
                        Some(Value::Array(arr)) => arr
                            .borrow()
                            .iter()
                            .map(|v| value_to_json(v).map_err(|e| format!("Invalid JSON: {}", e)))
                            .collect::<Result<Vec<_>, _>>()?,
                        _ => {
                            return Err("mock_query_result expects results array as third argument"
                                .to_string())
                        }
                    };
                    register_query_mock(query, results_array);
                    Ok(Value::Null)
                },
            )),
        );

        // Model.clear_mocks() - Clear all registered mock responses
        use super::crud::clear_query_mocks;
        native_static_methods.insert(
            "clear_mocks".to_string(),
            Rc::new(NativeFunction::new("Model.clear_mocks", None, |_| {
                clear_query_mocks();
                Ok(Value::Null)
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

        // Model.delete_all() - Remove every document in the collection.
        // Primarily intended for test setup/teardown. Fetches all keys
        // then deletes each individually — a single `FOR doc IN … REMOVE`
        // query isn't actually applied in SolidB, so iterate.
        native_static_methods.insert(
            "delete_all".to_string(),
            Rc::new(NativeFunction::new("Model.delete_all", Some(1), |args| {
                let collection = get_collection_from_class(&args)?;
                let sdbql = format!("FOR doc IN {} RETURN doc._key", collection);
                let results = match super::crud::exec_query(&collection, sdbql) {
                    Ok(r) => r,
                    Err(e) => return Err(format!("Model.delete_all() failed: {}", e)),
                };
                for key_json in &results {
                    let key = match key_json {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    let _ = super::crud::exec_delete(&collection, &key);
                }
                Ok(Value::Null)
            })),
        );

        // Model.transaction - Execute SDBQL or get transaction handle
        // Usage:
        //   Model.transaction(sdbql_string) - Execute SDBQL in transaction
        //   Model.transaction() - Get transaction handle: tx = User.transaction(); tx.create({...}); tx.commit()
        native_static_methods.insert(
            "transaction".to_string(),
            Rc::new(NativeFunction::new(
                "Model.transaction",
                Some(1),
                |args| match args.get(1) {
                    Some(Value::String(s)) => {
                        use super::crud::exec_transaction_sdbql;
                        match exec_transaction_sdbql(s) {
                            Ok(result) => Ok(json_to_value(&result)),
                            Err(e) => Ok(Value::String(format!("Error: {}", e))),
                        }
                    }
                    Some(Value::Function(_)) | Some(Value::VmClosure(_)) => {
                        // Block passed - return transaction handle, user can call .run() on it
                        let class_name = get_class_name_from_class(&args)?;
                        Ok(Value::Class(get_or_create_transaction_class(&class_name)))
                    }
                    None => {
                        let class_name = get_class_name_from_class(&args)?;
                        Ok(Value::Class(get_or_create_transaction_class(&class_name)))
                    }
                    Some(other) => Err(format!(
                        "Model.transaction() expects SDBQL string or no arguments, got {}",
                        other.type_name()
                    )),
                },
            )),
        );

        // Model.count() - Count documents
        use super::crud::exec_query;
        native_static_methods.insert(
            "count".to_string(),
            Rc::new(NativeFunction::new("Model.count", Some(1), |args| {
                let collection = get_collection_from_class(&args)?;

                let sdbql = format!("RETURN COLLECTION_COUNT(\"{}\")", collection);

                match exec_query(&collection, sdbql) {
                    Ok(results) => Ok(parse_count_result(&results)),
                    // `exec_with_auto_collection` auto-creates a missing
                    // collection and retries, so any error reaching us here
                    // is a real failure — surface it instead of silently
                    // returning 0 (which previously masked broken counts).
                    Err(e) => Err(format!("Model.count() failed: {}", e)),
                }
            })),
        );

        // Model.with_deleted - Returns a QueryBuilder that includes soft-deleted records
        native_static_methods.insert(
            "with_deleted".to_string(),
            Rc::new(NativeFunction::new("Model.with_deleted", Some(1), |args| {
                let class = get_class_rc_from_args(&args)?;
                let class_name = class.name.clone();
                let collection = class_name_to_collection(&class_name);
                let mut qb =
                    super::query::QueryBuilder::new_with_class(class_name, collection, class);
                qb.soft_delete_mode = super::query::SoftDeleteMode::WithDeleted;
                Ok(Value::QueryBuilder(Rc::new(RefCell::new(qb))))
            })),
        );

        // Model.only_deleted - Returns a QueryBuilder with only soft-deleted records
        native_static_methods.insert(
            "only_deleted".to_string(),
            Rc::new(NativeFunction::new("Model.only_deleted", Some(1), |args| {
                let class = get_class_rc_from_args(&args)?;
                let class_name = class.name.clone();
                let collection = class_name_to_collection(&class_name);
                let mut qb =
                    super::query::QueryBuilder::new_with_class(class_name, collection, class);
                qb.soft_delete_mode = super::query::SoftDeleteMode::OnlyDeleted;
                Ok(Value::QueryBuilder(Rc::new(RefCell::new(qb))))
            })),
        );

        // Model.offset(n) - Returns a QueryBuilder with offset
        native_static_methods.insert(
            "offset".to_string(),
            Rc::new(NativeFunction::new("Model.offset", Some(2), |args| {
                let class = get_class_rc_from_args(&args)?;
                let class_name = class.name.clone();
                let collection = class_name_to_collection(&class_name);
                let offset = match args.get(1) {
                    Some(Value::Int(n)) if *n >= 0 => *n as usize,
                    _ => return Err("offset() expects a positive integer".to_string()),
                };
                let mut qb =
                    super::query::QueryBuilder::new_with_class(class_name, collection, class);
                qb.set_offset(offset);
                Ok(Value::QueryBuilder(Rc::new(RefCell::new(qb))))
            })),
        );

        // Model.find_by(field, value) - Find first record matching field=value
        native_static_methods.insert(
            "find_by".to_string(),
            Rc::new(NativeFunction::new("Model.find_by", Some(3), |args| {
                let class = get_class_rc_from_args(&args)?;
                let class_name = class.name.clone();
                let collection = class_name_to_collection(&class_name);
                let field = match args.get(1) {
                    Some(Value::String(s)) => s.clone(),
                    _ => return Err("find_by() expects string field name".to_string()),
                };
                let value = match args.get(2) {
                    Some(v) => super::value_to_json(v).map_err(|e| e.to_string())?,
                    None => return Err("find_by() requires a value".to_string()),
                };
                let sdbql = format!(
                    "FOR doc IN {} FILTER doc.{} == @val LIMIT 1 RETURN doc",
                    collection, field
                );
                let mut binds = std::collections::HashMap::new();
                binds.insert("val".to_string(), value);
                match super::crud::exec_with_auto_collection(sdbql, Some(binds), &collection) {
                    Ok(results) if !results.is_empty() => {
                        Ok(super::crud::json_doc_to_instance(&class, &results[0]))
                    }
                    _ => Ok(Value::Null),
                }
            })),
        );

        // Model.first_by(field, value) - Find first record with ordering
        native_static_methods.insert(
            "first_by".to_string(),
            Rc::new(NativeFunction::new("Model.first_by", Some(3), |args| {
                let class = get_class_rc_from_args(&args)?;
                let class_name = class.name.clone();
                let collection = class_name_to_collection(&class_name);
                let field = match args.get(1) {
                    Some(Value::String(s)) => s.clone(),
                    _ => return Err("first_by() expects string field name".to_string()),
                };
                let value = match args.get(2) {
                    Some(v) => super::value_to_json(v).map_err(|e| e.to_string())?,
                    None => return Err("first_by() requires a value".to_string()),
                };
                let sdbql = format!(
                    "FOR doc IN {} FILTER doc.{} == @val SORT doc._key ASC LIMIT 1 RETURN doc",
                    collection, field
                );
                let mut binds = std::collections::HashMap::new();
                binds.insert("val".to_string(), value);
                match super::crud::exec_with_auto_collection(sdbql, Some(binds), &collection) {
                    Ok(results) if !results.is_empty() => {
                        Ok(super::crud::json_doc_to_instance(&class, &results[0]))
                    }
                    _ => Ok(Value::Null),
                }
            })),
        );

        // Model.find_or_create_by(field, value, defaults) - Find or create record
        native_static_methods.insert(
            "find_or_create_by".to_string(),
            Rc::new(NativeFunction::new(
                "Model.find_or_create_by",
                Some(4),
                |args| {
                    let class = get_class_rc_from_args(&args)?;
                    let class_name = class.name.clone();
                    let collection = class_name_to_collection(&class_name);
                    let field = match args.get(1) {
                        Some(Value::String(s)) => s.clone(),
                        _ => {
                            return Err("find_or_create_by() expects string field name".to_string())
                        }
                    };
                    let value = args
                        .get(2)
                        .ok_or_else(|| "find_or_create_by() requires a value".to_string())?;
                    let json_val = super::value_to_json(value).map_err(|e| e.to_string())?;

                    // Try to find existing
                    let sdbql = format!(
                        "FOR doc IN {} FILTER doc.{} == @val LIMIT 1 RETURN doc",
                        collection, field
                    );
                    let mut binds = std::collections::HashMap::new();
                    binds.insert("val".to_string(), json_val.clone());
                    match super::crud::exec_with_auto_collection(sdbql, Some(binds), &collection) {
                        Ok(results) if !results.is_empty() => {
                            return Ok(super::crud::json_doc_to_instance(&class, &results[0]));
                        }
                        _ => {}
                    }

                    // Not found — create with defaults
                    let defaults = match args.get(3) {
                        Some(Value::Hash(hash)) => {
                            let mut map = serde_json::Map::new();
                            for (k, v) in hash.borrow().iter() {
                                if let crate::interpreter::value::HashKey::String(key) = k {
                                    if let Ok(jv) = super::value_to_json(v) {
                                        map.insert(key.clone(), jv);
                                    }
                                }
                            }
                            map
                        }
                        _ => serde_json::Map::new(),
                    };
                    let mut doc = defaults;
                    doc.insert(field, json_val);
                    match super::crud::exec_insert(
                        &collection,
                        None,
                        serde_json::Value::Object(doc),
                    ) {
                        Ok(result) => Ok(super::crud::json_doc_to_instance(&class, &result)),
                        Err(e) => Err(format!("find_or_create_by create failed: {}", e)),
                    }
                },
            )),
        );

        // Model.upsert(key, data) - Insert or update document
        native_static_methods.insert(
            "upsert".to_string(),
            Rc::new(NativeFunction::new("Model.upsert", Some(3), |args| {
                let class = get_class_rc_from_args(&args)?;
                let class_name = class.name.clone();
                let collection = class_name_to_collection(&class_name);
                let key = match args.get(1) {
                    Some(Value::String(s)) => s.clone(),
                    _ => return Err("upsert() expects string key".to_string()),
                };
                let data = match args.get(2) {
                    Some(Value::Hash(hash)) => {
                        let mut map = serde_json::Map::new();
                        for (k, v) in hash.borrow().iter() {
                            if let crate::interpreter::value::HashKey::String(k_str) = k {
                                if let Ok(jv) = super::value_to_json(v) {
                                    map.insert(k_str.clone(), jv);
                                }
                            }
                        }
                        serde_json::Value::Object(map)
                    }
                    _ => return Err("upsert() expects hash data".to_string()),
                };

                // Try update first, create if not found
                match exec_update(&collection, &key, data.clone(), true) {
                    Ok(result) => Ok(super::crud::json_doc_to_instance(&class, &result)),
                    Err(_) => {
                        // Insert with key
                        let mut obj = match data {
                            serde_json::Value::Object(m) => m,
                            _ => serde_json::Map::new(),
                        };
                        obj.insert("_key".to_string(), serde_json::Value::String(key));
                        match super::crud::exec_insert(
                            &collection,
                            None,
                            serde_json::Value::Object(obj),
                        ) {
                            Ok(result) => Ok(super::crud::json_doc_to_instance(&class, &result)),
                            Err(e) => Err(format!("upsert failed: {}", e)),
                        }
                    }
                }
            })),
        );

        // Model.create_many(array) - Batch insert
        native_static_methods.insert(
            "create_many".to_string(),
            Rc::new(NativeFunction::new("Model.create_many", Some(2), |args| {
                let _class = get_class_rc_from_args(&args)?;
                let class_name = match &args[0] {
                    Value::Class(c) => c.name.clone(),
                    _ => return Err("Expected class".to_string()),
                };
                let collection = class_name_to_collection(&class_name);
                let items = match args.get(1) {
                    Some(Value::Array(arr)) => arr.borrow().clone(),
                    _ => return Err("create_many() expects an array".to_string()),
                };

                let mut created = 0;
                for item in &items {
                    let doc = match item {
                        Value::Hash(hash) => {
                            let mut map = serde_json::Map::new();
                            for (k, v) in hash.borrow().iter() {
                                if let crate::interpreter::value::HashKey::String(k_str) = k {
                                    if let Ok(jv) = super::value_to_json(v) {
                                        map.insert(k_str.clone(), jv);
                                    }
                                }
                            }
                            serde_json::Value::Object(map)
                        }
                        _ => continue,
                    };
                    if super::crud::exec_insert(&collection, None, doc).is_ok() {
                        created += 1;
                    }
                }

                let mut result = crate::interpreter::value::HashPairs::default();
                result.insert(
                    crate::interpreter::value::HashKey::String("created".to_string()),
                    Value::Int(created),
                );
                Ok(Value::Hash(Rc::new(RefCell::new(result))))
            })),
        );

        // Model.scope(name, query_fn) - Register named scope (stores on class)
        native_static_methods.insert(
            "scope".to_string(),
            Rc::new(NativeFunction::new("Model.scope", Some(3), |_args| {
                // Scopes are a more advanced feature — just register for now
                Ok(Value::Null)
            })),
        );

        // Model.pluck(field, ...) - Convenience: creates QB and sets pluck fields
        native_static_methods.insert(
            "pluck".to_string(),
            Rc::new(NativeFunction::new("Model.pluck", None, |args| {
                let class = get_class_rc_from_args(&args)?;
                let class_name = class.name.clone();
                let collection = class_name_to_collection(&class_name);
                let mut fields = Vec::new();
                for arg in args.iter().skip(1) {
                    match arg {
                        Value::String(s) => fields.push(s.clone()),
                        _ => return Err("pluck() expects string field names".to_string()),
                    }
                }
                if fields.is_empty() {
                    return Err("pluck() requires at least one field name".to_string());
                }
                let mut qb =
                    super::query::QueryBuilder::new_with_class(class_name, collection, class);
                qb.set_pluck(fields);
                Ok(Value::QueryBuilder(Rc::new(RefCell::new(qb))))
            })),
        );

        // Model.sum(field), Model.avg(field), Model.min(field), Model.max(field) - Aggregations
        for (name, func) in &[
            ("sum", super::AggregationFunc::Sum),
            ("avg", super::AggregationFunc::Avg),
            ("min", super::AggregationFunc::Min),
            ("max", super::AggregationFunc::Max),
        ] {
            let method_name = name.to_string();
            let func = func.clone();
            native_static_methods.insert(
                method_name.clone(),
                Rc::new(NativeFunction::new(
                    Box::leak(format!("Model.{}", method_name).into_boxed_str()),
                    Some(2),
                    move |args| {
                        let class = get_class_rc_from_args(&args)?;
                        let class_name = class.name.clone();
                        let collection = class_name_to_collection(&class_name);
                        let field = match args.get(1) {
                            Some(Value::String(s)) => s.clone(),
                            _ => {
                                return Err(format!("{}() expects string field name", method_name))
                            }
                        };
                        let mut qb = super::query::QueryBuilder::new_with_class(
                            class_name, collection, class,
                        );
                        qb.aggregation = Some((func.clone(), field));
                        Ok(Value::QueryBuilder(Rc::new(RefCell::new(qb))))
                    },
                )),
            );
        }

        // Model.group_by(field, func, agg_field) - Group by with aggregation
        native_static_methods.insert(
            "group_by".to_string(),
            Rc::new(NativeFunction::new("Model.group_by", Some(4), |args| {
                let class = get_class_rc_from_args(&args)?;
                let class_name = class.name.clone();
                let collection = class_name_to_collection(&class_name);
                let group_field = match args.get(1) {
                    Some(Value::String(s)) => s.clone(),
                    _ => return Err("group_by() expects string group field".to_string()),
                };
                let func_name = match args.get(2) {
                    Some(Value::String(s)) => s.clone().to_lowercase(),
                    _ => return Err("group_by() expects string function name".to_string()),
                };
                let agg_field = match args.get(3) {
                    Some(Value::String(s)) => s.clone(),
                    _ => return Err("group_by() expects string aggregate field".to_string()),
                };
                let func = match func_name.as_str() {
                    "sum" => super::AggregationFunc::Sum,
                    "avg" => super::AggregationFunc::Avg,
                    "min" => super::AggregationFunc::Min,
                    "max" => super::AggregationFunc::Max,
                    _ => {
                        return Err(
                            "group_by() function must be one of: sum, avg, min, max".to_string()
                        )
                    }
                };
                let mut qb =
                    super::query::QueryBuilder::new_with_class(class_name, collection, class);
                qb.group_by_info = Some((group_field, func, agg_field));
                Ok(Value::QueryBuilder(Rc::new(RefCell::new(qb))))
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
            Rc::new(NativeFunction::new(
                "Model#update",
                None,
                #[allow(clippy::collapsible_match)]
                |args| {
                    use super::validation::run_validations;
                    use crate::interpreter::builtins::i18n::helpers as i18n_helpers;
                    use crate::interpreter::builtins::model::get_translated_fields;

                    let instance = match &args[0] {
                        Value::Instance(inst) => inst.clone(),
                        _ => return Err("Expected instance".to_string()),
                    };

                    // Optional hash of attributes: `inst.update({...})`
                    // applies the hash to instance fields before running
                    // the existing persist pipeline, so no-arg callers keep
                    // working unchanged.
                    match args.len() {
                        1 => {}
                        2 => apply_hash_to_instance(&instance, &args[1])?,
                        n => {
                            return Err(format!(
                                "update takes 0 or 1 arguments (a hash of attributes), got {}",
                                n - 1
                            ))
                        }
                    }

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

                    // Handle pending translations before updating
                    let translated_field_names = get_translated_fields(&class_name);
                    if !translated_field_names.is_empty() {
                        let locale = i18n_helpers::get_locale();

                        // Get or create translated_fields JSON structure
                        let mut translated_fields_json: serde_json::Map<String, serde_json::Value> =
                            serde_json::Map::new();

                        // If instance already has translated_fields, copy it
                        if let Some(tf) = inst_ref.get("translated_fields") {
                            if let Ok(tf_json) = value_to_json(&tf) {
                                if let serde_json::Value::Object(obj) = tf_json {
                                    translated_fields_json = obj;
                                }
                            }
                        }

                        // Get pending translations and merge them
                        if let Some(pending) = inst_ref.get("_pending_translations") {
                            if let Ok(pending_json) = value_to_json(&pending) {
                                if let serde_json::Value::Object(pending_obj) = pending_json {
                                    for field_name in &translated_field_names {
                                        if let Some(pending_value) = pending_obj.get(field_name) {
                                            // Get or create the locale object for this field
                                            let field_obj =
                                                translated_fields_json
                                                    .entry(field_name.clone())
                                                    .or_insert_with(|| {
                                                        serde_json::Value::Object(
                                                            serde_json::Map::new(),
                                                        )
                                                    });

                                            if let serde_json::Value::Object(ref mut locale_obj) =
                                                *field_obj
                                            {
                                                locale_obj
                                                    .insert(locale.clone(), pending_value.clone());
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // Update the instance's translated_fields field
                        drop(inst_ref);
                        let mut inst_mut = instance.borrow_mut();
                        inst_mut.fields.insert(
                            "translated_fields".to_string(),
                            json_to_value(&serde_json::Value::Object(translated_fields_json)),
                        );

                        // Clear pending translations
                        inst_mut.fields.remove("_pending_translations");
                        drop(inst_mut);
                    } else {
                        drop(inst_ref);
                    }

                    // Run validations
                    let inst_ref2 = instance.borrow();
                    let data_hash = instance_fields_to_hash(&inst_ref2);
                    let errors = run_validations(&class_name, &data_hash, Some(&key_str))?;
                    if !errors.is_empty() {
                        let error_values: Vec<Value> =
                            errors.iter().map(|e| e.to_value()).collect();
                        drop(inst_ref2);
                        instance.borrow_mut().set(
                            "_errors".to_string(),
                            Value::Array(Rc::new(RefCell::new(error_values))),
                        );
                        return Ok(Value::Bool(false));
                    }

                    let mut map = serde_json::Map::new();
                    for (k, v) in &inst_ref2.fields {
                        if !k.starts_with('_') {
                            map.insert(k.clone(), value_to_json(v)?);
                        }
                    }
                    drop(inst_ref2);
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
                },
            )),
        );

        // instance.save([hash]) - Insert or update depending on whether _key
        // exists. Optional hash argument applies bulk attribute assignments
        // before the persist pipeline, so zero-arg callers keep working.
        // Returns true on success, false on validation/DB error (errors stored in _errors)
        native_methods.insert(
            "save".to_string(),
            Rc::new(NativeFunction::new(
                "Model#save",
                None,
                #[allow(clippy::collapsible_match)]
                |args| {
                    use super::validation::run_validations;
                    use crate::interpreter::builtins::i18n::helpers as i18n_helpers;
                    use crate::interpreter::builtins::model::get_translated_fields;

                    let instance = match &args[0] {
                        Value::Instance(inst) => inst.clone(),
                        _ => return Err("Expected instance".to_string()),
                    };

                    // Optional hash of attributes: `inst.save({...})` applies
                    // the hash to instance fields before running the existing
                    // persist pipeline, so no-arg callers keep working.
                    match args.len() {
                        1 => {}
                        2 => apply_hash_to_instance(&instance, &args[1])?,
                        n => {
                            return Err(format!(
                                "save takes 0 or 1 arguments (a hash of attributes), got {}",
                                n - 1
                            ))
                        }
                    }

                    let inst_ref = instance.borrow();
                    let class_name = inst_ref.class.name.clone();
                    let collection = class_name_to_collection(&class_name);
                    let key_opt = inst_ref.get("_key").and_then(|k| match k {
                        Value::String(s) => Some(s.clone()),
                        _ => None,
                    });

                    // Handle pending translations before saving
                    let translated_field_names = get_translated_fields(&class_name);
                    let has_translations = !translated_field_names.is_empty();

                    // Pre-compute translation data while we have inst_ref
                    let translation_update: Option<(
                        serde_json::Map<String, serde_json::Value>,
                        Vec<String>,
                    )> = if has_translations {
                        let locale = i18n_helpers::get_locale();

                        // Get or create translated_fields JSON structure
                        let mut translated_fields_json: serde_json::Map<String, serde_json::Value> =
                            serde_json::Map::new();

                        // If instance already has translated_fields, copy it
                        if let Some(tf) = inst_ref.get("translated_fields") {
                            if let Ok(tf_json) = value_to_json(&tf) {
                                if let serde_json::Value::Object(obj) = tf_json {
                                    translated_fields_json = obj;
                                }
                            }
                        }

                        // Get pending translations and merge them
                        if let Some(pending) = inst_ref.get("_pending_translations") {
                            if let Ok(pending_json) = value_to_json(&pending) {
                                if let serde_json::Value::Object(pending_obj) = pending_json {
                                    for field_name in &translated_field_names {
                                        if let Some(pending_value) = pending_obj.get(field_name) {
                                            // Get or create the locale object for this field
                                            let field_obj =
                                                translated_fields_json
                                                    .entry(field_name.clone())
                                                    .or_insert_with(|| {
                                                        serde_json::Value::Object(
                                                            serde_json::Map::new(),
                                                        )
                                                    });

                                            if let serde_json::Value::Object(ref mut locale_obj) =
                                                *field_obj
                                            {
                                                locale_obj
                                                    .insert(locale.clone(), pending_value.clone());
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        Some((translated_fields_json, translated_field_names))
                    } else {
                        None
                    };

                    // Get data we need before dropping inst_ref
                    let data_hash = instance_fields_to_hash(&inst_ref);

                    // Build map for DB operation before dropping inst_ref
                    let mut map = serde_json::Map::new();
                    for (k, v) in &inst_ref.fields {
                        if !k.starts_with('_') {
                            map.insert(k.clone(), value_to_json(v)?);
                        }
                    }

                    // Now we can drop inst_ref
                    drop(inst_ref);

                    // Apply translation update if needed
                    if let Some((translated_fields_json, _)) = translation_update {
                        let mut inst_mut = instance.borrow_mut();
                        inst_mut.fields.insert(
                            "translated_fields".to_string(),
                            json_to_value(&serde_json::Value::Object(translated_fields_json)),
                        );
                        // Clear pending translations
                        inst_mut.fields.remove("_pending_translations");
                    }

                    // Run validations
                    let errors = run_validations(&class_name, &data_hash, key_opt.as_deref())?;
                    if !errors.is_empty() {
                        let error_values: Vec<Value> =
                            errors.iter().map(|e| e.to_value()).collect();
                        instance.borrow_mut().set(
                            "_errors".to_string(),
                            Value::Array(Rc::new(RefCell::new(error_values))),
                        );
                        return Ok(Value::Bool(false));
                    }

                    if let Some(ref key_str) = key_opt {
                        // Update existing document
                        match exec_update(
                            &collection,
                            key_str,
                            serde_json::Value::Object(map),
                            true,
                        ) {
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
                        match exec_insert(&collection, None, serde_json::Value::Object(map)) {
                            Ok(result) => {
                                let mut inst_mut = instance.borrow_mut();
                                if let serde_json::Value::Object(ref res_map) = result {
                                    for field in
                                        &["_key", "_id", "_rev", "_created_at", "_updated_at"]
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
                },
            )),
        );

        // instance.delete() - Delete (or soft-delete) the document from DB
        native_methods.insert(
            "delete".to_string(),
            Rc::new(NativeFunction::new("Model#delete", Some(0), |args| {
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
                drop(inst_ref);

                if is_soft_delete(&class_name) {
                    // Soft delete: set deleted_at timestamp
                    let now = chrono::Utc::now().to_rfc3339();
                    let mut map = serde_json::Map::new();
                    map.insert(
                        "deleted_at".to_string(),
                        serde_json::Value::String(now.clone()),
                    );
                    match exec_update(&collection, &key_str, serde_json::Value::Object(map), true) {
                        Ok(_) => {
                            instance
                                .borrow_mut()
                                .set("deleted_at".to_string(), Value::String(now));
                            Ok(Value::Bool(true))
                        }
                        Err(e) => Ok(Value::String(format!("Error: {}", e))),
                    }
                } else {
                    match exec_delete(&collection, &key_str) {
                        Ok(result) => Ok(json_to_value(&result)),
                        Err(e) => Ok(Value::String(format!("Error: {}", e))),
                    }
                }
            })),
        );

        // instance.restore() - Restore a soft-deleted record (clear deleted_at)
        native_methods.insert(
            "restore".to_string(),
            Rc::new(NativeFunction::new("Model#restore", Some(0), |args| {
                let instance = match &args[0] {
                    Value::Instance(inst) => inst.clone(),
                    _ => return Err("Expected instance".to_string()),
                };
                let inst_ref = instance.borrow();
                let collection = class_name_to_collection(&inst_ref.class.name);
                let key = match inst_ref.get("_key") {
                    Some(k) => k,
                    None => return Ok(Value::Instance(instance.clone())),
                };
                let key_str = match key {
                    Value::String(s) => s,
                    _ => return Err("_key is not a string".to_string()),
                };
                drop(inst_ref);

                let mut map = serde_json::Map::new();
                map.insert("deleted_at".to_string(), serde_json::Value::Null);
                match exec_update(&collection, &key_str, serde_json::Value::Object(map), true) {
                    Ok(_) => {
                        instance
                            .borrow_mut()
                            .set("deleted_at".to_string(), Value::Null);
                        Ok(Value::Bool(true))
                    }
                    Err(e) => Err(format!("restore failed: {}", e)),
                }
            })),
        );

        // instance.increment(field, amount?) - Increment a numeric field
        native_methods.insert(
            "increment".to_string(),
            Rc::new(NativeFunction::new("Model#increment", None, |args| {
                let instance = match &args[0] {
                    Value::Instance(inst) => inst.clone(),
                    _ => return Err("Expected instance".to_string()),
                };
                let field = match args.get(1) {
                    Some(Value::String(s)) => s.clone(),
                    _ => return Err("increment() expects a string field name".to_string()),
                };
                let amount = match args.get(2) {
                    Some(Value::Int(n)) => *n,
                    Some(Value::Float(n)) => *n as i64,
                    None => 1,
                    _ => return Err("increment() amount must be a number".to_string()),
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
                let current = match inst_ref.get(&field) {
                    Some(Value::Int(n)) => n,
                    Some(Value::Float(n)) => n as i64,
                    _ => 0,
                };
                drop(inst_ref);

                let new_value = current + amount;
                let mut map = serde_json::Map::new();
                map.insert(
                    field.clone(),
                    serde_json::Value::Number(serde_json::Number::from(new_value)),
                );
                match exec_update(&collection, &key_str, serde_json::Value::Object(map), true) {
                    Ok(_) => {
                        instance.borrow_mut().set(field, Value::Int(new_value));
                        Ok(Value::Instance(instance))
                    }
                    Err(e) => Err(format!("increment failed: {}", e)),
                }
            })),
        );

        // instance.decrement(field, amount?) - Decrement a numeric field
        native_methods.insert(
            "decrement".to_string(),
            Rc::new(NativeFunction::new("Model#decrement", None, |args| {
                let instance = match &args[0] {
                    Value::Instance(inst) => inst.clone(),
                    _ => return Err("Expected instance".to_string()),
                };
                let field = match args.get(1) {
                    Some(Value::String(s)) => s.clone(),
                    _ => return Err("decrement() expects a string field name".to_string()),
                };
                let amount = match args.get(2) {
                    Some(Value::Int(n)) => *n,
                    Some(Value::Float(n)) => *n as i64,
                    None => 1,
                    _ => return Err("decrement() amount must be a number".to_string()),
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
                let current = match inst_ref.get(&field) {
                    Some(Value::Int(n)) => n,
                    Some(Value::Float(n)) => n as i64,
                    _ => 0,
                };
                drop(inst_ref);

                let new_value = current - amount;
                let mut map = serde_json::Map::new();
                map.insert(
                    field.clone(),
                    serde_json::Value::Number(serde_json::Number::from(new_value)),
                );
                match exec_update(&collection, &key_str, serde_json::Value::Object(map), true) {
                    Ok(_) => {
                        instance.borrow_mut().set(field, Value::Int(new_value));
                        Ok(Value::Instance(instance))
                    }
                    Err(e) => Err(format!("decrement failed: {}", e)),
                }
            })),
        );

        // instance.touch() - Update the _updated_at timestamp
        native_methods.insert(
            "touch".to_string(),
            Rc::new(NativeFunction::new("Model#touch", Some(0), |args| {
                let instance = match &args[0] {
                    Value::Instance(inst) => inst.clone(),
                    _ => return Err("Expected instance".to_string()),
                };
                let inst_ref = instance.borrow();
                let collection = class_name_to_collection(&inst_ref.class.name);
                let key = match inst_ref.get("_key") {
                    Some(k) => k,
                    None => return Ok(Value::Instance(instance.clone())),
                };
                let key_str = match key {
                    Value::String(s) => s,
                    _ => return Err("_key is not a string".to_string()),
                };
                drop(inst_ref);

                let now = chrono::Utc::now().to_rfc3339();
                let mut map = serde_json::Map::new();
                map.insert(
                    "_updated_at".to_string(),
                    serde_json::Value::String(now.clone()),
                );
                match exec_update(&collection, &key_str, serde_json::Value::Object(map), true) {
                    Ok(_) => {
                        instance
                            .borrow_mut()
                            .set("_updated_at".to_string(), Value::String(now));
                        Ok(Value::Instance(instance))
                    }
                    Err(e) => Err(format!("touch failed: {}", e)),
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
            methods: Rc::new(RefCell::new(HashMap::new())),
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

    // soft_delete - Mark a model as using soft delete
    env.define(
        "soft_delete".to_string(),
        Value::NativeFunction(NativeFunction::new("soft_delete", Some(1), |args| {
            let class_name = get_class_name_from_class(&args)?;
            let mut metadata = get_or_create_metadata(&class_name);
            metadata.soft_delete = true;
            update_metadata(&class_name, metadata);
            Ok(Value::Null)
        })),
    );

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
                    None,
                    None,
                );
                register_relation(&class_name, relation);
                Ok(Value::Null)
            })),
        );
    }
}
