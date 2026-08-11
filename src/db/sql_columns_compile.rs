//! Compile queries against **real columns** of an existing table.
//!
//! This is the column-aware counterpart to [`super::sql_compile`], which
//! compiles the `_key` + `doc` document layout. The two are deliberately
//! separate: every document compiler emits `SELECT doc` and `doc->>'field'`
//! operators, so parameterizing it would mean threading a mode flag through
//! every function and every consumer. A separate IR keeps the document path
//! untouched and lets the column path validate field names against a schema.
//!
//! Identifiers always go through `Dialect::quote_ident`, and values always go
//! through binds — a field name that isn't a real column is rejected before any
//! SQL is built.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::introspect::{ColType, ColumnDef, TableSchema};
use super::sql_compile::{CompiledSql, Dialect, SqlAgg, SqlBind};

/// A portable list query over real columns.
#[derive(Clone, Debug)]
pub struct ColumnQuery {
    pub schema: Arc<TableSchema>,
    /// Equality filters, keyed by column name. A JSON null means `IS NULL`.
    pub eq_filters: BTreeMap<String, serde_json::Value>,
    pub order_field: Option<String>,
    pub order_desc: bool,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

impl ColumnQuery {
    pub fn new(schema: Arc<TableSchema>) -> Self {
        Self {
            schema,
            eq_filters: BTreeMap::new(),
            order_field: None,
            order_desc: false,
            limit: None,
            offset: None,
        }
    }
}

/// Resolve a Soli-side field name to a real column.
///
/// `id` and `_key` alias the primary key, but only when no actual column of
/// that name exists — a real column always wins, so a table with its own `id`
/// column separate from the PK behaves as written.
pub fn resolve_col<'a>(schema: &'a TableSchema, field: &str) -> Result<&'a ColumnDef, String> {
    if let Some(col) = schema.column(field) {
        return Ok(col);
    }
    if matches!(field, "id" | "_key") {
        if let Some(pk) = schema.column(&schema.pk) {
            return Ok(pk);
        }
    }
    Err(format!(
        "unknown field {field:?} on table {:?}. Columns: {}",
        schema.table,
        schema.column_names().join(", ")
    ))
}

/// Bind one value for `col`, rejecting a type the column can't hold.
///
/// SQL NULL is never bound (the caller emits `IS NULL` / the `NULL` literal),
/// so this returns `None` for a JSON null.
pub fn bind_for_column(
    col: &ColumnDef,
    value: &serde_json::Value,
) -> Result<Option<SqlBind>, String> {
    if value.is_null() {
        return Ok(None);
    }
    let mismatch = || {
        format!(
            "column {:?} is {} — cannot store {}",
            col.name,
            col.ty.as_str(),
            describe_json(value)
        )
    };
    let bind = match col.ty {
        ColType::Int => match value {
            serde_json::Value::Number(n) => n
                .as_i64()
                .or_else(|| n.as_f64().map(|f| f as i64))
                .map(SqlBind::I64)
                .ok_or_else(mismatch)?,
            serde_json::Value::Bool(b) => SqlBind::I64(i64::from(*b)),
            // Accept a numeric string: ids arriving from URLs are strings.
            serde_json::Value::String(s) => {
                s.parse::<i64>().map(SqlBind::I64).map_err(|_| mismatch())?
            }
            _ => return Err(mismatch()),
        },
        ColType::Float | ColType::Decimal => match value {
            serde_json::Value::Number(n) => n.as_f64().map(SqlBind::F64).ok_or_else(mismatch)?,
            serde_json::Value::String(s) => {
                s.parse::<f64>().map(SqlBind::F64).map_err(|_| mismatch())?
            }
            _ => return Err(mismatch()),
        },
        ColType::Bool => match value {
            serde_json::Value::Bool(b) => SqlBind::Bool(*b),
            serde_json::Value::Number(n) => SqlBind::Bool(n.as_i64().unwrap_or(0) != 0),
            serde_json::Value::String(s) => match s.as_str() {
                "true" | "t" | "1" | "yes" => SqlBind::Bool(true),
                "false" | "f" | "0" | "no" => SqlBind::Bool(false),
                _ => return Err(mismatch()),
            },
            _ => return Err(mismatch()),
        },
        ColType::Text | ColType::Uuid | ColType::Date | ColType::DateTime => match value {
            serde_json::Value::String(s) => SqlBind::Text(s.clone()),
            // Numbers/bools stringify rather than failing a text column.
            serde_json::Value::Number(n) => SqlBind::Text(n.to_string()),
            serde_json::Value::Bool(b) => SqlBind::Text(b.to_string()),
            _ => return Err(mismatch()),
        },
        ColType::Json => SqlBind::Json(value.clone()),
        ColType::Unknown => {
            return Err(format!(
                "column {:?} has a type Soli cannot read or write on column-aware \
                 models (arrays, bytea, geometry, unsigned bigint, …). Exclude it \
                 from writes and filters.",
                col.name
            ))
        }
    };
    Ok(Some(bind))
}

fn describe_json(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => "null".into(),
        serde_json::Value::Bool(_) => "a boolean".into(),
        serde_json::Value::Number(_) => "a number".into(),
        serde_json::Value::String(_) => "a string".into(),
        serde_json::Value::Array(_) => "an array".into(),
        serde_json::Value::Object(_) => "a hash".into(),
    }
}

/// A `SELECT` expression for reading `col`, plus the alias it should carry.
///
/// Timestamps, dates, uuids, and exact numerics are read as **text** and
/// normalized in Rust: it keeps driver type-conversion out of the picture and
/// gives one canonical string form across both backends.
fn select_expr(d: Dialect, col: &ColumnDef) -> Result<String, String> {
    let quoted = d.quote_ident(&col.name)?;
    if col.ty.reads_as_text() {
        return Ok(match d {
            Dialect::Postgres => format!("{quoted}::text AS {quoted}"),
            // MySQL converts these itself; CAST keeps the shape identical.
            Dialect::Mysql => format!("CAST({quoted} AS CHAR) AS {quoted}"),
        });
    }
    Ok(quoted)
}

/// Comma-separated select list in schema order.
fn select_list(d: Dialect, schema: &TableSchema) -> Result<String, String> {
    let mut parts = Vec::with_capacity(schema.columns.len());
    for col in &schema.columns {
        // An unreadable column would fail the whole row; skip it instead so the
        // rest of the record is usable (it reads as absent/null in Soli).
        if col.ty == ColType::Unknown {
            continue;
        }
        parts.push(select_expr(d, col)?);
    }
    if parts.is_empty() {
        return Err(format!(
            "table {:?} has no columns Soli can read",
            schema.table
        ));
    }
    Ok(parts.join(", "))
}

/// Build the WHERE clause for `eq_filters`, appending binds to `params`.
fn compile_where(d: Dialect, q: &ColumnQuery, params: &mut Vec<SqlBind>) -> Result<String, String> {
    let mut clauses = Vec::new();
    for (field, value) in &q.eq_filters {
        let col = resolve_col(&q.schema, field)?;
        let quoted = d.quote_ident(&col.name)?;
        match bind_for_column(col, value)? {
            // `col = NULL` is never true in SQL; the caller means "is null".
            None => clauses.push(format!("{quoted} IS NULL")),
            Some(bind) => {
                params.push(bind);
                let ph = placeholder(d, params.len(), col.ty);
                clauses.push(format!("{quoted} = {ph}"));
            }
        }
    }
    if clauses.is_empty() {
        return Ok(String::new());
    }
    Ok(format!(" WHERE {}", clauses.join(" AND ")))
}

/// Placeholder for bind `n`, with the cast Postgres needs for text-carried types.
fn placeholder(d: Dialect, n: usize, ty: ColType) -> String {
    let base = match d {
        Dialect::Postgres => format!("${n}"),
        Dialect::Mysql => "?".to_string(),
    };
    if d == Dialect::Postgres {
        // Text binds into a typed column need an explicit cast on Postgres.
        return match ty {
            ColType::Uuid => format!("{base}::uuid"),
            ColType::Date => format!("{base}::date"),
            ColType::DateTime => format!("{base}::timestamptz"),
            ColType::Decimal => format!("{base}::numeric"),
            ColType::Json => format!("{base}::jsonb"),
            _ => base,
        };
    }
    if ty == ColType::Json {
        return format!("CAST({base} AS JSON)");
    }
    base
}

fn append_order_limit(
    d: Dialect,
    sql: &mut String,
    params: &mut Vec<SqlBind>,
    q: &ColumnQuery,
) -> Result<(), String> {
    if let Some(field) = &q.order_field {
        let col = resolve_col(&q.schema, field)?;
        let quoted = d.quote_ident(&col.name)?;
        let dir = if q.order_desc { "DESC" } else { "ASC" };
        sql.push_str(&format!(" ORDER BY {quoted} {dir}"));
    }
    if let Some(limit) = q.limit {
        params.push(SqlBind::I64(limit as i64));
        let lim = placeholder(d, params.len(), ColType::Int);
        sql.push_str(&format!(" LIMIT {lim}"));
        if let Some(offset) = q.offset {
            params.push(SqlBind::I64(offset as i64));
            let off = placeholder(d, params.len(), ColType::Int);
            sql.push_str(&format!(" OFFSET {off}"));
        }
    } else if let Some(offset) = q.offset {
        // Both dialects require a LIMIT before OFFSET. Rather than invent a
        // magic ceiling, use the maximum the dialect accepts so "skip N, take
        // the rest" means exactly that.
        match d {
            Dialect::Postgres => {
                params.push(SqlBind::I64(offset as i64));
                let off = placeholder(d, params.len(), ColType::Int);
                sql.push_str(&format!(" OFFSET {off}"));
            }
            Dialect::Mysql => {
                params.push(SqlBind::I64(offset as i64));
                let off = placeholder(d, params.len(), ColType::Int);
                sql.push_str(&format!(" LIMIT 18446744073709551615 OFFSET {off}"));
            }
        }
    }
    Ok(())
}

/// `SELECT <columns> FROM t [WHERE …] [ORDER BY …] [LIMIT …]`
pub fn compile_select_cols(d: Dialect, q: &ColumnQuery) -> Result<CompiledSql, String> {
    let table = d.quote_ident(&q.schema.table)?;
    let cols = select_list(d, &q.schema)?;
    let mut params = Vec::new();
    let mut sql = format!("SELECT {cols} FROM {table}");
    sql.push_str(&compile_where(d, q, &mut params)?);
    append_order_limit(d, &mut sql, &mut params, q)?;
    Ok(CompiledSql { sql, params })
}

/// `SELECT COUNT(*) FROM t [WHERE …]`
pub fn compile_count_cols(d: Dialect, q: &ColumnQuery) -> Result<CompiledSql, String> {
    let table = d.quote_ident(&q.schema.table)?;
    let mut params = Vec::new();
    let count = match d {
        Dialect::Postgres => "COUNT(*)::bigint",
        Dialect::Mysql => "COUNT(*)",
    };
    let mut sql = format!("SELECT {count} FROM {table}");
    sql.push_str(&compile_where(d, q, &mut params)?);
    Ok(CompiledSql { sql, params })
}

/// `SELECT 1 FROM t [WHERE …] LIMIT 1`
pub fn compile_exists_cols(d: Dialect, q: &ColumnQuery) -> Result<CompiledSql, String> {
    let table = d.quote_ident(&q.schema.table)?;
    let mut params = Vec::new();
    let mut sql = format!("SELECT 1 FROM {table}");
    sql.push_str(&compile_where(d, q, &mut params)?);
    sql.push_str(" LIMIT 1");
    Ok(CompiledSql { sql, params })
}

/// `SELECT SUM(col) FROM t [WHERE …]` and friends.
pub fn compile_aggregate_cols(
    d: Dialect,
    q: &ColumnQuery,
    func: SqlAgg,
    field: &str,
) -> Result<CompiledSql, String> {
    let col = resolve_col(&q.schema, field)?;
    if !matches!(func, SqlAgg::Count) && !col.ty.is_numeric() {
        return Err(format!(
            "cannot compute {} over column {:?}: it is {}, not numeric",
            agg_name(func),
            col.name,
            col.ty.as_str()
        ));
    }
    let table = d.quote_ident(&q.schema.table)?;
    let quoted = d.quote_ident(&col.name)?;
    let expr = match func {
        SqlAgg::Sum => format!("SUM({quoted})"),
        SqlAgg::Avg => format!("AVG({quoted})"),
        SqlAgg::Min => format!("MIN({quoted})"),
        SqlAgg::Max => format!("MAX({quoted})"),
        SqlAgg::Count => match d {
            Dialect::Postgres => "COUNT(*)::bigint".to_string(),
            Dialect::Mysql => "COUNT(*)".to_string(),
        },
    };
    // Aggregates come back as text so an exact numeric keeps its precision
    // until Soli parses it.
    let expr = match d {
        Dialect::Postgres if !matches!(func, SqlAgg::Count) => format!("({expr})::text"),
        Dialect::Mysql if !matches!(func, SqlAgg::Count) => format!("CAST({expr} AS CHAR)"),
        _ => expr,
    };
    let mut params = Vec::new();
    let mut sql = format!("SELECT {expr} FROM {table}");
    sql.push_str(&compile_where(d, q, &mut params)?);
    Ok(CompiledSql { sql, params })
}

fn agg_name(func: SqlAgg) -> &'static str {
    match func {
        SqlAgg::Sum => "sum",
        SqlAgg::Avg => "avg",
        SqlAgg::Min => "min",
        SqlAgg::Max => "max",
        SqlAgg::Count => "count",
    }
}

/// `INSERT INTO t (cols…) VALUES (…)`, returning the inserted row on Postgres.
///
/// An auto-generated primary key is omitted unless the caller supplied one, so
/// the database assigns it.
pub fn compile_insert_cols(
    d: Dialect,
    schema: &TableSchema,
    doc: &serde_json::Value,
) -> Result<CompiledSql, String> {
    let obj = doc
        .as_object()
        .ok_or_else(|| "column-aware insert expects a hash of fields".to_string())?;

    let mut names = Vec::new();
    let mut placeholders = Vec::new();
    let mut params = Vec::new();
    for col in &schema.columns {
        let Some(value) = obj.get(&col.name) else {
            continue;
        };
        if col.name == schema.pk && schema.pk_auto && value.is_null() {
            continue;
        }
        names.push(d.quote_ident(&col.name)?);
        match bind_for_column(col, value)? {
            None => placeholders.push("NULL".to_string()),
            Some(bind) => {
                params.push(bind);
                placeholders.push(placeholder(d, params.len(), col.ty));
            }
        }
    }
    if names.is_empty() {
        return Err(format!(
            "column-aware insert into {:?} had no writable fields. Columns: {}",
            schema.table,
            schema.column_names().join(", ")
        ));
    }

    let table = d.quote_ident(&schema.table)?;
    let mut sql = format!(
        "INSERT INTO {table} ({}) VALUES ({})",
        names.join(", "),
        placeholders.join(", ")
    );
    if d == Dialect::Postgres {
        sql.push_str(&format!(" RETURNING {}", select_list(d, schema)?));
    }
    Ok(CompiledSql { sql, params })
}

/// `UPDATE t SET … WHERE pk = ?`, returning the updated row on Postgres.
pub fn compile_update_cols(
    d: Dialect,
    schema: &TableSchema,
    pk_value: &serde_json::Value,
    patch: &serde_json::Value,
) -> Result<CompiledSql, String> {
    let obj = patch
        .as_object()
        .ok_or_else(|| "column-aware update expects a hash of fields".to_string())?;

    let mut assignments = Vec::new();
    let mut params = Vec::new();
    for col in &schema.columns {
        // The primary key is the row's identity, not a field to rewrite.
        if col.name == schema.pk {
            continue;
        }
        let Some(value) = obj.get(&col.name) else {
            continue;
        };
        let quoted = d.quote_ident(&col.name)?;
        match bind_for_column(col, value)? {
            None => assignments.push(format!("{quoted} = NULL")),
            Some(bind) => {
                params.push(bind);
                assignments.push(format!(
                    "{quoted} = {}",
                    placeholder(d, params.len(), col.ty)
                ));
            }
        }
    }
    if assignments.is_empty() {
        return Err(format!(
            "column-aware update of {:?} had no writable fields. Columns: {}",
            schema.table,
            schema.column_names().join(", ")
        ));
    }

    let pk_col = schema
        .column(&schema.pk)
        .ok_or_else(|| format!("table {:?} lost its primary key column", schema.table))?;
    let pk_bind = bind_for_column(pk_col, pk_value)?
        .ok_or_else(|| "column-aware update requires a primary-key value".to_string())?;
    params.push(pk_bind);
    let pk_ph = placeholder(d, params.len(), pk_col.ty);

    let table = d.quote_ident(&schema.table)?;
    let pk_quoted = d.quote_ident(&schema.pk)?;
    let mut sql = format!(
        "UPDATE {table} SET {} WHERE {pk_quoted} = {pk_ph}",
        assignments.join(", ")
    );
    if d == Dialect::Postgres {
        sql.push_str(&format!(" RETURNING {}", select_list(d, schema)?));
    }
    Ok(CompiledSql { sql, params })
}

/// `SELECT <columns> FROM t WHERE pk = ?`
pub fn compile_get_cols(
    d: Dialect,
    schema: &TableSchema,
    pk_value: &serde_json::Value,
) -> Result<CompiledSql, String> {
    let pk_col = schema
        .column(&schema.pk)
        .ok_or_else(|| format!("table {:?} lost its primary key column", schema.table))?;
    let bind = bind_for_column(pk_col, pk_value)?
        .ok_or_else(|| "lookup requires a primary-key value".to_string())?;
    let table = d.quote_ident(&schema.table)?;
    let cols = select_list(d, schema)?;
    let pk_quoted = d.quote_ident(&schema.pk)?;
    let ph = placeholder(d, 1, pk_col.ty);
    Ok(CompiledSql {
        sql: format!("SELECT {cols} FROM {table} WHERE {pk_quoted} = {ph}"),
        params: vec![bind],
    })
}

/// `DELETE FROM t WHERE pk = ?`
pub fn compile_delete_cols(
    d: Dialect,
    schema: &TableSchema,
    pk_value: &serde_json::Value,
) -> Result<CompiledSql, String> {
    let pk_col = schema
        .column(&schema.pk)
        .ok_or_else(|| format!("table {:?} lost its primary key column", schema.table))?;
    let bind = bind_for_column(pk_col, pk_value)?
        .ok_or_else(|| "delete requires a primary-key value".to_string())?;
    let table = d.quote_ident(&schema.table)?;
    let pk_quoted = d.quote_ident(&schema.pk)?;
    let ph = placeholder(d, 1, pk_col.ty);
    Ok(CompiledSql {
        sql: format!("DELETE FROM {table} WHERE {pk_quoted} = {ph}"),
        params: vec![bind],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::introspect::build_schema;
    use crate::db::introspect::{pg_coltype, RawColumns};

    fn orders_schema() -> Arc<TableSchema> {
        let raw = RawColumns {
            columns: vec![
                ("id".into(), "int8".into(), String::new(), false, true),
                ("name".into(), "text".into(), String::new(), false, false),
                ("qty".into(), "int4".into(), String::new(), true, false),
                ("total".into(), "numeric".into(), String::new(), true, false),
                ("active".into(), "bool".into(), String::new(), true, false),
                ("meta".into(), "jsonb".into(), String::new(), true, false),
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

    fn query() -> ColumnQuery {
        ColumnQuery::new(orders_schema())
    }

    #[test]
    fn select_names_real_columns_and_skips_unreadable_ones() {
        let c = compile_select_cols(Dialect::Postgres, &query()).unwrap();
        assert!(
            c.sql.starts_with("SELECT \"id\", \"name\", \"qty\""),
            "{}",
            c.sql
        );
        // Exact/temporal types are read as text for a canonical form.
        assert!(c.sql.contains("\"total\"::text AS \"total\""), "{}", c.sql);
        assert!(
            c.sql.contains("\"created_at\"::text AS \"created_at\""),
            "{}",
            c.sql
        );
        // A type Soli can't read is left out rather than failing every row.
        assert!(!c.sql.contains("blob"), "{}", c.sql);
        assert!(c.sql.contains("FROM \"orders\""), "{}", c.sql);
        assert!(c.params.is_empty());
    }

    #[test]
    fn eq_filters_bind_by_column_type() {
        let mut q = query();
        q.eq_filters.insert("name".into(), serde_json::json!("Ada"));
        q.eq_filters.insert("qty".into(), serde_json::json!(3));
        q.eq_filters
            .insert("active".into(), serde_json::json!(true));
        let c = compile_select_cols(Dialect::Postgres, &q).unwrap();
        // BTreeMap order: active, name, qty.
        assert!(c.sql.contains("WHERE \"active\" = $1"), "{}", c.sql);
        assert!(c.sql.contains("\"name\" = $2"), "{}", c.sql);
        assert!(c.sql.contains("\"qty\" = $3"), "{}", c.sql);
        assert_eq!(
            c.params,
            vec![
                SqlBind::Bool(true),
                SqlBind::Text("Ada".into()),
                SqlBind::I64(3)
            ]
        );
    }

    #[test]
    fn null_filter_compiles_to_is_null_and_binds_nothing() {
        // `col = NULL` is never true, so a null filter must become IS NULL.
        let mut q = query();
        q.eq_filters.insert("qty".into(), serde_json::Value::Null);
        let c = compile_select_cols(Dialect::Postgres, &q).unwrap();
        assert!(c.sql.contains("WHERE \"qty\" IS NULL"), "{}", c.sql);
        assert!(c.params.is_empty(), "NULL is emitted, never bound");
    }

    #[test]
    fn order_limit_offset_are_parameterized() {
        let mut q = query();
        q.order_field = Some("created_at".into());
        q.order_desc = true;
        q.limit = Some(10);
        q.offset = Some(20);
        let c = compile_select_cols(Dialect::Postgres, &q).unwrap();
        assert!(c.sql.contains("ORDER BY \"created_at\" DESC"), "{}", c.sql);
        assert!(c.sql.contains("LIMIT $1 OFFSET $2"), "{}", c.sql);
        assert_eq!(c.params, vec![SqlBind::I64(10), SqlBind::I64(20)]);
    }

    #[test]
    fn offset_without_limit_uses_a_bare_offset_on_postgres() {
        // The document compiler needs a magic LIMIT here; Postgres supports a
        // bare OFFSET, so no invented ceiling.
        let mut q = query();
        q.offset = Some(5);
        let pg = compile_select_cols(Dialect::Postgres, &q).unwrap();
        assert!(pg.sql.contains("OFFSET $1"), "{}", pg.sql);
        assert!(!pg.sql.contains("LIMIT"), "{}", pg.sql);
        // MySQL requires a LIMIT; use its documented maximum.
        let my = compile_select_cols(Dialect::Mysql, &q).unwrap();
        assert!(
            my.sql.contains("LIMIT 18446744073709551615 OFFSET ?"),
            "{}",
            my.sql
        );
    }

    #[test]
    fn id_and_key_alias_the_primary_key() {
        let schema = orders_schema();
        assert_eq!(resolve_col(&schema, "id").unwrap().name, "id");
        // `_key` is the document-mode name; it maps onto the real PK.
        assert_eq!(resolve_col(&schema, "_key").unwrap().name, "id");
        assert_eq!(resolve_col(&schema, "name").unwrap().name, "name");
    }

    #[test]
    fn unknown_field_lists_the_real_columns() {
        let schema = orders_schema();
        let err = resolve_col(&schema, "nope").expect_err("must error");
        assert!(err.contains("unknown field"), "{err}");
        assert!(err.contains("\"orders\""), "{err}");
        assert!(err.contains("name") && err.contains("qty"), "{err}");
    }

    #[test]
    fn type_mismatches_are_rejected_with_the_column_and_type() {
        let schema = orders_schema();
        let qty = schema.column("qty").unwrap();
        let err = bind_for_column(qty, &serde_json::json!("not-a-number")).expect_err("must error");
        assert!(err.contains("qty") && err.contains("Int"), "{err}");

        // An unsupported column type names itself rather than silently dropping.
        let blob = schema.column("blob").unwrap();
        let err = bind_for_column(blob, &serde_json::json!("x")).expect_err("must error");
        assert!(err.contains("blob"), "{err}");
    }

    #[test]
    fn numeric_strings_and_bools_coerce_where_sensible() {
        let schema = orders_schema();
        let qty = schema.column("qty").unwrap();
        // Ids and numbers arriving from URLs/forms are strings.
        assert_eq!(
            bind_for_column(qty, &serde_json::json!("42")).unwrap(),
            Some(SqlBind::I64(42))
        );
        let active = schema.column("active").unwrap();
        assert_eq!(
            bind_for_column(active, &serde_json::json!("true")).unwrap(),
            Some(SqlBind::Bool(true))
        );
        assert_eq!(
            bind_for_column(active, &serde_json::json!(0)).unwrap(),
            Some(SqlBind::Bool(false))
        );
    }

    #[test]
    fn insert_omits_a_generated_key_and_returns_the_row() {
        let schema = orders_schema();
        let doc = serde_json::json!({ "name": "Ada", "qty": 2, "id": null });
        let c = compile_insert_cols(Dialect::Postgres, &schema, &doc).unwrap();
        assert!(
            !c.sql.contains("(\"id\""),
            "a generated PK must be left to the database: {}",
            c.sql
        );
        assert!(
            c.sql.contains("INSERT INTO \"orders\" (\"name\", \"qty\")"),
            "{}",
            c.sql
        );
        assert!(c.sql.contains("RETURNING \"id\""), "{}", c.sql);
        assert_eq!(c.params, vec![SqlBind::Text("Ada".into()), SqlBind::I64(2)]);
    }

    #[test]
    fn insert_honors_an_explicit_key() {
        let schema = orders_schema();
        let doc = serde_json::json!({ "id": 7, "name": "Ada" });
        let c = compile_insert_cols(Dialect::Postgres, &schema, &doc).unwrap();
        assert!(c.sql.contains("(\"id\", \"name\")"), "{}", c.sql);
        assert_eq!(c.params, vec![SqlBind::I64(7), SqlBind::Text("Ada".into())]);
    }

    #[test]
    fn insert_casts_text_carried_types_on_postgres() {
        let schema = orders_schema();
        let doc = serde_json::json!({
            "name": "Ada",
            "created_at": "2026-08-11T10:00:00Z",
            "total": "19.99",
            "meta": { "k": 1 }
        });
        let c = compile_insert_cols(Dialect::Postgres, &schema, &doc).unwrap();
        assert!(c.sql.contains("::timestamptz"), "{}", c.sql);
        assert!(c.sql.contains("::numeric"), "{}", c.sql);
        assert!(c.sql.contains("::jsonb"), "{}", c.sql);
    }

    #[test]
    fn update_sets_only_supplied_fields_and_never_the_primary_key() {
        let schema = orders_schema();
        let patch = serde_json::json!({ "name": "Grace", "id": 999, "qty": null });
        let c =
            compile_update_cols(Dialect::Postgres, &schema, &serde_json::json!(7), &patch).unwrap();
        assert!(
            c.sql.contains("SET \"name\" = $1, \"qty\" = NULL"),
            "{}",
            c.sql
        );
        assert!(
            !c.sql.contains("SET \"id\"") && !c.sql.contains(", \"id\" ="),
            "the PK identifies the row, it is never rewritten: {}",
            c.sql
        );
        assert!(c.sql.contains("WHERE \"id\" = $2"), "{}", c.sql);
        assert_eq!(
            c.params,
            vec![SqlBind::Text("Grace".into()), SqlBind::I64(7)]
        );
    }

    #[test]
    fn get_and_delete_target_the_primary_key() {
        let schema = orders_schema();
        let get = compile_get_cols(Dialect::Postgres, &schema, &serde_json::json!("7")).unwrap();
        assert!(get.sql.contains("WHERE \"id\" = $1"), "{}", get.sql);
        // A string id coerces into the Int PK.
        assert_eq!(get.params, vec![SqlBind::I64(7)]);

        let del = compile_delete_cols(Dialect::Mysql, &schema, &serde_json::json!(7)).unwrap();
        assert_eq!(del.sql, "DELETE FROM `orders` WHERE `id` = ?");
    }

    #[test]
    fn count_and_exists_compile_per_dialect() {
        let mut q = query();
        q.eq_filters.insert("name".into(), serde_json::json!("Ada"));
        let pg = compile_count_cols(Dialect::Postgres, &q).unwrap();
        assert_eq!(
            pg.sql,
            "SELECT COUNT(*)::bigint FROM \"orders\" WHERE \"name\" = $1"
        );
        let my = compile_count_cols(Dialect::Mysql, &q).unwrap();
        assert_eq!(my.sql, "SELECT COUNT(*) FROM `orders` WHERE `name` = ?");
        let ex = compile_exists_cols(Dialect::Postgres, &q).unwrap();
        assert!(ex.sql.ends_with("LIMIT 1"), "{}", ex.sql);
    }

    #[test]
    fn aggregates_require_a_numeric_column() {
        let q = query();
        let ok = compile_aggregate_cols(Dialect::Postgres, &q, SqlAgg::Sum, "total").unwrap();
        assert!(ok.sql.contains("SUM(\"total\")"), "{}", ok.sql);

        // Summing text is a mistake worth naming precisely.
        let err = compile_aggregate_cols(Dialect::Postgres, &q, SqlAgg::Avg, "name")
            .expect_err("must error");
        assert!(
            err.contains("avg") && err.contains("name") && err.contains("Text"),
            "{err}"
        );

        // COUNT works regardless of column type.
        assert!(compile_aggregate_cols(Dialect::Postgres, &q, SqlAgg::Count, "name").is_ok());
    }

    #[test]
    fn identifiers_are_always_quoted_so_names_cannot_inject() {
        // A hostile table/column name must fail quoting, not reach the SQL.
        let raw = RawColumns {
            columns: vec![(
                "id\"; DROP TABLE users; --".into(),
                "int8".into(),
                String::new(),
                false,
                false,
            )],
            pk: vec!["id\"; DROP TABLE users; --".into()],
        };
        let schema = Arc::new(build_schema("legacy", "t", raw, |t, _| pg_coltype(t)).unwrap());
        let err = compile_select_cols(Dialect::Postgres, &ColumnQuery::new(schema))
            .expect_err("invalid identifier must be refused");
        assert!(err.contains("invalid SQL identifier"), "{err}");
    }
}
