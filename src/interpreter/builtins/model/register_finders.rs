//! `Model`'s nil-answering finders, its upserting writes, and the
//! declaration verbs that read like finders: `find_by_sql`, `find_by`,
//! `first_by`, `find_or_create_by`, `upsert`, `create_many`, `scope`,
//! `states`, `events`.
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
use super::crud::exec_update;
use crate::interpreter::value::{NativeFunction, Value};

pub(super) fn register(native_static_methods: &mut HashMap<String, Rc<NativeFunction>>) {
    // Model.find_by_sql(sql, binds?) — the escape hatch for a query the
    // portable surface cannot express.
    native_static_methods.insert(
        "find_by_sql".to_string(),
        Rc::new(NativeFunction::new("Model.find_by_sql", None, |args| {
            let class = get_class_rc_from_args(args)?;
            if !crate::db::caps().raw_sql {
                return Err(format!(
                    "Model.find_by_sql is only available on a SQL connection \
                         ({} does not speak SQL). On SoliDB use `Model.query(...)` \
                         with SDBQL instead.",
                    crate::db::adapter_label()
                ));
            }
            let sql = match args.get(1) {
                Some(Value::String(s)) => s.to_string(),
                _ => return Err("Model.find_by_sql(sql, binds?) expects a SQL string".into()),
            };
            // Binds are positional: $1/$2 on Postgres, ? elsewhere. Passing
            // values rather than interpolating them is the whole point.
            let binds = match args.get(2) {
                None | Some(Value::Null) => Vec::new(),
                Some(Value::Array(items)) => {
                    let mut out = Vec::new();
                    for item in items.borrow().iter() {
                        out.push(super::column_mode::bind_from_value(item)?);
                    }
                    out
                }
                Some(other) => {
                    return Err(format!(
                        "Model.find_by_sql binds must be an array, got {}",
                        other.type_name()
                    ))
                }
            };
            let rows = crate::db::sql::query_raw(&sql, &binds)
                .map_err(|e| format!("find_by_sql failed: {e}"))?;
            let values: Vec<Value> = rows
                .into_iter()
                .map(|row| {
                    // An object hydrates as an instance of this model; a
                    // scalar projection stays a plain value.
                    if row.is_object() {
                        super::crud::json_doc_to_instance_owned(&class, row)
                    } else {
                        super::crud::json_to_value_owned(row)
                    }
                })
                .collect();
            Ok(Value::Array(Rc::new(RefCell::new(values))))
        })),
    );

    // Model.find_by(field, value) - Find first record matching field=value
    native_static_methods.insert(
        "find_by".to_string(),
        Rc::new(NativeFunction::new("Model.find_by", Some(3), |args| {
            let class = get_class_rc_from_args(args)?;
            let class_name = class.name.clone();
            let collection = class_name_to_collection(&class_name);
            let field = match args.get(1) {
                Some(Value::String(s) | Value::Symbol(s)) => s.clone(),
                _ => return Err("find_by() expects a field name (string or symbol)".to_string()),
            };
            validate_field_name(&field, "find_by")?;
            let value = match args.get(2) {
                Some(v) => super::value_to_json(v).map_err(|e| e.to_string())?,
                None => return Err("find_by() requires a value".to_string()),
            };
            // SQL connections can't run the raw-SDBQL form below (and the
            // old `_ => nil` arm would silently swallow that error) —
            // express the lookup as a portable eq-filter query instead.
            // A `table "…"` model on a connection that cannot serve it must fail
            // here rather than fall through to the document path below.
            super::column_mode::ensure_supported(&collection)?;
            if super::crud::collection_is_sql(&collection) {
                return super::registry::run_on_collection_connection(&collection, || {
                    sql_find_first_by(&class, &collection, &field, value.clone(), false)
                });
            }
            let sdbql = format!(
                "FOR doc IN {} FILTER doc.{} == @val{} LIMIT 1 RETURN doc",
                collection,
                field,
                sti_scope_clause(&class.name)
            );
            let mut binds = std::collections::HashMap::new();
            binds.insert("val".to_string(), value);
            if super::batch::is_active() {
                let class2 = class.clone();
                return Ok(super::batch::register(
                    sdbql,
                    binds,
                    Box::new(move |rows| {
                        Ok(match rows.first() {
                            Some(doc) => super::crud::json_doc_to_instance(&class2, doc),
                            None => Value::Null,
                        })
                    }),
                ));
            }
            match super::crud::exec_with_auto_collection(sdbql, Some(binds), &collection) {
                Ok(results) if !results.is_empty() => {
                    Ok(super::crud::json_doc_to_instance(&class, &results[0]))
                }
                _ => Ok(Value::Null),
            }
        })),
    );

    // Model.first_by(field, value) - Find first record with ordering
    native_static_methods.insert(
        "first_by".to_string(),
        Rc::new(NativeFunction::new("Model.first_by", Some(3), |args| {
            let class = get_class_rc_from_args(args)?;
            let class_name = class.name.clone();
            let collection = class_name_to_collection(&class_name);
            let field = match args.get(1) {
                Some(Value::String(s) | Value::Symbol(s)) => s.clone(),
                _ => return Err("first_by() expects a field name (string or symbol)".to_string()),
            };
            validate_field_name(&field, "first_by")?;
            let value = match args.get(2) {
                Some(v) => super::value_to_json(v).map_err(|e| e.to_string())?,
                None => return Err("first_by() requires a value".to_string()),
            };
            // SQL connections: portable eq-filter query — see find_by.
            // A `table "…"` model on a connection that cannot serve it must fail
            // here rather than fall through to the document path below.
            super::column_mode::ensure_supported(&collection)?;
            if super::crud::collection_is_sql(&collection) {
                return super::registry::run_on_collection_connection(&collection, || {
                    sql_find_first_by(&class, &collection, &field, value.clone(), true)
                });
            }
            let sdbql = format!(
                "FOR doc IN {} FILTER doc.{} == @val{} SORT doc._key ASC LIMIT 1 RETURN doc",
                collection,
                field,
                sti_scope_clause(&class_name)
            );
            let mut binds = std::collections::HashMap::new();
            binds.insert("val".to_string(), value);
            if super::batch::is_active() {
                let class2 = class.clone();
                return Ok(super::batch::register(
                    sdbql,
                    binds,
                    Box::new(move |rows| {
                        Ok(match rows.first() {
                            Some(doc) => super::crud::json_doc_to_instance(&class2, doc),
                            None => Value::Null,
                        })
                    }),
                ));
            }
            match super::crud::exec_with_auto_collection(sdbql, Some(binds), &collection) {
                Ok(results) if !results.is_empty() => {
                    Ok(super::crud::json_doc_to_instance(&class, &results[0]))
                }
                _ => Ok(Value::Null),
            }
        })),
    );

    // Model.find_or_create_by(field, value, defaults) - Find or create record
    native_static_methods.insert(
        "find_or_create_by".to_string(),
        Rc::new(NativeFunction::new(
            "Model.find_or_create_by",
            Some(4),
            |args| {
                let class = get_class_rc_from_args(args)?;
                let class_name = class.name.clone();
                let collection = class_name_to_collection(&class_name);
                let field = match args.get(1) {
                    Some(Value::String(s) | Value::Symbol(s)) => s.clone(),
                    _ => {
                        return Err(
                            "find_or_create_by() expects a field name (string or symbol)"
                                .to_string(),
                        )
                    }
                };
                validate_field_name(&field, "find_or_create_by")?;
                let value = args
                    .get(2)
                    .ok_or_else(|| "find_or_create_by() requires a value".to_string())?;
                let json_val = super::value_to_json(value).map_err(|e| e.to_string())?;

                // Try to find existing
                let sdbql = format!(
                    "FOR doc IN {} FILTER doc.{} == @val{} LIMIT 1 RETURN doc",
                    collection,
                    field,
                    sti_scope_clause(&class_name)
                );
                let mut binds = std::collections::HashMap::new();
                binds.insert("val".to_string(), json_val.clone());
                match super::crud::exec_with_auto_collection(sdbql, Some(binds), &collection) {
                    Ok(results) if !results.is_empty() => {
                        return Ok(super::crud::json_doc_to_instance(&class, &results[0]));
                    }
                    _ => {}
                }

                // Not found — create with defaults. Run the same
                // strong-params filter as `Model.create`: when the
                // model declared `attr_accessible(...)`, drop any
                // non-whitelisted keys from the defaults hash before
                // they reach the insert.
                let defaults = match args.get(3) {
                    Some(hash_val @ Value::Hash(_)) => {
                        let filtered = filter_mass_assign(&class_name, hash_val);
                        let pairs = match &filtered {
                            Value::Hash(p) => p,
                            _ => unreachable!(),
                        };
                        let mut map = serde_json::Map::new();
                        for (k, v) in pairs.borrow().iter() {
                            if let crate::interpreter::value::HashKey::String(key) = k {
                                if let Ok(jv) = super::value_to_json(v) {
                                    map.insert(key.clone().to_string(), jv);
                                }
                            }
                        }
                        map
                    }
                    _ => serde_json::Map::new(),
                };
                let mut doc = defaults;
                doc.insert(field.clone().to_string(), json_val.clone());
                if super::registry::is_sti_subclass(&class_name) {
                    doc.insert(
                        "type".to_string(),
                        serde_json::Value::String(class_name.clone()),
                    );
                }
                match super::crud::exec_insert(&collection, None, serde_json::Value::Object(doc)) {
                    Ok(result) => Ok(super::crud::json_doc_to_instance(&class, &result)),
                    Err(e) => {
                        // SEC-039: another writer beat us between the
                        // initial find and this insert. If the model
                        // has a unique index on `field` the DB will
                        // tell us about the race; retry the find so
                        // find_or_create_by's contract holds (the
                        // record exists by the time we return) instead
                        // of bubbling a 409 back to the caller.
                        if super::validation::is_unique_violation(&e) {
                            let retry_sdbql = format!(
                                "FOR doc IN {} FILTER doc.{} == @val{} LIMIT 1 RETURN doc",
                                collection,
                                field,
                                sti_scope_clause(&class_name)
                            );
                            let mut retry_binds = std::collections::HashMap::new();
                            retry_binds.insert("val".to_string(), json_val);
                            if let Ok(results) = super::crud::exec_with_auto_collection(
                                retry_sdbql,
                                Some(retry_binds),
                                &collection,
                            ) {
                                if !results.is_empty() {
                                    return Ok(super::crud::json_doc_to_instance(
                                        &class,
                                        &results[0],
                                    ));
                                }
                            }
                        }
                        Err(format!("find_or_create_by create failed: {}", e))
                    }
                }
            },
        )),
    );

    // Model.upsert(key, data) - Insert or update document
    native_static_methods.insert(
        "upsert".to_string(),
        Rc::new(NativeFunction::new("Model.upsert", Some(3), |args| {
            let class = get_class_rc_from_args(args)?;
            let class_name = class.name.clone();
            let collection = class_name_to_collection(&class_name);
            if super::registry::is_timeseries_model(&class_name) {
                return Err(timeseries_insert_only_error(&class_name, "upsert"));
            }
            let key = match args.get(1) {
                Some(Value::String(s)) => s.clone(),
                _ => return Err("upsert() expects string key".to_string()),
            };
            let data = match args.get(2) {
                Some(hash_val @ Value::Hash(_)) => {
                    // Strong-params filter — `upsert` is just as exposed
                    // to mass-assignment as `create`/`update`.
                    let filtered = filter_mass_assign(&class_name, hash_val);
                    let pairs = match &filtered {
                        Value::Hash(p) => p,
                        _ => unreachable!(),
                    };
                    let mut map = serde_json::Map::new();
                    for (k, v) in pairs.borrow().iter() {
                        if let crate::interpreter::value::HashKey::String(k_str) = k {
                            if let Ok(jv) = super::value_to_json(v) {
                                map.insert(k_str.clone().to_string(), jv);
                            }
                        }
                    }
                    serde_json::Value::Object(map)
                }
                _ => return Err("upsert() expects hash data".to_string()),
            };

            // Try update first, create if not found
            match exec_update(&collection, &key, data.clone(), true) {
                Ok(result) => Ok(super::crud::json_doc_to_instance(&class, &result)),
                Err(_) => {
                    let mut insert_obj = match data {
                        serde_json::Value::Object(m) => m,
                        _ => serde_json::Map::new(),
                    };
                    // Snapshot the body sans `_key` so a retry-update
                    // (after a race) sends a normal update payload.
                    let update_payload = serde_json::Value::Object(insert_obj.clone());
                    insert_obj.insert(
                        "_key".to_string(),
                        serde_json::Value::String(key.clone().to_string()),
                    );
                    match super::crud::exec_insert(
                        &collection,
                        None,
                        serde_json::Value::Object(insert_obj),
                    ) {
                        Ok(result) => Ok(super::crud::json_doc_to_instance(&class, &result)),
                        Err(e) => {
                            // SEC-039: another writer created `key`
                            // between our failed update and our
                            // insert. Retry the update so upsert
                            // converges to the documented semantics
                            // instead of leaking a 409 the caller
                            // can't distinguish from a real conflict.
                            if super::validation::is_unique_violation(&e) {
                                if let Ok(result) =
                                    exec_update(&collection, &key, update_payload, true)
                                {
                                    return Ok(super::crud::json_doc_to_instance(&class, &result));
                                }
                            }
                            Err(format!("upsert failed: {}", e))
                        }
                    }
                }
            }
        })),
    );

    // Model.create_many(array) - Batch insert
    native_static_methods.insert(
        "create_many".to_string(),
        Rc::new(NativeFunction::new("Model.create_many", Some(2), |args| {
            let _class = get_class_rc_from_args(args)?;
            let class_name = match &args[0] {
                Value::Class(c) => c.name.clone(),
                _ => return Err("Expected class".to_string()),
            };
            let collection = class_name_to_collection(&class_name);
            let items = match args.get(1) {
                Some(Value::Array(arr)) => arr.borrow().clone(),
                _ => return Err("create_many() expects an array".to_string()),
            };

            let mut created = 0;
            // On a SQL document connection the whole batch goes in one
            // statement per chunk instead of one round trip per row. The
            // per-item strong-params filter still runs — bulk insert is
            // otherwise a perfect bypass for `attr_accessible`.
            // Connection-aware: a model with its own `connection` may be on
            // a different engine than the ambient default, and only that
            // engine's answer decides whether a bulk statement is possible.
            let bulk_sql = super::crud::collection_is_sql(&collection)
                && !super::column_mode::is_column_mode(&collection);
            let mut bulk_rows: Vec<(String, serde_json::Value)> = Vec::new();

            for item in &items {
                let doc = match item {
                    hash_val @ Value::Hash(_) => {
                        // Strong-params filter applied per-item — bulk
                        // inserts are otherwise a perfect bypass for
                        // `attr_accessible` (one trip through
                        // `create_many` writes any attribute on every
                        // document at once).
                        let filtered = filter_mass_assign(&class_name, hash_val);
                        let pairs = match &filtered {
                            Value::Hash(p) => p,
                            _ => unreachable!(),
                        };
                        let mut map = serde_json::Map::new();
                        for (k, v) in pairs.borrow().iter() {
                            if let crate::interpreter::value::HashKey::String(k_str) = k {
                                if let Ok(jv) = super::value_to_json(v) {
                                    map.insert(k_str.clone().to_string(), jv);
                                }
                            }
                        }
                        serde_json::Value::Object(map)
                    }
                    _ => continue,
                };
                if bulk_sql {
                    // The key the document already carries, else a fresh one
                    // — the same rule single-row insert applies.
                    let key = doc
                        .get("_key")
                        .and_then(|v| v.as_str())
                        .or_else(|| doc.get("id").and_then(|v| v.as_str()))
                        .map(str::to_string)
                        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                    let mut doc = doc;
                    if let Some(obj) = doc.as_object_mut() {
                        obj.insert("_key".to_string(), serde_json::json!(key));
                    }
                    // The bulk path skips `exec_insert`, so it must apply
                    // the `encrypts` transform itself — otherwise
                    // `create_many` is a silent plaintext write for every
                    // declared encrypted field, and reads still look right.
                    super::registry::encrypt_document_fields(&collection, &mut doc)?;
                    bulk_rows.push((key, doc));
                    continue;
                }
                if super::crud::exec_insert(&collection, None, doc).is_ok() {
                    created += 1;
                }
            }

            if bulk_sql && !bulk_rows.is_empty() {
                match super::registry::run_on_collection_connection(&collection, || {
                    crate::db::sql::insert_many(&collection, &bulk_rows)
                }) {
                    Ok(_) => created += bulk_rows.len() as i64,
                    // A batch failure is reported rather than silently
                    // counted as zero: the caller reads `created`.
                    Err(e) => {
                        let mut result = crate::interpreter::value::HashPairs::default();
                        result.insert(
                            crate::interpreter::value::HashKey::String("created".into()),
                            Value::Int(0),
                        );
                        result.insert(
                            crate::interpreter::value::HashKey::String("errors".into()),
                            Value::Array(Rc::new(RefCell::new(vec![Value::String(
                                format!("create_many failed: {e}").into(),
                            )]))),
                        );
                        return Ok(Value::Hash(Rc::new(RefCell::new(result))));
                    }
                }
            }

            let mut result = crate::interpreter::value::HashPairs::default();
            result.insert(
                crate::interpreter::value::HashKey::String("created".into()),
                Value::Int(created),
            );
            Ok(Value::Hash(Rc::new(RefCell::new(result))))
        })),
    );

    // Model.scope(name, query_fn) - Register a named scope on the model.
    //
    // In a class body the class is auto-prepended as args[0] (see
    // execute_class in `executor/statements.rs`), so user code reads
    // naturally as Ruby:
    //
    //   class User < Model
    //     scope("published", fn() { this.where("status = @s", { "s": "published" }) })
    //   end
    //
    // Inside the closure `this` is bound to a fresh QueryBuilder for the
    // model; the closure returns a (possibly refined) QueryBuilder.
    // Accessing `User.published`
    // invokes the closure (see scope dispatch in
    // `executor/access/member.rs`).
    native_static_methods.insert(
        "scope".to_string(),
        Rc::new(NativeFunction::new("Model.scope", Some(3), |args| {
            let class_name = get_class_name_from_class(args)?;
            let name = match args.get(1) {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Symbol(s)) => s.clone(),
                _ => {
                    return Err("scope(name, fn) expects a string or symbol scope name".to_string())
                }
            };
            let func = match args.get(2) {
                Some(Value::Function(f)) => f.clone(),
                _ => {
                    return Err("scope(name, fn) expects a function as second argument".to_string())
                }
            };
            super::scopes::register_scope(&class_name, &name, func);
            Ok(Value::Null)
        })),
    );

    // Model.states() / Model.events() — state machine reflection. Return the
    // distinct state tags / event names across all machines on the class.
    native_static_methods.insert(
        "states".to_string(),
        Rc::new(NativeFunction::new("Model.states", None, |args| {
            let class_name = get_class_name_from_class(args)?;
            let mut seen = std::collections::HashSet::new();
            let mut out = Vec::new();
            for machine in super::state_machine::machines_for(&class_name) {
                for state in machine.states {
                    if seen.insert(state.clone()) {
                        out.push(Value::String(state.into()));
                    }
                }
            }
            Ok(Value::Array(Rc::new(RefCell::new(out))))
        })),
    );
    native_static_methods.insert(
        "events".to_string(),
        Rc::new(NativeFunction::new("Model.events", None, |args| {
            let class_name = get_class_name_from_class(args)?;
            let mut seen = std::collections::HashSet::new();
            let mut out = Vec::new();
            for machine in super::state_machine::machines_for(&class_name) {
                for event in machine.events {
                    if seen.insert(event.name.clone()) {
                        out.push(Value::String(event.name.into()));
                    }
                }
            }
            Ok(Value::Array(Rc::new(RefCell::new(out))))
        })),
    );
}
