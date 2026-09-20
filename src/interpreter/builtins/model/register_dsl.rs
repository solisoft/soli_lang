//! `Model`'s declaration DSL: validations, lifecycle callbacks,
//! mass-assignment whitelisting, relations, uploaders, translated fields.
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

use std::collections::HashMap;
use std::rc::Rc;

use super::callbacks::register_callback;
use super::core::*;
use super::relations::{
    build_habtm_relation, build_relation, parse_relation_options, register_relation, RelationType,
};
use super::uploaders::register_uploader;
use super::validation::{parse_validates_options, register_validation_with_conditions};
use crate::interpreter::value::{NativeFunction, Value};

pub(super) fn register(native_static_methods: &mut HashMap<String, Rc<NativeFunction>>) {
    // Validation & Callback Registration Methods
    // ====================================================================

    // validates(field, options) - Register validation rules
    native_static_methods.insert(
        "validates".to_string(),
        Rc::new(NativeFunction::new("Model.validates", Some(3), |args| {
            let class_name = get_class_name_from_class(args)?;

            let field = arg_str_or_sym(args, 1, "validates()", "field name")?;

            let options = match args.get(2) {
                Some(Value::Hash(hash)) => hash.borrow().clone(),
                Some(other) => {
                    return Err(format!(
                        "validates() expects hash options, got {}",
                        other.type_name()
                    ))
                }
                None => return Err("validates() requires options argument".to_string()),
            };

            let (rule, conditions) = parse_validates_options(&field, &options)?;
            register_validation_with_conditions(&class_name, rule, conditions);
            Ok(Value::Null)
        })),
    );

    // Callback registration methods
    for callback_type in &[
        "before_save",
        "after_save",
        "before_create",
        "after_create",
        "before_update",
        "after_update",
        "before_delete",
        "after_delete",
    ] {
        let callback_name = callback_type.to_string();
        let method_name = format!("Model.{}", callback_type);
        native_static_methods.insert(
            callback_name.clone(),
            Rc::new(NativeFunction::new(&method_name, Some(2), move |args| {
                let class_name = get_class_name_from_class(args)?;
                let method_name =
                    arg_str_or_sym(args, 1, &format!("{}()", callback_name), "method name")?;
                register_callback(&class_name, &callback_name, &method_name);
                Ok(Value::Null)
            })),
        );
    }

    // attr_accessible(field1, field2, ...) or attr_accessible([field1, field2, ...])
    // Declares the whitelist of attributes that may be assigned via
    // mass-assignment paths (`Model.create(hash)`,
    // `Model.update(id, hash)`, `instance.update(hash)`,
    // `instance.save(hash)`). Any key not in the list is silently
    // dropped before validation, before instance population, and
    // before the DB write. Without a declaration the model accepts
    // every key (legacy behaviour) — see docs/models.md.
    native_static_methods.insert(
        "attr_accessible".to_string(),
        Rc::new(NativeFunction::new("Model.attr_accessible", None, |args| {
            let class_name = get_class_name_from_class(args)?;
            let fields = collect_accessible_fields(&args[1..])?;
            register_accessible_attributes(&class_name, fields);
            Ok(Value::Null)
        })),
    );

    // ====================================================================
    // Relation DSL Methods
    // ====================================================================

    // has_many(name) or has_many(name, options)
    for (rel_method, rel_type) in &[
        ("has_many", RelationType::HasMany),
        ("has_one", RelationType::HasOne),
        ("belongs_to", RelationType::BelongsTo),
    ] {
        let method_label = format!("Model.{}", rel_method);
        let rel_type = rel_type.clone();
        native_static_methods.insert(
            rel_method.to_string(),
            Rc::new(NativeFunction::new(&method_label, None, move |args| {
                let class_name = get_class_name_from_class(args)?;
                let name = arg_str_or_sym(args, 1, "relation", "name")?;

                // Optional config hash: class_name/foreign_key overrides
                // plus dependent:/through:/source:/counter_cache: (validated
                // per relation kind — a bad option raises at class load).
                let options = parse_relation_options(args.get(2), &rel_type)?;

                let relation = build_relation(&class_name, &name, rel_type.clone(), &options);
                register_relation(&class_name, relation);
                Ok(Value::Null)
            })),
        );
    }

    // has_and_belongs_to_many(name) or with options:
    //   { class_name, foreign_key, association_foreign_key, join_table }
    native_static_methods.insert(
        "has_and_belongs_to_many".to_string(),
        Rc::new(NativeFunction::new(
            "Model.has_and_belongs_to_many",
            None,
            |args| {
                let class_name = get_class_name_from_class(args)?;
                let name = match args.get(1) {
                    Some(Value::String(s)) => s.clone(),
                    Some(other) => {
                        return Err(format!(
                            "has_and_belongs_to_many expects string name, got {}",
                            other.type_name()
                        ))
                    }
                    None => {
                        return Err("has_and_belongs_to_many requires a name argument".to_string())
                    }
                };

                let options =
                    parse_relation_options(args.get(2), &RelationType::HasAndBelongsToMany)?;

                let relation = build_habtm_relation(&class_name, &name, &options);
                register_relation(&class_name, relation);
                Ok(Value::Null)
            },
        )),
    );

    // ====================================================================
    // Uploader DSL: uploader("photo", { multiple, content_types, ... })
    // ====================================================================

    native_static_methods.insert(
        "uploader".to_string(),
        Rc::new(NativeFunction::new("Model.uploader", Some(3), |args| {
            let class_name = get_class_name_from_class(args)?;
            let config = build_uploader_config_from_args(&class_name, args)?;
            register_uploader(&class_name, config);
            Ok(Value::Null)
        })),
    );

    native_static_methods.insert(
        "has_one_attached".to_string(),
        Rc::new(NativeFunction::new(
            "Model.has_one_attached",
            None,
            |args| {
                let class_name = get_class_name_from_class(args)?;
                let config = build_attached_config_from_args(&class_name, args, false)?;
                register_uploader(&class_name, config);
                Ok(Value::Null)
            },
        )),
    );
    native_static_methods.insert(
        "has_many_attached".to_string(),
        Rc::new(NativeFunction::new(
            "Model.has_many_attached",
            None,
            |args| {
                let class_name = get_class_name_from_class(args)?;
                let config = build_attached_config_from_args(&class_name, args, true)?;
                register_uploader(&class_name, config);
                Ok(Value::Null)
            },
        )),
    );

    // ====================================================================
    // Translation DSL: translate("field1", "field2", ...)
    // ====================================================================

    // Model.translate("title", "description") - declare translatable fields
    native_static_methods.insert(
        "translate".to_string(),
        Rc::new(NativeFunction::new("Model.translate", None, |args| {
            let class_name = get_class_name_from_class(args)?;

            // Accept one or more field names as arguments
            let field_names: Vec<String> = args[1..]
                .iter()
                .filter_map(|arg| {
                    if let Value::String(s) = arg {
                        Some(s.to_string())
                    } else {
                        None
                    }
                })
                .collect();

            if field_names.is_empty() {
                return Err("translate() requires at least one field name".to_string());
            }

            for field_name in &field_names {
                register_translation(&class_name, field_name);
            }
            Ok(Value::Null)
        })),
    );

    // ====================================================================
}
