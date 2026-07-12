//! Graph-augmented retrieval for the Model layer.
//!
//! `Model.graph_rag(query, { via:, ... })` seeds with vector ANN search, expands
//! each hit through an edge model's traversal, re-ranks the union by cosine
//! similarity, and returns instances carrying `_similarity_score`, `_graph_seed`,
//! and `_graph_hops`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::embedding::{cosine_similarity, generate_embedding};
use crate::interpreter::value::{Class, HashKey, Value};

use super::graph::{build_traverse_qb_from_seed, TraversalDirection};
use super::query::execute_query_builder;
use super::search;

/// Parsed options for `Model.graph_rag(query, options)`.
pub struct GraphRagOptions {
    pub via: Value,
    pub direction: TraversalDirection,
    pub min_depth: usize,
    pub max_depth: usize,
    pub field: String,
    pub seed_k: usize,
    pub limit: usize,
    pub explicit_vector: Option<Vec<f64>>,
}

fn instance_key(value: &Value) -> Option<String> {
    match value {
        Value::Instance(inst) => inst.borrow().get("_key").and_then(|v| match v {
            Value::String(s) => Some(s.to_string()),
            _ => None,
        }),
        _ => None,
    }
}

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

fn doc_vector(value: &Value, field: &str) -> Vec<f64> {
    match value {
        Value::Instance(inst) => inst
            .borrow()
            .get(field)
            .map(|v| value_to_float_vec(&v))
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn attach_graph_rag_meta(value: Value, score: f64, is_seed: bool, hops: i64) -> Value {
    if let Value::Instance(inst) = &value {
        let mut fields = inst.borrow().fields.clone();
        fields.insert("_similarity_score".to_string(), Value::Float(score));
        fields.insert("_graph_seed".to_string(), Value::Bool(is_seed));
        fields.insert("_graph_hops".to_string(), Value::Int(hops));
        return Value::Instance(Rc::new(RefCell::new(crate::interpreter::value::Instance {
            class: inst.borrow().class.clone(),
            fields,
            original_fields: inst.borrow().original_fields.clone(),
            previous_changes: inst.borrow().previous_changes.clone(),
        })));
    }
    value
}

/// Execute graph-augmented retrieval: ANN seeds → traverse expand → re-rank.
pub fn exec_graph_rag(
    class: &Rc<Class>,
    class_name: &str,
    collection: &str,
    query_text: &str,
    opts: GraphRagOptions,
) -> Result<Value, String> {
    let vindex =
        super::registry::get_vector_index_for_field(class_name, &opts.field).ok_or_else(|| {
            format!(
                "{}.graph_rag requires a `vector_index` declaration on field '{}'",
                class_name, opts.field
            )
        })?;

    let query_vec: Vec<f64> = if let Some(v) = opts.explicit_vector {
        v
    } else {
        generate_embedding(query_text).ok_or_else(|| {
            "graph_rag: embedding failed — set SOLI_EMBEDDING_API_KEY or pass vector: in options"
                .to_string()
        })?
    };

    // --- Seed: ANN over the collection index ---
    let seed_hits =
        search::exec_vector_search(collection, &vindex.name, &query_vec, opts.seed_k, None)?;

    let mut union: HashMap<String, (Value, bool, i64)> = HashMap::new();

    for hit in seed_hits {
        let inst = super::crud::json_doc_to_instance(class, &hit.document);
        if let Some(key) = instance_key(&inst) {
            union.insert(key, (inst, true, 0));
        }
    }

    // --- Expand: traverse from each seed through the edge model ---
    for (seed_val, is_seed, _) in union.clone().values() {
        if !is_seed {
            continue;
        }
        let Value::Instance(seed) = seed_val else {
            continue;
        };
        let qb = build_traverse_qb_from_seed(
            seed.clone(),
            &opts.via,
            opts.direction,
            opts.min_depth,
            opts.max_depth,
        )?;
        let expanded = execute_query_builder(&qb);
        if let Value::Array(arr) = expanded {
            for item in arr.borrow().iter() {
                if let Some(key) = instance_key(item) {
                    union.entry(key).or_insert((item.clone(), false, 1));
                }
            }
        }
    }

    // --- Re-rank by cosine similarity against the query vector ---
    let mut scored: Vec<(f64, Value, bool, i64)> = union
        .into_values()
        .filter_map(|(inst, is_seed, hops)| {
            let vec = doc_vector(&inst, &opts.field);
            if vec.is_empty() || vec.len() != query_vec.len() {
                return None;
            }
            let score = cosine_similarity(&query_vec, &vec);
            Some((score, inst, is_seed, hops))
        })
        .collect();

    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    let results: Vec<Value> = scored
        .into_iter()
        .take(opts.limit)
        .map(|(score, inst, is_seed, hops)| attach_graph_rag_meta(inst, score, is_seed, hops))
        .collect();

    Ok(Value::Array(Rc::new(RefCell::new(results))))
}

/// Parse the options hash for `graph_rag`.
pub fn parse_graph_rag_options(
    class_name: &str,
    opts: Option<&Value>,
) -> Result<GraphRagOptions, String> {
    let mut via: Option<Value> = None;
    let mut direction = TraversalDirection::Out;
    let mut min_depth: usize = 1;
    let mut max_depth: usize = 1;
    let mut field = "embedding".to_string();
    let mut seed_k: usize = 5;
    let mut limit: usize = 10;
    let mut explicit_vector: Option<Vec<f64>> = None;

    let Some(Value::Hash(hash)) = opts else {
        return Err(format!(
            "{}.graph_rag requires an options hash with via: EdgeModel",
            class_name
        ));
    };

    for (k, v) in hash.borrow().iter() {
        let key = match k {
            HashKey::String(s) => s.to_string(),
            _ => continue,
        };
        match (key.as_str(), v) {
            ("via", Value::Class(_) | Value::String(_)) => via = Some(v.clone()),
            ("direction", Value::String(s)) => {
                direction = TraversalDirection::parse(s)?;
            }
            ("direction", Value::Symbol(s)) => {
                direction = TraversalDirection::parse(s)?;
            }
            ("depth", Value::Int(n)) => {
                if *n < 1 {
                    return Err("graph_rag() depth must be >= 1".to_string());
                }
                min_depth = 1;
                max_depth = *n as usize;
            }
            ("depth", Value::Array(arr)) => {
                let ints: Vec<i64> = arr
                    .borrow()
                    .iter()
                    .map(|v| match v {
                        Value::Int(n) => Ok(*n),
                        other => Err(format!(
                            "graph_rag() depth array must hold ints, got {}",
                            other.type_name()
                        )),
                    })
                    .collect::<Result<_, _>>()?;
                let (min, max) = match (ints.iter().min(), ints.iter().max()) {
                    (Some(&min), Some(&max)) => (min, max),
                    _ => return Err("graph_rag() depth array is empty".to_string()),
                };
                if min < 1 {
                    return Err("graph_rag() depth must be >= 1".to_string());
                }
                min_depth = min as usize;
                max_depth = max as usize;
            }
            ("field", Value::String(s)) => field = s.to_string(),
            ("seed_k", Value::Int(n)) if *n > 0 => seed_k = *n as usize,
            ("limit", Value::Int(n)) if *n > 0 => limit = *n as usize,
            ("vector", Value::Array(arr)) => {
                let vec: Vec<f64> = arr
                    .borrow()
                    .iter()
                    .map(|v| match v {
                        Value::Int(n) => Ok(*n as f64),
                        Value::Float(f) => Ok(*f),
                        other => Err(format!(
                            "graph_rag() vector entries must be numbers, got {}",
                            other.type_name()
                        )),
                    })
                    .collect::<Result<_, _>>()?;
                if vec.is_empty() {
                    return Err("graph_rag() vector is empty".to_string());
                }
                explicit_vector = Some(vec);
            }
            ("via", other) => {
                return Err(format!(
                    "graph_rag() via: must be an edge model class, got {}",
                    other.type_name()
                ));
            }
            (other, _) => {
                return Err(format!(
                    "graph_rag() unknown/invalid option '{}': expected via:, direction:, \
                     depth:, field:, seed_k:, limit:, or vector:",
                    other
                ));
            }
        }
    }

    let via = via.ok_or_else(|| {
        format!(
            "{}.graph_rag requires via: EdgeModel in the options hash",
            class_name
        )
    })?;

    super::core::validate_field_name(&field, "graph_rag")?;

    Ok(GraphRagOptions {
        via,
        direction,
        min_depth,
        max_depth,
        field,
        seed_k,
        limit,
        explicit_vector,
    })
}
