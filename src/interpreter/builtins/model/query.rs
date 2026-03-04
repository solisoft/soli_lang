//! Query builder for chainable database queries.

use std::collections::HashMap;

use crate::interpreter::symbol::SymbolId;
use crate::interpreter::value::Value;

use super::crud::{exec_auto_collection, exec_auto_collection_with_binds};

/// A query builder for chainable database queries.
/// Uses SDBQL filter expressions with symbol-based bind variables for O(1) lookup.
#[derive(Debug, Clone)]
pub struct QueryBuilder {
    pub class_name: SymbolId,
    pub collection: SymbolId,
    pub filter: Option<String>,
    pub bind_vars: HashMap<SymbolId, serde_json::Value>,
    pub order_by: Option<(SymbolId, SymbolId)>,
    pub limit_val: Option<usize>,
    pub offset_val: Option<usize>,
}

impl QueryBuilder {
    pub fn new(class_name: String, collection: String) -> Self {
        let class_id = crate::interpreter::get_symbol(&class_name);
        let collection_id = crate::interpreter::get_symbol(&collection);
        Self {
            class_name: class_id,
            collection: collection_id,
            filter: None,
            bind_vars: HashMap::new(),
            order_by: None,
            limit_val: None,
            offset_val: None,
        }
    }

    pub fn set_filter(&mut self, filter: String, bind_vars: HashMap<String, serde_json::Value>) {
        self.filter = Some(filter);
        self.bind_vars = bind_vars
            .into_iter()
            .map(|(k, v)| (crate::interpreter::get_symbol(&k), v))
            .collect();
    }

    pub fn set_order(&mut self, field: String, direction: String) {
        let field_id = crate::interpreter::get_symbol(&field);
        let dir_id = crate::interpreter::get_symbol(&direction);
        self.order_by = Some((field_id, dir_id));
    }

    pub fn set_limit(&mut self, limit: usize) {
        self.limit_val = Some(limit);
    }

    pub fn set_offset(&mut self, offset: usize) {
        self.offset_val = Some(offset);
    }

    /// Build the SDBQL query string.
    pub fn build_query(&self) -> (String, HashMap<String, serde_json::Value>) {
        let collection_str =
            crate::interpreter::symbol_string(self.collection).unwrap_or("unknown");
        let mut query = format!("FOR doc IN {}", collection_str);

        if let Some(filter) = &self.filter {
            // Translate Soli operators to AQL operators
            let aql_filter = filter
                .replace(" && ", " AND ")
                .replace(" || ", " OR ");
            // Replace single `=` with `==` for AQL comparison, but leave `!=`, `>=`, `<=` intact
            let aql_filter = Self::normalize_equality_ops(&aql_filter);
            // Auto-prefix bare field names with `doc.`
            let aql_filter = Self::prefix_bare_fields(&aql_filter);
            query.push_str(&format!(" FILTER {}", aql_filter));
        }

        if let Some((field, direction)) = &self.order_by {
            let field_str = crate::interpreter::symbol_string(*field).unwrap_or("unknown");
            let dir_str = crate::interpreter::symbol_string(*direction).unwrap_or("asc");
            let dir = match dir_str.to_lowercase().as_str() {
                "desc" | "descending" => "DESC",
                _ => "ASC",
            };
            query.push_str(&format!(" SORT doc.{} {}", field_str, dir));
        }

        if let Some(limit) = self.limit_val {
            if let Some(offset) = self.offset_val {
                query.push_str(&format!(" LIMIT {}, {}", offset, limit));
            } else {
                query.push_str(&format!(" LIMIT {}", limit));
            }
        }

        query.push_str(" RETURN doc");

        let bind_vars_str: HashMap<String, serde_json::Value> = self
            .bind_vars
            .iter()
            .map(|(k, v)| {
                (
                    crate::interpreter::symbol_string(*k)
                        .unwrap_or("")
                        .to_string(),
                    v.clone(),
                )
            })
            .collect();

        (query, bind_vars_str)
    }

    /// Replace standalone `=` with `==` for AQL, preserving `!=`, `>=`, `<=`, `==`, and `===`.
    pub(crate) fn normalize_equality_ops(filter: &str) -> String {
        let bytes = filter.as_bytes();
        let len = bytes.len();
        let mut result = String::with_capacity(len + 8);
        let mut i = 0;
        while i < len {
            if bytes[i] == b'=' {
                let prev = if i > 0 { bytes[i - 1] } else { b' ' };
                let next = if i + 1 < len { bytes[i + 1] } else { b' ' };
                if prev == b'!' || prev == b'>' || prev == b'<' {
                    // Part of !=, >=, <= — pass through
                    result.push('=');
                } else if next == b'=' {
                    // Already == (or ===) — pass through both and skip next
                    result.push('=');
                    result.push('=');
                    i += 2;
                    // Skip any further = (e.g. ===)
                    while i < len && bytes[i] == b'=' {
                        result.push('=');
                        i += 1;
                    }
                    continue;
                } else {
                    // Standalone = → expand to ==
                    result.push_str("==");
                }
            } else {
                result.push(bytes[i] as char);
            }
            i += 1;
        }
        result
    }

    /// Prefix bare identifiers with `doc.` so users can write `username == @u`
    /// instead of `doc.username == @u`. Skips `@`-prefixed bind vars, `doc.`-prefixed
    /// fields, AQL keywords, string literals, and numeric literals.
    pub(crate) fn prefix_bare_fields(filter: &str) -> String {
        let aql_keywords: &[&str] = &[
            "AND", "OR", "NOT", "IN", "LIKE", "null", "true", "false", "NONE", "ANY", "ALL",
        ];
        let mut result = String::with_capacity(filter.len() + 16);
        let chars: Vec<char> = filter.chars().collect();
        let len = chars.len();
        let mut i = 0;

        while i < len {
            let c = chars[i];

            // Skip string literals
            if c == '"' || c == '\'' {
                let quote = c;
                result.push(c);
                i += 1;
                while i < len && chars[i] != quote {
                    if chars[i] == '\\' && i + 1 < len {
                        result.push(chars[i]);
                        i += 1;
                    }
                    result.push(chars[i]);
                    i += 1;
                }
                if i < len {
                    result.push(chars[i]);
                    i += 1;
                }
                continue;
            }

            // Skip bind vars (@name)
            if c == '@' {
                result.push(c);
                i += 1;
                while i < len && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    result.push(chars[i]);
                    i += 1;
                }
                continue;
            }

            // Collect identifiers
            if c.is_alphabetic() || c == '_' {
                let start = i;
                while i < len && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect();

                // Check if already `doc.` prefixed or is a keyword
                if word == "doc" && i < len && chars[i] == '.' {
                    // Already doc.field — pass through doc. and the field name
                    result.push_str("doc.");
                    i += 1; // skip the dot
                    let field_start = i;
                    while i < len && (chars[i].is_alphanumeric() || chars[i] == '_') {
                        i += 1;
                    }
                    let field: String = chars[field_start..i].iter().collect();
                    result.push_str(&field);
                    continue;
                } else if aql_keywords.iter().any(|kw| kw.eq_ignore_ascii_case(&word))
                {
                    result.push_str(&word);
                } else if i < len && chars[i] == '.' {
                    // Has a dot qualifier (e.g. `obj.field`) — pass through as-is
                    result.push_str(&word);
                } else {
                    // Bare field name → prefix with doc.
                    result.push_str("doc.");
                    result.push_str(&word);
                }
                continue;
            }

            result.push(c);
            i += 1;
        }

        result
    }
}

/// Execute a QueryBuilder and return results.
pub fn execute_query_builder(qb: &QueryBuilder) -> Value {
    let collection = crate::interpreter::symbol_string(qb.collection)
        .unwrap_or("unknown")
        .to_string();
    let (query, bind_vars) = qb.build_query();

    if bind_vars.is_empty() {
        exec_auto_collection(query, &collection)
    } else {
        exec_auto_collection_with_binds(query, bind_vars, &collection)
    }
}

/// Execute a QueryBuilder for first result only.
pub fn execute_query_builder_first(qb: &QueryBuilder) -> Value {
    let collection = crate::interpreter::symbol_string(qb.collection)
        .unwrap_or("unknown")
        .to_string();
    let mut qb_with_limit = qb.clone();
    qb_with_limit.set_limit(1);
    let (query, bind_vars) = qb_with_limit.build_query();

    let result = if bind_vars.is_empty() {
        exec_auto_collection(query, &collection)
    } else {
        exec_auto_collection_with_binds(query, bind_vars, &collection)
    };

    match result {
        Value::Array(arr) => arr.borrow().iter().next().cloned().unwrap_or(Value::Null),
        // DB errors return Value::String("Error: ...") - treat as no result
        Value::String(ref s) if s.starts_with("Error:") => Value::Null,
        other => other,
    }
}

/// Execute a QueryBuilder for count.
pub fn execute_query_builder_count(qb: &QueryBuilder) -> Value {
    let collection = crate::interpreter::symbol_string(qb.collection)
        .unwrap_or("unknown")
        .to_string();
    let mut query = format!("FOR doc IN {}", collection);
    let bind_vars_str: HashMap<String, serde_json::Value> = qb
        .bind_vars
        .iter()
        .map(|(k, v)| {
            (
                crate::interpreter::symbol_string(*k)
                    .unwrap_or("")
                    .to_string(),
                v.clone(),
            )
        })
        .collect();

    if let Some(filter) = &qb.filter {
        let aql_filter = filter
            .replace(" && ", " AND ")
            .replace(" || ", " OR ");
        let aql_filter = QueryBuilder::normalize_equality_ops(&aql_filter);
        let aql_filter = QueryBuilder::prefix_bare_fields(&aql_filter);
        query.push_str(&format!(" FILTER {}", aql_filter));
    }

    query.push_str(" COLLECT WITH COUNT INTO cnt RETURN cnt");

    let result = if bind_vars_str.is_empty() {
        exec_auto_collection(query, &collection)
    } else {
        exec_auto_collection_with_binds(query, bind_vars_str, &collection)
    };

    // Unwrap the single-element array: [count] → count
    if let Value::Array(arr) = &result {
        if let Some(val) = arr.borrow().first() {
            return val.clone();
        }
    }
    result
}
