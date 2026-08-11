//! Compile QueryBuilder-shaped constraints into parameterized SQL.
//!
//! Supports PostgreSQL and MySQL dialects. Only hash-style equality filters
//! (`doc.field == @field AND …`) are portable; raw SDBQL is rejected.

use std::collections::{BTreeMap, HashMap};

/// SQL dialect for identifier quoting, placeholders, and JSON operators.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialect {
    Postgres,
    Mysql,
}

/// Soft-delete handling for SQL emission (mirrors model SoftDeleteMode).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoftDeleteMode {
    Default,
    WithDeleted,
    OnlyDeleted,
}

/// Portable list query IR for the SQL compiler.
#[derive(Clone, Debug)]
pub struct ListQuery {
    pub table: String,
    pub eq_filters: BTreeMap<String, serde_json::Value>,
    pub filter_sdbql: Option<String>,
    pub soft_delete: SoftDeleteMode,
    pub is_soft_delete_model: bool,
    pub order_field: Option<String>,
    pub order_desc: bool,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

/// Inputs for [`list_query_from_parts`] (dialect-agnostic; lives here so SQL
/// backends can be feature-gated independently).
pub struct ListQueryParts {
    pub table: String,
    pub filter_sdbql: Option<String>,
    pub bind_vars: HashMap<String, serde_json::Value>,
    pub soft_delete: SoftDeleteMode,
    pub is_soft_delete_model: bool,
    pub order_field: Option<String>,
    pub order_desc: bool,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

/// Build a ListQuery IR (dialect-agnostic filters); validates with `dialect`.
pub fn list_query_from_parts(parts: ListQueryParts, dialect: Dialect) -> Result<ListQuery, String> {
    let mut eq_filters = BTreeMap::new();
    if let Some(ref f) = parts.filter_sdbql {
        if !f.trim().is_empty() {
            for (k, v) in &parts.bind_vars {
                if k.starts_with("__soli_") {
                    continue;
                }
                eq_filters.insert(k.clone(), v.clone());
            }
        }
    }
    let q = ListQuery {
        table: parts.table,
        eq_filters,
        filter_sdbql: parts.filter_sdbql,
        soft_delete: parts.soft_delete,
        is_soft_delete_model: parts.is_soft_delete_model,
        order_field: parts.order_field,
        order_desc: parts.order_desc,
        limit: parts.limit,
        offset: parts.offset,
    };
    let _ = compile_select_d(dialect, &q)?;
    Ok(q)
}

/// A bind parameter with explicit SQL type intent.
#[derive(Clone, Debug, PartialEq)]
pub enum SqlBind {
    Json(serde_json::Value),
    I64(i64),
    Text(String),
}

/// Compiled SQL + positional bind parameters.
#[derive(Clone, Debug, PartialEq)]
pub struct CompiledSql {
    pub sql: String,
    pub params: Vec<SqlBind>,
}

/// Scalar aggregate supported on SQL backends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SqlAgg {
    Sum,
    Avg,
    Min,
    Max,
    Count,
}

impl Dialect {
    fn ph(self, n: usize) -> String {
        match self {
            Dialect::Postgres => format!("${n}"),
            Dialect::Mysql => "?".to_string(),
        }
    }

    pub fn quote_ident(self, name: &str) -> Result<String, String> {
        if name.is_empty()
            || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            || name.chars().next().is_some_and(|c| c.is_ascii_digit())
        {
            return Err(format!("invalid SQL identifier: {name:?}"));
        }
        Ok(match self {
            Dialect::Postgres => format!("\"{}\"", name.replace('"', "\"\"")),
            Dialect::Mysql => format!("`{}`", name.replace('`', "``")),
        })
    }

    fn json_eq(self, field: &str, ph: &str) -> String {
        match self {
            Dialect::Postgres => format!("(doc->'{field}') = {ph}"),
            Dialect::Mysql => format!("JSON_EXTRACT(doc, '$.{field}') = CAST({ph} AS JSON)"),
        }
    }

    fn json_order(self, field: &str) -> String {
        match self {
            Dialect::Postgres => format!("doc->>'{field}'"),
            Dialect::Mysql => format!("JSON_UNQUOTE(JSON_EXTRACT(doc, '$.{field}'))"),
        }
    }

    fn soft_delete_null(self) -> &'static str {
        match self {
            Dialect::Postgres => "(doc->>'deleted_at') IS NULL",
            Dialect::Mysql => "JSON_EXTRACT(doc, '$.deleted_at') IS NULL",
        }
    }

    fn soft_delete_not_null(self) -> &'static str {
        match self {
            Dialect::Postgres => "(doc->>'deleted_at') IS NOT NULL",
            Dialect::Mysql => "JSON_EXTRACT(doc, '$.deleted_at') IS NOT NULL",
        }
    }

    fn count_expr(self) -> &'static str {
        match self {
            Dialect::Postgres => "COUNT(*)::bigint",
            Dialect::Mysql => "COUNT(*)",
        }
    }

    /// Numeric extract for SUM/AVG/MIN/MAX over a JSON field.
    fn json_num(self, field: &str) -> String {
        match self {
            Dialect::Postgres => format!("(doc->>'{field}')::float8"),
            Dialect::Mysql => {
                format!("CAST(JSON_UNQUOTE(JSON_EXTRACT(doc, '$.{field}')) AS DECIMAL(30,10))")
            }
        }
    }

    /// Text extract of a JSON field (for IN-lists and GROUP BY keys).
    fn json_text(self, field: &str) -> String {
        match self {
            Dialect::Postgres => format!("(doc->>'{field}')"),
            Dialect::Mysql => format!("JSON_UNQUOTE(JSON_EXTRACT(doc, '$.{field}'))"),
        }
    }
}

/// Reject string/raw SDBQL that is not the hash-equality shape.
pub fn assert_portable_filter(
    filter_sdbql: Option<&str>,
    eq_fields: &BTreeMap<String, serde_json::Value>,
) -> Result<(), String> {
    let Some(raw) = filter_sdbql.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(());
    };
    if is_hash_equality_sdbql(raw, eq_fields) {
        return Ok(());
    }
    Err("String/raw SDBQL `.where(\"…\")` is SoliDB-only. \
         On SQL adapters use the hash form: `.where({ \"field\": value })`. \
         See docs/sql-adapter-design.md."
        .to_string())
}

fn is_hash_equality_sdbql(filter: &str, binds: &BTreeMap<String, serde_json::Value>) -> bool {
    let normalized = filter
        .replace("&&", " AND ")
        .replace(" and ", " AND ")
        .replace(" And ", " AND ");
    let clauses: Vec<&str> = normalized
        .split(" AND ")
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .collect();
    if clauses.is_empty() {
        return false;
    }
    for clause in clauses {
        let clause = clause.trim();
        let Some((left, right)) = clause.split_once("==") else {
            return false;
        };
        let left = left.trim();
        let right = right.trim();
        let Some(field) = left.strip_prefix("doc.") else {
            return false;
        };
        let Some(bind) = right.strip_prefix('@') else {
            return false;
        };
        if field != bind || !binds.contains_key(field) {
            return false;
        }
        if !field.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return false;
        }
    }
    true
}

/// Quote identifier (defaults to Postgres for callers that still use the old API).
pub fn quote_ident(name: &str) -> Result<String, String> {
    Dialect::Postgres.quote_ident(name)
}

pub fn compile_select(q: &ListQuery) -> Result<CompiledSql, String> {
    compile_select_d(Dialect::Postgres, q)
}

pub fn compile_select_d(d: Dialect, q: &ListQuery) -> Result<CompiledSql, String> {
    assert_portable_filter(q.filter_sdbql.as_deref(), &q.eq_filters)?;
    let table = d.quote_ident(&q.table)?;
    let mut sql = format!("SELECT doc FROM {table}");
    let (where_sql, mut params) = compile_where(d, q)?;
    if !where_sql.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&where_sql);
    }
    if let Some(field) = &q.order_field {
        validate_field(field)?;
        let dir = if q.order_desc { "DESC" } else { "ASC" };
        if field == "_key" || field == "id" {
            sql.push_str(&format!(" ORDER BY _key {dir}"));
        } else {
            sql.push_str(&format!(" ORDER BY {} {dir}", d.json_order(field)));
        }
    }
    append_limit_offset(d, &mut sql, &mut params, q.limit, q.offset);
    Ok(CompiledSql { sql, params })
}

pub fn compile_count(q: &ListQuery) -> Result<CompiledSql, String> {
    compile_count_d(Dialect::Postgres, q)
}

pub fn compile_count_d(d: Dialect, q: &ListQuery) -> Result<CompiledSql, String> {
    assert_portable_filter(q.filter_sdbql.as_deref(), &q.eq_filters)?;
    let table = d.quote_ident(&q.table)?;
    let mut sql = format!("SELECT {} FROM {table}", d.count_expr());
    let (where_sql, params) = compile_where(d, q)?;
    if !where_sql.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&where_sql);
    }
    Ok(CompiledSql { sql, params })
}

pub fn compile_exists(q: &ListQuery) -> Result<CompiledSql, String> {
    compile_exists_d(Dialect::Postgres, q)
}

pub fn compile_exists_d(d: Dialect, q: &ListQuery) -> Result<CompiledSql, String> {
    let mut q = q.clone();
    q.limit = Some(1);
    q.offset = None;
    q.order_field = None;
    let mut compiled = compile_select_d(d, &q)?;
    compiled.sql = compiled.sql.replacen("SELECT doc FROM", "SELECT 1 FROM", 1);
    Ok(compiled)
}

pub fn compile_aggregate_d(
    d: Dialect,
    q: &ListQuery,
    func: SqlAgg,
    field: &str,
) -> Result<CompiledSql, String> {
    assert_portable_filter(q.filter_sdbql.as_deref(), &q.eq_filters)?;
    if !matches!(func, SqlAgg::Count) {
        validate_field(field)?;
    }
    let table = d.quote_ident(&q.table)?;
    let expr = match func {
        SqlAgg::Count => d.count_expr().to_string(),
        SqlAgg::Sum => format!("SUM({})", d.json_num(field)),
        SqlAgg::Avg => format!("AVG({})", d.json_num(field)),
        SqlAgg::Min => format!("MIN({})", d.json_num(field)),
        SqlAgg::Max => format!("MAX({})", d.json_num(field)),
    };
    let mut sql = format!("SELECT {expr} FROM {table}");
    let (where_sql, params) = compile_where(d, q)?;
    if !where_sql.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&where_sql);
    }
    Ok(CompiledSql { sql, params })
}

pub fn compile_delete_all_d(d: Dialect, q: &ListQuery) -> Result<CompiledSql, String> {
    assert_portable_filter(q.filter_sdbql.as_deref(), &q.eq_filters)?;
    let table = d.quote_ident(&q.table)?;
    let mut sql = format!("DELETE FROM {table}");
    let (where_sql, params) = compile_where(d, q)?;
    if !where_sql.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&where_sql);
    }
    Ok(CompiledSql { sql, params })
}

/// `SELECT doc FROM t WHERE _key IN (…)` — used for includes batching.
pub fn compile_select_by_keys_d(
    d: Dialect,
    table: &str,
    keys: &[String],
) -> Result<CompiledSql, String> {
    if keys.is_empty() {
        return Ok(CompiledSql {
            sql: String::new(),
            params: Vec::new(),
        });
    }
    let table_q = d.quote_ident(table)?;
    let mut params = Vec::new();
    let mut phs = Vec::new();
    for (i, k) in keys.iter().enumerate() {
        params.push(SqlBind::Text(k.clone()));
        phs.push(d.ph(i + 1));
    }
    let sql = format!(
        "SELECT doc FROM {table_q} WHERE _key IN ({})",
        phs.join(", ")
    );
    Ok(CompiledSql { sql, params })
}

/// `SELECT doc FROM t WHERE doc.field IN (…)` as text equality — has_many/has_one batching.
pub fn compile_select_json_text_in_d(
    d: Dialect,
    table: &str,
    field: &str,
    values: &[String],
) -> Result<CompiledSql, String> {
    validate_field(field)?;
    if values.is_empty() {
        return Ok(CompiledSql {
            sql: String::new(),
            params: Vec::new(),
        });
    }
    let table_q = d.quote_ident(table)?;
    let mut params = Vec::new();
    let mut phs = Vec::new();
    for (i, v) in values.iter().enumerate() {
        params.push(SqlBind::Text(v.clone()));
        phs.push(d.ph(i + 1));
    }
    let col = d.json_text(field);
    let sql = format!(
        "SELECT doc FROM {table_q} WHERE {col} IN ({})",
        phs.join(", ")
    );
    Ok(CompiledSql { sql, params })
}

/// One aggregate column of a multi-row GROUP BY.
#[derive(Clone, Debug)]
pub struct GroupAgg {
    pub alias: String,
    pub func: SqlAgg,
    pub field: String,
}

/// `SELECT group_cols…, aggregates… FROM t WHERE … GROUP BY group_cols`.
pub fn compile_group_by_d(
    d: Dialect,
    q: &ListQuery,
    group_fields: &[String],
    aggs: &[GroupAgg],
) -> Result<CompiledSql, String> {
    assert_portable_filter(q.filter_sdbql.as_deref(), &q.eq_filters)?;
    if group_fields.is_empty() {
        return Err("group_by requires at least one field".into());
    }
    for f in group_fields {
        validate_field(f)?;
    }
    let table = d.quote_ident(&q.table)?;
    let mut select_parts = Vec::new();
    let mut group_parts = Vec::new();
    for f in group_fields {
        let expr = if f == "_key" || f == "id" {
            "_key".to_string()
        } else {
            d.json_text(f)
        };
        // Alias group key with the field name so rows are self-describing.
        select_parts.push(format!("{expr} AS {}", d.quote_ident(f)?));
        group_parts.push(expr);
    }
    if aggs.is_empty() {
        select_parts.push(format!("{} AS {}", d.count_expr(), d.quote_ident("n")?));
    } else {
        for a in aggs {
            validate_field(&a.alias)?;
            let expr = match a.func {
                SqlAgg::Count => d.count_expr().to_string(),
                SqlAgg::Sum => {
                    validate_field(&a.field)?;
                    format!("SUM({})", d.json_num(&a.field))
                }
                SqlAgg::Avg => {
                    validate_field(&a.field)?;
                    format!("AVG({})", d.json_num(&a.field))
                }
                SqlAgg::Min => {
                    validate_field(&a.field)?;
                    format!("MIN({})", d.json_num(&a.field))
                }
                SqlAgg::Max => {
                    validate_field(&a.field)?;
                    format!("MAX({})", d.json_num(&a.field))
                }
            };
            select_parts.push(format!("{expr} AS {}", d.quote_ident(&a.alias)?));
        }
    }
    let mut sql = format!("SELECT {} FROM {table}", select_parts.join(", "));
    let (where_sql, mut params) = compile_where(d, q)?;
    if !where_sql.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&where_sql);
    }
    sql.push_str(&format!(" GROUP BY {}", group_parts.join(", ")));
    // Optional ORDER BY first group field for stable output.
    if let Some(field) = &q.order_field {
        validate_field(field)?;
        if group_fields.iter().any(|g| g == field)
            || aggs.iter().any(|a| a.alias == *field)
            || (aggs.is_empty() && field == "n")
        {
            let dir = if q.order_desc { "DESC" } else { "ASC" };
            let ord = if field == "_key" || field == "id" {
                "_key".to_string()
            } else if group_fields.iter().any(|g| g == field) {
                d.json_text(field)
            } else {
                d.quote_ident(field)?
            };
            sql.push_str(&format!(" ORDER BY {ord} {dir}"));
        }
    }
    append_limit_offset(d, &mut sql, &mut params, q.limit, q.offset);
    Ok(CompiledSql { sql, params })
}

/// Bulk patch: merge JSON into all matching rows.
pub fn compile_update_all_d(
    d: Dialect,
    q: &ListQuery,
    patch: &serde_json::Value,
) -> Result<CompiledSql, String> {
    assert_portable_filter(q.filter_sdbql.as_deref(), &q.eq_filters)?;
    let table = d.quote_ident(&q.table)?;
    let mut params = Vec::new();
    params.push(SqlBind::Json(patch.clone()));
    let patch_ph = d.ph(1);
    let set = match d {
        Dialect::Postgres => format!("doc = COALESCE(doc, '{{}}'::jsonb) || {patch_ph}"),
        Dialect::Mysql => {
            format!("doc = JSON_MERGE_PATCH(COALESCE(doc, '{{}}'), CAST({patch_ph} AS JSON))")
        }
    };
    let mut sql = format!("UPDATE {table} SET {set}");
    let (where_sql, where_params) = compile_where_offset(d, q, params.len())?;
    params.extend(where_params);
    if !where_sql.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&where_sql);
    }
    Ok(CompiledSql { sql, params })
}

fn append_limit_offset(
    d: Dialect,
    sql: &mut String,
    params: &mut Vec<SqlBind>,
    limit: Option<usize>,
    offset: Option<usize>,
) {
    if let Some(limit) = limit {
        params.push(SqlBind::I64(limit as i64));
        let n = params.len();
        let lim = d.ph(n);
        if let Some(offset) = offset {
            params.push(SqlBind::I64(offset as i64));
            let o = params.len();
            let off = d.ph(o);
            sql.push_str(&format!(" LIMIT {lim} OFFSET {off}"));
        } else {
            sql.push_str(&format!(" LIMIT {lim}"));
        }
    } else if let Some(offset) = offset {
        params.push(SqlBind::I64(1_000_000_i64));
        let n = params.len();
        let lim = d.ph(n);
        params.push(SqlBind::I64(offset as i64));
        let o = params.len();
        let off = d.ph(o);
        sql.push_str(&format!(" LIMIT {lim} OFFSET {off}"));
    }
}

fn compile_where(d: Dialect, q: &ListQuery) -> Result<(String, Vec<SqlBind>), String> {
    compile_where_offset(d, q, 0)
}

fn compile_where_offset(
    d: Dialect,
    q: &ListQuery,
    param_offset: usize,
) -> Result<(String, Vec<SqlBind>), String> {
    let mut parts = Vec::new();
    let mut params = Vec::new();
    for (field, value) in &q.eq_filters {
        validate_field(field)?;
        let n = param_offset + params.len() + 1;
        let ph = d.ph(n);
        if field == "_key" || field == "id" {
            let text = match value {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            params.push(SqlBind::Text(text));
            parts.push(format!("_key = {ph}"));
        } else {
            params.push(SqlBind::Json(value.clone()));
            parts.push(d.json_eq(field, &ph));
        }
    }
    if q.is_soft_delete_model {
        match q.soft_delete {
            SoftDeleteMode::Default => parts.push(d.soft_delete_null().to_string()),
            SoftDeleteMode::OnlyDeleted => parts.push(d.soft_delete_not_null().to_string()),
            SoftDeleteMode::WithDeleted => {}
        }
    }
    Ok((parts.join(" AND "), params))
}

fn validate_field(field: &str) -> Result<(), String> {
    if field.is_empty()
        || !field.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        || field.chars().next().is_some_and(|c| c.is_ascii_digit())
    {
        return Err(format!("invalid field name for SQL: {field:?}"));
    }
    Ok(())
}

pub fn create_table_sql(table: &str) -> Result<String, String> {
    create_table_sql_d(Dialect::Postgres, table)
}

pub fn create_table_sql_d(d: Dialect, table: &str) -> Result<String, String> {
    let t = d.quote_ident(table)?;
    let json_ty = match d {
        Dialect::Postgres => "JSONB",
        Dialect::Mysql => "JSON",
    };
    let key_ty = match d {
        Dialect::Postgres => "TEXT",
        Dialect::Mysql => "VARCHAR(255)",
    };
    Ok(format!(
        "CREATE TABLE IF NOT EXISTS {t} (\n\
         \t_key {key_ty} PRIMARY KEY,\n\
         \tdoc {json_ty} NOT NULL\n\
         )"
    ))
}

pub fn drop_table_sql(table: &str) -> Result<String, String> {
    drop_table_sql_d(Dialect::Postgres, table)
}

pub fn drop_table_sql_d(d: Dialect, table: &str) -> Result<String, String> {
    let t = d.quote_ident(table)?;
    Ok(format!("DROP TABLE IF EXISTS {t}"))
}

pub fn migrations_table_sql() -> &'static str {
    migrations_table_sql_d(Dialect::Postgres)
}

pub fn migrations_table_sql_d(d: Dialect) -> &'static str {
    match d {
        Dialect::Postgres => {
            "CREATE TABLE IF NOT EXISTS _migrations (\n\
             \tversion TEXT PRIMARY KEY,\n\
             \tname TEXT NOT NULL,\n\
             \texecuted_at TIMESTAMPTZ NOT NULL DEFAULT NOW()\n\
             )"
        }
        Dialect::Mysql => {
            "CREATE TABLE IF NOT EXISTS `_migrations` (\n\
             \t`version` VARCHAR(64) PRIMARY KEY,\n\
             \t`name` VARCHAR(255) NOT NULL,\n\
             \t`executed_at` TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP\n\
             )"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_list() -> ListQuery {
        let mut eq = BTreeMap::new();
        eq.insert("status".into(), serde_json::json!("up"));
        ListQuery {
            table: "users".into(),
            eq_filters: eq,
            filter_sdbql: Some("doc.status == @status".into()),
            soft_delete: SoftDeleteMode::Default,
            is_soft_delete_model: false,
            order_field: Some("name".into()),
            order_desc: false,
            limit: Some(10),
            offset: Some(5),
        }
    }

    #[test]
    fn compiles_hash_equality_select_pg() {
        let c = compile_select_d(Dialect::Postgres, &sample_list()).unwrap();
        assert!(c.sql.contains("SELECT doc FROM \"users\""));
        assert!(c.sql.contains("(doc->'status')"));
        assert!(c.sql.contains("ORDER BY doc->>'name' ASC"));
        assert!(c.sql.contains("$1"));
        assert_eq!(c.params[0], SqlBind::Json(serde_json::json!("up")));
    }

    #[test]
    fn compiles_hash_equality_select_mysql() {
        let c = compile_select_d(Dialect::Mysql, &sample_list()).unwrap();
        assert!(c.sql.contains("SELECT doc FROM `users`"));
        assert!(c.sql.contains("JSON_EXTRACT"));
        assert!(c.sql.contains('?'));
        // Placeholders are `?` (JSON paths may still contain `$.field`).
        assert!(!c.sql.contains("$1"));
    }

    #[test]
    fn rejects_raw_sdbql() {
        let mut q = sample_list();
        q.filter_sdbql = Some("doc.age >= @age".into());
        q.eq_filters.insert("age".into(), serde_json::json!(18));
        let err = compile_select_d(Dialect::Postgres, &q).unwrap_err();
        assert!(
            err.contains("SoliDB-only") || err.contains("hash form"),
            "{err}"
        );
    }

    #[test]
    fn compile_sum_pg() {
        let q = sample_list();
        let c = compile_aggregate_d(Dialect::Postgres, &q, SqlAgg::Sum, "amount").unwrap();
        assert!(c.sql.contains("SUM("));
        assert!(c.sql.contains("amount"));
    }

    #[test]
    fn compile_avg_min_max_mysql() {
        let q = sample_list();
        for (func, name) in [
            (SqlAgg::Avg, "AVG"),
            (SqlAgg::Min, "MIN"),
            (SqlAgg::Max, "MAX"),
            (SqlAgg::Count, "COUNT"),
        ] {
            let c = compile_aggregate_d(Dialect::Mysql, &q, func, "amount").unwrap();
            assert!(c.sql.contains(name), "{func:?}: {}", c.sql);
            assert!(c.sql.contains('?'));
            assert!(!c.sql.contains("$1"));
        }
    }

    #[test]
    fn compile_delete_all_and_update_all() {
        let q = sample_list();
        let d = compile_delete_all_d(Dialect::Postgres, &q).unwrap();
        assert!(d.sql.starts_with("DELETE FROM \"users\""));
        assert!(d.sql.contains("WHERE"));
        let u = compile_update_all_d(Dialect::Mysql, &q, &serde_json::json!({"views": 1})).unwrap();
        assert!(u.sql.starts_with("UPDATE `users` SET"));
        assert!(u.sql.contains("JSON_MERGE_PATCH"));
        assert_eq!(u.params[0], SqlBind::Json(serde_json::json!({"views": 1})));
    }

    #[test]
    fn soft_delete_default_and_only_deleted() {
        let mut q = sample_list();
        q.is_soft_delete_model = true;
        q.soft_delete = SoftDeleteMode::Default;
        let c = compile_select_d(Dialect::Postgres, &q).unwrap();
        assert!(
            c.sql.contains("deleted_at") && c.sql.contains("IS NULL"),
            "{}",
            c.sql
        );
        q.soft_delete = SoftDeleteMode::OnlyDeleted;
        let c2 = compile_select_d(Dialect::Mysql, &q).unwrap();
        assert!(
            c2.sql.contains("deleted_at") && c2.sql.contains("IS NOT NULL"),
            "{}",
            c2.sql
        );
        q.soft_delete = SoftDeleteMode::WithDeleted;
        let c3 = compile_select_d(Dialect::Postgres, &q).unwrap();
        assert!(!c3.sql.contains("deleted_at"), "{}", c3.sql);
    }

    #[test]
    fn multi_clause_and_key_filter() {
        let mut eq = BTreeMap::new();
        eq.insert("status".into(), serde_json::json!("up"));
        eq.insert("_key".into(), serde_json::json!("k1"));
        let q = ListQuery {
            table: "posts".into(),
            eq_filters: eq,
            filter_sdbql: Some("doc.status == @status AND doc._key == @_key".into()),
            soft_delete: SoftDeleteMode::Default,
            is_soft_delete_model: false,
            order_field: Some("_key".into()),
            order_desc: true,
            limit: Some(5),
            offset: None,
        };
        let c = compile_select_d(Dialect::Postgres, &q).unwrap();
        assert!(c.sql.contains("ORDER BY _key DESC"));
        assert!(c.sql.contains("_key = $"));
        assert!(c
            .params
            .iter()
            .any(|p| matches!(p, SqlBind::Text(s) if s == "k1")));
    }

    #[test]
    fn rejects_invalid_identifiers() {
        assert!(Dialect::Postgres.quote_ident("users;drop").is_err());
        assert!(Dialect::Mysql.quote_ident("1bad").is_err());
        let err =
            compile_aggregate_d(Dialect::Postgres, &sample_list(), SqlAgg::Sum, "a.b").unwrap_err();
        assert!(
            err.contains("invalid field") || err.contains("a.b"),
            "{err}"
        );
    }

    #[test]
    fn compile_select_by_keys_and_json_in() {
        let c = compile_select_by_keys_d(Dialect::Postgres, "users", &["a".into(), "b".into()])
            .unwrap();
        assert!(c.sql.contains("WHERE _key IN ($1, $2)"), "{}", c.sql);
        assert_eq!(c.params.len(), 2);
        let c2 = compile_select_json_text_in_d(Dialect::Mysql, "posts", "user_id", &["u1".into()])
            .unwrap();
        assert!(c2.sql.contains("JSON_UNQUOTE"), "{}", c2.sql);
        assert!(c2.sql.contains("IN (?)"), "{}", c2.sql);
    }

    #[test]
    fn compile_group_by_multi() {
        let q = sample_list();
        let aggs = vec![GroupAgg {
            alias: "total".into(),
            func: SqlAgg::Sum,
            field: "amount".into(),
        }];
        let c = compile_group_by_d(Dialect::Postgres, &q, &["country".into()], &aggs).unwrap();
        assert!(c.sql.contains("GROUP BY"), "{}", c.sql);
        assert!(c.sql.contains("SUM("), "{}", c.sql);
        assert!(c.sql.contains("AS \"country\""), "{}", c.sql);
        assert!(c.sql.contains("AS \"total\""), "{}", c.sql);
    }

    #[test]
    fn create_table_mysql_uses_json() {
        let s = create_table_sql_d(Dialect::Mysql, "blog_posts").unwrap();
        assert!(s.contains("`blog_posts`"));
        assert!(s.contains("JSON"));
        assert!(!s.contains("JSONB"));
    }

    #[test]
    fn create_table_quotes() {
        let s = create_table_sql("blog_posts").unwrap();
        assert!(s.contains("\"blog_posts\""));
        assert!(s.contains("JSONB"));
    }
}
