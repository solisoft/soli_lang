//! `Model`'s instance methods — the ones called on a record rather than on
//! the class: `user.update()`, `user.delete()`, `user.to_h()` and the rest.
//!
//! Split out of `register_model_class`, which registered the whole ORM surface
//! as inline closures in one 4,329-line function — the largest in the
//! repository, and why `core.rs` was over seven thousand lines. Every ORM
//! change touched that function, so every parallel change conflicted inside it.
//!
//! These files are siblings of `core.rs` rather than a `register/`
//! subdirectory on purpose: the moved code carries 224 `super::` paths to
//! sixteen sibling modules, and a directory would have changed what every one
//! of them means.
//!
//! Nothing here is new. Each `register` fills the same map the one function
//! filled, in the same order — though the order does not matter, since every
//! entry is an independent closure keyed by its own name.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use super::core::*;
use super::crud::{exec_delete, exec_get, exec_insert, exec_update, json_to_value};
use super::query::QueryBuilder;
use crate::interpreter::value::{value_to_json, HashKey};
use crate::interpreter::value::{NativeFunction, Value};

pub(super) fn register(native_methods: &mut HashMap<String, Rc<NativeFunction>>) {
    // Instance Methods (called on model instances: user.update(), user.delete())
    // ====================================================================

    // instance.to_h() - The instance's user fields as a Hash. Drops the
    // `_`-prefixed framework fields (`_key`, `_id`, `_rev`, `_errors`, …),
    // returning just the user-assigned attributes. Handy for serialization,
    // diffing, and content hashing (e.g. Ledger records).
    native_methods.insert(
        "to_h".to_string(),
        Rc::new(NativeFunction::new_auto_invocable(
            "Model#to_h",
            None,
            |args| {
                let instance = match &args[0] {
                    Value::Instance(inst) => inst.clone(),
                    _ => return Err("Expected instance".to_string()),
                };
                let inst_ref = instance.borrow();
                Ok(instance_fields_to_hash(&inst_ref))
            },
        )),
    );

    // instance.update() - Persist current instance fields to DB
    // Returns true on success, false on validation/DB error (errors stored in _errors)
    native_methods.insert(
        "update".to_string(),
        Rc::new(NativeFunction::new_auto_invocable(
            "Model#update",
            None,
            #[allow(clippy::collapsible_match)]
            |args| {
                use super::validation::run_validations;
                use crate::interpreter::builtins::i18n::helpers as i18n_helpers;
                use crate::interpreter::builtins::model::get_translated_fields;

                let instance = match &args[0] {
                    Value::Instance(inst) => inst.clone(),
                    _ => return Err("Expected instance".to_string()),
                };

                {
                    let class_name = instance.borrow().class.name.clone();
                    if super::registry::is_timeseries_model(&class_name) {
                        return Err(timeseries_insert_only_error(&class_name, "update"));
                    }
                }

                // Optional hash of attributes: `inst.update({...})`
                // applies the hash to instance fields before running
                // the existing persist pipeline, so no-arg callers keep
                // working unchanged. Hash-applied mutations are kept on
                // the in-memory instance even if validation or the DB
                // call later fails.
                match args.len() {
                    1 => {}
                    2 => apply_hash_to_instance(&instance, &args[1])?,
                    n => {
                        return Err(format!(
                            "update takes 0 or 1 arguments (a hash of attributes), got {}",
                            n - 1
                        ))
                    }
                }

                let inst_ref = instance.borrow();
                let class_name = inst_ref.class.name.clone();
                let collection = class_name_to_collection(&class_name);
                let key = inst_ref
                    .get("_key")
                    .ok_or_else(|| "Instance has no _key field".to_string())?;
                let key_str = match key {
                    Value::String(s) => s,
                    _ => return Err("_key is not a string".to_string()),
                };

                // Handle pending translations before updating
                let translated_field_names = get_translated_fields(&class_name);
                if !translated_field_names.is_empty() {
                    let locale = i18n_helpers::get_locale();

                    // Get or create translated_fields JSON structure
                    let mut translated_fields_json: serde_json::Map<String, serde_json::Value> =
                        serde_json::Map::new();

                    // If instance already has translated_fields, copy it
                    if let Some(tf) = inst_ref.get("translated_fields") {
                        if let Ok(tf_json) = value_to_json(&tf) {
                            if let serde_json::Value::Object(obj) = tf_json {
                                translated_fields_json = obj;
                            }
                        }
                    }

                    // Get pending translations and merge them
                    if let Some(pending) = inst_ref.get("_pending_translations") {
                        if let Ok(pending_json) = value_to_json(&pending) {
                            if let serde_json::Value::Object(pending_obj) = pending_json {
                                for field_name in &translated_field_names {
                                    if let Some(pending_value) = pending_obj.get(field_name) {
                                        // Get or create the locale object for this field
                                        let field_obj = translated_fields_json
                                            .entry(field_name.clone())
                                            .or_insert_with(|| {
                                                serde_json::Value::Object(serde_json::Map::new())
                                            });

                                        if let serde_json::Value::Object(ref mut locale_obj) =
                                            *field_obj
                                        {
                                            locale_obj
                                                .insert(locale.clone(), pending_value.clone());
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Update the instance's translated_fields field
                    drop(inst_ref);
                    let mut inst_mut = instance.borrow_mut();
                    inst_mut.fields.insert(
                        "translated_fields".into(),
                        json_to_value(&serde_json::Value::Object(translated_fields_json)),
                    );

                    // Clear pending translations
                    inst_mut.fields.remove("_pending_translations");
                    drop(inst_mut);
                } else {
                    drop(inst_ref);
                }

                // Run validations
                let inst_ref2 = instance.borrow();
                let data_hash = instance_fields_to_hash(&inst_ref2);
                let errors = run_validations(&class_name, &data_hash, Some(&key_str))?;
                if !errors.is_empty() {
                    let error_values: Vec<Value> = errors.iter().map(|e| e.to_value()).collect();
                    drop(inst_ref2);
                    instance
                        .borrow_mut()
                        .set("_errors", Value::Array(Rc::new(RefCell::new(error_values))));
                    return Ok(Value::Bool(false));
                }

                let mut map = serde_json::Map::new();
                for (k, v) in &inst_ref2.fields {
                    if !k.starts_with('_') {
                        map.insert(k.to_string(), value_to_json(v)?);
                    }
                }
                drop(inst_ref2);
                match exec_update(&collection, &key_str, serde_json::Value::Object(map), true) {
                    Ok(result) => {
                        let mut inst_mut = instance.borrow_mut();
                        if let serde_json::Value::Object(ref res_map) = result {
                            if let Some(rev) = res_map.get("_rev") {
                                inst_mut.set("_rev", json_to_value(rev));
                            }
                        }
                        // A column-aware update answers with the stored row
                        // (see `adopt_row_fields`): adopting it refreshes
                        // the database's own `updated_at`.
                        let column_schema = super::column_mode::schema_for_collection(&collection);
                        if column_schema.is_some() {
                            super::column_mode::adopt_row_fields(&mut inst_mut, &result);
                        }
                        inst_mut.set("_errors", Value::Array(Rc::new(RefCell::new(vec![]))));
                        let changes = super::dirty::finalize_persist(&mut inst_mut);
                        super::counter_cache::bump_for_changes(&inst_mut, &changes);
                        drop(inst_mut);
                        if let Some(schema) = column_schema {
                            super::column_mode::convert_temporal_fields(
                                &schema,
                                Value::Instance(instance.clone()),
                            );
                        }
                        Ok(Value::Bool(true))
                    }
                    Err(e) => {
                        let error_values = build_persistence_errors(&class_name, e);
                        instance
                            .borrow_mut()
                            .set("_errors", Value::Array(Rc::new(RefCell::new(error_values))));
                        Ok(Value::Bool(false))
                    }
                }
            },
        )),
    );

    // instance.save([hash]) - Insert or update depending on whether _key
    // exists. Optional hash argument applies bulk attribute assignments
    // before the persist pipeline, so zero-arg callers keep working.
    // Returns true on success, false on validation/DB error (errors stored in _errors)
    native_methods.insert(
        "save".to_string(),
        Rc::new(NativeFunction::new_auto_invocable(
            "Model#save",
            None,
            #[allow(clippy::collapsible_match)]
            |args| {
                use super::validation::run_validations;
                use crate::interpreter::builtins::i18n::helpers as i18n_helpers;
                use crate::interpreter::builtins::model::get_translated_fields;

                let instance = match &args[0] {
                    Value::Instance(inst) => inst.clone(),
                    _ => return Err("Expected instance".to_string()),
                };

                // Optional hash of attributes: `inst.save({...})` applies
                // the hash to instance fields before running the existing
                // persist pipeline, so no-arg callers keep working.
                // Hash-applied mutations are kept on the in-memory instance
                // even if validation or the DB call later fails.
                match args.len() {
                    1 => {}
                    2 => apply_hash_to_instance(&instance, &args[1])?,
                    n => {
                        return Err(format!(
                            "save takes 0 or 1 arguments (a hash of attributes), got {}",
                            n - 1
                        ))
                    }
                }

                // Edge models: coerce `from`/`to` fields (set via
                // save({from: ..., to: ...}) or plain assignment) into
                // _from/_to before the persisted map is built.
                {
                    let class_name = instance.borrow().class.name.clone();
                    if let Some(edge_spec) = super::registry::get_edge_spec(&class_name) {
                        let mut inst_mut = instance.borrow_mut();
                        for (field, expected, target) in [
                            ("from", &edge_spec.from_collection, "_from"),
                            ("to", &edge_spec.to_collection, "_to"),
                        ] {
                            if let Some(val) = inst_mut.get(field) {
                                match super::graph::edge_ref(&val, expected, field) {
                                    Ok(r) => {
                                        inst_mut.fields.remove(field);
                                        inst_mut.set(target.to_string(), Value::String(r.into()));
                                    }
                                    Err(message) => {
                                        let mut pairs =
                                            crate::interpreter::value::HashPairs::default();
                                        pairs.insert(
                                            HashKey::String("field".into()),
                                            Value::String(field.into()),
                                        );
                                        pairs.insert(
                                            HashKey::String("message".into()),
                                            Value::String(message.into()),
                                        );
                                        let err_hash = Value::Hash(Rc::new(RefCell::new(pairs)));
                                        inst_mut.set(
                                            "_errors",
                                            Value::Array(Rc::new(RefCell::new(vec![err_hash]))),
                                        );
                                        return Ok(Value::Bool(false));
                                    }
                                }
                            }
                        }
                    }
                }

                let inst_ref = instance.borrow();
                let class_name = inst_ref.class.name.clone();
                let collection = class_name_to_collection(&class_name);
                let key_opt = inst_ref.get("_key").and_then(|k| match k {
                    Value::String(s) => Some(s.clone()),
                    _ => None,
                });

                // Timeseries models are insert-only: saving an existing
                // record is an update, which the DB rejects — surface a
                // clear error before any DB round trip.
                if key_opt.is_some() && super::registry::is_timeseries_model(&class_name) {
                    return Err(timeseries_insert_only_error(&class_name, "save"));
                }

                // Handle pending translations before saving
                let translated_field_names = get_translated_fields(&class_name);
                let has_translations = !translated_field_names.is_empty();

                // Pre-compute translation data while we have inst_ref
                let translation_update: Option<(
                    serde_json::Map<String, serde_json::Value>,
                    Vec<String>,
                )> = if has_translations {
                    let locale = i18n_helpers::get_locale();

                    // Get or create translated_fields JSON structure
                    let mut translated_fields_json: serde_json::Map<String, serde_json::Value> =
                        serde_json::Map::new();

                    // If instance already has translated_fields, copy it
                    if let Some(tf) = inst_ref.get("translated_fields") {
                        if let Ok(tf_json) = value_to_json(&tf) {
                            if let serde_json::Value::Object(obj) = tf_json {
                                translated_fields_json = obj;
                            }
                        }
                    }

                    // Get pending translations and merge them
                    if let Some(pending) = inst_ref.get("_pending_translations") {
                        if let Ok(pending_json) = value_to_json(&pending) {
                            if let serde_json::Value::Object(pending_obj) = pending_json {
                                for field_name in &translated_field_names {
                                    if let Some(pending_value) = pending_obj.get(field_name) {
                                        // Get or create the locale object for this field
                                        let field_obj = translated_fields_json
                                            .entry(field_name.clone())
                                            .or_insert_with(|| {
                                                serde_json::Value::Object(serde_json::Map::new())
                                            });

                                        if let serde_json::Value::Object(ref mut locale_obj) =
                                            *field_obj
                                        {
                                            locale_obj
                                                .insert(locale.clone(), pending_value.clone());
                                        }
                                    }
                                }
                            }
                        }
                    }

                    Some((translated_fields_json, translated_field_names))
                } else {
                    None
                };

                // Get data we need before dropping inst_ref
                let data_hash = instance_fields_to_hash(&inst_ref);

                // Build map for DB operation before dropping inst_ref.
                // `_`-prefixed fields are DB-managed and stripped — except
                // _from/_to on edge models, which ARE the edge payload.
                let is_edge = super::registry::is_edge_model(&class_name);
                let mut map = serde_json::Map::new();
                for (k, v) in &inst_ref.fields {
                    if !k.starts_with('_') || (is_edge && matches!(k.as_str(), "_from" | "_to")) {
                        map.insert(k.to_string(), value_to_json(v)?);
                    }
                }

                // Now we can drop inst_ref
                drop(inst_ref);

                // Apply translation update if needed
                if let Some((translated_fields_json, _)) = translation_update {
                    let mut inst_mut = instance.borrow_mut();
                    inst_mut.fields.insert(
                        "translated_fields".into(),
                        json_to_value(&serde_json::Value::Object(translated_fields_json)),
                    );
                    // Clear pending translations
                    inst_mut.fields.remove("_pending_translations");
                }

                // Run validations
                let errors = run_validations(&class_name, &data_hash, key_opt.as_deref())?;
                if !errors.is_empty() {
                    let error_values: Vec<Value> = errors.iter().map(|e| e.to_value()).collect();
                    instance
                        .borrow_mut()
                        .set("_errors", Value::Array(Rc::new(RefCell::new(error_values))));
                    return Ok(Value::Bool(false));
                }

                if let Some(ref key_str) = key_opt {
                    // Update existing document
                    match exec_update(&collection, key_str, serde_json::Value::Object(map), true) {
                        Ok(result) => {
                            let mut inst_mut = instance.borrow_mut();
                            if let serde_json::Value::Object(ref res_map) = result {
                                if let Some(rev) = res_map.get("_rev") {
                                    inst_mut.set("_rev", json_to_value(rev));
                                }
                            }
                            inst_mut.set("_errors", Value::Array(Rc::new(RefCell::new(vec![]))));
                            let changes = super::dirty::finalize_persist(&mut inst_mut);
                            super::counter_cache::bump_for_changes(&inst_mut, &changes);
                            Ok(Value::Bool(true))
                        }
                        Err(e) => {
                            let error_values = build_persistence_errors(&class_name, e);
                            instance
                                .borrow_mut()
                                .set("_errors", Value::Array(Rc::new(RefCell::new(error_values))));
                            Ok(Value::Bool(false))
                        }
                    }
                } else {
                    // Insert new document
                    let mut map = map;
                    // STI subclasses stamp their discriminator so rows in
                    // the shared base collection hydrate as the right class.
                    if super::registry::is_sti_subclass(&class_name) {
                        map.insert(
                            "type".to_string(),
                            serde_json::Value::String(class_name.clone()),
                        );
                        instance
                            .borrow_mut()
                            .set("type", Value::String(class_name.as_str().into()));
                    }
                    match exec_insert(&collection, None, serde_json::Value::Object(map)) {
                        Ok(result) => {
                            let mut inst_mut = instance.borrow_mut();
                            if let serde_json::Value::Object(ref res_map) = result {
                                for field in &["_key", "_id", "_rev", "_created_at", "_updated_at"]
                                {
                                    if let Some(val) = res_map.get(*field) {
                                        inst_mut.set(field.to_string(), json_to_value(val));
                                    }
                                }
                            }
                            inst_mut.set("_errors", Value::Array(Rc::new(RefCell::new(vec![]))));
                            super::dirty::finalize_persist(&mut inst_mut);
                            super::counter_cache::bump_for_instance(&inst_mut, 1);
                            Ok(Value::Bool(true))
                        }
                        Err(e) => {
                            let error_values = build_persistence_errors(&class_name, e);
                            instance
                                .borrow_mut()
                                .set("_errors", Value::Array(Rc::new(RefCell::new(error_values))));
                            Ok(Value::Bool(false))
                        }
                    }
                }
            },
        )),
    );

    // instance.delete() - Delete (or soft-delete) the document from DB.
    // before_delete/after_delete run in the executor interceptor so user
    // methods and closures can execute with `this` bound to the instance.
    native_methods.insert(
        "delete".to_string(),
        Rc::new(NativeFunction::new("Model#delete", Some(0), |args| {
            let instance = match &args[0] {
                Value::Instance(inst) => inst.clone(),
                _ => return Err("Expected instance".to_string()),
            };
            let inst_ref = instance.borrow();
            let class_name = inst_ref.class.name.clone();
            let collection = class_name_to_collection(&class_name);
            let key = inst_ref
                .get("_key")
                .ok_or_else(|| "Instance has no _key field".to_string())?;
            let key_str = match key {
                Value::String(s) => s,
                _ => return Err("_key is not a string".to_string()),
            };
            drop(inst_ref);

            if is_soft_delete(&class_name) {
                // Soft delete: set deleted_at timestamp
                let was_active = matches!(
                    instance.borrow().get("deleted_at"),
                    None | Some(Value::Null)
                );
                let now = chrono::Utc::now().to_rfc3339();
                let mut map = serde_json::Map::new();
                map.insert(
                    "deleted_at".to_string(),
                    serde_json::Value::String(now.clone()),
                );
                match exec_update(&collection, &key_str, serde_json::Value::Object(map), true) {
                    Ok(_) => {
                        let mut inst_mut = instance.borrow_mut();
                        inst_mut.set("deleted_at", Value::String(now.into()));
                        super::dirty::sync_snapshot_field(&mut inst_mut, "deleted_at");
                        // Counters track default-scope-visible children:
                        // vanishing from the scope decrements the parent.
                        if was_active {
                            super::counter_cache::bump_for_instance(&inst_mut, -1);
                        }
                        Ok(Value::Bool(true))
                    }
                    Err(e) => Ok(Value::String(format!("Error: {}", e).into())),
                }
            } else {
                // Hard delete: remove document
                match exec_delete(&collection, &key_str) {
                    Ok(result) => {
                        super::counter_cache::bump_for_instance(&instance.borrow(), -1);
                        Ok(json_to_value(&result))
                    }
                    Err(e) => Ok(Value::String(format!("Error: {}", e).into())),
                }
            }
        })),
    );

    // instance.restore() - Restore a soft-deleted record (clear deleted_at)
    native_methods.insert(
        "restore".to_string(),
        Rc::new(NativeFunction::new("Model#restore", Some(0), |args| {
            let instance = match &args[0] {
                Value::Instance(inst) => inst.clone(),
                _ => return Err("Expected instance".to_string()),
            };
            let inst_ref = instance.borrow();
            let collection = class_name_to_collection(&inst_ref.class.name);
            let key = match inst_ref.get("_key") {
                Some(k) => k,
                None => return Ok(Value::Instance(instance.clone())),
            };
            let key_str = match key {
                Value::String(s) => s,
                _ => return Err("_key is not a string".to_string()),
            };
            drop(inst_ref);

            let was_deleted = !matches!(
                instance.borrow().get("deleted_at"),
                None | Some(Value::Null)
            );
            let mut map = serde_json::Map::new();
            map.insert("deleted_at".to_string(), serde_json::Value::Null);
            match exec_update(&collection, &key_str, serde_json::Value::Object(map), true) {
                Ok(_) => {
                    let mut inst_mut = instance.borrow_mut();
                    inst_mut.set("deleted_at", Value::Null);
                    super::dirty::sync_snapshot_field(&mut inst_mut, "deleted_at");
                    // Re-entering the default scope re-increments the parent.
                    if was_deleted {
                        super::counter_cache::bump_for_instance(&inst_mut, 1);
                    }
                    Ok(Value::Bool(true))
                }
                Err(e) => Err(format!("restore failed: {}", e)),
            }
        })),
    );

    // instance.increment(field, amount?) - Atomically bump a numeric field.
    // Uses a fetch + If-Match CAS retry loop so concurrent increments cannot
    // lose updates (see crud::cas_field_delta).
    native_methods.insert(
        "increment".to_string(),
        Rc::new(NativeFunction::new("Model#increment", None, |args| {
            apply_field_delta(args, /*sign=*/ 1, "increment")
        })),
    );

    // instance.decrement(field, amount?) - Atomically subtract from a numeric field.
    native_methods.insert(
        "decrement".to_string(),
        Rc::new(NativeFunction::new("Model#decrement", None, |args| {
            apply_field_delta(args, /*sign=*/ -1, "decrement")
        })),
    );

    // instance.touch() - Update the _updated_at timestamp
    native_methods.insert(
        "touch".to_string(),
        Rc::new(NativeFunction::new("Model#touch", Some(0), |args| {
            let instance = match &args[0] {
                Value::Instance(inst) => inst.clone(),
                _ => return Err("Expected instance".to_string()),
            };
            let inst_ref = instance.borrow();
            let collection = class_name_to_collection(&inst_ref.class.name);
            let key = match inst_ref.get("_key") {
                Some(k) => k,
                None => return Ok(Value::Instance(instance.clone())),
            };
            let key_str = match key {
                Value::String(s) => s,
                _ => return Err("_key is not a string".to_string()),
            };
            drop(inst_ref);

            let now = chrono::Utc::now().to_rfc3339();
            let mut map = serde_json::Map::new();
            map.insert(
                "_updated_at".to_string(),
                serde_json::Value::String(now.clone()),
            );
            match exec_update(&collection, &key_str, serde_json::Value::Object(map), true) {
                Ok(_) => {
                    instance
                        .borrow_mut()
                        .set("_updated_at", Value::String(now.into()));
                    Ok(Value::Instance(instance))
                }
                Err(e) => Err(format!("touch failed: {}", e)),
            }
        })),
    );

    // instance.errors - Return the list of errors from last save/update
    native_methods.insert(
        "errors".to_string(),
        Rc::new(NativeFunction::new("Model#errors", Some(0), |args| {
            let instance = match &args[0] {
                Value::Instance(inst) => inst.clone(),
                _ => return Err("Expected instance".to_string()),
            };
            let inst_ref = instance.borrow();
            match inst_ref.get("_errors") {
                Some(errors) => Ok(errors),
                None => Ok(Value::Array(Rc::new(RefCell::new(vec![])))),
            }
        })),
    );

    // instance.reload - Re-fetch from DB and refresh all fields
    native_methods.insert(
        "reload".to_string(),
        Rc::new(NativeFunction::new("Model#reload", Some(0), |args| {
            let instance = match &args[0] {
                Value::Instance(inst) => inst.clone(),
                _ => return Err("Expected instance".to_string()),
            };
            let inst_ref = instance.borrow();
            let collection = class_name_to_collection(&inst_ref.class.name);
            let key = inst_ref
                .get("_key")
                .ok_or_else(|| "Instance has no _key field, cannot reload".to_string())?;
            let key_str = match key {
                Value::String(s) => s,
                _ => return Err("_key is not a string".to_string()),
            };
            drop(inst_ref);
            match exec_get(&collection, &key_str) {
                Ok(doc) => {
                    if let serde_json::Value::Object(map) = &doc {
                        let mut inst_mut = instance.borrow_mut();
                        for (k, v) in map {
                            inst_mut.set(k.clone(), json_to_value(v));
                        }
                        super::dirty::seed_snapshot(&mut inst_mut);
                        inst_mut.previous_changes = None;
                    }
                    Ok(Value::Instance(instance))
                }
                Err(e) => Err(format!("reload failed: {}", e)),
            }
        })),
    );

    // Dirty tracking. The baseline snapshot is seeded on DB load /
    // successful persist (see model::dirty); these natives compare the
    // live fields against it lazily. All are auto-invocable so bare
    // `record.changed?` works.
    native_methods.insert(
        "changed?".to_string(),
        Rc::new(NativeFunction::new_auto_invocable(
            "Model#changed?",
            Some(0),
            |args| {
                let instance = match &args[0] {
                    Value::Instance(inst) => inst.clone(),
                    _ => return Err("Expected instance".to_string()),
                };
                let inst_ref = instance.borrow();
                Ok(Value::Bool(
                    !super::dirty::compute_changes(&inst_ref).is_empty(),
                ))
            },
        )),
    );

    // instance.changed - array of changed attribute names, sorted.
    native_methods.insert(
        "changed".to_string(),
        Rc::new(NativeFunction::new_auto_invocable(
            "Model#changed",
            Some(0),
            |args| {
                let instance = match &args[0] {
                    Value::Instance(inst) => inst.clone(),
                    _ => return Err("Expected instance".to_string()),
                };
                let inst_ref = instance.borrow();
                let names: Vec<Value> = super::dirty::compute_changes(&inst_ref)
                    .into_iter()
                    .map(|(name, _, _)| Value::String(name))
                    .collect();
                Ok(Value::Array(Rc::new(RefCell::new(names))))
            },
        )),
    );

    // instance.changes - { "name": [old, new] } for unsaved changes.
    native_methods.insert(
        "changes".to_string(),
        Rc::new(NativeFunction::new_auto_invocable(
            "Model#changes",
            Some(0),
            |args| {
                let instance = match &args[0] {
                    Value::Instance(inst) => inst.clone(),
                    _ => return Err("Expected instance".to_string()),
                };
                let inst_ref = instance.borrow();
                Ok(super::dirty::changes_to_hash(
                    &super::dirty::compute_changes(&inst_ref),
                ))
            },
        )),
    );

    // instance.previous_changes - what the last successful save/update/
    // create persisted, as { "name": [old, new] }. Empty hash when the
    // record has never been persisted by this instance.
    native_methods.insert(
        "previous_changes".to_string(),
        Rc::new(NativeFunction::new_auto_invocable(
            "Model#previous_changes",
            Some(0),
            |args| {
                let instance = match &args[0] {
                    Value::Instance(inst) => inst.clone(),
                    _ => return Err("Expected instance".to_string()),
                };
                let inst_ref = instance.borrow();
                let changes = inst_ref
                    .previous_changes
                    .as_deref()
                    .cloned()
                    .unwrap_or_default();
                Ok(super::dirty::changes_to_hash(&changes))
            },
        )),
    );

    // instance.attribute_was("name") - the baseline value of one
    // attribute (null on a new record or unknown attribute).
    native_methods.insert(
        "attribute_was".to_string(),
        Rc::new(NativeFunction::new(
            "Model#attribute_was",
            Some(1),
            |args| {
                let instance = match &args[0] {
                    Value::Instance(inst) => inst.clone(),
                    _ => return Err("Expected instance".to_string()),
                };
                let name = match args.get(1) {
                    Some(Value::String(s)) => s.to_string(),
                    _ => return Err("attribute_was() expects a string attribute name".to_string()),
                };
                let inst_ref = instance.borrow();
                let value = inst_ref
                    .original_fields
                    .as_deref()
                    .and_then(|original| original.get(name.as_str()).cloned())
                    .unwrap_or(Value::Null);
                Ok(value)
            },
        )),
    );

    // instance.traverse(EdgeModel[, {direction:, depth:}]) — graph
    // traversal from this record. Returns a chainable QueryBuilder whose
    // vertex variable is `doc`, so .where/.order/.limit/.count compose
    // exactly like a collection query.
    native_methods.insert(
        "traverse".to_string(),
        Rc::new(NativeFunction::new("Model#traverse", None, |args| {
            let instance = match &args[0] {
                Value::Instance(inst) => inst.clone(),
                _ => return Err("Expected instance".to_string()),
            };
            let own_class_name = instance.borrow().class.name.clone();
            let own_collection = class_name_to_collection(&own_class_name);
            let start_id = super::graph::edge_ref(
                &Value::Instance(instance.clone()),
                &own_collection,
                "traverse()",
            )
            .map_err(|_| {
                format!(
                    "traverse() requires a saved record ({} instance has no _key)",
                    own_class_name
                )
            })?;

            let spec = super::graph::parse_traverse_options(&args[1..])?;

            // The result class: follow the edge declaration in the walk
            // direction; per-document _id resolution corrects mixed
            // results at materialization time.
            let target_collection = match (&spec.edge_spec, spec.direction) {
                (Some(es), super::graph::TraversalDirection::Out) => es.to_collection.clone(),
                (Some(es), super::graph::TraversalDirection::In) => es.from_collection.clone(),
                _ => own_collection.clone(),
            };
            let target_class_name = super::relations::classify(&target_collection);
            let target_class = super::registry::get_model_class(&target_class_name)
                .or_else(|| super::registry::get_model_class(&own_class_name));

            let mut qb = match target_class {
                Some(class) => QueryBuilder::new_with_class(
                    target_class_name,
                    spec.edge_collection.clone(),
                    class,
                ),
                None => QueryBuilder::new(target_class_name, spec.edge_collection.clone()),
            };
            qb.bind_vars.insert(
                crate::interpreter::get_symbol(super::graph::TRAVERSE_START_BIND),
                serde_json::Value::String(start_id),
            );
            qb.traversal = Some(super::graph::TraversalClause {
                edge_collection: spec.edge_collection,
                direction: spec.direction,
                min_depth: spec.min_depth,
                max_depth: spec.max_depth,
            });
            Ok(Value::QueryBuilder(Rc::new(RefCell::new(qb))))
        })),
    );

    // instance.shortest_path(target, via: EdgeModel[, direction: "any"])
    // — BFS shortest path. Executes immediately; returns the Array of
    // vertices in path order (start → target), or [] when unconnected.
    native_methods.insert(
        "shortest_path".to_string(),
        Rc::new(NativeFunction::new("Model#shortest_path", None, |args| {
            let instance = match &args[0] {
                Value::Instance(inst) => inst.clone(),
                _ => return Err("Expected instance".to_string()),
            };
            let own_class = instance.borrow().class.clone();
            let own_class_name = own_class.name.clone();
            let own_collection = class_name_to_collection(&own_class_name);
            let start_id = super::graph::edge_ref(
                &Value::Instance(instance.clone()),
                &own_collection,
                "shortest_path()",
            )
            .map_err(|_| {
                format!(
                    "shortest_path() requires a saved record ({} instance has no _key)",
                    own_class_name
                )
            })?;

            let target = args.get(1).cloned().ok_or_else(|| {
                "shortest_path(target, via: EdgeModel) requires a target record".to_string()
            })?;

            // Options: via: (required), direction: (default "any").
            let mut via: Option<Value> = None;
            let mut direction = super::graph::TraversalDirection::Any;
            if let Some(opts) = args.get(2) {
                let hash = match opts {
                    Value::Hash(h) => h.clone(),
                    other => {
                        return Err(format!(
                            "shortest_path() options must be a hash, got {}",
                            other.type_name()
                        ))
                    }
                };
                for (k, v) in hash.borrow().iter() {
                    let key = match k {
                        HashKey::String(s) => s.to_string(),
                        _ => continue,
                    };
                    match key.as_str() {
                        "via" => via = Some(v.clone()),
                        "direction" => {
                            let dir = match v {
                                Value::String(s) => s.to_string(),
                                Value::Symbol(s) => s.to_string(),
                                other => {
                                    return Err(format!(
                                        "shortest_path() direction must be a string, got {}",
                                        other.type_name()
                                    ))
                                }
                            };
                            direction = super::graph::TraversalDirection::parse(&dir)?;
                        }
                        other => {
                            return Err(format!(
                                "shortest_path() unknown option '{}': expected via: or \
                                     direction:",
                                other
                            ))
                        }
                    }
                }
            }
            let via = via.ok_or_else(|| {
                "shortest_path() requires via: an edge model, e.g. \
                     shortest_path(other, via: Follow)"
                    .to_string()
            })?;

            let (edge_collection, edge_spec) = match &via {
                Value::Class(c) => {
                    let name = c.name.to_string();
                    let spec = super::registry::get_edge_spec(&name)
                        .ok_or_else(|| format!("{} has no `edge` declaration", name))?;
                    (class_name_to_collection(&name), Some(spec))
                }
                Value::String(s) => (s.to_string(), None),
                other => {
                    return Err(format!(
                        "shortest_path() via: expects an edge model class or collection \
                             name, got {}",
                        other.type_name()
                    ))
                }
            };
            super::graph::validate_collection_ident(&edge_collection, "shortest_path")?;

            // Coerce the target to a full "coll/key" id. Bare keys need a
            // declared edge spec to know the collection; walking OUT ends
            // on to_collection, IN on from_collection, ANY defaults to
            // the receiver's own collection.
            let end_expected = match (&edge_spec, direction) {
                (Some(es), super::graph::TraversalDirection::Out) => es.to_collection.clone(),
                (Some(es), super::graph::TraversalDirection::In) => es.from_collection.clone(),
                _ => own_collection.clone(),
            };
            let end_id = super::graph::any_vertex_ref(&target, &end_expected, "shortest_path()")?;

            let (query, binds) =
                super::graph::shortest_path_query(&edge_collection, direction, &start_id, &end_id);
            Ok(super::crud::exec_auto_collection_as_instances_with_binds(
                query,
                binds,
                &edge_collection,
                &own_class,
            ))
        })),
    );
}
