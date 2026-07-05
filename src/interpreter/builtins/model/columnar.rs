//! Columnar-engine support for the Model layer.
//!
//! A columnar model declares its schema in the class body:
//!
//! ```soli
//! class PageView < Model
//!   columnar compression: "lz4"
//!   column "url", "string"
//!   column "visited_at", "timestamp"
//!   column "duration_ms", "int", nullable: true
//!   column "country", "string", indexed: true
//! end
//! ```
//!
//! Columnar stores live behind their own HTTP API (`/_api/database/{db}/
//! columnar/...`), separate from document collections and NOT reachable from
//! SDBQL `FOR` queries. Rows have server-generated UUIDs and there is no
//! row-level get/update/delete — the honest surface is insert_rows /
//! aggregate / query / column indexes, and the document-model statics error
//! with a pointer here.

use std::cell::RefCell;
use std::rc::Rc;

use crate::interpreter::value::{HashKey, Value};

use super::crud::{exec_db_api_request, json_to_value};
use super::registry::ColumnarSchemaDef;

/// Server-recognized column types (`parse_column_type` in the DB). Unknown
/// strings silently default to String server-side, so we reject them here.
pub const COLUMN_TYPES: &[&str] = &[
    "int",
    "integer",
    "int64",
    "bigint",
    "float",
    "float64",
    "double",
    "number",
    "string",
    "text",
    "varchar",
    "bool",
    "boolean",
    "timestamp",
    "datetime",
    "date",
    "json",
    "object",
    "array",
];

/// Column index types (`ColumnarIndexType`).
pub const COLUMN_INDEX_TYPES: &[&str] = &["sorted", "hash", "bitmap", "minmax", "bloom"];

/// Filter operators of the columnar query endpoint.
pub const COLUMN_FILTER_OPS: &[&str] = &["eq", "ne", "gt", "gte", "lt", "lte", "in"];

/// Aggregate operations of the columnar aggregate endpoint.
pub const COLUMN_AGG_OPS: &[&str] = &["count", "sum", "avg", "min", "max", "count_distinct"];

/// Document-API statics that make no sense on a columnar store (no _key, no
/// row-level get/update/delete, not reachable from SDBQL). Checked at the
/// static-binding choke point in `bind_native_static_to_model_class`.
/// `count` and `aggregate` are absent — they branch on the model kind.
pub fn is_document_api_method(name: &str) -> bool {
    matches!(
        name,
        "find"
            | "find_by"
            | "first_by"
            | "find_or_create_by"
            | "create"
            | "create_many"
            | "update"
            | "upsert"
            | "delete"
            | "delete_all"
            | "where"
            | "order"
            | "limit"
            | "offset"
            | "all"
            | "all_json"
            | "first"
            | "paginate"
            | "pluck"
            | "sum"
            | "avg"
            | "min"
            | "max"
            | "median"
            | "stddev"
            | "variance"
            | "count_distinct"
            | "group_by"
            | "exists"
            | "includes"
            | "includes_count"
            | "select"
            | "fields"
            | "join"
            | "scope"
            | "with_deleted"
            | "only_deleted"
            | "time_bucket"
            | "prune"
            | "similar"
            | "search"
            | "near"
            | "within"
    )
}

/// The standard "this is a columnar model" error for document-style methods.
pub fn columnar_no_document_api_error(class_name: &str, method: &str) -> String {
    format!(
        "{}.{}: {} is a columnar model; columnar stores have no document API. \
         Use insert_rows / aggregate / query. See docs/database/analytics.",
        class_name, method, class_name
    )
}

fn is_missing_store_error(err: &str) -> bool {
    let lower = err.to_lowercase();
    lower.contains("not found") || lower.contains("notfound") || lower.contains("does not exist")
}

/// Create the columnar store from the declared schema.
pub fn create_store_from_schema(
    collection: &str,
    schema: &ColumnarSchemaDef,
) -> Result<(), String> {
    let columns: Vec<serde_json::Value> = schema
        .columns
        .iter()
        .map(|c| {
            serde_json::json!({
                "name": c.name,
                "type": c.data_type,
                "nullable": c.nullable,
                "indexed": c.indexed,
            })
        })
        .collect();
    let mut body = serde_json::json!({ "name": collection, "columns": columns });
    if let Some(compression) = &schema.compression {
        body["compression"] = serde_json::Value::String(compression.clone());
    }
    exec_db_api_request(reqwest::Method::POST, "/columnar", Some(body)).map(|_| ())
}

/// Insert rows; auto-creates the store from the declared schema on a
/// missing-store error (dev convenience — migrations are the production DDL
/// path). Returns `{"inserted": n, "ids": [...]}`.
pub fn insert_rows(
    collection: &str,
    schema: &ColumnarSchemaDef,
    rows: serde_json::Value,
) -> Result<Value, String> {
    let path = format!("/columnar/{}/insert", collection);
    let body = serde_json::json!({ "rows": rows });
    let resp = match exec_db_api_request(reqwest::Method::POST, &path, Some(body.clone())) {
        Err(e) if is_missing_store_error(&e) => {
            create_store_from_schema(collection, schema).map_err(|ce| {
                format!("columnar store '{}' auto-create failed: {}", collection, ce)
            })?;
            exec_db_api_request(reqwest::Method::POST, &path, Some(body))?
        }
        other => other?,
    };
    let mut pairs = crate::interpreter::value::HashPairs::default();
    pairs.insert(
        HashKey::String("inserted".into()),
        json_to_value(resp.get("inserted").unwrap_or(&serde_json::Value::from(0))),
    );
    pairs.insert(
        HashKey::String("ids".into()),
        json_to_value(resp.get("ids").unwrap_or(&serde_json::Value::Array(vec![]))),
    );
    Ok(Value::Hash(Rc::new(RefCell::new(pairs))))
}

/// Aggregate. Ungrouped → scalar; grouped → array of row hashes with the
/// server's `_agg` key renamed to `"value"`.
pub fn aggregate(
    collection: &str,
    column: &str,
    operation: &str,
    group_by: Option<Vec<String>>,
) -> Result<Value, String> {
    let mut body = serde_json::json!({
        "column": column,
        "operation": operation.to_uppercase(),
    });
    if let Some(cols) = &group_by {
        body["group_by"] = serde_json::json!(cols);
    }
    let resp = exec_db_api_request(
        reqwest::Method::POST,
        &format!("/columnar/{}/aggregate", collection),
        Some(body),
    )?;

    if let Some(groups) = resp.get("groups").and_then(|g| g.as_array()) {
        let rows: Vec<Value> = groups
            .iter()
            .map(|g| {
                let mut g = g.clone();
                if let Some(obj) = g.as_object_mut() {
                    if let Some(agg) = obj.remove("_agg") {
                        obj.insert("value".to_string(), agg);
                    }
                }
                json_to_value(&g)
            })
            .collect();
        Ok(Value::Array(Rc::new(RefCell::new(rows))))
    } else {
        Ok(json_to_value(
            resp.get("result").unwrap_or(&serde_json::Value::Null),
        ))
    }
}

/// Projection query with the endpoint's single optional filter.
pub fn query(
    collection: &str,
    columns: Vec<String>,
    filter: Option<serde_json::Value>,
    limit: Option<usize>,
) -> Result<Value, String> {
    let mut body = serde_json::json!({ "columns": columns });
    if let Some(f) = filter {
        body["filter"] = f;
    }
    if let Some(l) = limit {
        body["limit"] = serde_json::json!(l);
    }
    let resp = exec_db_api_request(
        reqwest::Method::POST,
        &format!("/columnar/{}/query", collection),
        Some(body),
    )?;
    let rows: Vec<Value> = resp
        .get("result")
        .and_then(|r| r.as_array())
        .map(|arr| arr.iter().map(json_to_value).collect())
        .unwrap_or_default();
    Ok(Value::Array(Rc::new(RefCell::new(rows))))
}

pub fn create_index(
    collection: &str,
    column: &str,
    index_type: Option<&str>,
) -> Result<Value, String> {
    let mut body = serde_json::json!({ "column": column });
    if let Some(t) = index_type {
        body["index_type"] = serde_json::Value::String(t.to_string());
    }
    exec_db_api_request(
        reqwest::Method::POST,
        &format!("/columnar/{}/index", collection),
        Some(body),
    )
    .map(|resp| json_to_value(&resp))
}

pub fn list_indexes(collection: &str) -> Result<Value, String> {
    let resp = exec_db_api_request(
        reqwest::Method::GET,
        &format!("/columnar/{}/indexes", collection),
        None,
    )?;
    let list = resp
        .get("indexes")
        .cloned()
        .unwrap_or(serde_json::Value::Array(vec![]));
    Ok(json_to_value(&list))
}

pub fn drop_index(collection: &str, column: &str) -> Result<Value, String> {
    exec_db_api_request(
        reqwest::Method::DELETE,
        &format!("/columnar/{}/index/{}", collection, column),
        None,
    )
    .map(|resp| json_to_value(&resp))
}

pub fn stats(collection: &str) -> Result<Value, String> {
    exec_db_api_request(
        reqwest::Method::GET,
        &format!("/columnar/{}", collection),
        None,
    )
    .map(|resp| json_to_value(&resp))
}

/// Parse the `{ "columns": [...], "filter": {...}, "limit": n }` argument of
/// `Model.query` into wire pieces, enforcing the endpoint's contract
/// client-side (single filter, known ops, `in` takes an array).
pub fn parse_query_options(
    options: &Value,
) -> Result<(Vec<String>, Option<serde_json::Value>, Option<usize>), String> {
    let hash = match options {
        Value::Hash(h) => h.clone(),
        other => {
            return Err(format!(
                "query() expects an options hash ({{\"columns\": [...], \"filter\": ..., \
                 \"limit\": n}}), got {}",
                other.type_name()
            ))
        }
    };

    let mut columns: Vec<String> = Vec::new();
    let mut filter: Option<serde_json::Value> = None;
    let mut limit: Option<usize> = None;

    for (k, v) in hash.borrow().iter() {
        let key = match k {
            HashKey::String(s) => s.to_string(),
            _ => continue,
        };
        match key.as_str() {
            "columns" => match v {
                Value::Array(arr) => {
                    for c in arr.borrow().iter() {
                        match c {
                            Value::String(s) => {
                                super::core::validate_field_name(s, "query")?;
                                columns.push(s.to_string());
                            }
                            other => {
                                return Err(format!(
                                    "query() columns must be strings, got {}",
                                    other.type_name()
                                ))
                            }
                        }
                    }
                }
                other => {
                    return Err(format!(
                        "query() columns must be an array, got {}",
                        other.type_name()
                    ))
                }
            },
            "filter" => {
                let fh = match v {
                    Value::Hash(h) => h.clone(),
                    other => {
                        return Err(format!(
                            "query() filter must be a hash ({{\"column\", \"op\", \"value\"}}), \
                             got {}",
                            other.type_name()
                        ))
                    }
                };
                let mut column = None;
                let mut op = None;
                let mut value = None;
                for (fk, fv) in fh.borrow().iter() {
                    let fkey = match fk {
                        HashKey::String(s) => s.to_string(),
                        _ => continue,
                    };
                    match fkey.as_str() {
                        "column" => {
                            if let Value::String(s) = fv {
                                super::core::validate_field_name(s, "query")?;
                                column = Some(s.to_string());
                            }
                        }
                        "op" => {
                            if let Value::String(s) = fv {
                                let s = s.to_lowercase();
                                if !COLUMN_FILTER_OPS.contains(&s.as_str()) {
                                    return Err(format!(
                                        "query() unknown filter op '{}': expected one of {}",
                                        s,
                                        COLUMN_FILTER_OPS.join(", ")
                                    ));
                                }
                                op = Some(s);
                            }
                        }
                        "value" => {
                            value = Some(
                                crate::interpreter::value::value_to_json(fv)
                                    .map_err(|e| format!("query() filter value: {}", e))?,
                            )
                        }
                        other => return Err(format!("query() unknown filter key '{}'", other)),
                    }
                }
                let (column, op, value) = match (column, op, value) {
                    (Some(c), Some(o), Some(v)) => (c, o, v),
                    _ => {
                        return Err("query() filter requires column, op, and value keys".to_string())
                    }
                };
                if op == "in" && !value.is_array() {
                    return Err("query() filter op \"in\" requires an array value".to_string());
                }
                filter = Some(serde_json::json!({
                    "column": column,
                    "op": op.to_uppercase(),
                    "value": value,
                }));
            }
            "limit" => match v {
                Value::Int(n) if *n >= 0 => limit = Some(*n as usize),
                other => {
                    return Err(format!(
                        "query() limit must be a non-negative Int, got {}",
                        other.type_name()
                    ))
                }
            },
            other => {
                return Err(format!(
                    "query() unknown option '{}': expected columns, filter, or limit",
                    other
                ))
            }
        }
    }

    if columns.is_empty() {
        return Err("query() requires a non-empty \"columns\" array".to_string());
    }
    Ok((columns, filter, limit))
}
