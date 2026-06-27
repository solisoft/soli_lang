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

/// A count-only eager-load clause. Emits `LET _rel_<alias> = LENGTH(...)`
/// and merges the count under `<alias>` on the parent doc. Only valid for
/// HasMany and HABTM; singular relations are rejected at registration time.
#[derive(Debug, Clone)]
pub struct IncludeCountClause {
    pub relation_name: String,
    pub relation: RelationDef,
    pub alias: String,
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
    pub includes_counts: Vec<IncludeCountClause>,
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
    /// Vector similarity search: (query_text, field, top_k) — set by .similar()
    pub similar_query: Option<(String, String, usize)>,
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
            includes_counts: Vec::new(),
            joins: Vec::new(),
            select_fields: None,
            pluck_fields: None,
            soft_delete_mode: SoftDeleteMode::Default,
            is_soft_delete_model: false,
            aggregation: None,
            exists_mode: false,
            group_by_info: None,
            similar_query: None,
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
            includes_counts: Vec::new(),
            joins: Vec::new(),
            select_fields: None,
            pluck_fields: None,
            soft_delete_mode: SoftDeleteMode::Default,
            is_soft_delete_model: is_sd,
            aggregation: None,
            exists_mode: false,
            group_by_info: None,
            similar_query: None,
        }
    }

    pub fn set_similar(&mut self, query: String, field: String, top_k: usize) {
        self.similar_query = Some((query, field, top_k));
    }

    pub fn set_pluck(&mut self, fields: Vec<String>) {
        self.pluck_fields = Some(fields);
    }

    pub fn set_filter(&mut self, filter: String, bind_vars: HashMap<String, serde_json::Value>) {
        // An empty filter (e.g. from `where({})`) is a no-op — leave `filter`
        // unset so no `FILTER` clause is emitted and the AQL stays valid.
        if filter.trim().is_empty() {
            return;
        }
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

    /// Register a count-only eager load. Returns Err for singular relations
    /// (BelongsTo, HasOne, Polymorphic) where a count is always 0 or 1 and
    /// the API would just add noise.
    pub fn add_include_count(
        &mut self,
        relation_name: String,
        relation: RelationDef,
    ) -> Result<(), String> {
        match relation.relation_type {
            RelationType::HasMany | RelationType::HasAndBelongsToMany => {}
            _ => {
                return Err(format!(
                    "includes_count('{}') is only supported for has_many and has_and_belongs_to_many relations",
                    relation_name
                ));
            }
        }
        if self
            .includes_counts
            .iter()
            .any(|inc| inc.relation_name == relation_name)
        {
            return Ok(());
        }
        let alias = format!("{}_count", relation_name);
        self.includes_counts.push(IncludeCountClause {
            relation_name,
            relation,
            alias,
        });
        Ok(())
    }

    pub fn add_include(
        &mut self,
        relation_name: String,
        relation: RelationDef,
        filter: Option<String>,
        bind_vars: HashMap<String, serde_json::Value>,
        fields: Option<Vec<String>>,
    ) {
        let has_extras = filter.is_some() || !bind_vars.is_empty() || fields.is_some();
        if let Some(idx) = self
            .includes
            .iter()
            .position(|inc| inc.relation_name == relation_name)
        {
            if !has_extras {
                return;
            }
            for (k, v) in &bind_vars {
                self.bind_vars
                    .insert(crate::interpreter::get_symbol(k), v.clone());
            }
            self.includes[idx] = IncludeClause {
                relation_name,
                relation,
                filter,
                bind_vars,
                fields,
            };
            return;
        }
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
        let has_extras = filter.is_some() || !bind_vars.is_empty();
        if let Some(idx) = self
            .joins
            .iter()
            .position(|j| j.relation_name == relation_name)
        {
            if !has_extras {
                return;
            }
            for (k, v) in &bind_vars {
                self.bind_vars
                    .insert(crate::interpreter::get_symbol(k), v.clone());
            }
            self.joins[idx] = JoinClause {
                relation_name,
                relation,
                filter,
                bind_vars,
            };
            return;
        }
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
            query.push_str(&Self::build_join_existence_filter(
                &join.relation,
                &join.filter,
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
            query.push_str(&Self::build_include_subquery(inc));
        }
        for inc in &self.includes_counts {
            query.push_str(&Self::build_include_count_subquery(inc));
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

        if self.includes.is_empty() && self.includes_counts.is_empty() {
            query.push_str(&format!(" RETURN {}", doc_return));
        } else {
            let mut merge_fields: Vec<String> = self
                .includes
                .iter()
                .map(|inc| {
                    let var_name = format!("_rel_{}", inc.relation_name);
                    match inc.relation.relation_type {
                        RelationType::HasMany | RelationType::HasAndBelongsToMany => {
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
            for inc in &self.includes_counts {
                let var_name = format!("_rel_{}", inc.alias);
                merge_fields.push(format!("{}: {}", inc.alias, var_name));
            }
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

    /// Build the ` FILTER LENGTH(...) > 0` existence-check fragment for a
    /// joined relation. Handles HABTM with a two-stage subquery.
    pub(crate) fn build_join_existence_filter(
        rel: &RelationDef,
        extra_filter: &Option<String>,
    ) -> String {
        if rel.relation_type == RelationType::HasAndBelongsToMany {
            let join_table = rel.join_table.as_deref().unwrap_or("");
            let assoc_fk = rel.association_foreign_key.as_deref().unwrap_or("");
            let extra = match extra_filter {
                Some(f) => {
                    let normalized = Self::normalize_equality_ops(f);
                    let prefixed = Self::prefix_bare_fields_with_alias(&normalized, "rel");
                    format!(" AND {}", prefixed)
                }
                None => String::new(),
            };
            return format!(
                " FILTER LENGTH(FOR jt IN {jt} FILTER jt.{owner_fk} == doc._key \
                 FOR rel IN {coll} FILTER rel._key == jt.{assoc_fk}{extra} LIMIT 1 RETURN 1) > 0",
                jt = join_table,
                owner_fk = rel.foreign_key,
                coll = rel.collection,
                assoc_fk = assoc_fk,
                extra = extra,
            );
        }

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
            RelationType::HasAndBelongsToMany => unreachable!(),
        };

        let subquery_filter = if let Some(extra) = extra_filter {
            let normalized = Self::normalize_equality_ops(extra);
            let prefixed = Self::prefix_bare_fields_with_alias(&normalized, "rel");
            format!("{} AND {}", fk_condition, prefixed)
        } else {
            fk_condition
        };

        format!(
            " FILTER LENGTH(FOR rel IN {} FILTER {} LIMIT 1 RETURN 1) > 0",
            rel.collection, subquery_filter
        )
    }

    /// Build a ` LET _rel_<alias> = LENGTH(...)` clause for a count-only
    /// include. Only HasMany and HABTM are reachable here (rejected at
    /// registration otherwise).
    pub(crate) fn build_include_count_subquery(inc: &IncludeCountClause) -> String {
        let rel = &inc.relation;
        let var_name = format!("_rel_{}", inc.alias);

        if rel.relation_type == RelationType::HasAndBelongsToMany {
            let join_table = rel.join_table.as_deref().unwrap_or("");
            return format!(
                " LET {var} = LENGTH(FOR jt IN {jt} FILTER jt.{owner_fk} == doc._key RETURN 1)",
                var = var_name,
                jt = join_table,
                owner_fk = rel.foreign_key,
            );
        }

        // HasMany
        format!(
            " LET {var} = LENGTH(FOR rel IN {coll} FILTER rel.{fk} == doc._key RETURN 1)",
            var = var_name,
            coll = rel.collection,
            fk = rel.foreign_key,
        )
    }

    /// Build a ` LET _rel_<name> = (...)` subquery clause for an include.
    pub(crate) fn build_include_subquery(inc: &IncludeClause) -> String {
        let rel = &inc.relation;
        let var_name = format!("_rel_{}", inc.relation_name);

        let return_clause = match &inc.fields {
            Some(fields) => {
                let pairs: Vec<String> =
                    fields.iter().map(|f| format!("{}: rel.{}", f, f)).collect();
                format!("RETURN {{{}}}", pairs.join(", "))
            }
            None => "RETURN rel".to_string(),
        };

        if rel.relation_type == RelationType::HasAndBelongsToMany {
            let join_table = rel.join_table.as_deref().unwrap_or("");
            let assoc_fk = rel.association_foreign_key.as_deref().unwrap_or("");
            let extra = match &inc.filter {
                Some(f) => {
                    let normalized = Self::normalize_equality_ops(f);
                    let prefixed = Self::prefix_bare_fields_with_alias(&normalized, "rel");
                    format!(" AND {}", prefixed)
                }
                None => String::new(),
            };
            return format!(
                " LET {var} = (FOR jt IN {jt} FILTER jt.{owner_fk} == doc._key \
                 FOR rel IN {coll} FILTER rel._key == jt.{assoc_fk}{extra} {ret})",
                var = var_name,
                jt = join_table,
                owner_fk = rel.foreign_key,
                coll = rel.collection,
                assoc_fk = assoc_fk,
                extra = extra,
                ret = return_clause,
            );
        }

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
            RelationType::HasAndBelongsToMany => unreachable!(),
        };

        let filter_condition = if let Some(extra) = &inc.filter {
            let normalized = Self::normalize_equality_ops(extra);
            let prefixed = Self::prefix_bare_fields_with_alias(&normalized, "rel");
            format!("{} AND {}", fk_condition, prefixed)
        } else {
            fk_condition
        };

        let limit_clause = match rel.relation_type {
            RelationType::HasMany => "",
            RelationType::HasOne | RelationType::BelongsTo | RelationType::Polymorphic => {
                " LIMIT 1"
            }
            RelationType::HasAndBelongsToMany => unreachable!(),
        };

        format!(
            " LET {} = (FOR rel IN {} FILTER {}{} {})",
            var_name, rel.collection, filter_condition, limit_clause, return_clause
        )
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
                } else if i < len {
                    if chars[i] == '(' || chars[i] == '.' {
                        result.push_str(&word);
                    } else {
                        result.push_str(alias);
                        result.push('.');
                        result.push_str(&word);
                    }
                } else {
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

    // Handle similarity search
    if let Some((ref query_text, ref field, ref top_k)) = qb.similar_query {
        return execute_similar_query(qb, &collection, query_text, field, *top_k);
    }

    let (query, bind_vars) = qb.build_query();

    // Inside a `grouped {}` block, register this read for coalescing instead of
    // firing it now; the rows→instances transform mirrors the paths below.
    if super::batch::is_active() {
        let class = qb.class.clone();
        return super::batch::register(
            query,
            bind_vars,
            Box::new(move |rows| {
                let values: Vec<Value> = match &class {
                    Some(c) => rows.iter().map(|j| json_doc_to_instance(c, j)).collect(),
                    None => rows.iter().map(super::crud::json_to_value).collect(),
                };
                Ok(Value::Array(std::rc::Rc::new(std::cell::RefCell::new(
                    values,
                ))))
            }),
        );
    }

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

/// Extract a float vector from a Soli Value (expects Array of floats).
fn value_to_float_vec(val: &Value) -> Vec<f64> {
    match val {
        Value::Array(arr) => arr
            .borrow()
            .iter()
            .filter_map(|v| match v {
                Value::Int(n) => Some(*n as f64),
                Value::Float(f) => Some(*f),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Execute a similarity search: fetch docs, compute cosine similarity, return top-k.
fn execute_similar_query(
    qb: &QueryBuilder,
    collection: &str,
    query_text: &str,
    field: &str,
    top_k: usize,
) -> Value {
    use crate::embedding::{cosine_similarity, generate_embedding};

    // Try to generate an embedding for the query text
    let query_vec = match generate_embedding(query_text) {
        Some(v) => v,
        None => return Value::Array(std::rc::Rc::new(std::cell::RefCell::new(Vec::new()))),
    };

    // Build a query to fetch matching docs (without similarity sorting)
    let mut fetch_qb = qb.clone();
    fetch_qb.similar_query = None;
    fetch_qb.limit_val = None;
    let (query, bind_vars) = fetch_qb.build_query();

    let results = if let Some(ref class) = qb.class {
        if bind_vars.is_empty() {
            exec_auto_collection_as_instances(query, collection, class)
        } else {
            exec_auto_collection_as_instances_with_binds(query, bind_vars, collection, class)
        }
    } else if bind_vars.is_empty() {
        exec_auto_collection(query, collection)
    } else {
        exec_auto_collection_with_binds(query, bind_vars, collection)
    };

    match results {
        Value::Array(arr) => {
            let mut scored: Vec<(f64, Value)> = Vec::new();
            for item in arr.borrow().iter() {
                let doc_vec = match item {
                    Value::Instance(inst) => inst
                        .borrow()
                        .get(field)
                        .map(|v| value_to_float_vec(&v))
                        .unwrap_or_default(),
                    Value::Hash(hash) => {
                        let h = hash.borrow();
                        // Try string key first, then HashKey
                        let field_val = h
                            .iter()
                            .find_map(|(k, v)| {
                                let k_str = format!("{}", k);
                                if k_str == field {
                                    Some(v.clone())
                                } else {
                                    None
                                }
                            })
                            .unwrap_or(Value::Null);
                        value_to_float_vec(&field_val)
                    }
                    _ => Vec::new(),
                };

                if !doc_vec.is_empty() && doc_vec.len() == query_vec.len() {
                    let score = cosine_similarity(&query_vec, &doc_vec);
                    scored.push((score, item.clone()));
                }
            }

            // Sort by score descending
            scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

            // Take top-k and attach _similarity_score
            let k = top_k.min(scored.len());
            let result: Vec<Value> = scored[..k]
                .iter()
                .map(|(score, item)| match item {
                    Value::Instance(inst) => {
                        let mut fields = inst.borrow().fields.clone();
                        fields.insert("_similarity_score".to_string(), Value::Float(*score));
                        Value::Instance(std::rc::Rc::new(std::cell::RefCell::new(
                            crate::interpreter::value::Instance {
                                class: inst.borrow().class.clone(),
                                fields,
                            },
                        )))
                    }
                    other => other.clone(),
                })
                .collect();

            Value::Array(std::rc::Rc::new(std::cell::RefCell::new(result)))
        }
        _ => Value::Array(std::rc::Rc::new(std::cell::RefCell::new(Vec::new()))),
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

    if super::batch::is_active() {
        let class = qb.class.clone();
        return super::batch::register(
            query,
            bind_vars,
            Box::new(move |rows| {
                Ok(match rows.first() {
                    Some(doc) => match &class {
                        Some(c) => json_doc_to_instance(c, doc),
                        None => super::crud::json_to_value(doc),
                    },
                    None => Value::Null,
                })
            }),
        );
    }

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
        query.push_str(&QueryBuilder::build_join_existence_filter(
            &join.relation,
            &join.filter,
        ));
    }

    if let Some(filter) = &qb.filter {
        let aql_filter = filter.replace(" && ", " AND ").replace(" || ", " OR ");
        let aql_filter = QueryBuilder::normalize_equality_ops(&aql_filter);
        let aql_filter = QueryBuilder::prefix_bare_fields(&aql_filter);
        query.push_str(&format!(" FILTER {}", aql_filter));
    }

    let query = if qb.joins.is_empty() && qb.filter.is_none() {
        format!("RETURN COLLECTION_COUNT(\"{}\")", collection)
    } else {
        format!("RETURN LENGTH({} RETURN 1)", query)
    };

    if super::batch::is_active() {
        return super::batch::register(
            query,
            bind_vars_str,
            // Subquery `(RETURN …)` yields a one-element array: [count].
            Box::new(move |rows| {
                Ok(rows
                    .first()
                    .map(super::crud::json_to_value)
                    .unwrap_or(Value::Int(0)))
            }),
        );
    }

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

/// Execute a QueryBuilder as a bulk REMOVE — every row matching the
/// accumulated filter / join clauses is deleted in a single AQL statement.
/// Mirrors Rails' `Model.where(...).delete_all` so callers don't have to
/// hand-roll `@sdbql{ FOR ... REMOVE ... }`. Returns `Null`.
///
/// Limitations: order/limit/offset/select/pluck/group_by are intentionally
/// ignored — they don't compose with REMOVE. Soft-deleted models still get
/// a real REMOVE here (this is a hard delete, not a soft-delete shortcut).
pub fn execute_query_builder_delete_all(qb: &QueryBuilder) -> Value {
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

    for join in &qb.joins {
        query.push_str(&QueryBuilder::build_join_existence_filter(
            &join.relation,
            &join.filter,
        ));
    }

    if let Some(filter) = &qb.filter {
        let aql_filter = filter.replace(" && ", " AND ").replace(" || ", " OR ");
        let aql_filter = QueryBuilder::normalize_equality_ops(&aql_filter);
        let aql_filter = QueryBuilder::prefix_bare_fields(&aql_filter);
        query.push_str(&format!(" FILTER {}", aql_filter));
    }

    query.push_str(&format!(" REMOVE doc IN {}", collection));

    let _ = if bind_vars_str.is_empty() {
        exec_auto_collection(query, &collection)
    } else {
        exec_auto_collection_with_binds(query, bind_vars_str, &collection)
    };

    Value::Null
}

/// Execute a QueryBuilder as a bulk UPDATE — every row matching the
/// accumulated filter / join clauses is patched with `update_data` in a
/// single AQL statement. Mirrors Rails' `Model.where(...).update_all(...)`
/// and the sibling [`execute_query_builder_delete_all`]. Returns `Null`.
///
/// Like `delete_all`, this is a raw bulk write: it skips validations and
/// lifecycle callbacks, and order/limit/offset/select/pluck/group_by are
/// intentionally ignored (they don't compose with UPDATE). The patch hash
/// is bound under a reserved name so it can't collide with user bind vars.
pub fn execute_query_builder_update_all(
    qb: &QueryBuilder,
    update_data: serde_json::Value,
) -> Value {
    let collection = crate::interpreter::symbol_string(qb.collection)
        .unwrap_or("unknown")
        .to_string();
    let mut query = format!("FOR doc IN {}", collection);

    let mut bind_vars_str: HashMap<String, serde_json::Value> = qb
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

    for join in &qb.joins {
        query.push_str(&QueryBuilder::build_join_existence_filter(
            &join.relation,
            &join.filter,
        ));
    }

    if let Some(filter) = &qb.filter {
        let aql_filter = filter.replace(" && ", " AND ").replace(" || ", " OR ");
        let aql_filter = QueryBuilder::normalize_equality_ops(&aql_filter);
        let aql_filter = QueryBuilder::prefix_bare_fields(&aql_filter);
        query.push_str(&format!(" FILTER {}", aql_filter));
    }

    bind_vars_str.insert("__soli_update".to_string(), update_data);
    query.push_str(&format!(
        " UPDATE doc WITH @__soli_update IN {}",
        collection
    ));

    let _ = exec_auto_collection_with_binds(query, bind_vars_str, &collection);

    Value::Null
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
        query.push_str(&QueryBuilder::build_join_existence_filter(
            &join.relation,
            &join.filter,
        ));
    }

    if let Some(filter) = &qb.filter {
        let aql_filter = filter.replace(" && ", " AND ").replace(" || ", " OR ");
        let aql_filter = QueryBuilder::normalize_equality_ops(&aql_filter);
        let aql_filter = QueryBuilder::prefix_bare_fields(&aql_filter);
        query.push_str(&format!(" FILTER {}", aql_filter));
    }

    query.push_str(" LIMIT 1 RETURN true");

    if super::batch::is_active() {
        return super::batch::register(
            query,
            bind_vars_str,
            Box::new(move |rows| Ok(Value::Bool(!rows.is_empty()))),
        );
    }

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

    if super::batch::is_active() {
        return super::batch::register(
            query,
            bind_vars_str,
            Box::new(move |rows| {
                Ok(rows
                    .first()
                    .map(super::crud::json_to_value)
                    .unwrap_or(Value::Null))
            }),
        );
    }

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
        let rel = build_relation(
            "User",
            "posts",
            RelationType::HasMany,
            None,
            None,
            None,
            None,
        );
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
        let rel = build_relation(
            "User",
            "profile",
            RelationType::HasOne,
            None,
            None,
            None,
            None,
        );
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
        let rel = build_relation(
            "Post",
            "user",
            RelationType::BelongsTo,
            None,
            None,
            None,
            None,
        );
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
        let posts_rel = build_relation(
            "User",
            "posts",
            RelationType::HasMany,
            None,
            None,
            None,
            None,
        );
        let profile_rel = build_relation(
            "User",
            "profile",
            RelationType::HasOne,
            None,
            None,
            None,
            None,
        );
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
    fn test_includes_is_idempotent() {
        let mut qb = make_qb("Contact", "contacts");
        let make_org = || {
            build_relation(
                "Contact",
                "organisation",
                RelationType::BelongsTo,
                None,
                None,
                None,
                None,
            )
        };
        qb.add_include(
            "organisation".to_string(),
            make_org(),
            None,
            HashMap::new(),
            None,
        );
        qb.add_include(
            "organisation".to_string(),
            make_org(),
            None,
            HashMap::new(),
            None,
        );
        let (query, _) = qb.build_query();
        assert_eq!(query.matches("LET _rel_organisation").count(), 1);
        assert_eq!(
            query
                .matches("organisation: FIRST(_rel_organisation)")
                .count(),
            1
        );
    }

    #[test]
    fn test_includes_second_call_with_filter_replaces_first() {
        let mut qb = make_qb("User", "users");
        let make_posts = || {
            build_relation(
                "User",
                "posts",
                RelationType::HasMany,
                None,
                None,
                None,
                None,
            )
        };
        qb.add_include(
            "posts".to_string(),
            make_posts(),
            None,
            HashMap::new(),
            None,
        );
        let mut binds = HashMap::new();
        binds.insert("p".to_string(), serde_json::Value::Bool(true));
        qb.add_include(
            "posts".to_string(),
            make_posts(),
            Some("published = @p".to_string()),
            binds,
            None,
        );
        let (query, _) = qb.build_query();
        assert_eq!(query.matches("LET _rel_posts").count(), 1);
        assert!(query.contains("rel.published == @p"));
    }

    #[test]
    fn test_join_has_many() {
        let mut qb = make_qb("User", "users");
        let rel = build_relation(
            "User",
            "posts",
            RelationType::HasMany,
            None,
            None,
            None,
            None,
        );
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
        let rel = build_relation(
            "User",
            "posts",
            RelationType::HasMany,
            None,
            None,
            None,
            None,
        );
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
        let rel = build_relation(
            "User",
            "posts",
            RelationType::HasMany,
            None,
            None,
            None,
            None,
        );
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
        let rel = build_relation(
            "Post",
            "user",
            RelationType::BelongsTo,
            None,
            None,
            None,
            None,
        );
        qb.add_join("user".to_string(), rel, None, HashMap::new());
        let (query, _) = qb.build_query();
        assert!(query.contains("doc.user_id == rel._key"));
    }

    #[test]
    fn test_filtered_include_has_many() {
        let mut qb = make_qb("User", "users");
        let rel = build_relation(
            "User",
            "posts",
            RelationType::HasMany,
            None,
            None,
            None,
            None,
        );
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
        let rel = build_relation(
            "User",
            "profile",
            RelationType::HasOne,
            None,
            None,
            None,
            None,
        );
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
        let rel = build_relation(
            "Post",
            "user",
            RelationType::BelongsTo,
            None,
            None,
            None,
            None,
        );
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
        let rel = build_relation(
            "User",
            "posts",
            RelationType::HasMany,
            None,
            None,
            None,
            None,
        );
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
        let rel = build_relation(
            "User",
            "posts",
            RelationType::HasMany,
            None,
            None,
            None,
            None,
        );
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
    fn test_includes_habtm() {
        let mut qb = make_qb("Post", "posts");
        let rel = crate::interpreter::builtins::model::relations::build_habtm_relation(
            "Post", "tags", None, None, None, None,
        );
        qb.add_include("tags".to_string(), rel, None, HashMap::new(), None);
        let (query, _) = qb.build_query();
        assert_eq!(
            query,
            "FOR doc IN posts LET _rel_tags = (FOR jt IN posts_tags FILTER jt.post_id == doc._key FOR rel IN tags FILTER rel._key == jt.tag_id RETURN rel) RETURN MERGE(doc, {tags: _rel_tags})"
        );
    }

    #[test]
    fn test_join_habtm() {
        let mut qb = make_qb("Post", "posts");
        let rel = crate::interpreter::builtins::model::relations::build_habtm_relation(
            "Post", "tags", None, None, None, None,
        );
        qb.add_join("tags".to_string(), rel, None, HashMap::new());
        let (query, _) = qb.build_query();
        assert_eq!(
            query,
            "FOR doc IN posts FILTER LENGTH(FOR jt IN posts_tags FILTER jt.post_id == doc._key FOR rel IN tags FILTER rel._key == jt.tag_id LIMIT 1 RETURN 1) > 0 RETURN doc"
        );
    }

    #[test]
    fn test_filtered_include_habtm() {
        let mut qb = make_qb("Post", "posts");
        let rel = crate::interpreter::builtins::model::relations::build_habtm_relation(
            "Post", "tags", None, None, None, None,
        );
        let mut bind_vars = HashMap::new();
        bind_vars.insert("a".to_string(), serde_json::Value::Bool(true));
        qb.add_include(
            "tags".to_string(),
            rel,
            Some("active = @a".to_string()),
            bind_vars,
            None,
        );
        let (query, binds) = qb.build_query();
        assert!(query.contains("FOR jt IN posts_tags FILTER jt.post_id == doc._key"));
        assert!(query.contains("FOR rel IN tags FILTER rel._key == jt.tag_id AND rel.active == @a"));
        assert!(binds.contains_key("a"));
    }

    #[test]
    fn test_select_with_includes() {
        let mut qb = make_qb("User", "users");
        qb.set_select(vec!["name".to_string(), "email".to_string()]);
        let rel = build_relation(
            "User",
            "posts",
            RelationType::HasMany,
            None,
            None,
            None,
            None,
        );
        qb.add_include("posts".to_string(), rel, None, HashMap::new(), None);
        let (query, _) = qb.build_query();
        assert!(query.contains(
            "RETURN MERGE({name: doc.name, email: doc.email, _key: doc._key}, {posts: _rel_posts})"
        ));
    }
}
