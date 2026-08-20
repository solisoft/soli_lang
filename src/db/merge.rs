//! Canonical JSON merge-patch (RFC 7396) for the SQL adapters.
//!
//! The three adapters reached for whatever their engine offered, and the
//! results disagreed:
//!
//! | patch                | Postgres `jsonb \|\|`   | SQLite `json_patch` / MySQL `JSON_MERGE_PATCH` |
//! |----------------------|-------------------------|-----------------------------------------------|
//! | `{"b": null}`        | sets `b` to JSON null   | **deletes** `b`                               |
//! | `{"p": {"x": 9}}`    | **replaces** all of `p` | merges into `p`, keeping siblings              |
//!
//! So the same `Model.update` call kept a key on one adapter and dropped it on
//! another, and destroyed unrelated nested keys on Postgres. RFC 7396 is the
//! published behaviour two of the three already implemented, so it is the one
//! this module defines and every adapter now produces.

use serde_json::{Map, Value};

/// True when `patch` needs recursive merging — i.e. any value is an object.
///
/// Postgres has no core RFC-7396 primitive: `||` is a shallow, null-preserving
/// merge. A patch of only scalars/arrays/nulls can still be done in one
/// statement there (see `postgres::update`), so this is the test for whether the
/// slower read-merge-write path is required.
pub fn needs_recursive_merge(patch: &Value) -> bool {
    match patch {
        Value::Object(map) => map.values().any(|v| v.is_object()),
        _ => false,
    }
}

/// Apply `patch` to `target` per RFC 7396, in place.
///
/// - an object value merges recursively
/// - a `null` value removes the key
/// - anything else replaces the key
/// - a non-object patch replaces the target outright
pub fn merge_patch(target: &mut Value, patch: &Value) {
    let Value::Object(patch_map) = patch else {
        *target = patch.clone();
        return;
    };
    if !target.is_object() {
        *target = Value::Object(Map::new());
    }
    let Value::Object(target_map) = target else {
        return;
    };
    for (key, patch_value) in patch_map {
        if patch_value.is_null() {
            target_map.remove(key);
            continue;
        }
        match target_map.get_mut(key) {
            // Recurse only when both sides are objects; otherwise the patch
            // value wins whole.
            Some(existing) if existing.is_object() && patch_value.is_object() => {
                merge_patch(existing, patch_value);
            }
            _ => {
                let mut fresh = patch_value.clone();
                // Strip nulls nested inside a fresh subtree: RFC 7396 says a
                // null means "absent", so it must not be written as JSON null.
                strip_nulls(&mut fresh);
                target_map.insert(key.clone(), fresh);
            }
        }
    }
}

/// Remove null-valued keys from an object subtree, recursively.
fn strip_nulls(value: &mut Value) {
    if let Value::Object(map) = value {
        map.retain(|_, v| !v.is_null());
        for v in map.values_mut() {
            strip_nulls(v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn merged(target: Value, patch: Value) -> Value {
        let mut out = target;
        merge_patch(&mut out, &patch);
        out
    }

    /// The two cases the adapters disagreed on.
    #[test]
    fn null_deletes_and_nested_objects_merge() {
        assert_eq!(
            merged(json!({"a": 1, "b": 2}), json!({"b": null})),
            json!({"a": 1}),
            "a null value removes the key rather than storing JSON null"
        );
        assert_eq!(
            merged(json!({"p": {"x": 1, "y": 2}}), json!({"p": {"x": 9}})),
            json!({"p": {"x": 9, "y": 2}}),
            "a nested object merges instead of replacing"
        );
    }

    /// The RFC's own examples (§3), which pin the rest of the behaviour.
    #[test]
    fn rfc_7396_examples() {
        assert_eq!(
            merged(json!({"a": "b"}), json!({"a": "c"})),
            json!({"a": "c"})
        );
        assert_eq!(
            merged(json!({"a": "b"}), json!({"b": "c"})),
            json!({"a": "b", "b": "c"})
        );
        assert_eq!(merged(json!({"a": "b"}), json!({"a": null})), json!({}));
        assert_eq!(
            merged(json!({"a": "b", "b": "c"}), json!({"a": null})),
            json!({"b": "c"})
        );
        assert_eq!(
            merged(json!({"a": ["b"]}), json!({"a": "c"})),
            json!({"a": "c"})
        );
        assert_eq!(
            merged(json!({"a": "c"}), json!({"a": ["b"]})),
            json!({"a": ["b"]})
        );
        assert_eq!(
            merged(
                json!({"a": {"b": "c", "d": "e"}}),
                json!({"a": {"b": "d", "c": null}})
            ),
            json!({"a": {"b": "d", "d": "e"}})
        );
        assert_eq!(
            merged(json!({"a": [{"b": "c"}]}), json!({"a": [1]})),
            json!({"a": [1]}),
            "arrays replace wholesale — never merged element-wise"
        );
        // A null inside a brand-new subtree is absent, not stored.
        assert_eq!(
            merged(json!({}), json!({"a": {"b": "c", "d": null}})),
            json!({"a": {"b": "c"}})
        );
    }

    #[test]
    fn a_non_object_patch_replaces_the_target() {
        assert_eq!(merged(json!({"a": 1}), json!(5)), json!(5));
        assert_eq!(merged(json!(null), json!({"a": 1})), json!({"a": 1}));
    }

    #[test]
    fn recursive_merge_is_only_needed_for_object_values() {
        assert!(!needs_recursive_merge(
            &json!({"a": 1, "b": null, "c": [1, 2]})
        ));
        assert!(needs_recursive_merge(&json!({"a": 1, "p": {"x": 1}})));
        assert!(!needs_recursive_merge(&json!(5)));
    }
}
