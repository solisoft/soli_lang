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
//! This is a *taint* marker, not a capability: it says "a client chose this",
//! which is precisely the question `.where` needs answered before it lets a
//! value pick an operator.

use std::cell::RefCell;
use std::collections::HashSet;

use crate::interpreter::value::Value;

/// How deep request marking descends. Inbound bodies are already depth-capped
/// by the JSON parser and the Rack-style param nester (32), so this only has to
/// be a backstop against a pathological shape.
const MAX_MARK_DEPTH: usize = 64;

thread_local! {
    /// Addresses of container allocations that arrived with the current
    /// request. Keyed by pointer, cleared between requests.
    static REQUEST_CONTAINERS: RefCell<HashSet<usize>> = RefCell::new(HashSet::new());
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
    REQUEST_CONTAINERS.with(|set| {
        let mut set = set.borrow_mut();
        mark_into(value, &mut set, 0);
    });
}

fn mark_into(value: &Value, set: &mut HashSet<usize>, depth: usize) {
    if depth >= MAX_MARK_DEPTH {
        return;
    }
    match value {
        Value::Hash(pairs) => {
            let addr = std::rc::Rc::as_ptr(pairs) as *const u8 as usize;
            // A cyclic or shared sub-tree is visited once; `insert` returning
            // false means we have already walked this allocation.
            if !set.insert(addr) {
                return;
            }
            if let Ok(borrowed) = pairs.try_borrow() {
                for (_, val) in borrowed.iter() {
                    mark_into(val, set, depth + 1);
                }
            }
        }
        Value::Array(items) => {
            let addr = std::rc::Rc::as_ptr(items) as *const u8 as usize;
            if !set.insert(addr) {
                return;
            }
            if let Ok(borrowed) = items.try_borrow() {
                for item in borrowed.iter() {
                    mark_into(item, set, depth + 1);
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
    REQUEST_CONTAINERS.with(|set| set.borrow().contains(&addr))
}

/// Forget the current request's containers. Called between requests so one
/// visitor's marks can never be consulted while serving the next.
pub fn clear_request_values() {
    REQUEST_CONTAINERS.with(|set| {
        let mut set = set.borrow_mut();
        // `clear` keeps the allocation, which is what we want on a hot worker.
        set.clear();
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
