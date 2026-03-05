//! Query builder for chainable database queries.

use std::collections::HashMap;

use crate::interpreter::symbol::SymbolId;
use crate::interpreter::value::Value;

use super::crud::{exec_auto_collection, exec_auto_collection_with_binds};
use super::relations::{RelationDef, RelationType};

/// An eager-load clause for a relation.
#[derive(Debug, Clone)]
pub struct IncludeClause {
    pub relation_name: String,
    pub relation: RelationDef,
}

/// A join-filter clause for a relation (existence check).
#[derive(Debug, Clone)]
pub struct JoinClause {
    pub relation_name: String,
    pub relation: RelationDef,
    pub filter: Option<String>,
    pub bind_vars: HashMap<String, serde_json::Value>,
}

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
    pub includes: Vec<IncludeClause>,
    pub joins: Vec<JoinClause>,
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
            includes: Vec::new(),
            joins: Vec::new(),
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

    pub fn add_include(&mut self, relation_name: String, relation: RelationDef) {
        self.includes.push(IncludeClause {
            relation_name,
            relation,
        });
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

        // Include subqueries (LET statements)
        for inc in &self.includes {
            let rel = &inc.relation;
            let var_name = format!("_rel_{}", inc.relation_name);

            match rel.relation_type {
                RelationType::HasMany => {
                    // FK on related model: rel.{fk} == doc._key
                    query.push_str(&format!(
                        " LET {} = (FOR rel IN {} FILTER rel.{} == doc._key RETURN rel)",
                        var_name, rel.collection, rel.foreign_key
                    ));
                }
                RelationType::HasOne => {
                    // FK on related model, LIMIT 1
                    query.push_str(&format!(
                        " LET {} = (FOR rel IN {} FILTER rel.{} == doc._key LIMIT 1 RETURN rel)",
                        var_name, rel.collection, rel.foreign_key
                    ));
                }
                RelationType::BelongsTo => {
                    // FK on owner model: doc.{fk} == rel._key
                    query.push_str(&format!(
                        " LET {} = (FOR rel IN {} FILTER rel._key == doc.{} LIMIT 1 RETURN rel)",
                        var_name, rel.collection, rel.foreign_key
                    ));
                }
            }
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

        // RETURN clause — with MERGE if includes are present
        if self.includes.is_empty() {
            query.push_str(" RETURN doc");
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
                        RelationType::HasOne | RelationType::BelongsTo => {
                            format!("{}: FIRST({})", inc.relation_name, var_name)
                        }
                    }
                })
                .collect();
            query.push_str(&format!(
                " RETURN MERGE(doc, {{{}}})",
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
        let rel = build_relation("User", "posts", RelationType::HasMany, None, None);
        qb.add_include("posts".to_string(), rel);
        let (query, _) = qb.build_query();
        assert_eq!(
            query,
            "FOR doc IN users LET _rel_posts = (FOR rel IN posts FILTER rel.user_id == doc._key RETURN rel) RETURN MERGE(doc, {posts: _rel_posts})"
        );
    }

    #[test]
    fn test_includes_has_one() {
        let mut qb = make_qb("User", "users");
        let rel = build_relation("User", "profile", RelationType::HasOne, None, None);
        qb.add_include("profile".to_string(), rel);
        let (query, _) = qb.build_query();
        assert_eq!(
            query,
            "FOR doc IN users LET _rel_profile = (FOR rel IN profiles FILTER rel.user_id == doc._key LIMIT 1 RETURN rel) RETURN MERGE(doc, {profile: FIRST(_rel_profile)})"
        );
    }

    #[test]
    fn test_includes_belongs_to() {
        let mut qb = make_qb("Post", "posts");
        let rel = build_relation("Post", "user", RelationType::BelongsTo, None, None);
        qb.add_include("user".to_string(), rel);
        let (query, _) = qb.build_query();
        assert_eq!(
            query,
            "FOR doc IN posts LET _rel_user = (FOR rel IN users FILTER rel._key == doc.user_id LIMIT 1 RETURN rel) RETURN MERGE(doc, {user: FIRST(_rel_user)})"
        );
    }

    #[test]
    fn test_includes_multiple() {
        let mut qb = make_qb("User", "users");
        let posts_rel = build_relation("User", "posts", RelationType::HasMany, None, None);
        let profile_rel = build_relation("User", "profile", RelationType::HasOne, None, None);
        qb.add_include("posts".to_string(), posts_rel);
        qb.add_include("profile".to_string(), profile_rel);
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
        let rel = build_relation("User", "posts", RelationType::HasMany, None, None);
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
        let rel = build_relation("User", "posts", RelationType::HasMany, None, None);
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
        let rel = build_relation("User", "posts", RelationType::HasMany, None, None);
        qb.add_include("posts".to_string(), rel);
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
        let rel = build_relation("Post", "user", RelationType::BelongsTo, None, None);
        qb.add_join("user".to_string(), rel, None, HashMap::new());
        let (query, _) = qb.build_query();
        assert!(query.contains("doc.user_id == rel._key"));
    }
}
