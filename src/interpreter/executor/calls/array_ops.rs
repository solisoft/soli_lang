//! Pure, engine-agnostic array transforms shared by the tree-walking
//! interpreter and the bytecode VM.
//!
//! The VM used to reimplement these inline, and the copies had drifted:
//! `flatten` in the VM flattened only a single level (and rejected a depth
//! argument) while the interpreter — the reference engine — flattens
//! recursively with an optional depth. Sharing one implementation keeps the
//! engines in lockstep so a fix lands in exactly one place.

use crate::interpreter::value::Value;

/// Flatten `items` up to `max_depth` levels deep (`None` = fully recursive).
pub(crate) fn flatten_values(items: &[Value], max_depth: Option<usize>) -> Vec<Value> {
    fn recur(arr: &[Value], depth: usize, max: Option<usize>) -> Vec<Value> {
        if let Some(max) = max {
            if depth >= max {
                return arr.to_vec();
            }
        }
        let mut result = Vec::new();
        for item in arr {
            if let Value::Array(inner) = item {
                result.extend(recur(&inner.borrow(), depth + 1, max));
            } else {
                result.push(item.clone());
            }
        }
        result
    }
    recur(items, 0, max_depth)
}

/// Deduplicate `items`, preserving first-occurrence order (value equality).
pub(crate) fn uniq_values(items: &[Value]) -> Vec<Value> {
    let mut result: Vec<Value> = Vec::new();
    for item in items {
        if !result.contains(item) {
            result.push(item.clone());
        }
    }
    result
}

/// Drop `null` elements from `items`.
pub(crate) fn compact_values(items: &[Value]) -> Vec<Value> {
    items
        .iter()
        .filter(|v| !matches!(v, Value::Null))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn arr(v: Vec<Value>) -> Value {
        Value::Array(Rc::new(RefCell::new(v)))
    }

    #[test]
    fn flatten_is_recursive_by_default() {
        // [[1, [2]], 3] -> [1, 2, 3] (this is where the VM previously diverged,
        // producing the shallow [1, [2], 3]).
        let input = vec![
            arr(vec![Value::Int(1), arr(vec![Value::Int(2)])]),
            Value::Int(3),
        ];
        assert_eq!(
            flatten_values(&input, None),
            vec![Value::Int(1), Value::Int(2), Value::Int(3)]
        );
    }

    #[test]
    fn flatten_respects_max_depth() {
        // depth 1: [[1, [2]]] -> [1, [2]]
        let input = vec![arr(vec![Value::Int(1), arr(vec![Value::Int(2)])])];
        assert_eq!(
            flatten_values(&input, Some(1)),
            vec![Value::Int(1), arr(vec![Value::Int(2)])]
        );
    }

    #[test]
    fn uniq_preserves_first_occurrence() {
        let input = vec![
            Value::Int(1),
            Value::Int(2),
            Value::Int(1),
            Value::Int(3),
            Value::Int(2),
        ];
        assert_eq!(
            uniq_values(&input),
            vec![Value::Int(1), Value::Int(2), Value::Int(3)]
        );
    }

    #[test]
    fn compact_drops_nulls() {
        let input = vec![Value::Int(1), Value::Null, Value::Int(2), Value::Null];
        assert_eq!(compact_values(&input), vec![Value::Int(1), Value::Int(2)]);
    }
}
