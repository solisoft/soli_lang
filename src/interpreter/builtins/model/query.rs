//! Query builder for chainable database queries.

use std::collections::HashMap;

use std::rc::Rc;

use crate::interpreter::symbol::SymbolId;
use crate::interpreter::value::{Class, Value};

use super::crud::{
    exec_auto_collection, exec_auto_collection_as_instances,
    exec_auto_collection_as_instances_with_binds, exec_auto_collection_with_binds,
    json_doc_to_instance,
};
use super::relations::{RelationDef, RelationType};

/// An eager-load clause for a relation.
#[derive(Debug, Clone)]
pub struct IncludeClause {
    pub relation_name: String,
    pub relation: RelationDef,
    pub filter: Option<String>,
    pub bind_vars: HashMap<String, serde_json::Value>,
    pub fields: Option<Vec<String>>,
}

/// A join-filter clause for a relation (existence check).
#[derive(Debug, Clone)]
pub struct JoinClause {
    pub relation_name: String,
    pub relation: RelationDef,
    pub filter: Option<String>,
    pub bind_vars: HashMap<String, serde_json::Value>,
}

/// Controls how soft-deleted records are handled in queries.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum SoftDeleteMode {
    /// Default: exclude soft-deleted records (add `FILTER doc.deleted_at == null`)
    #[default]
    Default,
    /// Include all records, including soft-deleted
    WithDeleted,
    /// Only return soft-deleted records (add `FILTER doc.deleted_at != null`)
    OnlyDeleted,
}

/// A query builder for chainable database queries.
/// Uses SDBQL filter expressions with symbol-based bind variables for O(1) lookup.
#[derive(Debug, Clone)]
pub struct QueryBuilder {
    pub class_name: SymbolId,
    pub collection: SymbolId,
    pub class: Option<Rc<Class>>,
    pub filter: Option<String>,
    pub bind_vars: HashMap<SymbolId, serde_json::Value>,
    pub order_by: Option<(SymbolId, SymbolId)>,
    pub limit_val: Option<usize>,
    pub offset_val: Option<usize>,
    pub includes: Vec<IncludeClause>,
    pub joins: Vec<JoinClause>,
    pub select_fields: Option<Vec<String>>,
    pub pluck_fields: Option<Vec<String>>,
    pub soft_delete_mode: SoftDeleteMode,
    pub is_soft_delete_model: bool,
    /// Aggregation mode: (func, field) — set by sum/avg/min/max
    pub aggregation: Option<(AggregationFunc, String)>,
    /// Exists mode — set by .exists
    pub exists_mode: bool,
    /// Group-by mode: (group_field, func, agg_field) — set by .group_by
    pub group_by_info: Option<(String, AggregationFunc, String)>,
}

impl QueryBuilder {
    pub fn new(class_name: String, collection: String) -> Self {
        let class_id = crate::interpreter::get_symbol(&class_name);
        let collection_id = crate::interpreter::get_symbol(&collection);
        Self {
            class_name: class_id,
            collection: collection_id,
            class: None,
            filter: None,
            bind_vars: HashMap::new(),
            order_by: None,
            limit_val: None,
            offset_val: None,
            includes: Vec::new(),
            joins: Vec::new(),
            select_fields: None,
            pluck_fields: None,
            soft_delete_mode: SoftDeleteMode::Default,
            is_soft_delete_model: false,
            aggregation: None,
            exists_mode: false,
            group_by_info: None,
        }
    }

    pub fn new_with_class(class_name: String, collection: String, class: Rc<Class>) -> Self {
        let is_sd = super::core::is_soft_delete(&class_name);
        let class_id = crate::interpreter::get_symbol(&class_name);
        let collection_id = crate::interpreter::get_symbol(&collection);
        Self {
            class_name: class_id,
            collection: collection_id,
            class: Some(class),
            filter: None,
            bind_vars: HashMap::new(),
            order_by: None,
            limit_val: None,
            offset_val: None,
            includes: Vec::new(),
            joins: Vec::new(),
            select_fields: None,
            pluck_fields: None,
            soft_delete_mode: SoftDeleteMode::Default,
            is_soft_delete_model: is_sd,
            aggregation: None,
            exists_mode: false,
            group_by_info: None,
        }
    }

    pub fn set_pluck(&mut self, fields: Vec<String>) {
        self.pluck_fields = Some(fields);
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

    pub fn add_include(
        &mut self,
        relation_name: String,
        relation: RelationDef,
        filter: Option<String>,
        bind_vars: HashMap<String, serde_json::Value>,
        fields: Option<Vec<String>>,
    ) {
        // Merge include bind vars into the main bind_vars
        for (k, v) in &bind_vars {
            self.bind_vars
                .insert(crate::interpreter::get_symbol(k), v.clone());
        }
        self.includes.push(IncludeClause {
            relation_name,
            relation,
            filter,
            bind_vars,
            fields,
        });
    }

    pub fn set_select(&mut self, fields: Vec<String>) {
        self.select_fields = Some(fields);
    }

    pub fn add_join(
        &mut self,
        relation_name: String,
        relation: RelationDef,
        filter: Option<String>,
        bind_vars: HashMap<String, serde_json::Value>,
    ) {
        // Merge join bind vars into the main bind_vars
        for (k, v) in &bind_vars {
            self.bind_vars
                .insert(crate::interpreter::get_symbol(k), v.clone());
        }
        self.joins.push(JoinClause {
            relation_name,
            relation,
            filter,
            bind_vars,
        });
    }

    /// Build the SDBQL query string.
    pub fn build_query(&self) -> (String, HashMap<String, serde_json::Value>) {
        let collection_str =
            crate::interpreter::symbol_string(self.collection).unwrap_or("unknown");
        let mut query = format!("FOR doc IN {}", collection_str);

        // Join filters (existence checks) — before user filters
        for join in &self.joins {
            let rel = &join.relation;
            let fk_condition = match rel.relation_type {
                RelationType::HasMany | RelationType::HasOne => {
                    // FK is on the related model: rel.{fk} == doc._key
                    format!("rel.{} == doc._key", rel.foreign_key)
                }
                RelationType::BelongsTo => {
                    // FK is on the owner: doc.{fk} == rel._key
                    format!("doc.{} == rel._key", rel.foreign_key)
                }
                RelationType::Polymorphic => {
                    let type_field = rel
                        .polymorphic_type_field
                        .clone()
                        .unwrap_or_else(|| format!("{}_type", rel.name));
                    let type_value = rel
                        .polymorphic_type_value
                        .clone()
                        .unwrap_or_else(|| rel.class_name.clone());
                    format!(
                        "rel.{} == doc._key AND rel.{} == \"{}\"",
                        rel.foreign_key, type_field, type_value
                    )
                }
            };

            let subquery_filter = if let Some(extra) = &join.filter {
                let normalized = Self::normalize_equality_ops(extra);
                let prefixed = Self::prefix_bare_fields_with_alias(&normalized, "rel");
                format!("{} AND {}", fk_condition, prefixed)
            } else {
                fk_condition
            };

            query.push_str(&format!(
                " FILTER LENGTH(FOR rel IN {} FILTER {} LIMIT 1 RETURN 1) > 0",
                rel.collection, subquery_filter
            ));
        }

        if let Some(filter) = &self.filter {
            let aql_filter = filter.replace(" && ", " AND ").replace(" || ", " OR ");
            let aql_filter = Self::normalize_equality_ops(&aql_filter);
            let aql_filter = Self::prefix_bare_fields(&aql_filter);
            query.push_str(&format!(" FILTER {}", aql_filter));
        }

        // Soft delete filtering
        if self.is_soft_delete_model {
            match self.soft_delete_mode {
                SoftDeleteMode::Default => {
                    query.push_str(" FILTER doc.deleted_at == null");
                }
                SoftDeleteMode::OnlyDeleted => {
                    query.push_str(" FILTER doc.deleted_at != null");
                }
                SoftDeleteMode::WithDeleted => {
                    // Include all records — no additional filter
                }
            }
        }

        // Include subqueries (LET statements)
        for inc in &self.includes {
            let rel = &inc.relation;
            let var_name = format!("_rel_{}", inc.relation_name);

            let fk_condition = match rel.relation_type {
                RelationType::HasMany | RelationType::HasOne => {
                    format!("rel.{} == doc._key", rel.foreign_key)
                }
                RelationType::BelongsTo => {
                    format!("rel._key == doc.{}", rel.foreign_key)
                }
                RelationType::Polymorphic => {
                    let type_field = rel
                        .polymorphic_type_field
                        .clone()
                        .unwrap_or_else(|| format!("{}_type", rel.name));
                    let type_value = rel
                        .polymorphic_type_value
                        .clone()
                        .unwrap_or_else(|| rel.class_name.clone());
                    format!(
                        "rel._key == doc.{} AND rel.{} == \"{}\"",
                        rel.foreign_key, type_field, type_value
                    )
                }
            };

            let filter_condition = if let Some(extra) = &inc.filter {
                let normalized = Self::normalize_equality_ops(extra);
                let prefixed = Self::prefix_bare_fields_with_alias(&normalized, "rel");
                format!("{} AND {}", fk_condition, prefixed)
            } else {
                fk_condition
            };

            let return_clause = match &inc.fields {
                Some(fields) => {
                    let pairs: Vec<String> =
                        fields.iter().map(|f| format!("{}: rel.{}", f, f)).collect();
                    format!("RETURN {{{}}}", pairs.join(", "))
                }
                None => "RETURN rel".to_string(),
            };

            let limit_clause = match rel.relation_type {
                RelationType::HasMany => "",
                RelationType::HasOne | RelationType::BelongsTo | RelationType::Polymorphic => {
                    " LIMIT 1"
                }
            };

            query.push_str(&format!(
                " LET {} = (FOR rel IN {} FILTER {}{} {})",
                var_name, rel.collection, filter_condition, limit_clause, return_clause
            ));
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
        } else if let Some(offset) = self.offset_val {
            // Offset without explicit limit — use a large default
            query.push_str(&format!(" LIMIT {}, 1000000", offset));
        }

        // RETURN clause — with optional select projection, pluck, and MERGE for includes
        // pluck_fields takes precedence over select_fields
        let doc_return = if let Some(fields) = &self.pluck_fields {
            if fields.len() == 1 {
                format!("doc.{}", fields[0])
            } else {
                let pairs: Vec<String> =
                    fields.iter().map(|f| format!("{}: doc.{}", f, f)).collect();
                format!("{{{}}}", pairs.join(", "))
            }
        } else if let Some(fields) = &self.select_fields {
            let mut pairs: Vec<String> =
                fields.iter().map(|f| format!("{}: doc.{}", f, f)).collect();
            pairs.push("_key: doc._key".to_string());
            format!("{{{}}}", pairs.join(", "))
        } else {
            "doc".to_string()
        };

        if self.includes.is_empty() {
            query.push_str(&format!(" RETURN {}", doc_return));
        } else {
            let merge_fields: Vec<String> = self
                .includes
                .iter()
                .map(|inc| {
                    let var_name = format!("_rel_{}", inc.relation_name);
                    match inc.relation.relation_type {
                        RelationType::HasMany => {
                            format!("{}: {}", inc.relation_name, var_name)
                        }
                        RelationType::HasOne
                        | RelationType::BelongsTo
                        | RelationType::Polymorphic => {
                            format!("{}: FIRST({})", inc.relation_name, var_name)
                        }
                    }
                })
                .collect();
            query.push_str(&format!(
                " RETURN MERGE({}, {{{}}})",
                doc_return,
                merge_fields.join(", ")
            ));
        }

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

    /// Build an EXISTS query: FOR doc IN coll ... LIMIT 1 RETURN true
    pub fn build_exists_query(&self) -> (String, HashMap<String, serde_json::Value>) {
        let collection_str =
            crate::interpreter::symbol_string(self.collection).unwrap_or("unknown");
        let mut query = format!("FOR doc IN {}", collection_str);

        if let Some(filter) = &self.filter {
            let aql_filter = filter.replace(" && ", " AND ").replace(" || ", " OR ");
            let aql_filter = Self::normalize_equality_ops(&aql_filter);
            let aql_filter = Self::prefix_bare_fields(&aql_filter);
            query.push_str(&format!(" FILTER {}", aql_filter));
        }

        // Soft-delete filter
        if self.is_soft_delete_model {
            match self.soft_delete_mode {
                SoftDeleteMode::Default => query.push_str(" FILTER doc.deleted_at == null"),
                SoftDeleteMode::OnlyDeleted => query.push_str(" FILTER doc.deleted_at != null"),
                SoftDeleteMode::WithDeleted => {}
            }
        }

        query.push_str(" LIMIT 1 RETURN true");

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

    /// Build a GROUP BY query
    pub fn build_group_by_query(
        &self,
        group_field: &str,
        func: AggregationFunc,
        agg_field: &str,
    ) -> (String, HashMap<String, serde_json::Value>) {
        let collection_str =
            crate::interpreter::symbol_string(self.collection).unwrap_or("unknown");
        let mut query = format!("FOR doc IN {}", collection_str);

        if let Some(filter) = &self.filter {
            let aql_filter = filter.replace(" && ", " AND ").replace(" || ", " OR ");
            let aql_filter = Self::normalize_equality_ops(&aql_filter);
            let aql_filter = Self::prefix_bare_fields(&aql_filter);
            query.push_str(&format!(" FILTER {}", aql_filter));
        }

        let agg_expr = func.to_sdbql(agg_field);
        query.push_str(&format!(
            " COLLECT group = doc.{} AGGREGATE result = {} RETURN {{group: group, result: result}}",
            group_field, agg_expr
        ));

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
                    result.push('=');
                } else if next == b'=' {
                    result.push('=');
                    result.push('=');
                    i += 2;
                    while i < len && bytes[i] == b'=' {
                        result.push('=');
                        i += 1;
                    }
                    continue;
                } else {
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
        Self::prefix_bare_fields_with_alias(filter, "doc")
    }

    /// Prefix bare identifiers with a given alias (e.g. "doc" or "rel").
    pub(crate) fn prefix_bare_fields_with_alias(filter: &str, alias: &str) -> String {
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

                // Check if already prefixed with any alias (e.g. doc.field, rel.field)
                if i < len && chars[i] == '.' && (word == "doc" || word == "rel") {
                    // Already prefixed — pass through as-is
                    result.push_str(&word);
                    result.push('.');
                    i += 1; // skip the dot
                    let field_start = i;
                    while i < len && (chars[i].is_alphanumeric() || chars[i] == '_') {
                        i += 1;
                    }
                    let field: String = chars[field_start..i].iter().collect();
                    result.push_str(&field);
                    continue;
                } else if aql_keywords.iter().any(|kw| kw.eq_ignore_ascii_case(&word)) {
                    result.push_str(&word);
                } else if i < len && chars[i] == '.' {
                    // Has a dot qualifier (e.g. `obj.field`) — pass through as-is
                    result.push_str(&word);
                } else {
                    // Bare field name → prefix with alias
                    result.push_str(alias);
                    result.push('.');
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

/// Execute a QueryBuilder and return results (as instances if class is available).
pub fn execute_query_builder(qb: &QueryBuilder) -> Value {
    let collection = crate::interpreter::symbol_string(qb.collection)
        .unwrap_or("unknown")
        .to_string();
    let (query, bind_vars) = qb.build_query();

    if let Some(ref class) = qb.class {
        if bind_vars.is_empty() {
            exec_auto_collection_as_instances(query, &collection, class)
        } else {
            exec_auto_collection_as_instances_with_binds(query, bind_vars, &collection, class)
        }
    } else if bind_vars.is_empty() {
        exec_auto_collection(query, &collection)
    } else {
        exec_auto_collection_with_binds(query, bind_vars, &collection)
    }
}

/// Execute a QueryBuilder for first result only (as instance if class is available).
pub fn execute_query_builder_first(qb: &QueryBuilder) -> Value {
    let collection = crate::interpreter::symbol_string(qb.collection)
        .unwrap_or("unknown")
        .to_string();
    let mut qb_with_limit = qb.clone();
    qb_with_limit.set_limit(1);
    let (query, bind_vars) = qb_with_limit.build_query();

    // For first(), we can use raw JSON results + convert single item to instance
    let raw_result = if bind_vars.is_empty() {
        super::crud::exec_with_auto_collection(query, None, &collection)
    } else {
        super::crud::exec_with_auto_collection(query, Some(bind_vars), &collection)
    };

    match raw_result {
        Ok(results) => {
            if let Some(doc) = results.first() {
                if let Some(ref class) = qb.class {
                    json_doc_to_instance(class, doc)
                } else {
                    super::crud::json_to_value(doc)
                }
            } else {
                Value::Null
            }
        }
        Err(_) => Value::Null,
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

    // Join filters for count queries too
    for join in &qb.joins {
        let rel = &join.relation;
        let fk_condition = match rel.relation_type {
            RelationType::HasMany | RelationType::HasOne => {
                format!("rel.{} == doc._key", rel.foreign_key)
            }
            RelationType::BelongsTo => {
                format!("doc.{} == rel._key", rel.foreign_key)
            }
            RelationType::Polymorphic => {
                let type_field = rel
                    .polymorphic_type_field
                    .clone()
                    .unwrap_or_else(|| format!("{}_type", rel.name));
                let type_value = rel
                    .polymorphic_type_value
                    .clone()
                    .unwrap_or_else(|| rel.class_name.clone());
                format!(
                    "rel.{} == doc._key AND rel.{} == \"{}\"",
                    rel.foreign_key, type_field, type_value
                )
            }
        };

        let subquery_filter = if let Some(extra) = &join.filter {
            let normalized = QueryBuilder::normalize_equality_ops(extra);
            let prefixed = QueryBuilder::prefix_bare_fields_with_alias(&normalized, "rel");
            format!("{} AND {}", fk_condition, prefixed)
        } else {
            fk_condition
        };

        query.push_str(&format!(
            " FILTER LENGTH(FOR rel IN {} FILTER {} LIMIT 1 RETURN 1) > 0",
            rel.collection, subquery_filter
        ));
    }

    if let Some(filter) = &qb.filter {
        let aql_filter = filter.replace(" && ", " AND ").replace(" || ", " OR ");
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

/// Execute a QueryBuilder for exists check - returns boolean.
pub fn execute_query_builder_exists(qb: &QueryBuilder) -> Value {
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

    // Join filters for exists queries too
    for join in &qb.joins {
        let rel = &join.relation;
        let fk_condition = match rel.relation_type {
            RelationType::HasMany | RelationType::HasOne => {
                format!("rel.{} == doc._key", rel.foreign_key)
            }
            RelationType::BelongsTo => {
                format!("doc.{} == rel._key", rel.foreign_key)
            }
            RelationType::Polymorphic => {
                let type_field = rel
                    .polymorphic_type_field
                    .clone()
                    .unwrap_or_else(|| format!("{}_type", rel.name));
                let type_value = rel
                    .polymorphic_type_value
                    .clone()
                    .unwrap_or_else(|| rel.class_name.clone());
                format!(
                    "rel.{} == doc._key AND rel.{} == \"{}\"",
                    rel.foreign_key, type_field, type_value
                )
            }
        };

        let subquery_filter = if let Some(extra) = &join.filter {
            let normalized = QueryBuilder::normalize_equality_ops(extra);
            let prefixed = QueryBuilder::prefix_bare_fields_with_alias(&normalized, "rel");
            format!("{} AND {}", fk_condition, prefixed)
        } else {
            fk_condition
        };

        query.push_str(&format!(
            " FILTER LENGTH(FOR rel IN {} FILTER {} LIMIT 1 RETURN 1) > 0",
            rel.collection, subquery_filter
        ));
    }

    if let Some(filter) = &qb.filter {
        let aql_filter = filter.replace(" && ", " AND ").replace(" || ", " OR ");
        let aql_filter = QueryBuilder::normalize_equality_ops(&aql_filter);
        let aql_filter = QueryBuilder::prefix_bare_fields(&aql_filter);
        query.push_str(&format!(" FILTER {}", aql_filter));
    }

    query.push_str(" LIMIT 1 RETURN true");

    let result = if bind_vars_str.is_empty() {
        exec_auto_collection(query, &collection)
    } else {
        exec_auto_collection_with_binds(query, bind_vars_str, &collection)
    };

    // Return true if any result, false otherwise
    if let Value::Array(arr) = &result {
        Value::Bool(!arr.borrow().is_empty())
    } else {
        Value::Bool(false)
    }
}

#[derive(Debug, Clone)]
pub enum AggregationFunc {
    Sum,
    Avg,
    Min,
    Max,
}

impl AggregationFunc {
    pub fn to_sdbql(&self, field: &str) -> String {
        match self {
            AggregationFunc::Sum => format!("SUM(doc.{})", field),
            AggregationFunc::Avg => format!("AVG(doc.{})", field),
            AggregationFunc::Min => format!("MIN(doc.{})", field),
            AggregationFunc::Max => format!("MAX(doc.{})", field),
        }
    }
}

pub fn build_aggregation_query(
    qb: &QueryBuilder,
    func: AggregationFunc,
    field: &str,
) -> (String, HashMap<String, serde_json::Value>) {
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
        let aql_filter = filter.replace(" && ", " AND ").replace(" || ", " OR ");
        let aql_filter = QueryBuilder::normalize_equality_ops(&aql_filter);
        let aql_filter = QueryBuilder::prefix_bare_fields(&aql_filter);
        query.push_str(&format!(" FILTER {}", aql_filter));
    }

    query.push_str(&format!(" RETURN {}", func.to_sdbql(field)));

    (query, bind_vars_str)
}

/// Execute aggregation: sum, avg, min, max
pub fn execute_query_builder_aggregate(
    qb: &QueryBuilder,
    func: AggregationFunc,
    field: &str,
) -> Value {
    let (query, bind_vars_str) = build_aggregation_query(qb, func, field);

    let collection = crate::interpreter::symbol_string(qb.collection)
        .unwrap_or("unknown")
        .to_string();

    let result = if bind_vars_str.is_empty() {
        exec_auto_collection(query, &collection)
    } else {
        exec_auto_collection_with_binds(query, bind_vars_str, &collection)
    };

    if let Value::Array(arr) = &result {
        if let Some(val) = arr.borrow().first() {
            return val.clone();
        }
    }
    Value::Null
}

/// Execute group by aggregation
pub fn execute_query_builder_group_by(
    qb: &QueryBuilder,
    group_field: &str,
    func: AggregationFunc,
    agg_field: &str,
) -> Value {
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
        let aql_filter = filter.replace(" && ", " AND ").replace(" || ", " OR ");
        let aql_filter = QueryBuilder::normalize_equality_ops(&aql_filter);
        let aql_filter = QueryBuilder::prefix_bare_fields(&aql_filter);
        query.push_str(&format!(" FILTER {}", aql_filter));
    }

    let agg_expr = func.to_sdbql(agg_field);
    query.push_str(&format!(
        " COLLECT group = doc.{} AGGREGATE result = {} RETURN {{group: group, result: result}}",
        group_field, agg_expr
    ));

    if bind_vars_str.is_empty() {
        exec_auto_collection(query, &collection)
    } else {
        exec_auto_collection_with_binds(query, bind_vars_str, &collection)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::builtins::model::relations::{build_relation, RelationType};

    fn make_qb(class: &str, collection: &str) -> QueryBuilder {
        QueryBuilder::new(class.to_string(), collection.to_string())
    }

    #[test]
    fn test_basic_query() {
        let qb = make_qb("User", "users");
        let (query, binds) = qb.build_query();
        assert_eq!(query, "FOR doc IN users RETURN doc");
        assert!(binds.is_empty());
    }

    #[test]
    fn test_includes_has_many() {
        let mut qb = make_qb("User", "users");
        let rel = build_relation("User", "posts", RelationType::HasMany, None, None, None, None);
        qb.add_include("posts".to_string(), rel, None, HashMap::new(), None);
        let (query, _) = qb.build_query();
        assert_eq!(
            query,
            "FOR doc IN users LET _rel_posts = (FOR rel IN posts FILTER rel.user_id == doc._key RETURN rel) RETURN MERGE(doc, {posts: _rel_posts})"
        );
    }

    #[test]
    fn test_includes_has_one() {
        let mut qb = make_qb("User", "users");
        let rel = build_relation("User", "profile", RelationType::HasOne, None, None, None, None);
        qb.add_include("profile".to_string(), rel, None, HashMap::new(), None);
        let (query, _) = qb.build_query();
        assert_eq!(
            query,
            "FOR doc IN users LET _rel_profile = (FOR rel IN profiles FILTER rel.user_id == doc._key LIMIT 1 RETURN rel) RETURN MERGE(doc, {profile: FIRST(_rel_profile)})"
        );
    }

    #[test]
    fn test_includes_belongs_to() {
        let mut qb = make_qb("Post", "posts");
        let rel = build_relation("Post", "user", RelationType::BelongsTo, None, None, None, None);
        qb.add_include("user".to_string(), rel, None, HashMap::new(), None);
        let (query, _) = qb.build_query();
        assert_eq!(
            query,
            "FOR doc IN posts LET _rel_user = (FOR rel IN users FILTER rel._key == doc.user_id LIMIT 1 RETURN rel) RETURN MERGE(doc, {user: FIRST(_rel_user)})"
        );
    }

    #[test]
    fn test_includes_multiple() {
        let mut qb = make_qb("User", "users");
        let posts_rel = build_relation("User", "posts", RelationType::HasMany, None, None, None, None);
        let profile_rel = build_relation("User", "profile", RelationType::HasOne, None, None, None, None);
        qb.add_include("posts".to_string(), posts_rel, None, HashMap::new(), None);
        qb.add_include(
            "profile".to_string(),
            profile_rel,
            None,
            HashMap::new(),
            None,
        );
        let (query, _) = qb.build_query();
        assert!(query.contains("LET _rel_posts ="));
        assert!(query.contains("LET _rel_profile ="));
        assert!(
            query.contains("RETURN MERGE(doc, {posts: _rel_posts, profile: FIRST(_rel_profile)})")
        );
    }

    #[test]
    fn test_join_has_many() {
        let mut qb = make_qb("User", "users");
        let rel = build_relation("User", "posts", RelationType::HasMany, None, None, None, None);
        qb.add_join("posts".to_string(), rel, None, HashMap::new());
        let (query, _) = qb.build_query();
        assert_eq!(
            query,
            "FOR doc IN users FILTER LENGTH(FOR rel IN posts FILTER rel.user_id == doc._key LIMIT 1 RETURN 1) > 0 RETURN doc"
        );
    }

    #[test]
    fn test_join_with_filter() {
        let mut qb = make_qb("User", "users");
        let rel = build_relation("User", "posts", RelationType::HasMany, None, None, None, None);
        let mut bind_vars = HashMap::new();
        bind_vars.insert("p".to_string(), serde_json::Value::Bool(true));
        qb.add_join(
            "posts".to_string(),
            rel,
            Some("published = @p".to_string()),
            bind_vars,
        );
        let (query, binds) = qb.build_query();
        assert!(query.contains("rel.user_id == doc._key AND rel.published == @p"));
        assert!(binds.contains_key("p"));
    }

    #[test]
    fn test_includes_with_where() {
        let mut qb = make_qb("User", "users");
        let rel = build_relation("User", "posts", RelationType::HasMany, None, None, None, None);
        qb.add_include("posts".to_string(), rel, None, HashMap::new(), None);
        let mut bind_vars = HashMap::new();
        bind_vars.insert("a".to_string(), serde_json::Value::Bool(true));
        qb.set_filter("active = @a".to_string(), bind_vars);
        let (query, binds) = qb.build_query();
        assert!(query.contains("FILTER doc.active == @a"));
        assert!(query.contains("LET _rel_posts ="));
        assert!(query.contains("RETURN MERGE(doc, {posts: _rel_posts})"));
        assert!(binds.contains_key("a"));
    }

    #[test]
    fn test_join_belongs_to() {
        let mut qb = make_qb("Post", "posts");
        let rel = build_relation("Post", "user", RelationType::BelongsTo, None, None, None, None);
        qb.add_join("user".to_string(), rel, None, HashMap::new());
        let (query, _) = qb.build_query();
        assert!(query.contains("doc.user_id == rel._key"));
    }

    #[test]
    fn test_filtered_include_has_many() {
        let mut qb = make_qb("User", "users");
        let rel = build_relation("User", "posts", RelationType::HasMany, None, None, None, None);
        let mut bind_vars = HashMap::new();
        bind_vars.insert("p".to_string(), serde_json::Value::Bool(true));
        qb.add_include(
            "posts".to_string(),
            rel,
            Some("published = @p".to_string()),
            bind_vars,
            None,
        );
        let (query, binds) = qb.build_query();
        assert!(query.contains("rel.user_id == doc._key AND rel.published == @p"));
        assert!(query.contains("RETURN rel)"));
        assert!(binds.contains_key("p"));
    }

    #[test]
    fn test_filtered_include_has_one() {
        let mut qb = make_qb("User", "users");
        let rel = build_relation("User", "profile", RelationType::HasOne, None, None, None, None);
        let mut bind_vars = HashMap::new();
        bind_vars.insert("a".to_string(), serde_json::Value::Bool(true));
        qb.add_include(
            "profile".to_string(),
            rel,
            Some("active = @a".to_string()),
            bind_vars,
            None,
        );
        let (query, binds) = qb.build_query();
        assert!(query.contains("rel.user_id == doc._key AND rel.active == @a"));
        assert!(query.contains("LIMIT 1 RETURN rel)"));
        assert!(binds.contains_key("a"));
    }

    #[test]
    fn test_filtered_include_belongs_to() {
        let mut qb = make_qb("Post", "posts");
        let rel = build_relation("Post", "user", RelationType::BelongsTo, None, None, None, None);
        let mut bind_vars = HashMap::new();
        bind_vars.insert("a".to_string(), serde_json::Value::Bool(true));
        qb.add_include(
            "user".to_string(),
            rel,
            Some("active = @a".to_string()),
            bind_vars,
            None,
        );
        let (query, binds) = qb.build_query();
        assert!(query.contains("rel._key == doc.user_id AND rel.active == @a"));
        assert!(query.contains("LIMIT 1 RETURN rel)"));
        assert!(binds.contains_key("a"));
    }

    #[test]
    fn test_include_with_fields() {
        let mut qb = make_qb("User", "users");
        let rel = build_relation("User", "posts", RelationType::HasMany, None, None, None, None);
        qb.add_include(
            "posts".to_string(),
            rel,
            None,
            HashMap::new(),
            Some(vec!["title".to_string(), "body".to_string()]),
        );
        let (query, _) = qb.build_query();
        assert!(query.contains("RETURN {title: rel.title, body: rel.body})"));
    }

    #[test]
    fn test_filtered_include_with_fields() {
        let mut qb = make_qb("User", "users");
        let rel = build_relation("User", "posts", RelationType::HasMany, None, None, None, None);
        let mut bind_vars = HashMap::new();
        bind_vars.insert("p".to_string(), serde_json::Value::Bool(true));
        qb.add_include(
            "posts".to_string(),
            rel,
            Some("published = @p".to_string()),
            bind_vars,
            Some(vec!["title".to_string()]),
        );
        let (query, binds) = qb.build_query();
        assert!(query.contains("rel.user_id == doc._key AND rel.published == @p"));
        assert!(query.contains("RETURN {title: rel.title})"));
        assert!(binds.contains_key("p"));
    }

    #[test]
    fn test_select_fields() {
        let mut qb = make_qb("User", "users");
        qb.set_select(vec!["name".to_string(), "email".to_string()]);
        let (query, _) = qb.build_query();
        assert_eq!(
            query,
            "FOR doc IN users RETURN {name: doc.name, email: doc.email, _key: doc._key}"
        );
    }

    #[test]
    fn test_select_with_includes() {
        let mut qb = make_qb("User", "users");
        qb.set_select(vec!["name".to_string(), "email".to_string()]);
        let rel = build_relation("User", "posts", RelationType::HasMany, None, None, None, None);
        qb.add_include("posts".to_string(), rel, None, HashMap::new(), None);
        let (query, _) = qb.build_query();
        assert!(query.contains(
            "RETURN MERGE({name: doc.name, email: doc.email, _key: doc._key}, {posts: _rel_posts})"
        ));
    }
}
