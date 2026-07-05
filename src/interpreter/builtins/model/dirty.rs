//! Dirty tracking for model instances.
//!
//! The baseline (`Instance::original_fields`) is a shallow clone of the
//! non-`_` fields taken when a record is loaded from or persisted to the
//! database; `None` means a new record. Diffs are computed lazily by the
//! `changed?`/`changed`/`changes` natives — the attribute-assignment path
//! stays untouched. Because the snapshot shares `Rc`s with the live values,
//! in-place mutation of a nested Hash/Array is invisible to tracking
//! (documented: reassign the attribute to record such a change).

use std::collections::HashMap;

use ahash::RandomState as AHasher;

use crate::interpreter::value::{enum_aware_equal, Instance, Value};

/// Shallow clone of the persistable (non-`_`) fields.
fn snapshot_fields(inst: &Instance) -> HashMap<String, Value, AHasher> {
    inst.fields
        .iter()
        .filter(|(name, _)| !name.starts_with('_'))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

/// Reset the dirty baseline to the instance's current fields.
pub fn seed_snapshot(inst: &mut Instance) {
    inst.original_fields = Some(Box::new(snapshot_fields(inst)));
}

/// Compute `(name, old, new)` for every attribute whose current value
/// differs from the baseline, sorted by name. A `None` baseline (new
/// record) reports every non-`_` field as `[null, value]`.
pub fn compute_changes(inst: &Instance) -> Vec<(String, Value, Value)> {
    let empty = HashMap::default();
    let original: &HashMap<String, Value, AHasher> =
        inst.original_fields.as_deref().unwrap_or(&empty);

    let mut names: Vec<&String> = inst
        .fields
        .keys()
        .filter(|name| !name.starts_with('_'))
        .chain(original.keys())
        .collect();
    names.sort();
    names.dedup();

    let mut changes = Vec::new();
    for name in names {
        let old = original.get(name).cloned().unwrap_or(Value::Null);
        let new = inst.fields.get(name).cloned().unwrap_or(Value::Null);
        if !enum_aware_equal(&old, &new) {
            changes.push((name.clone(), old, new));
        }
    }
    changes
}

/// After a successful create/save/update: record what the persist changed
/// into `previous_changes`, reseed the baseline, and return the changes
/// (counter caches consume the return value for FK reassignment).
pub fn finalize_persist(inst: &mut Instance) -> Vec<(String, Value, Value)> {
    let changes = compute_changes(inst);
    inst.previous_changes = Some(Box::new(changes.clone()));
    seed_snapshot(inst);
    changes
}

/// Sync a single field into the baseline after a write that persisted just
/// that field (soft-delete/restore `deleted_at`, `increment`/`decrement`).
pub fn sync_snapshot_field(inst: &mut Instance, name: &str) {
    if name.starts_with('_') {
        return;
    }
    let live = inst.fields.get(name).cloned();
    if let Some(original) = inst.original_fields.as_deref_mut() {
        match live {
            Some(value) => {
                original.insert(name.to_string(), value);
            }
            None => {
                original.remove(name);
            }
        }
    }
}

/// Build the `{ name: [old, new] }` hash the `changes`/`previous_changes`
/// natives return.
pub fn changes_to_hash(changes: &[(String, Value, Value)]) -> Value {
    use crate::interpreter::value::{HashKey, HashPairs};
    use std::cell::RefCell;
    use std::rc::Rc;

    let mut map = HashPairs::default();
    for (name, old, new) in changes {
        let pair = Value::Array(Rc::new(RefCell::new(vec![old.clone(), new.clone()])));
        map.insert(HashKey::String(name.as_str().into()), pair);
    }
    Value::Hash(Rc::new(RefCell::new(map)))
}
