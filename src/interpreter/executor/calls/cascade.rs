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

impl Interpreter {
    /// Run every `dependent:` strategy declared by the owner's class, in
    /// declaration order. Called after `before_delete` (a veto also skips
    /// cascades) and before the owner row is removed — Rails ordering.
    pub(crate) fn run_dependent_cascades(
        &mut self,
        instance: &Rc<RefCell<Instance>>,
        span: Span,
    ) -> RuntimeResult<()> {
        let (class_name, owner_key) = {
            let inst_ref = instance.borrow();
            let key = match inst_ref.get("_key") {
                Some(Value::String(s)) => s.to_string(),
                // Unpersisted owner: nothing can reference it yet.
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
                    // Polymorphic inverse: clear the type discriminator too,
                    // so the orphan doesn't keep a dangling half-reference.
                    if let Some(type_field) = &rel.polymorphic_type_field {
                        patch.insert(type_field.clone(), serde_json::Value::Null);
                    }
                    let result =
                        crate::interpreter::builtins::model::execute_query_builder_update_all(
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
                    self.cascade_delete_children(&rel, &owner_key, span)?;
                }
            }
        }

        Ok(())
    }

    /// `dependent: "delete"`: load each child and delete it through the
    /// interpreter so its callbacks, nested cascades, soft-delete semantics,
    /// and counter-cache bumps all run. A child veto or error aborts the
    /// rest of the cascade and the owner delete.
    fn cascade_delete_children(
        &mut self,
        rel: &RelationDef,
        owner_key: &str,
        span: Span,
    ) -> RuntimeResult<()> {
        use crate::interpreter::builtins::model::crud;

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

        // Skip already-soft-deleted children of soft-delete child models —
        // they are invisible to the default scope and stay archived.
        let soft_guard = if crate::interpreter::builtins::model::is_soft_delete(&rel.class_name) {
            " AND doc.deleted_at == null"
        } else {
            ""
        };
        let limit =
            if rel.relation_type == crate::interpreter::builtins::model::RelationType::HasOne {
                " LIMIT 1"
            } else {
                ""
            };
        let mut binds = HashMap::new();
        binds.insert(
            "fk".to_string(),
            serde_json::Value::String(owner_key.to_string()),
        );
        // `as:` inverse of a polymorphic belongs_to: only this owner type's
        // children cascade.
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

        for doc in &docs {
            let child = crud::json_doc_to_instance(&child_class, doc);
            let result = self.delete_model_instance(&child, span)?;
            let failed = matches!(&result, Value::String(s) if s.starts_with("Error:"))
                || matches!(&result, Value::Bool(false));
            if failed {
                let child_key = doc
                    .get("_key")
                    .and_then(|v| v.as_str())
                    .unwrap_or("<unknown>");
                return Err(RuntimeError::new(
                    format!(
                        "dependent: \"delete\" aborted: child {}/{} could not be deleted (callback veto or DB error)",
                        rel.collection, child_key
                    ),
                    span,
                ));
            }
        }

        Ok(())
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
