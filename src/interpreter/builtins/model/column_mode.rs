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

/// A Soli value as a positional SQL bind, for `Model.find_by_sql`.
///
/// There is no column to consult here, so the value's own type decides — which is
/// also why a hash or array is refused rather than guessed at.
pub fn bind_from_value(value: &Value) -> Result<crate::db::sql_compile::SqlBind, String> {
    use crate::db::sql_compile::SqlBind;
    Ok(match value {
        Value::String(s) => SqlBind::Text(s.to_string()),
        Value::Int(n) => SqlBind::I64(*n),
        Value::Float(f) => SqlBind::F64(*f),
        Value::Bool(b) => SqlBind::Bool(*b),
        Value::Decimal(d) => SqlBind::Text(d.to_string()),
        Value::DateTime(_) => SqlBind::Text(
            crate::interpreter::value::value_to_json(value)
                .ok()
                .and_then(|j| j.as_str().map(str::to_string))
                .unwrap_or_default(),
        ),
        Value::Null => SqlBind::Text(String::new()),
        other => {
            return Err(format!(
                "a SQL bind must be a string, number, boolean, or datetime — got {}. \
                 Pass a hash or array as JSON text if the column is JSON.",
                other.type_name()
            ))
        }
    })
}

/// Does this table have the `deleted_at` column soft delete needs?
fn q_schema_has_deleted_at(schema: &TableSchema) -> bool {
    schema.has_column("deleted_at")
}

/// Was this failure "that column does not exist"?
///
/// `resolve_col` produces exactly this shape, so the test is on our own message
/// rather than on driver prose.
pub fn is_unknown_field_error(err: &str) -> bool {
    err.starts_with("unknown field ") && err.contains(" on table ")
}

/// Retry `f` once against a freshly introspected schema when it failed because a
/// column was unknown.
///
/// An `ALTER TABLE` applied while the server runs otherwise stays invisible: the
/// cached schema is authoritative and would keep rejecting the new column until
/// restart.
pub fn retry_after_alter<T>(
    collection: &str,
    mut f: impl FnMut() -> Result<T, String>,
) -> Result<T, String> {
    match f() {
        Err(e) if is_unknown_field_error(&e) => {
            let Some(table) = super::registry::get_table_mapping(collection) else {
                return Err(e);
            };
            super::registry::run_on_collection_connection(collection, || {
                crate::db::introspect::invalidate_schema(&table);
            });
            // One retry only: if the column is still unknown it really is a typo,
            // and the original message is the useful one.
            f().map_err(|_| e)
        }
        other => other,
    }
}

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
    // `.includes` is handled after the rows come back (see
    // `include_relations`), so it is not a reason to reject the query here.
    if !qb.joins.is_empty() {
        return Err(unsupported("`.join`", collection));
    }
    if qb.traversal.is_some() || qb.through.is_some() {
        return Err(unsupported("graph / through queries", collection));
    }
    // `group_by` compiles over real columns (see `compile_group_by_cols`); the
    // grouping itself is applied by the caller, not by this filter builder.
    if qb.having.is_some() {
        return Err(unsupported("`.having`", collection));
    }
    if qb.similar_query.is_some() || qb.time_bucket_info.is_some() {
        return Err(unsupported("vector / time-bucket queries", collection));
    }
    if qb.sti_types.is_some() {
        return Err(unsupported("single-table inheritance", collection));
    }
    // Soft delete needs a real `deleted_at` column. When the schema has one, the
    // scope is a normal filter; when it does not, there is nowhere to record the
    // deletion, so say that rather than silently returning every row.
    if qb.is_soft_delete_model && !q_schema_has_deleted_at(&schema) {
        return Err(format!(
            "model for table {collection:?} declares `soft_delete`, but the table has no \
             `deleted_at` column. Add one (a nullable timestamp), or drop the declaration \
             — column mode never alters your schema."
        ));
    }

    let mut q = ColumnQuery::new(schema.clone());
    // The soft-delete scope, expressed as an ordinary filter on the column.
    if qb.is_soft_delete_model {
        match qb.soft_delete_mode {
            super::query::SoftDeleteMode::Default => {
                q.eq_filters
                    .insert("deleted_at".to_string(), serde_json::Value::Null);
            }
            super::query::SoftDeleteMode::OnlyDeleted => {
                q.not_null_filters.push("deleted_at".to_string());
            }
            super::query::SoftDeleteMode::WithDeleted => {}
        }
    }
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
    // Push `pluck` / `select` into the SELECT list when every field is a real
    // column. A field that is not (a nested path, a computed alias) falls back to
    // the client-side projection rather than failing the query.
    if let Some(fields) = qb.pluck_fields.as_ref().or(qb.select_fields.as_ref()) {
        if fields
            .iter()
            .all(|field| resolve_col(&schema, field).is_ok())
        {
            q.select_fields = Some(fields.clone());
        }
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

/// Adopt a stored row onto an instance after a column-mode write.
///
/// A SQL write returns the row as the database now holds it, which is the only
/// way a generated key, a column `DEFAULT`, or a database-side trigger becomes
/// visible without a second query. The document path has no equivalent problem:
/// there the document written *is* the document stored.
///
/// Framework metadata (`_key`, `_rev`, …) is left to the caller, which handles
/// it identically for both paths. Returns true when the row was adopted.
pub fn adopt_row_fields(
    inst: &mut crate::interpreter::value::Instance,
    row: &serde_json::Value,
) -> bool {
    let Some(map) = row.as_object() else {
        return false;
    };
    for (key, value) in map {
        if key.starts_with('_') {
            continue;
        }
        inst.set(key.clone(), super::crud::json_to_value_owned(value.clone()));
    }
    true
}

/// Eager-load declared associations onto column rows.
///
/// One batched query per association, whatever the parent count — the same
/// guarantee the document path gives, using `column IN (…)` over the real
/// foreign-key columns instead of a JSON extract.
///
/// Both sides must be column-mode: eager loading between a column table and a
/// document collection would have to join a real column to a JSON field, so it
/// is refused with a message naming both models rather than silently returning
/// nothing.
pub fn include_relations(
    qb: &super::query::QueryBuilder,
    schema: &TableSchema,
    rows: &mut [serde_json::Value],
) -> Result<(), String> {
    for inc in &qb.includes {
        if inc.filter.is_some() {
            return Err(unsupported(
                "a filtered `.includes(…, where: …)`",
                &schema.table,
            ));
        }
        let related = require_related_schema(&inc.relation.collection, &inc.relation_name)?;
        match inc.relation.relation_type {
            super::relations::RelationType::BelongsTo
            | super::relations::RelationType::Polymorphic => {
                include_belongs_to(rows, inc, schema, &related)?
            }
            super::relations::RelationType::HasMany | super::relations::RelationType::HasOne => {
                include_has_many(rows, inc, schema, &related)?
            }
            super::relations::RelationType::HasAndBelongsToMany => {
                return Err(unsupported(
                    "`.includes` on has_and_belongs_to_many",
                    &schema.table,
                ))
            }
        }
    }
    for inc in &qb.includes_counts {
        let related = require_related_schema(&inc.relation.collection, &inc.relation_name)?;
        include_count(rows, inc, schema, &related)?;
    }
    Ok(())
}

/// The related model's schema, or an error naming why it cannot be joined.
fn require_related_schema(
    collection: &str,
    relation_name: &str,
) -> Result<Arc<TableSchema>, String> {
    schema_for_collection(collection).ok_or_else(|| {
        format!(
            "`.includes(\"{relation_name}\")`: {collection:?} is not a column-aware model. \
             Eager loading between a column table and a document collection would have to \
             match a real column against a JSON field — give both models a `table \"…\"`, \
             or load the two sides separately."
        )
    })
}

/// Value of `column` on a row, as the text used to key the lookup map.
fn row_text(row: &serde_json::Value, column: &str) -> Option<String> {
    match row.get(column) {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(s)) => Some(s.clone()),
        Some(other) => Some(other.to_string()),
    }
}

/// Distinct non-null values of `column` across `rows`, in first-seen order.
fn distinct_values(rows: &[serde_json::Value], column: &str) -> Vec<serde_json::Value> {
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for row in rows {
        let Some(text) = row_text(row, column) else {
            continue;
        };
        if seen.iter().any(|s| s == &text) {
            continue;
        }
        seen.push(text);
        out.push(row.get(column).cloned().unwrap_or(serde_json::Value::Null));
    }
    out
}

/// Rows of `schema` where `column` is one of `values` — one query.
fn fetch_in(
    schema: &Arc<TableSchema>,
    column: &str,
    values: Vec<serde_json::Value>,
) -> Result<Vec<serde_json::Value>, String> {
    if values.is_empty() {
        return Ok(Vec::new());
    }
    let mut query = ColumnQuery::new(schema.clone());
    query.in_filters.insert(column.to_string(), values);
    crate::db::columns::select_rows(&query)
}

fn project(row: serde_json::Value, fields: &Option<Vec<String>>) -> serde_json::Value {
    let Some(fields) = fields else {
        return row;
    };
    let Some(obj) = row.as_object() else {
        return row;
    };
    let mut out = serde_json::Map::new();
    for field in fields {
        if let Some(value) = obj.get(field) {
            out.insert(field.clone(), value.clone());
        }
    }
    // The key stays available so the caller can still identify the record.
    if let Some(key) = obj.get("_key") {
        out.insert("_key".to_string(), key.clone());
    }
    serde_json::Value::Object(out)
}

/// `belongs_to`: the parent holds the FK, the target is found by primary key.
fn include_belongs_to(
    rows: &mut [serde_json::Value],
    inc: &super::query::IncludeClause,
    schema: &TableSchema,
    related: &Arc<TableSchema>,
) -> Result<(), String> {
    let fk = resolve_col(schema, &inc.relation.foreign_key)?.name.clone();
    let keys = distinct_values(rows, &fk);
    let fetched = fetch_in(related, &related.pk.clone(), keys)?;

    let mut by_key: std::collections::HashMap<String, serde_json::Value> =
        std::collections::HashMap::new();
    for row in fetched {
        if let Some(key) = row_text(&row, &related.pk) {
            by_key.insert(key, project(row, &inc.fields));
        }
    }
    for row in rows.iter_mut() {
        let attached = row_text(row, &fk)
            .and_then(|key| by_key.get(&key).cloned())
            .unwrap_or(serde_json::Value::Null);
        if let Some(obj) = row.as_object_mut() {
            obj.insert(inc.relation_name.clone(), attached);
        }
    }
    Ok(())
}

/// `has_many` / `has_one`: the child holds the FK pointing at the parent key.
fn include_has_many(
    rows: &mut [serde_json::Value],
    inc: &super::query::IncludeClause,
    schema: &TableSchema,
    related: &Arc<TableSchema>,
) -> Result<(), String> {
    let fk = resolve_col(related, &inc.relation.foreign_key)?
        .name
        .clone();
    let parent_keys = distinct_values(rows, &schema.pk);
    let fetched = fetch_in(related, &fk, parent_keys)?;

    let mut by_fk: std::collections::HashMap<String, Vec<serde_json::Value>> =
        std::collections::HashMap::new();
    for row in fetched {
        if let Some(key) = row_text(&row, &fk) {
            by_fk
                .entry(key)
                .or_default()
                .push(project(row, &inc.fields));
        }
    }
    let singular = matches!(
        inc.relation.relation_type,
        super::relations::RelationType::HasOne
    );
    for row in rows.iter_mut() {
        let children = row_text(row, &schema.pk)
            .and_then(|key| by_fk.remove(&key))
            .unwrap_or_default();
        // A parent with no children gets `[]`, never null — callers iterate it.
        let attached = if singular {
            children
                .into_iter()
                .next()
                .unwrap_or(serde_json::Value::Null)
        } else {
            serde_json::Value::Array(children)
        };
        if let Some(obj) = row.as_object_mut() {
            obj.insert(inc.relation_name.clone(), attached);
        }
    }
    Ok(())
}

/// `includes_count`: the number of children per parent, in one query.
fn include_count(
    rows: &mut [serde_json::Value],
    inc: &super::query::IncludeCountClause,
    schema: &TableSchema,
    related: &Arc<TableSchema>,
) -> Result<(), String> {
    let fk = resolve_col(related, &inc.relation.foreign_key)?
        .name
        .clone();
    let parent_keys = distinct_values(rows, &schema.pk);
    let fetched = fetch_in(related, &fk, parent_keys)?;

    let mut counts: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    for row in fetched {
        if let Some(key) = row_text(&row, &fk) {
            *counts.entry(key).or_insert(0) += 1;
        }
    }
    for row in rows.iter_mut() {
        let count = row_text(row, &schema.pk)
            .and_then(|key| counts.get(&key).copied())
            .unwrap_or(0);
        if let Some(obj) = row.as_object_mut() {
            obj.insert(inc.alias.clone(), serde_json::json!(count));
        }
    }
    Ok(())
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
    // `datetime_parse` answers in SECONDS; `Value::DateTime` is NANOSECONDS
    // everywhere else in the runtime (see `Value::datetime_to_rfc3339`). Storing
    // the seconds directly made every column-mode timestamp read as 1970 — and
    // writing such a value back rewrote the row with that wrong date.
    crate::interpreter::builtins::datetime::helpers::datetime_parse(raw)
        .and_then(|secs| secs.checked_mul(1_000_000_000))
        .map(Value::DateTime)
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
        // `soft_delete` is allowed now, provided the table actually has a
        // `deleted_at` column — checked below, once the schema is known.
        // Declarations that assume Soli-managed document storage still cannot
        // work against a schema Soli does not own.
        for (decl, present) in [
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
        match super::registry::run_on_collection_connection(&model.collection, || {
            crate::db::introspect::get_schema(&model.table)
        }) {
            Err(e) => problems.push(e),
            Ok(schema) => {
                // Soft delete records the deletion in a column; without it there
                // is nowhere to write, and every scoped read would silently
                // return deleted rows.
                if model.soft_delete && !schema.has_column("deleted_at") {
                    problems.push(format!(
                        "{} declares `soft_delete` and maps to table {:?}, which has no \
                         `deleted_at` column. Add a nullable timestamp column, or drop the \
                         declaration — column mode never alters your schema.",
                        model.class_name, model.table
                    ));
                }
            }
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
