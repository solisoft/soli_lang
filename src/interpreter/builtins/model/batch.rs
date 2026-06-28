//! Request-coalescing batch context for `grouped(fn() { ... })`.
//!
//! While a batch is active, terminal model reads (`.all`, `.count`, `.first`,
//! `find`, …) register their AQL plus a row→`Value` transform instead of
//! issuing an HTTP request, and return a [`Value::Deferred`] placeholder. When
//! the batch flushes, every pending query is combined into a single
//!
//! ```text
//! LET _b0 = (…) LET _b1 = (…) … RETURN [_b0, _b1, …]
//! ```
//!
//! statement and executed in one round-trip; each subquery result then feeds
//! its transform and fills the corresponding deferred cell.
//!
//! Each registered query is a complete standalone statement (a `FOR … RETURN`
//! block, or a scalar `RETURN <expr>` for `count`). When embedded as a
//! parenthesised subquery, a leading top-level `RETURN ` is stripped so the
//! body is a valid subquery expression — see [`build_combined_query`].
//!
//! A flush happens at block end (see the `grouped` interceptor in
//! `executor/calls/function.rs`) or the first time a deferred result is read
//! inside the block — the "auto-flush" behaviour. Auto-flush is driven by the
//! read sites calling [`force`] / [`flush_current`].

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::value::{DeferredCell, Value};

/// Transform a subquery's raw JSON rows into the `Value` the terminal method
/// would normally return. May fail (e.g. `find` raising `RecordNotFound` on an
/// empty result); the error surfaces when the deferred is forced.
type Transform = Box<dyn Fn(Vec<serde_json::Value>) -> Result<Value, String>>;

struct PendingQuery {
    aql: String,
    binds: HashMap<String, serde_json::Value>,
    transform: Transform,
    cell: Rc<RefCell<DeferredCell>>,
}

struct BatchState {
    pending: Vec<PendingQuery>,
}

thread_local! {
    static BATCH: RefCell<Option<BatchState>> = const { RefCell::new(None) };
}

/// Whether a `grouped {}` batch is currently collecting queries.
#[inline]
pub fn is_active() -> bool {
    BATCH.with(|b| b.borrow().is_some())
}

/// Begin a batch. Returns `true` if this call started a *new* batch (the caller
/// owns it and must call [`end`]); returns `false` if a batch was already
/// active (a nested `grouped` joins the outer one and must NOT end it).
pub fn begin() -> bool {
    BATCH.with(|b| {
        let mut slot = b.borrow_mut();
        if slot.is_some() {
            false
        } else {
            *slot = Some(BatchState {
                pending: Vec::new(),
            });
            true
        }
    })
}

/// End the batch owned by this caller: flush any still-pending queries, then
/// clear the context. Returns `Err` if the final flush failed.
pub fn end() -> Result<(), String> {
    let res = flush_inner();
    BATCH.with(|b| *b.borrow_mut() = None);
    res
}

/// Register a query for coalescing and return its `Value::Deferred` placeholder.
/// Must only be called while [`is_active`]; callers gate on it.
pub fn register(
    aql: String,
    binds: HashMap<String, serde_json::Value>,
    transform: Transform,
) -> Value {
    let cell = Rc::new(RefCell::new(DeferredCell::default()));
    BATCH.with(|b| {
        if let Some(state) = b.borrow_mut().as_mut() {
            state.pending.push(PendingQuery {
                aql,
                binds,
                transform,
                cell: cell.clone(),
            });
        }
    });
    Value::Deferred(cell)
}

/// Resolve a deferred cell, flushing the pending batch first if needed.
/// Propagates a flush/transform error (e.g. `find`'s `RecordNotFound`).
pub fn force(cell: &Rc<RefCell<DeferredCell>>) -> Result<Value, String> {
    if cell.borrow().resolved.is_none() {
        flush_inner()?;
    }
    Ok(cell.borrow().resolved.clone().unwrap_or(Value::Null))
}

/// Best-effort flush used by the `Value` formatting/comparison helpers, which
/// have no error channel. Errors are swallowed here — the real read sites use
/// [`force`], which surfaces them.
pub fn flush_current() {
    let _ = flush_inner();
}

/// Combine all currently-pending queries into one statement, execute it in a
/// single round-trip, run each transform, and fill the deferred cells. No-op
/// when nothing is pending. Leaves the batch active (with an empty queue) so
/// queries registered after an auto-flush coalesce into a fresh batch.
fn flush_inner() -> Result<(), String> {
    let pending: Vec<PendingQuery> = BATCH.with(|b| match b.borrow_mut().as_mut() {
        Some(state) => std::mem::take(&mut state.pending),
        None => Vec::new(),
    });
    if pending.is_empty() {
        return Ok(());
    }

    // Single query: skip the LET/RETURN wrapper — run it directly so the dev
    // query log shows the natural statement and there's no array unwrapping.
    if pending.len() == 1 {
        let pq = pending.into_iter().next().unwrap();
        let binds = if pq.binds.is_empty() {
            None
        } else {
            Some(pq.binds)
        };
        let rows = super::crud::exec_async_query_with_binds(pq.aql, binds)?;
        let value = (pq.transform)(rows)?;
        pq.cell.borrow_mut().resolved = Some(value);
        return Ok(());
    }

    let (combined, merged_binds) = build_combined_query(&pending);
    let binds_opt = if merged_binds.is_empty() {
        None
    } else {
        Some(merged_binds)
    };
    let result = super::crud::exec_async_query_with_binds(combined, binds_opt)?;

    // The top-level `RETURN [_b0, _b1, …]` yields a single row that is the
    // array of subquery results.
    let row = result.into_iter().next().unwrap_or(serde_json::Value::Null);
    let subresults: Vec<serde_json::Value> = match row {
        serde_json::Value::Array(a) => a,
        serde_json::Value::Null => Vec::new(),
        other => vec![other],
    };

    for (i, pq) in pending.into_iter().enumerate() {
        // Each subquery is wrapped in `(…)`, so its result is an array of the
        // subquery's RETURN rows.
        let rows: Vec<serde_json::Value> = match subresults.get(i).cloned() {
            Some(serde_json::Value::Array(a)) => a,
            Some(serde_json::Value::Null) | None => Vec::new(),
            Some(other) => vec![other],
        };
        let value = (pq.transform)(rows)?;
        pq.cell.borrow_mut().resolved = Some(value);
    }
    Ok(())
}

/// Build the combined `LET … RETURN […]` statement and the merged bind map.
/// Each subquery's binds are prefixed with `b{i}__` to avoid collisions
/// between subqueries that happen to use the same bind name (`@val`, `@active`).
/// A leading top-level `RETURN ` is stripped from each query so scalar reads
/// (`count`, registered as `RETURN <expr>`) become a valid parenthesised
/// subquery `(<expr>)` rather than the invalid `(RETURN <expr>)`.
fn build_combined_query(pending: &[PendingQuery]) -> (String, HashMap<String, serde_json::Value>) {
    let mut lets = String::new();
    let mut merged: HashMap<String, serde_json::Value> = HashMap::new();
    let mut return_items = Vec::with_capacity(pending.len());
    for (i, pq) in pending.iter().enumerate() {
        let prefix = format!("b{}__", i);
        let rewritten = rewrite_binds(&pq.aql, &pq.binds, &prefix);
        for (k, v) in &pq.binds {
            merged.insert(format!("{}{}", prefix, k), v.clone());
        }
        let var = format!("_b{}", i);
        // A parenthesised subquery must be an expression or a `FOR…RETURN`
        // block, never a bare top-level `RETURN`. Scalar reads (`count`)
        // register their standalone form `RETURN <expr>` (valid when run
        // alone); strip the leading `RETURN ` so the subquery is just
        // `<expr>`, which evaluates to the scalar and lands in `_bi`. Only the
        // leading prefix is removed — an inner `RETURN` (e.g. inside a
        // `LENGTH(FOR … RETURN 1)` count) is untouched.
        let body = rewritten
            .strip_prefix("RETURN ")
            .unwrap_or(rewritten.as_str());
        lets.push_str("LET ");
        lets.push_str(&var);
        lets.push_str(" = (");
        lets.push_str(body);
        lets.push_str(")\n");
        return_items.push(var);
    }
    let combined = format!("{}RETURN [{}]", lets, return_items.join(", "));
    (combined, merged)
}

/// Rewrite every `@name` bind reference in `aql` whose `name` is a key of
/// `binds` to `@{prefix}{name}`. Identifier-boundary aware so `@val` is not
/// matched inside `@validate`. Generated model queries inline collection names
/// (no `@@coll` collection binds), so only single-`@` references occur.
fn rewrite_binds(aql: &str, binds: &HashMap<String, serde_json::Value>, prefix: &str) -> String {
    if binds.is_empty() {
        return aql.to_string();
    }
    let bytes = aql.as_bytes();
    let mut out = String::with_capacity(aql.len() + binds.len() * prefix.len());
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c == '@' {
            // Collect the identifier following '@'.
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && is_ident_char(bytes[j] as char) {
                j += 1;
            }
            let name = &aql[start..j];
            if !name.is_empty() && binds.contains_key(name) {
                out.push('@');
                out.push_str(prefix);
                out.push_str(name);
            } else {
                // Not one of our binds (or a `@@` collection bind / lone `@`):
                // copy the `@` and the identifier verbatim.
                out.push('@');
                out.push_str(name);
            }
            i = if name.is_empty() { i + 1 } else { j };
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

#[inline]
fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binds(keys: &[&str]) -> HashMap<String, serde_json::Value> {
        keys.iter()
            .map(|k| (k.to_string(), serde_json::Value::Bool(true)))
            .collect()
    }

    #[test]
    fn rewrites_only_matching_bind_names() {
        let aql = "FOR doc IN users FILTER doc.age >= @val RETURN doc";
        let out = rewrite_binds(aql, &binds(&["val"]), "b0__");
        assert_eq!(
            out,
            "FOR doc IN users FILTER doc.age >= @b0__val RETURN doc"
        );
    }

    #[test]
    fn respects_identifier_boundaries() {
        // `@validate` must NOT be rewritten when only `val` is a bind, and
        // `@val` standing alone must be.
        let aql = "FILTER doc.a == @val AND doc.b == @validate";
        let out = rewrite_binds(aql, &binds(&["val"]), "b3__");
        assert_eq!(out, "FILTER doc.a == @b3__val AND doc.b == @validate");
    }

    #[test]
    fn rewrites_multiple_distinct_binds() {
        let aql = "FILTER doc.x == @active AND doc.y IN @ids";
        let out = rewrite_binds(aql, &binds(&["active", "ids"]), "b1__");
        assert_eq!(out, "FILTER doc.x == @b1__active AND doc.y IN @b1__ids");
    }

    #[test]
    fn leaves_query_untouched_when_no_binds() {
        let aql = "FOR doc IN posts RETURN doc";
        let out = rewrite_binds(aql, &HashMap::new(), "b0__");
        assert_eq!(out, aql);
    }

    #[test]
    fn build_combined_prefixes_colliding_binds() {
        // Two subqueries both using `@val` must not collide in the merged map.
        let pending = vec![
            PendingQuery {
                aql: "FOR d IN a FILTER d.k == @val RETURN d".to_string(),
                binds: {
                    let mut m = HashMap::new();
                    m.insert("val".to_string(), serde_json::json!("x"));
                    m
                },
                transform: Box::new(|_| Ok(Value::Null)),
                cell: Rc::new(RefCell::new(DeferredCell::default())),
            },
            PendingQuery {
                aql: "FOR d IN b FILTER d.k == @val RETURN d".to_string(),
                binds: {
                    let mut m = HashMap::new();
                    m.insert("val".to_string(), serde_json::json!("y"));
                    m
                },
                transform: Box::new(|_| Ok(Value::Null)),
                cell: Rc::new(RefCell::new(DeferredCell::default())),
            },
        ];
        let (combined, merged) = build_combined_query(&pending);
        assert!(combined.contains("LET _b0 = (FOR d IN a FILTER d.k == @b0__val RETURN d)"));
        assert!(combined.contains("LET _b1 = (FOR d IN b FILTER d.k == @b1__val RETURN d)"));
        assert!(combined.trim_end().ends_with("RETURN [_b0, _b1]"));
        assert_eq!(merged.get("b0__val"), Some(&serde_json::json!("x")));
        assert_eq!(merged.get("b1__val"), Some(&serde_json::json!("y")));
    }

    fn pending(aql: &str, binds: HashMap<String, serde_json::Value>) -> PendingQuery {
        PendingQuery {
            aql: aql.to_string(),
            binds,
            transform: Box::new(|_| Ok(Value::Null)),
            cell: Rc::new(RefCell::new(DeferredCell::default())),
        }
    }

    #[test]
    fn build_combined_unwraps_leading_return_for_scalar_queries() {
        // A scalar `count` read registers `RETURN COLLECTION_COUNT(...)`. When
        // embedded as a parenthesised subquery the bare leading RETURN is
        // invalid AQL, so it must be stripped to the expression. A `FOR…RETURN`
        // read alongside it is wrapped verbatim.
        let plist = vec![
            pending("RETURN COLLECTION_COUNT(\"orders\")", HashMap::new()),
            pending("FOR doc IN orders RETURN doc", HashMap::new()),
        ];
        let (combined, _) = build_combined_query(&plist);
        assert!(combined.contains("LET _b0 = (COLLECTION_COUNT(\"orders\"))"));
        assert!(combined.contains("LET _b1 = (FOR doc IN orders RETURN doc)"));
        // No subquery may begin with a bare RETURN.
        assert!(!combined.contains("(RETURN"));
        assert!(combined.trim_end().ends_with("RETURN [_b0, _b1]"));
    }

    #[test]
    fn build_combined_strips_only_leading_return() {
        // A filtered count is `RETURN LENGTH(FOR … RETURN 1)`. Only the leading
        // RETURN is stripped; the inner `RETURN 1` of the FOR subquery survives.
        let mut binds = HashMap::new();
        binds.insert("val".to_string(), serde_json::json!(1));
        let plist = vec![pending(
            "RETURN LENGTH(FOR doc IN orders FILTER doc.x == @val RETURN 1)",
            binds,
        )];
        let (combined, _) = build_combined_query(&plist);
        assert!(combined
            .contains("LET _b0 = (LENGTH(FOR doc IN orders FILTER doc.x == @b0__val RETURN 1))"));
        assert!(!combined.contains("(RETURN"));
    }
}
