//! Framework-owned collections in the app's own database: `_soli_errors`,
//! `_soli_slow_queries`.
//!
//! The error tracker grew these calls first, each one written twice — once for
//! SoliDB, once for the SQL document adapters. The slow-query tracker needs the
//! same six, so they live here, parameterised by collection, and each tracker
//! keeps only what is its own: what a document holds and how it is merged.
//!
//! Collection and field names passed in are framework constants, never user
//! input; the SoliDB list query inlines them.

use crate::db;
use crate::interpreter::builtins::model::crud;

/// Create the collection (and one index) if it is not there yet.
pub(crate) fn ensure(collection: &str, index_field: &str) -> Result<(), String> {
    if db::is_sql() {
        db::sql::ensure_table(collection).and_then(|_| {
            db::sql::ensure_doc_index(
                collection,
                &[index_field.to_string()],
                &format!("idx_{collection}_{index_field}"),
                false,
            )
            .map(|_| ())
        })
    } else {
        crud::ensure_collection(collection)
            .and_then(|_| crud::ensure_index(collection, index_field))
    }
}

/// One document, or `None` when the key is unknown.
pub(crate) fn get(collection: &str, key: &str) -> Result<Option<serde_json::Value>, String> {
    if db::is_sql() {
        return db::sql::get(collection, key);
    }
    // A missing document is a normal answer here; anything else (a timeout,
    // a refused connection) is an error. Reading every failure as "missing"
    // made the writer insert over a group it merely could not read.
    match crud::exec_get(collection, key) {
        Ok(doc) => Ok(Some(doc)),
        Err(e) if is_missing_document(&e) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Does this SoliDB error say the document (or its collection) is not there?
pub(crate) fn is_missing_document(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("404")
        || lower.contains("not found")
        || lower.contains("notfound")
        || lower.contains("does not exist")
}

/// Is this read error "the collection was never created"? Pages show that as
/// an empty state, not as a failure.
pub(crate) fn is_missing_collection(error: &str) -> bool {
    error.contains("404")
        || error.to_lowercase().contains("not found")
        || error.contains("no such table")
}

pub(crate) fn insert(collection: &str, key: &str, doc: serde_json::Value) -> Result<(), String> {
    if db::is_sql() {
        db::sql::insert(collection, Some(key), doc)?;
    } else {
        crud::exec_insert(collection, Some(key), doc)?;
    }
    Ok(())
}

/// Merge `fields` into the document.
pub(crate) fn patch(collection: &str, key: &str, fields: serde_json::Value) -> Result<(), String> {
    if db::is_sql() {
        db::sql::update(collection, key, fields, true)?;
    } else {
        crud::exec_update(collection, key, fields, true)?;
    }
    Ok(())
}

/// Forget a document. `false` when it did not exist.
pub(crate) fn delete(collection: &str, key: &str) -> Result<bool, String> {
    if get(collection, key)?.is_none() {
        return Ok(false);
    }
    if db::is_sql() {
        db::sql::delete(collection, key)?;
    } else {
        crud::exec_delete(collection, key)?;
    }
    Ok(true)
}

/// Documents stored in `collection`.
pub(crate) fn count(collection: &str) -> Result<u64, String> {
    if db::is_sql() {
        return db::sql::count(&list_query(collection, None, None, false, None))
            .map(|n| n.max(0) as u64);
    }
    let rows = crud::exec_query(
        collection,
        format!("RETURN COLLECTION_COUNT(\"{collection}\")"),
    )?;
    match crate::interpreter::builtins::model::core::parse_count_result(&rows) {
        crate::interpreter::value::Value::Int(n) => Ok(n.max(0) as u64),
        other => Err(format!("unexpected count result: {other}")),
    }
}

/// Up to `limit` documents, `order_field` first-to-last (`desc` reverses),
/// optionally only those whose `field == value`. Only `fields` (and `_key`) are
/// returned, so a list page does not read every stored sample.
pub(crate) fn list(
    collection: &str,
    filter: Option<(&str, &str)>,
    order_field: &str,
    desc: bool,
    limit: usize,
    fields: &[&str],
) -> Result<Vec<serde_json::Value>, String> {
    let mut rows = if db::is_sql() {
        db::sql::select(&list_query(
            collection,
            filter,
            Some(order_field),
            desc,
            Some(limit),
        ))?
    } else {
        let filter_clause = match filter {
            Some((field, value)) => {
                format!("FILTER doc.{field} == {} ", serde_json::Value::from(value))
            }
            None => String::new(),
        };
        let projection: Vec<String> = std::iter::once("_key".to_string())
            .chain(fields.iter().map(|f| f.to_string()))
            .map(|f| format!("{f}: doc.{f}"))
            .collect();
        let sdbql = format!(
            "FOR doc IN {collection} {filter_clause}SORT doc.{order_field} {dir} LIMIT {limit} \
             RETURN {{{}}}",
            projection.join(", "),
            dir = if desc { "DESC" } else { "ASC" },
        );
        crud::exec_query(collection, sdbql)?
    };
    // The SQL adapters return whole documents; keep the list rows the same shape.
    for row in &mut rows {
        if let Some(map) = row.as_object_mut() {
            map.retain(|k, _| k == "_key" || fields.contains(&k.as_str()));
        }
    }
    Ok(rows)
}

fn list_query(
    collection: &str,
    filter: Option<(&str, &str)>,
    order_field: Option<&str>,
    desc: bool,
    limit: Option<usize>,
) -> db::ListQuery {
    let mut eq_filters = std::collections::BTreeMap::new();
    let mut filter_sdbql = None;
    if let Some((field, value)) = filter {
        eq_filters.insert(field.to_string(), serde_json::Value::from(value));
        filter_sdbql = Some(format!("doc.{field} == @{field}"));
    }
    db::ListQuery {
        table: collection.to_string(),
        eq_filters,
        hash_filter: None,
        filter_sdbql,
        having: None,
        exists_filters: Vec::new(),
        soft_delete: db::SqlSoftDeleteMode::WithDeleted,
        is_soft_delete_model: false,
        order_field: order_field.map(str::to_string),
        order_desc: desc,
        limit,
        offset: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_missing_document_reads_as_absent() {
        assert!(is_missing_document("HTTP 404 Not Found http://db/x: {}"));
        assert!(is_missing_document("driver get failed: document not found"));
        assert!(!is_missing_document("HTTP error: connection refused"));
        assert!(!is_missing_document(
            "HTTP 503 Service Unavailable http://db/x: busy"
        ));
    }
}
