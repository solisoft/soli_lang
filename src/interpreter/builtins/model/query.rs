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
use super::graph::TraversalClause;
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

/// Time-bucketed aggregation over a timeseries model — set by
/// `.time_bucket(interval, aggregates)`. Emits `COLLECT bucket =
/// TIME_BUCKET(doc.<ts>, "<interval>") AGGREGATE ...`.
#[derive(Debug, Clone)]
pub struct TimeBucketSpec {
    /// `<n><unit>` with unit s/m/h/d (what the DB's TIME_BUCKET accepts).
    pub interval: String,
    /// Timestamp field; the DSL `timestamp:` option, else `_created_at`
    /// (server-set RFC3339 on every document).
    pub timestamp_field: String,
    /// (alias, FUNC, field) — FUNC is upper-cased SDBQL (SUM/AVG/MIN/MAX/
    /// COUNT); field is empty for COUNT.
    pub aggregates: Vec<(String, String, String)>,
}

/// The query side of a `.similar()` call: embed a text query client-side,
/// or take a raw vector literal (no embedding round-trip).
#[derive(Debug, Clone)]
pub enum SimilarInput {
    Text(String),
    Vector(Vec<f64>),
}

/// `.similar()` state. With a declared `vector_index` on the field, the
/// search is pushed down to the DB's HNSW index (unless `exact: true`);
/// otherwise the historical client-side exact-cosine path runs.
#[derive(Debug, Clone)]
pub struct SimilarSpec {
    pub input: SimilarInput,
    pub field: String,
    pub top_k: usize,
    pub exact: bool,
    pub ef_search: Option<usize>,
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
    /// Vector similarity search — set by .similar().
    pub similar_query: Option<SimilarSpec>,
    /// Graph-traversal mode — set by instance.traverse(). The vertex variable
    /// is `doc`, so every FILTER/SORT/LIMIT/RETURN path applies unchanged; the
    /// start vertex rides in bind_vars as @__soli_traverse_start.
    pub traversal: Option<TraversalClause>,
    /// Time-bucketed aggregation mode — set by .time_bucket().
    pub time_bucket_info: Option<TimeBucketSpec>,
    /// Multi-key grouping — set by .group_by("f") / .group_by(["a", "b"]).
    /// Mutually exclusive with the legacy 3-arg group_by_info mode.
    pub group_fields: Vec<String>,
    /// Aggregates for the grouped mode — set by .aggregate({...}). With
    /// group_fields set and no aggregates, an implicit `n = COUNT()` applies.
    pub aggregate_specs: Vec<AggregateSpec>,
    /// Post-COLLECT FILTER (HAVING). String references bare group/aggregate
    /// aliases (no doc. prefix); developer-trusted like string-form where().
    pub having: Option<String>,
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
            traversal: None,
            time_bucket_info: None,
            group_fields: Vec::new(),
            aggregate_specs: Vec::new(),
            having: None,
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
            traversal: None,
            time_bucket_info: None,
            group_fields: Vec::new(),
            aggregate_specs: Vec::new(),
            having: None,
        }
    }

    /// The FOR-head shared by build_query / count / exists: a plain collection
    /// scan, or the graph-traversal head when `traversal` is set. Keeping the
    /// vertex variable named `doc` lets every downstream FILTER/SORT/RETURN
    /// path work identically in both modes.
    pub(crate) fn for_head(&self) -> String {
        let collection_str =
            crate::interpreter::symbol_string(self.collection).unwrap_or("unknown");
        match &self.traversal {
            Some(t) => format!(
                "FOR doc, edge IN {}..{} {} @{} {}",
                t.min_depth,
                t.max_depth,
                t.direction.sdbql(),
                super::graph::TRAVERSE_START_BIND,
                t.edge_collection
            ),
            None => format!("FOR doc IN {}", collection_str),
        }
    }

    pub fn set_similar(&mut self, query: String, field: String, top_k: usize) {
        self.similar_query = Some(SimilarSpec {
            input: SimilarInput::Text(query),
            field,
            top_k,
            exact: false,
            ef_search: None,
        });
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
        // Internal machinery binds (`__soli_`-prefixed, e.g. the traversal
        // start vertex) must survive a later .where(); user binds keep the
        // historical replace semantics.
        let preserved: Vec<(SymbolId, serde_json::Value)> = self
            .bind_vars
            .iter()
            .filter(|(k, _)| {
                crate::interpreter::symbol_string(**k)
                    .map(|s| s.starts_with("__soli_"))
                    .unwrap_or(false)
            })
            .map(|(k, v)| (*k, v.clone()))
            .collect();
        self.bind_vars = bind_vars
            .into_iter()
            .map(|(k, v)| (crate::interpreter::get_symbol(&k), v))
            .collect();
        for (k, v) in preserved {
            self.bind_vars.insert(k, v);
        }
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
        let mut query = self.for_head();

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
        let mut query = self.for_head();

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

                // Check if already prefixed with any alias (e.g. doc.field,
                // rel.field, or edge.field in graph traversals)
                if i < len && chars[i] == '.' && (word == "doc" || word == "rel" || word == "edge")
                {
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
    if let Some(ref spec) = qb.similar_query {
        return execute_similar_query(qb, &collection, spec);
    }

    // Time-bucketed aggregation (so the Enumerable array passthrough also
    // works on time_bucket chains).
    if qb.time_bucket_info.is_some() {
        return execute_query_builder_time_bucket(qb);
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

/// Execute a similarity search. With a declared `vector_index` on the field
/// (and no `exact: true` override), the search runs on the DB's HNSW index;
/// otherwise: fetch docs, compute cosine similarity client-side, return
/// top-k (the historical exact path, byte-for-byte).
fn execute_similar_query(qb: &QueryBuilder, collection: &str, spec: &SimilarSpec) -> Value {
    use crate::embedding::{cosine_similarity, generate_embedding};

    let field = spec.field.as_str();
    let top_k = spec.top_k;

    // Resolve the query vector: embed text client-side, or take the literal.
    let query_vec: Vec<f64> = match &spec.input {
        SimilarInput::Text(text) => match generate_embedding(text) {
            Some(v) => v,
            None => return Value::Array(std::rc::Rc::new(std::cell::RefCell::new(Vec::new()))),
        },
        SimilarInput::Vector(v) => v.clone(),
    };

    // ANN pushdown: opt-in via the `vector_index` declaration.
    let class_name = crate::interpreter::symbol_string(qb.class_name)
        .unwrap_or_default()
        .to_string();
    if !spec.exact {
        if let Some(index_def) = super::registry::get_vector_index_for_field(&class_name, field) {
            return execute_similar_pushdown(qb, collection, spec, &index_def.name, &query_vec);
        }
    }

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

/// ANN pushdown for `.similar()` on a model with a declared vector index.
///
/// Without composed filters, hits materialize straight from the search
/// response. With a filter (or soft-delete scope), candidates are
/// over-fetched (4×k, capped at 400), then one standard SDBQL query applies
/// the filters via `doc._key IN @__soli_sim_keys`; scores re-attach and the
/// result re-sorts by score. Filtering happens after candidate selection, so
/// fewer than k rows may come back — `exact: true` is the escape hatch.
fn execute_similar_pushdown(
    qb: &QueryBuilder,
    collection: &str,
    spec: &SimilarSpec,
    index_name: &str,
    query_vec: &[f64],
) -> Value {
    let has_filters = qb.filter.is_some()
        || !qb.joins.is_empty()
        || (qb.is_soft_delete_model && qb.soft_delete_mode != SoftDeleteMode::WithDeleted);

    let fetch_k = if has_filters {
        (spec.top_k * 4).clamp(spec.top_k, 400)
    } else {
        spec.top_k
    };

    let hits = match super::search::exec_vector_search(
        collection,
        index_name,
        query_vec,
        fetch_k,
        spec.ef_search,
    ) {
        Ok(hits) => hits,
        Err(e) => return Value::String(format!("Error: {}", e).into()),
    };

    if !has_filters {
        let values: Vec<Value> = hits
            .iter()
            .take(spec.top_k)
            .map(|hit| {
                let value = match &qb.class {
                    Some(class) => json_doc_to_instance(class, &hit.document),
                    None => super::crud::json_to_value(&hit.document),
                };
                super::search::attach_score(value, hit.score)
            })
            .collect();
        return Value::Array(std::rc::Rc::new(std::cell::RefCell::new(values)));
    }

    // Filtered: candidate keys → standard query with the key-set conjunct.
    let mut scores: HashMap<String, f64> = HashMap::new();
    let keys: Vec<serde_json::Value> = hits
        .iter()
        .map(|h| {
            scores.insert(h.doc_key.clone(), h.score);
            serde_json::Value::String(h.doc_key.clone())
        })
        .collect();

    let mut fetch_qb = qb.clone();
    fetch_qb.similar_query = None;
    fetch_qb.limit_val = None;
    let key_filter = "doc._key IN @__soli_sim_keys".to_string();
    fetch_qb.filter = Some(match &qb.filter {
        Some(existing) => format!("({}) AND {}", existing, key_filter),
        None => key_filter,
    });
    fetch_qb.bind_vars.insert(
        crate::interpreter::get_symbol("__soli_sim_keys"),
        serde_json::Value::Array(keys),
    );

    let (query, bind_vars) = fetch_qb.build_query();
    let results = if let Some(ref class) = qb.class {
        exec_auto_collection_as_instances_with_binds(query, bind_vars, collection, class)
    } else {
        exec_auto_collection_with_binds(query, bind_vars, collection)
    };

    let Value::Array(arr) = results else {
        return results;
    };
    let mut scored: Vec<(f64, Value)> = arr
        .borrow()
        .iter()
        .map(|item| {
            let key = match item {
                Value::Instance(inst) => inst.borrow().get("_key").and_then(|k| match k {
                    Value::String(s) => Some(s.to_string()),
                    _ => None,
                }),
                _ => None,
            };
            let score = key.and_then(|k| scores.get(&k).copied()).unwrap_or(0.0);
            (score, super::search::attach_score(item.clone(), score))
        })
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let values: Vec<Value> = scored
        .into_iter()
        .take(spec.top_k)
        .map(|(_, v)| v)
        .collect();
    Value::Array(std::rc::Rc::new(std::cell::RefCell::new(values)))
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
    let mut query = qb.for_head();

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

    let query = if qb.joins.is_empty() && qb.filter.is_none() && qb.traversal.is_none() {
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

#[derive(Debug, Clone, PartialEq)]
pub enum AggregationFunc {
    Sum,
    Avg,
    Min,
    Max,
    /// COUNT() — no field.
    Count,
    /// COUNT_DISTINCT(doc.field).
    CountDistinct,
    // The list-stats below aren't SDBQL AGGREGATE functions — they are array
    // functions applied to a COLLECT_LIST(...) helper variable in RETURN.
    Median,
    Stddev,
    Variance,
}

impl AggregationFunc {
    /// Parse a user-facing function name. PERCENTILE is intentionally not
    /// here — SolidB has no such function.
    pub fn parse_name(name: &str) -> Option<Self> {
        match name {
            "sum" => Some(Self::Sum),
            "avg" => Some(Self::Avg),
            "min" => Some(Self::Min),
            "max" => Some(Self::Max),
            "count" => Some(Self::Count),
            "count_distinct" => Some(Self::CountDistinct),
            "median" => Some(Self::Median),
            "stddev" => Some(Self::Stddev),
            "variance" => Some(Self::Variance),
            _ => None,
        }
    }

    /// True for functions that must be computed over a COLLECT_LIST helper
    /// variable rather than as a direct AGGREGATE entry.
    pub fn is_list_stat(&self) -> bool {
        matches!(self, Self::Median | Self::Stddev | Self::Variance)
    }

    /// The RETURN-side expression for a list-stat over the helper variable.
    pub fn stat_expr(&self, list_var: &str) -> String {
        match self {
            Self::Median => format!("MEDIAN({})", list_var),
            Self::Stddev => format!("STDDEV({})", list_var),
            Self::Variance => format!("VARIANCE({})", list_var),
            _ => unreachable!("stat_expr is only defined for list stats"),
        }
    }

    pub fn to_sdbql(&self, field: &str) -> String {
        match self {
            AggregationFunc::Sum => format!("SUM(doc.{})", field),
            AggregationFunc::Avg => format!("AVG(doc.{})", field),
            AggregationFunc::Min => format!("MIN(doc.{})", field),
            AggregationFunc::Max => format!("MAX(doc.{})", field),
            AggregationFunc::Count => "COUNT()".to_string(),
            AggregationFunc::CountDistinct => format!("COUNT_DISTINCT(doc.{})", field),
            // List stats collect the values; the caller applies stat_expr.
            AggregationFunc::Median | AggregationFunc::Stddev | AggregationFunc::Variance => {
                format!("COLLECT_LIST(doc.{})", field)
            }
        }
    }
}

/// One `alias: [func, field]` entry of an `.aggregate({...})` spec.
#[derive(Debug, Clone)]
pub struct AggregateSpec {
    pub alias: String,
    pub func: AggregationFunc,
    /// Empty for Count.
    pub field: String,
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

    if func.is_list_stat() {
        // MEDIAN/STDDEV/VARIANCE are array functions — collect the values
        // first, then apply the function to the helper variable.
        query.push_str(&format!(
            " COLLECT AGGREGATE __soli_vals = COLLECT_LIST(doc.{}) RETURN {}",
            field,
            func.stat_expr("__soli_vals")
        ));
    } else if matches!(
        func,
        AggregationFunc::Count | AggregationFunc::CountDistinct
    ) {
        query.push_str(&format!(
            " COLLECT AGGREGATE __soli_agg = {} RETURN __soli_agg",
            func.to_sdbql(field)
        ));
    } else {
        query.push_str(&format!(" RETURN {}", func.to_sdbql(field)));
    }

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

impl QueryBuilder {
    /// Build the TIME_BUCKET COLLECT query for a timeseries model:
    ///
    /// ```text
    /// FOR doc IN metrics FILTER ...
    /// COLLECT bucket = TIME_BUCKET(doc._created_at, "1h")
    /// AGGREGATE avg = AVG(doc.value), n = COUNT()
    /// SORT bucket RETURN {bucket: bucket, avg: avg, n: n}
    /// ```
    ///
    /// Interval, timestamp field, aliases, and aggregate fields are all
    /// validated identifiers/durations at .time_bucket() time, so plain
    /// formatting here is injection-safe.
    pub fn build_time_bucket_query(&self) -> (String, HashMap<String, serde_json::Value>) {
        let spec = self
            .time_bucket_info
            .as_ref()
            .expect("build_time_bucket_query requires time_bucket_info");
        let mut query = self.for_head();

        if let Some(filter) = &self.filter {
            let aql_filter = filter.replace(" && ", " AND ").replace(" || ", " OR ");
            let aql_filter = Self::normalize_equality_ops(&aql_filter);
            let aql_filter = Self::prefix_bare_fields(&aql_filter);
            query.push_str(&format!(" FILTER {}", aql_filter));
        }

        query.push_str(&format!(
            " COLLECT bucket = TIME_BUCKET(doc.{}, \"{}\")",
            spec.timestamp_field, spec.interval
        ));

        let agg_exprs: Vec<String> = spec
            .aggregates
            .iter()
            .map(|(alias, func, field)| {
                if func == "COUNT" {
                    format!("{} = COUNT()", alias)
                } else {
                    format!("{} = {}(doc.{})", alias, func, field)
                }
            })
            .collect();
        query.push_str(&format!(" AGGREGATE {}", agg_exprs.join(", ")));

        let mut return_fields = vec!["bucket: bucket".to_string()];
        for (alias, _, _) in &spec.aggregates {
            return_fields.push(format!("{}: {}", alias, alias));
        }
        query.push_str(&format!(
            " SORT bucket RETURN {{{}}}",
            return_fields.join(", ")
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
}

/// Parse the (interval, aggregates?) arguments of `time_bucket` into a
/// TimeBucketSpec. Shared by the static `Model.time_bucket` and the
/// QueryBuilder `.time_bucket` so both accept the same shapes:
/// interval "<n><s|m|h|d>", aggregates hash of sum/avg/min/max → field
/// (+ count: true). No aggregates → count per bucket.
pub fn parse_time_bucket_args(
    interval_arg: &Value,
    aggregates_arg: Option<&Value>,
    class_name: &str,
) -> Result<TimeBucketSpec, String> {
    use crate::interpreter::value::HashKey;

    let interval = match interval_arg {
        Value::String(s) => s.to_string(),
        other => {
            return Err(format!(
                "time_bucket() interval must be a string like \"5m\", got {}",
                other.type_name()
            ))
        }
    };
    // Match the DB's TIME_BUCKET contract: <n><s|m|h|d>, n > 0.
    let valid_interval = interval.len() >= 2
        && interval.ends_with(['s', 'm', 'h', 'd'])
        && interval[..interval.len() - 1]
            .parse::<u64>()
            .map(|n| n > 0)
            .unwrap_or(false);
    if !valid_interval {
        return Err(format!(
            "time_bucket() invalid interval {:?}: expected <number><unit> with unit s/m/h/d, \
             e.g. \"5m\"",
            interval
        ));
    }

    let mut aggregates: Vec<(String, String, String)> = Vec::new();
    if let Some(arg) = aggregates_arg {
        let hash = match arg {
            Value::Hash(h) => h.clone(),
            other => {
                return Err(format!(
                    "time_bucket() aggregates must be a hash, got {}",
                    other.type_name()
                ))
            }
        };
        for (k, v) in hash.borrow().iter() {
            let func_key = match k {
                HashKey::String(s) => s.to_string(),
                _ => return Err("time_bucket() aggregate keys must be strings".to_string()),
            };
            match func_key.as_str() {
                "sum" | "avg" | "min" | "max" => {
                    let field = match v {
                        Value::String(s) => s.to_string(),
                        other => {
                            return Err(format!(
                                "time_bucket() aggregate '{}' expects a field name, got {}",
                                func_key,
                                other.type_name()
                            ))
                        }
                    };
                    super::core::validate_field_name(&field, "time_bucket")?;
                    aggregates.push((func_key.clone(), func_key.to_uppercase(), field));
                }
                "count" => {
                    aggregates.push(("count".to_string(), "COUNT".to_string(), String::new()));
                }
                other => {
                    return Err(format!(
                        "time_bucket() unknown aggregate '{}': expected sum, avg, min, max, or \
                         count",
                        other
                    ))
                }
            }
        }
    }
    if aggregates.is_empty() {
        // Bare time_bucket("1h") counts rows per bucket.
        aggregates.push(("count".to_string(), "COUNT".to_string(), String::new()));
    }

    // Timestamp field: the model's `timeseries timestamp:` declaration, else
    // the server-set _created_at.
    let timestamp_field = super::registry::get_timeseries_spec(class_name)
        .and_then(|s| s.timestamp_field)
        .unwrap_or_else(|| "_created_at".to_string());

    Ok(TimeBucketSpec {
        interval,
        timestamp_field,
        aggregates,
    })
}

/// Parse an `.aggregate({...})` spec hash into AggregateSpecs. Entries are
/// `alias: [func]` (count) or `alias: [func, field]`; every alias, function
/// name, and field is identifier-validated so nothing user-controlled can
/// reach the query text.
pub fn parse_aggregate_spec_hash(value: &Value) -> Result<Vec<AggregateSpec>, String> {
    use crate::interpreter::value::HashKey;

    let hash = match value {
        Value::Hash(h) => h.clone(),
        other => {
            return Err(format!(
                "aggregate() expects a hash of alias: [func, field] entries, got {}",
                other.type_name()
            ))
        }
    };

    let mut specs = Vec::new();
    for (k, v) in hash.borrow().iter() {
        let alias = match k {
            HashKey::String(s) => s.to_string(),
            _ => return Err("aggregate() aliases must be strings".to_string()),
        };
        // Aliases become COLLECT variable names — same identifier grammar.
        super::core::validate_field_name(&alias, "aggregate")?;

        let parts: Vec<Value> = match v {
            Value::Array(arr) => arr.borrow().clone(),
            Value::String(s) => vec![Value::String(s.clone())],
            other => {
                return Err(format!(
                    "aggregate() entry '{}' must be [func, field] (or [\"count\"]), got {}",
                    alias,
                    other.type_name()
                ))
            }
        };
        let func_name = match parts.first() {
            Some(Value::String(s)) => s.to_string(),
            _ => {
                return Err(format!(
                    "aggregate() entry '{}' must start with a function name string",
                    alias
                ))
            }
        };
        if func_name == "percentile" {
            return Err(
                "aggregate(): percentile is not supported by SolidB — available functions: \
                 sum, avg, min, max, count, count_distinct, median, stddev, variance"
                    .to_string(),
            );
        }
        let func = AggregationFunc::parse_name(&func_name).ok_or_else(|| {
            format!(
                "aggregate() unknown function '{}': expected sum, avg, min, max, count, \
                 count_distinct, median, stddev, or variance",
                func_name
            )
        })?;

        let field = if matches!(func, AggregationFunc::Count) {
            if parts.len() > 1 {
                return Err(format!(
                    "aggregate() entry '{}': count takes no field — use [\"count\"]",
                    alias
                ));
            }
            String::new()
        } else {
            let field = match parts.get(1) {
                Some(Value::String(s)) => s.to_string(),
                _ => {
                    return Err(format!(
                        "aggregate() entry '{}': {} requires a field name, e.g. [{:?}, \
                         \"amount\"]",
                        alias, func_name, func_name
                    ))
                }
            };
            super::core::validate_field_name(&field, "aggregate")?;
            field
        };

        specs.push(AggregateSpec { alias, func, field });
    }
    if specs.is_empty() {
        return Err("aggregate() requires at least one alias: [func, field] entry".to_string());
    }
    Ok(specs)
}

impl QueryBuilder {
    /// Build the grouped (multi-key COLLECT + AGGREGATE) query:
    ///
    /// ```text
    /// FOR doc IN orders FILTER doc.status == @s
    /// COLLECT country = doc.country, plan = doc.plan
    /// AGGREGATE total = SUM(doc.amount), n = COUNT()
    /// FILTER total > @min          -- .having()
    /// SORT total DESC LIMIT 20
    /// RETURN {country: country, plan: plan, total: total, n: n}
    /// ```
    ///
    /// List stats (median/stddev/variance) collect their values into a
    /// `__soli_vals_<alias>` helper and apply the array function in RETURN.
    /// Errors on an ORDER field that is neither a group key nor an alias.
    pub fn build_grouped_query(
        &self,
    ) -> Result<(String, HashMap<String, serde_json::Value>), String> {
        let mut query = self.for_head();

        // Join existence filters compose pre-COLLECT like in build_query.
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

        // Unlike the legacy 3-arg group_by, grouped queries respect the
        // soft-delete scope (legacy is left as-is for backwards compat).
        if self.is_soft_delete_model {
            match self.soft_delete_mode {
                SoftDeleteMode::Default => query.push_str(" FILTER doc.deleted_at == null"),
                SoftDeleteMode::OnlyDeleted => query.push_str(" FILTER doc.deleted_at != null"),
                SoftDeleteMode::WithDeleted => {}
            }
        }

        // Implicit count when grouping without explicit aggregates.
        let implicit_count = AggregateSpec {
            alias: "n".to_string(),
            func: AggregationFunc::Count,
            field: String::new(),
        };
        let specs: Vec<&AggregateSpec> = if self.aggregate_specs.is_empty() {
            vec![&implicit_count]
        } else {
            self.aggregate_specs.iter().collect()
        };

        let group_part: Vec<String> = self
            .group_fields
            .iter()
            .map(|f| format!("{} = doc.{}", f, f))
            .collect();
        if group_part.is_empty() {
            query.push_str(" COLLECT");
        } else {
            query.push_str(&format!(" COLLECT {}", group_part.join(", ")));
        }

        let agg_part: Vec<String> = specs
            .iter()
            .map(|s| {
                if s.func.is_list_stat() {
                    format!("__soli_vals_{} = COLLECT_LIST(doc.{})", s.alias, s.field)
                } else {
                    format!("{} = {}", s.alias, s.func.to_sdbql(&s.field))
                }
            })
            .collect();
        query.push_str(&format!(" AGGREGATE {}", agg_part.join(", ")));

        // HAVING — post-COLLECT filter over bare aliases (no doc. prefixing).
        if let Some(having) = &self.having {
            let having = having.replace(" && ", " AND ").replace(" || ", " OR ");
            let having = Self::normalize_equality_ops(&having);
            query.push_str(&format!(" FILTER {}", having));
        }

        // SORT — bare group key or aggregate alias only.
        if let Some((field, direction)) = &self.order_by {
            let field_str = crate::interpreter::symbol_string(*field)
                .unwrap_or("unknown")
                .to_string();
            let known = self.group_fields.iter().any(|f| f == &field_str)
                || specs.iter().any(|s| s.alias == field_str);
            if !known {
                return Err(format!(
                    "order({:?}) in a grouped query must name a group field or aggregate \
                     alias (have: {})",
                    field_str,
                    self.group_fields
                        .iter()
                        .cloned()
                        .chain(specs.iter().map(|s| s.alias.clone()))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            let dir_str = crate::interpreter::symbol_string(*direction).unwrap_or("asc");
            let dir = match dir_str.to_lowercase().as_str() {
                "desc" | "descending" => "DESC",
                _ => "ASC",
            };
            query.push_str(&format!(" SORT {} {}", field_str, dir));
        }

        if let Some(limit) = self.limit_val {
            if let Some(offset) = self.offset_val {
                query.push_str(&format!(" LIMIT {}, {}", offset, limit));
            } else {
                query.push_str(&format!(" LIMIT {}", limit));
            }
        } else if let Some(offset) = self.offset_val {
            query.push_str(&format!(" LIMIT {}, 1000000", offset));
        }

        let mut return_fields: Vec<String> = self
            .group_fields
            .iter()
            .map(|f| format!("{}: {}", f, f))
            .collect();
        for s in &specs {
            if s.func.is_list_stat() {
                return_fields.push(format!(
                    "{}: {}",
                    s.alias,
                    s.func.stat_expr(&format!("__soli_vals_{}", s.alias))
                ));
            } else {
                return_fields.push(format!("{}: {}", s.alias, s.alias));
            }
        }
        query.push_str(&format!(" RETURN {{{}}}", return_fields.join(", ")));

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

        Ok((query, bind_vars_str))
    }
}

/// Execute a grouped (group_fields/aggregate_specs) QueryBuilder. Returns
/// raw hash rows — one per group, keyed by group fields + aliases.
pub fn execute_query_builder_grouped(qb: &QueryBuilder) -> Result<Value, String> {
    let collection = crate::interpreter::symbol_string(qb.collection)
        .unwrap_or("unknown")
        .to_string();
    let (query, bind_vars_str) = qb.build_grouped_query()?;

    if super::batch::is_active() {
        return Ok(super::batch::register(
            query,
            bind_vars_str,
            Box::new(move |rows| {
                let values: Vec<Value> = rows.iter().map(super::crud::json_to_value).collect();
                Ok(Value::Array(std::rc::Rc::new(std::cell::RefCell::new(
                    values,
                ))))
            }),
        ));
    }

    Ok(if bind_vars_str.is_empty() {
        exec_auto_collection(query, &collection)
    } else {
        exec_auto_collection_with_binds(query, bind_vars_str, &collection)
    })
}

/// Execute a time_bucket QueryBuilder. Returns raw hash rows
/// ({bucket, <alias>...}) like group_by — buckets aren't documents.
pub fn execute_query_builder_time_bucket(qb: &QueryBuilder) -> Value {
    let collection = crate::interpreter::symbol_string(qb.collection)
        .unwrap_or("unknown")
        .to_string();
    let (query, bind_vars_str) = qb.build_time_bucket_query();

    if super::batch::is_active() {
        return super::batch::register(
            query,
            bind_vars_str,
            Box::new(move |rows| {
                let values: Vec<Value> = rows.iter().map(super::crud::json_to_value).collect();
                Ok(Value::Array(std::rc::Rc::new(std::cell::RefCell::new(
                    values,
                ))))
            }),
        );
    }

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

    // ---- graph traversal mode ----

    fn make_traversal_qb(direction: super::super::graph::TraversalDirection) -> QueryBuilder {
        let mut qb = make_qb("User", "follows");
        qb.bind_vars.insert(
            crate::interpreter::get_symbol(super::super::graph::TRAVERSE_START_BIND),
            serde_json::Value::String("users/alice".to_string()),
        );
        qb.traversal = Some(TraversalClause {
            edge_collection: "follows".to_string(),
            direction,
            min_depth: 1,
            max_depth: 1,
        });
        qb
    }

    #[test]
    fn test_traversal_basic_outbound() {
        let qb = make_traversal_qb(super::super::graph::TraversalDirection::Out);
        let (query, binds) = qb.build_query();
        assert_eq!(
            query,
            "FOR doc, edge IN 1..1 OUTBOUND @__soli_traverse_start follows RETURN doc"
        );
        assert_eq!(binds["__soli_traverse_start"], "users/alice");
    }

    #[test]
    fn test_traversal_depth_range_and_direction() {
        let mut qb = make_traversal_qb(super::super::graph::TraversalDirection::Any);
        if let Some(t) = qb.traversal.as_mut() {
            t.min_depth = 2;
            t.max_depth = 3;
        }
        let (query, _) = qb.build_query();
        assert!(query.starts_with("FOR doc, edge IN 2..3 ANY @__soli_traverse_start follows"));
    }

    #[test]
    fn test_traversal_with_filter_sort_limit() {
        let mut qb = make_traversal_qb(super::super::graph::TraversalDirection::In);
        let mut binds = HashMap::new();
        binds.insert("act".to_string(), serde_json::Value::Bool(true));
        qb.set_filter("active = @act".to_string(), binds);
        qb.set_order("name".to_string(), "asc".to_string());
        qb.set_limit(10);
        let (query, binds) = qb.build_query();
        assert_eq!(
            query,
            "FOR doc, edge IN 1..1 INBOUND @__soli_traverse_start follows \
             FILTER doc.active == @act SORT doc.name ASC LIMIT 10 RETURN doc"
        );
        assert!(binds.contains_key("act"));
        assert!(binds.contains_key("__soli_traverse_start"));
    }

    #[test]
    fn test_traversal_soft_delete_filter() {
        let mut qb = make_traversal_qb(super::super::graph::TraversalDirection::Out);
        qb.is_soft_delete_model = true;
        let (query, _) = qb.build_query();
        assert!(query.contains("FILTER doc.deleted_at == null"));
    }

    #[test]
    fn test_traversal_edge_attribute_filter_passthrough() {
        let mut qb = make_traversal_qb(super::super::graph::TraversalDirection::Out);
        let mut binds = HashMap::new();
        binds.insert("y".to_string(), serde_json::Value::from(2024));
        qb.set_filter("edge.since >= @y".to_string(), binds);
        let (query, _) = qb.build_query();
        assert!(
            query.contains("FILTER edge.since >= @y"),
            "edge.-prefixed refs must not be doc.-prefixed: {}",
            query
        );
    }

    #[test]
    fn test_traversal_exists_query() {
        let qb = make_traversal_qb(super::super::graph::TraversalDirection::Out);
        let (query, _) = qb.build_exists_query();
        assert_eq!(
            query,
            "FOR doc, edge IN 1..1 OUTBOUND @__soli_traverse_start follows LIMIT 1 RETURN true"
        );
    }

    // ---- time_bucket mode ----

    #[test]
    fn test_time_bucket_single_aggregate() {
        let mut qb = make_qb("Metric", "metrics");
        qb.time_bucket_info = Some(TimeBucketSpec {
            interval: "1h".to_string(),
            timestamp_field: "_created_at".to_string(),
            aggregates: vec![("avg".to_string(), "AVG".to_string(), "value".to_string())],
        });
        let (query, _) = qb.build_time_bucket_query();
        assert_eq!(
            query,
            "FOR doc IN metrics COLLECT bucket = TIME_BUCKET(doc._created_at, \"1h\") \
             AGGREGATE avg = AVG(doc.value) SORT bucket RETURN {bucket: bucket, avg: avg}"
        );
    }

    #[test]
    fn test_time_bucket_multi_aggregate_count_and_filter() {
        let mut qb = make_qb("Metric", "metrics");
        let mut binds = HashMap::new();
        binds.insert(
            "d".to_string(),
            serde_json::Value::String("srv1".to_string()),
        );
        qb.set_filter("device = @d".to_string(), binds);
        qb.time_bucket_info = Some(TimeBucketSpec {
            interval: "5m".to_string(),
            timestamp_field: "recorded_at".to_string(),
            aggregates: vec![
                ("avg".to_string(), "AVG".to_string(), "value".to_string()),
                ("count".to_string(), "COUNT".to_string(), String::new()),
            ],
        });
        let (query, binds) = qb.build_time_bucket_query();
        assert_eq!(
            query,
            "FOR doc IN metrics FILTER doc.device == @d COLLECT bucket = \
             TIME_BUCKET(doc.recorded_at, \"5m\") AGGREGATE avg = AVG(doc.value), \
             count = COUNT() SORT bucket RETURN {bucket: bucket, avg: avg, count: count}"
        );
        assert!(binds.contains_key("d"));
    }

    // ---- grouped (multi-key COLLECT) mode ----

    #[test]
    fn test_grouped_multikey_multi_aggregate_having_sort_limit() {
        let mut qb = make_qb("Order", "orders");
        let mut binds = HashMap::new();
        binds.insert(
            "s".to_string(),
            serde_json::Value::String("paid".to_string()),
        );
        qb.set_filter("status = @s".to_string(), binds);
        qb.group_fields = vec!["country".to_string(), "plan".to_string()];
        qb.aggregate_specs = vec![
            AggregateSpec {
                alias: "total".to_string(),
                func: AggregationFunc::Sum,
                field: "amount".to_string(),
            },
            AggregateSpec {
                alias: "n".to_string(),
                func: AggregationFunc::Count,
                field: String::new(),
            },
        ];
        qb.having = Some("total > @min".to_string());
        qb.bind_vars.insert(
            crate::interpreter::get_symbol("min"),
            serde_json::json!(1000),
        );
        qb.set_order("total".to_string(), "desc".to_string());
        qb.set_limit(20);
        let (query, binds) = qb.build_grouped_query().unwrap();
        assert_eq!(
            query,
            "FOR doc IN orders FILTER doc.status == @s COLLECT country = doc.country, \
             plan = doc.plan AGGREGATE total = SUM(doc.amount), n = COUNT() \
             FILTER total > @min SORT total DESC LIMIT 20 \
             RETURN {country: country, plan: plan, total: total, n: n}"
        );
        assert!(binds.contains_key("s") && binds.contains_key("min"));
    }

    #[test]
    fn test_grouped_implicit_count_and_soft_delete() {
        let mut qb = make_qb("User", "users");
        qb.is_soft_delete_model = true;
        qb.group_fields = vec!["role".to_string()];
        let (query, _) = qb.build_grouped_query().unwrap();
        assert_eq!(
            query,
            "FOR doc IN users FILTER doc.deleted_at == null COLLECT role = doc.role \
             AGGREGATE n = COUNT() RETURN {role: role, n: n}"
        );
    }

    #[test]
    fn test_grouped_list_stats_emit_collect_list() {
        let mut qb = make_qb("Order", "orders");
        qb.group_fields = vec!["country".to_string()];
        qb.aggregate_specs = vec![AggregateSpec {
            alias: "med".to_string(),
            func: AggregationFunc::Median,
            field: "amount".to_string(),
        }];
        let (query, _) = qb.build_grouped_query().unwrap();
        assert_eq!(
            query,
            "FOR doc IN orders COLLECT country = doc.country AGGREGATE __soli_vals_med = \
             COLLECT_LIST(doc.amount) RETURN {country: country, med: MEDIAN(__soli_vals_med)}"
        );
    }

    #[test]
    fn test_grouped_ungrouped_multi_aggregate() {
        let mut qb = make_qb("Order", "orders");
        qb.aggregate_specs = vec![AggregateSpec {
            alias: "total".to_string(),
            func: AggregationFunc::Sum,
            field: "amount".to_string(),
        }];
        let (query, _) = qb.build_grouped_query().unwrap();
        assert_eq!(
            query,
            "FOR doc IN orders COLLECT AGGREGATE total = SUM(doc.amount) \
             RETURN {total: total}"
        );
    }

    #[test]
    fn test_grouped_order_must_be_key_or_alias() {
        let mut qb = make_qb("Order", "orders");
        qb.group_fields = vec!["country".to_string()];
        qb.set_order("amount".to_string(), "asc".to_string());
        let err = qb.build_grouped_query().unwrap_err();
        assert!(err.contains("group field or aggregate alias"), "{}", err);
    }

    #[test]
    fn test_legacy_group_by_query_unchanged() {
        // Regression pin: the 3-arg group_by emission must stay byte-stable.
        let qb = make_qb("Order", "orders");
        let (query, _) = qb.build_group_by_query("country", AggregationFunc::Sum, "amount");
        assert_eq!(
            query,
            "FOR doc IN orders COLLECT group = doc.country AGGREGATE result = \
             SUM(doc.amount) RETURN {group: group, result: result}"
        );
    }

    #[test]
    fn test_parse_aggregate_spec_hash_validation() {
        fn hash_of(alias: &str, parts: Vec<Value>) -> Value {
            let mut pairs = crate::interpreter::value::HashPairs::default();
            pairs.insert(
                crate::interpreter::value::HashKey::String(alias.into()),
                Value::Array(Rc::new(std::cell::RefCell::new(parts))),
            );
            Value::Hash(Rc::new(std::cell::RefCell::new(pairs)))
        }
        // count takes no field
        let specs =
            parse_aggregate_spec_hash(&hash_of("n", vec![Value::String("count".into())])).unwrap();
        assert_eq!(specs[0].alias, "n");
        assert!(matches!(specs[0].func, AggregationFunc::Count));
        // percentile explicitly rejected
        let err = parse_aggregate_spec_hash(&hash_of(
            "p95",
            vec![
                Value::String("percentile".into()),
                Value::String("x".into()),
            ],
        ))
        .unwrap_err();
        assert!(err.contains("percentile"), "{}", err);
        // unknown func rejected
        assert!(parse_aggregate_spec_hash(&hash_of(
            "x",
            vec![Value::String("mode".into()), Value::String("f".into())]
        ))
        .is_err());
        // non-count without field rejected
        assert!(
            parse_aggregate_spec_hash(&hash_of("t", vec![Value::String("sum".into())])).is_err()
        );
    }

    #[test]
    fn test_parse_time_bucket_args_validation() {
        // Bad intervals rejected.
        for bad in ["5x", "m", "0m", "", "5"] {
            assert!(
                parse_time_bucket_args(&Value::String(bad.into()), None, "Metric").is_err(),
                "interval {:?} should be rejected",
                bad
            );
        }
        // No aggregates → count per bucket.
        let spec = parse_time_bucket_args(&Value::String("1d".into()), None, "Metric").unwrap();
        assert_eq!(
            spec.aggregates,
            vec![("count".to_string(), "COUNT".to_string(), String::new())]
        );
        assert_eq!(spec.timestamp_field, "_created_at");
        // Unknown aggregate key rejected.
        let mut pairs = crate::interpreter::value::HashPairs::default();
        pairs.insert(
            crate::interpreter::value::HashKey::String("median".into()),
            Value::String("value".into()),
        );
        let aggs = Value::Hash(Rc::new(std::cell::RefCell::new(pairs)));
        assert!(
            parse_time_bucket_args(&Value::String("1h".into()), Some(&aggs), "Metric").is_err()
        );
    }
}
