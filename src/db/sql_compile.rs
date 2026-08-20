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
    Sqlite,
}

/// A relation existence filter — `.join("comments")` keeps only parents that have
/// at least one matching child.
///
/// Compiled as a correlated `EXISTS` subquery rather than a real join, so the
/// parent rows stay unduplicated and the existing `SELECT doc` shape is untouched.
#[derive(Clone, Debug, PartialEq)]
pub struct ExistsFilter {
    /// The child collection to look in.
    pub table: String,
    /// The child's JSON field holding the parent key.
    pub foreign_key: String,
    /// Equality filters on the child, in the portable hash shape.
    pub eq_filters: BTreeMap<String, serde_json::Value>,
    /// Structured hash filter on the child (comparisons, IN, LIKE, OR).
    pub hash_filter: Option<super::hash_filter::HashFilter>,
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
    /// Structured hash `.where` (comparisons, IN, LIKE, OR). When set, this is
    /// compiled instead of flattening binds into equalities.
    pub hash_filter: Option<super::hash_filter::HashFilter>,
    pub filter_sdbql: Option<String>,
    /// Post-`GROUP BY` comparison, in the portable `alias op number` shape.
    pub having: Option<String>,
    /// Relation existence filters (`.join`), compiled as `EXISTS (…)`.
    pub exists_filters: Vec<ExistsFilter>,
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
    /// Structured hash `.where`. Passed in rather than attached afterwards: the
    /// validation compile below must see the filter that will actually run, or it
    /// judges the legacy `filter_sdbql` + binds pair that the IR replaces.
    pub hash_filter: Option<super::hash_filter::HashFilter>,
    pub filter_sdbql: Option<String>,
    /// True when a *string* `.where("…")` contributed to `filter_sdbql`, as
    /// opposed to it being the SDBQL echo a hash `.where` also produces. Only
    /// the echo is safe to drop when `hash_filter` is set.
    pub has_raw_where: bool,
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
    // A hash `.where` sets BOTH `hash_filter` (the IR) and `filter_sdbql` (its
    // SDBQL echo, for the SoliDB path), so the two being present together is
    // the normal case — the echo's binds are compiled from the IR and must not
    // be re-emitted as equalities.
    //
    // `has_raw_where` is the thing that distinguishes a *chained string*
    // `.where` from that echo. Dropping the raw filter in that case silently
    // discarded a predicate the caller asked for: `.where({active: true})
    // .where("doc.age >= @min", {min: 18})` returned minors. Raw SDBQL is not
    // portable to SQL, so refuse the combination loudly instead.
    let structured = parts.hash_filter.is_some();
    if structured && parts.has_raw_where {
        return Err(
            "String/raw SDBQL `.where(\"…\")` cannot be combined with a hash \
             `.where({ … })` on a SQL adapter. Express both conditions in the hash \
             form: `.where({ \"active\": true, \"age\": { \"gte\": 18 } })`."
                .to_string(),
        );
    }
    let mut eq_filters = BTreeMap::new();
    if let Some(ref f) = parts.filter_sdbql {
        if !structured && !f.trim().is_empty() {
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
        hash_filter: parts.hash_filter,
        filter_sdbql: if structured { None } else { parts.filter_sdbql },
        having: None,
        exists_filters: Vec::new(),
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
///
/// `Bool` and `F64` exist for column-aware models, whose binds go into real
/// typed columns — a `bool` column will not accept a JSON or text bind.
/// SQL NULL is never a bind: the column compiler emits `IS NULL` / the `NULL`
/// literal instead, which keeps both drivers' typed-null handling out of play.
#[derive(Clone, Debug, PartialEq)]
pub enum SqlBind {
    Json(serde_json::Value),
    I64(i64),
    F64(f64),
    Bool(bool),
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
    pub(crate) fn ph(self, n: usize) -> String {
        match self {
            Dialect::Postgres => format!("${n}"),
            // SQLite accepts `?` positionally, like MySQL.
            Dialect::Mysql | Dialect::Sqlite => "?".to_string(),
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
            // SQLite quotes identifiers with double quotes, like Postgres.
            Dialect::Postgres | Dialect::Sqlite => {
                format!("\"{}\"", name.replace('"', "\"\""))
            }
            Dialect::Mysql => format!("`{}`", name.replace('`', "``")),
        })
    }

    pub(crate) fn json_eq(self, field: &str, ph: &str) -> String {
        self.json_eq_on("doc", field, ph)
    }

    pub(crate) fn json_eq_on(self, doc: &str, field: &str, ph: &str) -> String {
        match self {
            Dialect::Postgres => format!("({doc}->'{field}') = {ph}"),
            Dialect::Mysql => format!("JSON_EXTRACT({doc}, '$.{field}') = CAST({ph} AS JSON)"),
            // `->` yields JSON text on both sides, so a JSON-encoded bind
            // compares like it does on the other two adapters — including a
            // JSON null, which `->>` would flatten to SQL NULL and never match.
            Dialect::Sqlite => format!("({doc} -> '$.{field}') = json({ph})"),
        }
    }

    fn json_order(self, field: &str) -> String {
        match self {
            Dialect::Postgres => format!("doc->>'{field}'"),
            Dialect::Mysql => format!("JSON_UNQUOTE(JSON_EXTRACT(doc, '$.{field}'))"),
            Dialect::Sqlite => format!("(doc ->> '$.{field}')"),
        }
    }

    fn soft_delete_null(self) -> &'static str {
        match self {
            Dialect::Postgres => "(doc->>'deleted_at') IS NULL",
            Dialect::Mysql => "JSON_EXTRACT(doc, '$.deleted_at') IS NULL",
            Dialect::Sqlite => "(doc ->> '$.deleted_at') IS NULL",
        }
    }

    fn soft_delete_not_null(self) -> &'static str {
        match self {
            Dialect::Postgres => "(doc->>'deleted_at') IS NOT NULL",
            Dialect::Mysql => "JSON_EXTRACT(doc, '$.deleted_at') IS NOT NULL",
            Dialect::Sqlite => "(doc ->> '$.deleted_at') IS NOT NULL",
        }
    }

    fn count_expr(self) -> &'static str {
        match self {
            Dialect::Postgres => "COUNT(*)::bigint",
            Dialect::Mysql | Dialect::Sqlite => "COUNT(*)",
        }
    }

    /// Numeric extract for SUM/AVG/MIN/MAX over a JSON field.
    pub(crate) fn json_num(self, field: &str) -> String {
        self.json_num_on("doc", field)
    }

    pub(crate) fn json_num_on(self, doc: &str, field: &str) -> String {
        match self {
            Dialect::Postgres => format!("({doc}->>'{field}')::float8"),
            Dialect::Mysql => {
                format!("CAST(JSON_UNQUOTE(JSON_EXTRACT({doc}, '$.{field}')) AS DECIMAL(30,10))")
            }
            // SQLite has no exact numeric type; REAL is the widest it offers.
            Dialect::Sqlite => format!("CAST(({doc} ->> '$.{field}') AS REAL)"),
        }
    }

    /// Text extract of a JSON field on a **qualified** table, for a correlated
    /// subquery where a bare `doc` would resolve to the wrong row.
    pub fn json_text_on(self, table: &str, field: &str) -> String {
        match self {
            Dialect::Postgres => format!("({table}.doc ->> '{field}')"),
            Dialect::Mysql => format!("JSON_UNQUOTE(JSON_EXTRACT({table}.doc, '$.{field}'))"),
            Dialect::Sqlite => format!("({table}.doc ->> '$.{field}')"),
        }
    }

    /// Text extract of a JSON field (for IN-lists, GROUP BY keys, string
    /// equality, and the expression indexes in [`super::ddl`]).
    ///
    /// Public because an index is only used when its expression is *identical*
    /// to the one in the predicate — `ddl::doc_index_sql` and `compile_where`
    /// must render it from the same place, or the index is dead weight.
    pub fn json_text(self, field: &str) -> String {
        match self {
            Dialect::Postgres => format!("(doc->>'{field}')"),
            Dialect::Mysql => format!("JSON_UNQUOTE(JSON_EXTRACT(doc, '$.{field}'))"),
            Dialect::Sqlite => format!("(doc ->> '$.{field}')"),
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
    if q.hash_filter.is_none() {
        assert_portable_filter(q.filter_sdbql.as_deref(), &q.eq_filters)?;
    }
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
    if q.hash_filter.is_none() {
        assert_portable_filter(q.filter_sdbql.as_deref(), &q.eq_filters)?;
    }
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
    if q.hash_filter.is_none() {
        assert_portable_filter(q.filter_sdbql.as_deref(), &q.eq_filters)?;
    }
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
    if q.hash_filter.is_none() {
        assert_portable_filter(q.filter_sdbql.as_deref(), &q.eq_filters)?;
    }
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
/// A `HAVING` comparison in the portable shape: `<alias> <op> <number>`.
///
/// The alias must be one the query already emits, so a typo cannot become SQL.
/// Anything richer (arithmetic, several conditions, a string comparison) is
/// refused with the supported shape named, rather than passed through — the
/// clause is developer-authored on SoliDB, where it is AQL, not SQL.
fn compile_having(
    d: Dialect,
    having: &str,
    group_fields: &[String],
    aggs: &[GroupAgg],
) -> Result<String, String> {
    let refuse = || {
        format!(
            "`.having({having:?})` on a SQL adapter supports one comparison of a \
             group key or aggregate alias against a number, e.g. \
             \"n > 5\" or \"total >= 100\". Filter in Soli after `.all` for anything else."
        )
    };
    let text = having.trim();
    // Longest operators first, or `>=` would match `>`.
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
             or aggregate aliases ({}).",
            group_fields
                .iter()
                .cloned()
                .chain(aggs.iter().map(|a| a.alias.clone()))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    // Only a numeric literal: no bind placeholder can appear in HAVING without
    // renumbering every parameter after it.
    if value.parse::<f64>().is_err() {
        return Err(refuse());
    }
    let sql_op = match op {
        "==" | "=" => "=",
        other => other,
    };
    // Repeat the aggregate expression rather than its alias: MySQL allows the
    // alias, Postgres does not.
    let left = if let Some(agg) = aggs.iter().find(|a| a.alias == alias) {
        match agg.func {
            SqlAgg::Count => d.count_expr().to_string(),
            SqlAgg::Sum => format!("SUM({})", d.json_num(&agg.field)),
            SqlAgg::Avg => format!("AVG({})", d.json_num(&agg.field)),
            SqlAgg::Min => format!("MIN({})", d.json_num(&agg.field)),
            SqlAgg::Max => format!("MAX({})", d.json_num(&agg.field)),
        }
    } else if aggs.is_empty() && alias == "n" {
        d.count_expr().to_string()
    } else {
        d.json_text(alias)
    };
    Ok(format!(" HAVING {left} {sql_op} {value}"))
}

pub fn compile_group_by_d(
    d: Dialect,
    q: &ListQuery,
    group_fields: &[String],
    aggs: &[GroupAgg],
) -> Result<CompiledSql, String> {
    if q.hash_filter.is_none() {
        assert_portable_filter(q.filter_sdbql.as_deref(), &q.eq_filters)?;
    }
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
    if let Some(having) = &q.having {
        sql.push_str(&compile_having(d, having, group_fields, aggs)?);
    }
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
    if q.hash_filter.is_none() {
        assert_portable_filter(q.filter_sdbql.as_deref(), &q.eq_filters)?;
    }
    let table = d.quote_ident(&q.table)?;
    let mut params = Vec::new();
    params.push(SqlBind::Json(patch.clone()));
    let patch_ph = d.ph(1);
    // Every adapter applies RFC 7396 (see `super::merge`). MySQL and SQLite have
    // a primitive for it; Postgres does not — `jsonb ||` is shallow and stores a
    // null instead of removing the key, so the null half is corrected with a
    // trailing key removal.
    //
    // The recursive half cannot be expressed in one Postgres statement, and a
    // read-merge-write loop over an unbounded row set is not a bulk update. A
    // patch containing a nested object is refused there rather than silently
    // destroying the stored object's other keys.
    if d == Dialect::Postgres && super::merge::needs_recursive_merge(patch) {
        return Err(
            "`update_all` with a nested object in the patch is not supported on \
             Postgres: merging into a stored object requires per-row recursion, and \
             `jsonb ||` would replace the object and drop its other keys. Patch the \
             nested field with a flat value, or iterate the records and `save` each."
                .to_string(),
        );
    }
    let set = match d {
        Dialect::Postgres => format!(
            "doc = (COALESCE(doc, '{{}}'::jsonb) || {patch_ph}) - (\
                 SELECT COALESCE(array_agg(key), ARRAY[]::text[]) \
                 FROM jsonb_each({patch_ph}) WHERE value = 'null'::jsonb\
             )"
        ),
        Dialect::Mysql => {
            format!("doc = JSON_MERGE_PATCH(COALESCE(doc, '{{}}'), CAST({patch_ph} AS JSON))")
        }
        Dialect::Sqlite => {
            format!("doc = json_patch(COALESCE(doc, '{{}}'), json({patch_ph}))")
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
    if let Some(pred) = &q.hash_filter {
        let sql =
            super::hash_filter::compile_doc_pred_on(d, None, pred, &mut params, param_offset)?;
        if !sql.is_empty() {
            parts.push(sql);
        }
    }
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
        } else if let serde_json::Value::String(text) = value {
            // Strings compare on the TEXT extract, which is what
            // `ddl::doc_index_sql` indexes — so `.where({ status: "open" })` can
            // use an index instead of scanning and extracting every row.
            // Numbers and booleans stay on the JSON comparison below, where
            // numeric equality (`10` matching a stored `10.0`) is preserved.
            params.push(SqlBind::Text(text.clone()));
            parts.push(format!("{} = {ph}", d.json_text(field)));
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
    for exists in &q.exists_filters {
        let child = d.quote_ident(&exists.table)?;
        let parent = d.quote_ident(&q.table)?;
        validate_field(&exists.foreign_key)?;
        // Correlate on the parent's `_key`, qualified so the subquery cannot
        // resolve `_key` to its own row.
        let mut inner = vec![format!(
            "{} = {parent}._key",
            d.json_text_on(&child, &exists.foreign_key)
        )];
        if let Some(pred) = &exists.hash_filter {
            let extra = super::hash_filter::compile_doc_pred_on(
                d,
                Some(&child),
                pred,
                &mut params,
                param_offset,
            )?;
            if !extra.is_empty() {
                inner.push(extra);
            }
        }
        for (field, value) in &exists.eq_filters {
            validate_field(field)?;
            let n = param_offset + params.len() + 1;
            let ph = d.ph(n);
            match value {
                serde_json::Value::String(text) => {
                    params.push(SqlBind::Text(text.clone()));
                    inner.push(format!("{} = {ph}", d.json_text_on(&child, field)));
                }
                other => {
                    // A non-string keeps JSON comparison semantics, qualified the
                    // same way.
                    params.push(SqlBind::Json(other.clone()));
                    inner.push(format!(
                        "{} = {}",
                        match d {
                            Dialect::Postgres => format!("({child}.doc->'{field}')"),
                            Dialect::Mysql => format!("JSON_EXTRACT({child}.doc, '$.{field}')"),
                            Dialect::Sqlite => format!("({child}.doc -> '$.{field}')"),
                        },
                        match d {
                            Dialect::Mysql => format!("CAST({ph} AS JSON)"),
                            Dialect::Sqlite => format!("json({ph})"),
                            Dialect::Postgres => ph.clone(),
                        }
                    ));
                }
            }
        }
        parts.push(format!(
            "EXISTS (SELECT 1 FROM {child} WHERE {})",
            inner.join(" AND ")
        ));
    }
    Ok((parts.join(" AND "), params))
}

pub(crate) fn validate_field(field: &str) -> Result<(), String> {
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

/// `INSERT INTO t (_key, doc) VALUES (…), (…), …` — one statement for many rows.
///
/// `create_many` issued one statement (and one round trip) per row; on SQLite it
/// also meant one transaction each. Chunking is the caller's job: every backend
/// has a bind limit, and Postgres's is the tightest at 65535.
pub fn compile_insert_many_d(
    d: Dialect,
    table: &str,
    rows: &[(String, serde_json::Value)],
) -> Result<CompiledSql, String> {
    if rows.is_empty() {
        return Err("insert_many with no rows".to_string());
    }
    let table_q = d.quote_ident(table)?;
    let mut params = Vec::with_capacity(rows.len() * 2);
    let mut tuples = Vec::with_capacity(rows.len());
    for (key, doc) in rows {
        params.push(SqlBind::Text(key.clone()));
        let key_ph = d.ph(params.len());
        params.push(SqlBind::Json(doc.clone()));
        let doc_ph = d.ph(params.len());
        let doc_expr = match d {
            Dialect::Postgres => format!("{doc_ph}::jsonb"),
            Dialect::Mysql => format!("CAST({doc_ph} AS JSON)"),
            Dialect::Sqlite => format!("json({doc_ph})"),
        };
        tuples.push(format!("({key_ph}, {doc_expr})"));
    }
    // Upsert, matching single-row `insert`: re-running a seed must not fail.
    let conflict = match d {
        Dialect::Postgres => " ON CONFLICT (_key) DO UPDATE SET doc = EXCLUDED.doc",
        Dialect::Mysql => " ON DUPLICATE KEY UPDATE doc = VALUES(doc)",
        Dialect::Sqlite => " ON CONFLICT(_key) DO UPDATE SET doc = excluded.doc",
    };
    Ok(CompiledSql {
        sql: format!(
            "INSERT INTO {table_q} (_key, doc) VALUES {}{conflict}",
            tuples.join(", ")
        ),
        params,
    })
}

pub fn create_table_sql_d(d: Dialect, table: &str) -> Result<String, String> {
    let t = d.quote_ident(table)?;
    let json_ty = match d {
        Dialect::Postgres => "JSONB",
        Dialect::Mysql => "JSON",
        // SQLite stores JSON as TEXT and validates it with the json1 functions.
        Dialect::Sqlite => "TEXT",
    };
    let key_ty = match d {
        Dialect::Postgres | Dialect::Sqlite => "TEXT",
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
        Dialect::Sqlite => {
            "CREATE TABLE IF NOT EXISTS \"_migrations\" (\n\
             \t\"version\" TEXT PRIMARY KEY,\n\
             \t\"name\" TEXT NOT NULL,\n\
             \t\"executed_at\" TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP\n\
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
            hash_filter: None,
            filter_sdbql: Some("doc.status == @status".into()),
            having: None,
            exists_filters: Vec::new(),
            soft_delete: SoftDeleteMode::Default,
            is_soft_delete_model: false,
            order_field: Some("name".into()),
            order_desc: false,
            limit: Some(10),
            offset: Some(5),
        }
    }

    #[test]
    fn a_structured_filter_survives_the_validation_compile() {
        use crate::db::hash_filter::HashFilter;

        // The regression this pins: `list_query_from_parts` ends with a
        // validation compile, so a structured filter has to be present *then* —
        // attaching it afterwards left the portable-shape assert judging the
        // legacy `filter_sdbql` + binds pair that the IR replaces, and a plain
        // `.where({ "status": "open" })` failed as "raw SDBQL".
        let mut bind_vars = HashMap::new();
        bind_vars.insert("status".to_string(), serde_json::json!("open"));
        let parts = ListQueryParts {
            table: "posts".into(),
            hash_filter: Some(
                HashFilter::from_json_map(
                    serde_json::json!({ "views": { "gt": 10 } })
                        .as_object()
                        .unwrap(),
                    "where",
                )
                .unwrap(),
            ),
            // A legacy pair that would NOT pass the portable-shape assert.
            filter_sdbql: Some("doc.views > @views".into()),
            has_raw_where: false,
            bind_vars,
            soft_delete: SoftDeleteMode::Default,
            is_soft_delete_model: false,
            order_field: None,
            order_desc: false,
            limit: None,
            offset: None,
        };
        let q =
            list_query_from_parts(parts, Dialect::Postgres).expect("structured filter compiles");
        // The IR wins: the legacy pair is dropped rather than emitted twice.
        assert!(q.hash_filter.is_some());
        assert!(q.filter_sdbql.is_none());
        assert!(q.eq_filters.is_empty());

        let c = compile_select_d(Dialect::Postgres, &q).unwrap();
        assert!(c.sql.contains("WHERE"), "{}", c.sql);
        assert_eq!(c.params.len(), 1, "one bind, from the IR only");

        // A *chained* string `.where` is not the hash filter's echo — it is an
        // independent predicate. Dropping it silently returned rows the caller
        // had excluded, so the mixed shape has to be refused.
        let mut bind_vars = HashMap::new();
        bind_vars.insert("active".to_string(), serde_json::json!(true));
        bind_vars.insert("min".to_string(), serde_json::json!(18));
        let parts = ListQueryParts {
            table: "users".into(),
            hash_filter: Some(
                HashFilter::from_json_map(
                    serde_json::json!({ "active": true }).as_object().unwrap(),
                    "where",
                )
                .unwrap(),
            ),
            filter_sdbql: Some("(doc.active == @active) AND (doc.age >= @min)".into()),
            has_raw_where: true,
            bind_vars,
            soft_delete: SoftDeleteMode::Default,
            is_soft_delete_model: false,
            order_field: None,
            order_desc: false,
            limit: None,
            offset: None,
        };
        let err = list_query_from_parts(parts, Dialect::Postgres)
            .expect_err("a chained raw .where must not be silently dropped");
        assert!(err.contains("cannot be combined"), "{err}");

        // Without a structured filter, a non-portable raw filter is still refused.
        let mut bind_vars = HashMap::new();
        bind_vars.insert("views".to_string(), serde_json::json!(10));
        let parts = ListQueryParts {
            table: "posts".into(),
            hash_filter: None,
            filter_sdbql: Some("doc.views > @views".into()),
            has_raw_where: false,
            bind_vars,
            soft_delete: SoftDeleteMode::Default,
            is_soft_delete_model: false,
            order_field: None,
            order_desc: false,
            limit: None,
            offset: None,
        };
        assert!(list_query_from_parts(parts, Dialect::Postgres).is_err());
    }

    #[test]
    fn having_compiles_the_portable_comparison_shape() {
        let mut q = sample_list();
        q.eq_filters.clear();
        q.filter_sdbql = None;
        q.order_field = None;
        q.having = Some("n > 5".into());
        let c = compile_group_by_d(Dialect::Postgres, &q, &["status".to_string()], &[]).unwrap();
        // The aggregate expression is repeated rather than its alias: MySQL
        // accepts the alias in HAVING, Postgres does not.
        assert!(c.sql.contains("HAVING COUNT(*)::bigint > 5"), "{}", c.sql);

        let aggs = vec![GroupAgg {
            alias: "total".into(),
            func: SqlAgg::Sum,
            field: "amount".into(),
        }];
        let mut q2 = q.clone();
        q2.having = Some("total >= 100".into());
        let c = compile_group_by_d(Dialect::Sqlite, &q2, &["status".to_string()], &aggs).unwrap();
        assert!(
            c.sql
                .contains("HAVING SUM(CAST((doc ->> '$.amount') AS REAL)) >= 100"),
            "{}",
            c.sql
        );

        // An alias the query does not emit is refused, listing what it does.
        let mut q3 = q.clone();
        q3.having = Some("nope > 1".into());
        let err =
            compile_group_by_d(Dialect::Postgres, &q3, &["status".to_string()], &aggs).unwrap_err();
        assert!(err.contains("nope") && err.contains("total"), "{err}");

        // Anything richer than one comparison against a number is refused with
        // the supported shape named, rather than passed through as SQL.
        for having in ["n > 5 AND total < 2", "n > total", "status = 'open'", "n"] {
            let mut q4 = q.clone();
            q4.having = Some(having.to_string());
            assert!(
                compile_group_by_d(Dialect::Postgres, &q4, &["status".to_string()], &aggs).is_err(),
                "{having:?} must be refused"
            );
        }
    }

    #[test]
    fn hash_filter_compiles_instead_of_flattening_to_equality() {
        use crate::db::hash_filter::HashFilter;
        let mut q = sample_list();
        q.eq_filters.clear();
        q.filter_sdbql = None;
        q.order_field = None;
        q.limit = None;
        q.offset = None;
        q.hash_filter = Some(
            HashFilter::from_json_map(
                serde_json::json!({"total": {"gt": 10}, "status": "open"})
                    .as_object()
                    .unwrap(),
                "where",
            )
            .unwrap(),
        );
        let c = compile_select_d(Dialect::Sqlite, &q).unwrap();
        assert!(c.sql.contains(" > ?"), "{}", c.sql);
        assert!(
            c.sql.contains(" = ?") || c.sql.contains("LIKE"),
            "{}",
            c.sql
        );
        assert_eq!(c.params.len(), 2);
    }

    #[test]
    fn join_compiles_a_correlated_exists() {
        let mut q = sample_list();
        q.eq_filters.clear();
        q.filter_sdbql = None;
        q.order_field = None;
        q.exists_filters = vec![ExistsFilter {
            table: "comments".into(),
            foreign_key: "post_id".into(),
            eq_filters: BTreeMap::new(),
            hash_filter: None,
        }];
        let c = compile_select_d(Dialect::Postgres, &q).unwrap();
        // Correlated on the parent's key, and qualified: an unqualified `doc`
        // inside the subquery would resolve to the child's own row.
        assert!(
            c.sql.contains(
                "EXISTS (SELECT 1 FROM \"comments\" WHERE (\"comments\".doc ->> 'post_id') = \"users\"._key)"
            ),
            "{}",
            c.sql
        );
        // No join means no duplicated parents, so `SELECT doc` is unchanged.
        assert!(c.sql.starts_with("SELECT doc FROM \"users\""));

        // A filter on the child rides inside the subquery.
        let mut with_filter = q.clone();
        with_filter.exists_filters[0]
            .eq_filters
            .insert("approved".into(), serde_json::json!("yes"));
        let c = compile_select_d(Dialect::Sqlite, &with_filter).unwrap();
        assert!(
            c.sql.contains("(\"comments\".doc ->> '$.approved') = ?"),
            "{}",
            c.sql
        );
        assert_eq!(c.params[0], SqlBind::Text("yes".into()));
    }

    #[test]
    fn compiles_hash_equality_select_pg() {
        let c = compile_select_d(Dialect::Postgres, &sample_list()).unwrap();
        assert!(c.sql.contains("SELECT doc FROM \"users\""));
        // A string filter compares on the TEXT extract, byte-for-byte the
        // expression `ddl::doc_index_sql` indexes — otherwise the index built
        // for `status` would never be used.
        assert!(c.sql.contains("(doc->>'status') = $1"), "{}", c.sql);
        assert!(c.sql.contains("ORDER BY doc->>'name' ASC"));
        assert_eq!(c.params[0], SqlBind::Text("up".into()));
    }

    #[test]
    fn non_string_equality_keeps_json_semantics() {
        // Numbers stay on the JSON comparison so `10` still matches a stored
        // `10.0`, which a text comparison would miss.
        let mut q = sample_list();
        q.eq_filters.clear();
        q.filter_sdbql = Some("doc.age == @age".into());
        q.eq_filters.insert("age".into(), serde_json::json!(10));
        let c = compile_select_d(Dialect::Postgres, &q).unwrap();
        assert!(c.sql.contains("(doc->'age')"), "{}", c.sql);
        assert_eq!(c.params[0], SqlBind::Json(serde_json::json!(10)));

        // A JSON null must still match through the JSON path: `->>` flattens it
        // to SQL NULL, which never compares equal.
        let mut q = sample_list();
        q.eq_filters.clear();
        q.filter_sdbql = Some("doc.deleted_at == @deleted_at".into());
        q.eq_filters
            .insert("deleted_at".into(), serde_json::Value::Null);
        let c = compile_select_d(Dialect::Postgres, &q).unwrap();
        assert!(c.sql.contains("(doc->'deleted_at')"), "{}", c.sql);
    }

    #[test]
    fn compiles_hash_equality_select_mysql() {
        let c = compile_select_d(Dialect::Mysql, &sample_list()).unwrap();
        assert!(c.sql.contains("SELECT doc FROM `users`"));
        assert!(
            c.sql
                .contains("JSON_UNQUOTE(JSON_EXTRACT(doc, '$.status'))"),
            "{}",
            c.sql
        );
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
            hash_filter: None,
            filter_sdbql: Some("doc.status == @status AND doc._key == @_key".into()),
            having: None,
            exists_filters: Vec::new(),
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
