//! `Model`'s core reads and its first write: `create`, `find`, `where`,
//! `live_where`, `broadcast`, the query mocks, `all` / `all_json`, and the
//! chain modifiers `order`, `timeout` and `limit`.
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
use super::query::QueryBuilder;
use crate::interpreter::value::HashKey;
use crate::interpreter::value::{NativeFunction, Value};

pub(super) fn register(native_static_methods: &mut HashMap<String, Rc<NativeFunction>>) {
    // CRUD Methods
    // ====================================================================

    // Model.create(data) - Insert document with validation. Always returns
    // an instance of the class. On success, `_errors` is unset (reads as
    // null). On validation or DB failure, the instance is NOT persisted
    // and `_errors` is populated as an Array — of {field, message} hashes
    // for validation errors, or of String messages for DB errors.
    use super::crud::{exec_insert, json_to_value};
    use super::validation::run_validations;
    use crate::interpreter::value::value_to_json;
    native_static_methods.insert(
        "create".to_string(),
        Rc::new(NativeFunction::new("Model.create", Some(2), |args| {
            let class = get_class_rc_from_args(args)?;
            let class_name = class.name.clone();
            let collection = class_name_to_collection(&class_name);

            let raw_data = args
                .get(1)
                .cloned()
                .ok_or_else(|| "Model.create() requires data argument".to_string())?;

            // Strong-params filter: when the model declared
            // `attr_accessible(...)`, drop any non-whitelisted keys
            // before they reach validation, the in-memory instance,
            // or the DB write. Models without a declaration get the
            // raw hash through unchanged (back-compat).
            let data = filter_mass_assign(&class_name, &raw_data);

            // Edge models: coerce from:/to: into _from/_to document ids.
            // Endpoints are read from the raw hash so attr_accessible
            // can't strip them; missing/invalid endpoints surface as
            // _errors like validation failures.
            let mut edge_refs: Option<(String, String)> = None;
            let data = match super::registry::get_edge_spec(&class_name) {
                Some(edge_spec) => match super::graph::transform_edge_data(&raw_data, &edge_spec) {
                    Ok((from_ref, to_ref)) => {
                        edge_refs = Some((from_ref.clone(), to_ref.clone()));
                        super::graph::rebuild_edge_data(&data, &from_ref, &to_ref)
                    }
                    Err(endpoint_errors) => {
                        let instance = Rc::new(RefCell::new(
                            crate::interpreter::value::Instance::new(class.clone()),
                        ));
                        apply_hash_to_instance(&instance, &data)?;
                        let error_values: Vec<Value> = endpoint_errors
                            .into_iter()
                            .map(|(field, message)| {
                                let mut pairs = crate::interpreter::value::HashPairs::default();
                                pairs.insert(
                                    HashKey::String("field".into()),
                                    Value::String(field.into()),
                                );
                                pairs.insert(
                                    HashKey::String("message".into()),
                                    Value::String(message.into()),
                                );
                                Value::Hash(Rc::new(RefCell::new(pairs)))
                            })
                            .collect();
                        instance
                            .borrow_mut()
                            .set("_errors", Value::Array(Rc::new(RefCell::new(error_values))));
                        return Ok(Value::Instance(instance));
                    }
                },
                None => data,
            };

            // Build a base instance from the (filtered) attributes so
            // the returned object carries the data we'd actually
            // persist, even on failure.
            let instance = Rc::new(RefCell::new(crate::interpreter::value::Instance::new(
                class.clone(),
            )));
            apply_hash_to_instance(&instance, &data)?;
            // apply_hash_to_instance skips `_`-prefixed keys; the edge
            // payload is the exception the caller should see.
            if let Some((ref from_ref, ref to_ref)) = edge_refs {
                let mut inst_mut = instance.borrow_mut();
                inst_mut.set("_from", Value::String(from_ref.clone().into()));
                inst_mut.set("_to", Value::String(to_ref.clone().into()));
            }

            // Run validations against the filtered input — non-permitted
            // fields are gone, so callers can't satisfy a validation
            // (or trigger one) by smuggling fields the model never
            // intended to accept.
            let errors = run_validations(&class_name, &data, None)?;
            if !errors.is_empty() {
                let error_values: Vec<Value> = errors.iter().map(|e| e.to_value()).collect();
                instance
                    .borrow_mut()
                    .set("_errors", Value::Array(Rc::new(RefCell::new(error_values))));
                return Ok(Value::Instance(instance));
            }

            let data_value: Result<serde_json::Value, String> = match &data {
                Value::Hash(hash) => {
                    let mut map = serde_json::Map::new();
                    for (k, v) in hash.borrow().iter() {
                        if let HashKey::String(key) = k {
                            map.insert(key.clone().to_string(), value_to_json(v)?);
                        }
                    }
                    Ok(serde_json::Value::Object(map))
                }
                other => Err(format!(
                    "Model.create() expects hash data, got {}",
                    other.type_name()
                )),
            };
            let mut data_value = data_value?;
            strip_reserved_document_keys(&mut data_value, &class_name);
            // Edge endpoints go back in after the strip. They are not mass
            // assignment: `transform_edge_data` resolved and validated them
            // against the edge spec's declared collections just above, and
            // an edge document without them is not an edge.
            if let Some((ref from_ref, ref to_ref)) = edge_refs {
                if let serde_json::Value::Object(ref mut map) = data_value {
                    map.insert(
                        "_from".to_string(),
                        serde_json::Value::String(from_ref.clone()),
                    );
                    map.insert("_to".to_string(), serde_json::Value::String(to_ref.clone()));
                }
            }

            // STI subclasses stamp their discriminator so rows in the
            // shared base collection hydrate as the right class.
            if super::registry::is_sti_subclass(&class_name) {
                if let serde_json::Value::Object(ref mut map) = data_value {
                    map.insert(
                        "type".to_string(),
                        serde_json::Value::String(class_name.clone()),
                    );
                }
                instance
                    .borrow_mut()
                    .set("type", Value::String(class_name.as_str().into()));
            }

            match exec_insert(&collection, None, data_value) {
                Ok(id) => {
                    let mut inst_mut = instance.borrow_mut();
                    if let serde_json::Value::Object(ref id_map) = id {
                        for field in &["_key", "_id", "_rev", "_created_at", "_updated_at"] {
                            if let Some(val) = id_map.get(*field) {
                                inst_mut.set(field.to_string(), json_to_value(val));
                            }
                        }
                    }
                    // A column-aware insert answers with the stored row, so
                    // adopt it: column defaults and the stamped
                    // created_at/updated_at are otherwise invisible until
                    // the record is read back.
                    let column_schema = super::column_mode::schema_for_collection(&collection);
                    if column_schema.is_some() {
                        super::column_mode::adopt_row_fields(&mut inst_mut, &id);
                    }
                    // `id` is the row's identifier, not the whole response.
                    // The response shape differs per backend — SoliDB
                    // returns `{_key, _id, _rev}`, while a column-aware
                    // insert returns every column of the stored row — so
                    // take the row's own `id` when it has one, else its
                    // `_key`. Assigning the response object wholesale made
                    // `instance.id` a hash (and, in column mode, clobbered
                    // the real `id` column that hydration had just set).
                    let id_value = match &id {
                        serde_json::Value::Object(map) => map
                            .get("id")
                            .or_else(|| map.get("_key"))
                            .map(json_to_value)
                            .unwrap_or_else(|| json_to_value(&id)),
                        other => json_to_value(other),
                    };
                    inst_mut.set("id", id_value);
                    super::dirty::finalize_persist(&mut inst_mut);
                    super::counter_cache::bump_for_instance(&inst_mut, 1);
                    drop(inst_mut);
                    // Same conversion the read path applies, so a timestamp
                    // is a DateTime whether the record was created or found.
                    if let Some(schema) = column_schema {
                        super::column_mode::convert_temporal_fields(
                            &schema,
                            Value::Instance(instance.clone()),
                        );
                    }
                    Ok(Value::Instance(instance))
                }
                Err(e) => {
                    let error_values = build_persistence_errors(&class_name, e);
                    instance
                        .borrow_mut()
                        .set("_errors", Value::Array(Rc::new(RefCell::new(error_values))));
                    Ok(Value::Instance(instance))
                }
            }
        })),
    );

    // Model.find(id) - Get by ID, returns a class instance
    use super::crud::{exec_get, json_doc_to_instance};
    native_static_methods.insert(
        "find".to_string(),
        Rc::new(NativeFunction::new("Model.find", Some(2), |args| {
            let class = get_class_rc_from_args(args)?;
            let collection = class_name_to_collection(&class.name);

            let id = match args.get(1) {
                Some(Value::String(s)) => s.clone(),
                // A column-aware model on a serial/identity key is looked
                // up with a number (`Order.find(42)`), which is also what
                // `instance.id` hands back. Keys travel as strings
                // internally; the column layer converts back to the
                // primary key's real type.
                Some(Value::Int(n)) => n.to_string().into(),
                Some(other) => {
                    return Err(format!(
                        "Model.find() expects a string or integer id, got {}",
                        other.type_name()
                    ))
                }
                None => return Err("Model.find() requires id argument".to_string()),
            };

            // Inside a `grouped {}` block: rewrite the key lookup to a
            // cursor query so it can be coalesced with the others. The
            // transform preserves the RecordNotFound-on-miss contract, so
            // the error still surfaces (as a 404) when the deferred is read.
            // SQL-bound collections skip registration — the coalesced flush
            // is raw SDBQL, which SQL adapters reject; exec_get below
            // routes them correctly.
            if super::batch::is_active() && !super::crud::collection_is_sql(&collection) {
                let sdbql = format!(
                    "FOR doc IN {} FILTER doc._key == @k LIMIT 1 RETURN doc",
                    collection
                );
                let mut binds = std::collections::HashMap::new();
                binds.insert("k".to_string(), serde_json::Value::String(id.to_string()));
                let class2 = class.clone();
                let class_name = class.name.clone();
                let id2 = id.clone();
                return Ok(super::batch::register(
                    sdbql,
                    binds,
                    Box::new(move |rows| match rows.first() {
                        Some(doc) if sti_row_matches(&class_name, doc) => {
                            Ok(json_doc_to_instance(&class2, doc))
                        }
                        _ => Err(format!(
                            "{}{} with id '{}' not found",
                            crate::error::RuntimeError::RECORD_NOT_FOUND_MARKER,
                            class_name,
                            id2
                        )),
                    }),
                ));
            }

            match exec_get(&collection, &id) {
                // STI: a subclass find only matches rows of its own
                // hierarchy — a base-class row raises RecordNotFound
                // exactly like a missing key (Rails semantics).
                Ok(doc) if sti_row_matches(&class.name, &doc) => {
                    Ok(json_doc_to_instance(&class, &doc))
                }
                // Not found → raise with the RecordNotFound marker so the
                // HTTP request handler converts it into a 404 response.
                // Callers that want the "or null" shape should use
                // find_by / first_by, or wrap in try/catch.
                _ => Err(format!(
                    "{}{} with id '{}' not found",
                    crate::error::RuntimeError::RECORD_NOT_FOUND_MARKER,
                    class.name,
                    id
                )),
            }
        })),
    );

    // Model.where(...) - Returns a QueryBuilder for chaining.
    //
    // Two forms:
    //   1. Hash form (safe — recommended for user input):
    //        Model.where({"email": "alice@x", "active": true})
    //      Each key is validated as an AQL identifier; values flow
    //      through bind parameters, so attacker-controlled data
    //      cannot reach the query template.
    //
    //   2. String form (developer-trusted — see docs/models.md for
    //      the security note):
    //        Model.where("doc.age >= @age", {"age": 18})
    //      Filter is concatenated verbatim into the AQL FILTER
    //      clause, so the *string itself* must never come from
    //      untrusted input.
    use std::collections::HashMap as StdHashMap;
    native_static_methods.insert(
        "where".to_string(),
        Rc::new(NativeFunction::new("Model.where", Some(3), |args| {
            let class = get_class_rc_from_args(args)?;
            let class_name = class.name.clone();
            let collection = class_name_to_collection(&class_name);

            let (filter, bind_vars): (String, StdHashMap<String, serde_json::Value>) =
                match args.get(1) {
                    Some(Value::Hash(hash)) => {
                        // Safe hash form. A second argument (bind_vars
                        // for the string form) is meaningless here and
                        // is rejected up-front so callers don't think
                        // they can mix forms.
                        if args.get(2).is_some() {
                            return Err("Model.where(Hash) takes a single argument; \
                                    the bind-vars hash is only valid with the string filter form"
                                .to_string());
                        }
                        let (pred, filter, binds) = parse_hash_filter(hash, "where")?;
                        let mut qb = QueryBuilder::new_with_class(
                            class_name.clone(),
                            collection.clone(),
                            class.clone(),
                        );
                        qb.hash_filter = if pred.is_empty() { None } else { Some(pred) };
                        qb.set_filter(filter, binds);
                        return Ok(Value::QueryBuilder(Rc::new(RefCell::new(qb))));
                    }
                    Some(Value::String(s)) => {
                        let filter = s.clone();
                        let binds = match args.get(2) {
                            Some(Value::Hash(hash)) => {
                                let mut map = StdHashMap::new();
                                for (k, v) in hash.borrow().iter() {
                                    if let HashKey::String(key) = k {
                                        map.insert(
                                            key.to_string(),
                                            ensure_string_form_bind_value(v, key, "where")?,
                                        );
                                    }
                                }
                                map
                            }
                            Some(other) => {
                                return Err(format!(
                                    "Model.where() expects hash for bind variables, got {}",
                                    other.type_name()
                                ))
                            }
                            None => StdHashMap::new(),
                        };
                        (filter.to_string(), binds)
                    }
                    Some(other) => {
                        return Err(format!(
                            "Model.where() expects a Hash filter or a string filter expression, \
                             got {}",
                            other.type_name()
                        ))
                    }
                    None => return Err("Model.where() requires a filter argument".to_string()),
                };

            let mut qb = QueryBuilder::new_with_class(class_name, collection, class);
            // Only the string form reaches here (the hash form returned
            // above), so this filter is raw SDBQL rather than a hash echo.
            qb.has_raw_where = true;
            qb.set_filter(filter, bind_vars);

            Ok(Value::QueryBuilder(Rc::new(RefCell::new(qb))))
        })),
    );

    // Model.live_where(filter) — reactive live query. Behaves exactly like
    // `where(filter).all()` (runs the query, returns the instances array),
    // but ALSO subscribes the currently-rendering LiveView to the queried
    // collection, so a later write to it re-renders the view. Outside a
    // LiveView render the subscribe is a no-op, so `live_where` is a safe
    // drop-in for `.all()` anywhere. See `crate::live::live_query`.
    native_static_methods.insert(
            "live_where".to_string(),
            Rc::new(NativeFunction::new("Model.live_where", Some(3), |args| {
                let class = get_class_rc_from_args(args)?;
                let class_name = class.name.clone();
                let collection = class_name_to_collection(&class_name);

                // For the hash form the binds map IS the flat field==value
                // matcher, so we can wake only rows that satisfy it. The string
                // form is opaque SDBQL -> `None` (wake conservatively).
                let (filter, bind_vars, matcher): (
                    String,
                    StdHashMap<String, serde_json::Value>,
                    crate::live::live_query::Matcher,
                ) = match args.get(1) {
                        Some(Value::Hash(hash)) => {
                            if args.get(2).is_some() {
                                return Err("Model.live_where(Hash) takes a single argument; \
                                    the bind-vars hash is only valid with the string filter form"
                                    .to_string());
                            }
                            let (filter, binds) = build_safe_filter_from_hash(hash, "live_where")?;
                            let matcher = Some(binds.clone());
                            (filter, binds, matcher)
                        }
                        Some(Value::String(s)) => {
                            let filter = s.clone();
                            let binds = match args.get(2) {
                                Some(Value::Hash(hash)) => {
                                    let mut map = StdHashMap::new();
                                    for (k, v) in hash.borrow().iter() {
                                        if let HashKey::String(key) = k {
                                            map.insert(
                                                key.to_string(),
                                                ensure_string_form_bind_value(v, key, "live_where")?,
                                            );
                                        }
                                    }
                                    map
                                }
                                Some(other) => {
                                    return Err(format!(
                                        "Model.live_where() expects hash for bind variables, got {}",
                                        other.type_name()
                                    ))
                                }
                                None => StdHashMap::new(),
                            };
                            (filter.to_string(), binds, None)
                        }
                        Some(other) => {
                            return Err(format!(
                                "Model.live_where() expects a Hash filter or a string filter expression, got {}",
                                other.type_name()
                            ))
                        }
                        None => {
                            return Err("Model.live_where() requires a filter argument".to_string())
                        }
                    };

                // Record the subscription before `collection` is moved into the
                // builder (no-op when not inside a LiveView render).
                crate::live::live_query::subscribe(&collection, matcher);

                let mut qb = QueryBuilder::new_with_class(class_name, collection, class);
                qb.set_filter(filter, bind_vars);

                // One-shot: run the query and return the instances, like `.all()`.
                Ok(super::query::execute_query_builder(&qb))
            })),
        );

    // Model.broadcast(payload) — publish `payload` to the model's collection
    // channel over WebSocket AND SSE, so any subscribed client (raw WS, SSE,
    // htmx) gets model-change events. A convenience over the top-level
    // `broadcast(channel, payload)` with channel = the collection name.
    // Returns the SSE subscriber count. Non-string payloads JSON-encode.
    native_static_methods.insert(
        "broadcast".to_string(),
        Rc::new(NativeFunction::new("Model.broadcast", Some(2), |args| {
            let class = get_class_rc_from_args(args)?;
            let channel = class_name_to_collection(&class.name);
            let payload = args
                .get(1)
                .ok_or_else(|| "Model.broadcast() requires a payload argument".to_string())?;
            let message = crate::interpreter::builtins::server::broadcast_payload_to_string(
                payload,
                "Model.broadcast",
            )?;
            let delivered =
                crate::interpreter::builtins::server::broadcast_message(&channel, &message);
            Ok(Value::Int(delivered as i64))
        })),
    );

    // Model.mock_query_result(query, results_array) - Register mock DB response for testing
    use super::crud::register_query_mock;
    native_static_methods.insert(
        "mock_query_result".to_string(),
        Rc::new(NativeFunction::new(
            "Model.mock_query_result",
            Some(2),
            |args| {
                let query = match args.get(1) {
                    Some(Value::String(s)) => s.clone(),
                    _ => {
                        return Err(
                            "mock_query_result expects query string as second argument".to_string()
                        )
                    }
                };
                let results_array = match args.get(2) {
                    Some(Value::Array(arr)) => arr
                        .borrow()
                        .iter()
                        .map(|v| value_to_json(v).map_err(|e| format!("Invalid JSON: {}", e)))
                        .collect::<Result<Vec<_>, _>>()?,
                    _ => {
                        return Err(
                            "mock_query_result expects results array as third argument".to_string()
                        )
                    }
                };
                register_query_mock(query.to_string(), results_array);
                Ok(Value::Null)
            },
        )),
    );

    // Model.clear_mocks() - Clear all registered mock responses
    use super::crud::clear_query_mocks;
    native_static_methods.insert(
        "clear_mocks".to_string(),
        Rc::new(NativeFunction::new("Model.clear_mocks", None, |_| {
            clear_query_mocks();
            Ok(Value::Null)
        })),
    );

    // Model.all() - Get all documents as class instances
    use super::crud::exec_auto_collection_as_instances;
    native_static_methods.insert(
        "all".to_string(),
        Rc::new(NativeFunction::new_auto_invocable(
            "Model.all",
            Some(1),
            |args| {
                let class = get_class_rc_from_args(args)?;
                let collection = class_name_to_collection(&class.name);
                // SQL document adapters have no raw SDBQL — route through
                // the QueryBuilder so hash filters / soft-delete / STI
                // scopes compile to portable SQL.
                // Connection-aware: a model with its own `connection` may sit on a
                // different engine than the ambient default. The bare
                // `db::is_sql()` sent a Postgres-backed model's read to SoliDB
                // while `.where(...).all()` on the same model reached Postgres.
                // A `table "…"` model on a connection that cannot serve it must fail
                // here rather than fall through to the document path below.
                super::column_mode::ensure_supported(&collection)?;
                if super::crud::collection_is_sql(&collection) {
                    let qb =
                        QueryBuilder::new_with_class(class.name.clone(), collection, class.clone());
                    return Ok(super::query::execute_query_builder(&qb));
                }
                let sdbql = format!(
                    "FOR doc IN {}{} RETURN doc",
                    collection,
                    sti_scope_clause(&class.name)
                );
                if super::batch::is_active() {
                    let class2 = class.clone();
                    return Ok(super::batch::register(
                        sdbql,
                        std::collections::HashMap::new(),
                        Box::new(move |rows| {
                            let values: Vec<Value> = rows
                                .iter()
                                .map(|j| super::crud::json_doc_to_instance(&class2, j))
                                .collect();
                            Ok(Value::Array(Rc::new(RefCell::new(values))))
                        }),
                    ));
                }
                Ok(exec_auto_collection_as_instances(
                    sdbql,
                    &collection,
                    &class,
                ))
            },
        )),
    );

    // Model.all_json() - Get all documents as raw JSON string (fastest)
    use super::crud::exec_async_query_raw;
    native_static_methods.insert(
        "all_json".to_string(),
        Rc::new(NativeFunction::new_auto_invocable(
            "Model.all_json",
            Some(1),
            |args| {
                let class = get_class_rc_from_args(args)?;
                let collection = class_name_to_collection(&class.name);
                // Connection-aware: a model with its own `connection` may sit on a
                // different engine than the ambient default. The bare
                // `db::is_sql()` sent a Postgres-backed model's read to SoliDB
                // while `.where(...).all()` on the same model reached Postgres.
                // A `table "…"` model on a connection that cannot serve it must fail
                // here rather than fall through to the document path below.
                super::column_mode::ensure_supported(&collection)?;
                if super::crud::collection_is_sql(&collection) {
                    let qb =
                        QueryBuilder::new_with_class(class.name.clone(), collection, class.clone());
                    let rows = super::query::execute_query_builder(&qb);
                    // Mirror SoliDB all_json: return a JSON array string.
                    return Ok(match rows {
                        Value::Array(arr) => {
                            let json_rows: Vec<serde_json::Value> = arr
                                .borrow()
                                .iter()
                                .map(|v| {
                                    crate::interpreter::value::value_to_json(v)
                                        .unwrap_or(serde_json::Value::Null)
                                })
                                .collect();
                            Value::String(
                                serde_json::to_string(&json_rows)
                                    .unwrap_or_else(|_| "[]".into())
                                    .into(),
                            )
                        }
                        other => Value::String(format!("{}", other).into()),
                    });
                }
                let sdbql = format!(
                    "FOR doc IN {}{} RETURN doc",
                    collection,
                    sti_scope_clause(&class.name)
                );
                Ok(exec_async_query_raw(sdbql))
            },
        )),
    );

    // Model.order(field, direction?) - Returns a QueryBuilder with ordering (no filter)
    native_static_methods.insert(
        "order".to_string(),
        Rc::new(NativeFunction::new("Model.order", Some(3), |args| {
            let class = get_class_rc_from_args(args)?;
            let class_name = class.name.clone();
            let collection = class_name_to_collection(&class_name);

            let field = match args.get(1) {
                Some(Value::String(s) | Value::Symbol(s)) => s.clone(),
                Some(other) => {
                    return Err(format!(
                        "Model.order() expects a field name (string or symbol), got {}",
                        other.type_name()
                    ))
                }
                None => return Err("Model.order() requires a field name".to_string()),
            };
            validate_field_name(&field, "order")?;

            let direction = match args.get(2) {
                Some(Value::String(s) | Value::Symbol(s)) => s.clone(),
                _ => "asc".into(),
            };
            validate_order_direction(&direction, "order")?;

            let mut qb = QueryBuilder::new_with_class(class_name, collection, class);
            qb.set_order(field.to_string(), direction.to_string());

            Ok(Value::QueryBuilder(Rc::new(RefCell::new(qb))))
        })),
    );

    // Model.timeout(secs) - Returns a QueryBuilder whose one request may
    // run for `secs` instead of the internal DB client's 10s default.
    native_static_methods.insert(
        "timeout".to_string(),
        Rc::new(NativeFunction::new("Model.timeout", Some(2), |args| {
            let class = get_class_rc_from_args(args)?;
            let class_name = class.name.clone();
            let collection = class_name_to_collection(&class_name);

            let secs = match args.get(1) {
                Some(Value::Int(n)) => *n as f64,
                Some(Value::Float(f)) => *f,
                Some(other) => {
                    return Err(format!(
                        "Model.timeout() expects a number of seconds, got {}",
                        other.type_name()
                    ))
                }
                None => return Err("Model.timeout() requires a number".to_string()),
            };
            if !secs.is_finite() || secs <= 0.0 {
                return Err(format!(
                    "Model.timeout() expects a positive number of seconds, got {}",
                    secs
                ));
            }

            let mut qb = QueryBuilder::new_with_class(class_name, collection, class);
            qb.set_timeout(secs);
            Ok(Value::QueryBuilder(Rc::new(RefCell::new(qb))))
        })),
    );

    // Model.limit(n) - Returns a QueryBuilder with a limit (no filter)
    native_static_methods.insert(
        "limit".to_string(),
        Rc::new(NativeFunction::new("Model.limit", Some(2), |args| {
            let class = get_class_rc_from_args(args)?;
            let class_name = class.name.clone();
            let collection = class_name_to_collection(&class_name);

            let limit = match args.get(1) {
                // `*n as usize` turned a negative into `usize::MAX`, i.e.
                // `LIMIT 18446744073709551615` — a full-collection scan from
                // `?limit=-1`.
                Some(Value::Int(n)) if *n < 0 => {
                    return Err(format!(
                        "Model.limit() expects a non-negative integer, got {n}"
                    ))
                }
                Some(Value::Float(f)) if *f < 0.0 => {
                    return Err(format!(
                        "Model.limit() expects a non-negative number, got {f}"
                    ))
                }
                Some(Value::Int(n)) => *n as usize,
                Some(Value::Float(f)) => *f as usize,
                Some(other) => {
                    return Err(format!(
                        "Model.limit() expects integer, got {}",
                        other.type_name()
                    ))
                }
                None => return Err("Model.limit() requires a number".to_string()),
            };

            let mut qb = QueryBuilder::new_with_class(class_name, collection, class);
            qb.set_limit(limit);

            Ok(Value::QueryBuilder(Rc::new(RefCell::new(qb))))
        })),
    );
}
