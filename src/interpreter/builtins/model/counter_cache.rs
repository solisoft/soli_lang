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
//!
//! Polymorphic belongs_to relations are supported: the parent *collection*
//! is resolved at bump time from the record's `{name}_type` field, and a
//! reassignment treats the (type, id) pair as the parent identity.

use super::relations::{RelationDef, RelationType};
use crate::interpreter::value::{Instance, Value};

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

fn type_string(value: &Value) -> Option<String> {
    match value {
        Value::String(s) if !s.is_empty() => Some(s.to_string()),
        _ => None,
    }
}

fn type_string_json(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

/// The parent's collection for one bump: fixed for a plain belongs_to,
/// resolved from the record's type string for a polymorphic one.
fn parent_collection(rel: &RelationDef, type_value: Option<&str>) -> Option<String> {
    if rel.relation_type == RelationType::Polymorphic {
        type_value.map(super::core::class_name_to_collection)
    } else {
        Some(rel.collection.clone())
    }
}

/// Bump one parent's counter column. Best-effort by contract.
fn bump_parent(rel: &RelationDef, collection: &str, parent_key: &str, delta: i64) {
    let column = match &rel.counter_cache {
        Some(column) => column,
        None => return,
    };
    // Swallow failures: the primary write is already committed, and
    // reset_counters is the documented repair tool.
    let _ = super::crud::cas_field_delta(collection, parent_key, column, delta);
}

/// The (collection, key) parent identity a relation points at on an instance.
fn parent_target_from_instance(rel: &RelationDef, inst: &Instance) -> Option<(String, String)> {
    let key = inst.fields.get(&rel.foreign_key).and_then(fk_key)?;
    let type_value = rel
        .polymorphic_type_field
        .as_ref()
        .and_then(|field| inst.fields.get(field))
        .and_then(type_string);
    let collection = parent_collection(rel, type_value.as_deref())?;
    Some((collection, key))
}

/// Same as [`parent_target_from_instance`] for a raw JSON document.
fn parent_target_from_json(rel: &RelationDef, doc: &serde_json::Value) -> Option<(String, String)> {
    let key = doc.get(&rel.foreign_key).and_then(fk_key_json)?;
    let type_value = rel
        .polymorphic_type_field
        .as_ref()
        .and_then(|field| doc.get(field))
        .and_then(type_string_json);
    let collection = parent_collection(rel, type_value.as_deref())?;
    Some((collection, key))
}

/// Bump every counter-cached parent referenced by the instance's FK fields
/// (`+1` after insert/restore, `-1` after delete/soft-delete).
pub fn bump_for_instance(inst: &Instance, delta: i64) {
    let class_name = inst.class.name.clone();
    for rel in counter_cached_relations(&class_name) {
        if let Some((collection, key)) = parent_target_from_instance(&rel, inst) {
            bump_parent(&rel, &collection, &key, delta);
        }
    }
}

/// Same as [`bump_for_instance`] for a raw JSON document (class-form paths
/// that never hydrate an instance).
pub fn bump_for_json(class_name: &str, doc: &serde_json::Value, delta: i64) {
    for rel in counter_cached_relations(class_name) {
        if let Some((collection, key)) = parent_target_from_json(&rel, doc) {
            bump_parent(&rel, &collection, &key, delta);
        }
    }
}

/// Handle parent reassignment: consume the `(name, old, new)` changes a
/// successful update persisted (dirty-tracking's `finalize_persist` return)
/// and move the count from the old parent to the new one. The parent
/// identity is the (collection, key) pair — for polymorphic relations a
/// type-only change moves the count too. `inst` supplies the unchanged
/// component of the pair (its fields hold the post-update values).
pub fn bump_for_changes(inst: &Instance, changes: &[(String, Value, Value)]) {
    if changes.is_empty() {
        return;
    }
    let class_name = inst.class.name.clone();
    for rel in counter_cached_relations(&class_name) {
        let changed = |field: &str| changes.iter().find(|(name, _, _)| name == field);

        let fk_change = changed(&rel.foreign_key);
        let type_change = rel.polymorphic_type_field.as_deref().and_then(changed);
        if fk_change.is_none() && type_change.is_none() {
            continue;
        }

        let new_key = inst.fields.get(&rel.foreign_key).and_then(fk_key);
        let old_key = match fk_change {
            Some((_, old, _)) => fk_key(old),
            None => new_key.clone(),
        };
        let new_type = rel
            .polymorphic_type_field
            .as_ref()
            .and_then(|field| inst.fields.get(field))
            .and_then(type_string);
        let old_type = match type_change {
            Some((_, old, _)) => type_string(old),
            None => new_type.clone(),
        };

        let old_target = parent_collection(&rel, old_type.as_deref()).zip(old_key);
        let new_target = parent_collection(&rel, new_type.as_deref()).zip(new_key);
        if old_target == new_target {
            continue;
        }
        if let Some((collection, key)) = old_target {
            bump_parent(&rel, &collection, &key, -1);
        }
        if let Some((collection, key)) = new_target {
            bump_parent(&rel, &collection, &key, 1);
        }
    }
}

/// JSON-diff variant for the class form `Model.update(id, data)`: compare
/// the pre-update document with the patch and move counts when the parent
/// (collection, key) identity changed.
pub fn bump_for_json_change(
    class_name: &str,
    old_doc: &serde_json::Value,
    patch: &serde_json::Value,
) {
    for rel in counter_cached_relations(class_name) {
        let touches_fk = patch.get(&rel.foreign_key).is_some();
        let touches_type = rel
            .polymorphic_type_field
            .as_deref()
            .is_some_and(|field| patch.get(field).is_some());
        if !touches_fk && !touches_type {
            continue;
        }

        let field_value = |doc: &serde_json::Value, field: &str| {
            patch
                .get(field)
                .or_else(|| doc.get(field))
                .cloned()
                .unwrap_or(serde_json::Value::Null)
        };

        let old_key = old_doc.get(&rel.foreign_key).and_then(fk_key_json);
        let new_key = fk_key_json(&field_value(old_doc, &rel.foreign_key));
        let (old_type, new_type) = match rel.polymorphic_type_field.as_deref() {
            Some(field) => (
                old_doc.get(field).and_then(type_string_json),
                type_string_json(&field_value(old_doc, field)),
            ),
            None => (None, None),
        };

        let old_target = parent_collection(&rel, old_type.as_deref()).zip(old_key);
        let new_target = parent_collection(&rel, new_type.as_deref()).zip(new_key);
        if old_target == new_target {
            continue;
        }
        if let Some((collection, key)) = old_target {
            bump_parent(&rel, &collection, &key, -1);
        }
        if let Some((collection, key)) = new_target {
            bump_parent(&rel, &collection, &key, 1);
        }
    }
}

/// Recount a parent's children and write the counter column. Returns the
/// fresh count. `relation_name` is a has_many on the parent class; an `as:`
/// relation counts with its polymorphic type guard.
pub fn reset_counters(
    parent_class: &str,
    parent_collection_name: &str,
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
                        RelationType::HasMany | RelationType::HasOne
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
    // pointing back at this parent. For an `as:` relation the child side is
    // Polymorphic — match on the shared type field; otherwise match the
    // child relation whose target collection is this parent's collection.
    let column = counter_cached_relations(&relation.class_name)
        .into_iter()
        .find(|rel| match &relation.polymorphic_type_field {
            Some(type_field) => {
                rel.relation_type == RelationType::Polymorphic
                    && rel.polymorphic_type_field.as_ref() == Some(type_field)
            }
            None => rel.collection == parent_collection_name,
        })
        .and_then(|rel| rel.counter_cache)
        .unwrap_or_else(|| format!("{}_count", relation.collection));

    let type_guard = match (
        &relation.polymorphic_type_field,
        &relation.polymorphic_type_value,
    ) {
        (Some(field), Some(value)) => format!(" AND d.{} == \"{}\"", field, value),
        _ => String::new(),
    };
    let soft_guard = if super::registry::is_soft_delete(&relation.class_name) {
        " AND d.deleted_at == null"
    } else {
        ""
    };
    let query = format!(
        "RETURN LENGTH(FOR d IN {} FILTER d.{} == @k{}{} RETURN 1)",
        relation.collection, relation.foreign_key, type_guard, soft_guard
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
        parent_collection_name,
        parent_key,
        serde_json::Value::Object(patch),
        true,
    )?;

    Ok(count)
}
