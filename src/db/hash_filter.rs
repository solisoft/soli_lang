//! Structured hash `.where` — equality plus comparisons, `IN`, `LIKE`, and `OR`.
//!
//! The Soli hash form used to be `{ "status": "open" }` only. Anything richer
//! had to be raw SDBQL, which SQL adapters refuse. This IR is what both
//! compilers consume so a comparison cannot silently become equality.
//!
//! ```soli
//! Order.where({
//!   "status": "open",
//!   "total": { "gt": 100 },
//!   "id": [1, 2, 3],
//!   "email": { "like": "%@x.com" },
//!   "or": [{ "state": "draft" }, { "state": "open" }]
//! })
//! ```

use std::collections::HashMap;

/// A comparison operator in the portable vocabulary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Gt,
    Gte,
    Lt,
    Lte,
    Like,
    Ilike,
}

impl CmpOp {
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "eq" | "==" | "=" => Some(CmpOp::Eq),
            "ne" | "!=" | "<>" => Some(CmpOp::Ne),
            "gt" | ">" => Some(CmpOp::Gt),
            "gte" | ">=" => Some(CmpOp::Gte),
            "lt" | "<" => Some(CmpOp::Lt),
            "lte" | "<=" => Some(CmpOp::Lte),
            "like" => Some(CmpOp::Like),
            "ilike" => Some(CmpOp::Ilike),
            _ => None,
        }
    }

    fn sdbql(self) -> &'static str {
        match self {
            CmpOp::Eq => "==",
            CmpOp::Ne => "!=",
            CmpOp::Gt => ">",
            CmpOp::Gte => ">=",
            CmpOp::Lt => "<",
            CmpOp::Lte => "<=",
            CmpOp::Like | CmpOp::Ilike => "LIKE",
        }
    }

    fn sql(self) -> &'static str {
        match self {
            CmpOp::Eq => "=",
            CmpOp::Ne => "<>",
            CmpOp::Gt => ">",
            CmpOp::Gte => ">=",
            CmpOp::Lt => "<",
            CmpOp::Lte => "<=",
            CmpOp::Like | CmpOp::Ilike => "LIKE",
        }
    }

    fn bind_suffix(self) -> &'static str {
        match self {
            CmpOp::Eq => "eq",
            CmpOp::Ne => "ne",
            CmpOp::Gt => "gt",
            CmpOp::Gte => "gte",
            CmpOp::Lt => "lt",
            CmpOp::Lte => "lte",
            CmpOp::Like => "like",
            CmpOp::Ilike => "ilike",
        }
    }
}

/// A portable predicate tree built from a Soli hash `.where`.
#[derive(Clone, Debug, PartialEq)]
pub enum HashFilter {
    And(Vec<HashFilter>),
    Or(Vec<HashFilter>),
    Cmp {
        field: String,
        op: CmpOp,
        value: serde_json::Value,
    },
    In {
        field: String,
        values: Vec<serde_json::Value>,
    },
}

impl HashFilter {
    /// Parse a JSON object the way `build_safe_filter_from_hash` delivers it.
    pub fn from_json_map(
        map: &serde_json::Map<String, serde_json::Value>,
        method: &str,
    ) -> Result<Self, String> {
        if map.is_empty() {
            return Ok(HashFilter::And(Vec::new()));
        }
        let mut parts = Vec::with_capacity(map.len());
        for (key, value) in map {
            if key.eq_ignore_ascii_case("or") {
                parts.push(Self::parse_group(value, true, method)?);
                continue;
            }
            if key.eq_ignore_ascii_case("and") {
                parts.push(Self::parse_group(value, false, method)?);
                continue;
            }
            crate::db::sql_compile::validate_field(key).map_err(|e| format!("{method}(): {e}"))?;
            parts.push(Self::parse_field(key, value, method)?);
        }
        Ok(if parts.len() == 1 {
            parts.pop().unwrap()
        } else {
            HashFilter::And(parts)
        })
    }

    fn parse_group(value: &serde_json::Value, or: bool, method: &str) -> Result<Self, String> {
        let serde_json::Value::Array(items) = value else {
            return Err(format!(
                "{method}() \"{}\" expects an array of hashes",
                if or { "or" } else { "and" }
            ));
        };
        if items.is_empty() {
            return Err(format!(
                "{method}() \"{}\" needs at least one clause",
                if or { "or" } else { "and" }
            ));
        }
        let mut kids = Vec::with_capacity(items.len());
        for item in items {
            let serde_json::Value::Object(map) = item else {
                return Err(format!(
                    "{method}() \"{}\" entries must be hashes",
                    if or { "or" } else { "and" }
                ));
            };
            kids.push(Self::from_json_map(map, method)?);
        }
        Ok(if or {
            HashFilter::Or(kids)
        } else {
            HashFilter::And(kids)
        })
    }

    fn parse_field(field: &str, value: &serde_json::Value, method: &str) -> Result<Self, String> {
        match value {
            serde_json::Value::Array(items) => {
                for (i, item) in items.iter().enumerate() {
                    if item.is_object() || item.is_array() {
                        return Err(format!(
                            "{method}() IN-list for {field:?} element {i} must be a scalar"
                        ));
                    }
                }
                Ok(HashFilter::In {
                    field: field.to_string(),
                    values: items.clone(),
                })
            }
            serde_json::Value::Object(ops) => {
                if ops.is_empty() {
                    return Err(format!(
                        "{method}() operator hash for {field:?} is empty. Use gt/gte/lt/lte/ne/like/ilike/in."
                    ));
                }
                let mut parts = Vec::with_capacity(ops.len());
                for (raw_op, val) in ops {
                    if raw_op == "in" {
                        let serde_json::Value::Array(items) = val else {
                            return Err(format!(
                                "{method}() \"in\" for {field:?} expects an array"
                            ));
                        };
                        parts.push(HashFilter::In {
                            field: field.to_string(),
                            values: items.clone(),
                        });
                        continue;
                    }
                    let Some(op) = CmpOp::parse(raw_op) else {
                        return Err(format!(
                            "{method}() unknown operator {raw_op:?} on {field:?}. \
                             Use gt, gte, lt, lte, eq, ne, like, ilike, in \
                             (or the symbols >, >=, <, <=, ==, =, !=, <>)."
                        ));
                    };
                    if val.is_object() || val.is_array() {
                        return Err(format!("{method}() {raw_op} on {field:?} expects a scalar"));
                    }
                    if matches!(op, CmpOp::Like | CmpOp::Ilike) && !val.is_string() {
                        return Err(format!(
                            "{method}() {raw_op} on {field:?} expects a string pattern"
                        ));
                    }
                    parts.push(HashFilter::Cmp {
                        field: field.to_string(),
                        op,
                        value: val.clone(),
                    });
                }
                Ok(if parts.len() == 1 {
                    parts.pop().unwrap()
                } else {
                    HashFilter::And(parts)
                })
            }
            other => Ok(HashFilter::Cmp {
                field: field.to_string(),
                op: CmpOp::Eq,
                value: other.clone(),
            }),
        }
    }

    /// SDBQL text plus bind map for the SoliDB path.
    pub fn to_sdbql(&self) -> (String, HashMap<String, serde_json::Value>) {
        let mut binds = HashMap::new();
        let mut n = 0u32;
        let sql = self.render_sdbql(&mut binds, &mut n);
        (sql, binds)
    }

    fn render_sdbql(&self, binds: &mut HashMap<String, serde_json::Value>, n: &mut u32) -> String {
        match self {
            HashFilter::And(parts) if parts.is_empty() => String::new(),
            HashFilter::And(parts) => parts
                .iter()
                .map(|p| p.render_sdbql(binds, n))
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" AND "),
            HashFilter::Or(parts) => {
                let inner = parts
                    .iter()
                    .map(|p| p.render_sdbql(binds, n))
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(" OR ");
                format!("({inner})")
            }
            HashFilter::In { field, values } => {
                let bind = unique_bind(field, "in", n);
                binds.insert(bind.clone(), serde_json::Value::Array(values.clone()));
                format!("doc.{field} IN @{bind}")
            }
            HashFilter::Cmp { field, op, value } => {
                let bind = unique_bind(field, op.bind_suffix(), n);
                binds.insert(bind.clone(), value.clone());
                match op {
                    CmpOp::Like => format!("LIKE(doc.{field}, @{bind}, false)"),
                    CmpOp::Ilike => format!("LIKE(doc.{field}, @{bind}, true)"),
                    other => format!("doc.{field} {} @{bind}", other.sdbql()),
                }
            }
        }
    }

    /// Evaluate this predicate against a JSON document (include-time filter).
    pub fn matches_json(&self, row: &serde_json::Value) -> bool {
        match self {
            HashFilter::And(parts) => parts.iter().all(|p| p.matches_json(row)),
            HashFilter::Or(parts) => parts.iter().any(|p| p.matches_json(row)),
            HashFilter::In { field, values } => {
                let got = json_field(row, field);
                match got {
                    None => values.iter().any(|v| v.is_null()),
                    Some(g) => values.iter().any(|v| json_eq(g, v)),
                }
            }
            HashFilter::Cmp { field, op, value } => {
                let got = json_field(row, field);
                match op {
                    CmpOp::Eq => match got {
                        None => value.is_null(),
                        Some(g) => json_eq(g, value),
                    },
                    CmpOp::Ne => match got {
                        None => !value.is_null(),
                        Some(g) => !json_eq(g, value),
                    },
                    CmpOp::Like | CmpOp::Ilike => {
                        let Some(serde_json::Value::String(text)) = got else {
                            return false;
                        };
                        let Some(pat) = value.as_str() else {
                            return false;
                        };
                        like_match(text, pat, *op == CmpOp::Ilike)
                    }
                    CmpOp::Gt | CmpOp::Gte | CmpOp::Lt | CmpOp::Lte => {
                        let Some(got) = got else {
                            return false;
                        };
                        cmp_ord(got, value, *op)
                    }
                }
            }
        }
    }

    /// True when this tree has no predicate (an empty `where({})`).
    pub fn is_empty(&self) -> bool {
        match self {
            HashFilter::And(parts) => parts.iter().all(HashFilter::is_empty),
            HashFilter::Or(parts) => parts.is_empty(),
            _ => false,
        }
    }
}

fn json_field<'a>(row: &'a serde_json::Value, field: &str) -> Option<&'a serde_json::Value> {
    row.get(field)
        .or_else(|| row.get("doc").and_then(|d| d.get(field)))
}

fn json_eq(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    a == b
}

fn cmp_ord(got: &serde_json::Value, want: &serde_json::Value, op: CmpOp) -> bool {
    let pair = match (got, want) {
        (serde_json::Value::Number(a), serde_json::Value::Number(b)) => {
            (a.as_f64().unwrap_or(0.0), b.as_f64().unwrap_or(0.0))
        }
        (serde_json::Value::String(a), serde_json::Value::String(b)) => {
            return match op {
                CmpOp::Gt => a > b,
                CmpOp::Gte => a >= b,
                CmpOp::Lt => a < b,
                CmpOp::Lte => a <= b,
                _ => false,
            };
        }
        (serde_json::Value::String(a), serde_json::Value::Number(b)) => {
            let Some(af) = a.parse::<f64>().ok() else {
                return false;
            };
            (af, b.as_f64().unwrap_or(0.0))
        }
        (serde_json::Value::Number(a), serde_json::Value::String(b)) => {
            let Some(bf) = b.parse::<f64>().ok() else {
                return false;
            };
            (a.as_f64().unwrap_or(0.0), bf)
        }
        _ => return false,
    };
    match op {
        CmpOp::Gt => pair.0 > pair.1,
        CmpOp::Gte => pair.0 >= pair.1,
        CmpOp::Lt => pair.0 < pair.1,
        CmpOp::Lte => pair.0 <= pair.1,
        _ => false,
    }
}

fn like_match(text: &str, pattern: &str, case_insensitive: bool) -> bool {
    let (text, pattern) = if case_insensitive {
        (text.to_lowercase(), pattern.to_lowercase())
    } else {
        (text.to_string(), pattern.to_string())
    };
    like_glob(&text, &pattern)
}

fn like_glob(text: &str, pattern: &str) -> bool {
    // `%` → any run, `_` → one character. Recursive is fine: include filters
    // are short patterns typed by the developer.
    fn rec(t: &[u8], p: &[u8]) -> bool {
        match (t, p) {
            (_, []) => t.is_empty(),
            (_, [b'%', rest @ ..]) => (0..=t.len()).any(|i| rec(&t[i..], rest)),
            ([_, t_rest @ ..], [b'_', p_rest @ ..]) => rec(t_rest, p_rest),
            ([th, t_rest @ ..], [ph, p_rest @ ..]) if th == ph => rec(t_rest, p_rest),
            _ => false,
        }
    }
    rec(text.as_bytes(), pattern.as_bytes())
}

fn unique_bind(field: &str, suffix: &str, n: &mut u32) -> String {
    *n += 1;
    format!("{field}__{suffix}_{n}")
}

/// Compile a hash filter against a JSON document column (`doc`).
pub fn compile_doc_pred(
    d: super::sql_compile::Dialect,
    pred: &HashFilter,
    params: &mut Vec<super::sql_compile::SqlBind>,
) -> Result<String, String> {
    compile_doc_pred_on(d, None, pred, params, 0)
}

/// Same as [`compile_doc_pred`], qualifying extracts with `table.doc` so a
/// correlated `EXISTS` cannot resolve `doc` to the parent row.
///
/// `param_offset` is the number of binds already claimed by the outer
/// statement (e.g. the patch in `UPDATE … SET`), so `$n` stays sequential.
pub fn compile_doc_pred_on(
    d: super::sql_compile::Dialect,
    table: Option<&str>,
    pred: &HashFilter,
    params: &mut Vec<super::sql_compile::SqlBind>,
    param_offset: usize,
) -> Result<String, String> {
    use super::sql_compile::SqlBind;
    match pred {
        HashFilter::And(parts) if parts.is_empty() => Ok(String::new()),
        HashFilter::And(parts) => {
            let mut out = Vec::new();
            for p in parts {
                let sql = compile_doc_pred_on(d, table, p, params, param_offset)?;
                if !sql.is_empty() {
                    out.push(sql);
                }
            }
            Ok(out.join(" AND "))
        }
        HashFilter::Or(parts) => {
            let mut out = Vec::new();
            for p in parts {
                let sql = compile_doc_pred_on(d, table, p, params, param_offset)?;
                if !sql.is_empty() {
                    out.push(sql);
                }
            }
            Ok(format!("({})", out.join(" OR ")))
        }
        HashFilter::In { field, values } => {
            super::sql_compile::validate_field(field)?;
            if values.is_empty() {
                return Ok("1 = 0".into());
            }
            let extract = match table {
                Some(t) => d.json_text_on(t, field),
                None => d.json_text(field),
            };
            // An IN list must agree with `eq` value by value, and the two extracts
            // are not interchangeable: a string compares on the TEXT extract
            // (which is what an expression index holds), while a number or bool
            // compares as JSON so `10` still matches a stored `10.0`. Binding a
            // number against the text extract is why `{ "n": [5, 500] }` used to
            // match nothing.
            let mut phs = Vec::new();
            let mut json_clauses = Vec::new();
            let mut has_null = false;
            for value in values {
                if value.is_null() {
                    has_null = true;
                    continue;
                }
                if let serde_json::Value::String(text) = value {
                    let n = param_offset + params.len() + 1;
                    params.push(SqlBind::Text(text.clone()));
                    phs.push(d.ph(n));
                    continue;
                }
                let n = param_offset + params.len() + 1;
                let ph = d.ph(n);
                params.push(SqlBind::Json(value.clone()));
                json_clauses.push(match table {
                    // Qualified, for a correlated EXISTS: an unqualified `doc`
                    // there would resolve to the outer row.
                    Some(t) => {
                        use super::sql_compile::Dialect;
                        let (extract, bind) = match d {
                            Dialect::Postgres => (format!("({t}.doc->'{field}')"), ph.clone()),
                            Dialect::Mysql => (
                                format!("JSON_EXTRACT({t}.doc, '$.{field}')"),
                                format!("CAST({ph} AS JSON)"),
                            ),
                            Dialect::Sqlite => {
                                (format!("({t}.doc -> '$.{field}')"), format!("json({ph})"))
                            }
                        };
                        format!("{extract} = {bind}")
                    }
                    None => d.json_eq(field, &ph),
                });
            }
            let mut clause = if phs.is_empty() {
                String::new()
            } else {
                format!("{extract} IN ({})", phs.join(", "))
            };
            for json_clause in json_clauses {
                clause = if clause.is_empty() {
                    json_clause
                } else {
                    format!("{clause} OR {json_clause}")
                };
            }
            if has_null {
                let null_sql = format!("{extract} IS NULL");
                clause = if clause.is_empty() {
                    null_sql
                } else {
                    format!("({clause} OR {null_sql})")
                };
            }
            Ok(clause)
        }
        HashFilter::Cmp { field, op, value } => {
            super::sql_compile::validate_field(field)?;
            let text = match table {
                Some(t) => d.json_text_on(t, field),
                None => d.json_text(field),
            };
            if value.is_null() {
                return Ok(match op {
                    CmpOp::Eq => format!("{text} IS NULL"),
                    CmpOp::Ne => format!("{text} IS NOT NULL"),
                    _ => {
                        return Err(format!(
                            "NULL only compares with equality, not {op:?} on {field:?}"
                        ))
                    }
                });
            }
            let n = param_offset + params.len() + 1;
            let ph = d.ph(n);
            match op {
                CmpOp::Like | CmpOp::Ilike => {
                    let serde_json::Value::String(pat) = value else {
                        return Err(format!("LIKE on {field:?} expects a string"));
                    };
                    params.push(SqlBind::Text(pat.clone()));
                    Ok(like_sql(d, &text, &ph, *op == CmpOp::Ilike))
                }
                CmpOp::Gt | CmpOp::Gte | CmpOp::Lt | CmpOp::Lte => {
                    push_cmp_bind(value, params);
                    // A string bound compares as TEXT, not as a number: casting
                    // the extract to numeric yields NULL for a text field, so
                    // `{ "created_at": { "gte": "2026-01-01" } }` matched nothing.
                    // ISO-8601 timestamps and dates order correctly as text,
                    // which is what makes that the useful default.
                    if value.is_string() {
                        return Ok(format!("{text} {} {ph}", op.sql()));
                    }
                    let num = match table {
                        Some(t) => d.json_num_on(&format!("{t}.doc"), field),
                        None => d.json_num(field),
                    };
                    Ok(format!("{num} {} {ph}", op.sql()))
                }
                CmpOp::Eq | CmpOp::Ne => {
                    if let serde_json::Value::String(_) = value {
                        params.push(SqlBind::Text(
                            value.as_str().unwrap_or_default().to_string(),
                        ));
                        Ok(format!("{text} {} {ph}", op.sql()))
                    } else {
                        params.push(SqlBind::Json(value.clone()));
                        let eq = match table {
                            Some(t) => d.json_eq_on(&format!("{t}.doc"), field, &ph),
                            None => d.json_eq(field, &ph),
                        };
                        if *op == CmpOp::Ne {
                            Ok(format!("NOT ({eq})"))
                        } else {
                            Ok(eq)
                        }
                    }
                }
            }
        }
    }
}

#[allow(dead_code)]
fn push_doc_bind(
    d: super::sql_compile::Dialect,
    field: &str,
    value: &serde_json::Value,
    params: &mut Vec<super::sql_compile::SqlBind>,
    phs: &mut Vec<String>,
    param_offset: usize,
) {
    use super::sql_compile::SqlBind;
    let n = param_offset + params.len() + 1;
    let ph = d.ph(n);
    if let serde_json::Value::String(text) = value {
        params.push(SqlBind::Text(text.clone()));
    } else {
        params.push(SqlBind::Json(value.clone()));
    }
    let _ = field;
    phs.push(ph);
}

fn push_cmp_bind(value: &serde_json::Value, params: &mut Vec<super::sql_compile::SqlBind>) {
    use super::sql_compile::SqlBind;
    match value {
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                params.push(SqlBind::I64(i));
            } else if let Some(f) = n.as_f64() {
                params.push(SqlBind::F64(f));
            } else {
                params.push(SqlBind::Text(n.to_string()));
            }
        }
        serde_json::Value::String(s) => params.push(SqlBind::Text(s.clone())),
        other => params.push(SqlBind::Json(other.clone())),
    }
}

/// `LIKE` / case-insensitive `LIKE` for one extract or column expression.
pub fn like_sql(
    d: super::sql_compile::Dialect,
    expr: &str,
    ph: &str,
    case_insensitive: bool,
) -> String {
    use super::sql_compile::Dialect;
    if !case_insensitive {
        return format!("{expr} LIKE {ph}");
    }
    match d {
        Dialect::Postgres => format!("{expr} ILIKE {ph}"),
        Dialect::Mysql | Dialect::Sqlite => format!("LOWER({expr}) LIKE LOWER({ph})"),
    }
}

/// Compile a hash filter against real columns.
pub fn compile_col_pred(
    d: super::sql_compile::Dialect,
    schema: &super::introspect::TableSchema,
    pred: &HashFilter,
    params: &mut Vec<super::sql_compile::SqlBind>,
) -> Result<String, String> {
    use super::sql_columns_compile::{bind_for_column, placeholder, resolve_col};
    match pred {
        HashFilter::And(parts) if parts.is_empty() => Ok(String::new()),
        HashFilter::And(parts) => {
            let mut out = Vec::new();
            for p in parts {
                let sql = compile_col_pred(d, schema, p, params)?;
                if !sql.is_empty() {
                    out.push(sql);
                }
            }
            Ok(out.join(" AND "))
        }
        HashFilter::Or(parts) => {
            let mut out = Vec::new();
            for p in parts {
                let sql = compile_col_pred(d, schema, p, params)?;
                if !sql.is_empty() {
                    out.push(sql);
                }
            }
            Ok(format!("({})", out.join(" OR ")))
        }
        HashFilter::In { field, values } => {
            let col = resolve_col(schema, field)?;
            let quoted = d.quote_ident(&col.name)?;
            if values.is_empty() {
                return Ok("1 = 0".into());
            }
            let mut phs = Vec::new();
            let mut has_null = false;
            for value in values {
                match bind_for_column(col, value)? {
                    None => has_null = true,
                    Some(bind) => {
                        params.push(bind);
                        phs.push(placeholder(d, params.len(), col.ty));
                    }
                }
            }
            let mut clause = if phs.is_empty() {
                String::new()
            } else {
                format!("{quoted} IN ({})", phs.join(", "))
            };
            if has_null {
                let null_sql = format!("{quoted} IS NULL");
                clause = if clause.is_empty() {
                    null_sql
                } else {
                    format!("({clause} OR {null_sql})")
                };
            }
            Ok(clause)
        }
        HashFilter::Cmp { field, op, value } => {
            let col = resolve_col(schema, field)?;
            let quoted = d.quote_ident(&col.name)?;
            if value.is_null() {
                return Ok(match op {
                    CmpOp::Eq => format!("{quoted} IS NULL"),
                    CmpOp::Ne => format!("{quoted} IS NOT NULL"),
                    _ => {
                        return Err(format!(
                            "NULL only compares with equality, not {op:?} on {field:?}"
                        ))
                    }
                });
            }
            match op {
                CmpOp::Like | CmpOp::Ilike => {
                    let serde_json::Value::String(pat) = value else {
                        return Err(format!("LIKE on {field:?} expects a string"));
                    };
                    params.push(super::sql_compile::SqlBind::Text(pat.clone()));
                    let ph = placeholder(d, params.len(), col.ty);
                    Ok(like_sql(d, &quoted, &ph, *op == CmpOp::Ilike))
                }
                _ => {
                    let Some(bind) = bind_for_column(col, value)? else {
                        return Ok(format!("{quoted} IS NULL"));
                    };
                    params.push(bind);
                    let ph = placeholder(d, params.len(), col.ty);
                    Ok(format!("{quoted} {} {ph}", op.sql()))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(raw: serde_json::Value) -> HashFilter {
        let serde_json::Value::Object(map) = raw else {
            panic!("object");
        };
        HashFilter::from_json_map(&map, "where").expect("parse")
    }

    #[test]
    fn equality_and_in_and_comparison() {
        let f = parse(serde_json::json!({
            "status": "open",
            "id": [1, 2, 3],
            "total": { "gt": 10, "lte": 99 }
        }));
        let (sdbql, binds) = f.to_sdbql();
        assert!(sdbql.contains("doc.status == @"), "{sdbql}");
        assert!(sdbql.contains("doc.id IN @"), "{sdbql}");
        assert!(sdbql.contains("doc.total > @"), "{sdbql}");
        assert!(sdbql.contains("doc.total <= @"), "{sdbql}");
        assert!(binds.values().any(|v| v == "open"));
    }

    #[test]
    fn or_group() {
        let f = parse(serde_json::json!({
            "paid": true,
            "or": [{ "status": "open" }, { "status": "draft" }]
        }));
        let (sdbql, _) = f.to_sdbql();
        assert!(sdbql.contains(" OR "), "{sdbql}");
        assert!(sdbql.contains("doc.paid"), "{sdbql}");
    }

    #[test]
    fn like_and_unknown_operator() {
        let f = parse(serde_json::json!({ "email": { "ilike": "%@x.com" } }));
        let (sdbql, _) = f.to_sdbql();
        assert!(sdbql.contains("LIKE(doc.email"), "{sdbql}");
        let err = HashFilter::from_json_map(
            serde_json::json!({ "n": { "between": 1 } })
                .as_object()
                .unwrap(),
            "where",
        )
        .unwrap_err();
        assert!(err.contains("unknown operator"), "{err}");
    }

    /// The symbolic spellings are part of the accepted vocabulary (documented in
    /// models.md), so they must compile to the same predicate as their names.
    #[test]
    fn symbolic_operators_match_their_named_forms() {
        use super::super::sql_compile::Dialect;
        for (symbol, name) in [
            (">", "gt"),
            (">=", "gte"),
            ("<", "lt"),
            ("<=", "lte"),
            ("==", "eq"),
            ("=", "eq"),
            ("!=", "ne"),
            ("<>", "ne"),
        ] {
            let compile = |op: &str| {
                let filter = HashFilter::from_json_map(
                    serde_json::json!({ "total": { op: 100 } })
                        .as_object()
                        .unwrap(),
                    "where",
                )
                .unwrap_or_else(|e| panic!("{op:?} should parse: {e}"));
                let mut params = Vec::new();
                compile_doc_pred(Dialect::Sqlite, &filter, &mut params).unwrap()
            };
            assert_eq!(compile(symbol), compile(name), "{symbol} vs {name}");
        }
    }

    #[test]
    fn empty_in_is_never_match() {
        use super::super::sql_compile::Dialect;
        let f = HashFilter::In {
            field: "id".into(),
            values: vec![],
        };
        let mut params = Vec::new();
        let sql = compile_doc_pred(Dialect::Sqlite, &f, &mut params).unwrap();
        assert_eq!(sql, "1 = 0");
    }

    #[test]
    fn matches_json_covers_comparisons_like_and_or() {
        let row = serde_json::json!({"status": "open", "total": 50, "email": "a@x.com"});
        let gt = parse(serde_json::json!({"total": {"gt": 10}}));
        assert!(gt.matches_json(&row));
        let too_high = parse(serde_json::json!({"total": {"gt": 100}}));
        assert!(!too_high.matches_json(&row));
        let like = parse(serde_json::json!({"email": {"like": "%@x.com"}}));
        assert!(like.matches_json(&row));
        let or = parse(serde_json::json!({
            "or": [{"status": "draft"}, {"status": "open"}]
        }));
        assert!(or.matches_json(&row));
        let inn = parse(serde_json::json!({"status": ["open", "paid"]}));
        assert!(inn.matches_json(&row));
    }

    #[test]
    fn like_glob_percent_and_underscore() {
        assert!(like_match("INV-1", "INV%", false));
        assert!(like_match("INV-1", "inv%", true));
        assert!(!like_match("INV-1", "inv%", false));
        assert!(like_match("abc", "a_c", false));
        assert!(!like_match("ac", "a_c", false));
    }
}
