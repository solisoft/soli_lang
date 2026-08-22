//! Cascade deletes for `dependent:` relations.
//!
//! Runs from the executor (not the natives) because the `"delete"` strategy
//! deletes children *through the interpreter* — child `before_delete`/
//! `after_delete` callbacks, nested cascades, and the child's own
//! soft-delete semantics all apply. Cascades fire only on **hard** owner
//! deletes; a soft-deleting owner keeps its children (no restore-asymmetry).
//! Bulk writes (`Model.delete_all`, `QueryBuilder.delete_all`) never cascade.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::error::RuntimeError;
use crate::interpreter::builtins::model::relations::DependentStrategy;
use crate::interpreter::builtins::model::{
    get_model_class, get_relations, QueryBuilder, RelationDef,
};
use crate::interpreter::executor::{Interpreter, RuntimeResult};
use crate::interpreter::value::{Instance, Value};
use crate::span::Span;

// Documents currently being cascade-deleted on this thread, as
// "collection/key". Membership breaks `dependent:` cycles: re-entering an
// in-flight document is a no-op success instead of infinite recursion.
thread_local! {
    static CASCADE_IN_FLIGHT: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    /// Test-only: owner `_key` → child instances for `dependent: "delete"`,
    /// skipping the database load so VM unit tests can gate without SoliDB.
    ///
    /// `cfg(test)`, along with every reader: only `cfg(test)` setters can ever
    /// populate these, so in a release build the probes below were a
    /// thread-local lookup on every `dependent:` cascade and every
    /// `Model.delete(id)` that could not possibly hit.
    #[cfg(test)]
    static TEST_CASCADE_CHILDREN: RefCell<HashMap<String, Vec<Value>>> =
        RefCell::new(HashMap::new());
    /// Test-only: document id → instance for class `Model.delete(id)`.
    #[cfg(test)]
    static TEST_LOADED_INSTANCES: RefCell<HashMap<String, Value>> =
        RefCell::new(HashMap::new());
}

/// Install children that `dependent: "delete"` should cascade to for `owner_key`.
#[cfg(test)]
pub(crate) fn set_test_cascade_children(owner_key: &str, children: Vec<Value>) {
    TEST_CASCADE_CHILDREN.with(|m| {
        m.borrow_mut().insert(owner_key.to_string(), children);
    });
}

#[cfg(test)]
pub(crate) fn clear_test_cascade_children() {
    TEST_CASCADE_CHILDREN.with(|m| m.borrow_mut().clear());
}

/// Install the instance `Model.delete(id)` should load instead of querying.
#[cfg(test)]
pub(crate) fn set_test_loaded_instance(id: &str, instance: Value) {
    TEST_LOADED_INSTANCES.with(|m| {
        m.borrow_mut().insert(id.to_string(), instance);
    });
}

/// The instance a test installed for `id`, or `None`. Always `None` outside
/// tests, where the map does not exist.
pub(crate) fn take_test_loaded_instance(_id: &str) -> Option<Value> {
    #[cfg(test)]
    return TEST_LOADED_INSTANCES.with(|m| m.borrow_mut().remove(_id));
    #[cfg(not(test))]
    None
}

#[cfg(test)]
pub(crate) fn clear_test_loaded_instances() {
    TEST_LOADED_INSTANCES.with(|m| m.borrow_mut().clear());
}

const MAX_CASCADE_DEPTH: usize = 32;

/// Removes its tag from the in-flight set on drop, so an erroring cascade
/// can't leak entries.
pub(crate) struct CascadeGuard(Option<String>);

impl Drop for CascadeGuard {
    fn drop(&mut self) {
        if let Some(tag) = self.0.take() {
            CASCADE_IN_FLIGHT.with(|set| {
                set.borrow_mut().remove(&tag);
            });
        }
    }
}

/// Track `collection/key` as being deleted. Returns `None` when the document
/// is already in flight higher up the cascade (cycle — caller should treat
/// the delete as an already-handled no-op).
pub(crate) fn enter_cascade(collection: &str, key: &str) -> Option<CascadeGuard> {
    let tag = format!("{}/{}", collection, key);
    let inserted = CASCADE_IN_FLIGHT.with(|set| set.borrow_mut().insert(tag.clone()));
    if inserted {
        Some(CascadeGuard(Some(tag)))
    } else {
        None
    }
}

fn cascade_depth() -> usize {
    CASCADE_IN_FLIGHT.with(|set| set.borrow().len())
}

/// Does this model class declare any `dependent:` relation?
pub(crate) fn class_declares_dependents(class_name: &str) -> bool {
    get_relations(class_name)
        .iter()
        .any(|rel| rel.dependent.is_some())
}

/// Build a QueryBuilder over the relation's collection seeded with the
/// owner-FK filter (the same shape the `user.posts` accessor uses).
fn relation_query_builder(
    rel: &RelationDef,
    owner_key: &str,
    fallback_class: &Rc<crate::interpreter::value::Class>,
) -> QueryBuilder {
    let related_class = get_model_class(&rel.class_name).unwrap_or_else(|| fallback_class.clone());
    let mut qb = QueryBuilder::new_with_class(
        rel.class_name.clone(),
        rel.collection.clone(),
        related_class,
    );
    let mut binds = HashMap::new();
    binds.insert(
        "__rel_fk".to_string(),
        serde_json::Value::String(owner_key.to_string()),
    );
    // `as:` inverse of a polymorphic belongs_to: only rows pointing back at
    // THIS owner type belong to the relation.
    let type_guard = match (&rel.polymorphic_type_field, &rel.polymorphic_type_value) {
        (Some(field), Some(value)) => {
            binds.insert(
                "__rel_type".to_string(),
                serde_json::Value::String(value.clone()),
            );
            format!(" AND {} == @__rel_type", field)
        }
        _ => String::new(),
    };
    qb.set_filter(
        format!("{} == @__rel_fk{}", rel.foreign_key, type_guard),
        binds,
    );
    qb
}

/// Load children for `dependent: "delete"`. Test hooks skip the database.
fn load_cascade_children(
    rel: &RelationDef,
    owner_key: &str,
    span: Span,
) -> RuntimeResult<Vec<Value>> {
    use crate::interpreter::builtins::model::crud;

    #[cfg(test)]
    if let Some(children) = TEST_CASCADE_CHILDREN.with(|m| m.borrow().get(owner_key).cloned()) {
        return Ok(children);
    }

    let child_class = match get_model_class(&rel.class_name) {
        Some(class) => class,
        None => {
            return Err(RuntimeError::new(
                format!(
                    "dependent: \"delete\" on \"{}\": model class {} is not defined",
                    rel.name, rel.class_name
                ),
                span,
            ))
        }
    };

    let soft_guard = if crate::interpreter::builtins::model::is_soft_delete(&rel.class_name) {
        " AND doc.deleted_at == null"
    } else {
        ""
    };
    let limit = if rel.relation_type == crate::interpreter::builtins::model::RelationType::HasOne {
        " LIMIT 1"
    } else {
        ""
    };
    let mut binds = HashMap::new();
    binds.insert(
        "fk".to_string(),
        serde_json::Value::String(owner_key.to_string()),
    );
    let type_guard = match (&rel.polymorphic_type_field, &rel.polymorphic_type_value) {
        (Some(field), Some(value)) => {
            binds.insert(
                "rel_type".to_string(),
                serde_json::Value::String(value.clone()),
            );
            format!(" AND doc.{} == @rel_type", field)
        }
        _ => String::new(),
    };
    let query = format!(
        "FOR doc IN {} FILTER doc.{} == @fk{}{}{} RETURN doc",
        rel.collection, rel.foreign_key, type_guard, soft_guard, limit
    );

    let docs =
        crud::exec_with_auto_collection(query, Some(binds), &rel.collection).map_err(|e| {
            RuntimeError::new(
                format!("dependent: \"delete\" on \"{}\" failed: {}", rel.name, e),
                span,
            )
        })?;

    Ok(docs
        .iter()
        .map(|doc| crud::json_doc_to_instance(&child_class, doc))
        .collect())
}

/// Run every `dependent:` strategy. `delete_child` is the instance-delete
/// path (callbacks, nested cascades) used by both engines.
pub(crate) fn run_dependent_cascades_with(
    instance: &Rc<RefCell<Instance>>,
    span: Span,
    mut delete_child: impl FnMut(&Value, Span) -> RuntimeResult<Value>,
) -> RuntimeResult<()> {
    let (class_name, owner_key) = {
        let inst_ref = instance.borrow();
        let key = match inst_ref.get("_key") {
            Some(Value::String(s)) => s.to_string(),
            _ => return Ok(()),
        };
        (inst_ref.class.name.clone(), key)
    };

    let dependents: Vec<RelationDef> = get_relations(&class_name)
        .into_iter()
        .filter(|rel| rel.dependent.is_some())
        .collect();
    if dependents.is_empty() {
        return Ok(());
    }

    if cascade_depth() > MAX_CASCADE_DEPTH {
        return Err(RuntimeError::new(
            format!(
                "dependent delete recursion exceeded {} levels — cycle in `dependent:` declarations?",
                MAX_CASCADE_DEPTH
            ),
            span,
        ));
    }

    let fallback_class = instance.borrow().class.clone();
    for rel in dependents {
        match rel.dependent.expect("filtered on is_some") {
            DependentStrategy::DeleteAll => {
                let qb = relation_query_builder(&rel, &owner_key, &fallback_class);
                let result =
                    crate::interpreter::builtins::model::execute_query_builder_delete_all(&qb);
                if let Value::String(s) = &result {
                    if s.starts_with("Error:") {
                        return Err(RuntimeError::new(
                            format!("dependent: \"delete_all\" on {} failed: {}", rel.name, s),
                            span,
                        ));
                    }
                }
            }
            DependentStrategy::Nullify => {
                let qb = relation_query_builder(&rel, &owner_key, &fallback_class);
                let mut patch = serde_json::Map::new();
                patch.insert(rel.foreign_key.clone(), serde_json::Value::Null);
                if let Some(type_field) = &rel.polymorphic_type_field {
                    patch.insert(type_field.clone(), serde_json::Value::Null);
                }
                let result = crate::interpreter::builtins::model::execute_query_builder_update_all(
                    &qb,
                    serde_json::Value::Object(patch),
                );
                if let Value::String(s) = &result {
                    if s.starts_with("Error:") {
                        return Err(RuntimeError::new(
                            format!("dependent: \"nullify\" on {} failed: {}", rel.name, s),
                            span,
                        ));
                    }
                }
            }
            DependentStrategy::Delete => {
                let children = load_cascade_children(&rel, &owner_key, span)?;
                for child in &children {
                    let result = delete_child(child, span)?;
                    let failed = matches!(&result, Value::String(s) if s.starts_with("Error:"))
                        || matches!(&result, Value::Bool(false));
                    if failed {
                        let child_key = match child {
                            Value::Instance(inst) => inst
                                .borrow()
                                .get("_key")
                                .and_then(|v| match v {
                                    Value::String(s) => Some(s.to_string()),
                                    _ => None,
                                })
                                .unwrap_or_else(|| "<unknown>".to_string()),
                            _ => "<unknown>".to_string(),
                        };
                        return Err(RuntimeError::new(
                            format!(
                                "dependent: \"delete\" aborted: child {}/{} could not be deleted (callback veto or DB error)",
                                rel.collection, child_key
                            ),
                            span,
                        ));
                    }
                }
            }
        }
    }

    Ok(())
}

impl Interpreter {
    /// Run every `dependent:` strategy declared by the owner's class, in
    /// declaration order. Called after `before_delete` (a veto also skips
    /// cascades) and before the owner row is removed — Rails ordering.
    pub(crate) fn run_dependent_cascades(
        &mut self,
        instance: &Rc<RefCell<Instance>>,
        span: Span,
    ) -> RuntimeResult<()> {
        run_dependent_cascades_with(instance, span, |child, span| {
            self.delete_model_instance(child, span)
        })
    }

    /// Delete a model instance through the same path a user-level
    /// `record.delete()` takes: the callback/cascade interceptor when the
    /// class needs it, else the plain native method.
    pub(crate) fn delete_model_instance(
        &mut self,
        instance_value: &Value,
        span: Span,
    ) -> RuntimeResult<Value> {
        if let Some(result) =
            self.try_run_model_delete_callbacks(instance_value, "delete", &[], span)?
        {
            return Ok(result);
        }
        let callee = self.evaluate_member_on_value(instance_value.clone(), "delete", span)?;
        self.call_value(callee, Vec::new(), span)
    }

    /// Save a model instance through the same path a user-level
    /// `record.save()` takes: the persist-callback interceptor when the
    /// class registers callbacks (new records run the create chain), else
    /// the plain native method. Validations, counter caches, and dirty
    /// tracking all apply either way. Used by the association writers
    /// (`owner.rel << record`, `owner.rel.create(hash)`).
    pub(crate) fn save_model_instance(
        &mut self,
        instance_value: &Value,
        span: Span,
    ) -> RuntimeResult<Value> {
        if let Some(result) =
            self.try_run_model_persist_callbacks(instance_value, "save", &[], span)?
        {
            return Ok(result);
        }
        let callee = self.evaluate_member_on_value(instance_value.clone(), "save", span)?;
        self.call_value(callee, Vec::new(), span)
    }
}
