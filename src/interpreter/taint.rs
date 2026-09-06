//! Which runtime values came in with the request.
//!
//! Soli's hash `.where` accepts two shapes for a value: a scalar means
//! equality, and a nested hash means an operator map (`{ "gt": 10 }`). That is
//! a deliberate, well-documented convenience — and it is also the classic
//! NoSQL-injection footgun, because a JSON body can produce exactly the same
//! shape as a developer-written literal:
//!
//! ```soli
//! # the developer meant an equality check on a secret
//! User.where({ "email": params["email"], "api_token": params["token"] }).first
//! ```
//!
//! With `{"email": "admin@x.com", "token": {"ne": null}}` the token predicate
//! becomes `api_token != null` and the check is gone. There is no structural
//! difference between the two hashes to key off — MongoDB drivers have the same
//! problem with `$`-prefixed keys — so the only precise answer is to remember
//! *where a value came from*.
//!
//! Request parsing hands the interpreter a tree of hashes and arrays. Those are
//! `Rc`-shared, so `params["token"]` yields the very same allocation that was
//! registered here, and cloning a `Value` clones the `Rc` rather than the
//! contents. Recording the pointers once per request therefore gives an exact
//! answer with no change to `Value` itself and no cost on any other path.
//!
//! # Why the marks hold a strong reference
//!
//! An address is only an identity for as long as the allocation lives. Keying
//! marks on `Rc::as_ptr` alone was unsound: a container marked at the start of
//! a request can be dropped *during* that request, and the allocator is then
//! free to hand the very same block to something else — including a literal
//! the developer wrote. That literal inherits a mark it never earned.
//!
//! It was not theoretical. A request with no params gets a freshly allocated
//! empty hash, which is marked and installed as the `params` global; dispatch
//! then replaces that global, the empty hash is freed, and its address stays
//! in the table. `ApiRequest.where({"status": {"gte": 400}})` later allocates
//! a one-entry hash, lands on the recycled block, and the filter is refused as
//! client-supplied — on roughly one request in twenty, with an injection
//! diagnostic pointing at a `GET /` whose params are empty.
//!
//! So the table keeps the `Value` next to the address. One `Rc` clone per
//! marked container is enough to make the address mean one thing for as long
//! as the mark is consulted, and `clear_request_values` releases them at the
//! start of the next request on this thread.
//!
//! The cost of that is one request's worth of parsed body held per worker
//! thread between two requests, where it used to be freed as soon as the
//! handler let go. It is bounded by the body-size limit and by the thread
//! count, and it buys an answer that cannot be wrong — a trade this table has
//! to make, because a mark that is sometimes false is worse than useless: it
//! rejects the developer's own literals.
//!
//! This is a *taint* marker, not a capability: it says "a client chose this",
//! which is precisely the question `.where` needs answered before it lets a
//! value pick an operator.

use std::cell::RefCell;
use std::collections::hash_map::Entry;
use std::collections::HashMap;

use crate::interpreter::value::Value;

/// How deep request marking descends. Inbound bodies are already depth-capped
/// by the JSON parser and the Rack-style param nester (32), so this only has to
/// be a backstop against a pathological shape.
const MAX_MARK_DEPTH: usize = 64;

thread_local! {
    /// Container allocations that arrived with the current request, keyed by
    /// pointer and cleared between requests.
    ///
    /// The value is not decoration: holding the `Value` keeps the allocation
    /// alive, so its address cannot be recycled while the mark is still
    /// consulted. Keying on a pointer whose allocation may already be gone is
    /// what made this table report literals as client-supplied.
    static REQUEST_CONTAINERS: RefCell<HashMap<usize, Value>> =
        RefCell::new(HashMap::new());
}

/// Pointer identity of a container value, or `None` for scalars.
fn container_addr(value: &Value) -> Option<usize> {
    match value {
        Value::Hash(pairs) => Some(std::rc::Rc::as_ptr(pairs) as *const u8 as usize),
        Value::Array(items) => Some(std::rc::Rc::as_ptr(items) as *const u8 as usize),
        _ => None,
    }
}

/// Record `value` and everything nested inside it as request-supplied.
///
/// Called by the server for `params`, `req` and `cookies` before the handler
/// runs. Scalars need no marking: a string or a number can only ever mean
/// equality in a filter, which is what the developer asked for.
pub fn mark_request_value(value: &Value) {
    REQUEST_CONTAINERS.with(|marks| {
        let mut marks = marks.borrow_mut();
        mark_into(value, &mut marks, 0);
    });
}

/// Record this container and recurse. Returns early on an allocation already
/// seen, which is what terminates a cyclic or diamond-shaped request tree.
fn mark_into(value: &Value, marks: &mut HashMap<usize, Value>, depth: usize) {
    if depth >= MAX_MARK_DEPTH {
        return;
    }
    let Some(addr) = container_addr(value) else {
        return;
    };
    match marks.entry(addr) {
        Entry::Occupied(_) => return,
        // The clone is an `Rc` bump, and it is the whole point: it pins the
        // allocation so no later value can be handed this address.
        Entry::Vacant(slot) => {
            slot.insert(value.clone());
        }
    }
    match value {
        Value::Hash(pairs) => {
            if let Ok(borrowed) = pairs.try_borrow() {
                for (_, val) in borrowed.iter() {
                    mark_into(val, marks, depth + 1);
                }
            }
        }
        Value::Array(items) => {
            if let Ok(borrowed) = items.try_borrow() {
                for item in borrowed.iter() {
                    mark_into(item, marks, depth + 1);
                }
            }
        }
        _ => {}
    }
}

/// Did this container arrive with the current request?
pub fn is_request_supplied(value: &Value) -> bool {
    let Some(addr) = container_addr(value) else {
        return false;
    };
    REQUEST_CONTAINERS.with(|marks| marks.borrow().contains_key(&addr))
}

/// Forget the current request's containers. Called between requests so one
/// visitor's marks can never be consulted while serving the next.
pub fn clear_request_values() {
    REQUEST_CONTAINERS.with(|marks| {
        let mut marks = marks.borrow_mut();
        // `clear` keeps the table's own allocation, which is what we want on a
        // hot worker, and drops the strong references the marks were holding.
        marks.clear();
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::value::{HashKey, HashPairs};
    use std::rc::Rc;

    fn hash(pairs: Vec<(&str, Value)>) -> Value {
        let mut out = HashPairs::default();
        for (k, v) in pairs {
            out.insert(HashKey::String((*k).into()), v);
        }
        Value::Hash(Rc::new(RefCell::new(out)))
    }

    #[test]
    fn nested_request_containers_are_marked_and_scalars_are_not() {
        clear_request_values();
        let operator_hash = hash(vec![("ne", Value::Null)]);
        let params = hash(vec![
            ("email", Value::String("admin@x.com".into())),
            ("token", operator_hash.clone()),
        ]);
        mark_request_value(&params);

        assert!(is_request_supplied(&params));
        // The nested hash the client sent is what `.where` must refuse.
        assert!(is_request_supplied(&operator_hash));
        // A scalar is never tainted: equality is the only thing it can mean.
        assert!(!is_request_supplied(&Value::String("admin@x.com".into())));
        // A hash the developer wrote in source is untouched.
        assert!(!is_request_supplied(&hash(vec![("gt", Value::Int(10))])));
    }

    #[test]
    fn marks_survive_being_read_back_out_of_the_params_hash() {
        clear_request_values();
        let params = hash(vec![("filter", hash(vec![("ne", Value::Null)]))]);
        mark_request_value(&params);

        // `params["filter"]` clones the Value, which clones the Rc — the same
        // allocation, so the mark has to follow it.
        let Value::Hash(pairs) = &params else {
            unreachable!()
        };
        let read_back = pairs
            .borrow()
            .get(&HashKey::String("filter".into()))
            .cloned()
            .unwrap();
        assert!(is_request_supplied(&read_back));
    }

    #[test]
    fn clearing_forgets_the_previous_request() {
        clear_request_values();
        let params = hash(vec![("a", hash(vec![("ne", Value::Null)]))]);
        mark_request_value(&params);
        assert!(is_request_supplied(&params));

        clear_request_values();
        assert!(!is_request_supplied(&params));
    }

    /// The regression this table exists to prevent, stated as an invariant a
    /// test can check without racing the allocator: while a mark is live, the
    /// allocation it names must still be alive. If it can be freed, its address
    /// can be handed to a literal built later in the same request, and that
    /// literal is then read as client-supplied.
    #[test]
    fn a_marked_container_outlives_every_other_reference() {
        clear_request_values();
        let weak = {
            let params = hash(vec![("a", Value::Int(1))]);
            mark_request_value(&params);
            let Value::Hash(pairs) = &params else {
                unreachable!()
            };
            Rc::downgrade(pairs)
            // `params` is dropped here: the table now holds the only strong
            // reference.
        };
        assert!(
            weak.upgrade().is_some(),
            "a marked allocation was freed while its mark was still live"
        );

        clear_request_values();
        assert!(
            weak.upgrade().is_none(),
            "clearing must release the marks, not leak them for the whole worker"
        );
    }

    /// The shape that actually happened in production: a nested container is
    /// dropped from its parent mid-request. The parent staying alive is not
    /// enough — each marked allocation has to be pinned on its own.
    #[test]
    fn a_nested_container_removed_from_its_parent_stays_pinned() {
        clear_request_values();
        let params = hash(vec![("filter", hash(vec![("ne", Value::Null)]))]);
        mark_request_value(&params);

        let Value::Hash(pairs) = &params else {
            unreachable!()
        };
        let nested = pairs
            .borrow()
            .get(&HashKey::String("filter".into()))
            .cloned()
            .unwrap();
        let Value::Hash(nested_pairs) = &nested else {
            unreachable!()
        };
        let weak = Rc::downgrade(nested_pairs);

        // The request handler replaces the sub-tree, then every local handle
        // goes away.
        pairs
            .borrow_mut()
            .insert(HashKey::String("filter".into()), Value::Int(1));
        drop(nested);

        assert!(
            weak.upgrade().is_some(),
            "a nested marked allocation was freed while its mark was still live"
        );
        clear_request_values();
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn a_cyclic_request_value_terminates() {
        clear_request_values();
        let node = hash(vec![("name", Value::String("root".into()))]);
        if let Value::Hash(pairs) = &node {
            pairs
                .borrow_mut()
                .insert(HashKey::String("self".into()), node.clone());
        }
        mark_request_value(&node);
        assert!(is_request_supplied(&node));
    }
}
