//! `Model`'s mutations and paging: `update`, `delete`, `reset_counters`,
//! `delete_all`, `transaction`, `count`, the soft-delete scopes, `offset`
//! and `paginate`.
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
use super::crud::json_to_value;
use super::query::QueryBuilder;
use super::validation::run_validations;
use crate::interpreter::value::{value_to_json, HashKey};
use crate::interpreter::value::{NativeFunction, Value};

pub(super) fn register(native_static_methods: &mut HashMap<String, Rc<NativeFunction>>) {
    // Model.update(id, data) - Update document (accepts hash or instance as data)
    use super::crud::exec_update;
    native_static_methods.insert(
        "update".to_string(),
        Rc::new(NativeFunction::new("Model.update", Some(3), |args| {
            let class_name = get_class_name_from_class(args)?;
            let collection = class_name_to_collection(&class_name);

            if super::registry::is_timeseries_model(&class_name) {
                return Err(timeseries_insert_only_error(&class_name, "update"));
            }

            let id = match args.get(1) {
                Some(Value::String(s)) => s.clone(),
                // Integer keys: see Model.find.
                Some(Value::Int(n)) => n.to_string().into(),
                Some(other) => {
                    return Err(format!(
                        "Model.update() expects a string or integer id, got {}",
                        other.type_name()
                    ))
                }
                None => return Err("Model.update() requires id argument".to_string()),
            };

            let data_value: Result<serde_json::Value, String> = match args.get(2) {
                Some(hash_val @ Value::Hash(_)) => {
                    // Strong-params filter for Hash-shaped input. The
                    // Instance branch below skips this on purpose:
                    // instance fields are populated by `Model.find` /
                    // queries, so they're already server-controlled —
                    // applying the whitelist there would corrupt
                    // legitimate persistence rather than block an
                    // attacker.
                    let filtered = filter_mass_assign(&class_name, hash_val);
                    let pairs = match &filtered {
                        Value::Hash(p) => p,
                        _ => unreachable!(),
                    };
                    let mut map = serde_json::Map::new();
                    for (k, v) in pairs.borrow().iter() {
                        if let HashKey::String(key) = k {
                            map.insert(key.clone().to_string(), value_to_json(v)?);
                        }
                    }
                    Ok(serde_json::Value::Object(map))
                }
                Some(Value::Instance(inst)) => {
                    let inst_ref = inst.borrow();
                    let mut map = serde_json::Map::new();
                    for (k, v) in &inst_ref.fields {
                        if !k.starts_with('_') {
                            map.insert(k.to_string(), value_to_json(v)?);
                        }
                    }
                    Ok(serde_json::Value::Object(map))
                }
                Some(other) => Err(format!(
                    "Model.update() expects hash or instance data, got {}",
                    other.type_name()
                )),
                None => Err("Model.update() requires data argument".to_string()),
            };
            let mut data_value = data_value?;
            strip_reserved_document_keys(&mut data_value, &class_name);

            // Counter caches / STI / validations: pre-read the old document
            // (only when this class declares counter-cached belongs_to, is
            // an STI subclass, or has validations) so an FK change can move
            // parent counts, a subclass can refuse rows outside its
            // hierarchy, and validations see the whole record rather than
            // just the changed fields.
            let has_validations = class_has_validations(&class_name);
            let needs_preread = super::counter_cache::class_has_counter_caches(&class_name)
                || super::registry::is_sti_subclass(&class_name)
                || has_validations;
            let old_doc = if needs_preread {
                super::crud::exec_get(&collection, &id).ok()
            } else {
                None
            };

            // The static update used to skip validations entirely — only
            // `Model.create` and the instance mutators ran them — while the
            // docs present it as the ordinary update API. So
            // `Post.update(id, permitted)` wrote blank required fields,
            // invalid enum values and duplicate uniqueness keys straight to
            // the database, past the model's own rules.
            //
            // Validate the *merged* record: a partial update names only the
            // fields it changes, so validating the patch alone would fail
            // every `presence` rule for a field it did not touch.
            if has_validations {
                let merged = merge_json_documents(old_doc.as_ref(), &data_value);
                let errors = run_validations(&class_name, &json_to_value(&merged), Some(&id))?;
                if !errors.is_empty() {
                    let error_values: Vec<Value> = errors.iter().map(|e| e.to_value()).collect();
                    let mut out = crate::interpreter::value::HashPairs::default();
                    out.insert(
                        HashKey::String("_errors".into()),
                        Value::Array(Rc::new(RefCell::new(error_values))),
                    );
                    return Ok(Value::Hash(Rc::new(RefCell::new(out))));
                }
            }
            if super::registry::is_sti_subclass(&class_name)
                && !old_doc
                    .as_ref()
                    .is_some_and(|doc| sti_row_matches(&class_name, doc))
            {
                return Ok(Value::String(
                    format!("Error: {} with id '{}' not found", class_name, id).into(),
                ));
            }

            match exec_update(&collection, &id, data_value.clone(), true) {
                Ok(result) => {
                    if let Some(old_doc) = &old_doc {
                        super::counter_cache::bump_for_json_change(
                            &class_name,
                            old_doc,
                            &data_value,
                        );
                    }
                    Ok(json_to_value(&result))
                }
                Err(e) => Ok(Value::String(format!("Error: {}", e).into())),
            }
        })),
    );

    // Model.delete(id) - Delete document
    use super::crud::exec_delete;
    native_static_methods.insert(
        "delete".to_string(),
        Rc::new(NativeFunction::new("Model.delete", Some(2), |args| {
            let collection = get_collection_from_class(args)?;

            let id = match args.get(1) {
                Some(Value::String(s)) => s.clone(),
                // Integer keys: see Model.find.
                Some(Value::Int(n)) => n.to_string().into(),
                Some(other) => {
                    return Err(format!(
                        "Model.delete() expects a string or integer id, got {}",
                        other.type_name()
                    ))
                }
                None => return Err("Model.delete() requires id argument".to_string()),
            };

            let class_name = get_class_name_from_class(args)?;
            let needs_preread = super::counter_cache::class_has_counter_caches(&class_name)
                || super::registry::is_sti_subclass(&class_name);
            let old_doc = if needs_preread {
                super::crud::exec_get(&collection, &id).ok()
            } else {
                None
            };
            if super::registry::is_sti_subclass(&class_name)
                && !old_doc
                    .as_ref()
                    .is_some_and(|doc| sti_row_matches(&class_name, doc))
            {
                return Ok(Value::String(
                    format!("Error: {} with id '{}' not found", class_name, id).into(),
                ));
            }

            match exec_delete(&collection, &id) {
                Ok(result) => {
                    if let Some(old_doc) = &old_doc {
                        super::counter_cache::bump_for_json(&class_name, old_doc, -1);
                    }
                    Ok(json_to_value(&result))
                }
                Err(e) => Ok(Value::String(format!("Error: {}", e).into())),
            }
        })),
    );

    // Model.reset_counters(id, relation) - Recount a has_many relation's
    // children and write the counter column on the parent row. The
    // repair tool for counter drift (bulk writes skip bumps by design).
    native_static_methods.insert(
        "reset_counters".to_string(),
        Rc::new(NativeFunction::new(
            "Model.reset_counters",
            Some(3),
            |args| {
                let class_name = get_class_name_from_class(args)?;
                let collection = class_name_to_collection(&class_name);
                let id = match args.get(1) {
                    Some(Value::String(s)) => s.to_string(),
                    _ => return Err("Model.reset_counters() expects a string id".to_string()),
                };
                let relation_name = match args.get(2) {
                    Some(Value::String(s)) => s.to_string(),
                    Some(Value::Symbol(s)) => s.to_string(),
                    _ => return Err("Model.reset_counters() expects a relation name".to_string()),
                };
                let count = super::counter_cache::reset_counters(
                    &class_name,
                    &collection,
                    &id,
                    &relation_name,
                )?;
                Ok(Value::Int(count))
            },
        )),
    );

    // Model.delete_all() - Remove every document in the collection.
    // Primarily intended for test setup/teardown. Fetches all keys
    // then deletes each individually — a single `FOR doc IN … REMOVE`
    // query isn't actually applied in SolidB, so iterate. On SQL, use
    // bulk DELETE via the query-builder path.
    native_static_methods.insert(
        "delete_all".to_string(),
        Rc::new(NativeFunction::new("Model.delete_all", Some(1), |args| {
            let class = get_class_rc_from_args(args)?;
            let collection = class_name_to_collection(&class.name);
            // Connection-aware: the bare `db::is_sql()` listed keys from the
            // ambient default and then routed each delete to the model's own
            // connection — two different databases in one operation.
            // A `table "…"` model on a connection that cannot serve it must fail
            // here rather than fall through to the document path below.
            super::column_mode::ensure_supported(&collection)?;
            if super::crud::collection_is_sql(&collection) {
                let qb =
                    QueryBuilder::new_with_class(class.name.clone(), collection, class.clone());
                return Ok(super::query::execute_query_builder_delete_all(&qb));
            }
            let sdbql = format!(
                "FOR doc IN {}{} RETURN doc._key",
                collection,
                sti_scope_clause(&class.name)
            );
            let results = match super::crud::exec_query(&collection, sdbql) {
                Ok(r) => r,
                Err(e) => return Err(format!("Model.delete_all() failed: {}", e)),
            };
            for key_json in &results {
                let key = match key_json {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                let _ = super::crud::exec_delete(&collection, &key);
            }
            Ok(Value::Null)
        })),
    );

    // Model.transaction - Execute SDBQL or get transaction handle
    // Usage:
    //   Model.transaction(sdbql_string) - Execute SDBQL in transaction
    //   Model.transaction() - Get transaction handle: tx = User.transaction(); tx.create({...}); tx.commit()
    native_static_methods.insert(
        "transaction".to_string(),
        Rc::new(NativeFunction::new(
            "Model.transaction",
            Some(1),
            |args| match args.get(1) {
                Some(Value::String(s)) => {
                    use super::crud::exec_transaction_sdbql;
                    match exec_transaction_sdbql(s) {
                        Ok(result) => Ok(json_to_value(&result)),
                        Err(e) => Ok(Value::String(format!("Error: {}", e).into())),
                    }
                }
                Some(Value::Function(_)) | Some(Value::VmClosure(_)) => {
                    // The block form `Model.transaction(fn() { ... })` is run by the
                    // executor interceptor (begin → run → commit / rollback on throw),
                    // which only recognizes an *inline* block literal. Reaching the native
                    // means a non-literal callable was passed; guide the caller instead of
                    // silently dropping the block and returning a handle (the old behavior).
                    Err(
                        "Model.transaction { ... } expects the block as an inline function \
                             literal, e.g. Model.transaction(fn() { ... }). For manual control, \
                             call Model.transaction() with no block and use .commit()/.rollback() \
                             on the returned handle."
                            .to_string(),
                    )
                }
                None => {
                    let class_name = get_class_name_from_class(args)?;
                    Ok(Value::Class(get_or_create_transaction_class(&class_name)))
                }
                Some(other) => Err(format!(
                    "Model.transaction() expects SDBQL string or no arguments, got {}",
                    other.type_name()
                )),
            },
        )),
    );

    // Model.count() - Count documents
    use super::crud::exec_query;
    native_static_methods.insert(
        "count".to_string(),
        Rc::new(NativeFunction::new_auto_invocable(
            "Model.count",
            Some(1),
            |args| {
                let collection = get_collection_from_class(args)?;

                // Columnar models count through the columnar engine
                // (COLLECTION_COUNT only sees document collections).
                if let Ok(class_name) = get_class_name_from_class(args) {
                    if super::registry::is_columnar_model(&class_name) {
                        let schema =
                            super::registry::get_columnar_schema(&class_name).unwrap_or_default();
                        let column =
                            schema
                                .columns
                                .first()
                                .map(|c| c.name.clone())
                                .ok_or_else(|| {
                                    format!(
                                        "{}.count: columnar model has no `column` declarations",
                                        class_name
                                    )
                                })?;
                        return super::columnar::aggregate(&collection, &column, "count", None);
                    }
                }

                // Connection-aware: a model with its own `connection` may sit on a
                // different engine than the ambient default. The bare
                // `db::is_sql()` sent a Postgres-backed model's read to SoliDB
                // while `.where(...).all()` on the same model reached Postgres.
                // A `table "…"` model on a connection that cannot serve it must fail
                // here rather than fall through to the document path below.
                super::column_mode::ensure_supported(&collection)?;
                if super::crud::collection_is_sql(&collection) {
                    let class = get_class_rc_from_args(args)?;
                    let qb =
                        QueryBuilder::new_with_class(class.name.clone(), collection.clone(), class);
                    // `count` must yield a number. The query layer reports
                    // failures by RETURNING `Value::String("Error: …")`, so a
                    // missing column-mode table came back as a string that
                    // `try`/`catch` never saw — every guard of the shape
                    // `try { Model.count() } catch { … }` silently believed
                    // the table was there. Raise instead.
                    return raise_if_error_value(super::query::execute_query_builder_count(&qb));
                }

                let sti_clause = get_class_name_from_class(args)
                    .map(|n| sti_scope_clause(&n))
                    .unwrap_or_default();
                let sdbql = if sti_clause.is_empty() {
                    format!("RETURN COLLECTION_COUNT(\"{}\")", collection)
                } else {
                    format!(
                        "RETURN LENGTH(FOR doc IN {}{} RETURN 1)",
                        collection, sti_clause
                    )
                };

                // Inside a `grouped {}` block, defer this count so it
                // coalesces with the other reads into one round-trip
                // instead of firing on its own. The combined-query builder
                // strips the leading `RETURN ` so the scalar count embeds
                // as `LET _bi = (COLLECTION_COUNT(...))`.
                if super::batch::is_active() {
                    return Ok(super::batch::register(
                        sdbql,
                        std::collections::HashMap::new(),
                        Box::new(|rows| Ok(parse_count_result(&rows))),
                    ));
                }

                match exec_query(&collection, sdbql) {
                    Ok(results) => Ok(parse_count_result(&results)),
                    // `exec_with_auto_collection` auto-creates a missing
                    // collection and retries, so any error reaching us here
                    // is a real failure — surface it instead of silently
                    // returning 0 (which previously masked broken counts).
                    Err(e) => Err(format!("Model.count() failed: {}", e)),
                }
            },
        )),
    );

    // Model.with_deleted - Returns a QueryBuilder that includes soft-deleted records
    native_static_methods.insert(
        "with_deleted".to_string(),
        Rc::new(NativeFunction::new("Model.with_deleted", Some(1), |args| {
            let class = get_class_rc_from_args(args)?;
            let class_name = class.name.clone();
            let collection = class_name_to_collection(&class_name);
            let mut qb = super::query::QueryBuilder::new_with_class(class_name, collection, class);
            qb.soft_delete_mode = super::query::SoftDeleteMode::WithDeleted;
            Ok(Value::QueryBuilder(Rc::new(RefCell::new(qb))))
        })),
    );

    // Model.only_deleted - Returns a QueryBuilder with only soft-deleted records
    native_static_methods.insert(
        "only_deleted".to_string(),
        Rc::new(NativeFunction::new("Model.only_deleted", Some(1), |args| {
            let class = get_class_rc_from_args(args)?;
            let class_name = class.name.clone();
            let collection = class_name_to_collection(&class_name);
            let mut qb = super::query::QueryBuilder::new_with_class(class_name, collection, class);
            qb.soft_delete_mode = super::query::SoftDeleteMode::OnlyDeleted;
            Ok(Value::QueryBuilder(Rc::new(RefCell::new(qb))))
        })),
    );

    // Model.offset(n) - Returns a QueryBuilder with offset
    native_static_methods.insert(
        "offset".to_string(),
        Rc::new(NativeFunction::new("Model.offset", Some(2), |args| {
            let class = get_class_rc_from_args(args)?;
            let class_name = class.name.clone();
            let collection = class_name_to_collection(&class_name);
            let offset = match args.get(1) {
                Some(Value::Int(n)) if *n >= 0 => *n as usize,
                _ => return Err("offset() expects a positive integer".to_string()),
            };
            let mut qb = super::query::QueryBuilder::new_with_class(class_name, collection, class);
            qb.set_offset(offset);
            Ok(Value::QueryBuilder(Rc::new(RefCell::new(qb))))
        })),
    );

    // Model.paginate({ page: 1, per: 25 }) - Paginate results.
    // Returns { "records": [...], "pagination": { "page": n, "per": n, "total": n, "total_pages": n } }
    native_static_methods.insert(
        "paginate".to_string(),
        Rc::new(NativeFunction::new("Model.paginate", Some(2), |args| {
            let class = get_class_rc_from_args(args)?;
            let class_name = class.name.clone();
            let collection = class_name_to_collection(&class_name);

            let params = match args.get(1) {
                Some(Value::Hash(h)) => h.borrow().clone(),
                _ => return Err("paginate() expects a Hash argument".to_string()),
            };

            let page = match params.get(&crate::interpreter::value::HashKey::String("page".into()))
            {
                Some(Value::Int(n)) if *n > 0 => *n as usize,
                _ => 1,
            };
            let per = match params.get(&crate::interpreter::value::HashKey::String("per".into())) {
                // Clamped: `per` is a request parameter, and an
                // unbounded page size loads the whole collection.
                Some(Value::Int(n)) if *n > 0 => {
                    crate::interpreter::limits::clamp_page_size(*n as usize)
                }
                _ => 25,
            };

            let mut qb = super::query::QueryBuilder::new_with_class(class_name, collection, class);
            // Same in-band error convention as `Model.count()`: the query
            // layer RETURNs `Value::String("Error: …")` on failure. Treating
            // that as `0` reported `total: 0, total_pages: 1` plus an
            // error-string `records` payload — a silent success for a
            // missing table. Raise so `try`/`catch` can see it.
            let total = match raise_if_error_value(super::query::execute_query_builder_count(&qb))?
            {
                Value::Int(n) => n as usize,
                _ => 0,
            };

            let total_pages = if total == 0 { 1 } else { total.div_ceil(per) };
            let page = if page > total_pages {
                total_pages
            } else {
                page
            };
            let offset = (page - 1) * per;

            qb.set_offset(offset);
            qb.set_limit(per);
            let records = raise_if_error_value(super::query::execute_query_builder(&qb))?;

            let mut pagination = crate::interpreter::value::HashPairs::default();
            pagination.insert(
                crate::interpreter::value::HashKey::String("page".into()),
                Value::Int(page as i64),
            );
            pagination.insert(
                crate::interpreter::value::HashKey::String("per".into()),
                Value::Int(per as i64),
            );
            pagination.insert(
                crate::interpreter::value::HashKey::String("total".into()),
                Value::Int(total as i64),
            );
            pagination.insert(
                crate::interpreter::value::HashKey::String("total_pages".into()),
                Value::Int(total_pages as i64),
            );

            let mut result = crate::interpreter::value::HashPairs::default();
            result.insert(
                crate::interpreter::value::HashKey::String("records".into()),
                records,
            );
            result.insert(
                crate::interpreter::value::HashKey::String("pagination".into()),
                Value::Hash(Rc::new(RefCell::new(pagination))),
            );

            Ok(Value::Hash(Rc::new(RefCell::new(result))))
        })),
    );
}
