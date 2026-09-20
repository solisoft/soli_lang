//! `Model`'s query-chain starters: `includes`, `includes_count`,
//! `select`/`fields` and `join` — the calls that open a chain.
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
use super::relations::get_relation;
use crate::interpreter::value::{NativeFunction, Value};

pub(super) fn register(native_static_methods: &mut HashMap<String, Rc<NativeFunction>>) {
    // Query Chain Starters: includes, join
    // ====================================================================

    // Model.includes("posts", "profile") - eager load relations
    use super::query::QueryBuilder;
    use crate::interpreter::value::HashKey;
    native_static_methods.insert(
        "includes".to_string(),
        Rc::new(NativeFunction::new("Model.includes", None, |args| {
            let class = get_class_rc_from_args(args)?;
            let class_name = class.name.clone();
            let collection = class_name_to_collection(&class_name);

            let mut qb = QueryBuilder::new_with_class(class_name.clone(), collection, class);
            let arguments = &args[1..];

            if arguments.len() == 1 && matches!(&arguments[0], Value::Hash(_)) {
                // Pattern B: hash arg → { "posts": ["title", "body"] }
                if let Value::Hash(hash) = &arguments[0] {
                    for (k, v) in hash.borrow().iter() {
                        let rel_name = match k {
                            HashKey::String(s) => s.clone(),
                            _ => continue,
                        };
                        let rel = get_relation(&class_name, &rel_name).ok_or_else(|| {
                            format!("No relation '{}' defined on {}", rel_name, class_name)
                        })?;
                        super::relations::reject_through_on_solidb("includes", &rel)?;
                        super::relations::reject_polymorphic_relation("includes", &rel)?;
                        let fields = match v {
                            Value::Array(arr) => {
                                let names: Vec<String> = arr
                                    .borrow()
                                    .iter()
                                    .filter_map(|v| {
                                        if let Value::String(s) = v {
                                            Some(s.to_string())
                                        } else {
                                            None
                                        }
                                    })
                                    .collect();
                                if names.is_empty() {
                                    None
                                } else {
                                    Some(names)
                                }
                            }
                            _ => None,
                        };
                        qb.add_include(
                            rel_name.to_string(),
                            rel,
                            None,
                            std::collections::HashMap::new(),
                            fields,
                        );
                    }
                }
            } else if arguments.len() >= 2 && matches!(arguments.last(), Some(Value::Hash(_))) {
                // Pattern C: filtered include
                let rel_name = match &arguments[0] {
                    Value::String(s) => s.clone(),
                    other => {
                        return Err(format!(
                            "includes() expects string relation name, got {}",
                            other.type_name()
                        ))
                    }
                };
                let rel = get_relation(&class_name, &rel_name).ok_or_else(|| {
                    format!("No relation '{}' defined on {}", rel_name, class_name)
                })?;
                super::relations::reject_through_on_solidb("includes", &rel)?;
                super::relations::reject_polymorphic_relation("includes", &rel)?;

                let filter = if arguments.len() >= 3 {
                    match &arguments[1] {
                        Value::String(s) => Some(s.to_string()),
                        _ => None,
                    }
                } else {
                    None
                };

                let options_hash = match arguments.last() {
                    Some(Value::Hash(h)) => h.borrow(),
                    _ => unreachable!(),
                };

                let mut bind_vars = std::collections::HashMap::new();
                let mut fields: Option<Vec<String>> = None;
                let mut hash_filter = None;

                for (k, v) in options_hash.iter() {
                    if let HashKey::String(key) = k {
                        if **key == *"where" {
                            if let Value::Hash(inner) = v {
                                let (pred, _, binds) = parse_hash_filter(inner, "includes")?;
                                if !pred.is_empty() {
                                    hash_filter = Some(pred);
                                }
                                bind_vars.extend(binds);
                            }
                        } else if **key == *"fields" {
                            if let Value::Array(arr) = v {
                                let names: Vec<String> = arr
                                    .borrow()
                                    .iter()
                                    .filter_map(|v| {
                                        if let Value::String(s) = v {
                                            Some(s.to_string())
                                        } else {
                                            None
                                        }
                                    })
                                    .collect();
                                if !names.is_empty() {
                                    fields = Some(names);
                                }
                            }
                        } else {
                            bind_vars.insert(
                                key.to_string(),
                                crate::interpreter::value::value_to_json(v)?,
                            );
                        }
                    }
                }

                if hash_filter.is_none() && filter.is_none() && !bind_vars.is_empty() {
                    let mut map = serde_json::Map::new();
                    for (k, v) in &bind_vars {
                        map.insert(k.clone(), v.clone());
                    }
                    if let Ok(pred) =
                        crate::db::hash_filter::HashFilter::from_json_map(&map, "includes")
                    {
                        if !pred.is_empty() {
                            hash_filter = Some(pred);
                        }
                    }
                }
                qb.add_include(rel_name.to_string(), rel, filter, bind_vars, fields);
                if let Some(pred) = hash_filter {
                    if let Some(last) = qb.includes.last_mut() {
                        last.hash_filter = Some(pred);
                    }
                }
            } else {
                // Pattern A: all strings → multi-relation unfiltered
                for arg in arguments {
                    let rel_name = match arg {
                        Value::String(s) => s.clone(),
                        other => {
                            return Err(format!(
                                "includes() expects string relation names, got {}",
                                other.type_name()
                            ))
                        }
                    };
                    let rel = get_relation(&class_name, &rel_name).ok_or_else(|| {
                        format!("No relation '{}' defined on {}", rel_name, class_name)
                    })?;
                    super::relations::reject_through_on_solidb("includes", &rel)?;
                    super::relations::reject_polymorphic_relation("includes", &rel)?;
                    qb.add_include(
                        rel_name.to_string(),
                        rel,
                        None,
                        std::collections::HashMap::new(),
                        None,
                    );
                }
            }

            Ok(Value::QueryBuilder(Rc::new(RefCell::new(qb))))
        })),
    );

    // Model.includes_count("posts", "comments") — preload relation counts
    // as <name>_count fields on each parent doc. Only valid for HasMany
    // and HABTM relations.
    native_static_methods.insert(
        "includes_count".to_string(),
        Rc::new(NativeFunction::new("Model.includes_count", None, |args| {
            let class = get_class_rc_from_args(args)?;
            let class_name = class.name.clone();
            let collection = class_name_to_collection(&class_name);

            let mut qb = QueryBuilder::new_with_class(class_name.clone(), collection, class);
            let arguments = &args[1..];

            if arguments.is_empty() {
                return Err("includes_count() requires at least one relation name".to_string());
            }

            for arg in arguments {
                let rel_name = match arg {
                    Value::String(s) => s.clone(),
                    other => {
                        return Err(format!(
                            "includes_count() expects string relation names, got {}",
                            other.type_name()
                        ))
                    }
                };
                let rel = get_relation(&class_name, &rel_name).ok_or_else(|| {
                    format!("No relation '{}' defined on {}", rel_name, class_name)
                })?;
                super::relations::reject_through_on_solidb("includes", &rel)?;
                super::relations::reject_polymorphic_relation("includes", &rel)?;
                qb.add_include_count(rel_name.to_string(), rel)?;
            }

            Ok(Value::QueryBuilder(Rc::new(RefCell::new(qb))))
        })),
    );

    // Model.select("name", "email") / Model.fields("name", "email") - field selection
    let select_fn = Rc::new(NativeFunction::new("Model.select", None, |args| {
        let class = get_class_rc_from_args(args)?;
        let class_name = class.name.clone();
        let collection = class_name_to_collection(&class_name);

        let mut qb = QueryBuilder::new_with_class(class_name, collection, class);
        let mut fields = Vec::new();
        for arg in &args[1..] {
            match arg {
                Value::String(s) | Value::Symbol(s) => {
                    validate_field_name(s, "select")?;
                    fields.push(s.to_string());
                }
                other => {
                    return Err(format!(
                        "select() expects a field name (string or symbol)s, got {}",
                        other.type_name()
                    ))
                }
            }
        }
        qb.set_select(fields);
        Ok(Value::QueryBuilder(Rc::new(RefCell::new(qb))))
    }));
    native_static_methods.insert("select".to_string(), select_fn.clone());
    native_static_methods.insert("fields".to_string(), select_fn);

    // Model.join("posts") or Model.join("posts", "published = @p", { p: true })
    native_static_methods.insert(
        "join".to_string(),
        Rc::new(NativeFunction::new("Model.join", None, |args| {
            let class = get_class_rc_from_args(args)?;
            let class_name = class.name.clone();
            let collection = class_name_to_collection(&class_name);

            let rel_name = match args.get(1) {
                Some(Value::String(s)) => s.clone(),
                Some(other) => {
                    return Err(format!(
                        "join() expects string relation name, got {}",
                        other.type_name()
                    ))
                }
                None => return Err("join() requires a relation name".to_string()),
            };

            let rel = get_relation(&class_name, &rel_name)
                .ok_or_else(|| format!("No relation '{}' defined on {}", rel_name, class_name))?;
            super::relations::reject_through_on_solidb("join", &rel)?;
            super::relations::reject_polymorphic_relation("join", &rel)?;

            let (filter, bind_vars, hash_filter) =
                parse_join_filter_args(args.get(2), args.get(3))?;

            let mut qb = QueryBuilder::new_with_class(class_name, collection, class);
            qb.add_join(rel_name.to_string(), rel, filter, bind_vars);
            if let Some(pred) = hash_filter {
                if let Some(last) = qb.joins.last_mut() {
                    last.hash_filter = Some(pred);
                }
            }

            Ok(Value::QueryBuilder(Rc::new(RefCell::new(qb))))
        })),
    );

    // ====================================================================
}
