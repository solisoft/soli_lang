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

use super::hash_filter::HashFilter;
use super::introspect::{ColType, ColumnDef, TableSchema};
use super::sql_compile::{CompiledSql, Dialect, GroupAgg, SqlAgg, SqlBind};

/// A `.join` existence filter over real columns.
#[derive(Clone, Debug)]
pub struct ColExistsFilter {
    pub table: String,
    pub foreign_key: String,
    pub parent_pk: String,
    pub hash_filter: Option<HashFilter>,
    pub eq_filters: BTreeMap<String, serde_json::Value>,
    pub child_schema: Arc<TableSchema>,
}

/// A portable list query over real columns.
#[derive(Clone, Debug)]
pub struct ColumnQuery {
    pub schema: Arc<TableSchema>,
    /// Equality filters, keyed by column name. A JSON null means `IS NULL`.
    pub eq_filters: BTreeMap<String, serde_json::Value>,
    /// `column IN (…)` filters. This is what makes eager loading one query per
    /// association instead of one per parent row.
    pub in_filters: BTreeMap<String, Vec<serde_json::Value>>,
    /// Project only these columns (`pluck` / `select`), plus the primary key.
    /// `None` selects the whole row.
    pub select_fields: Option<Vec<String>>,
    /// Structured hash `.where` (comparisons, IN, LIKE, OR).
    pub hash_filter: Option<super::hash_filter::HashFilter>,
    /// `.join` existence filters, compiled as correlated `EXISTS`.
    pub exists_filters: Vec<ColExistsFilter>,
    /// Portable `.having` text (`n > 5`, `total >= 100`).
    pub having: Option<String>,
    /// Columns that must be non-null (`only_deleted` on a soft-delete model).
    pub not_null_filters: Vec<String>,
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
            in_filters: BTreeMap::new(),
            select_fields: None,
            hash_filter: None,
            exists_filters: Vec::new(),
            having: None,
            not_null_filters: Vec::new(),
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
        ColType::Float => match value {
            serde_json::Value::Number(n) => n.as_f64().map(SqlBind::F64).ok_or_else(mismatch)?,
            serde_json::Value::String(s) => {
                s.parse::<f64>().map(SqlBind::F64).map_err(|_| mismatch())?
            }
            _ => return Err(mismatch()),
        },
        // Exact numerics travel as text so no scale is lost in an f64 round
        // trip, and so the driver never has to bind a float into `numeric`.
        ColType::Decimal => match value {
            serde_json::Value::Number(n) => SqlBind::Text(n.to_string()),
            serde_json::Value::String(s) => {
                // Validate before sending: a bad string would be a runtime
                // database error rather than a named field problem.
                s.parse::<f64>().map_err(|_| mismatch())?;
                SqlBind::Text(s.clone())
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
            // MySQL and SQLite convert these themselves; CAST keeps the shape
            // identical across all three.
            Dialect::Mysql => format!("CAST({quoted} AS CHAR) AS {quoted}"),
            Dialect::Sqlite => format!("CAST({quoted} AS TEXT) AS {quoted}"),
        });
    }
    Ok(quoted)
}

/// The columns a `SELECT` for this query actually returns, **in order**.
///
/// The reader walks results positionally, so it must derive its column list from
/// the same function the select list does. Deriving them independently is how a
/// projection ends up writing column 0's value into the schema's first field.
pub fn selected_columns(
    schema: &TableSchema,
    select_fields: &Option<Vec<String>>,
) -> Result<Vec<String>, String> {
    let Some(fields) = select_fields else {
        // The whole row, minus columns Soli cannot read (they are not selected).
        return Ok(schema
            .columns
            .iter()
            .filter(|c| c.ty != ColType::Unknown)
            .map(|c| c.name.clone())
            .collect());
    };
    let mut out: Vec<String> = Vec::with_capacity(fields.len() + 1);
    for field in fields {
        let col = resolve_col(schema, field)?;
        if col.ty == ColType::Unknown {
            return Err(format!(
                "column {:?} has a type Soli cannot read, so it cannot be projected",
                col.name
            ));
        }
        if !out.contains(&col.name) {
            out.push(col.name.clone());
        }
    }
    // The key keeps `_key` / `id` available on the projected row.
    if !out.contains(&schema.pk) && schema.column(&schema.pk).is_some() {
        out.push(schema.pk.clone());
    }
    if out.is_empty() {
        return Err("a projection must name at least one column".to_string());
    }
    Ok(out)
}

/// Select list for the columns [`selected_columns`] returned.
fn select_list_for(d: Dialect, schema: &TableSchema, columns: &[String]) -> Result<String, String> {
    let mut parts = Vec::with_capacity(columns.len());
    for name in columns {
        let col = schema
            .column(name)
            .ok_or_else(|| format!("table {:?} has no column {name:?}", schema.table))?;
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
    if let Some(pred) = &q.hash_filter {
        let sql = super::hash_filter::compile_col_pred(d, &q.schema, pred, params)?;
        if !sql.is_empty() {
            clauses.push(sql);
        }
    }
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
    for (field, values) in &q.in_filters {
        let col = resolve_col(&q.schema, field)?;
        let quoted = d.quote_ident(&col.name)?;
        // An empty IN list matches nothing. Emitting `IN ()` is a syntax error on
        // every backend, so say so in SQL instead.
        if values.is_empty() {
            clauses.push("1 = 0".to_string());
            continue;
        }
        let mut placeholders = Vec::with_capacity(values.len());
        let mut has_null = false;
        for value in values {
            match bind_for_column(col, value)? {
                // A JSON null in the list means "or IS NULL": SQL `IN` never
                // matches NULL.
                None => has_null = true,
                Some(bind) => {
                    params.push(bind);
                    placeholders.push(placeholder(d, params.len(), col.ty));
                }
            }
        }
        let mut clause = if placeholders.is_empty() {
            String::new()
        } else {
            format!("{quoted} IN ({})", placeholders.join(", "))
        };
        if has_null {
            if clause.is_empty() {
                clause = format!("{quoted} IS NULL");
            } else {
                clause = format!("({clause} OR {quoted} IS NULL)");
            }
        }
        clauses.push(clause);
    }
    for field in &q.not_null_filters {
        let col = resolve_col(&q.schema, field)?;
        clauses.push(format!("{} IS NOT NULL", d.quote_ident(&col.name)?));
    }
    let parent = d.quote_ident(&q.schema.table)?;
    let parent_pk = d.quote_ident(&q.schema.pk)?;
    for exists in &q.exists_filters {
        let child = d.quote_ident(&exists.table)?;
        let fk = resolve_col(&exists.child_schema, &exists.foreign_key)?;
        let fk_q = d.quote_ident(&fk.name)?;
        let mut inner = vec![format!("{child}.{fk_q} = {parent}.{parent_pk}")];
        if let Some(pred) = &exists.hash_filter {
            let extra =
                super::hash_filter::compile_col_pred(d, &exists.child_schema, pred, params)?;
            if !extra.is_empty() {
                inner.push(extra);
            }
        }
        for (field, value) in &exists.eq_filters {
            let col = resolve_col(&exists.child_schema, field)?;
            let quoted = d.quote_ident(&col.name)?;
            match bind_for_column(col, value)? {
                None => inner.push(format!("{quoted} IS NULL")),
                Some(bind) => {
                    params.push(bind);
                    let ph = placeholder(d, params.len(), col.ty);
                    inner.push(format!("{quoted} = {ph}"));
                }
            }
        }
        clauses.push(format!(
            "EXISTS (SELECT 1 FROM {child} WHERE {})",
            inner.join(" AND ")
        ));
    }
    if clauses.is_empty() {
        return Ok(String::new());
    }
    Ok(format!(" WHERE {}", clauses.join(" AND ")))
}

/// Placeholder for bind `n`, with the cast Postgres needs for text-carried types.
pub(crate) fn placeholder(d: Dialect, n: usize, ty: ColType) -> String {
    let base = match d {
        Dialect::Postgres => format!("${n}"),
        Dialect::Mysql | Dialect::Sqlite => "?".to_string(),
    };
    if d == Dialect::Postgres {
        // Text binds into a typed column need an explicit cast on Postgres.
        // A bare `$n::uuid` makes Postgres infer the *parameter* as uuid, which
        // then refuses the text we actually send. `$n::text::uuid` pins the
        // parameter to text and casts server-side.
        return match ty {
            ColType::Uuid => format!("{base}::text::uuid"),
            ColType::Date => format!("{base}::text::date"),
            ColType::DateTime => format!("{base}::text::timestamptz"),
            ColType::Decimal => format!("{base}::text::numeric"),
            ColType::Json => format!("{base}::jsonb"),
            // Binds are i64/f64, but the column may be int2/int4/float4. Cast
            // so the driver's type matches and Postgres does the
            // (assignment-legal) narrowing.
            ColType::Int => format!("{base}::bigint"),
            ColType::Float => format!("{base}::float8"),
            _ => base,
        };
    }
    if ty == ColType::Json {
        return match d {
            // SQLite has no JSON type. `CAST(? AS JSON)` would give the value
            // NUMERIC affinity and mangle the document; `json()` validates the
            // text and stores it as JSON the json1 functions can read.
            Dialect::Sqlite => format!("json({base})"),
            _ => format!("CAST({base} AS JSON)"),
        };
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
    append_limit_offset_cols(d, sql, params, q)
}

/// The `LIMIT` / `OFFSET` tail, without the `ORDER BY`. Shared with the grouped
/// compiler, which resolves its ordering against group keys and aggregate
/// aliases rather than against table columns.
fn append_limit_offset_cols(
    d: Dialect,
    sql: &mut String,
    params: &mut Vec<SqlBind>,
    q: &ColumnQuery,
) -> Result<(), String> {
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
        // MySQL requires a LIMIT before OFFSET, so it needs a ceiling; Postgres
        // takes a bare OFFSET, and SQLite reads `LIMIT -1` as "no limit". None
        // of the three needs an invented row cap.
        params.push(SqlBind::I64(offset as i64));
        let off = placeholder(d, params.len(), ColType::Int);
        match d {
            Dialect::Postgres => sql.push_str(&format!(" OFFSET {off}")),
            Dialect::Mysql => sql.push_str(&format!(" LIMIT 18446744073709551615 OFFSET {off}")),
            Dialect::Sqlite => sql.push_str(&format!(" LIMIT -1 OFFSET {off}")),
        }
    }
    Ok(())
}

/// `SELECT <group cols>, <aggregates> FROM t [WHERE …] GROUP BY <group cols>`.
///
/// Aggregates come back as text for the same reason scalar ones do: the exact
/// numeric width is not knowable from the schema, so Rust parses one canonical
/// form.
pub fn compile_group_by_cols(
    d: Dialect,
    q: &ColumnQuery,
    group_fields: &[String],
    aggs: &[super::sql_compile::GroupAgg],
) -> Result<CompiledSql, String> {
    if group_fields.is_empty() {
        return Err("group_by needs at least one column".to_string());
    }
    let mut selected = Vec::with_capacity(group_fields.len() + aggs.len().max(1));
    let mut grouped = Vec::with_capacity(group_fields.len());
    // Every entry carries `AS "<result name>"`. The readers index positionally,
    // so the aliases are not needed for that — they exist so `ORDER BY` can name
    // a group key or an aggregate, which is how the document path orders too.
    // Without them `.order("total", …)` compiled to `ORDER BY "total"` against a
    // SELECT list that had no such output column.
    for field in group_fields {
        let col = resolve_col(&q.schema, field)?;
        let quoted = d.quote_ident(&col.name)?;
        let alias = d.quote_ident(field)?;
        // Group keys are read as text so every dialect returns one shape.
        selected.push(match d {
            Dialect::Postgres => format!("{quoted}::text AS {alias}"),
            Dialect::Mysql => format!("CAST({quoted} AS CHAR) AS {alias}"),
            Dialect::Sqlite => format!("CAST({quoted} AS TEXT) AS {alias}"),
        });
        grouped.push(quoted);
    }
    // alias -> the un-cast expression behind it, so `ORDER BY` can sort on the
    // number rather than on its text rendering (see the ordering block below).
    let mut agg_exprs: Vec<(String, String)> = Vec::new();
    if aggs.is_empty() {
        // The default aggregate is a row count, aliased `n` like the document path.
        let alias = d.quote_ident("n")?;
        selected.push(match d {
            Dialect::Postgres => format!("COUNT(*)::text AS {alias}"),
            Dialect::Mysql => format!("CAST(COUNT(*) AS CHAR) AS {alias}"),
            Dialect::Sqlite => format!("CAST(COUNT(*) AS TEXT) AS {alias}"),
        });
        agg_exprs.push(("n".to_string(), "COUNT(*)".to_string()));
    } else {
        for agg in aggs {
            let expr = if matches!(agg.func, SqlAgg::Count) {
                "COUNT(*)".to_string()
            } else {
                let col = resolve_col(&q.schema, &agg.field)?;
                let name = match agg.func {
                    SqlAgg::Sum => "SUM",
                    SqlAgg::Avg => "AVG",
                    SqlAgg::Min => "MIN",
                    SqlAgg::Max => "MAX",
                    SqlAgg::Count => "COUNT",
                };
                if !col.ty.is_numeric() {
                    return Err(format!(
                        "column {:?} is {} — {name} needs a numeric column",
                        col.name,
                        col.ty.as_str()
                    ));
                }
                format!("{name}({})", d.quote_ident(&col.name)?)
            };
            let alias = d.quote_ident(&agg.alias)?;
            selected.push(match d {
                Dialect::Postgres => format!("({expr})::text AS {alias}"),
                Dialect::Mysql => format!("CAST({expr} AS CHAR) AS {alias}"),
                Dialect::Sqlite => format!("CAST({expr} AS TEXT) AS {alias}"),
            });
            agg_exprs.push((agg.alias.clone(), expr));
        }
    }

    let table = d.quote_ident(&q.schema.table)?;
    let mut params = Vec::new();
    let mut sql = format!("SELECT {} FROM {table}", selected.join(", "));
    sql.push_str(&compile_where(d, q, &mut params)?);
    sql.push_str(&format!(" GROUP BY {}", grouped.join(", ")));
    if let Some(having) = &q.having {
        sql.push_str(&compile_having_cols(
            d,
            having,
            group_fields,
            aggs,
            &q.schema,
        )?);
    }
    // `.order`, `.limit` and `.offset` were silently dropped here, while the
    // document-mode twin honours all three — so the same grouped query returned
    // unordered, unbounded rows on a `table "…"` model and a paginated,
    // sorted set on a document one. Ordering resolves against the group keys and
    // aggregate aliases (what the SELECT list actually holds), not against
    // arbitrary table columns, which `GROUP BY` would reject anyway.
    //
    // Order on the *expression*, never on the alias. The SELECT list casts
    // every group key and aggregate to text so all three dialects return one
    // shape, and a bare name in `ORDER BY` binds to the output column — so
    // `ORDER BY "n"` would sort those texts lexicographically and rank a count
    // of 9 above 100. A table-qualified column and a spelled-out aggregate
    // both bind to the input instead, and sort numerically.
    if let Some(field) = &q.order_field {
        let dir = if q.order_desc { "DESC" } else { "ASC" };
        if let Some(group_field) = group_fields.iter().find(|g| *g == field) {
            let col = resolve_col(&q.schema, group_field)?;
            let quoted = d.quote_ident(&col.name)?;
            sql.push_str(&format!(" ORDER BY {table}.{quoted} {dir}"));
        } else if let Some((_, expr)) = agg_exprs.iter().find(|(alias, _)| alias == field) {
            sql.push_str(&format!(" ORDER BY {expr} {dir}"));
        } else {
            return Err(format!(
                "`.order({field:?})` on a grouped query must name one of this query's \
                 group keys or aggregate aliases ({}).",
                group_fields
                    .iter()
                    .cloned()
                    .chain(aggs.iter().map(|a| a.alias.clone()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    append_limit_offset_cols(d, &mut sql, &mut params, q)?;
    Ok(CompiledSql { sql, params })
}

fn compile_having_cols(
    d: Dialect,
    having: &str,
    group_fields: &[String],
    aggs: &[GroupAgg],
    schema: &TableSchema,
) -> Result<String, String> {
    let refuse = || {
        format!(
            "`.having({having:?})` on a SQL adapter supports one comparison of a \
             group key or aggregate alias against a number, e.g. \
             \"n > 5\" or \"total >= 100\"."
        )
    };
    let text = having.trim();
    let (alias, op, value) = ["!=", ">=", "<=", "==", ">", "<", "="]
        .iter()
        .find_map(|op| {
            text.split_once(op)
                .map(|(left, right)| (left.trim(), *op, right.trim()))
        })
        .ok_or_else(refuse)?;
    let known = group_fields.iter().any(|f| f == alias)
        || aggs.iter().any(|a| a.alias == alias)
        || (aggs.is_empty() && alias == "n");
    if !known {
        return Err(format!(
            "`.having({having:?})`: {alias:?} is not one of this query's group keys \
             or aggregate aliases."
        ));
    }
    if value.parse::<f64>().is_err() {
        return Err(refuse());
    }
    let sql_op = match op {
        "==" | "=" => "=",
        other => other,
    };
    let left = if let Some(agg) = aggs.iter().find(|a| a.alias == alias) {
        if matches!(agg.func, SqlAgg::Count) {
            "COUNT(*)".to_string()
        } else {
            let col = resolve_col(schema, &agg.field)?;
            let name = match agg.func {
                SqlAgg::Sum => "SUM",
                SqlAgg::Avg => "AVG",
                SqlAgg::Min => "MIN",
                SqlAgg::Max => "MAX",
                SqlAgg::Count => "COUNT",
            };
            format!("{name}({})", d.quote_ident(&col.name)?)
        }
    } else if aggs.is_empty() && alias == "n" {
        "COUNT(*)".to_string()
    } else {
        d.quote_ident(alias)?
    };
    Ok(format!(" HAVING {left} {sql_op} {value}"))
}

/// `DELETE FROM t [WHERE …]`.
pub fn compile_delete_all_cols(d: Dialect, q: &ColumnQuery) -> Result<CompiledSql, String> {
    let table = d.quote_ident(&q.schema.table)?;
    let mut params = Vec::new();
    let mut sql = format!("DELETE FROM {table}");
    sql.push_str(&compile_where(d, q, &mut params)?);
    Ok(CompiledSql { sql, params })
}

/// `UPDATE t SET col = ?, … [WHERE …]` — the bulk form, which skips validations
/// and callbacks exactly like the document path's `update_all`.
pub fn compile_update_all_cols(
    d: Dialect,
    q: &ColumnQuery,
    patch: &serde_json::Value,
) -> Result<CompiledSql, String> {
    let obj = patch
        .as_object()
        .ok_or_else(|| "update_all expects a hash of fields".to_string())?;
    let mut assignments = Vec::new();
    let mut params = Vec::new();
    for col in &q.schema.columns {
        // The primary key identifies the row; a bulk write must not rewrite it.
        if col.name == q.schema.pk {
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
            "update_all on {:?} had no writable fields. Columns: {}",
            q.schema.table,
            q.schema.column_names().join(", ")
        ));
    }
    let table = d.quote_ident(&q.schema.table)?;
    let mut sql = format!("UPDATE {table} SET {}", assignments.join(", "));
    // `compile_where` appends to the same params vec, so its placeholders
    // continue after the SET binds — which is what Postgres's $n requires.
    sql.push_str(&compile_where(d, q, &mut params)?);
    Ok(CompiledSql { sql, params })
}

/// `UPDATE t SET col = COALESCE(col,0) + ? WHERE pk = ?`, returning the new
/// value where the dialect supports `RETURNING`.
///
/// Doing the arithmetic in SQL is what makes a counter safe under concurrency:
/// the row lock serializes the increments instead of two readers both seeing the
/// old value.
pub fn compile_increment_col(
    d: Dialect,
    schema: &TableSchema,
    pk_value: &serde_json::Value,
    column: &str,
    delta: i64,
) -> Result<(String, Vec<SqlBind>), String> {
    let col = resolve_col(schema, column)?;
    if !col.ty.is_numeric() {
        return Err(format!(
            "column {:?} is {} — increment/decrement (and counter caches) need a \
             numeric column",
            col.name,
            col.ty.as_str()
        ));
    }
    let pk_col = schema
        .column(&schema.pk)
        .ok_or_else(|| format!("table {:?} lost its primary key column", schema.table))?;
    let pk_bind = bind_for_column(pk_col, pk_value)?
        .ok_or_else(|| "increment requires a primary-key value".to_string())?;

    let mut params = vec![SqlBind::I64(delta)];
    let delta_ph = placeholder(d, 1, ColType::Int);
    params.push(pk_bind);
    let pk_ph = placeholder(d, 2, pk_col.ty);

    let table = d.quote_ident(&schema.table)?;
    let quoted = d.quote_ident(&col.name)?;
    let pk_quoted = d.quote_ident(&schema.pk)?;
    let mut sql = format!(
        "UPDATE {table} SET {quoted} = COALESCE({quoted}, 0) + {delta_ph} \
         WHERE {pk_quoted} = {pk_ph}"
    );
    if matches!(d, Dialect::Postgres | Dialect::Sqlite) {
        // Cast to text for the same reason reads do: the driver never has to
        // match the column's exact numeric width.
        sql.push_str(&format!(
            " RETURNING {}",
            match d {
                Dialect::Postgres => format!("{quoted}::text"),
                _ => format!("CAST({quoted} AS TEXT)"),
            }
        ));
    }
    Ok((sql, params))
}

/// `SELECT col FROM t WHERE pk = ?` as text — the read-back MySQL needs after an
/// increment, since it has no `RETURNING`.
pub fn compile_read_column(
    d: Dialect,
    schema: &TableSchema,
    pk_value: &serde_json::Value,
    column: &str,
) -> Result<CompiledSql, String> {
    let col = resolve_col(schema, column)?;
    let pk_col = schema
        .column(&schema.pk)
        .ok_or_else(|| format!("table {:?} lost its primary key column", schema.table))?;
    let pk_bind = bind_for_column(pk_col, pk_value)?
        .ok_or_else(|| "reading a column requires a primary-key value".to_string())?;
    let params = vec![pk_bind];
    let pk_ph = placeholder(d, 1, pk_col.ty);
    let quoted = d.quote_ident(&col.name)?;
    let expr = match d {
        Dialect::Postgres => format!("{quoted}::text"),
        Dialect::Mysql => format!("CAST({quoted} AS CHAR)"),
        Dialect::Sqlite => format!("CAST({quoted} AS TEXT)"),
    };
    Ok(CompiledSql {
        sql: format!(
            "SELECT {expr} FROM {} WHERE {} = {pk_ph}",
            d.quote_ident(&schema.table)?,
            d.quote_ident(&schema.pk)?
        ),
        params,
    })
}

/// `SELECT <columns> FROM t [WHERE …] [ORDER BY …] [LIMIT …]`
pub fn compile_select_cols(d: Dialect, q: &ColumnQuery) -> Result<CompiledSql, String> {
    let table = d.quote_ident(&q.schema.table)?;
    // A projection fetches only what was asked for: on a wide table that is the
    // difference between reading two columns and reading fifty.
    let cols = select_list_for(
        d,
        &q.schema,
        &selected_columns(&q.schema, &q.select_fields)?,
    )?;
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
        Dialect::Mysql | Dialect::Sqlite => "COUNT(*)",
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
            Dialect::Mysql | Dialect::Sqlite => "COUNT(*)".to_string(),
        },
    };
    // Aggregates come back as text so an exact numeric keeps its precision
    // until Soli parses it.
    let expr = match d {
        Dialect::Postgres if !matches!(func, SqlAgg::Count) => format!("({expr})::text"),
        Dialect::Mysql if !matches!(func, SqlAgg::Count) => format!("CAST({expr} AS CHAR)"),
        Dialect::Sqlite if !matches!(func, SqlAgg::Count) => format!("CAST({expr} AS TEXT)"),
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
    // Postgres and SQLite both support RETURNING; MySQL reads the row back.
    if matches!(d, Dialect::Postgres | Dialect::Sqlite) {
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
    // Postgres and SQLite both support RETURNING; MySQL reads the row back.
    if matches!(d, Dialect::Postgres | Dialect::Sqlite) {
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
    fn in_filters_batch_and_handle_null() {
        // `IN` is what makes eager loading one query per association instead of
        // one per parent row.
        let mut q = query();
        q.in_filters.insert(
            "qty".into(),
            vec![serde_json::json!(1), serde_json::json!(2)],
        );
        let c = compile_select_cols(Dialect::Postgres, &q).unwrap();
        assert!(
            c.sql.contains("\"qty\" IN ($1::bigint, $2::bigint)"),
            "{}",
            c.sql
        );
        assert_eq!(c.params.len(), 2);

        // A null in the list means "or IS NULL": SQL `IN` never matches NULL.
        let mut q = query();
        q.in_filters.insert(
            "qty".into(),
            vec![serde_json::json!(1), serde_json::Value::Null],
        );
        let c = compile_select_cols(Dialect::Sqlite, &q).unwrap();
        assert!(
            c.sql.contains("(\"qty\" IN (?) OR \"qty\" IS NULL)"),
            "{}",
            c.sql
        );

        // An empty list matches nothing — `IN ()` is a syntax error everywhere.
        let mut q = query();
        q.in_filters.insert("qty".into(), vec![]);
        let c = compile_select_cols(Dialect::Mysql, &q).unwrap();
        assert!(c.sql.contains("1 = 0"), "{}", c.sql);
        assert!(c.params.is_empty());

        // An unknown column is refused before any SQL is built.
        let mut q = query();
        q.in_filters
            .insert("nope".into(), vec![serde_json::json!(1)]);
        assert!(compile_select_cols(Dialect::Postgres, &q).is_err());
    }

    #[test]
    fn select_names_real_columns_and_skips_unreadable_ones() {
        let c = compile_select_cols(Dialect::Postgres, &query()).unwrap();
        // Numeric and temporal columns are read as text (the exact SQL width is
        // not knowable from information_schema, and the driver refuses
        // int4 -> i64) and parsed in Rust; plain text columns select directly.
        assert!(
            c.sql
                .starts_with("SELECT \"id\"::text AS \"id\", \"name\", \"qty\"::text AS \"qty\""),
            "{}",
            c.sql
        );
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
        // LIMIT/OFFSET binds are i64, cast like any other integer bind.
        assert!(
            c.sql.contains("LIMIT $1::bigint OFFSET $2::bigint"),
            "{}",
            c.sql
        );
        assert_eq!(c.params, vec![SqlBind::I64(10), SqlBind::I64(20)]);
    }

    #[test]
    fn offset_without_limit_uses_a_bare_offset_on_postgres() {
        // The document compiler needs a magic LIMIT here; Postgres supports a
        // bare OFFSET, so no invented ceiling.
        let mut q = query();
        q.offset = Some(5);
        let pg = compile_select_cols(Dialect::Postgres, &q).unwrap();
        assert!(pg.sql.contains("OFFSET $1::bigint"), "{}", pg.sql);
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
    fn hash_filter_compiles_comparisons_in_like_and_or() {
        use crate::db::hash_filter::HashFilter;
        let mut q = query();
        q.hash_filter = Some(
            HashFilter::from_json_map(
                serde_json::json!({
                    "qty": { "gte": 5 },
                    "name": { "like": "INV%" },
                    "or": [{ "active": true }, { "qty": 0 }]
                })
                .as_object()
                .unwrap(),
                "where",
            )
            .unwrap(),
        );
        let c = compile_select_cols(Dialect::Sqlite, &q).unwrap();
        assert!(c.sql.contains("\"qty\" >= ?"), "{}", c.sql);
        assert!(c.sql.contains("\"name\" LIKE ?"), "{}", c.sql);
        assert!(c.sql.contains(" OR "), "{}", c.sql);
        assert!(!c.params.is_empty());

        let mut q = query();
        q.hash_filter = Some(HashFilter::In {
            field: "qty".into(),
            values: vec![serde_json::json!(1), serde_json::json!(2)],
        });
        let c = compile_select_cols(Dialect::Postgres, &q).unwrap();
        assert!(c.sql.contains("\"qty\" IN ("), "{}", c.sql);
    }

    #[test]
    fn having_compiles_against_count_or_named_aggregate() {
        let mut q = query();
        q.having = Some("n > 5".into());
        let c = compile_group_by_cols(Dialect::Sqlite, &q, &["name".into()], &[]).unwrap();
        assert!(c.sql.contains("HAVING COUNT(*) > 5"), "{}", c.sql);

        q.having = Some("total >= 100".into());
        let aggs = [GroupAgg {
            alias: "total".into(),
            func: SqlAgg::Sum,
            field: "qty".into(),
        }];
        let c = compile_group_by_cols(Dialect::Postgres, &q, &["name".into()], &aggs).unwrap();
        assert!(c.sql.contains("HAVING SUM(\"qty\") >= 100"), "{}", c.sql);

        q.having = Some("nope > 1".into());
        let err = compile_group_by_cols(Dialect::Sqlite, &q, &["name".into()], &[]).unwrap_err();
        assert!(err.contains("nope"), "{err}");
    }

    #[test]
    fn grouped_order_by_sorts_on_the_expression_not_the_text_alias() {
        // The SELECT list casts everything to text, and a bare name in ORDER BY
        // binds to the output column — so ordering by the alias would compare
        // "9" > "100". Aggregates must sort on the aggregate expression and
        // group keys on the table-qualified column.
        let mut q = query();
        q.order_field = Some("n".into());
        q.order_desc = true;
        for d in [Dialect::Postgres, Dialect::Mysql, Dialect::Sqlite] {
            let c = compile_group_by_cols(d, &q, &["name".into()], &[]).unwrap();
            assert!(c.sql.contains("ORDER BY COUNT(*) DESC"), "{d:?}: {}", c.sql);
        }

        let aggs = [GroupAgg {
            alias: "total".into(),
            func: SqlAgg::Sum,
            field: "qty".into(),
        }];
        q.order_field = Some("total".into());
        let c = compile_group_by_cols(Dialect::Postgres, &q, &["name".into()], &aggs).unwrap();
        assert!(c.sql.contains("ORDER BY SUM(\"qty\") DESC"), "{}", c.sql);

        // A group key orders on the input column, qualified so it cannot bind
        // to the same-named text alias in the SELECT list.
        q.order_field = Some("name".into());
        q.order_desc = false;
        let c = compile_group_by_cols(Dialect::Postgres, &q, &["name".into()], &aggs).unwrap();
        assert!(
            c.sql.contains("ORDER BY \"orders\".\"name\" ASC"),
            "{}",
            c.sql
        );

        // An unknown field is still a clear error, not a silent drop.
        q.order_field = Some("nope".into());
        let err = compile_group_by_cols(Dialect::Sqlite, &q, &["name".into()], &aggs).unwrap_err();
        assert!(err.contains("nope"), "{err}");
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
        // Parameters are pinned to text, then cast server-side.
        assert!(c.sql.contains("::text::timestamptz"), "{}", c.sql);
        assert!(c.sql.contains("::text::numeric"), "{}", c.sql);
        assert!(c.sql.contains("::jsonb"), "{}", c.sql);
        // An exact numeric is sent as text so no scale is lost.
        assert!(
            c.params.contains(&SqlBind::Text("19.99".into())),
            "{:?}",
            c.params
        );
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
