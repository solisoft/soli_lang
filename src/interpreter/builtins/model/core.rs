//! Core Model types, registry, and database configuration.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, Function, NativeFunction, Value};

pub use super::db_config::{
    db_url, force_refresh_jwt_token, get_api_key, get_basic_auth, get_cursor_url,
    get_database_name, get_jwt_token, init_db_config, init_jwt_token, DbConfig, DB_CONFIG,
};
pub use super::engine_context::{
    get_model_engine_context, set_model_engine_context, EngineContextGuard,
};
pub use super::registry::{
    get_accessible_attributes, get_or_create_metadata, get_translated_fields, is_soft_delete,
    is_translated_field, register_accessible_attributes, register_translation, update_metadata,
    ModelMetadata, MODEL_REGISTRY,
};

use super::callbacks::register_callback;
use super::relations::{
    build_habtm_relation, build_relation, get_relation, parse_relation_options, register_relation,
    RelationType,
};
use super::uploaders::{default_collection, get_uploader, register_uploader, UploaderConfig};
use super::validation::{parse_validates_options, register_validation_with_conditions};

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
                    Err(e) => Ok(Value::String(format!("Error: {}", e).into())),
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
                    Err(e) => Ok(Value::String(format!("Error: {}", e).into())),
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
                    Err(e) => Ok(Value::String(format!("Error: {}", e).into())),
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
                    Err(e) => Ok(Value::String(format!("Error: {}", e).into())),
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
                    Err(e) => Ok(Value::String(format!("Error: {}", e).into())),
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
                    Err(e) => Ok(Value::String(format!("Error: {}", e).into())),
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
///
/// STI: a model inheriting from another model shares its base's collection
/// ("Admin" < "User" → "users"), so the name is resolved to the hierarchy
/// root before deriving the collection.
pub fn class_name_to_collection(name: &str) -> String {
    let base = super::registry::sti_base(name);
    class_name_to_collection_with_engine(&base, get_model_engine_context().as_deref())
}

pub fn class_name_to_collection_with_engine(name: &str, engine: Option<&str>) -> String {
    let base = compute_base_collection_name(name);

    match engine {
        Some(e) => format!("{}_{}", e, base),
        None => base,
    }
}

/// STI: the FILTER clause scoping a raw (non-QueryBuilder) query on an STI
/// subclass to its own type + descendants. Empty for non-STI classes (the
/// base class deliberately matches every row, Rails-style).
pub(crate) fn sti_scope_clause(class_name: &str) -> String {
    if !super::registry::is_sti_subclass(class_name) {
        return String::new();
    }
    let quoted: Vec<String> = super::registry::sti_type_names(class_name)
        .into_iter()
        .map(|t| format!("\"{}\"", t))
        .collect();
    format!(" FILTER doc.type IN [{}]", quoted.join(", "))
}

/// STI: does a fetched row belong to `class_name`'s hierarchy? Non-STI
/// classes match everything; subclass matches require the row's `type` to
/// name the class or one of its descendants.
pub(crate) fn sti_row_matches(class_name: &str, doc: &serde_json::Value) -> bool {
    if !super::registry::is_sti_subclass(class_name) {
        return true;
    }
    doc.get("type")
        .and_then(|v| v.as_str())
        .map(|t| {
            super::registry::sti_type_names(class_name)
                .iter()
                .any(|n| n == t)
        })
        .unwrap_or(false)
}

fn compute_base_collection_name(name: &str) -> String {
    let name = name.strip_suffix("Model").unwrap_or(name);

    let mut snake = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            snake.push('_');
        }
        snake.push(c.to_lowercase().next().unwrap());
    }
    crate::inflect::pluralize(&snake)
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;

    use crate::interpreter::value::Value;

    #[test]
    fn string_form_accepts_array_of_scalars() {
        let v = Value::Array(std::rc::Rc::new(std::cell::RefCell::new(vec![
            Value::String("a".into()),
            Value::String("b".into()),
        ])));
        let json = ensure_string_form_bind_value(&v, "ids", "where").unwrap();
        assert!(json.is_array());
        assert_eq!(json.as_array().unwrap().len(), 2);
    }

    #[test]
    fn string_form_accepts_empty_array() {
        let v = Value::Array(std::rc::Rc::new(std::cell::RefCell::new(vec![])));
        ensure_string_form_bind_value(&v, "ids", "where").unwrap();
    }

    #[test]
    fn string_form_accepts_scalar() {
        let v = Value::String("a".into());
        ensure_string_form_bind_value(&v, "id", "where").unwrap();
    }

    #[test]
    fn string_form_rejects_array_of_arrays() {
        let inner = Value::Array(std::rc::Rc::new(std::cell::RefCell::new(vec![
            Value::String("a".into()),
        ])));
        let v = Value::Array(std::rc::Rc::new(std::cell::RefCell::new(vec![inner])));
        let err = ensure_string_form_bind_value(&v, "ids", "where").unwrap_err();
        assert!(err.contains("element 0 is not a scalar"), "got: {}", err);
    }

    #[test]
    fn string_form_rejects_top_level_object() {
        let v = Value::Hash(std::rc::Rc::new(std::cell::RefCell::new(
            crate::interpreter::value::HashPairs::default(),
        )));
        let err = ensure_string_form_bind_value(&v, "f", "where").unwrap_err();
        assert!(err.contains("must be a scalar or an array of scalars"));
    }

    #[test]
    fn hash_form_still_rejects_arrays() {
        let v = Value::Array(std::rc::Rc::new(std::cell::RefCell::new(vec![
            Value::String("a".into()),
        ])));
        let err = ensure_scalar_bind_value(&v, "ids", "where").unwrap_err();
        assert!(err.contains("must be a scalar"));
    }

    #[test]
    fn empty_hash_filter_is_a_noop_not_an_error() {
        // `where({})` must not raise — it produces an empty filter so callers
        // skip the FILTER clause and match all rows.
        let hash = std::rc::Rc::new(std::cell::RefCell::new(
            crate::interpreter::value::HashPairs::default(),
        ));
        let (filter, binds) = build_safe_filter_from_hash(&hash, "where").unwrap();
        assert_eq!(filter, "");
        assert!(binds.is_empty());
    }

    #[test]
    fn test_compute_base_collection_name() {
        assert_eq!(compute_base_collection_name("User"), "users");
        assert_eq!(compute_base_collection_name("BlogPost"), "blog_posts");
        assert_eq!(compute_base_collection_name("UserProfile"), "user_profiles");
        assert_eq!(compute_base_collection_name("CustomerModel"), "customers");
        assert_eq!(
            compute_base_collection_name("ProductCategory"),
            "product_categories"
        );
        assert_eq!(compute_base_collection_name("Category"), "categories");
        assert_eq!(compute_base_collection_name("Box"), "boxes");
        assert_eq!(compute_base_collection_name("Person"), "people");
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

/// Validate the `order(field, direction)` direction argument. Accepts
/// only `asc`/`desc`/`ascending`/`descending` (case-insensitive); any
/// other value is rejected before it reaches the QueryBuilder. Today
/// the SORT-clause builder already coerces unknown directions to
/// `ASC`, so this is fail-fast / defense-in-depth rather than a
/// runtime exploit fix — it ensures a refactor of the downstream match
/// can't silently re-introduce direction injection.
pub fn validate_order_direction(direction: &str, method: &str) -> Result<(), String> {
    match direction.to_ascii_lowercase().as_str() {
        "asc" | "desc" | "ascending" | "descending" => Ok(()),
        _ => Err(format!(
            "{}() direction must be one of asc/desc/ascending/descending — got {:?}",
            method, direction
        )),
    }
}

/// Build a safe `FILTER` clause from a `{field: value, ...}` Hash. Each
/// key is validated through `validate_field_name`, and each value is
/// pushed into the AQL bind map (so attacker-controlled values can never
/// reach the query template). The returned tuple is
/// `(filter_string, bind_map)` ready to be set on a `QueryBuilder`.
///
/// This is the safe alternative to the raw-string form
/// `where("doc.foo == @foo", {foo: ...})`, which the docs flag as
/// developer-trusted.
pub fn build_safe_filter_from_hash(
    hash: &Rc<RefCell<crate::interpreter::value::HashPairs>>,
    method: &str,
) -> Result<(String, std::collections::HashMap<String, serde_json::Value>), String> {
    use crate::interpreter::value::HashKey;
    let pairs = hash.borrow();
    if pairs.is_empty() {
        // An empty hash filter is a no-op: `where({})` adds no constraint and
        // matches all rows (mirroring no `.where` at all). Return an empty
        // filter string so callers simply skip building a FILTER clause rather
        // than raising.
        let _ = method;
        return Ok((String::new(), std::collections::HashMap::new()));
    }
    let mut clauses = Vec::with_capacity(pairs.len());
    let mut binds = std::collections::HashMap::new();
    for (k, v) in pairs.iter() {
        let key = match k {
            HashKey::String(s) => s.clone(),
            _ => return Err(format!("{}() Hash filter keys must be strings", method)),
        };
        validate_field_name(&key, method)?;
        // Bind name shadows the field name (`@email` for key `"email"`).
        // Field names are already sanitized to safe identifiers, so they
        // make valid bind names too. Callers that supply the legacy raw
        // string form take a separate code path, so there's no risk of
        // colliding bind namespaces between the two forms.
        clauses.push(format!("doc.{0} == @{0}", key));
        let json_val = ensure_scalar_bind_value(v, &key, method)?;
        binds.insert(key.to_string(), json_val);
    }
    Ok((clauses.join(" AND "), binds))
}

/// SEC-062: enforce that user-supplied bind values are scalars (string, number,
/// bool, null). Used by the safe hash form `where({field: val})`, which builds
/// `doc.field == @field` AQL — arrays/objects against `==` produce surprising
/// semantics and are almost always a mistake.
pub fn ensure_scalar_bind_value(
    value: &Value,
    key: &str,
    method: &str,
) -> Result<serde_json::Value, String> {
    use crate::interpreter::value::value_to_json;
    let json_val = value_to_json(value).map_err(|e| e.to_string())?;
    match json_val {
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => Ok(json_val),
        _ => Err(format!(
            "{}() bind value for '{}' must be a scalar (string, number, bool, null); \
             got {}. Pass complex values via raw AQL strings instead.",
            method, key, json_val
        )),
    }
}

/// Bind-value validator for the developer-trusted string form
/// `where("doc.field IN @ids", { "ids": [...] })`. Caller wrote the AQL, so
/// arrays are a legitimate shape (for `IN`, `ANY`, `ALL`). Allows scalars and
/// arrays-of-scalars one level deep; rejects nested arrays and objects, which
/// have no clean AQL bind interpretation and belong in `@sdbql{}` instead.
pub fn ensure_string_form_bind_value(
    value: &Value,
    key: &str,
    method: &str,
) -> Result<serde_json::Value, String> {
    use crate::interpreter::value::value_to_json;
    let json_val = value_to_json(value).map_err(|e| e.to_string())?;
    match &json_val {
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => Ok(json_val),
        serde_json::Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                match item {
                    serde_json::Value::Null
                    | serde_json::Value::Bool(_)
                    | serde_json::Value::Number(_)
                    | serde_json::Value::String(_) => {}
                    _ => {
                        return Err(format!(
                            "{}() bind value for '{}' is an array, but element {} is not a scalar \
                             (string, number, bool, null). Nested structures must go through \
                             raw AQL strings (`@sdbql{{...}}`).",
                            method, key, i
                        ));
                    }
                }
            }
            Ok(json_val)
        }
        serde_json::Value::Object(_) => Err(format!(
            "{}() bind value for '{}' must be a scalar or an array of scalars; \
             got an object. Pass complex values via raw AQL strings (`@sdbql{{...}}`) instead.",
            method, key
        )),
    }
}

/// Validate that a string is a safe AQL identifier before it's
/// `format!`-interpolated into a query template such as
/// `FOR doc IN ... FILTER doc.{field} == @val` or
/// `SORT doc.{field}`.
///
/// Pattern: `^[A-Za-z_][A-Za-z0-9_]*$` — letter-or-underscore first
/// char, then letters/digits/underscores. Any other character (including
/// dots, spaces, quotes, semicolons, parens, AQL keywords) is rejected
/// so a controller calling `User.find_by(req["params"]["field"], v)`
/// can't smuggle in `1==1 RETURN doc REMOVE doc` etc.
///
/// `method` is the user-facing call name (e.g. `"find_by"`) so the error
/// message points the developer at the right line.
pub fn validate_field_name(field: &str, method: &str) -> Result<(), String> {
    let mut chars = field.chars();
    let first_ok = matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_');
    if !first_ok {
        return Err(format!(
            "{}() field name must start with a letter or underscore — got {:?}",
            method, field
        ));
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(format!(
            "{}() field name may only contain letters, digits, and underscores — got {:?}",
            method, field
        ));
    }
    Ok(())
}

/// Validate a retention/duration string of the form `<number><unit>` with
/// unit s/m/h/d/w (e.g. "30d", "90m"). Used by the `timeseries` DSL and
/// `Model.prune`.
pub fn validate_retention_duration(value: &str) -> Result<(), String> {
    let (digits, unit) = value.split_at(value.len().saturating_sub(1));
    let unit_ok = matches!(unit, "s" | "m" | "h" | "d" | "w");
    let digits_ok = !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit());
    if unit_ok && digits_ok && digits.parse::<u64>().map(|n| n > 0).unwrap_or(false) {
        Ok(())
    } else {
        Err(format!(
            "invalid duration {:?}: expected <number><unit> with unit s/m/h/d/w, e.g. \"30d\"",
            value
        ))
    }
}

/// The standard insert-only violation message for timeseries models. Mirrors
/// the DB's own restriction (updates/upserts rejected; deletes allowed) but
/// fails before any DB round trip with an actionable message.
pub fn timeseries_insert_only_error(class_name: &str, op: &str) -> String {
    format!(
        "{} is a timeseries model: records are insert-only. {} is not supported — \
         use prune() for retention.",
        class_name, op
    )
}

/// Convert a duration string (validated by `validate_retention_duration`)
/// into an RFC3339 cutoff timestamp `now - duration`.
pub fn duration_to_cutoff_rfc3339(value: &str) -> Result<String, String> {
    validate_retention_duration(value)?;
    let (digits, unit) = value.split_at(value.len() - 1);
    let n: u64 = digits
        .parse()
        .map_err(|_| format!("invalid duration {:?}", value))?;
    let seconds = match unit {
        "s" => n,
        "m" => n * 60,
        "h" => n * 3600,
        "d" => n * 86_400,
        "w" => n * 604_800,
        _ => unreachable!(),
    };
    let cutoff = chrono::Utc::now() - chrono::Duration::seconds(seconds as i64);
    Ok(cutoff.to_rfc3339())
}

/// Collect field names from `attr_accessible(...)` arguments. Accepts
/// either a single Array of strings (`attr_accessible(["a", "b"])`) or a
/// variadic string list (`attr_accessible("a", "b")`) — both forms read
/// naturally in Soli code. Empty (`attr_accessible()`) is allowed and
/// means "no field is mass-assignable", which is a useful lock-down.
fn collect_accessible_fields(args: &[Value]) -> Result<Vec<String>, String> {
    if args.len() == 1 {
        if let Value::Array(arr) = &args[0] {
            let arr = arr.borrow();
            let mut out = Vec::with_capacity(arr.len());
            for v in arr.iter() {
                match v {
                    Value::String(s) => out.push(s.to_string()),
                    other => {
                        return Err(format!(
                            "attr_accessible() expects string field names, got {} in array",
                            other.type_name()
                        ))
                    }
                }
            }
            return Ok(out);
        }
    }
    let mut out = Vec::with_capacity(args.len());
    for v in args {
        match v {
            Value::String(s) => out.push(s.to_string()),
            other => {
                return Err(format!(
                    "attr_accessible() expects string field names, got {}",
                    other.type_name()
                ))
            }
        }
    }
    Ok(out)
}

/// Build an `UploaderConfig` from `uploader(class, name, options_hash)` args.
/// The class is `args[0]`, the field name is `args[1]`, the options hash is
/// `args[2]`.
fn build_uploader_config_from_args(
    class_name: &str,
    args: &[Value],
) -> Result<UploaderConfig, String> {
    use crate::interpreter::value::HashKey;

    let name = match args.get(1) {
        Some(Value::String(s)) => s.clone(),
        Some(other) => {
            return Err(format!(
                "uploader() expects string field name, got {}",
                other.type_name()
            ))
        }
        None => return Err("uploader() requires a field name".to_string()),
    };

    let options = match args.get(2) {
        Some(Value::Hash(hash)) => hash.borrow().clone(),
        Some(other) => {
            return Err(format!(
                "uploader() expects an options hash, got {}",
                other.type_name()
            ))
        }
        None => return Err("uploader() requires an options hash".to_string()),
    };

    let mut multiple = false;
    let mut content_types: Vec<String> = Vec::new();
    let mut max_size: Option<u64> = None;
    let mut collection: Option<String> = None;
    let mut format: Option<String> = None;
    let mut quality: Option<u8> = None;
    let mut max_width: Option<u32> = None;
    let mut max_height: Option<u32> = None;

    for (k, v) in options {
        if let HashKey::String(key) = k {
            match key.as_ref() {
                "multiple" => {
                    if let Value::Bool(b) = v {
                        multiple = b;
                    }
                }
                "content_types" => {
                    if let Value::Array(arr) = v {
                        for item in arr.borrow().iter() {
                            if let Value::String(s) = item {
                                content_types.push(s.clone().to_string());
                            }
                        }
                    }
                }
                "max_size" => match v {
                    Value::Int(n) if n >= 0 => max_size = Some(n as u64),
                    Value::Int(_) => {
                        return Err("uploader() max_size must be non-negative".to_string())
                    }
                    _ => {}
                },
                "collection" => {
                    if let Value::String(s) = v {
                        collection = Some(s.to_string());
                    }
                }
                "format" => {
                    if let Value::String(s) = v {
                        let normalized = s.to_lowercase();
                        let canonical = match normalized.as_str() {
                            "jpg" | "jpeg" => "jpeg",
                            "png" => "png",
                            "webp" => "webp",
                            _ => {
                                return Err(format!(
                                    "uploader(\"{}\") format must be \"jpeg\", \"png\", or \"webp\", got {:?}",
                                    name, s
                                ))
                            }
                        };
                        format = Some(canonical.to_string());
                    }
                }
                "quality" => match v {
                    Value::Int(n) if (1..=100).contains(&n) => quality = Some(n as u8),
                    Value::Int(_) => {
                        return Err(format!(
                            "uploader(\"{}\") quality must be between 1 and 100",
                            name
                        ))
                    }
                    _ => {}
                },
                "max_width" => match v {
                    Value::Int(n) if n > 0 => max_width = Some(n as u32),
                    Value::Int(_) => {
                        return Err(format!(
                            "uploader(\"{}\") max_width must be a positive integer",
                            name
                        ))
                    }
                    _ => {}
                },
                "max_height" => match v {
                    Value::Int(n) if n > 0 => max_height = Some(n as u32),
                    Value::Int(_) => {
                        return Err(format!(
                            "uploader(\"{}\") max_height must be a positive integer",
                            name
                        ))
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }

    if content_types.is_empty() {
        return Err(format!(
            "uploader(\"{}\") requires a non-empty content_types array",
            name
        ));
    }
    let max_size =
        max_size.ok_or_else(|| format!("uploader(\"{}\") requires a max_size (bytes)", name))?;
    let collection = collection.unwrap_or_else(|| default_collection(class_name, &name));

    Ok(UploaderConfig {
        name: name.to_string(),
        multiple,
        content_types,
        max_size,
        collection,
        format,
        quality,
        max_width,
        max_height,
    })
}

/// Convert an `UploaderConfig` (or `None`) to a Soli `Value` so Soli code can
/// inspect it. `None` becomes `Value::Null`.
fn uploader_config_to_value(config: Option<UploaderConfig>) -> Value {
    use crate::interpreter::value::{HashKey, HashPairs};

    let Some(c) = config else {
        return Value::Null;
    };
    let mut pairs: HashPairs = HashPairs::default();
    pairs.insert(HashKey::String("name".into()), Value::String(c.name.into()));
    pairs.insert(HashKey::String("multiple".into()), Value::Bool(c.multiple));
    let cts: Vec<Value> = c
        .content_types
        .into_iter()
        .map(|s| Value::String(s.into()))
        .collect();
    pairs.insert(
        HashKey::String("content_types".into()),
        Value::Array(Rc::new(RefCell::new(cts))),
    );
    pairs.insert(
        HashKey::String("max_size".into()),
        Value::Int(c.max_size as i64),
    );
    pairs.insert(
        HashKey::String("collection".into()),
        Value::String(c.collection.into()),
    );
    pairs.insert(
        HashKey::String("format".into()),
        c.format
            .map(|s| Value::String(s.into()))
            .unwrap_or(Value::Null),
    );
    pairs.insert(
        HashKey::String("quality".into()),
        c.quality
            .map(|q| Value::Int(q as i64))
            .unwrap_or(Value::Null),
    );
    pairs.insert(
        HashKey::String("max_width".into()),
        c.max_width
            .map(|w| Value::Int(w as i64))
            .unwrap_or(Value::Null),
    );
    pairs.insert(
        HashKey::String("max_height".into()),
        c.max_height
            .map(|h| Value::Int(h as i64))
            .unwrap_or(Value::Null),
    );
    Value::Hash(Rc::new(RefCell::new(pairs)))
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
            pairs.insert(HashKey::String(k.clone().into()), v.clone());
        }
    }
    Value::Hash(Rc::new(RefCell::new(pairs)))
}

/// Filter a `Value::Hash` to the model's `attr_accessible` whitelist for
/// mass-assign paths (`Model.create`, `Model.update(id, hash)`,
/// `instance.update(hash)`, `instance.save(hash)`).
///
/// - When the model never declared `attr_accessible(...)`, the hash is
///   returned unchanged. This keeps every existing app working — the
///   filter is opt-in per model.
/// - When `attr_accessible([...])` was declared, any key not in the list
///   is silently dropped. `_`-prefixed framework keys are dropped too,
///   independent of the whitelist, so they can never be smuggled in via
///   the request body.
/// - Non-Hash inputs are passed through untouched; the caller already
///   handles the type error elsewhere with a more specific message.
///
/// We always allocate a fresh `Value::Hash` rather than mutating in place
/// so the caller's original input is preserved (validation and error
/// reporting still see the request's full shape if they want it).
fn filter_mass_assign(class_name: &str, data: &Value) -> Value {
    use crate::interpreter::value::{HashKey, HashPairs};
    let pairs = match data {
        Value::Hash(p) => p,
        _ => return data.clone(),
    };
    let whitelist = match get_accessible_attributes(class_name) {
        Some(list) => list,
        None => return data.clone(),
    };
    // Tiny lists stay as Vec; HashSet would be overkill here (typical
    // attr_accessible call lists 5–15 fields).
    let mut filtered: HashPairs = HashPairs::default();
    let mut dropped: Vec<String> = Vec::new();
    for (k, v) in pairs.borrow().iter() {
        if let HashKey::String(field) = k {
            if field.starts_with('_') {
                continue;
            }
            if whitelist.iter().any(|w| **w == **field) {
                filtered.insert(k.clone(), v.clone());
            } else {
                dropped.push(field.to_string());
            }
        }
    }
    // The silent-intersection trap: a controller `permit`s a key the model's
    // whitelist doesn't list, and the value vanishes here with no error.
    // Surface the drop in dev mode so the drift is visible when it bites.
    if !dropped.is_empty() && crate::interpreter::builtins::template::is_dev_mode() {
        eprintln!(
            "[WARN] attr_accessible on {} dropped mass-assign key(s): {} — add them to the model's whitelist or remove them from the controller's permit()",
            class_name,
            dropped.join(", ")
        );
    }
    Value::Hash(Rc::new(RefCell::new(filtered)))
}

/// Apply every entry of a `Value::Hash` onto an instance's fields,
/// matching direct assignment (`inst.field = ...`). Framework-internal
/// `_`-prefixed keys (`_key`, `_errors`, `_pending_translations`) are
/// skipped — callers must never overwrite those via bulk update.
///
/// If the instance's class declared `attr_accessible(...)`, the hash is
/// filtered to the whitelist *before* assignment so a passing client
/// can't smuggle in `role`/`is_admin`/etc. via `instance.update(req)` or
/// `instance.save(req)`.
///
/// Non-Hash argument returns an error; non-String hash keys are silently
/// skipped (instances only have string-keyed fields).
fn apply_hash_to_instance(
    inst: &Rc<RefCell<crate::interpreter::value::Instance>>,
    hash: &Value,
) -> Result<(), String> {
    use crate::interpreter::value::HashKey;
    if !matches!(hash, Value::Hash(_)) {
        return Err(format!(
            "expected a Hash of attributes, got {}",
            hash.type_name()
        ));
    }
    let class_name = inst.borrow().class.name.clone();
    let filtered = filter_mass_assign(&class_name, hash);
    let pairs = match &filtered {
        Value::Hash(p) => p,
        // filter_mass_assign returns the input unchanged for non-Hash;
        // we already early-returned above for that case.
        _ => unreachable!(),
    };
    let mut inst_mut = inst.borrow_mut();
    for (k, v) in pairs.borrow().iter() {
        if let HashKey::String(field) = k {
            if field.starts_with('_') {
                continue;
            }
            inst_mut.set(field.clone().to_string(), v.clone());
        }
    }
    Ok(())
}

/// Build the `_errors` array contents for a failed insert/update. SEC-039:
/// when the DB rejects the write with a unique-index conflict (e.g. two
/// concurrent `User.create({...})` calls racing on `email`), translate the
/// 409 into the same `[{field, message: "has already been taken"}]` shape
/// `validates uniqueness:` already produces, so callers handle the race
/// case identically to the SELECT-pre-flight case. Other errors are
/// preserved verbatim as before.
fn build_persistence_errors(class_name: &str, err: String) -> Vec<Value> {
    if super::validation::is_unique_violation(&err) {
        super::validation::build_unique_violation_errors(class_name, &err)
            .iter()
            .map(|v| v.to_value())
            .collect()
    } else {
        vec![Value::String(err.into())]
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
                Ok(exec_async_query_raw(query.to_string()))
            })),
        );

        // Debug: show the cursor URL
        env.define(
            "db_cursor_url".to_string(),
            Value::NativeFunction(NativeFunction::new("db_cursor_url", Some(0), |_args| {
                Ok(Value::String(get_cursor_url().to_string().into()))
            })),
        );

        // Effective DB name, honouring the per-worker thread-local override
        // installed by the parallel test runner. Tests should call this
        // instead of `getenv("SOLIDB_DATABASE")` so they target their own
        // worker's database.
        env.define(
            "db_name".to_string(),
            Value::NativeFunction(NativeFunction::new("db_name", Some(0), |_args| {
                Ok(Value::String(get_database_name().to_string().into()))
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
                Ok(exec_query_hardcoded(query.to_string()))
            })),
        );

        Self::register_ai_builtins(env);
    }

    /// App-facing AI primitives: `embed` / `embed_batch` (write-side embedding
    /// generation, the counterpart to the read-side `similar()` push-down) and
    /// `llm_generate` (OpenAI-compatible chat completion). Endpoints and keys
    /// are read from environment here, so credentials stay out of Soli code and
    /// there is one place to review where text is sent (GDPR).
    fn register_ai_builtins(env: &mut Environment) {
        // embed(text) -> Array<Float>
        // Generate an embedding vector for `text` via SOLI_EMBEDDING_* config.
        env.define(
            "embed".to_string(),
            Value::NativeFunction(NativeFunction::new("embed", Some(1), |args| {
                let text = match args.first() {
                    Some(Value::String(s)) => s.to_string(),
                    _ => {
                        return Err("embed expects a text string, e.g. embed(\"hello\")".to_string())
                    }
                };
                let vector = crate::embedding::generate_embedding(&text).ok_or_else(|| {
                    "embed could not generate an embedding: set SOLI_EMBEDDING_API_KEY \
                     (and optionally SOLI_EMBEDDING_URL / SOLI_EMBEDDING_MODEL)"
                        .to_string()
                })?;
                let items: Vec<Value> = vector.into_iter().map(Value::Float).collect();
                Ok(Value::Array(Rc::new(RefCell::new(items))))
            })),
        );

        // embed_batch(texts) -> Array<Array<Float>>
        // Embed many texts in a single request — for back-filling a collection.
        env.define(
            "embed_batch".to_string(),
            Value::NativeFunction(NativeFunction::new("embed_batch", Some(1), |args| {
                let texts: Vec<String> = match args.first() {
                    Some(Value::Array(arr)) => {
                        let mut out = Vec::with_capacity(arr.borrow().len());
                        for item in arr.borrow().iter() {
                            match item {
                                Value::String(s) => out.push(s.to_string()),
                                _ => {
                                    return Err(
                                        "embed_batch expects an array of strings".to_string()
                                    )
                                }
                            }
                        }
                        out
                    }
                    _ => {
                        return Err("embed_batch expects an array of strings, e.g. \
                                    embed_batch([\"a\", \"b\"])"
                            .to_string())
                    }
                };
                let vectors =
                    crate::embedding::generate_embeddings_batch(&texts).ok_or_else(|| {
                        "embed_batch could not generate embeddings: set SOLI_EMBEDDING_API_KEY \
                     (and optionally SOLI_EMBEDDING_URL / SOLI_EMBEDDING_MODEL)"
                            .to_string()
                    })?;
                let rows: Vec<Value> = vectors
                    .into_iter()
                    .map(|vector| {
                        let items: Vec<Value> = vector.into_iter().map(Value::Float).collect();
                        Value::Array(Rc::new(RefCell::new(items)))
                    })
                    .collect();
                Ok(Value::Array(Rc::new(RefCell::new(rows))))
            })),
        );

        // llm_generate(system, user) -> String
        // Chat completion via an OpenAI-compatible endpoint (SOLI_LLM_* config).
        env.define(
            "llm_generate".to_string(),
            Value::NativeFunction(NativeFunction::new("llm_generate", Some(2), |args| {
                let system = match args.first() {
                    Some(Value::String(s)) => s.to_string(),
                    _ => return Err("llm_generate expects (system, user) strings".to_string()),
                };
                let user = match args.get(1) {
                    Some(Value::String(s)) => s.to_string(),
                    _ => return Err("llm_generate expects (system, user) strings".to_string()),
                };
                let output =
                    crate::generation::generate_completion(&system, &user).ok_or_else(|| {
                        "llm_generate failed: set SOLI_LLM_URL (and optionally SOLI_LLM_API_KEY / \
                         SOLI_LLM_MODEL / SOLI_LLM_TEMPERATURE / SOLI_LLM_MAX_TOKENS)"
                            .to_string()
                    })?;
                Ok(Value::String(output.into()))
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
                    Some(Value::Symbol(s)) => s.clone(),
                    Some(other) => {
                        return Err(format!(
                            "validates() expects string or symbol field name, got {}",
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

                let (rule, conditions) = parse_validates_options(&field, &options)?;
                register_validation_with_conditions(&class_name, rule, conditions);
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
                        Some(Value::Symbol(s)) => s.clone(),
                        Some(other) => {
                            return Err(format!(
                                "{}() expects string or symbol method name, got {}",
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

        // attr_accessible(field1, field2, ...) or attr_accessible([field1, field2, ...])
        // Declares the whitelist of attributes that may be assigned via
        // mass-assignment paths (`Model.create(hash)`,
        // `Model.update(id, hash)`, `instance.update(hash)`,
        // `instance.save(hash)`). Any key not in the list is silently
        // dropped before validation, before instance population, and
        // before the DB write. Without a declaration the model accepts
        // every key (legacy behaviour) — see docs/models.md.
        native_static_methods.insert(
            "attr_accessible".to_string(),
            Rc::new(NativeFunction::new("Model.attr_accessible", None, |args| {
                let class_name = get_class_name_from_class(&args)?;
                let fields = collect_accessible_fields(&args[1..])?;
                register_accessible_attributes(&class_name, fields);
                Ok(Value::Null)
            })),
        );

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
                        Some(Value::Symbol(s)) => s.clone(),
                        Some(other) => {
                            return Err(format!(
                                "relation expects string or symbol name, got {}",
                                other.type_name()
                            ))
                        }
                        None => return Err("relation requires a name argument".to_string()),
                    };

                    // Optional config hash: class_name/foreign_key overrides
                    // plus dependent:/through:/source:/counter_cache: (validated
                    // per relation kind — a bad option raises at class load).
                    let options = parse_relation_options(args.get(2), &rel_type)?;

                    let relation = build_relation(&class_name, &name, rel_type.clone(), &options);
                    register_relation(&class_name, relation);
                    Ok(Value::Null)
                })),
            );
        }

        // has_and_belongs_to_many(name) or with options:
        //   { class_name, foreign_key, association_foreign_key, join_table }
        native_static_methods.insert(
            "has_and_belongs_to_many".to_string(),
            Rc::new(NativeFunction::new(
                "Model.has_and_belongs_to_many",
                None,
                |args| {
                    let class_name = get_class_name_from_class(&args)?;
                    let name = match args.get(1) {
                        Some(Value::String(s)) => s.clone(),
                        Some(other) => {
                            return Err(format!(
                                "has_and_belongs_to_many expects string name, got {}",
                                other.type_name()
                            ))
                        }
                        None => {
                            return Err(
                                "has_and_belongs_to_many requires a name argument".to_string()
                            )
                        }
                    };

                    let options =
                        parse_relation_options(args.get(2), &RelationType::HasAndBelongsToMany)?;

                    let relation = build_habtm_relation(&class_name, &name, &options);
                    register_relation(&class_name, relation);
                    Ok(Value::Null)
                },
            )),
        );

        // ====================================================================
        // Uploader DSL: uploader("photo", { multiple, content_types, ... })
        // ====================================================================

        native_static_methods.insert(
            "uploader".to_string(),
            Rc::new(NativeFunction::new("Model.uploader", Some(3), |args| {
                let class_name = get_class_name_from_class(&args)?;
                let config = build_uploader_config_from_args(&class_name, &args)?;
                register_uploader(&class_name, config);
                Ok(Value::Null)
            })),
        );

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
                            Some(s.to_string())
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
                            super::relations::reject_through_relation("includes", &rel)?;
                            super::relations::reject_polymorphic_relation("includes", &rel)?;
                            let fields = match v {
                                Value::Array(arr) => {
                                    let names: Vec<String> = arr
                                        .borrow()
                                        .iter()
                                        .filter_map(|v| {
                                            if let Value::String(s) = v {
                                                Some(s.to_string())
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
                                rel_name.to_string(),
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
                    super::relations::reject_through_relation("includes", &rel)?;
                    super::relations::reject_polymorphic_relation("includes", &rel)?;

                    let filter = if arguments.len() >= 3 {
                        match &arguments[1] {
                            Value::String(s) => Some(s.to_string()),
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
                            if **key == *"fields" {
                                if let Value::Array(arr) = v {
                                    let names: Vec<String> = arr
                                        .borrow()
                                        .iter()
                                        .filter_map(|v| {
                                            if let Value::String(s) = v {
                                                Some(s.to_string())
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
                                    key.to_string(),
                                    crate::interpreter::value::value_to_json(v)?,
                                );
                            }
                        }
                    }

                    qb.add_include(rel_name.to_string(), rel, filter, bind_vars, fields);
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
                        super::relations::reject_through_relation("includes", &rel)?;
                        super::relations::reject_polymorphic_relation("includes", &rel)?;
                        qb.add_include(
                            rel_name.to_string(),
                            rel,
                            None,
                            std::collections::HashMap::new(),
                            None,
                        );
                    }
                }

                Ok(Value::QueryBuilder(Rc::new(RefCell::new(qb))))
            })),
        );

        // Model.includes_count("posts", "comments") — preload relation counts
        // as <name>_count fields on each parent doc. Only valid for HasMany
        // and HABTM relations.
        native_static_methods.insert(
            "includes_count".to_string(),
            Rc::new(NativeFunction::new("Model.includes_count", None, |args| {
                let class = get_class_rc_from_args(&args)?;
                let class_name = class.name.clone();
                let collection = class_name_to_collection(&class_name);

                let mut qb = QueryBuilder::new_with_class(class_name.clone(), collection, class);
                let arguments = &args[1..];

                if arguments.is_empty() {
                    return Err("includes_count() requires at least one relation name".to_string());
                }

                for arg in arguments {
                    let rel_name = match arg {
                        Value::String(s) => s.clone(),
                        other => {
                            return Err(format!(
                                "includes_count() expects string relation names, got {}",
                                other.type_name()
                            ))
                        }
                    };
                    let rel = get_relation(&class_name, &rel_name).ok_or_else(|| {
                        format!("No relation '{}' defined on {}", rel_name, class_name)
                    })?;
                    super::relations::reject_through_relation("includes", &rel)?;
                    super::relations::reject_polymorphic_relation("includes", &rel)?;
                    qb.add_include_count(rel_name.to_string(), rel)?;
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
                    Value::String(s) => {
                        validate_field_name(s, "select")?;
                        fields.push(s.to_string());
                    }
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
                super::relations::reject_through_relation("join", &rel)?;
                super::relations::reject_polymorphic_relation("join", &rel)?;

                let filter = match args.get(2) {
                    Some(Value::String(s)) => Some(s.to_string()),
                    _ => None,
                };

                let bind_vars = match args.get(3) {
                    Some(Value::Hash(hash)) => {
                        use crate::interpreter::value::HashKey;
                        let mut map = std::collections::HashMap::new();
                        for (k, v) in hash.borrow().iter() {
                            if let HashKey::String(key) = k {
                                map.insert(
                                    key.to_string(),
                                    crate::interpreter::value::value_to_json(v)?,
                                );
                            }
                        }
                        map
                    }
                    _ => std::collections::HashMap::new(),
                };

                let mut qb = QueryBuilder::new_with_class(class_name, collection, class);
                qb.add_join(rel_name.to_string(), rel, filter, bind_vars);

                Ok(Value::QueryBuilder(Rc::new(RefCell::new(qb))))
            })),
        );

        // ====================================================================
        // CRUD Methods
        // ====================================================================

        // Model.create(data) - Insert document with validation. Always returns
        // an instance of the class. On success, `_errors` is unset (reads as
        // null). On validation or DB failure, the instance is NOT persisted
        // and `_errors` is populated as an Array — of {field, message} hashes
        // for validation errors, or of String messages for DB errors.
        use super::crud::{exec_insert, json_to_value};
        use super::validation::run_validations;
        use crate::interpreter::value::value_to_json;
        native_static_methods.insert(
            "create".to_string(),
            Rc::new(NativeFunction::new("Model.create", Some(2), |args| {
                let class = get_class_rc_from_args(&args)?;
                let class_name = class.name.clone();
                let collection = class_name_to_collection(&class_name);

                let raw_data = args
                    .get(1)
                    .cloned()
                    .ok_or_else(|| "Model.create() requires data argument".to_string())?;

                // Strong-params filter: when the model declared
                // `attr_accessible(...)`, drop any non-whitelisted keys
                // before they reach validation, the in-memory instance,
                // or the DB write. Models without a declaration get the
                // raw hash through unchanged (back-compat).
                let data = filter_mass_assign(&class_name, &raw_data);

                // Edge models: coerce from:/to: into _from/_to document ids.
                // Endpoints are read from the raw hash so attr_accessible
                // can't strip them; missing/invalid endpoints surface as
                // _errors like validation failures.
                let mut edge_refs: Option<(String, String)> = None;
                let data = match super::registry::get_edge_spec(&class_name) {
                    Some(edge_spec) => {
                        match super::graph::transform_edge_data(&raw_data, &edge_spec) {
                            Ok((from_ref, to_ref)) => {
                                edge_refs = Some((from_ref.clone(), to_ref.clone()));
                                super::graph::rebuild_edge_data(&data, &from_ref, &to_ref)
                            }
                            Err(endpoint_errors) => {
                                let instance = Rc::new(RefCell::new(
                                    crate::interpreter::value::Instance::new(class.clone()),
                                ));
                                apply_hash_to_instance(&instance, &data)?;
                                let error_values: Vec<Value> = endpoint_errors
                                    .into_iter()
                                    .map(|(field, message)| {
                                        let mut pairs =
                                            crate::interpreter::value::HashPairs::default();
                                        pairs.insert(
                                            HashKey::String("field".into()),
                                            Value::String(field.into()),
                                        );
                                        pairs.insert(
                                            HashKey::String("message".into()),
                                            Value::String(message.into()),
                                        );
                                        Value::Hash(Rc::new(RefCell::new(pairs)))
                                    })
                                    .collect();
                                instance.borrow_mut().set(
                                    "_errors".to_string(),
                                    Value::Array(Rc::new(RefCell::new(error_values))),
                                );
                                return Ok(Value::Instance(instance));
                            }
                        }
                    }
                    None => data,
                };

                // Build a base instance from the (filtered) attributes so
                // the returned object carries the data we'd actually
                // persist, even on failure.
                let instance = Rc::new(RefCell::new(crate::interpreter::value::Instance::new(
                    class.clone(),
                )));
                apply_hash_to_instance(&instance, &data)?;
                // apply_hash_to_instance skips `_`-prefixed keys; the edge
                // payload is the exception the caller should see.
                if let Some((ref from_ref, ref to_ref)) = edge_refs {
                    let mut inst_mut = instance.borrow_mut();
                    inst_mut.set("_from".to_string(), Value::String(from_ref.clone().into()));
                    inst_mut.set("_to".to_string(), Value::String(to_ref.clone().into()));
                }

                // Run validations against the filtered input — non-permitted
                // fields are gone, so callers can't satisfy a validation
                // (or trigger one) by smuggling fields the model never
                // intended to accept.
                let errors = run_validations(&class_name, &data, None)?;
                if !errors.is_empty() {
                    let error_values: Vec<Value> = errors.iter().map(|e| e.to_value()).collect();
                    instance.borrow_mut().set(
                        "_errors".to_string(),
                        Value::Array(Rc::new(RefCell::new(error_values))),
                    );
                    return Ok(Value::Instance(instance));
                }

                let data_value: Result<serde_json::Value, String> = match &data {
                    Value::Hash(hash) => {
                        let mut map = serde_json::Map::new();
                        for (k, v) in hash.borrow().iter() {
                            if let HashKey::String(key) = k {
                                map.insert(key.clone().to_string(), value_to_json(v)?);
                            }
                        }
                        Ok(serde_json::Value::Object(map))
                    }
                    other => Err(format!(
                        "Model.create() expects hash data, got {}",
                        other.type_name()
                    )),
                };
                let mut data_value = data_value?;

                // STI subclasses stamp their discriminator so rows in the
                // shared base collection hydrate as the right class.
                if super::registry::is_sti_subclass(&class_name) {
                    if let serde_json::Value::Object(ref mut map) = data_value {
                        map.insert(
                            "type".to_string(),
                            serde_json::Value::String(class_name.clone()),
                        );
                    }
                    instance.borrow_mut().set(
                        "type".to_string(),
                        Value::String(class_name.as_str().into()),
                    );
                }

                match exec_insert(&collection, None, data_value) {
                    Ok(id) => {
                        let mut inst_mut = instance.borrow_mut();
                        if let serde_json::Value::Object(ref id_map) = id {
                            for field in &["_key", "_id", "_rev", "_created_at", "_updated_at"] {
                                if let Some(val) = id_map.get(*field) {
                                    inst_mut.set(field.to_string(), json_to_value(val));
                                }
                            }
                        }
                        inst_mut.set("id".to_string(), json_to_value(&id));
                        super::dirty::finalize_persist(&mut inst_mut);
                        super::counter_cache::bump_for_instance(&inst_mut, 1);
                        drop(inst_mut);
                        Ok(Value::Instance(instance))
                    }
                    Err(e) => {
                        let error_values = build_persistence_errors(&class_name, e);
                        instance.borrow_mut().set(
                            "_errors".to_string(),
                            Value::Array(Rc::new(RefCell::new(error_values))),
                        );
                        Ok(Value::Instance(instance))
                    }
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

                // Inside a `grouped {}` block: rewrite the key lookup to a
                // cursor query so it can be coalesced with the others. The
                // transform preserves the RecordNotFound-on-miss contract, so
                // the error still surfaces (as a 404) when the deferred is read.
                if super::batch::is_active() {
                    let sdbql = format!(
                        "FOR doc IN {} FILTER doc._key == @k LIMIT 1 RETURN doc",
                        collection
                    );
                    let mut binds = std::collections::HashMap::new();
                    binds.insert("k".to_string(), serde_json::Value::String(id.to_string()));
                    let class2 = class.clone();
                    let class_name = class.name.clone();
                    let id2 = id.clone();
                    return Ok(super::batch::register(
                        sdbql,
                        binds,
                        Box::new(move |rows| match rows.first() {
                            Some(doc) if sti_row_matches(&class_name, doc) => {
                                Ok(json_doc_to_instance(&class2, doc))
                            }
                            _ => Err(format!(
                                "{}{} with id '{}' not found",
                                crate::error::RuntimeError::RECORD_NOT_FOUND_MARKER,
                                class_name,
                                id2
                            )),
                        }),
                    ));
                }

                match exec_get(&collection, &id) {
                    // STI: a subclass find only matches rows of its own
                    // hierarchy — a base-class row raises RecordNotFound
                    // exactly like a missing key (Rails semantics).
                    Ok(doc) if sti_row_matches(&class.name, &doc) => {
                        Ok(json_doc_to_instance(&class, &doc))
                    }
                    // Not found → raise with the RecordNotFound marker so the
                    // HTTP request handler converts it into a 404 response.
                    // Callers that want the "or null" shape should use
                    // find_by / first_by, or wrap in try/catch.
                    _ => Err(format!(
                        "{}{} with id '{}' not found",
                        crate::error::RuntimeError::RECORD_NOT_FOUND_MARKER,
                        class.name,
                        id
                    )),
                }
            })),
        );

        // Model.where(...) - Returns a QueryBuilder for chaining.
        //
        // Two forms:
        //   1. Hash form (safe — recommended for user input):
        //        Model.where({"email": "alice@x", "active": true})
        //      Each key is validated as an AQL identifier; values flow
        //      through bind parameters, so attacker-controlled data
        //      cannot reach the query template.
        //
        //   2. String form (developer-trusted — see docs/models.md for
        //      the security note):
        //        Model.where("doc.age >= @age", {"age": 18})
        //      Filter is concatenated verbatim into the AQL FILTER
        //      clause, so the *string itself* must never come from
        //      untrusted input.
        use std::collections::HashMap as StdHashMap;
        native_static_methods.insert(
            "where".to_string(),
            Rc::new(NativeFunction::new("Model.where", Some(3), |args| {
                let class = get_class_rc_from_args(&args)?;
                let class_name = class.name.clone();
                let collection = class_name_to_collection(&class_name);

                let (filter, bind_vars): (String, StdHashMap<String, serde_json::Value>) =
                    match args.get(1) {
                        Some(Value::Hash(hash)) => {
                            // Safe hash form. A second argument (bind_vars
                            // for the string form) is meaningless here and
                            // is rejected up-front so callers don't think
                            // they can mix forms.
                            if args.get(2).is_some() {
                                return Err("Model.where(Hash) takes a single argument; \
                                    the bind-vars hash is only valid with the string filter form"
                                    .to_string());
                            }
                            build_safe_filter_from_hash(hash, "where")?
                        }
                        Some(Value::String(s)) => {
                            let filter = s.clone();
                            let binds = match args.get(2) {
                                Some(Value::Hash(hash)) => {
                                    let mut map = StdHashMap::new();
                                    for (k, v) in hash.borrow().iter() {
                                        if let HashKey::String(key) = k {
                                            map.insert(
                                                key.to_string(),
                                                ensure_string_form_bind_value(v, key, "where")?,
                                            );
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
                                None => StdHashMap::new(),
                            };
                            (filter.to_string(), binds)
                        }
                        Some(other) => {
                            return Err(format!(
                                "Model.where() expects a Hash filter or a string filter expression, got {}",
                                other.type_name()
                            ))
                        }
                        None => return Err("Model.where() requires a filter argument".to_string()),
                    };

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
                    register_query_mock(query.to_string(), results_array);
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
            Rc::new(NativeFunction::new_auto_invocable(
                "Model.all",
                Some(1),
                |args| {
                    let class = get_class_rc_from_args(&args)?;
                    let collection = class_name_to_collection(&class.name);
                    let sdbql = format!(
                        "FOR doc IN {}{} RETURN doc",
                        collection,
                        sti_scope_clause(&class.name)
                    );
                    if super::batch::is_active() {
                        let class2 = class.clone();
                        return Ok(super::batch::register(
                            sdbql,
                            std::collections::HashMap::new(),
                            Box::new(move |rows| {
                                let values: Vec<Value> = rows
                                    .iter()
                                    .map(|j| super::crud::json_doc_to_instance(&class2, j))
                                    .collect();
                                Ok(Value::Array(Rc::new(RefCell::new(values))))
                            }),
                        ));
                    }
                    Ok(exec_auto_collection_as_instances(
                        sdbql,
                        &collection,
                        &class,
                    ))
                },
            )),
        );

        // Model.all_json() - Get all documents as raw JSON string (fastest)
        use super::crud::exec_async_query_raw;
        native_static_methods.insert(
            "all_json".to_string(),
            Rc::new(NativeFunction::new_auto_invocable(
                "Model.all_json",
                Some(1),
                |args| {
                    let class_name = get_class_name_from_class(&args)?;
                    let collection = class_name_to_collection(&class_name);
                    let sdbql = format!(
                        "FOR doc IN {}{} RETURN doc",
                        collection,
                        sti_scope_clause(&class_name)
                    );
                    Ok(exec_async_query_raw(sdbql))
                },
            )),
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
                validate_field_name(&field, "order")?;

                let direction = match args.get(2) {
                    Some(Value::String(s)) => s.clone(),
                    _ => "asc".into(),
                };
                validate_order_direction(&direction, "order")?;

                let mut qb = QueryBuilder::new_with_class(class_name, collection, class);
                qb.set_order(field.to_string(), direction.to_string());

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
                let class_name = get_class_name_from_class(&args)?;
                let collection = class_name_to_collection(&class_name);

                if super::registry::is_timeseries_model(&class_name) {
                    return Err(timeseries_insert_only_error(&class_name, "update"));
                }

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
                    Some(hash_val @ Value::Hash(_)) => {
                        // Strong-params filter for Hash-shaped input. The
                        // Instance branch below skips this on purpose:
                        // instance fields are populated by `Model.find` /
                        // queries, so they're already server-controlled —
                        // applying the whitelist there would corrupt
                        // legitimate persistence rather than block an
                        // attacker.
                        let filtered = filter_mass_assign(&class_name, hash_val);
                        let pairs = match &filtered {
                            Value::Hash(p) => p,
                            _ => unreachable!(),
                        };
                        let mut map = serde_json::Map::new();
                        for (k, v) in pairs.borrow().iter() {
                            if let HashKey::String(key) = k {
                                map.insert(key.clone().to_string(), value_to_json(v)?);
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

                // Counter caches / STI: pre-read the old document (only when
                // this class declares counter-cached belongs_to or is an STI
                // subclass) so an FK change can move parent counts and a
                // subclass can refuse rows outside its hierarchy.
                let needs_preread = super::counter_cache::class_has_counter_caches(&class_name)
                    || super::registry::is_sti_subclass(&class_name);
                let old_doc = if needs_preread {
                    super::crud::exec_get(&collection, &id).ok()
                } else {
                    None
                };
                if super::registry::is_sti_subclass(&class_name)
                    && !old_doc
                        .as_ref()
                        .is_some_and(|doc| sti_row_matches(&class_name, doc))
                {
                    return Ok(Value::String(
                        format!("Error: {} with id '{}' not found", class_name, id).into(),
                    ));
                }

                match exec_update(&collection, &id, data_value.clone(), true) {
                    Ok(result) => {
                        if let Some(old_doc) = &old_doc {
                            super::counter_cache::bump_for_json_change(
                                &class_name,
                                old_doc,
                                &data_value,
                            );
                        }
                        Ok(json_to_value(&result))
                    }
                    Err(e) => Ok(Value::String(format!("Error: {}", e).into())),
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

                let class_name = get_class_name_from_class(&args)?;
                let needs_preread = super::counter_cache::class_has_counter_caches(&class_name)
                    || super::registry::is_sti_subclass(&class_name);
                let old_doc = if needs_preread {
                    super::crud::exec_get(&collection, &id).ok()
                } else {
                    None
                };
                if super::registry::is_sti_subclass(&class_name)
                    && !old_doc
                        .as_ref()
                        .is_some_and(|doc| sti_row_matches(&class_name, doc))
                {
                    return Ok(Value::String(
                        format!("Error: {} with id '{}' not found", class_name, id).into(),
                    ));
                }

                match exec_delete(&collection, &id) {
                    Ok(result) => {
                        if let Some(old_doc) = &old_doc {
                            super::counter_cache::bump_for_json(&class_name, old_doc, -1);
                        }
                        Ok(json_to_value(&result))
                    }
                    Err(e) => Ok(Value::String(format!("Error: {}", e).into())),
                }
            })),
        );

        // Model.reset_counters(id, relation) - Recount a has_many relation's
        // children and write the counter column on the parent row. The
        // repair tool for counter drift (bulk writes skip bumps by design).
        native_static_methods.insert(
            "reset_counters".to_string(),
            Rc::new(NativeFunction::new(
                "Model.reset_counters",
                Some(3),
                |args| {
                    let class_name = get_class_name_from_class(&args)?;
                    let collection = class_name_to_collection(&class_name);
                    let id = match args.get(1) {
                        Some(Value::String(s)) => s.to_string(),
                        _ => return Err("Model.reset_counters() expects a string id".to_string()),
                    };
                    let relation_name = match args.get(2) {
                        Some(Value::String(s)) => s.to_string(),
                        Some(Value::Symbol(s)) => s.to_string(),
                        _ => {
                            return Err("Model.reset_counters() expects a relation name".to_string())
                        }
                    };
                    let count = super::counter_cache::reset_counters(
                        &class_name,
                        &collection,
                        &id,
                        &relation_name,
                    )?;
                    Ok(Value::Int(count))
                },
            )),
        );

        // Model.delete_all() - Remove every document in the collection.
        // Primarily intended for test setup/teardown. Fetches all keys
        // then deletes each individually — a single `FOR doc IN … REMOVE`
        // query isn't actually applied in SolidB, so iterate.
        native_static_methods.insert(
            "delete_all".to_string(),
            Rc::new(NativeFunction::new("Model.delete_all", Some(1), |args| {
                let class_name = get_class_name_from_class(&args)?;
                let collection = class_name_to_collection(&class_name);
                let sdbql = format!(
                    "FOR doc IN {}{} RETURN doc._key",
                    collection,
                    sti_scope_clause(&class_name)
                );
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
                            Err(e) => Ok(Value::String(format!("Error: {}", e).into())),
                        }
                    }
                    Some(Value::Function(_)) | Some(Value::VmClosure(_)) => {
                        // The block form `Model.transaction(fn() { ... })` is run by the
                        // executor interceptor (begin → run → commit / rollback on throw),
                        // which only recognizes an *inline* block literal. Reaching the native
                        // means a non-literal callable was passed; guide the caller instead of
                        // silently dropping the block and returning a handle (the old behavior).
                        Err(
                            "Model.transaction { ... } expects the block as an inline function \
                             literal, e.g. Model.transaction(fn() { ... }). For manual control, \
                             call Model.transaction() with no block and use .commit()/.rollback() \
                             on the returned handle."
                                .to_string(),
                        )
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
            Rc::new(NativeFunction::new_auto_invocable(
                "Model.count",
                Some(1),
                |args| {
                    let collection = get_collection_from_class(&args)?;

                    // Columnar models count through the columnar engine
                    // (COLLECTION_COUNT only sees document collections).
                    if let Ok(class_name) = get_class_name_from_class(&args) {
                        if super::registry::is_columnar_model(&class_name) {
                            let schema = super::registry::get_columnar_schema(&class_name)
                                .unwrap_or_default();
                            let column =
                                schema.columns.first().map(|c| c.name.clone()).ok_or_else(
                                    || {
                                        format!(
                                            "{}.count: columnar model has no `column` declarations",
                                            class_name
                                        )
                                    },
                                )?;
                            return super::columnar::aggregate(&collection, &column, "count", None);
                        }
                    }

                    let sti_clause = get_class_name_from_class(&args)
                        .map(|n| sti_scope_clause(&n))
                        .unwrap_or_default();
                    let sdbql = if sti_clause.is_empty() {
                        format!("RETURN COLLECTION_COUNT(\"{}\")", collection)
                    } else {
                        format!(
                            "RETURN LENGTH(FOR doc IN {}{} RETURN 1)",
                            collection, sti_clause
                        )
                    };

                    // Inside a `grouped {}` block, defer this count so it
                    // coalesces with the other reads into one round-trip
                    // instead of firing on its own. The combined-query builder
                    // strips the leading `RETURN ` so the scalar count embeds
                    // as `LET _bi = (COLLECTION_COUNT(...))`.
                    if super::batch::is_active() {
                        return Ok(super::batch::register(
                            sdbql,
                            std::collections::HashMap::new(),
                            Box::new(|rows| Ok(parse_count_result(&rows))),
                        ));
                    }

                    match exec_query(&collection, sdbql) {
                        Ok(results) => Ok(parse_count_result(&results)),
                        // `exec_with_auto_collection` auto-creates a missing
                        // collection and retries, so any error reaching us here
                        // is a real failure — surface it instead of silently
                        // returning 0 (which previously masked broken counts).
                        Err(e) => Err(format!("Model.count() failed: {}", e)),
                    }
                },
            )),
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

        // Model.paginate({ page: 1, per: 25 }) - Paginate results.
        // Returns { "records": [...], "pagination": { "page": n, "per": n, "total": n, "total_pages": n } }
        native_static_methods.insert(
            "paginate".to_string(),
            Rc::new(NativeFunction::new("Model.paginate", Some(2), |args| {
                let class = get_class_rc_from_args(&args)?;
                let class_name = class.name.clone();
                let collection = class_name_to_collection(&class_name);

                let params = match args.get(1) {
                    Some(Value::Hash(h)) => h.borrow().clone(),
                    _ => return Err("paginate() expects a Hash argument".to_string()),
                };

                let page =
                    match params.get(&crate::interpreter::value::HashKey::String("page".into())) {
                        Some(Value::Int(n)) if *n > 0 => *n as usize,
                        _ => 1,
                    };
                let per =
                    match params.get(&crate::interpreter::value::HashKey::String("per".into())) {
                        Some(Value::Int(n)) if *n > 0 => *n as usize,
                        _ => 25,
                    };

                let mut qb =
                    super::query::QueryBuilder::new_with_class(class_name, collection, class);
                let total_val = super::query::execute_query_builder_count(&qb);
                let total = match total_val {
                    Value::Int(n) => n as usize,
                    _ => 0,
                };

                let total_pages = if total == 0 { 1 } else { total.div_ceil(per) };
                let page = if page > total_pages {
                    total_pages
                } else {
                    page
                };
                let offset = (page - 1) * per;

                qb.set_offset(offset);
                qb.set_limit(per);
                let records = super::query::execute_query_builder(&qb);

                let mut pagination = crate::interpreter::value::HashPairs::default();
                pagination.insert(
                    crate::interpreter::value::HashKey::String("page".into()),
                    Value::Int(page as i64),
                );
                pagination.insert(
                    crate::interpreter::value::HashKey::String("per".into()),
                    Value::Int(per as i64),
                );
                pagination.insert(
                    crate::interpreter::value::HashKey::String("total".into()),
                    Value::Int(total as i64),
                );
                pagination.insert(
                    crate::interpreter::value::HashKey::String("total_pages".into()),
                    Value::Int(total_pages as i64),
                );

                let mut result = crate::interpreter::value::HashPairs::default();
                result.insert(
                    crate::interpreter::value::HashKey::String("records".into()),
                    records,
                );
                result.insert(
                    crate::interpreter::value::HashKey::String("pagination".into()),
                    Value::Hash(Rc::new(RefCell::new(pagination))),
                );

                Ok(Value::Hash(Rc::new(RefCell::new(result))))
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
                validate_field_name(&field, "find_by")?;
                let value = match args.get(2) {
                    Some(v) => super::value_to_json(v).map_err(|e| e.to_string())?,
                    None => return Err("find_by() requires a value".to_string()),
                };
                let sdbql = format!(
                    "FOR doc IN {} FILTER doc.{} == @val{} LIMIT 1 RETURN doc",
                    collection,
                    field,
                    sti_scope_clause(&class.name)
                );
                let mut binds = std::collections::HashMap::new();
                binds.insert("val".to_string(), value);
                if super::batch::is_active() {
                    let class2 = class.clone();
                    return Ok(super::batch::register(
                        sdbql,
                        binds,
                        Box::new(move |rows| {
                            Ok(match rows.first() {
                                Some(doc) => super::crud::json_doc_to_instance(&class2, doc),
                                None => Value::Null,
                            })
                        }),
                    ));
                }
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
                validate_field_name(&field, "first_by")?;
                let value = match args.get(2) {
                    Some(v) => super::value_to_json(v).map_err(|e| e.to_string())?,
                    None => return Err("first_by() requires a value".to_string()),
                };
                let sdbql = format!(
                    "FOR doc IN {} FILTER doc.{} == @val{} SORT doc._key ASC LIMIT 1 RETURN doc",
                    collection,
                    field,
                    sti_scope_clause(&class_name)
                );
                let mut binds = std::collections::HashMap::new();
                binds.insert("val".to_string(), value);
                if super::batch::is_active() {
                    let class2 = class.clone();
                    return Ok(super::batch::register(
                        sdbql,
                        binds,
                        Box::new(move |rows| {
                            Ok(match rows.first() {
                                Some(doc) => super::crud::json_doc_to_instance(&class2, doc),
                                None => Value::Null,
                            })
                        }),
                    ));
                }
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
                    validate_field_name(&field, "find_or_create_by")?;
                    let value = args
                        .get(2)
                        .ok_or_else(|| "find_or_create_by() requires a value".to_string())?;
                    let json_val = super::value_to_json(value).map_err(|e| e.to_string())?;

                    // Try to find existing
                    let sdbql = format!(
                        "FOR doc IN {} FILTER doc.{} == @val{} LIMIT 1 RETURN doc",
                        collection,
                        field,
                        sti_scope_clause(&class_name)
                    );
                    let mut binds = std::collections::HashMap::new();
                    binds.insert("val".to_string(), json_val.clone());
                    match super::crud::exec_with_auto_collection(sdbql, Some(binds), &collection) {
                        Ok(results) if !results.is_empty() => {
                            return Ok(super::crud::json_doc_to_instance(&class, &results[0]));
                        }
                        _ => {}
                    }

                    // Not found — create with defaults. Run the same
                    // strong-params filter as `Model.create`: when the
                    // model declared `attr_accessible(...)`, drop any
                    // non-whitelisted keys from the defaults hash before
                    // they reach the insert.
                    let defaults = match args.get(3) {
                        Some(hash_val @ Value::Hash(_)) => {
                            let filtered = filter_mass_assign(&class_name, hash_val);
                            let pairs = match &filtered {
                                Value::Hash(p) => p,
                                _ => unreachable!(),
                            };
                            let mut map = serde_json::Map::new();
                            for (k, v) in pairs.borrow().iter() {
                                if let crate::interpreter::value::HashKey::String(key) = k {
                                    if let Ok(jv) = super::value_to_json(v) {
                                        map.insert(key.clone().to_string(), jv);
                                    }
                                }
                            }
                            map
                        }
                        _ => serde_json::Map::new(),
                    };
                    let mut doc = defaults;
                    doc.insert(field.clone().to_string(), json_val.clone());
                    if super::registry::is_sti_subclass(&class_name) {
                        doc.insert(
                            "type".to_string(),
                            serde_json::Value::String(class_name.clone()),
                        );
                    }
                    match super::crud::exec_insert(
                        &collection,
                        None,
                        serde_json::Value::Object(doc),
                    ) {
                        Ok(result) => Ok(super::crud::json_doc_to_instance(&class, &result)),
                        Err(e) => {
                            // SEC-039: another writer beat us between the
                            // initial find and this insert. If the model
                            // has a unique index on `field` the DB will
                            // tell us about the race; retry the find so
                            // find_or_create_by's contract holds (the
                            // record exists by the time we return) instead
                            // of bubbling a 409 back to the caller.
                            if super::validation::is_unique_violation(&e) {
                                let retry_sdbql = format!(
                                    "FOR doc IN {} FILTER doc.{} == @val{} LIMIT 1 RETURN doc",
                                    collection,
                                    field,
                                    sti_scope_clause(&class_name)
                                );
                                let mut retry_binds = std::collections::HashMap::new();
                                retry_binds.insert("val".to_string(), json_val);
                                if let Ok(results) = super::crud::exec_with_auto_collection(
                                    retry_sdbql,
                                    Some(retry_binds),
                                    &collection,
                                ) {
                                    if !results.is_empty() {
                                        return Ok(super::crud::json_doc_to_instance(
                                            &class,
                                            &results[0],
                                        ));
                                    }
                                }
                            }
                            Err(format!("find_or_create_by create failed: {}", e))
                        }
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
                if super::registry::is_timeseries_model(&class_name) {
                    return Err(timeseries_insert_only_error(&class_name, "upsert"));
                }
                let key = match args.get(1) {
                    Some(Value::String(s)) => s.clone(),
                    _ => return Err("upsert() expects string key".to_string()),
                };
                let data = match args.get(2) {
                    Some(hash_val @ Value::Hash(_)) => {
                        // Strong-params filter — `upsert` is just as exposed
                        // to mass-assignment as `create`/`update`.
                        let filtered = filter_mass_assign(&class_name, hash_val);
                        let pairs = match &filtered {
                            Value::Hash(p) => p,
                            _ => unreachable!(),
                        };
                        let mut map = serde_json::Map::new();
                        for (k, v) in pairs.borrow().iter() {
                            if let crate::interpreter::value::HashKey::String(k_str) = k {
                                if let Ok(jv) = super::value_to_json(v) {
                                    map.insert(k_str.clone().to_string(), jv);
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
                        let mut insert_obj = match data {
                            serde_json::Value::Object(m) => m,
                            _ => serde_json::Map::new(),
                        };
                        // Snapshot the body sans `_key` so a retry-update
                        // (after a race) sends a normal update payload.
                        let update_payload = serde_json::Value::Object(insert_obj.clone());
                        insert_obj.insert(
                            "_key".to_string(),
                            serde_json::Value::String(key.clone().to_string()),
                        );
                        match super::crud::exec_insert(
                            &collection,
                            None,
                            serde_json::Value::Object(insert_obj),
                        ) {
                            Ok(result) => Ok(super::crud::json_doc_to_instance(&class, &result)),
                            Err(e) => {
                                // SEC-039: another writer created `key`
                                // between our failed update and our
                                // insert. Retry the update so upsert
                                // converges to the documented semantics
                                // instead of leaking a 409 the caller
                                // can't distinguish from a real conflict.
                                if super::validation::is_unique_violation(&e) {
                                    if let Ok(result) =
                                        exec_update(&collection, &key, update_payload, true)
                                    {
                                        return Ok(super::crud::json_doc_to_instance(
                                            &class, &result,
                                        ));
                                    }
                                }
                                Err(format!("upsert failed: {}", e))
                            }
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
                        hash_val @ Value::Hash(_) => {
                            // Strong-params filter applied per-item — bulk
                            // inserts are otherwise a perfect bypass for
                            // `attr_accessible` (one trip through
                            // `create_many` writes any attribute on every
                            // document at once).
                            let filtered = filter_mass_assign(&class_name, hash_val);
                            let pairs = match &filtered {
                                Value::Hash(p) => p,
                                _ => unreachable!(),
                            };
                            let mut map = serde_json::Map::new();
                            for (k, v) in pairs.borrow().iter() {
                                if let crate::interpreter::value::HashKey::String(k_str) = k {
                                    if let Ok(jv) = super::value_to_json(v) {
                                        map.insert(k_str.clone().to_string(), jv);
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
                    crate::interpreter::value::HashKey::String("created".into()),
                    Value::Int(created),
                );
                Ok(Value::Hash(Rc::new(RefCell::new(result))))
            })),
        );

        // Model.scope(name, query_fn) - Register a named scope on the model.
        //
        // In a class body the class is auto-prepended as args[0] (see
        // execute_class in `executor/statements.rs`), so user code reads
        // naturally as Ruby:
        //
        //   class User < Model
        //     scope("published", fn() { this.where("status = @s", { "s": "published" }) })
        //   end
        //
        // Inside the closure `this` is bound to a fresh QueryBuilder for the
        // model; the closure returns a (possibly refined) QueryBuilder.
        // Accessing `User.published`
        // invokes the closure (see scope dispatch in
        // `executor/access/member.rs`).
        native_static_methods.insert(
            "scope".to_string(),
            Rc::new(NativeFunction::new("Model.scope", Some(3), |args| {
                let class_name = get_class_name_from_class(&args)?;
                let name = match args.get(1) {
                    Some(Value::String(s)) => s.clone(),
                    Some(Value::Symbol(s)) => s.clone(),
                    _ => {
                        return Err(
                            "scope(name, fn) expects a string or symbol scope name".to_string()
                        )
                    }
                };
                let func = match args.get(2) {
                    Some(Value::Function(f)) => f.clone(),
                    _ => {
                        return Err(
                            "scope(name, fn) expects a function as second argument".to_string()
                        )
                    }
                };
                super::scopes::register_scope(&class_name, &name, func);
                Ok(Value::Null)
            })),
        );

        // Model.states() / Model.events() — state machine reflection. Return the
        // distinct state tags / event names across all machines on the class.
        native_static_methods.insert(
            "states".to_string(),
            Rc::new(NativeFunction::new("Model.states", None, |args| {
                let class_name = get_class_name_from_class(&args)?;
                let mut seen = std::collections::HashSet::new();
                let mut out = Vec::new();
                for machine in super::state_machine::machines_for(&class_name) {
                    for state in machine.states {
                        if seen.insert(state.clone()) {
                            out.push(Value::String(state.into()));
                        }
                    }
                }
                Ok(Value::Array(Rc::new(RefCell::new(out))))
            })),
        );
        native_static_methods.insert(
            "events".to_string(),
            Rc::new(NativeFunction::new("Model.events", None, |args| {
                let class_name = get_class_name_from_class(&args)?;
                let mut seen = std::collections::HashSet::new();
                let mut out = Vec::new();
                for machine in super::state_machine::machines_for(&class_name) {
                    for event in machine.events {
                        if seen.insert(event.name.clone()) {
                            out.push(Value::String(event.name.into()));
                        }
                    }
                }
                Ok(Value::Array(Rc::new(RefCell::new(out))))
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
                        Value::String(s) => {
                            validate_field_name(s, "pluck")?;
                            fields.push(s.to_string());
                        }
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

        // Model.sum(field), Model.avg(field), ... - Aggregations. The stats
        // funcs (median/stddev/variance) are emitted via COLLECT_LIST — see
        // build_aggregation_query. PERCENTILE is not a SolidB function (yet),
        // so it is deliberately absent.
        for (name, func) in &[
            ("sum", super::AggregationFunc::Sum),
            ("avg", super::AggregationFunc::Avg),
            ("min", super::AggregationFunc::Min),
            ("max", super::AggregationFunc::Max),
            ("median", super::AggregationFunc::Median),
            ("stddev", super::AggregationFunc::Stddev),
            ("variance", super::AggregationFunc::Variance),
            ("count_distinct", super::AggregationFunc::CountDistinct),
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
                        validate_field_name(&field, &method_name)?;
                        let mut qb = super::query::QueryBuilder::new_with_class(
                            class_name, collection, class,
                        );
                        qb.aggregation = Some((func.clone(), field.to_string()));
                        Ok(Value::QueryBuilder(Rc::new(RefCell::new(qb))))
                    },
                )),
            );
        }

        // Model.aggregate(...) — ONE static, two model kinds:
        //   Document models: aggregate({alias: [func, field], ...}) →
        //     QueryBuilder in grouped mode (combine with .group_by/.having;
        //     chain .all / .first).
        //   Columnar models: aggregate(column, op[, {"group_by": [...]}]) →
        //     executes immediately via the columnar engine (scalar, or rows
        //     of {group cols..., "value"} when grouped).
        native_static_methods.insert(
            "aggregate".to_string(),
            Rc::new(NativeFunction::new("Model.aggregate", None, |args| {
                let class = get_class_rc_from_args(&args)?;
                let class_name = class.name.clone();
                let collection = class_name_to_collection(&class_name);

                if super::registry::is_columnar_model(&class_name) {
                    let column = match args.get(1) {
                        Some(Value::String(s)) => s.to_string(),
                        _ => {
                            return Err(format!(
                                "{}.aggregate(column, op[, options]) expects a column name \
                                 string (columnar form)",
                                class_name
                            ))
                        }
                    };
                    validate_field_name(&column, "aggregate")?;
                    let op = match args.get(2) {
                        Some(Value::String(s)) => s.to_lowercase(),
                        _ => {
                            return Err(format!(
                                "{}.aggregate(column, op[, options]) expects an operation \
                                 string ({})",
                                class_name,
                                super::columnar::COLUMN_AGG_OPS.join(", ")
                            ))
                        }
                    };
                    if !super::columnar::COLUMN_AGG_OPS.contains(&op.as_str()) {
                        return Err(format!(
                            "{}.aggregate unknown operation '{}': expected one of {}",
                            class_name,
                            op,
                            super::columnar::COLUMN_AGG_OPS.join(", ")
                        ));
                    }
                    let mut group_by: Option<Vec<String>> = None;
                    if let Some(Value::Hash(opts)) = args.get(3) {
                        use crate::interpreter::value::HashKey;
                        for (k, v) in opts.borrow().iter() {
                            match k {
                                HashKey::String(s) if s.as_str() == "group_by" => {
                                    let cols = match v {
                                        Value::Array(arr) => {
                                            let arr = arr.borrow();
                                            let mut out = Vec::with_capacity(arr.len());
                                            for c in arr.iter() {
                                                match c {
                                                    Value::String(s) => {
                                                        validate_field_name(s, "aggregate")?;
                                                        out.push(s.to_string());
                                                    }
                                                    other => {
                                                        return Err(format!(
                                                            "aggregate() group_by entries must \
                                                             be strings, got {}",
                                                            other.type_name()
                                                        ))
                                                    }
                                                }
                                            }
                                            out
                                        }
                                        Value::String(s) => {
                                            validate_field_name(s, "aggregate")?;
                                            vec![s.to_string()]
                                        }
                                        other => {
                                            return Err(format!(
                                                "aggregate() group_by must be an array of \
                                                 columns, got {}",
                                                other.type_name()
                                            ))
                                        }
                                    };
                                    group_by = Some(cols);
                                }
                                HashKey::String(s) => {
                                    return Err(format!(
                                        "aggregate() unknown option '{}': expected group_by",
                                        s
                                    ))
                                }
                                _ => {}
                            }
                        }
                    }
                    return super::columnar::aggregate(&collection, &column, &op, group_by);
                }

                let spec_arg = args.get(1).ok_or_else(|| {
                    "aggregate() requires a spec hash, e.g. aggregate({ \"total\": [\"sum\", \
                     \"amount\"] })"
                        .to_string()
                })?;
                let specs = super::query::parse_aggregate_spec_hash(spec_arg)?;
                let mut qb =
                    super::query::QueryBuilder::new_with_class(class_name, collection, class);
                qb.aggregate_specs = specs;
                Ok(Value::QueryBuilder(Rc::new(RefCell::new(qb))))
            })),
        );

        // Columnar-model statics. Each requires a `columnar` declaration;
        // document models get a clear pointer at the document API instead.
        {
            fn require_columnar(args: &[Value], method: &str) -> Result<(String, String), String> {
                let class_name = get_class_name_from_class(args)?;
                if !super::registry::is_columnar_model(&class_name) {
                    return Err(format!(
                        "{}.{} requires a `columnar` declaration in the class body (regular \
                         models use the document API)",
                        class_name, method
                    ));
                }
                let collection = class_name_to_collection(&class_name);
                Ok((class_name, collection))
            }

            // Model.insert_rows([{...}, ...]) — bulk row insert; auto-creates
            // the store from the declared schema in dev.
            native_static_methods.insert(
                "insert_rows".to_string(),
                Rc::new(NativeFunction::new("Model.insert_rows", Some(2), |args| {
                    let (class_name, collection) = require_columnar(&args, "insert_rows")?;
                    let rows = match args.get(1) {
                        Some(rows @ Value::Array(_)) => {
                            crate::interpreter::value::value_to_json(rows)?
                        }
                        _ => {
                            return Err(format!(
                                "{}.insert_rows expects an array of row hashes",
                                class_name
                            ))
                        }
                    };
                    let schema =
                        super::registry::get_columnar_schema(&class_name).unwrap_or_default();
                    super::columnar::insert_rows(&collection, &schema, rows)
                })),
            );

            // Model.query({"columns": [...], "filter": {...}, "limit": n}) —
            // projection scan with the engine's single optional filter.
            native_static_methods.insert(
                "query".to_string(),
                Rc::new(NativeFunction::new("Model.query", Some(2), |args| {
                    let (class_name, collection) = require_columnar(&args, "query")?;
                    let options = args.get(1).ok_or_else(|| {
                        format!(
                            "{}.query expects an options hash with a \"columns\" array",
                            class_name
                        )
                    })?;
                    let (columns, filter, limit) = super::columnar::parse_query_options(options)?;
                    super::columnar::query(&collection, columns, filter, limit)
                })),
            );

            // Model.add_column_index(column[, type]) — sorted (default),
            // hash, bitmap, minmax, or bloom.
            native_static_methods.insert(
                "add_column_index".to_string(),
                Rc::new(NativeFunction::new(
                    "Model.add_column_index",
                    None,
                    |args| {
                        let (class_name, collection) = require_columnar(&args, "add_column_index")?;
                        let column = match args.get(1) {
                            Some(Value::String(s)) => s.to_string(),
                            _ => {
                                return Err(format!(
                                    "{}.add_column_index expects a column name",
                                    class_name
                                ))
                            }
                        };
                        validate_field_name(&column, "add_column_index")?;
                        let index_type = match args.get(2) {
                            Some(Value::String(s)) => {
                                let t = s.to_lowercase();
                                if !super::columnar::COLUMN_INDEX_TYPES.contains(&t.as_str()) {
                                    return Err(format!(
                                        "{}.add_column_index unknown type '{}': expected one \
                                         of {}",
                                        class_name,
                                        t,
                                        super::columnar::COLUMN_INDEX_TYPES.join(", ")
                                    ));
                                }
                                Some(t)
                            }
                            None => None,
                            Some(other) => {
                                return Err(format!(
                                    "{}.add_column_index type must be a string, got {}",
                                    class_name,
                                    other.type_name()
                                ))
                            }
                        };
                        super::columnar::create_index(&collection, &column, index_type.as_deref())
                    },
                )),
            );

            native_static_methods.insert(
                "column_indexes".to_string(),
                Rc::new(NativeFunction::new_auto_invocable(
                    "Model.column_indexes",
                    Some(1),
                    |args| {
                        let (_, collection) = require_columnar(&args, "column_indexes")?;
                        super::columnar::list_indexes(&collection)
                    },
                )),
            );

            native_static_methods.insert(
                "drop_column_index".to_string(),
                Rc::new(NativeFunction::new(
                    "Model.drop_column_index",
                    Some(2),
                    |args| {
                        let (class_name, collection) =
                            require_columnar(&args, "drop_column_index")?;
                        let column = match args.get(1) {
                            Some(Value::String(s)) => s.to_string(),
                            _ => {
                                return Err(format!(
                                    "{}.drop_column_index expects a column name",
                                    class_name
                                ))
                            }
                        };
                        validate_field_name(&column, "drop_column_index")?;
                        super::columnar::drop_index(&collection, &column)
                    },
                )),
            );

            native_static_methods.insert(
                "columnar_stats".to_string(),
                Rc::new(NativeFunction::new_auto_invocable(
                    "Model.columnar_stats",
                    Some(1),
                    |args| {
                        let (_, collection) = require_columnar(&args, "columnar_stats")?;
                        super::columnar::stats(&collection)
                    },
                )),
            );
        }

        // Model.group_by(...) — two forms:
        //   group_by("country") / group_by(["country", "plan"]) — multi-key
        //   grouping (combine with .aggregate/.having; implicit count).
        //   group_by(field, func, agg_field) — legacy single-aggregate form,
        //   returns [{group, result}] rows (unchanged).
        native_static_methods.insert(
            "group_by".to_string(),
            Rc::new(NativeFunction::new("Model.group_by", None, |args| {
                let class = get_class_rc_from_args(&args)?;
                let class_name = class.name.clone();
                let collection = class_name_to_collection(&class_name);

                if args.len() == 2 {
                    let fields: Vec<String> = match args.get(1) {
                        Some(Value::String(s)) => vec![s.to_string()],
                        Some(Value::Array(arr)) => {
                            let arr = arr.borrow();
                            let mut out = Vec::with_capacity(arr.len());
                            for v in arr.iter() {
                                match v {
                                    Value::String(s) => out.push(s.to_string()),
                                    other => {
                                        return Err(format!(
                                            "group_by() expects string field names, got {} in \
                                             array",
                                            other.type_name()
                                        ))
                                    }
                                }
                            }
                            out
                        }
                        _ => {
                            return Err("group_by() expects a field name or array of field names"
                                .to_string())
                        }
                    };
                    if fields.is_empty() {
                        return Err("group_by() requires at least one field".to_string());
                    }
                    for f in &fields {
                        validate_field_name(f, "group_by")?;
                    }
                    let mut qb =
                        super::query::QueryBuilder::new_with_class(class_name, collection, class);
                    qb.group_fields = fields;
                    return Ok(Value::QueryBuilder(Rc::new(RefCell::new(qb))));
                }

                let group_field = match args.get(1) {
                    Some(Value::String(s)) => s.clone(),
                    _ => return Err("group_by() expects string group field".to_string()),
                };
                validate_field_name(&group_field, "group_by")?;
                let func_name = match args.get(2) {
                    Some(Value::String(s)) => s.clone().to_lowercase(),
                    _ => return Err("group_by() expects string function name".to_string()),
                };
                let agg_field = match args.get(3) {
                    Some(Value::String(s)) => s.clone(),
                    _ => return Err("group_by() expects string aggregate field".to_string()),
                };
                validate_field_name(&agg_field, "group_by")?;
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
                qb.group_by_info = Some((group_field.to_string(), func, agg_field.to_string()));
                Ok(Value::QueryBuilder(Rc::new(RefCell::new(qb))))
            })),
        );

        // Model.similar(query[, field][, k][, options]) — start a similarity
        // chain without a where(): returns a QueryBuilder with the similar
        // spec set (same argument shapes as the chainable .similar()).
        native_static_methods.insert(
            "similar".to_string(),
            Rc::new(NativeFunction::new("Model.similar", None, |args| {
                use super::query::{SimilarInput, SimilarSpec};
                let class = get_class_rc_from_args(&args)?;
                let class_name = class.name.clone();
                let collection = class_name_to_collection(&class_name);

                let input = match args.get(1) {
                    Some(Value::String(s)) => SimilarInput::Text(s.to_string()),
                    Some(Value::Array(arr)) => {
                        let vec: Vec<f64> = arr
                            .borrow()
                            .iter()
                            .map(|v| match v {
                                Value::Int(n) => Ok(*n as f64),
                                Value::Float(f) => Ok(*f),
                                other => Err(format!(
                                    "similar() vector entries must be numbers, got {}",
                                    other.type_name()
                                )),
                            })
                            .collect::<Result<_, _>>()?;
                        if vec.is_empty() {
                            return Err("similar() vector is empty".to_string());
                        }
                        SimilarInput::Vector(vec)
                    }
                    _ => return Err("similar() expects query text or a numeric vector".to_string()),
                };
                let field = match args.get(2) {
                    Some(Value::String(s)) => s.to_string(),
                    _ => "embedding".to_string(),
                };
                validate_field_name(&field, "similar")?;
                let top_k = match args.get(3) {
                    Some(Value::Int(n)) if *n > 0 => *n as usize,
                    _ => 10,
                };
                let mut exact = false;
                let mut ef_search: Option<usize> = None;
                if let Some(Value::Hash(opts)) = args.get(4) {
                    use crate::interpreter::value::HashKey;
                    for (k, v) in opts.borrow().iter() {
                        let key = match k {
                            HashKey::String(s) => s.to_string(),
                            _ => continue,
                        };
                        match (key.as_str(), v) {
                            ("exact", Value::Bool(b)) => exact = *b,
                            ("ef_search", Value::Int(n)) if *n > 0 => ef_search = Some(*n as usize),
                            (other, _) => {
                                return Err(format!("similar() unknown/invalid option '{}'", other))
                            }
                        }
                    }
                }

                let mut qb =
                    super::query::QueryBuilder::new_with_class(class_name, collection, class);
                qb.similar_query = Some(SimilarSpec {
                    input,
                    field,
                    top_k,
                    exact,
                    ef_search,
                });
                qb.limit_val = Some(top_k);
                Ok(Value::QueryBuilder(Rc::new(RefCell::new(qb))))
            })),
        );

        // Model.search("query"[, options]) — fulltext search over the fields
        // of the declared `fulltext_index`. Options: field:, distance:,
        // limit:, highlight:. Returns ranked instances with _search_score.
        native_static_methods.insert(
            "search".to_string(),
            Rc::new(NativeFunction::new("Model.search", None, |args| {
                let class = get_class_rc_from_args(&args)?;
                let class_name = class.name.clone();
                let collection = class_name_to_collection(&class_name);

                let indexes = super::registry::get_fulltext_indexes(&class_name);
                let declared = indexes.first().ok_or_else(|| {
                    format!(
                        "{}.search requires a `fulltext_index` declaration in the class body, \
                         e.g. fulltext_index \"title\", \"body\"",
                        class_name
                    )
                })?;

                let query_text = match args.get(1) {
                    Some(Value::String(s)) => s.to_string(),
                    _ => return Err(format!("{}.search expects a query string", class_name)),
                };

                let mut field = declared
                    .fields
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "".to_string());
                let mut distance: usize = 2;
                let mut limit: usize = 10;
                let mut highlight = false;
                if let Some(Value::Hash(opts)) = args.get(2) {
                    use crate::interpreter::value::HashKey;
                    for (k, v) in opts.borrow().iter() {
                        let key = match k {
                            HashKey::String(s) => s.to_string(),
                            _ => continue,
                        };
                        match (key.as_str(), v) {
                            ("field", Value::String(s)) => {
                                let s = s.to_string();
                                if !declared.fields.iter().any(|f| f == &s) {
                                    return Err(format!(
                                        "{}.search field '{}' is not covered by the \
                                         fulltext_index (declared: {})",
                                        class_name,
                                        s,
                                        declared.fields.join(", ")
                                    ));
                                }
                                field = s;
                            }
                            ("distance", Value::Int(n)) if *n >= 0 => distance = *n as usize,
                            ("limit", Value::Int(n)) if *n > 0 => limit = *n as usize,
                            ("highlight", Value::Bool(b)) => highlight = *b,
                            (other, _) => {
                                return Err(format!(
                                    "search() unknown/invalid option '{}': expected field:, \
                                     distance:, limit:, or highlight:",
                                    other
                                ))
                            }
                        }
                    }
                }

                super::search::exec_fulltext_search(
                    &collection,
                    &class,
                    &field,
                    &query_text,
                    distance,
                    limit,
                    highlight,
                    is_soft_delete(&class_name),
                )
            })),
        );

        // Model.hybrid("query"[, options]) — combined vector + fulltext
        // search over the declared `vector_index` and `fulltext_index`.
        // The query text is embedded client-side for the vector leg (pass
        // vector: to skip) and used raw for the fulltext leg. Eager, like
        // search(): returns ranked instances carrying _hybrid_score,
        // _vector_score, _text_score and _sources. Options: vector:,
        // vector_field:, field:, vector_weight:, text_weight:, fusion:
        // ("weighted" | "rrf"), limit:.
        native_static_methods.insert(
            "hybrid".to_string(),
            Rc::new(NativeFunction::new("Model.hybrid", None, |args| {
                let class = get_class_rc_from_args(&args)?;
                let class_name = class.name.clone();
                let collection = class_name_to_collection(&class_name);

                let query_text = match args.get(1) {
                    Some(Value::String(s)) => s.to_string(),
                    _ => return Err(format!("{}.hybrid expects a query string", class_name)),
                };

                let mut explicit_vector: Option<Vec<f64>> = None;
                let mut vector_field: Option<String> = None;
                let mut fulltext_field_opt: Option<String> = None;
                let mut vector_weight: Option<f64> = None;
                let mut text_weight: Option<f64> = None;
                let mut fusion: Option<String> = None;
                let mut limit: Option<usize> = None;
                if let Some(Value::Hash(opts)) = args.get(2) {
                    use crate::interpreter::value::HashKey;
                    for (k, v) in opts.borrow().iter() {
                        let key = match k {
                            HashKey::String(s) => s.to_string(),
                            _ => continue,
                        };
                        match (key.as_str(), v) {
                            ("vector", Value::Array(arr)) => {
                                let vec: Vec<f64> = arr
                                    .borrow()
                                    .iter()
                                    .map(|v| match v {
                                        Value::Int(n) => Ok(*n as f64),
                                        Value::Float(f) => Ok(*f),
                                        other => Err(format!(
                                            "hybrid() vector entries must be numbers, got {}",
                                            other.type_name()
                                        )),
                                    })
                                    .collect::<Result<_, _>>()?;
                                if vec.is_empty() {
                                    return Err("hybrid() vector is empty".to_string());
                                }
                                explicit_vector = Some(vec);
                            }
                            ("vector_field", Value::String(s)) => {
                                vector_field = Some(s.to_string())
                            }
                            ("field", Value::String(s)) => fulltext_field_opt = Some(s.to_string()),
                            ("vector_weight", Value::Float(f)) => vector_weight = Some(*f),
                            ("vector_weight", Value::Int(n)) => vector_weight = Some(*n as f64),
                            ("text_weight", Value::Float(f)) => text_weight = Some(*f),
                            ("text_weight", Value::Int(n)) => text_weight = Some(*n as f64),
                            ("fusion", Value::String(s)) => {
                                let s = s.to_string();
                                if s != "weighted" && s != "rrf" {
                                    return Err(format!(
                                        "hybrid() fusion must be \"weighted\" or \"rrf\", \
                                         got \"{}\"",
                                        s
                                    ));
                                }
                                fusion = Some(s);
                            }
                            ("limit", Value::Int(n)) if *n > 0 => limit = Some(*n as usize),
                            (other, _) => {
                                return Err(format!(
                                    "hybrid() unknown/invalid option '{}': expected vector:, \
                                     vector_field:, field:, vector_weight:, text_weight:, \
                                     fusion:, or limit:",
                                    other
                                ))
                            }
                        }
                    }
                }

                // Resolve the vector index from the class declarations.
                let vindex = match &vector_field {
                    Some(f) => super::registry::get_vector_index_for_field(&class_name, f)
                        .ok_or_else(|| {
                            format!(
                                "{}.hybrid: no vector_index declared on field '{}'",
                                class_name, f
                            )
                        })?,
                    None => {
                        let mut declared = super::registry::get_vector_indexes(&class_name);
                        match declared.len() {
                            0 => {
                                return Err(format!(
                                    "{}.hybrid requires a `vector_index` declaration in the \
                                     class body, e.g. vector_index \"embedding\", dimension: 1536",
                                    class_name
                                ))
                            }
                            1 => declared.remove(0),
                            _ => {
                                return Err(format!(
                                    "{}.hybrid: several vector indexes declared; pass \
                                     vector_field: to pick one",
                                    class_name
                                ))
                            }
                        }
                    }
                };

                // Resolve the fulltext field from the class declarations.
                let ft_indexes = super::registry::get_fulltext_indexes(&class_name);
                let declared_ft = ft_indexes.first().ok_or_else(|| {
                    format!(
                        "{}.hybrid requires a `fulltext_index` declaration in the class body, \
                         e.g. fulltext_index \"title\", \"body\"",
                        class_name
                    )
                })?;
                let fulltext_field = match fulltext_field_opt {
                    Some(f) => {
                        if !ft_indexes
                            .iter()
                            .any(|idx| idx.fields.iter().any(|x| x == &f))
                        {
                            return Err(format!(
                                "{}.hybrid field '{}' is not covered by a fulltext_index \
                                 (declared: {})",
                                class_name,
                                f,
                                ft_indexes
                                    .iter()
                                    .flat_map(|i| i.fields.iter().cloned())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            ));
                        }
                        f
                    }
                    None => declared_ft.fields.first().cloned().unwrap_or_default(),
                };

                // Resolve the query vector: explicit literal, or embed the
                // query text client-side (same path as similar()).
                let query_vector = match explicit_vector {
                    Some(v) => v,
                    None => crate::embedding::generate_embedding(&query_text).ok_or_else(|| {
                        format!(
                            "{}.hybrid could not embed the query text: set \
                             SOLI_EMBEDDING_API_KEY (and optionally SOLI_EMBEDDING_URL / \
                             SOLI_EMBEDDING_MODEL) or pass vector: with a query vector",
                            class_name
                        )
                    })?,
                };

                super::search::exec_hybrid_search(
                    &collection,
                    &class,
                    &vindex.name,
                    &fulltext_field,
                    &query_vector,
                    &query_text,
                    vector_weight,
                    text_weight,
                    fusion,
                    limit,
                    is_soft_delete(&class_name),
                )
            })),
        );

        // Model.graph_rag(query, { via:, ... }) — graph-augmented retrieval:
        // ANN seeds on the declared vector_index, expand each seed through the
        // edge model's traversal, re-rank the union by cosine similarity.
        // Options: via: (required), direction:, depth:, field:, seed_k:,
        // limit:, vector:.
        native_static_methods.insert(
            "graph_rag".to_string(),
            Rc::new(NativeFunction::new("Model.graph_rag", None, |args| {
                let class = get_class_rc_from_args(&args)?;
                let class_name = class.name.clone();
                let collection = class_name_to_collection(&class_name);

                let query_text = match args.get(1) {
                    Some(Value::String(s)) => s.to_string(),
                    _ => return Err(format!("{}.graph_rag expects a query string", class_name)),
                };

                let opts = super::graph_rag::parse_graph_rag_options(&class_name, args.get(2))?;

                super::graph_rag::exec_graph_rag(
                    &class,
                    &class_name,
                    &collection,
                    &query_text,
                    opts,
                )
            })),
        );

        // Model.near(lat, lon[, options]) / Model.within(lat, lon, radius) —
        // geo queries over the declared `geo_index` field. Results carry
        // `_distance` (meters).
        fn geo_args(
            args: &[Value],
            method: &str,
        ) -> Result<(String, String, Rc<Class>, f64, f64), String> {
            let class = get_class_rc_from_args(args)?;
            let class_name = class.name.clone();
            let collection = class_name_to_collection(&class_name);
            let geo = super::registry::get_geo_indexes(&class_name);
            let field = geo.first().map(|g| g.field.clone()).ok_or_else(|| {
                format!(
                    "{}.{} requires a `geo_index` declaration in the class body, e.g. \
                         geo_index \"location\"",
                    class_name, method
                )
            })?;
            let num = |v: Option<&Value>, what: &str| -> Result<f64, String> {
                match v {
                    Some(Value::Int(n)) => Ok(*n as f64),
                    Some(Value::Float(f)) => Ok(*f),
                    _ => Err(format!(
                        "{}.{} expects numeric {}",
                        class_name, method, what
                    )),
                }
            };
            let lat = num(args.get(1), "lat")?;
            let lon = num(args.get(2), "lon")?;
            Ok((collection, field, class, lat, lon))
        }

        native_static_methods.insert(
            "near".to_string(),
            Rc::new(NativeFunction::new("Model.near", None, |args| {
                let class_name = get_class_name_from_class(&args)?;
                let (collection, field, class, lat, lon) = geo_args(&args, "near")?;
                let mut limit: f64 = 10.0;
                if let Some(Value::Hash(opts)) = args.get(3) {
                    use crate::interpreter::value::HashKey;
                    for (k, v) in opts.borrow().iter() {
                        match (k, v) {
                            (HashKey::String(key), Value::Int(n))
                                if key.as_str() == "limit" && *n > 0 =>
                            {
                                limit = *n as f64
                            }
                            (HashKey::String(key), _) => {
                                return Err(format!(
                                    "near() unknown/invalid option '{}': expected limit:",
                                    key
                                ))
                            }
                            _ => {}
                        }
                    }
                }
                super::search::exec_geo_query(
                    &collection,
                    &class,
                    &field,
                    "near",
                    lat,
                    lon,
                    ("limit", limit),
                    is_soft_delete(&class_name),
                )
            })),
        );

        native_static_methods.insert(
            "within".to_string(),
            Rc::new(NativeFunction::new("Model.within", None, |args| {
                let class_name = get_class_name_from_class(&args)?;
                let (collection, field, class, lat, lon) = geo_args(&args, "within")?;
                let radius = match args.get(3) {
                    Some(Value::Int(n)) if *n > 0 => *n as f64,
                    Some(Value::Float(f)) if *f > 0.0 => *f,
                    _ => {
                        return Err(format!(
                            "{}.within expects a radius in meters as the third argument",
                            class_name
                        ))
                    }
                };
                super::search::exec_geo_query(
                    &collection,
                    &class,
                    &field,
                    "within",
                    lat,
                    lon,
                    ("radius", radius),
                    is_soft_delete(&class_name),
                )
            })),
        );

        // Model.time_bucket(interval[, aggregates]) — bucketed aggregation for
        // timeseries models. Returns a QueryBuilder (chain .all to execute).
        native_static_methods.insert(
            "time_bucket".to_string(),
            Rc::new(NativeFunction::new("Model.time_bucket", None, |args| {
                let class = get_class_rc_from_args(&args)?;
                let class_name = class.name.clone();
                let collection = class_name_to_collection(&class_name);

                if !super::registry::is_timeseries_model(&class_name) {
                    return Err(format!(
                        "{}.time_bucket() requires a `timeseries` declaration in the class \
                         body — for regular models use group_by()",
                        class_name
                    ));
                }

                let interval_arg = args.get(1).ok_or_else(|| {
                    "time_bucket(interval, {aggregates}) requires an interval string, \
                         e.g. time_bucket(\"1h\", { \"avg\": \"value\" })"
                        .to_string()
                })?;
                let spec =
                    super::query::parse_time_bucket_args(interval_arg, args.get(2), &class_name)?;

                let mut qb =
                    super::query::QueryBuilder::new_with_class(class_name, collection, class);
                qb.time_bucket_info = Some(spec);
                Ok(Value::QueryBuilder(Rc::new(RefCell::new(qb))))
            })),
        );

        // Model.prune([older_than]) — timeseries retention. Deletes documents
        // older than the cutoff: a duration ("30d" → now - 30d), an RFC3339
        // timestamp, or (no argument) the declared `retention:`. Returns the
        // number of deleted documents.
        native_static_methods.insert(
            "prune".to_string(),
            Rc::new(NativeFunction::new_auto_invocable(
                "Model.prune",
                None,
                |args| {
                    let class = get_class_rc_from_args(&args)?;
                    let class_name = class.name.clone();
                    let collection = class_name_to_collection(&class_name);

                    let ts_spec =
                        super::registry::get_timeseries_spec(&class_name).ok_or_else(|| {
                            format!(
                                "{}.prune() requires a `timeseries` declaration in the class \
                                 body",
                                class_name
                            )
                        })?;

                    let cutoff_iso = match args.get(1) {
                        Some(Value::String(s)) => {
                            let s = s.to_string();
                            if validate_retention_duration(&s).is_ok() {
                                duration_to_cutoff_rfc3339(&s)?
                            } else if chrono::DateTime::parse_from_rfc3339(&s).is_ok() {
                                s
                            } else {
                                return Err(format!(
                                    "{}.prune() expects a duration (\"30d\") or an RFC3339 \
                                     timestamp, got {:?}",
                                    class_name, s
                                ));
                            }
                        }
                        Some(other) => {
                            return Err(format!(
                                "{}.prune() expects a string argument, got {}",
                                class_name,
                                other.type_name()
                            ))
                        }
                        None => match &ts_spec.retention {
                            Some(retention) => duration_to_cutoff_rfc3339(retention)?,
                            None => {
                                return Err(format!(
                                    "{}.prune requires an argument or a retention: declaration \
                                     (e.g. timeseries retention: \"30d\")",
                                    class_name
                                ))
                            }
                        },
                    };

                    let deleted = super::crud::exec_prune(&collection, &cutoff_iso)?;
                    Ok(Value::Int(deleted))
                },
            )),
        );

        // ====================================================================
        // Instance Methods (called on model instances: user.update(), user.delete())
        // ====================================================================
        let mut native_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

        // instance.to_h() - The instance's user fields as a Hash. Drops the
        // `_`-prefixed framework fields (`_key`, `_id`, `_rev`, `_errors`, …),
        // returning just the user-assigned attributes. Handy for serialization,
        // diffing, and content hashing (e.g. Ledger records).
        native_methods.insert(
            "to_h".to_string(),
            Rc::new(NativeFunction::new_auto_invocable(
                "Model#to_h",
                None,
                |args| {
                    let instance = match &args[0] {
                        Value::Instance(inst) => inst.clone(),
                        _ => return Err("Expected instance".to_string()),
                    };
                    let inst_ref = instance.borrow();
                    Ok(instance_fields_to_hash(&inst_ref))
                },
            )),
        );

        // instance.update() - Persist current instance fields to DB
        // Returns true on success, false on validation/DB error (errors stored in _errors)
        native_methods.insert(
            "update".to_string(),
            Rc::new(NativeFunction::new_auto_invocable(
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

                    {
                        let class_name = instance.borrow().class.name.clone();
                        if super::registry::is_timeseries_model(&class_name) {
                            return Err(timeseries_insert_only_error(&class_name, "update"));
                        }
                    }

                    // Optional hash of attributes: `inst.update({...})`
                    // applies the hash to instance fields before running
                    // the existing persist pipeline, so no-arg callers keep
                    // working unchanged. Hash-applied mutations are kept on
                    // the in-memory instance even if validation or the DB
                    // call later fails.
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
                            let changes = super::dirty::finalize_persist(&mut inst_mut);
                            super::counter_cache::bump_for_changes(&inst_mut, &changes);
                            Ok(Value::Bool(true))
                        }
                        Err(e) => {
                            let error_values = build_persistence_errors(&class_name, e);
                            instance.borrow_mut().set(
                                "_errors".to_string(),
                                Value::Array(Rc::new(RefCell::new(error_values))),
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
            Rc::new(NativeFunction::new_auto_invocable(
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
                    // Hash-applied mutations are kept on the in-memory instance
                    // even if validation or the DB call later fails.
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

                    // Edge models: coerce `from`/`to` fields (set via
                    // save({from: ..., to: ...}) or plain assignment) into
                    // _from/_to before the persisted map is built.
                    {
                        let class_name = instance.borrow().class.name.clone();
                        if let Some(edge_spec) = super::registry::get_edge_spec(&class_name) {
                            let mut inst_mut = instance.borrow_mut();
                            for (field, expected, target) in [
                                ("from", &edge_spec.from_collection, "_from"),
                                ("to", &edge_spec.to_collection, "_to"),
                            ] {
                                if let Some(val) = inst_mut.get(field) {
                                    match super::graph::edge_ref(&val, expected, field) {
                                        Ok(r) => {
                                            inst_mut.fields.remove(field);
                                            inst_mut
                                                .set(target.to_string(), Value::String(r.into()));
                                        }
                                        Err(message) => {
                                            let mut pairs =
                                                crate::interpreter::value::HashPairs::default();
                                            pairs.insert(
                                                HashKey::String("field".into()),
                                                Value::String(field.into()),
                                            );
                                            pairs.insert(
                                                HashKey::String("message".into()),
                                                Value::String(message.into()),
                                            );
                                            let err_hash =
                                                Value::Hash(Rc::new(RefCell::new(pairs)));
                                            inst_mut.set(
                                                "_errors".to_string(),
                                                Value::Array(Rc::new(RefCell::new(vec![err_hash]))),
                                            );
                                            return Ok(Value::Bool(false));
                                        }
                                    }
                                }
                            }
                        }
                    }

                    let inst_ref = instance.borrow();
                    let class_name = inst_ref.class.name.clone();
                    let collection = class_name_to_collection(&class_name);
                    let key_opt = inst_ref.get("_key").and_then(|k| match k {
                        Value::String(s) => Some(s.clone()),
                        _ => None,
                    });

                    // Timeseries models are insert-only: saving an existing
                    // record is an update, which the DB rejects — surface a
                    // clear error before any DB round trip.
                    if key_opt.is_some() && super::registry::is_timeseries_model(&class_name) {
                        return Err(timeseries_insert_only_error(&class_name, "save"));
                    }

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

                    // Build map for DB operation before dropping inst_ref.
                    // `_`-prefixed fields are DB-managed and stripped — except
                    // _from/_to on edge models, which ARE the edge payload.
                    let is_edge = super::registry::is_edge_model(&class_name);
                    let mut map = serde_json::Map::new();
                    for (k, v) in &inst_ref.fields {
                        if !k.starts_with('_') || (is_edge && matches!(k.as_str(), "_from" | "_to"))
                        {
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
                                let changes = super::dirty::finalize_persist(&mut inst_mut);
                                super::counter_cache::bump_for_changes(&inst_mut, &changes);
                                Ok(Value::Bool(true))
                            }
                            Err(e) => {
                                let error_values = build_persistence_errors(&class_name, e);
                                instance.borrow_mut().set(
                                    "_errors".to_string(),
                                    Value::Array(Rc::new(RefCell::new(error_values))),
                                );
                                Ok(Value::Bool(false))
                            }
                        }
                    } else {
                        // Insert new document
                        let mut map = map;
                        // STI subclasses stamp their discriminator so rows in
                        // the shared base collection hydrate as the right class.
                        if super::registry::is_sti_subclass(&class_name) {
                            map.insert(
                                "type".to_string(),
                                serde_json::Value::String(class_name.clone()),
                            );
                            instance.borrow_mut().set(
                                "type".to_string(),
                                Value::String(class_name.as_str().into()),
                            );
                        }
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
                                super::dirty::finalize_persist(&mut inst_mut);
                                super::counter_cache::bump_for_instance(&inst_mut, 1);
                                Ok(Value::Bool(true))
                            }
                            Err(e) => {
                                let error_values = build_persistence_errors(&class_name, e);
                                instance.borrow_mut().set(
                                    "_errors".to_string(),
                                    Value::Array(Rc::new(RefCell::new(error_values))),
                                );
                                Ok(Value::Bool(false))
                            }
                        }
                    }
                },
            )),
        );

        // instance.delete() - Delete (or soft-delete) the document from DB.
        // before_delete/after_delete run in the executor interceptor so user
        // methods and closures can execute with `this` bound to the instance.
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
                    let was_active = matches!(
                        instance.borrow().get("deleted_at"),
                        None | Some(Value::Null)
                    );
                    let now = chrono::Utc::now().to_rfc3339();
                    let mut map = serde_json::Map::new();
                    map.insert(
                        "deleted_at".to_string(),
                        serde_json::Value::String(now.clone()),
                    );
                    match exec_update(&collection, &key_str, serde_json::Value::Object(map), true) {
                        Ok(_) => {
                            let mut inst_mut = instance.borrow_mut();
                            inst_mut.set("deleted_at".to_string(), Value::String(now.into()));
                            super::dirty::sync_snapshot_field(&mut inst_mut, "deleted_at");
                            // Counters track default-scope-visible children:
                            // vanishing from the scope decrements the parent.
                            if was_active {
                                super::counter_cache::bump_for_instance(&inst_mut, -1);
                            }
                            Ok(Value::Bool(true))
                        }
                        Err(e) => Ok(Value::String(format!("Error: {}", e).into())),
                    }
                } else {
                    // Hard delete: remove document
                    match exec_delete(&collection, &key_str) {
                        Ok(result) => {
                            super::counter_cache::bump_for_instance(&instance.borrow(), -1);
                            Ok(json_to_value(&result))
                        }
                        Err(e) => Ok(Value::String(format!("Error: {}", e).into())),
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

                let was_deleted = !matches!(
                    instance.borrow().get("deleted_at"),
                    None | Some(Value::Null)
                );
                let mut map = serde_json::Map::new();
                map.insert("deleted_at".to_string(), serde_json::Value::Null);
                match exec_update(&collection, &key_str, serde_json::Value::Object(map), true) {
                    Ok(_) => {
                        let mut inst_mut = instance.borrow_mut();
                        inst_mut.set("deleted_at".to_string(), Value::Null);
                        super::dirty::sync_snapshot_field(&mut inst_mut, "deleted_at");
                        // Re-entering the default scope re-increments the parent.
                        if was_deleted {
                            super::counter_cache::bump_for_instance(&inst_mut, 1);
                        }
                        Ok(Value::Bool(true))
                    }
                    Err(e) => Err(format!("restore failed: {}", e)),
                }
            })),
        );

        // instance.increment(field, amount?) - Atomically bump a numeric field.
        // Uses a fetch + If-Match CAS retry loop so concurrent increments cannot
        // lose updates (see crud::cas_field_delta).
        native_methods.insert(
            "increment".to_string(),
            Rc::new(NativeFunction::new("Model#increment", None, |args| {
                apply_field_delta(&args, /*sign=*/ 1, "increment")
            })),
        );

        // instance.decrement(field, amount?) - Atomically subtract from a numeric field.
        native_methods.insert(
            "decrement".to_string(),
            Rc::new(NativeFunction::new("Model#decrement", None, |args| {
                apply_field_delta(&args, /*sign=*/ -1, "decrement")
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
                            .set("_updated_at".to_string(), Value::String(now.into()));
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
                            super::dirty::seed_snapshot(&mut inst_mut);
                            inst_mut.previous_changes = None;
                        }
                        Ok(Value::Instance(instance))
                    }
                    Err(e) => Err(format!("reload failed: {}", e)),
                }
            })),
        );

        // Dirty tracking. The baseline snapshot is seeded on DB load /
        // successful persist (see model::dirty); these natives compare the
        // live fields against it lazily. All are auto-invocable so bare
        // `record.changed?` works.
        native_methods.insert(
            "changed?".to_string(),
            Rc::new(NativeFunction::new_auto_invocable(
                "Model#changed?",
                Some(0),
                |args| {
                    let instance = match &args[0] {
                        Value::Instance(inst) => inst.clone(),
                        _ => return Err("Expected instance".to_string()),
                    };
                    let inst_ref = instance.borrow();
                    Ok(Value::Bool(
                        !super::dirty::compute_changes(&inst_ref).is_empty(),
                    ))
                },
            )),
        );

        // instance.changed - array of changed attribute names, sorted.
        native_methods.insert(
            "changed".to_string(),
            Rc::new(NativeFunction::new_auto_invocable(
                "Model#changed",
                Some(0),
                |args| {
                    let instance = match &args[0] {
                        Value::Instance(inst) => inst.clone(),
                        _ => return Err("Expected instance".to_string()),
                    };
                    let inst_ref = instance.borrow();
                    let names: Vec<Value> = super::dirty::compute_changes(&inst_ref)
                        .into_iter()
                        .map(|(name, _, _)| Value::String(name.into()))
                        .collect();
                    Ok(Value::Array(Rc::new(RefCell::new(names))))
                },
            )),
        );

        // instance.changes - { "name": [old, new] } for unsaved changes.
        native_methods.insert(
            "changes".to_string(),
            Rc::new(NativeFunction::new_auto_invocable(
                "Model#changes",
                Some(0),
                |args| {
                    let instance = match &args[0] {
                        Value::Instance(inst) => inst.clone(),
                        _ => return Err("Expected instance".to_string()),
                    };
                    let inst_ref = instance.borrow();
                    Ok(super::dirty::changes_to_hash(
                        &super::dirty::compute_changes(&inst_ref),
                    ))
                },
            )),
        );

        // instance.previous_changes - what the last successful save/update/
        // create persisted, as { "name": [old, new] }. Empty hash when the
        // record has never been persisted by this instance.
        native_methods.insert(
            "previous_changes".to_string(),
            Rc::new(NativeFunction::new_auto_invocable(
                "Model#previous_changes",
                Some(0),
                |args| {
                    let instance = match &args[0] {
                        Value::Instance(inst) => inst.clone(),
                        _ => return Err("Expected instance".to_string()),
                    };
                    let inst_ref = instance.borrow();
                    let changes = inst_ref
                        .previous_changes
                        .as_deref()
                        .cloned()
                        .unwrap_or_default();
                    Ok(super::dirty::changes_to_hash(&changes))
                },
            )),
        );

        // instance.attribute_was("name") - the baseline value of one
        // attribute (null on a new record or unknown attribute).
        native_methods.insert(
            "attribute_was".to_string(),
            Rc::new(NativeFunction::new(
                "Model#attribute_was",
                Some(1),
                |args| {
                    let instance = match &args[0] {
                        Value::Instance(inst) => inst.clone(),
                        _ => return Err("Expected instance".to_string()),
                    };
                    let name = match args.get(1) {
                        Some(Value::String(s)) => s.to_string(),
                        _ => {
                            return Err(
                                "attribute_was() expects a string attribute name".to_string()
                            )
                        }
                    };
                    let inst_ref = instance.borrow();
                    let value = inst_ref
                        .original_fields
                        .as_deref()
                        .and_then(|original| original.get(&name).cloned())
                        .unwrap_or(Value::Null);
                    Ok(value)
                },
            )),
        );

        // instance.traverse(EdgeModel[, {direction:, depth:}]) — graph
        // traversal from this record. Returns a chainable QueryBuilder whose
        // vertex variable is `doc`, so .where/.order/.limit/.count compose
        // exactly like a collection query.
        native_methods.insert(
            "traverse".to_string(),
            Rc::new(NativeFunction::new("Model#traverse", None, |args| {
                let instance = match &args[0] {
                    Value::Instance(inst) => inst.clone(),
                    _ => return Err("Expected instance".to_string()),
                };
                let own_class_name = instance.borrow().class.name.clone();
                let own_collection = class_name_to_collection(&own_class_name);
                let start_id = super::graph::edge_ref(
                    &Value::Instance(instance.clone()),
                    &own_collection,
                    "traverse()",
                )
                .map_err(|_| {
                    format!(
                        "traverse() requires a saved record ({} instance has no _key)",
                        own_class_name
                    )
                })?;

                let spec = super::graph::parse_traverse_options(&args[1..])?;

                // The result class: follow the edge declaration in the walk
                // direction; per-document _id resolution corrects mixed
                // results at materialization time.
                let target_collection = match (&spec.edge_spec, spec.direction) {
                    (Some(es), super::graph::TraversalDirection::Out) => es.to_collection.clone(),
                    (Some(es), super::graph::TraversalDirection::In) => es.from_collection.clone(),
                    _ => own_collection.clone(),
                };
                let target_class_name = super::relations::classify(&target_collection);
                let target_class = super::registry::get_model_class(&target_class_name)
                    .or_else(|| super::registry::get_model_class(&own_class_name));

                let mut qb = match target_class {
                    Some(class) => QueryBuilder::new_with_class(
                        target_class_name,
                        spec.edge_collection.clone(),
                        class,
                    ),
                    None => QueryBuilder::new(target_class_name, spec.edge_collection.clone()),
                };
                qb.bind_vars.insert(
                    crate::interpreter::get_symbol(super::graph::TRAVERSE_START_BIND),
                    serde_json::Value::String(start_id),
                );
                qb.traversal = Some(super::graph::TraversalClause {
                    edge_collection: spec.edge_collection,
                    direction: spec.direction,
                    min_depth: spec.min_depth,
                    max_depth: spec.max_depth,
                });
                Ok(Value::QueryBuilder(Rc::new(RefCell::new(qb))))
            })),
        );

        // instance.shortest_path(target, via: EdgeModel[, direction: "any"])
        // — BFS shortest path. Executes immediately; returns the Array of
        // vertices in path order (start → target), or [] when unconnected.
        native_methods.insert(
            "shortest_path".to_string(),
            Rc::new(NativeFunction::new("Model#shortest_path", None, |args| {
                let instance = match &args[0] {
                    Value::Instance(inst) => inst.clone(),
                    _ => return Err("Expected instance".to_string()),
                };
                let own_class = instance.borrow().class.clone();
                let own_class_name = own_class.name.clone();
                let own_collection = class_name_to_collection(&own_class_name);
                let start_id = super::graph::edge_ref(
                    &Value::Instance(instance.clone()),
                    &own_collection,
                    "shortest_path()",
                )
                .map_err(|_| {
                    format!(
                        "shortest_path() requires a saved record ({} instance has no _key)",
                        own_class_name
                    )
                })?;

                let target = args.get(1).cloned().ok_or_else(|| {
                    "shortest_path(target, via: EdgeModel) requires a target record".to_string()
                })?;

                // Options: via: (required), direction: (default "any").
                let mut via: Option<Value> = None;
                let mut direction = super::graph::TraversalDirection::Any;
                if let Some(opts) = args.get(2) {
                    let hash = match opts {
                        Value::Hash(h) => h.clone(),
                        other => {
                            return Err(format!(
                                "shortest_path() options must be a hash, got {}",
                                other.type_name()
                            ))
                        }
                    };
                    for (k, v) in hash.borrow().iter() {
                        let key = match k {
                            HashKey::String(s) => s.to_string(),
                            _ => continue,
                        };
                        match key.as_str() {
                            "via" => via = Some(v.clone()),
                            "direction" => {
                                let dir = match v {
                                    Value::String(s) => s.to_string(),
                                    Value::Symbol(s) => s.to_string(),
                                    other => {
                                        return Err(format!(
                                            "shortest_path() direction must be a string, got {}",
                                            other.type_name()
                                        ))
                                    }
                                };
                                direction = super::graph::TraversalDirection::parse(&dir)?;
                            }
                            other => {
                                return Err(format!(
                                    "shortest_path() unknown option '{}': expected via: or \
                                     direction:",
                                    other
                                ))
                            }
                        }
                    }
                }
                let via = via.ok_or_else(|| {
                    "shortest_path() requires via: an edge model, e.g. \
                     shortest_path(other, via: Follow)"
                        .to_string()
                })?;

                let (edge_collection, edge_spec) = match &via {
                    Value::Class(c) => {
                        let name = c.name.to_string();
                        let spec = super::registry::get_edge_spec(&name)
                            .ok_or_else(|| format!("{} has no `edge` declaration", name))?;
                        (class_name_to_collection(&name), Some(spec))
                    }
                    Value::String(s) => (s.to_string(), None),
                    other => {
                        return Err(format!(
                            "shortest_path() via: expects an edge model class or collection \
                             name, got {}",
                            other.type_name()
                        ))
                    }
                };
                super::graph::validate_collection_ident(&edge_collection, "shortest_path")?;

                // Coerce the target to a full "coll/key" id. Bare keys need a
                // declared edge spec to know the collection; walking OUT ends
                // on to_collection, IN on from_collection, ANY defaults to
                // the receiver's own collection.
                let end_expected = match (&edge_spec, direction) {
                    (Some(es), super::graph::TraversalDirection::Out) => es.to_collection.clone(),
                    (Some(es), super::graph::TraversalDirection::In) => es.from_collection.clone(),
                    _ => own_collection.clone(),
                };
                let end_id =
                    super::graph::any_vertex_ref(&target, &end_expected, "shortest_path()")?;

                let (query, binds) = super::graph::shortest_path_query(
                    &edge_collection,
                    direction,
                    &start_id,
                    &end_id,
                );
                Ok(super::crud::exec_auto_collection_as_instances_with_binds(
                    query,
                    binds,
                    &edge_collection,
                    &own_class,
                ))
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

/// Shared body for `Model#increment` / `Model#decrement`. Resolves the
/// instance + field + amount from `args`, then drives a fetch + If-Match
/// CAS retry loop via `crud::cas_field_delta`. On success, refreshes the
/// in-memory instance's field value and `_rev` so subsequent reads observe
/// the same state the DB now holds.
fn apply_field_delta(args: &[Value], sign: i64, op_name: &str) -> Result<Value, String> {
    let instance = match &args[0] {
        Value::Instance(inst) => inst.clone(),
        _ => return Err("Expected instance".to_string()),
    };
    {
        let class_name = instance.borrow().class.name.clone();
        if super::registry::is_timeseries_model(&class_name) {
            return Err(timeseries_insert_only_error(&class_name, op_name));
        }
    }
    let field = match args.get(1) {
        Some(Value::String(s)) => s.clone(),
        _ => return Err(format!("{}() expects a string field name", op_name)),
    };
    let amount = match args.get(2) {
        Some(Value::Int(n)) => *n,
        Some(Value::Float(n)) => *n as i64,
        None => 1,
        _ => return Err(format!("{}() amount must be a number", op_name)),
    };

    let inst_ref = instance.borrow();
    let collection = class_name_to_collection(&inst_ref.class.name);
    let key_str = match inst_ref.get("_key") {
        Some(Value::String(s)) => s,
        Some(_) => return Err("_key is not a string".to_string()),
        None => return Err("Instance has no _key field".to_string()),
    };
    drop(inst_ref);

    let delta = sign * amount;
    match super::crud::cas_field_delta(&collection, &key_str, &field, delta) {
        Ok((new_value, new_rev)) => {
            let mut inst_mut = instance.borrow_mut();
            inst_mut.set(field.to_string(), Value::Int(new_value));
            inst_mut.set("_rev".to_string(), Value::String(new_rev.into()));
            super::dirty::sync_snapshot_field(&mut inst_mut, &field);
            drop(inst_mut);
            Ok(Value::Instance(instance))
        }
        Err(e) => Err(format!("{} failed: {}", op_name, e)),
    }
}

pub fn register_model_builtins(env: &mut Environment) {
    Model::register_builtins(env);

    // dev_queries() - Returns the AQL queries executed during the current
    // request (dev mode only; empty array in production). Each entry:
    // { "query": String, "bind_vars": Hash | null, "duration_ms": Float }.
    env.define(
        "dev_queries".to_string(),
        Value::NativeFunction(NativeFunction::new("dev_queries", Some(0), |_| {
            use crate::interpreter::value::{HashKey, HashPairs};
            let entries = super::query_log::snapshot();
            let mut arr: Vec<Value> = Vec::with_capacity(entries.len());
            for entry in entries {
                let mut hash = HashPairs::default();
                hash.insert(
                    HashKey::String("query".into()),
                    Value::String(entry.query.into()),
                );
                let binds_value = match entry.bind_vars {
                    Some(map) => {
                        let mut bh = HashPairs::default();
                        for (k, v) in map {
                            bh.insert(HashKey::String(k.into()), super::crud::json_to_value(&v));
                        }
                        Value::Hash(Rc::new(RefCell::new(bh)))
                    }
                    None => Value::Null,
                };
                hash.insert(HashKey::String("bind_vars".into()), binds_value);
                hash.insert(
                    HashKey::String("duration_ms".into()),
                    Value::Float(entry.duration_ms),
                );
                arr.push(Value::Hash(Rc::new(RefCell::new(hash))));
            }
            Ok(Value::Array(Rc::new(RefCell::new(arr))))
        })),
    );

    // Register global wrapper functions for class-level DSL
    // These functions expect the class as the first argument (passed by execute_class)

    // validates(class, field, options) - Register validation rules
    env.define(
        "validates".to_string(),
        Value::NativeFunction(NativeFunction::new("validates", Some(3), |args| {
            let class_name = get_class_name_from_class(&args)?;

            let field = match args.get(1) {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Symbol(s)) => s.clone(),
                Some(other) => {
                    return Err(format!(
                        "validates() expects string or symbol field name, got {}",
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

            let (rule, conditions) = parse_validates_options(&field, &options)?;
            register_validation_with_conditions(&class_name, rule, conditions);
            Ok(Value::Null)
        })),
    );

    // attr_accessible(class, fields...) — global form mirrors the class-body
    // DSL (`attr_accessible("name", "email")`). See the static-method copy
    // above for filter semantics.
    env.define(
        "attr_accessible".to_string(),
        Value::NativeFunction(NativeFunction::new("attr_accessible", None, |args| {
            let class_name = get_class_name_from_class(&args)?;
            let fields = collect_accessible_fields(&args[1..])?;
            register_accessible_attributes(&class_name, fields);
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
                        Some(Value::Symbol(s)) => s.clone(),
                        Some(other) => {
                            return Err(format!(
                                "{}() expects string or symbol method name, got {}",
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

    // __sync_model_indexes() — run the declared-index reconciler (the same
    // sweep as dev-boot / `soli db:indexes`). Returns the report lines.
    // Double-underscore: internal surface, used by tests and setup scripts.
    env.define(
        "__sync_model_indexes".to_string(),
        Value::NativeFunction(NativeFunction::new_auto_invocable(
            "__sync_model_indexes",
            Some(0),
            |_args| {
                let lines: Vec<Value> = super::index_sync::sync_declared_indexes()
                    .into_iter()
                    .map(|l| Value::String(l.into()))
                    .collect();
                Ok(Value::Array(Rc::new(RefCell::new(lines))))
            },
        )),
    );

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

    // edge from: "users", to: "users" — mark the model as an edge collection.
    // The named args collapse into a trailing hash, so args = [Class, Hash].
    // Endpoints accept collection names or model classes. Records the edge
    // spec (drives Follow.create endpoint coercion + traverse()) and the
    // collection type (drives typed auto-create).
    env.define(
        "edge".to_string(),
        Value::NativeFunction(NativeFunction::new("edge", Some(2), |args| {
            use crate::interpreter::value::HashKey;
            let class_name = get_class_name_from_class(&args)?;
            let collection = class_name_to_collection(&class_name);

            let usage = "edge requires from: and to: collections, e.g. \
                         edge from: \"users\", to: \"users\"";
            let opts = match args.get(1) {
                Some(Value::Hash(h)) => h.clone(),
                _ => return Err(usage.to_string()),
            };
            let mut from_val: Option<Value> = None;
            let mut to_val: Option<Value> = None;
            for (k, v) in opts.borrow().iter() {
                match k {
                    HashKey::String(s) if s.as_str() == "from" => from_val = Some(v.clone()),
                    HashKey::String(s) if s.as_str() == "to" => to_val = Some(v.clone()),
                    HashKey::String(s) => {
                        return Err(format!("edge: unknown option '{}'. {}", s, usage))
                    }
                    _ => return Err(usage.to_string()),
                }
            }
            let (from_val, to_val) = match (from_val, to_val) {
                (Some(f), Some(t)) => (f, t),
                _ => return Err(usage.to_string()),
            };

            let from_collection = super::graph::endpoint_to_collection(&from_val)?;
            let to_collection = super::graph::endpoint_to_collection(&to_val)?;

            let mut metadata = get_or_create_metadata(&class_name);
            metadata.edge = Some(super::registry::EdgeSpec {
                from_collection,
                to_collection,
            });
            update_metadata(&class_name, metadata);
            super::registry::register_collection_type(&collection, "edge");
            Ok(Value::Null)
        })),
    );

    // timeseries [retention: "30d", timestamp: "recorded_at"] — mark the model
    // as a timeseries collection (insert-only on the DB side; UUIDv7 keys give
    // time ordering). Bare `timeseries` is valid: args = [Class] or
    // [Class, Hash].
    env.define(
        "timeseries".to_string(),
        Value::NativeFunction(NativeFunction::new("timeseries", None, |args| {
            use crate::interpreter::value::HashKey;
            let class_name = get_class_name_from_class(&args)?;
            let collection = class_name_to_collection(&class_name);

            let mut spec = super::registry::TimeseriesSpec::default();
            if let Some(opts) = args.get(1) {
                let hash = match opts {
                    Value::Hash(h) => h.clone(),
                    other => {
                        return Err(format!(
                            "timeseries options must be a hash (retention:, timestamp:), got {}",
                            other.type_name()
                        ))
                    }
                };
                for (k, v) in hash.borrow().iter() {
                    let key = match k {
                        HashKey::String(s) => s.to_string(),
                        _ => {
                            return Err(
                                "timeseries options must be retention: or timestamp:".to_string()
                            )
                        }
                    };
                    match key.as_str() {
                        "retention" => {
                            let val = match v {
                                Value::String(s) => s.to_string(),
                                other => {
                                    return Err(format!(
                                        "timeseries retention: must be a duration string \
                                         like \"30d\", got {}",
                                        other.type_name()
                                    ))
                                }
                            };
                            validate_retention_duration(&val)?;
                            spec.retention = Some(val);
                        }
                        "timestamp" => {
                            let val = match v {
                                Value::String(s) => s.to_string(),
                                Value::Symbol(s) => s.to_string(),
                                other => {
                                    return Err(format!(
                                        "timeseries timestamp: must be a field name, got {}",
                                        other.type_name()
                                    ))
                                }
                            };
                            validate_field_name(&val, "timeseries")?;
                            spec.timestamp_field = Some(val);
                        }
                        other => {
                            return Err(format!(
                                "timeseries: unknown option '{}': expected retention: or \
                                 timestamp:",
                                other
                            ))
                        }
                    }
                }
            }

            let mut metadata = get_or_create_metadata(&class_name);
            metadata.timeseries = Some(spec);
            update_metadata(&class_name, metadata);
            super::registry::register_collection_type(&collection, "timeseries");
            Ok(Value::Null)
        })),
    );

    // columnar [compression: "lz4"|"none"] — mark the model as backed by the
    // columnar engine (its own HTTP API; no document CRUD). Declare the
    // schema with `column` lines below it.
    env.define(
        "columnar".to_string(),
        Value::NativeFunction(NativeFunction::new("columnar", None, |args| {
            use crate::interpreter::value::HashKey;
            let class_name = get_class_name_from_class(&args)?;

            let mut compression: Option<String> = None;
            if let Some(opts) = args.get(1) {
                let hash = match opts {
                    Value::Hash(h) => h.clone(),
                    other => {
                        return Err(format!(
                            "columnar options must be a hash (compression:), got {}",
                            other.type_name()
                        ))
                    }
                };
                for (k, v) in hash.borrow().iter() {
                    match k {
                        HashKey::String(s) if s.as_str() == "compression" => {
                            let val = match v {
                                Value::String(s) => s.to_lowercase().to_string(),
                                other => {
                                    return Err(format!(
                                        "columnar compression: must be \"lz4\" or \"none\", \
                                         got {}",
                                        other.type_name()
                                    ))
                                }
                            };
                            if val != "lz4" && val != "none" {
                                return Err(format!(
                                    "columnar compression: must be \"lz4\" or \"none\", got \
                                     {:?}",
                                    val
                                ));
                            }
                            compression = Some(val);
                        }
                        HashKey::String(s) => {
                            return Err(format!(
                                "columnar: unknown option '{}': expected compression:",
                                s
                            ))
                        }
                        _ => {}
                    }
                }
            }

            super::registry::set_columnar(&class_name, compression);
            Ok(Value::Null)
        })),
    );

    // column "name", "type"[, nullable: true, indexed: true] — declare one
    // column of a columnar model. Types are validated against the server
    // whitelist because unknown types silently degrade to String there.
    env.define(
        "column".to_string(),
        Value::NativeFunction(NativeFunction::new("column", None, |args| {
            use crate::interpreter::value::HashKey;
            let class_name = get_class_name_from_class(&args)?;

            let name = match args.get(1) {
                Some(Value::String(s)) => s.to_string(),
                Some(Value::Symbol(s)) => s.to_string(),
                _ => {
                    return Err(
                        "column expects a name and a type, e.g. column \"url\", \"string\""
                            .to_string(),
                    )
                }
            };
            validate_field_name(&name, "column")?;

            let data_type = match args.get(2) {
                Some(Value::String(s)) => s.to_lowercase().to_string(),
                Some(Value::Symbol(s)) => s.to_lowercase().to_string(),
                _ => {
                    return Err(format!(
                        "column \"{}\" requires a type: one of {}",
                        name,
                        super::columnar::COLUMN_TYPES.join(", ")
                    ))
                }
            };
            if !super::columnar::COLUMN_TYPES.contains(&data_type.as_str()) {
                return Err(format!(
                    "column \"{}\": unknown type {:?} — expected one of {}",
                    name,
                    data_type,
                    super::columnar::COLUMN_TYPES.join(", ")
                ));
            }

            let mut nullable = false;
            let mut indexed = false;
            if let Some(Value::Hash(opts)) = args.get(3) {
                for (k, v) in opts.borrow().iter() {
                    let key = match k {
                        HashKey::String(s) => s.to_string(),
                        _ => continue,
                    };
                    let flag = match v {
                        Value::Bool(b) => *b,
                        other => {
                            return Err(format!(
                                "column \"{}\" option {} must be a bool, got {}",
                                name,
                                key,
                                other.type_name()
                            ))
                        }
                    };
                    match key.as_str() {
                        "nullable" => nullable = flag,
                        "indexed" => indexed = flag,
                        other => {
                            return Err(format!(
                                "column \"{}\": unknown option '{}': expected nullable: or \
                                 indexed:",
                                name, other
                            ))
                        }
                    }
                }
            }

            super::registry::add_columnar_column(
                &class_name,
                super::registry::ColumnarColumnDef {
                    name,
                    data_type,
                    nullable,
                    indexed,
                },
            );
            Ok(Value::Null)
        })),
    );

    // vector_index "embedding", dimension: 1536[, metric:, m:,
    // ef_construction:, quantization:, name:] — declare an HNSW ANN index.
    // Makes similar() push down to the DB (exact: true opts out per call).
    // Ensured by sync_declared_indexes (dev boot / `soli db:indexes`).
    env.define(
        "vector_index".to_string(),
        Value::NativeFunction(NativeFunction::new("vector_index", None, |args| {
            use crate::interpreter::value::HashKey;
            let class_name = get_class_name_from_class(&args)?;
            let collection = class_name_to_collection(&class_name);

            let field = match args.get(1) {
                Some(Value::String(s)) => s.to_string(),
                Some(Value::Symbol(s)) => s.to_string(),
                _ => {
                    return Err("vector_index expects a field name, e.g. vector_index \
                                \"embedding\", dimension: 1536"
                        .to_string())
                }
            };
            validate_field_name(&field, "vector_index")?;

            let mut dimension: Option<usize> = None;
            let mut metric: Option<String> = None;
            let mut m: Option<usize> = None;
            let mut ef_construction: Option<usize> = None;
            let mut quantization: Option<String> = None;
            let mut name: Option<String> = None;

            if let Some(Value::Hash(opts)) = args.get(2) {
                for (k, v) in opts.borrow().iter() {
                    let key = match k {
                        HashKey::String(s) => s.to_string(),
                        _ => continue,
                    };
                    match (key.as_str(), v) {
                        ("dimension", Value::Int(n)) if *n > 0 => dimension = Some(*n as usize),
                        ("metric", Value::String(s)) => {
                            let val = s.to_lowercase().to_string();
                            if !["cosine", "euclidean", "dot_product", "dotproduct"]
                                .contains(&val.as_str())
                            {
                                return Err(format!(
                                    "vector_index metric: unknown '{}': expected cosine, \
                                     euclidean, or dot_product",
                                    val
                                ));
                            }
                            metric = Some(val);
                        }
                        ("m", Value::Int(n)) if *n > 0 => m = Some(*n as usize),
                        ("ef_construction", Value::Int(n)) if *n > 0 => {
                            ef_construction = Some(*n as usize)
                        }
                        ("quantization", Value::String(s)) => quantization = Some(s.to_string()),
                        ("name", Value::String(s)) => {
                            validate_field_name(s, "vector_index")?;
                            name = Some(s.to_string());
                        }
                        (other, _) => {
                            return Err(format!("vector_index: unknown/invalid option '{}'", other))
                        }
                    }
                }
            }

            let dimension = dimension.ok_or_else(|| {
                "vector_index requires dimension:, e.g. vector_index \"embedding\", \
                 dimension: 1536"
                    .to_string()
            })?;
            let _ = &collection;
            super::registry::add_vector_index(
                &class_name,
                super::registry::VectorIndexDef {
                    name: name.unwrap_or_else(|| format!("idx_{}", field)),
                    field,
                    dimension,
                    metric,
                    m,
                    ef_construction,
                    quantization,
                },
            );
            Ok(Value::Null)
        })),
    );

    // fulltext_index "title", "body"[, name: "..."] — declare an n-gram
    // fulltext index over one or more fields; enables Model.search().
    env.define(
        "fulltext_index".to_string(),
        Value::NativeFunction(NativeFunction::new("fulltext_index", None, |args| {
            use crate::interpreter::value::HashKey;
            let class_name = get_class_name_from_class(&args)?;
            let collection = class_name_to_collection(&class_name);

            let mut fields: Vec<String> = Vec::new();
            let mut name: Option<String> = None;
            for arg in args.iter().skip(1) {
                match arg {
                    Value::String(s) => {
                        validate_field_name(s, "fulltext_index")?;
                        fields.push(s.to_string());
                    }
                    Value::Symbol(s) => {
                        validate_field_name(s, "fulltext_index")?;
                        fields.push(s.to_string());
                    }
                    Value::Hash(opts) => {
                        for (k, v) in opts.borrow().iter() {
                            match (k, v) {
                                (HashKey::String(key), Value::String(s))
                                    if key.as_str() == "name" =>
                                {
                                    validate_field_name(s, "fulltext_index")?;
                                    name = Some(s.to_string());
                                }
                                (HashKey::String(key), _) => {
                                    return Err(format!("fulltext_index: unknown option '{}'", key))
                                }
                                _ => {}
                            }
                        }
                    }
                    other => {
                        return Err(format!(
                            "fulltext_index expects field names, got {}",
                            other.type_name()
                        ))
                    }
                }
            }
            if fields.is_empty() {
                return Err(
                    "fulltext_index requires at least one field, e.g. fulltext_index \
                     \"title\", \"body\""
                        .to_string(),
                );
            }
            super::registry::add_fulltext_index(
                &class_name,
                super::registry::FulltextIndexDef {
                    name: name.unwrap_or_else(|| format!("ft_{}", collection)),
                    fields,
                },
            );
            Ok(Value::Null)
        })),
    );

    // geo_index "location"[, name: "..."] — declare a geo index on a
    // {lat, lon} field; enables Model.near() / Model.within().
    env.define(
        "geo_index".to_string(),
        Value::NativeFunction(NativeFunction::new("geo_index", None, |args| {
            use crate::interpreter::value::HashKey;
            let class_name = get_class_name_from_class(&args)?;

            let field = match args.get(1) {
                Some(Value::String(s)) => s.to_string(),
                Some(Value::Symbol(s)) => s.to_string(),
                _ => {
                    return Err(
                        "geo_index expects a field name, e.g. geo_index \"location\"".to_string(),
                    )
                }
            };
            validate_field_name(&field, "geo_index")?;

            let mut name: Option<String> = None;
            if let Some(Value::Hash(opts)) = args.get(2) {
                for (k, v) in opts.borrow().iter() {
                    match (k, v) {
                        (HashKey::String(key), Value::String(s)) if key.as_str() == "name" => {
                            validate_field_name(s, "geo_index")?;
                            name = Some(s.to_string());
                        }
                        (HashKey::String(key), _) => {
                            return Err(format!("geo_index: unknown option '{}'", key))
                        }
                        _ => {}
                    }
                }
            }
            super::registry::add_geo_index(
                &class_name,
                super::registry::GeoIndexDef {
                    name: name.unwrap_or_else(|| format!("geo_{}", field)),
                    field,
                },
            );
            Ok(Value::Null)
        })),
    );

    // index "email", unique: true / index ["tenant_id", "email"], type:
    // "hash" — declare a secondary index. Types: hash, persistent (default),
    // fulltext, bloom, cuckoo.
    env.define(
        "index".to_string(),
        Value::NativeFunction(NativeFunction::new("index", None, |args| {
            use crate::interpreter::value::HashKey;
            let class_name = get_class_name_from_class(&args)?;
            let collection = class_name_to_collection(&class_name);

            let fields: Vec<String> = match args.get(1) {
                Some(Value::String(s)) => vec![s.to_string()],
                Some(Value::Symbol(s)) => vec![s.to_string()],
                Some(Value::Array(arr)) => {
                    let arr = arr.borrow();
                    let mut out = Vec::with_capacity(arr.len());
                    for v in arr.iter() {
                        match v {
                            Value::String(s) => out.push(s.to_string()),
                            Value::Symbol(s) => out.push(s.to_string()),
                            other => {
                                return Err(format!(
                                    "index expects string field names, got {} in array",
                                    other.type_name()
                                ))
                            }
                        }
                    }
                    out
                }
                _ => {
                    return Err("index expects a field name or array of field names, e.g. \
                                index \"email\", unique: true"
                        .to_string())
                }
            };
            if fields.is_empty() {
                return Err("index requires at least one field".to_string());
            }
            for f in &fields {
                validate_field_name(f, "index")?;
            }

            let mut index_type = "persistent".to_string();
            let mut unique = false;
            let mut name: Option<String> = None;
            if let Some(Value::Hash(opts)) = args.get(2) {
                for (k, v) in opts.borrow().iter() {
                    let key = match k {
                        HashKey::String(s) => s.to_string(),
                        _ => continue,
                    };
                    match (key.as_str(), v) {
                        ("unique", Value::Bool(b)) => unique = *b,
                        ("type", Value::String(s)) => {
                            let t = match s.to_lowercase().as_str() {
                                // skiplist/btree are aliases the server maps
                                // to persistent.
                                "skiplist" | "btree" => "persistent".to_string(),
                                t @ ("hash" | "persistent" | "fulltext" | "bloom" | "cuckoo") => {
                                    t.to_string()
                                }
                                other => {
                                    return Err(format!(
                                        "index type: unknown '{}': expected hash, persistent, \
                                         fulltext, bloom, or cuckoo",
                                        other
                                    ))
                                }
                            };
                            index_type = t;
                        }
                        ("name", Value::String(s)) => {
                            validate_field_name(s, "index")?;
                            name = Some(s.to_string());
                        }
                        (other, _) => {
                            return Err(format!("index: unknown/invalid option '{}'", other))
                        }
                    }
                }
            }

            super::registry::add_secondary_index(
                &class_name,
                super::registry::SecondaryIndexDef {
                    name: name
                        .unwrap_or_else(|| format!("idx_{}_{}", collection, fields.join("_"))),
                    fields,
                    index_type,
                    unique,
                },
            );
            Ok(Value::Null)
        })),
    );

    // encrypts(:field, ...) - Encrypt the named fields at rest (AES-256-GCM).
    // Auto-encrypted on create/save/update, auto-decrypted on load. The key
    // comes from SOLI_ENCRYPTION_KEY. NOTE: encrypted fields can't be queried
    // by value (the nonce is random, so the ciphertext differs each write).
    env.define(
        "encrypts".to_string(),
        Value::NativeFunction(NativeFunction::new("encrypts", None, |args| {
            let class_name = get_class_name_from_class(&args)?;
            let collection = class_name_to_collection(&class_name);
            for arg in args.iter().skip(1) {
                let field = match arg {
                    Value::String(s) => s.to_string(),
                    Value::Symbol(s) => s.to_string(),
                    other => {
                        return Err(format!(
                            "encrypts() expects field names (symbols or strings), got {}",
                            other.type_name()
                        ))
                    }
                };
                super::registry::register_encryption(&class_name, &collection, &field);
            }
            Ok(Value::Null)
        })),
    );

    // enum_field(:status, Status) - declare that a model field holds values of
    // an enum type. Stored as the variant tag (unit) / a tagged object
    // (payload); reconstructed to the enum value on read. The model class is
    // auto-prepended in class bodies, so args = [ModelClass, field, EnumClass].
    env.define(
        "enum_field".to_string(),
        Value::NativeFunction(NativeFunction::new("enum_field", Some(3), |args| {
            let class_name = get_class_name_from_class(&args)?;
            let field = match args.get(1) {
                Some(Value::String(s)) => s.to_string(),
                Some(Value::Symbol(s)) => s.to_string(),
                _ => {
                    return Err(
                        "enum_field(field, EnumType) expects a symbol or string field name"
                            .to_string(),
                    )
                }
            };
            let enum_class = match args.get(2) {
                Some(Value::Class(c)) => c.clone(),
                other => {
                    return Err(format!(
                        "enum_field(field, EnumType) expects an enum class as the second \
                         argument, got {}",
                        other
                            .map(|v| v.type_name())
                            .unwrap_or_else(|| "nothing".to_string())
                    ))
                }
            };
            super::registry::register_enum_field(&class_name, &field, enum_class);
            Ok(Value::Null)
        })),
    );

    // scope(name, fn) - Register a named scope on a model. The class is
    // auto-prepended in class bodies (see `executor/statements.rs`), so user
    // code reads naturally:
    //
    //   class User < Model
    //     scope("published", fn() { this.where("status = @s", { "s": "published" }) })
    //   end
    //
    // Inside the closure `this` is bound to a fresh QueryBuilder for the
    // model; calling `User.published` invokes it.
    env.define(
        "scope".to_string(),
        Value::NativeFunction(NativeFunction::new("scope", Some(3), |args| {
            let class_name = get_class_name_from_class(&args)?;
            let name = match args.get(1) {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Symbol(s)) => s.clone(),
                _ => {
                    return Err("scope(name, fn) expects a string or symbol scope name".to_string())
                }
            };
            let func = match args.get(2) {
                Some(Value::Function(f)) => f.clone(),
                _ => {
                    return Err("scope(name, fn) expects a function as second argument".to_string())
                }
            };
            super::scopes::register_scope(&class_name, &name, func);
            Ok(Value::Null)
        })),
    );

    // State machine DSL. `state_machine :field do … end` is intercepted in
    // `execute_class` (the block must run with `&mut Interpreter`), so this
    // native is only reached when `state_machine(...)` is (mis)used outside a
    // model class body — it raises a clear usage error.
    env.define(
        "state_machine".to_string(),
        Value::NativeFunction(NativeFunction::new("state_machine", None, |_args| {
            Err("state_machine(:field) { … } can only be used in a model class body".to_string())
        })),
    );
    // `event :name do … end` is intercepted inside the state_machine block
    // (`evaluate_call`); reached here only when used outside one.
    env.define(
        "event".to_string(),
        Value::NativeFunction(NativeFunction::new("event", None, |_args| {
            Err("event(:name) { … } can only be used inside a state_machine block".to_string())
        })),
    );
    // initial / transition / guard / before_transition / after_transition all
    // record onto the state machine currently being built.
    for (name, native) in super::state_machine::recorder_natives() {
        env.define(name.to_string(), Value::NativeFunction(native));
    }

    // uploader(name, options) - Declare a blob attachment on the model
    env.define(
        "uploader".to_string(),
        Value::NativeFunction(NativeFunction::new("uploader", Some(3), |args| {
            let class_name = get_class_name_from_class(&args)?;
            let config = build_uploader_config_from_args(&class_name, &args)?;
            register_uploader(&class_name, config);
            Ok(Value::Null)
        })),
    );

    // model_uploader_config(class_name, field) - Read an uploader config from
    // Soli code. Used by the CRM `attach_upload`/`detach_upload` helpers and
    // by the generic AttachmentsController to drive validation + storage.
    env.define(
        "model_uploader_config".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "model_uploader_config",
            Some(2),
            |args| {
                let class_name =
                    match args.first() {
                        Some(Value::String(s)) => s.clone(),
                        Some(Value::Class(c)) => c.name.clone().into(),
                        _ => return Err(
                            "model_uploader_config(class_name, field) expects a class or string"
                                .to_string(),
                        ),
                    };
                let field = match args.get(1) {
                    Some(Value::String(s)) => s.clone(),
                    _ => {
                        return Err(
                            "model_uploader_config(class_name, field) expects a string field"
                                .to_string(),
                        )
                    }
                };
                Ok(uploader_config_to_value(get_uploader(&class_name, &field)))
            },
        )),
    );

    // model_uploader_fields(class_name) → list of declared uploader field
    // names (e.g. ["photo"]). Lets generic helpers iterate every attachment
    // on a model — used by `detach_all_uploads` for destroy-time cleanup.
    env.define(
        "model_uploader_fields".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "model_uploader_fields",
            Some(1),
            |args| {
                let class_name = match args.first() {
                    Some(Value::String(s)) => s.clone(),
                    Some(Value::Class(c)) => c.name.clone().into(),
                    _ => {
                        return Err(
                            "model_uploader_fields(class_name) expects a class or string"
                                .to_string(),
                        )
                    }
                };
                use super::uploaders::get_uploaders;
                let names: Vec<Value> = get_uploaders(&class_name)
                    .into_iter()
                    .map(|u| Value::String(u.name.into()))
                    .collect();
                Ok(Value::Array(Rc::new(RefCell::new(names))))
            },
        )),
    );

    // find_model_class_by_collection("contacts") → Contact class, or null.
    // Lets controllers route on URL segments without hardcoding a resource→
    // class table. Walks the MODEL_CLASSES thread-local (populated when
    // model files are loaded) and matches on `class_name_to_collection`.
    env.define(
        "find_model_class_by_collection".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "find_model_class_by_collection",
            Some(1),
            |args| {
                let collection = match args.first() {
                    Some(Value::String(s)) => s.clone(),
                    _ => {
                        return Err(
                            "find_model_class_by_collection(name) expects a string".to_string()
                        )
                    }
                };
                use super::registry::MODEL_CLASSES;
                let result = MODEL_CLASSES.with(|classes| {
                    classes.borrow().iter().find_map(|(name, class)| {
                        if *class_name_to_collection(name) == *collection {
                            Some(Value::Class(class.clone()))
                        } else {
                            None
                        }
                    })
                });
                Ok(result.unwrap_or(Value::Null))
            },
        )),
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
                    Some(Value::Symbol(s)) => s.clone(),
                    Some(other) => {
                        return Err(format!(
                            "relation expects string or symbol name, got {}",
                            other.type_name()
                        ))
                    }
                    None => return Err("relation requires a name argument".to_string()),
                };

                let options = parse_relation_options(args.get(2), &rel_type)?;

                let relation = build_relation(&class_name, &name, rel_type.clone(), &options);
                register_relation(&class_name, relation);
                Ok(Value::Null)
            })),
        );
    }

    // has_and_belongs_to_many global function
    env.define(
        "has_and_belongs_to_many".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "has_and_belongs_to_many",
            None,
            |args| {
                let class_name = get_class_name_from_class(&args)?;
                let name = match args.get(1) {
                    Some(Value::String(s)) => s.clone(),
                    Some(Value::Symbol(s)) => s.clone(),
                    Some(other) => {
                        return Err(format!(
                            "has_and_belongs_to_many expects string or symbol name, got {}",
                            other.type_name()
                        ))
                    }
                    None => {
                        return Err("has_and_belongs_to_many requires a name argument".to_string())
                    }
                };

                let options =
                    parse_relation_options(args.get(2), &RelationType::HasAndBelongsToMany)?;

                let relation = build_habtm_relation(&class_name, &name, &options);
                register_relation(&class_name, relation);
                Ok(Value::Null)
            },
        )),
    );
}
