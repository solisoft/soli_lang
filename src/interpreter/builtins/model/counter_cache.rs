//! Counter caches for `belongs_to ..., counter_cache:`.
//!
//! The child keeps a `<children>_count` column on its parent up to date via
//! the same If-Match CAS loop `increment`/`decrement` use
//! (`crud::cas_field_delta`; a missing column reads as 0, so parents need no
//! schema preparation). Bumps are **best-effort**: the primary write has
//! already committed, so a failing bump (contention, vanished parent) never
//! fails the operation — counters are eventually consistent under failures
//! and `Model.reset_counters(id, relation)` repairs drift. Bulk writes
//! (`delete_all`, `update_all`, `upsert`, `import`, `prune`) skip bumps by
//! design.

use super::relations::RelationDef;
use crate::interpreter::value::Value;

/// The owner class's belongs_to relations that maintain a parent counter.
pub fn counter_cached_relations(class_name: &str) -> Vec<RelationDef> {
    super::relations::get_relations(class_name)
        .into_iter()
        .filter(|rel| rel.counter_cache.is_some())
        .collect()
}

/// Does this class declare any counter-cached belongs_to?
pub fn class_has_counter_caches(class_name: &str) -> bool {
    !counter_cached_relations(class_name).is_empty()
}

fn fk_key(value: &Value) -> Option<String> {
    match value {
        Value::String(s) if !s.is_empty() => Some(s.to_string()),
        Value::Int(n) => Some(n.to_string()),
        _ => None,
    }
}

fn fk_key_json(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) if !s.is_empty() => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Bump one parent's counter column. Best-effort by contract.
fn bump_parent(rel: &RelationDef, parent_key: &str, delta: i64) {
    let column = match &rel.counter_cache {
        Some(column) => column,
        None => return,
    };
    // Swallow failures: the primary write is already committed, and
    // reset_counters is the documented repair tool.
    let _ = super::crud::cas_field_delta(&rel.collection, parent_key, column, delta);
}

/// Bump every counter-cached parent referenced by the instance's FK fields
/// (`+1` after insert/restore, `-1` after delete/soft-delete).
pub fn bump_for_instance(inst: &crate::interpreter::value::Instance, delta: i64) {
    let class_name = inst.class.name.clone();
    for rel in counter_cached_relations(&class_name) {
        if let Some(parent_key) = inst.fields.get(&rel.foreign_key).and_then(fk_key) {
            bump_parent(&rel, &parent_key, delta);
        }
    }
}

/// Same as [`bump_for_instance`] for a raw JSON document (class-form paths
/// that never hydrate an instance).
pub fn bump_for_json(class_name: &str, doc: &serde_json::Value, delta: i64) {
    for rel in counter_cached_relations(class_name) {
        if let Some(parent_key) = doc.get(&rel.foreign_key).and_then(fk_key_json) {
            bump_parent(&rel, &parent_key, delta);
        }
    }
}

/// Handle FK reassignment: consume the `(name, old, new)` changes a
/// successful update persisted (dirty-tracking's `finalize_persist` return)
/// and move the count from the old parent to the new one.
pub fn bump_for_changes(class_name: &str, changes: &[(String, Value, Value)]) {
    if changes.is_empty() {
        return;
    }
    for rel in counter_cached_relations(class_name) {
        if let Some((_, old, new)) = changes.iter().find(|(name, _, _)| *name == rel.foreign_key) {
            let old_key = fk_key(old);
            let new_key = fk_key(new);
            if old_key == new_key {
                continue;
            }
            if let Some(key) = old_key {
                bump_parent(&rel, &key, -1);
            }
            if let Some(key) = new_key {
                bump_parent(&rel, &key, 1);
            }
        }
    }
}

/// JSON-diff variant for the class form `Model.update(id, data)`: compare
/// the pre-update document with the patch and move counts on FK change.
pub fn bump_for_json_change(
    class_name: &str,
    old_doc: &serde_json::Value,
    patch: &serde_json::Value,
) {
    for rel in counter_cached_relations(class_name) {
        let Some(new_value) = patch.get(&rel.foreign_key) else {
            continue; // patch doesn't touch this FK
        };
        let old_key = old_doc.get(&rel.foreign_key).and_then(fk_key_json);
        let new_key = fk_key_json(new_value);
        if old_key == new_key {
            continue;
        }
        if let Some(key) = old_key {
            bump_parent(&rel, &key, -1);
        }
        if let Some(key) = new_key {
            bump_parent(&rel, &key, 1);
        }
    }
}

/// Recount a parent's children and write the counter column. Returns the
/// fresh count. `relation_name` is a has_many on the parent class.
pub fn reset_counters(
    parent_class: &str,
    parent_collection: &str,
    parent_key: &str,
    relation_name: &str,
) -> Result<i64, String> {
    let relation =
        super::relations::get_relation(parent_class, relation_name).ok_or_else(|| {
            let available: Vec<String> = super::relations::get_relations(parent_class)
                .into_iter()
                .filter(|r| {
                    matches!(
                        r.relation_type,
                        super::relations::RelationType::HasMany
                            | super::relations::RelationType::HasOne
                    )
                })
                .map(|r| r.name)
                .collect();
            format!(
                "reset_counters: {} has no relation \"{}\" (available: {})",
                parent_class,
                relation_name,
                available.join(", ")
            )
        })?;

    // The counter column: the child class's counter-cached belongs_to
    // pointing back at this collection when declared, else the default name.
    let column = counter_cached_relations(&relation.class_name)
        .into_iter()
        .find(|rel| rel.collection == parent_collection)
        .and_then(|rel| rel.counter_cache)
        .unwrap_or_else(|| format!("{}_count", relation.collection));

    let soft_guard = if super::registry::is_soft_delete(&relation.class_name) {
        " AND d.deleted_at == null"
    } else {
        ""
    };
    let query = format!(
        "RETURN LENGTH(FOR d IN {} FILTER d.{} == @k{} RETURN 1)",
        relation.collection, relation.foreign_key, soft_guard
    );
    let mut binds = std::collections::HashMap::new();
    binds.insert(
        "k".to_string(),
        serde_json::Value::String(parent_key.to_string()),
    );

    let rows = super::crud::exec_with_auto_collection(query, Some(binds), &relation.collection)?;
    let count = rows.first().and_then(|v| v.as_i64()).unwrap_or(0);

    let mut patch = serde_json::Map::new();
    patch.insert(
        column,
        serde_json::Value::Number(serde_json::Number::from(count)),
    );
    super::crud::exec_update(
        parent_collection,
        parent_key,
        serde_json::Value::Object(patch),
        true,
    )?;

    Ok(count)
}
