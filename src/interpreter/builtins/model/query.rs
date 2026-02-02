//! Query builder for chainable database queries.

use std::collections::HashMap;

use crate::interpreter::symbol::SymbolId;
use crate::interpreter::value::Value;

use super::crud::{exec_async_query_with_binds, json_to_value};

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
            query.push_str(&format!(" FILTER {}", filter));
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
}

/// Execute a QueryBuilder and return results.
pub fn execute_query_builder(qb: &QueryBuilder) -> Value {
    let (query, bind_vars) = qb.build_query();
    let bind_vars_opt = if bind_vars.is_empty() {
        None
    } else {
        Some(bind_vars)
    };

    match exec_async_query_with_binds(query, bind_vars_opt) {
        Ok(results) => json_to_value(&serde_json::Value::Array(results)),
        Err(e) => Value::String(format!("Error: {}", e)),
    }
}

/// Execute a QueryBuilder for first result only.
pub fn execute_query_builder_first(qb: &QueryBuilder) -> Value {
    let mut qb_with_limit = qb.clone();
    qb_with_limit.set_limit(1);
    let (query, bind_vars) = qb_with_limit.build_query();
    let bind_vars_opt = if bind_vars.is_empty() {
        None
    } else {
        Some(bind_vars)
    };

    match exec_async_query_with_binds(query, bind_vars_opt) {
        Ok(results) => {
            let first = results
                .into_iter()
                .next()
                .unwrap_or(serde_json::Value::Null);
            json_to_value(&first)
        }
        Err(e) => Value::String(format!("Error: {}", e)),
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
        query.push_str(&format!(" FILTER {}", filter));
    }

    query.push_str(" COLLECT WITH COUNT INTO count RETURN count");

    let bind_vars_opt = if bind_vars_str.is_empty() {
        None
    } else {
        Some(bind_vars_str)
    };

    match exec_async_query_with_binds(query, bind_vars_opt) {
        Ok(results) => json_to_value(&serde_json::Value::Array(results)),
        Err(e) => Value::String(format!("Error: {}", e)),
    }
}
