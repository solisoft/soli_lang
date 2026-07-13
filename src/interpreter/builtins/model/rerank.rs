//! Client-side lexical reranking for the Model layer.
//!
//! `rerank(query, docs [, { field:, limit: }])` reorders an array of records
//! (model instances or hashes) by how many query tokens each record's text
//! contains — most-overlap first, stable on ties. Pure and offline (no server
//! round-trip, no LLM), mirroring SolidB's `RERANK` lexical mode. Handy after
//! `similar` / `graph_rag` / `hybrid` to re-order retrieved rows by a phrase.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use crate::interpreter::value::{HashKey, Value};

fn tokenize(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 2)
        .map(|w| w.to_lowercase())
        .collect()
}

/// Read a field off a record (model instance or hash).
fn field_lookup(val: &Value, field: &str) -> Option<Value> {
    match val {
        Value::Instance(inst) => inst.borrow().get(field),
        Value::Hash(h) => {
            for (k, v) in h.borrow().iter() {
                if let HashKey::String(s) = k {
                    if &**s == field {
                        return Some(v.clone());
                    }
                }
            }
            None
        }
        _ => None,
    }
}

fn as_text(val: &Value) -> Option<String> {
    match val {
        Value::String(s) => Some(s.to_string()),
        _ => None,
    }
}

/// Text to rank a record by: the named `field`, else the first non-empty of a
/// handful of common text fields.
fn record_text(val: &Value, field: Option<&str>) -> String {
    if let Some(f) = field {
        return field_lookup(val, f)
            .as_ref()
            .and_then(as_text)
            .unwrap_or_default();
    }
    for cand in ["content", "text", "summary", "body", "title"] {
        if let Some(t) = field_lookup(val, cand).as_ref().and_then(as_text) {
            if !t.trim().is_empty() {
                return t;
            }
        }
    }
    String::new()
}

/// `rerank(query, docs_array [, { field:, limit: }])`.
pub fn exec_rerank(args: &[Value]) -> Result<Value, String> {
    let query = match args.first() {
        Some(Value::String(s)) => s.to_string(),
        _ => {
            return Err(
                "rerank expects (query_string, docs_array[, options]), e.g. \
                 rerank(\"vector search\", results, { field: \"content\", limit: 5 })"
                    .to_string(),
            )
        }
    };
    let arr = match args.get(1) {
        Some(Value::Array(a)) => a.clone(),
        _ => return Err("rerank: second argument must be an array of records".to_string()),
    };

    let mut field: Option<String> = None;
    let mut limit: Option<usize> = None;
    if let Some(Value::Hash(h)) = args.get(2) {
        for (k, v) in h.borrow().iter() {
            let key = match k {
                HashKey::String(s) => s.to_string(),
                _ => continue,
            };
            match (key.as_str(), v) {
                ("field", Value::String(fs)) => field = Some(fs.to_string()),
                ("limit", Value::Int(n)) if *n >= 0 => limit = Some(*n as usize),
                (other, _) => {
                    return Err(format!(
                        "rerank() unknown/invalid option '{}': expected field: or limit:",
                        other
                    ))
                }
            }
        }
    }

    let q_tokens: HashSet<String> = tokenize(&query).into_iter().collect();
    let items: Vec<Value> = arr.borrow().clone();
    let mut scored: Vec<(usize, usize, Value)> = items
        .into_iter()
        .enumerate()
        .map(|(i, el)| {
            let matches = if q_tokens.is_empty() {
                0
            } else {
                tokenize(&record_text(&el, field.as_deref()))
                    .into_iter()
                    .filter(|t| q_tokens.contains(t))
                    .count()
            };
            (matches, i, el)
        })
        .collect();
    // Most matches first; original order breaks ties (stable).
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));

    let mut out: Vec<Value> = scored.into_iter().map(|(_, _, el)| el).collect();
    if let Some(l) = limit {
        out.truncate(l);
    }
    Ok(Value::Array(Rc::new(RefCell::new(out))))
}
