//! Model-layer glue for column-aware models.
//!
//! A model that declared `table "orders"` reads and writes real columns of an
//! existing table. Every dispatch point in `crud`/`query`/`core` asks
//! [`schema_for_collection`] first: `Some(schema)` routes to
//! [`crate::db::columns`], `None` falls through to the untouched document path.

use std::sync::Arc;

use crate::db::introspect::{ColType, TableSchema};
use crate::db::sql_columns_compile::{resolve_col, ColumnQuery};
use crate::interpreter::value::Value;

/// Schema for `collection` when its model is column-mode, else `None`.
///
/// Introspection runs under the collection's connection, so a model bound to a
/// named connection is inspected on the right database.
pub fn schema_for_collection(collection: &str) -> Option<Arc<TableSchema>> {
    let table = super::registry::get_table_mapping(collection)?;
    match super::registry::run_on_collection_connection(collection, || {
        crate::db::introspect::get_schema(&table)
    }) {
        Ok(schema) => Some(schema),
        Err(e) => {
            // A column-mode model whose table can't be inspected must not
            // silently fall back to document storage — that would read and
            // write a `_key`/`doc` table nobody asked for. Surface it and stay
            // in column mode; the operation itself then reports the error.
            eprintln!("[column-mode] {collection}: {e}");
            None
        }
    }
}

/// Like [`schema_for_collection`] but propagates the introspection error, for
/// callers that have somewhere to report it.
pub fn require_schema(collection: &str) -> Result<Option<Arc<TableSchema>>, String> {
    let Some(table) = super::registry::get_table_mapping(collection) else {
        return Ok(None);
    };
    super::registry::run_on_collection_connection(collection, || {
        crate::db::introspect::get_schema(&table)
    })
    .map(Some)
}

/// Whether `collection` belongs to a column-aware model.
pub fn is_column_mode(collection: &str) -> bool {
    super::registry::get_table_mapping(collection).is_some()
}

/// The error a SoliDB/document-only feature raises on a column-aware model.
pub fn unsupported(feature: &str, collection: &str) -> String {
    format!(
        "{feature} is not supported on column-aware models (collection {collection:?} \
         is mapped to a real table with `table \"…\"`). Supported: find, find_by, \
         hash `.where`, order/limit/offset, count/exists, sum/avg/min/max, \
         create/save/update/delete, and Model.transaction. \
         See docs/multi-database.md."
    )
}

/// Convert a Soli-side key string to the JSON value the primary key expects.
/// `Model.find("7")` on a serial key must compare as a number, not a string.
pub fn pk_value(schema: &TableSchema, key: &str) -> Result<serde_json::Value, String> {
    match schema.pk_type {
        ColType::Int => key
            .parse::<i64>()
            .map(|n| serde_json::json!(n))
            .map_err(|_| {
                format!(
                    "{:?} has an integer primary key ({}); {key:?} is not a number",
                    schema.table, schema.pk
                )
            }),
        ColType::Float | ColType::Decimal => key
            .parse::<f64>()
            .map(|f| serde_json::json!(f))
            .map_err(|_| {
                format!(
                    "{:?} has a numeric primary key ({}); {key:?} is not a number",
                    schema.table, schema.pk
                )
            }),
        _ => Ok(serde_json::json!(key)),
    }
}

/// Fields that name row identity rather than a column to write. They are
/// dropped from inserts/updates instead of erroring, because the model layer
/// adds them itself.
const RESERVED_FIELDS: [&str; 4] = ["_key", "_id", "_rev", "_collection"];

/// Keep only real columns from a document being written, and stamp
/// created_at/updated_at when the table has them.
pub fn prepare_write(
    schema: &TableSchema,
    doc: &serde_json::Value,
    inserting: bool,
) -> Result<serde_json::Value, String> {
    let obj = doc
        .as_object()
        .ok_or_else(|| "column-aware write expects a hash of fields".to_string())?;

    let mut out = serde_json::Map::new();
    for (field, value) in obj {
        if RESERVED_FIELDS.contains(&field.as_str()) {
            continue;
        }
        // A field that is not a column is a typo worth reporting — silently
        // dropping it would look like a successful write that lost data.
        let col = resolve_col(schema, field)?;
        // Skip columns Soli cannot write rather than failing the whole row when
        // the value is absent anyway.
        if col.ty == ColType::Unknown && value.is_null() {
            continue;
        }
        out.insert(col.name.clone(), value.clone());
    }

    let mut prepared = serde_json::Value::Object(out);
    crate::db::columns::apply_timestamps(schema, &mut prepared, inserting);
    Ok(prepared)
}

/// Build a [`ColumnQuery`] from a QueryBuilder, rejecting anything the column
/// path does not implement.
pub fn column_query_from_qb(
    qb: &super::query::QueryBuilder,
    schema: Arc<TableSchema>,
    collection: &str,
) -> Result<ColumnQuery, String> {
    // Everything below is either SoliDB-specific or a slice-2 item; each must
    // say so rather than quietly returning wrong rows.
    if !qb.includes.is_empty() || !qb.includes_counts.is_empty() {
        return Err(unsupported("`.includes` eager loading", collection));
    }
    if !qb.joins.is_empty() {
        return Err(unsupported("`.join`", collection));
    }
    if qb.traversal.is_some() || qb.through.is_some() {
        return Err(unsupported("graph / through queries", collection));
    }
    if qb.group_by_info.is_some() || !qb.group_fields.is_empty() {
        return Err(unsupported("`group_by`", collection));
    }
    if qb.having.is_some() {
        return Err(unsupported("`.having`", collection));
    }
    if qb.similar_query.is_some() || qb.time_bucket_info.is_some() {
        return Err(unsupported("vector / time-bucket queries", collection));
    }
    if qb.sti_types.is_some() {
        return Err(unsupported("single-table inheritance", collection));
    }
    if !matches!(qb.soft_delete_mode, super::query::SoftDeleteMode::Default)
        || qb.is_soft_delete_model
    {
        return Err(unsupported("soft delete", collection));
    }

    let mut q = ColumnQuery::new(schema);
    for (key, value) in &qb.bind_vars {
        let field = crate::interpreter::symbol_string(*key)
            .unwrap_or("")
            .to_string();
        if field.starts_with("__soli_") {
            continue;
        }
        q.eq_filters.insert(field, value.clone());
    }
    // A raw filter carries binds too (`.where("doc.age >= @age", {age: 18})`),
    // so its presence alone cannot distinguish it from the hash form. Validate
    // that the filter text really is the hash-equality shape matching these
    // binds — otherwise `age >= 18` would silently become `age = 18`.
    if crate::db::sql_compile::assert_portable_filter(qb.filter.as_deref(), &q.eq_filters).is_err()
    {
        return Err(unsupported("raw/string `.where(\"…\")`", collection));
    }
    if let Some((field, dir)) = &qb.order_by {
        let field = crate::interpreter::symbol_string(*field)
            .unwrap_or("unknown")
            .to_string();
        let dir = crate::interpreter::symbol_string(*dir)
            .unwrap_or("asc")
            .to_lowercase();
        q.order_desc = matches!(dir.as_str(), "desc" | "descending");
        q.order_field = Some(field);
    }
    q.limit = qb.limit_val;
    q.offset = qb.offset_val;
    Ok(q)
}

/// Hydrate column rows into instances (or plain hashes), converting temporal
/// columns to native DateTime values.
pub fn hydrate_column_rows(
    qb: &super::query::QueryBuilder,
    schema: &TableSchema,
    rows: Vec<serde_json::Value>,
) -> Value {
    let values: Vec<Value> = match qb.hydration_class() {
        Some(class) => rows
            .into_iter()
            .map(|row| {
                let instance = super::crud::json_doc_to_instance_owned(class, row);
                convert_temporal_fields(schema, instance)
            })
            .collect(),
        None => rows
            .into_iter()
            .map(|row| {
                let value = super::crud::json_to_value_owned(row);
                convert_temporal_fields(schema, value)
            })
            .collect(),
    };
    Value::Array(std::rc::Rc::new(std::cell::RefCell::new(values)))
}

/// Replace date/timestamp strings with native DateTime values, so `.year`,
/// comparisons, and formatting behave as they do on document models.
pub fn convert_temporal_fields(schema: &TableSchema, value: Value) -> Value {
    let temporal: Vec<&str> = schema
        .columns
        .iter()
        .filter(|c| matches!(c.ty, ColType::Date | ColType::DateTime))
        .map(|c| c.name.as_str())
        .collect();
    if temporal.is_empty() {
        return value;
    }
    match &value {
        Value::Instance(instance) => {
            let mut fields = instance.borrow_mut();
            for name in temporal {
                let current = fields.fields.get(name).cloned();
                if let Some(Value::String(s)) = current {
                    if let Some(dt) = parse_datetime(&s) {
                        fields.fields.insert(name.into(), dt);
                    }
                }
            }
            drop(fields);
            value
        }
        Value::Hash(hash) => {
            let mut pairs = hash.borrow_mut();
            for name in temporal {
                let key = crate::interpreter::value::HashKey::String(name.into());
                let current = pairs.get(&key).cloned();
                if let Some(Value::String(s)) = current {
                    if let Some(dt) = parse_datetime(&s) {
                        pairs.insert(key, dt);
                    }
                }
            }
            drop(pairs);
            value
        }
        _ => value,
    }
}

/// Parse one of our normalized RFC-3339 strings into a native DateTime value.
/// `Value::DateTime` is a Unix timestamp, so this reuses the same parser the
/// `DateTime` builtins use.
fn parse_datetime(raw: &str) -> Option<Value> {
    crate::interpreter::builtins::datetime::helpers::datetime_parse(raw).map(Value::DateTime)
}

/// Introspect every declared column-aware model and validate the combination of
/// class-body declarations, so a misconfiguration fails at boot with a clear
/// message rather than on the first request.
///
/// Returns one line per problem; an empty vec means everything checked out.
/// Callers decide whether a problem is fatal (serve) or a warning (CLI).
pub fn validate_declared_models() -> Vec<String> {
    let mut problems = Vec::new();
    for model in super::registry::all_column_models() {
        // Declarations that assume Soli-managed document storage cannot work
        // against a schema Soli does not own.
        for (decl, present) in [
            ("soft_delete", model.soft_delete),
            ("encrypts", model.has_encrypted_fields),
            (
                "edge / timeseries / columnar",
                model.collection_type.is_some(),
            ),
        ] {
            if present {
                problems.push(format!(
                    "{} maps to table {:?} (column mode) but also declares `{decl}`, \
                     which needs Soli-managed document storage. Remove one of the two.",
                    model.class_name, model.table
                ));
            }
        }

        // Introspect now, so a missing table, a composite primary key, or a
        // non-SQL connection is reported at boot instead of mid-request.
        if let Err(e) = super::registry::run_on_collection_connection(&model.collection, || {
            crate::db::introspect::get_schema(&model.table)
        }) {
            problems.push(e);
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::introspect::{build_schema, pg_coltype, RawColumns};

    fn schema() -> Arc<TableSchema> {
        let raw = RawColumns {
            columns: vec![
                ("id".into(), "int8".into(), String::new(), false, true),
                ("name".into(), "text".into(), String::new(), false, false),
                (
                    "created_at".into(),
                    "timestamptz".into(),
                    String::new(),
                    true,
                    false,
                ),
                ("blob".into(), "bytea".into(), String::new(), true, false),
            ],
            pk: vec!["id".into()],
        };
        Arc::new(build_schema("legacy", "orders", raw, |t, _| pg_coltype(t)).unwrap())
    }

    #[test]
    fn pk_values_are_coerced_to_the_key_type() {
        let s = schema();
        // A string id from a URL must compare as a number against a serial key.
        assert_eq!(pk_value(&s, "7").unwrap(), serde_json::json!(7));
        let err = pk_value(&s, "abc").expect_err("non-numeric id must error");
        assert!(err.contains("integer primary key"), "{err}");
        assert!(err.contains("orders"), "{err}");
    }

    #[test]
    fn text_primary_keys_pass_through() {
        let raw = RawColumns {
            columns: vec![("slug".into(), "text".into(), String::new(), false, false)],
            pk: vec!["slug".into()],
        };
        let s = Arc::new(build_schema("legacy", "pages", raw, |t, _| pg_coltype(t)).unwrap());
        assert_eq!(pk_value(&s, "about").unwrap(), serde_json::json!("about"));
    }

    #[test]
    fn writes_drop_reserved_fields_and_stamp_timestamps() {
        let s = schema();
        let doc = serde_json::json!({ "_key": "7", "_rev": "abc", "name": "Ada" });
        let prepared = prepare_write(&s, &doc, true).unwrap();
        assert!(
            prepared.get("_key").is_none(),
            "identity fields are not columns"
        );
        assert!(prepared.get("_rev").is_none());
        assert_eq!(prepared["name"], "Ada");
        assert!(prepared["created_at"].is_string(), "table has created_at");
    }

    #[test]
    fn writing_an_unknown_field_is_an_error_not_a_silent_drop() {
        // Silently dropping would look like a successful write that lost data.
        let s = schema();
        let doc = serde_json::json!({ "name": "Ada", "nickname": "A" });
        let err = prepare_write(&s, &doc, true).expect_err("must error");
        assert!(err.contains("unknown field"), "{err}");
        assert!(err.contains("nickname"), "{err}");
    }

    #[test]
    fn unsupported_message_names_the_feature_and_lists_what_works() {
        let msg = unsupported("`group_by`", "orders");
        assert!(msg.contains("`group_by`"), "{msg}");
        assert!(msg.contains("orders"), "{msg}");
        assert!(msg.contains("Supported:"), "{msg}");
        assert!(msg.contains("Model.transaction"), "{msg}");
    }

    #[test]
    fn integer_and_string_ids_both_reach_the_key_type() {
        // `Model.find` normalizes an Int id to a string before it reaches the
        // column layer (a serial key is naturally called with a number), so the
        // conversion back to the key's real type must accept that form.
        let s = schema();
        assert_eq!(
            pk_value(&s, &42.to_string()).unwrap(),
            serde_json::json!(42)
        );
        assert_eq!(pk_value(&s, "42").unwrap(), serde_json::json!(42));
    }

    /// A raw filter carries binds exactly like the hash form, so presence of
    /// binds cannot tell them apart. `.where("doc.age >= @age", {age: 18})`
    /// must be REFUSED, never silently compiled to `age = 18` — that would
    /// return the wrong rows with no error.
    #[test]
    fn raw_comparison_filters_are_refused_not_rewritten_as_equality() {
        use crate::db::sql_compile::assert_portable_filter;
        use std::collections::BTreeMap;

        let mut eq: BTreeMap<String, serde_json::Value> = BTreeMap::new();
        eq.insert("age".to_string(), serde_json::json!(18));

        // The hash form's generated filter is accepted…
        assert!(assert_portable_filter(Some("doc.age == @age"), &eq).is_ok());
        // …but a comparison, or any other raw SDBQL, is not.
        for raw in [
            "doc.age >= @age",
            "doc.age != @age",
            "doc.age == @age OR doc.age == @other",
            "LENGTH(doc.age) == @age",
        ] {
            assert!(
                assert_portable_filter(Some(raw), &eq).is_err(),
                "{raw:?} must not be treated as a hash-equality filter"
            );
        }
    }
}
