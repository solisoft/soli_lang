//! Execution facade for column-aware models.
//!
//! Mirrors [`super::sql`] but for real-column tables: it routes each operation
//! to the active adapter's `col_*` implementation (Postgres, MySQL, SQLite).
//! Those run through the same private `with_conn` the document path uses, which
//! is what makes `Model.transaction` work here for free — an open transaction on
//! the same named connection is reused automatically.
//!
//! Nothing on this path ever issues DDL. Column mode maps to a schema someone
//! else owns.

// When no SQL client is linked, every dispatch arm returns `feature_missing`
// without reading the call args — silence that noise.
#![cfg_attr(
    not(any(feature = "postgres", feature = "mysql", feature = "sqlite")),
    allow(unused_variables)
)]

use std::sync::Arc;

use super::introspect::TableSchema;
use super::sql_columns_compile::ColumnQuery;
use super::sql_compile::SqlAgg;
use super::Adapter;

/// Only referenced when one of the SQL adapters is off.
#[cfg(any(
    not(feature = "postgres"),
    not(feature = "mysql"),
    not(feature = "sqlite")
))]
fn feature_missing(adapter: &str) -> String {
    format!(
        "column-aware models need the `{adapter}` adapter, which is not compiled \
         into this soli binary. Rebuild with `--features {adapter}`."
    )
}

/// Dispatch to the active SQL backend, or a clear missing-feature error.
macro_rules! route_cols {
    ($pg:expr, $my:expr, $lite:expr) => {{
        match super::registry::active_spec()?.adapter {
            Adapter::Mysql => {
                #[cfg(feature = "mysql")]
                {
                    $my
                }
                #[cfg(not(feature = "mysql"))]
                {
                    Err(feature_missing("mysql"))
                }
            }
            Adapter::Postgres => {
                #[cfg(feature = "postgres")]
                {
                    $pg
                }
                #[cfg(not(feature = "postgres"))]
                {
                    Err(feature_missing("postgres"))
                }
            }
            Adapter::Sqlite => {
                #[cfg(feature = "sqlite")]
                {
                    $lite
                }
                #[cfg(not(feature = "sqlite"))]
                {
                    Err(feature_missing("sqlite"))
                }
            }
            Adapter::Solidb => Err("column-aware models require a SQL connection \
                                    (internal error; report this)"
                .to_string()),
        }
    }};
}

/// Fetch one row by primary key.
pub fn get_row(
    schema: &Arc<TableSchema>,
    pk: &serde_json::Value,
) -> Result<Option<serde_json::Value>, String> {
    route_cols!(
        super::postgres::col_get(schema, pk),
        super::mysql::col_get(schema, pk),
        super::sqlite::col_get(schema, pk)
    )
}

/// Insert a row, returning it as stored (so generated keys and defaults are visible).
pub fn insert_row(
    schema: &Arc<TableSchema>,
    doc: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    route_cols!(
        super::postgres::col_insert(schema, doc),
        super::mysql::col_insert(schema, doc),
        super::sqlite::col_insert(schema, doc)
    )
}

/// Update a row by primary key, returning it as stored.
pub fn update_row(
    schema: &Arc<TableSchema>,
    pk: &serde_json::Value,
    patch: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    route_cols!(
        super::postgres::col_update(schema, pk, patch),
        super::mysql::col_update(schema, pk, patch),
        super::sqlite::col_update(schema, pk, patch)
    )
}

/// Delete a row by primary key.
pub fn delete_row(schema: &Arc<TableSchema>, pk: &serde_json::Value) -> Result<(), String> {
    route_cols!(
        super::postgres::col_delete(schema, pk),
        super::mysql::col_delete(schema, pk),
        super::sqlite::col_delete(schema, pk)
    )
}

pub fn select_rows(q: &ColumnQuery) -> Result<Vec<serde_json::Value>, String> {
    route_cols!(
        super::postgres::col_select(q),
        super::mysql::col_select(q),
        super::sqlite::col_select(q)
    )
}

pub fn count(q: &ColumnQuery) -> Result<i64, String> {
    route_cols!(
        super::postgres::col_count(q),
        super::mysql::col_count(q),
        super::sqlite::col_count(q)
    )
}

pub fn exists(q: &ColumnQuery) -> Result<bool, String> {
    route_cols!(
        super::postgres::col_exists(q),
        super::mysql::col_exists(q),
        super::sqlite::col_exists(q)
    )
}

/// Scalar aggregate over a numeric column. Returns JSON null for an empty set.
pub fn aggregate(q: &ColumnQuery, func: SqlAgg, field: &str) -> Result<serde_json::Value, String> {
    route_cols!(
        super::postgres::col_aggregate(q, func, field),
        super::mysql::col_aggregate(q, func, field),
        super::sqlite::col_aggregate(q, func, field)
    )
}

/// Add `delta` to a numeric column of one row, atomically, returning the new
/// value (`None` when the row is absent).
pub fn increment_column(
    schema: &Arc<TableSchema>,
    pk: &serde_json::Value,
    column: &str,
    delta: i64,
) -> Result<Option<i64>, String> {
    route_cols!(
        super::postgres::col_increment(schema, pk, column, delta),
        super::mysql::col_increment(schema, pk, column, delta),
        super::sqlite::col_increment(schema, pk, column, delta)
    )
}

/// Grouped aggregation over real columns.
pub fn group_by(
    q: &ColumnQuery,
    group_fields: &[String],
    aggs: &[super::sql_compile::GroupAgg],
) -> Result<Vec<serde_json::Value>, String> {
    route_cols!(
        super::postgres::col_group_by(q, group_fields, aggs),
        super::mysql::col_group_by(q, group_fields, aggs),
        super::sqlite::col_group_by(q, group_fields, aggs)
    )
}

/// Bulk delete. Returns the number of rows removed.
pub fn delete_all(q: &ColumnQuery) -> Result<u64, String> {
    route_cols!(
        super::postgres::col_delete_all(q),
        super::mysql::col_delete_all(q),
        super::sqlite::col_delete_all(q)
    )
}

/// Bulk update. Returns the number of rows changed.
pub fn update_all(q: &ColumnQuery, patch: &serde_json::Value) -> Result<u64, String> {
    route_cols!(
        super::postgres::col_update_all(q, patch),
        super::mysql::col_update_all(q, patch),
        super::sqlite::col_update_all(q, patch)
    )
}

/// Result column names for a grouped query: the group columns, then each
/// aggregate's alias (or `n` for the implicit count).
pub fn group_result_names(
    group_fields: &[String],
    aggs: &[super::sql_compile::GroupAgg],
) -> Vec<String> {
    let mut names: Vec<String> = group_fields.to_vec();
    if aggs.is_empty() {
        names.push("n".to_string());
    } else {
        for agg in aggs {
            names.push(agg.alias.clone());
        }
    }
    names
}

/// Parse an aggregate's text result into a number, or keep it as a string.
///
/// Only ever applied to aggregate columns: `COUNT`/`SUM`/`AVG`/`MIN`/`MAX` are
/// numeric by construction, so a numeric-looking result *is* a number.
pub fn parse_aggregate_text(text: String) -> serde_json::Value {
    if let Ok(n) = text.trim().parse::<i64>() {
        serde_json::json!(n)
    } else if let Ok(f) = text.trim().parse::<f64>() {
        serde_json::json!(f)
    } else {
        serde_json::Value::String(text)
    }
}

/// One grouped row. Aggregates parse to numbers; a group key parses to a number
/// only when its column is genuinely numeric, and otherwise keeps the exact text
/// the column held.
///
/// `group_key_count` is how many leading entries of `names` are group keys (the
/// rest are aggregate aliases — see [`group_result_names`]), and
/// `group_key_numeric` says, per group key, whether its column type is numeric.
///
/// The type has to be consulted rather than guessed either way. Parsing every
/// column (the original behaviour) turned a *text* key of `"00042"` into `42`
/// and collapsed `"01"` and `"1"` into one bucket downstream. Parsing none of
/// them fixes that but changes `Order.group_by("year")` on an integer column
/// from `{"year": 2024}` to `{"year": "2024"}`, breaking arithmetic on the key
/// and any `row.year == 2024` comparison.
pub fn group_row_to_json(
    names: &[String],
    texts: &[Option<String>],
    group_key_count: usize,
    group_key_numeric: &[bool],
) -> serde_json::Value {
    let mut out = serde_json::Map::new();
    for (index, name) in names.iter().enumerate() {
        let raw = texts.get(index).cloned().flatten();
        let value = match raw {
            None => serde_json::Value::Null,
            Some(text) if index < group_key_count => {
                if group_key_numeric.get(index).copied().unwrap_or(false) {
                    parse_aggregate_text(text)
                } else {
                    serde_json::Value::String(text)
                }
            }
            Some(text) => parse_aggregate_text(text),
        };
        out.insert(name.clone(), value);
    }
    serde_json::Value::Object(out)
}

/// Per group key, whether its column holds a numeric type — the companion
/// argument to [`group_row_to_json`]. An unknown column name is treated as
/// non-numeric so its text is preserved verbatim.
pub fn group_key_numeric_flags(schema: &TableSchema, group_fields: &[String]) -> Vec<bool> {
    group_fields
        .iter()
        .map(|field| schema.column(field).is_some_and(|col| col.ty.is_numeric()))
        .collect()
}

/// Stamp `created_at` / `updated_at` when the table has them and the caller
/// didn't set them. Column mode never invents columns, so this is a no-op on a
/// table without those names.
pub fn apply_timestamps(schema: &TableSchema, doc: &mut serde_json::Value, inserting: bool) {
    let now = crate::jobs::now_iso();
    let Some(obj) = doc.as_object_mut() else {
        return;
    };
    if inserting && schema.has_created_at && !obj.contains_key("created_at") {
        obj.insert("created_at".to_string(), serde_json::json!(now));
    }
    if schema.has_updated_at && !obj.contains_key("updated_at") {
        obj.insert("updated_at".to_string(), serde_json::json!(now));
    }
}

/// Parse an aggregate's text result into a JSON number (or null for an empty
/// set). Aggregates come back as text so exact numerics keep their precision
/// until here.
pub fn parse_agg_text(raw: Option<String>) -> serde_json::Value {
    match raw {
        None => serde_json::Value::Null,
        Some(text) => {
            let trimmed = text.trim();
            if let Ok(n) = trimmed.parse::<i64>() {
                return serde_json::json!(n);
            }
            match trimmed.parse::<f64>() {
                Ok(f) => serde_json::json!(f),
                Err(_) => serde_json::Value::Null,
            }
        }
    }
}

#[cfg(test)]
mod group_row_tests {
    use super::{group_key_numeric_flags, group_row_to_json, parse_aggregate_text};
    use crate::db::introspect::{build_schema, pg_coltype, RawColumns};
    use serde_json::json;

    /// A group key on a *text* column keeps the text the column held. Parsing it
    /// collapsed distinct keys — `"00042"` became `42`, and `"01"` and `"1"`
    /// landed in one bucket.
    #[test]
    fn text_group_keys_keep_their_text_and_aggregates_parse() {
        let names = vec!["code".to_string(), "n".to_string()];
        let row = group_row_to_json(
            &names,
            &[Some("00042".to_string()), Some("7".to_string())],
            1,
            &[false],
        );
        assert_eq!(row, json!({ "code": "00042", "n": 7 }));
    }

    /// A group key on a numeric column stays a JSON number, so `row.year == 2024`
    /// and arithmetic on the key keep working.
    #[test]
    fn numeric_group_keys_stay_numbers() {
        let names = vec!["year".to_string(), "n".to_string()];
        let row = group_row_to_json(
            &names,
            &[Some("2024".to_string()), Some("7".to_string())],
            1,
            &[true],
        );
        assert_eq!(row, json!({ "year": 2024, "n": 7 }));
    }

    #[test]
    fn leading_zero_text_keys_stay_distinct() {
        let names = vec!["code".to_string(), "n".to_string()];
        let a = group_row_to_json(
            &names,
            &[Some("01".to_string()), Some("1".to_string())],
            1,
            &[false],
        );
        let b = group_row_to_json(
            &names,
            &[Some("1".to_string()), Some("1".to_string())],
            1,
            &[false],
        );
        assert_ne!(a["code"], b["code"], "distinct keys must not merge");
    }

    #[test]
    fn multiple_group_keys_and_aggregates_split_at_the_boundary() {
        let names = vec![
            "region".to_string(),
            "zip".to_string(),
            "total".to_string(),
            "avg".to_string(),
        ];
        let row = group_row_to_json(
            &names,
            &[
                Some("north".to_string()),
                Some("01234".to_string()),
                Some("10".to_string()),
                Some("2.5".to_string()),
            ],
            2,
            &[false, false],
        );
        assert_eq!(
            row,
            json!({ "region": "north", "zip": "01234", "total": 10, "avg": 2.5 })
        );
    }

    #[test]
    fn a_null_cell_is_null_on_either_side_of_the_boundary() {
        let names = vec!["k".to_string(), "n".to_string()];
        let row = group_row_to_json(&names, &[None, None], 1, &[false]);
        assert_eq!(row, json!({ "k": null, "n": null }));
    }

    /// The flags come from the schema, so `int`/`numeric` keys parse and
    /// `text`/`date` keys do not. An unknown name is treated as non-numeric.
    #[test]
    fn numeric_flags_follow_the_column_types() {
        let mut raw = RawColumns {
            columns: vec![("id".into(), "int8".into(), String::new(), false, true)],
            pk: vec!["id".into()],
        };
        for (name, ty) in [
            ("year", "int4"),
            ("amount", "numeric"),
            ("code", "text"),
            ("day", "date"),
        ] {
            raw.columns
                .push((name.to_string(), ty.to_string(), String::new(), true, false));
        }
        let schema = build_schema("legacy", "orders", raw, |t, _| pg_coltype(t)).unwrap();
        assert_eq!(
            group_key_numeric_flags(
                &schema,
                &[
                    "year".to_string(),
                    "amount".to_string(),
                    "code".to_string(),
                    "day".to_string(),
                    "nope".to_string(),
                ]
            ),
            vec![true, true, false, false, false]
        );
    }

    #[test]
    fn aggregate_text_parses_ints_floats_and_keeps_the_rest() {
        assert_eq!(parse_aggregate_text("7".into()), json!(7));
        assert_eq!(parse_aggregate_text("2.5".into()), json!(2.5));
        assert_eq!(parse_aggregate_text("abc".into()), json!("abc"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::introspect::{build_schema, pg_coltype, RawColumns};

    fn schema_with(columns: &[(&str, &str)]) -> Arc<TableSchema> {
        let mut raw = RawColumns {
            columns: vec![("id".into(), "int8".into(), String::new(), false, true)],
            pk: vec!["id".into()],
        };
        for (name, ty) in columns {
            raw.columns.push((
                (*name).to_string(),
                (*ty).to_string(),
                String::new(),
                true,
                false,
            ));
        }
        Arc::new(build_schema("legacy", "t", raw, |t, _| pg_coltype(t)).unwrap())
    }

    #[test]
    fn timestamps_are_stamped_only_when_the_columns_exist() {
        let with_ts = schema_with(&[("created_at", "timestamptz"), ("updated_at", "timestamptz")]);
        let mut doc = serde_json::json!({ "id": 1 });
        apply_timestamps(&with_ts, &mut doc, true);
        assert!(doc["created_at"].is_string());
        assert!(doc["updated_at"].is_string());

        // A table without those columns must not gain invented fields — column
        // mode writes only real columns.
        let without = schema_with(&[("name", "text")]);
        let mut doc = serde_json::json!({ "id": 1 });
        apply_timestamps(&without, &mut doc, true);
        assert!(doc.get("created_at").is_none());
        assert!(doc.get("updated_at").is_none());
    }

    #[test]
    fn update_refreshes_updated_at_but_not_created_at() {
        let schema = schema_with(&[("created_at", "timestamptz"), ("updated_at", "timestamptz")]);
        let mut doc = serde_json::json!({ "name": "x" });
        apply_timestamps(&schema, &mut doc, false);
        assert!(doc.get("created_at").is_none(), "created_at is insert-only");
        assert!(doc["updated_at"].is_string());
    }

    #[test]
    fn caller_supplied_timestamps_win() {
        let schema = schema_with(&[("created_at", "timestamptz"), ("updated_at", "timestamptz")]);
        let mut doc = serde_json::json!({
            "created_at": "2020-01-01T00:00:00Z",
            "updated_at": "2020-01-02T00:00:00Z"
        });
        apply_timestamps(&schema, &mut doc, true);
        assert_eq!(doc["created_at"], "2020-01-01T00:00:00Z");
        assert_eq!(doc["updated_at"], "2020-01-02T00:00:00Z");
    }

    #[test]
    fn aggregate_text_parses_to_int_float_or_null() {
        assert_eq!(parse_agg_text(Some("42".into())), serde_json::json!(42));
        assert_eq!(
            parse_agg_text(Some(" 19.99 ".into())),
            serde_json::json!(19.99)
        );
        // An empty result set is null, not zero — SUM of nothing isn't 0.
        assert_eq!(parse_agg_text(None), serde_json::Value::Null);
        assert_eq!(parse_agg_text(Some("".into())), serde_json::Value::Null);
    }
}
