//! Core Model types, registry, and database configuration.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, Function, NativeFunction, SoliStr, Value};

pub use super::db_config::{
    db_host, db_scheme_and_host, db_url, force_refresh_jwt_token, get_api_key, get_basic_auth,
    get_cursor_url, get_database_name, get_jwt_token, init_db_config, init_jwt_token,
    resolve_api_key, resolve_basic_auth, with_soli_db_config, DbConfig,
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
    build_habtm_relation, build_relation, parse_relation_options, register_relation, RelationType,
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

/// `find_by` / `first_by` on a SQL connection: the raw-SDBQL form those
/// methods build is SoliDB-only, so express the lookup as a portable
/// eq-filter `ListQuery` instead. Errors propagate — swallowing them to nil
/// makes an outage indistinguishable from "not found".
pub(super) fn sql_find_first_by(
    class: &Rc<Class>,
    collection: &str,
    field: &str,
    value: serde_json::Value,
    order_by_key: bool,
) -> Result<Value, String> {
    // Column-aware models filter on a real column.
    if let Some(schema) = super::column_mode::require_schema(collection)? {
        let mut q = crate::db::sql_columns_compile::ColumnQuery::new(schema.clone());
        q.eq_filters.insert(field.to_string(), value);
        if super::registry::is_sti_subclass(&class.name) {
            if !schema.has_column("type") {
                return Err(format!(
                    "{}.find_by/first_by needs a `type` column on table {:?} \
                     (STI subclass).",
                    class.name, schema.table
                ));
            }
            q.in_filters.insert(
                "type".to_string(),
                super::registry::sti_type_names(&class.name)
                    .into_iter()
                    .map(serde_json::Value::String)
                    .collect(),
            );
        }
        if order_by_key {
            q.order_field = Some(schema.pk.clone());
        }
        q.limit = Some(1);
        let rows = crate::db::columns::select_rows(&q)?;
        return Ok(match rows.into_iter().next() {
            Some(row) => super::column_mode::convert_temporal_fields(
                &schema,
                super::crud::json_doc_to_instance_owned(class, row),
            ),
            None => Value::Null,
        });
    }
    let mut eq_filters = std::collections::BTreeMap::new();
    eq_filters.insert(field.to_string(), value);
    let mut lq = crate::db::ListQuery {
        table: collection.to_string(),
        eq_filters,
        hash_filter: None,
        filter_sdbql: None,
        having: None,
        exists_filters: Vec::new(),
        // Mirrors the SoliDB form, which applies no deleted_at scope here.
        soft_delete: crate::db::SqlSoftDeleteMode::Default,
        is_soft_delete_model: false,
        order_field: order_by_key.then(|| "_key".to_string()),
        order_desc: false,
        limit: Some(1),
        offset: None,
    };
    if super::registry::is_sti_subclass(&class.name) {
        lq.hash_filter = Some(crate::db::hash_filter::HashFilter::In {
            field: "type".to_string(),
            values: super::registry::sti_type_names(&class.name)
                .into_iter()
                .map(serde_json::Value::String)
                .collect(),
        });
    }
    let rows = crate::db::sql::select(&lq)?;
    Ok(match rows.into_iter().next() {
        Some(doc) => super::crud::json_doc_to_instance_owned(class, doc),
        None => Value::Null,
    })
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
    fn hash_form_array_is_an_in_list() {
        let mut pairs = crate::interpreter::value::HashPairs::default();
        pairs.insert(
            crate::interpreter::value::HashKey::String("ids".into()),
            Value::Array(std::rc::Rc::new(std::cell::RefCell::new(vec![
                Value::String("a".into()),
                Value::String("b".into()),
            ]))),
        );
        let hash = std::rc::Rc::new(std::cell::RefCell::new(pairs));
        let (pred, sdbql, _binds) = parse_hash_filter(&hash, "where").unwrap();
        assert!(matches!(
            pred,
            crate::db::hash_filter::HashFilter::In { .. }
        ));
        assert!(sdbql.contains("IN @"), "{sdbql}");
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

/// Extract a required string-or-symbol argument at `idx` as a `SoliStr`.
///
/// Collapses the identical `match args.get(idx) { String | Symbol => clone,
/// other => type error, None => missing-arg error }` block repeated across the
/// model DSL statics. `method` is the DSL name shown in the message (include
/// `()` where the caller does); `what` names the argument, e.g. "field name".
pub(super) fn arg_str_or_sym(
    args: &[Value],
    idx: usize,
    method: &str,
    what: &str,
) -> Result<SoliStr, String> {
    match args.get(idx) {
        Some(Value::String(s)) | Some(Value::Symbol(s)) => Ok(s.clone()),
        Some(other) => Err(format!(
            "{method} expects string or symbol {what}, got {}",
            other.type_name()
        )),
        None => Err(format!("{method} requires {what} argument")),
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
pub(super) fn get_class_rc_from_args(args: &[Value]) -> Result<Rc<Class>, String> {
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
    let (pred, sdbql, binds) = parse_hash_filter(hash, method)?;
    let _ = pred;
    Ok((sdbql, binds))
}

/// Parse a hash `.where` into the structured IR plus the SDBQL string SoliDB
/// still executes. SQL compilers use the IR so a `gt` cannot become `==`.
pub fn parse_hash_filter(
    hash: &Rc<RefCell<crate::interpreter::value::HashPairs>>,
    method: &str,
) -> Result<
    (
        crate::db::hash_filter::HashFilter,
        String,
        std::collections::HashMap<String, serde_json::Value>,
    ),
    String,
> {
    use crate::interpreter::value::{value_to_json, HashKey};
    let pairs = hash.borrow();
    let mut map = serde_json::Map::new();
    for (k, v) in pairs.iter() {
        let HashKey::String(key) = k else {
            return Err(format!("{method}() Hash filter keys must be strings"));
        };
        reject_client_supplied_operator(v, key.as_ref(), method)?;
        map.insert(
            key.to_string(),
            value_to_json(v).map_err(|e| e.to_string())?,
        );
    }
    let pred = crate::db::hash_filter::HashFilter::from_json_map(&map, method)?;
    let (sdbql, binds) = pred.to_sdbql();
    Ok((pred, sdbql, binds))
}

/// Same as [`parse_hash_filter`], rendered in a bind-name namespace that avoids
/// the names already on a query builder.
///
/// Bind names are `field__op_n` with `n` restarting per render, so chaining two
/// `.where` hashes that touch the same field produced the same name twice and
/// the later value quietly replaced the earlier one — turning an ownership
/// scope into whatever the client passed. Each generation is tried in turn
/// until the new names are disjoint from `taken`.
pub fn parse_hash_filter_avoiding(
    hash: &Rc<RefCell<crate::interpreter::value::HashPairs>>,
    method: &str,
    taken: &dyn Fn(&str) -> bool,
) -> Result<
    (
        crate::db::hash_filter::HashFilter,
        String,
        std::collections::HashMap<String, serde_json::Value>,
    ),
    String,
> {
    let (pred, sdbql, binds) = parse_hash_filter(hash, method)?;
    if !binds.keys().any(|name| taken(name)) {
        return Ok((pred, sdbql, binds));
    }
    // A handful of generations is plenty: each one is a fresh namespace, so a
    // collision can only repeat if the builder already holds names from that
    // exact generation.
    for generation in 1..=64u32 {
        let (sdbql, binds) = pred.to_sdbql_in_generation(generation);
        if !binds.keys().any(|name| taken(name)) {
            return Ok((pred, sdbql, binds));
        }
    }
    Err(format!(
        "{method}(): could not find a free bind-variable namespace for this filter;          chain fewer conditions on the same fields or use the string form with          explicit bind names"
    ))
}

/// `.join("posts")` / `.join("posts", { published: true })` /
/// `.join("posts", "published = @p", { p: true })`.
type JoinFilterArgs = (
    Option<String>,
    std::collections::HashMap<String, serde_json::Value>,
    Option<crate::db::hash_filter::HashFilter>,
);

pub fn parse_join_filter_args(
    filter_arg: Option<&Value>,
    binds_arg: Option<&Value>,
) -> Result<JoinFilterArgs, String> {
    match filter_arg {
        Some(Value::Hash(hash)) => {
            let (pred, sdbql, binds) = parse_hash_filter(hash, "join")?;
            Ok((
                if sdbql.trim().is_empty() {
                    None
                } else {
                    Some(sdbql)
                },
                binds,
                if pred.is_empty() { None } else { Some(pred) },
            ))
        }
        Some(Value::String(s)) => {
            let binds = match binds_arg {
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
            Ok((Some(s.to_string()), binds, None))
        }
        _ => Ok((None, std::collections::HashMap::new(), None)),
    }
}

/// Refuse a filter value that a client chose *and* that would act as an
/// operator map or an `IN` list.
///
/// The hash form of `.where` reads a nested hash as operators (`{"gt": 10}`)
/// and an array as `IN`. That is a documented convenience for filters the
/// developer writes, and it is indistinguishable, by shape alone, from what a
/// JSON body can send — so
///
/// ```soli
/// User.where({ "email": params["email"], "api_token": params["token"] }).first
/// ```
///
/// turned into `api_token != null` when the client posted
/// `{"token": {"ne": null}}`, and the secret check simply vanished. (SEC-062
/// guarded exactly this with a scalar-only rule; the hash-filter IR rewrite
/// stopped calling it.)
///
/// Rather than removing the operator syntax, we ask the one question that
/// actually separates the two cases: did this container arrive with the
/// request? A literal written in a `.sl` file never did.
fn reject_client_supplied_operator(value: &Value, key: &str, method: &str) -> Result<(), String> {
    let shape = match value {
        Value::Hash(_) => "an operator map",
        Value::Array(_) => "an IN list",
        _ => return Ok(()),
    };
    if !crate::interpreter::taint::is_request_supplied(value) {
        return Ok(());
    }
    Err(format!(
        "{method}(): the value for '{key}' came from the request and would be read as          {shape}, not compared. A client can turn an equality check into          `{{\"ne\": null}}` this way and bypass it. Compare against a scalar          (`str(params[\"{key}\"])`), or whitelist the shape first with          `permit(params, {{\"{key}\": true}})`, which drops containers from          scalar slots."
    ))
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

/// Longest field name `validate_field_name` accepts.
pub const MAX_FIELD_NAME_LEN: usize = 128;

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
    // A field name is often a request parameter (`Post.order(params["sort"])`)
    // and ends up in query text, logs and caches; no real field is this long.
    if field.len() > MAX_FIELD_NAME_LEN {
        return Err(format!(
            "{}() field name is longer than {} characters",
            method, MAX_FIELD_NAME_LEN
        ));
    }
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
pub(super) fn collect_accessible_fields(args: &[Value]) -> Result<Vec<String>, String> {
    if args.len() == 1 {
        if let Value::Array(arr) = &args[0] {
            let arr = arr.borrow();
            let mut out = Vec::with_capacity(arr.len());
            for v in arr.iter() {
                match v {
                    Value::String(s) | Value::Symbol(s) => out.push(s.to_string()),
                    other => {
                        return Err(format!(
                            "attr_accessible() expects a field name (string or symbol)s, got {} in array",
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
            Value::String(s) | Value::Symbol(s) => out.push(s.to_string()),
            other => {
                return Err(format!(
                    "attr_accessible() expects a field name (string or symbol)s, got {}",
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
pub(super) fn build_uploader_config_from_args(
    class_name: &str,
    args: &[Value],
) -> Result<UploaderConfig, String> {
    use crate::interpreter::value::HashKey;

    let name = match args.get(1) {
        Some(Value::String(s) | Value::Symbol(s)) => s.clone(),
        Some(other) => {
            return Err(format!(
                "uploader() expects a field name (string or symbol), got {}",
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
    let mut service: Option<String> = None;

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
                "service" => {
                    if let Value::String(s) = v {
                        let normalized = s.to_lowercase();
                        if !matches!(normalized.as_str(), "solidb" | "disk" | "s3") {
                            return Err(format!(
                                "uploader(\"{}\") service must be \"solidb\", \"disk\", or \"s3\", got {:?}",
                                name, s
                            ));
                        }
                        service = Some(normalized.to_string());
                    }
                }
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
    let service = service.unwrap_or_else(|| "solidb".to_string());

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
        service,
    })
}

/// Turn the query layer's in-band error value into a real error.
///
/// Several query entry points signal failure by returning
/// `Value::String("Error: …")` rather than an `Err`. That is fine where the
/// caller renders the value, but for a scalar result — a count, an aggregate —
/// it hands back a String where a number belongs, and `try`/`catch` never fires.
pub(super) fn raise_if_error_value(value: Value) -> Result<Value, String> {
    if let Value::String(text) = &value {
        if let Some(message) = text.strip_prefix("Error: ") {
            return Err(message.to_string());
        }
    }
    Ok(value)
}

/// The allowlist `has_one_attached` / `has_many_attached` use when the
/// declaration names no `content_types`. Deliberately excludes every type a
/// browser will execute script from — `text/html`, `application/xhtml+xml`,
/// `image/svg+xml`, `text/xml` — because the blob route serves attachments
/// from the application's own origin. Pass an explicit `content_types` to
/// narrow it; there is no way to widen it to "anything".
///
/// This default used to be "any content type", so it is a breaking change for
/// an app that upgrades while storing something not listed here — hence the
/// list stays as wide as it safely can, and only the script-executable types
/// above are held back. `Content-Disposition: attachment` and `nosniff` on the
/// blob route (see `serve/uploads_prelude.rs`) are the primary defence; this
/// allowlist is defence-in-depth on top of it.
const DEFAULT_ATTACHMENT_CONTENT_TYPES: &[&str] = &[
    "image/jpeg",
    "image/png",
    "image/gif",
    "image/webp",
    "image/avif",
    "image/heic",
    "image/heif",
    "image/bmp",
    "image/tiff",
    "application/pdf",
    "text/plain",
    "text/markdown",
    "text/csv",
    "application/json",
    "application/zip",
    // The MIME older Windows clients send for a .zip.
    "application/x-zip-compressed",
    "application/msword",
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    "application/vnd.ms-excel",
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    "application/vnd.ms-powerpoint",
    "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    "audio/mpeg",
    "audio/mp4",
    "audio/ogg",
    "audio/wav",
    "video/mp4",
    "video/webm",
    "video/quicktime",
];

/// `has_one_attached("avatar")` / `has_many_attached("photos", { ... })`.
/// Options are optional. Defaults: disk service, 10 MiB cap, and the
/// `DEFAULT_ATTACHMENT_CONTENT_TYPES` allowlist.
pub(super) fn build_attached_config_from_args(
    class_name: &str,
    args: &[Value],
    multiple: bool,
) -> Result<UploaderConfig, String> {
    use crate::interpreter::value::HashKey;
    let name = match args.get(1) {
        Some(Value::String(s) | Value::Symbol(s)) => s.clone(),
        Some(other) => {
            return Err(format!(
                "has_*_attached() expects a field name (string or symbol), got {}",
                other.type_name()
            ))
        }
        None => return Err("has_*_attached() requires a field name".to_string()),
    };

    let options = match args.get(2) {
        Some(Value::Hash(hash)) => Some(hash.borrow().clone()),
        Some(Value::Null) | None => None,
        Some(other) => {
            return Err(format!(
                "has_*_attached() expects an options hash, got {}",
                other.type_name()
            ))
        }
    };

    let mut content_types: Vec<String> = Vec::new();
    let mut max_size: Option<u64> = None;
    let mut collection: Option<String> = None;
    let mut format: Option<String> = None;
    let mut quality: Option<u8> = None;
    let mut max_width: Option<u32> = None;
    let mut max_height: Option<u32> = None;
    let mut service: Option<String> = None;

    if let Some(options) = options {
        for (k, v) in options {
            if let HashKey::String(key) = k {
                match key.as_ref() {
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
                        _ => {}
                    },
                    "collection" => {
                        if let Value::String(s) = v {
                            collection = Some(s.to_string());
                        }
                    }
                    "format" => {
                        if let Value::String(s) = v {
                            format = Some(s.to_lowercase().to_string());
                        }
                    }
                    "quality" => match v {
                        Value::Int(n) if (1..=100).contains(&n) => quality = Some(n as u8),
                        _ => {}
                    },
                    "max_width" => match v {
                        Value::Int(n) if n > 0 => max_width = Some(n as u32),
                        _ => {}
                    },
                    "max_height" => match v {
                        Value::Int(n) if n > 0 => max_height = Some(n as u32),
                        _ => {}
                    },
                    "service" => {
                        if let Value::String(s) = v {
                            let normalized = s.to_lowercase();
                            if !matches!(normalized.as_str(), "solidb" | "disk" | "s3") {
                                return Err(format!(
                                    "has_*_attached(\"{}\") service must be \"solidb\", \"disk\", or \"s3\"",
                                    name
                                ));
                            }
                            service = Some(normalized.to_string());
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    let env_service = std::env::var("SOLI_ATTACHMENTS_SERVICE")
        .ok()
        .map(|s| s.to_lowercase())
        .filter(|s| matches!(s.as_str(), "solidb" | "disk" | "s3"));
    let service = service
        .or(env_service)
        .unwrap_or_else(|| "disk".to_string());
    let max_size = max_size.unwrap_or(10 * 1024 * 1024);
    let collection = collection.unwrap_or_else(|| default_collection(class_name, &name));

    // `uploader()` *requires* a non-empty content_types; `has_*_attached` is the
    // low-ceremony form and takes a default instead. That default must never be
    // "anything", because the blob route echoes the stored content type back
    // from the app's own origin: an empty allowlist accepted `text/html` and
    // turned every bare `has_one_attached("avatar")` into stored XSS. Note the
    // omissions — no text/html, no image/svg+xml, no XML — all of which execute
    // script when a browser renders them.
    let content_types = if content_types.is_empty() {
        DEFAULT_ATTACHMENT_CONTENT_TYPES
            .iter()
            .map(|s| s.to_string())
            .collect()
    } else {
        content_types
    };

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
        service,
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
    pairs.insert(
        HashKey::String("service".into()),
        Value::String(c.service.into()),
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
pub(super) fn instance_fields_to_hash(
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
/// Strip the keys a client must never choose from a document about to be
/// persisted.
///
/// `attr_accessible` is opt-in, and without it `filter_mass_assign` hands the
/// hash back untouched — so `Post.create(req["json"])`, a shape the docs
/// present as legal, let the client pick its own `_key` (predictable ids, key
/// squatting, 409 collisions), set `_from`/`_to` on an edge collection, forge
/// `_id`/`_rev`, or set `type` on an STI base class so the row hydrates as a
/// privileged subclass. `apply_hash_to_instance` already skipped `_`-prefixed
/// keys, but only for the in-memory instance: the JSON sent to the server kept
/// them, and the server honours a supplied `_key`.
///
/// Applies to `create` and the static `update` regardless of whitelists.
/// `type` is only reserved where it is a discriminator, so ordinary models can
/// still have a `type` column.
/// Does this class declare any validation rules or custom validators?
pub(super) fn class_has_validations(class_name: &str) -> bool {
    super::validation::class_has_validations(class_name)
}

/// Overlay an update patch on the stored document, so validations see the
/// record as it will be *after* the write rather than only the changed fields.
/// A missing stored document (a create-through-update, or a read that failed)
/// simply validates the patch on its own.
pub(super) fn merge_json_documents(
    stored: Option<&serde_json::Value>,
    patch: &serde_json::Value,
) -> serde_json::Value {
    let mut merged = match stored {
        Some(serde_json::Value::Object(map)) => map.clone(),
        _ => serde_json::Map::new(),
    };
    if let serde_json::Value::Object(patch_map) = patch {
        for (key, value) in patch_map {
            merged.insert(key.clone(), value.clone());
        }
    }
    serde_json::Value::Object(merged)
}

pub(super) fn strip_reserved_document_keys(data: &mut serde_json::Value, class_name: &str) {
    let serde_json::Value::Object(map) = data else {
        return;
    };
    map.retain(|key, _| !key.starts_with('_'));
    if super::registry::is_sti_subclass(class_name) || super::registry::is_sti_base(class_name) {
        map.remove("type");
    }
}

pub(super) fn filter_mass_assign(class_name: &str, data: &Value) -> Value {
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
pub(super) fn apply_hash_to_instance(
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
pub(super) fn build_persistence_errors(class_name: &str, err: String) -> Vec<Value> {
    if super::validation::is_unique_violation(&err) {
        return super::validation::build_unique_violation_errors(class_name, &err)
            .iter()
            .map(|v| v.to_value())
            .collect();
    }
    // A foreign-key or NOT NULL violation is just as much a validation failure
    // as a duplicate: the caller wants the field, not the driver's sentence.
    if let Some(errors) = super::validation::build_constraint_errors(&err) {
        return errors.iter().map(|v| v.to_value()).collect();
    }
    vec![Value::String(err.into())]
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

        // rerank(query, docs[, { field:, limit: }]) -> reordered Array
        // Client-side lexical reranking by query-token overlap — no LLM, no
        // server round-trip. Reorder rows from similar/graph_rag/hybrid by a phrase.
        env.define(
            "rerank".to_string(),
            Value::NativeFunction(NativeFunction::new("rerank", None, |args| {
                super::rerank::exec_rerank(args)
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

    /// Register the `Model` class: every static and instance method the ORM
    /// exposes.
    ///
    /// This was 4,329 lines of inline closures, and the reason this file ran to
    /// seven thousand. It now says which group of methods lives where; the
    /// `register_*` siblings hold them. Order is preserved but does not matter:
    /// each entry is an independent closure keyed by its own name.
    fn register_model_class(env: &mut Environment) {
        let mut native_static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

        super::register_dsl::register(&mut native_static_methods);
        super::register_chain::register(&mut native_static_methods);
        super::register_crud::register(&mut native_static_methods);
        super::register_mutate::register(&mut native_static_methods);
        super::register_finders::register(&mut native_static_methods);
        super::register_aggregate::register(&mut native_static_methods);
        super::register_search::register(&mut native_static_methods);

        let mut native_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();
        super::register_instance::register(&mut native_methods);

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
pub(super) fn apply_field_delta(args: &[Value], sign: i64, op_name: &str) -> Result<Value, String> {
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
        Some(Value::String(s) | Value::Symbol(s)) => s.clone(),
        _ => {
            return Err(format!(
                "{}() expects a field name (string or symbol)",
                op_name
            ))
        }
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
            inst_mut.set("_rev", Value::String(new_rev.into()));
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
            let class_name = get_class_name_from_class(args)?;

            let field = arg_str_or_sym(args, 1, "validates()", "field name")?;

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
            let class_name = get_class_name_from_class(args)?;
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
                    let class_name = get_class_name_from_class(args)?;
                    let method_name = arg_str_or_sym(
                        args,
                        1,
                        &format!("{}()", callback_name_for_closure),
                        "method name",
                    )?;
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
            let class_name = get_class_name_from_class(args)?;
            let mut metadata = get_or_create_metadata(&class_name);
            metadata.soft_delete = true;
            update_metadata(&class_name, metadata);
            Ok(Value::Null)
        })),
    );

    // connection "name" — bind this model to a named entry in
    // config/database.toml (or the env-derived primary connection).
    env.define(
        "connection".to_string(),
        Value::NativeFunction(NativeFunction::new("connection", Some(2), |args| {
            let class_name = get_class_name_from_class(args)?;
            let name = match args.get(1) {
                Some(Value::String(s) | Value::Symbol(s)) => s.to_string(),
                Some(other) => {
                    return Err(format!(
                        "connection() expects a name (string or symbol), got {}",
                        other.type_name()
                    ))
                }
                None => return Err("connection() requires a connection name".to_string()),
            };
            // Validate name exists in the registry early so class load fails
            // fast rather than on first query.
            crate::db::registry().resolve(Some(&name))?;
            super::registry::register_connection(&class_name, &name);
            Ok(Value::Null)
        })),
    );

    // table "orders" — bind the model to an EXISTING relational table with real
    // columns (column mode) instead of the `_key` + `doc` document layout.
    //
    // Records the mapping only: the schema is introspected on first use (and by
    // the boot sweep), so `connection` and `table` may appear in either order,
    // and loading a class never depends on the database being reachable.
    env.define(
        "table".to_string(),
        Value::NativeFunction(NativeFunction::new("table", Some(2), |args| {
            let class_name = get_class_name_from_class(args)?;
            let name = match args.get(1) {
                Some(Value::String(s) | Value::Symbol(s)) => s.to_string(),
                Some(other) => {
                    return Err(format!(
                        "table() expects a table name (string or symbol), got {}",
                        other.type_name()
                    ))
                }
                None => return Err("table() requires a table name".to_string()),
            };
            // Reject a name that could not be quoted safely, at declaration
            // rather than at query time.
            crate::db::sql_compile::quote_ident(&name).map_err(|e| {
                format!(
                    "table {name:?} is not a usable SQL identifier ({e}). \
                     Column mode supports plain tables in the connection's \
                     default schema — no \"schema.table\" qualification."
                )
            })?;
            super::registry::register_table_mapping(&class_name, &name);
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
            let class_name = get_class_name_from_class(args)?;
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
            let class_name = get_class_name_from_class(args)?;
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
            let class_name = get_class_name_from_class(args)?;

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
            let class_name = get_class_name_from_class(args)?;

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
            let class_name = get_class_name_from_class(args)?;
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
            let class_name = get_class_name_from_class(args)?;
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
            let class_name = get_class_name_from_class(args)?;

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
            let class_name = get_class_name_from_class(args)?;
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
                                "index expects a field name (string or symbol)s, got {} in array",
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
            let class_name = get_class_name_from_class(args)?;
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
            let class_name = get_class_name_from_class(args)?;
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
            let class_name = get_class_name_from_class(args)?;
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
            let class_name = get_class_name_from_class(args)?;
            let config = build_uploader_config_from_args(&class_name, args)?;
            register_uploader(&class_name, config);
            Ok(Value::Null)
        })),
    );
    env.define(
        "has_one_attached".to_string(),
        Value::NativeFunction(NativeFunction::new("has_one_attached", None, |args| {
            let class_name = get_class_name_from_class(args)?;
            let config = build_attached_config_from_args(&class_name, args, false)?;
            register_uploader(&class_name, config);
            Ok(Value::Null)
        })),
    );
    env.define(
        "has_many_attached".to_string(),
        Value::NativeFunction(NativeFunction::new("has_many_attached", None, |args| {
            let class_name = get_class_name_from_class(args)?;
            let config = build_attached_config_from_args(&class_name, args, true)?;
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
                        // A record answers for its own class, so a caller
                        // holding one does not have to remember whether the
                        // helper wanted the instance or the class.
                        Some(Value::Instance(i)) => i.borrow().class.name.clone().into(),
                        _ => return Err(
                            "model_uploader_config(class_name, field) expects a class, a record or a string"
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
                let class_name = get_class_name_from_class(args)?;
                let name = arg_str_or_sym(args, 1, "relation", "name")?;

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
                let class_name = get_class_name_from_class(args)?;
                let name = arg_str_or_sym(args, 1, "has_and_belongs_to_many", "name")?;

                let options =
                    parse_relation_options(args.get(2), &RelationType::HasAndBelongsToMany)?;

                let relation = build_habtm_relation(&class_name, &name, &options);
                register_relation(&class_name, relation);
                Ok(Value::Null)
            },
        )),
    );
}

#[cfg(test)]
mod where_operator_injection_tests {
    use super::*;
    use crate::interpreter::value::{HashKey, HashPairs};

    fn hash(pairs: Vec<(&str, Value)>) -> Rc<RefCell<HashPairs>> {
        let mut out = HashPairs::default();
        for (k, v) in pairs {
            out.insert(HashKey::String((*k).into()), v);
        }
        Rc::new(RefCell::new(out))
    }

    /// The reported bypass: a controller compares a secret against a request
    /// value, and the client sends an operator map instead of a string.
    ///
    /// ```soli
    /// User.where({ "email": params["email"], "api_token": params["token"] }).first
    /// ```
    /// with `{"email": "admin@x.com", "token": {"ne": null}}` used to compile to
    /// `doc.api_token != @bind` (bind `null`) — the token check gone.
    #[test]
    fn a_request_supplied_operator_map_is_refused() {
        crate::interpreter::taint::clear_request_values();

        let operator_map = Value::Hash(hash(vec![("ne", Value::Null)]));
        let params = Value::Hash(hash(vec![
            ("email", Value::String("admin@x.com".into())),
            ("token", operator_map.clone()),
        ]));
        crate::interpreter::taint::mark_request_value(&params);

        let filter = hash(vec![
            ("email", Value::String("admin@x.com".into())),
            ("api_token", operator_map),
        ]);
        let err = parse_hash_filter(&filter, "where")
            .expect_err("a client-supplied operator map must not build a filter");
        assert!(err.contains("api_token"), "{err}");
        assert!(err.contains("came from the request"), "{err}");
        assert!(
            err.contains("permit"),
            "the error should say how to fix it: {err}"
        );
    }

    /// An array from the request would silently widen an equality check into
    /// `IN (...)`.
    #[test]
    fn a_request_supplied_in_list_is_refused() {
        crate::interpreter::taint::clear_request_values();

        let list = Value::Array(Rc::new(RefCell::new(vec![Value::Int(1), Value::Int(2)])));
        let params = Value::Hash(hash(vec![("role", list.clone())]));
        crate::interpreter::taint::mark_request_value(&params);

        let filter = hash(vec![("role_id", list)]);
        let err = parse_hash_filter(&filter, "where").expect_err("an IN list from a client");
        assert!(err.contains("IN list"), "{err}");
    }

    /// The documented developer-authored operator syntax keeps working — that
    /// is the whole reason the fix is a provenance check and not a shape check.
    #[test]
    fn a_literal_operator_map_still_compiles() {
        crate::interpreter::taint::clear_request_values();

        let filter = hash(vec![
            ("age", Value::Hash(hash(vec![("gte", Value::Int(18))]))),
            ("active", Value::Bool(true)),
        ]);
        let (_pred, sdbql, binds) =
            parse_hash_filter(&filter, "where").expect("a literal filter must still compile");
        assert!(sdbql.contains(">="), "{sdbql}");
        assert_eq!(binds.len(), 2, "{binds:?}");
    }

    /// Scalars taken straight from the request are the normal, intended case
    /// and must stay allowed — only containers can change the operator.
    #[test]
    fn request_supplied_scalars_are_still_allowed() {
        crate::interpreter::taint::clear_request_values();

        let email = Value::String("admin@x.com".into());
        let params = Value::Hash(hash(vec![("email", email.clone())]));
        crate::interpreter::taint::mark_request_value(&params);

        let filter = hash(vec![("email", email)]);
        let (_pred, sdbql, _binds) =
            parse_hash_filter(&filter, "where").expect("a scalar param is a normal filter");
        assert!(sdbql.contains("=="), "{sdbql}");
    }
}

#[cfg(test)]
mod reserved_document_key_tests {
    use super::*;

    /// Without `attr_accessible`, `Model.create(req["json"])` used to persist
    /// whatever the client sent — including `_key`, which the server honours,
    /// so a client could choose (or squat, or collide with) document ids, and
    /// `_from`/`_to`, which are edge endpoints.
    #[test]
    fn underscore_keys_never_reach_the_document() {
        let mut doc = serde_json::json!({
            "title": "hello",
            "_key": "attacker-chosen",
            "_id": "other/1",
            "_rev": "99",
            "_from": "users/1",
            "_to": "admins/1",
        });
        strip_reserved_document_keys(&mut doc, "Post");

        let map = doc.as_object().expect("object");
        assert_eq!(map.len(), 1, "only real attributes should remain: {map:?}");
        assert_eq!(map["title"], serde_json::json!("hello"));
    }

    /// `type` is only reserved where it is an STI discriminator; an ordinary
    /// model is free to have a `type` column.
    #[test]
    fn type_survives_on_a_model_that_is_not_part_of_an_sti_hierarchy() {
        let mut doc = serde_json::json!({"type": "invoice", "total": 10});
        strip_reserved_document_keys(&mut doc, "Document");
        assert_eq!(doc["type"], serde_json::json!("invoice"));
    }

    /// A partial update must validate the record as it will be after the write,
    /// not just the changed fields — otherwise every `presence` rule on an
    /// untouched field would fail.
    #[test]
    fn merging_a_patch_keeps_untouched_stored_fields() {
        let stored = serde_json::json!({"title": "old", "author": "ada", "views": 3});
        let patch = serde_json::json!({"title": "new"});
        let merged = merge_json_documents(Some(&stored), &patch);

        assert_eq!(merged["title"], serde_json::json!("new"));
        assert_eq!(merged["author"], serde_json::json!("ada"));
        assert_eq!(merged["views"], serde_json::json!(3));
    }

    #[test]
    fn merging_without_a_stored_document_validates_the_patch_alone() {
        let patch = serde_json::json!({"title": "new"});
        let merged = merge_json_documents(None, &patch);
        assert_eq!(merged, patch);
    }
}

#[cfg(test)]
mod field_name_length_tests {
    use super::*;

    #[test]
    fn an_overlong_field_name_is_rejected() {
        let longest = "a".repeat(MAX_FIELD_NAME_LEN);
        assert!(validate_field_name(&longest, "order").is_ok());
        let too_long = "a".repeat(MAX_FIELD_NAME_LEN + 1);
        assert!(validate_field_name(&too_long, "order").is_err());
    }

    /// `order` keeps its field as an owned string: a sort column taken from a
    /// request must not be interned into the process-wide symbol table.
    #[test]
    fn ordering_does_not_intern_the_field() {
        let field = format!("never_interned_sort_{}", uuid::Uuid::new_v4().simple());
        let mut qb = crate::interpreter::builtins::model::query::QueryBuilder::new(
            "Post".to_string(),
            "posts".to_string(),
        );
        qb.set_order(field.clone(), "desc".to_string());
        assert!(crate::interpreter::symbol::lookup_symbol(&field).is_none());
        assert_eq!(qb.order_by, Some((field, "desc".to_string())));
    }
}
