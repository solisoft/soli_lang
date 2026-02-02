//! Input validation system for Solilang.
//!
//! Provides schema-based validation with type coercion, required/optional fields,
//! and validation rules.

mod coercion;
mod rules;
mod validator;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use indexmap::IndexMap;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, HashKey, NativeFunction, Value};

pub use coercion::coerce_value;
pub use rules::*;
pub use validator::*;

/// Register the validation builtins in the given environment.
pub fn register_validation_builtins(env: &mut Environment) {
    // Register the V class for validator construction
    register_v_class(env);

    // Register the validate() function
    env.define(
        "validate".to_string(),
        Value::NativeFunction(NativeFunction::new("validate", Some(2), |args| {
            validate_data(&args[0], &args[1])
        })),
    );
}

/// Register the V class with static methods for creating validators.
fn register_v_class(env: &mut Environment) {
    let mut native_static_methods = HashMap::new();

    // V.string() - Create a string validator
    native_static_methods.insert(
        "string".to_string(),
        Rc::new(NativeFunction::new("V.string", Some(0), |_args| {
            Ok(create_validator(ValidatorType::String))
        })),
    );

    // V.int() - Create an integer validator
    native_static_methods.insert(
        "int".to_string(),
        Rc::new(NativeFunction::new("V.int", Some(0), |_args| {
            Ok(create_validator(ValidatorType::Int))
        })),
    );

    // V.float() - Create a float validator
    native_static_methods.insert(
        "float".to_string(),
        Rc::new(NativeFunction::new("V.float", Some(0), |_args| {
            Ok(create_validator(ValidatorType::Float))
        })),
    );

    // V.bool() - Create a boolean validator
    native_static_methods.insert(
        "bool".to_string(),
        Rc::new(NativeFunction::new("V.bool", Some(0), |_args| {
            Ok(create_validator(ValidatorType::Bool))
        })),
    );

    // V.array(schema) - Create an array validator with element schema
    native_static_methods.insert(
        "array".to_string(),
        Rc::new(NativeFunction::new("V.array", None, |args| {
            let element_schema = args.first().cloned();
            Ok(create_validator_with_schema(
                ValidatorType::Array,
                element_schema,
            ))
        })),
    );

    // V.hash(schema) - Create a hash validator with nested schema
    native_static_methods.insert(
        "hash".to_string(),
        Rc::new(NativeFunction::new("V.hash", None, |args| {
            let nested_schema = args.first().cloned();
            Ok(create_validator_with_schema(
                ValidatorType::Hash,
                nested_schema,
            ))
        })),
    );

    let v_class = Class {
        name: "V".to_string(),
        superclass: None,
        methods: HashMap::new(),
        static_methods: HashMap::new(),
        native_static_methods,
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        all_methods_cache: RefCell::new(None),
        all_native_methods_cache: RefCell::new(None),
    };

    env.define("V".to_string(), Value::Class(Rc::new(v_class)));
}

/// Create a validator value of the given type.
fn create_validator(validator_type: ValidatorType) -> Value {
    create_validator_with_schema(validator_type, None)
}

/// Create a validator value with an optional nested schema.
fn create_validator_with_schema(validator_type: ValidatorType, schema: Option<Value>) -> Value {
    let validator = Validator::new(validator_type, schema);
    validator.to_value()
}

/// Validate data against a schema.
/// Returns: { "valid": bool, "data": {...}, "errors": [...] }
fn validate_data(data: &Value, schema: &Value) -> Result<Value, String> {
    let schema_hash = match schema {
        Value::Hash(h) => h.borrow().clone(),
        _ => return Err("validate() expects schema to be a hash".to_string()),
    };

    let data_hash = match data {
        Value::Hash(h) => h.borrow().clone(),
        Value::Null => IndexMap::new(),
        _ => return Err("validate() expects data to be a hash or null".to_string()),
    };

    let mut validated_data: IndexMap<HashKey, Value> = IndexMap::new();
    let mut errors: Vec<Value> = Vec::new();

    // Build a map from data for faster lookup
    let data_map: HashMap<String, Value> = data_hash
        .iter()
        .filter_map(|(k, v)| {
            if let HashKey::String(key) = k {
                Some((key.clone(), v.clone()))
            } else {
                None
            }
        })
        .collect();

    // Validate each field in the schema
    for (field_key, validator_value) in schema_hash.iter() {
        let field_name = match field_key {
            HashKey::String(s) => s.clone(),
            _ => continue,
        };

        let field_value = data_map.get(&field_name).cloned();

        match validate_field(&field_name, field_value, validator_value) {
            Ok(Some(validated_value)) => {
                validated_data.insert(HashKey::String(field_name), validated_value);
            }
            Ok(None) => {
                // Field is optional and not present, skip it
            }
            Err(error) => {
                errors.push(error);
            }
        }
    }

    // Build result hash
    let is_valid = errors.is_empty();
    let mut result_pairs: IndexMap<HashKey, Value> = IndexMap::new();
    result_pairs.insert(HashKey::String("valid".to_string()), Value::Bool(is_valid));
    result_pairs.insert(
        HashKey::String("data".to_string()),
        Value::Hash(Rc::new(RefCell::new(validated_data))),
    );
    result_pairs.insert(
        HashKey::String("errors".to_string()),
        Value::Array(Rc::new(RefCell::new(errors))),
    );

    Ok(Value::Hash(Rc::new(RefCell::new(result_pairs))))
}

/// Validate a single field.
/// Returns Ok(Some(value)) if valid, Ok(None) if optional and missing, Err(error) if invalid.
fn validate_field(
    field_name: &str,
    value: Option<Value>,
    validator_value: &Value,
) -> Result<Option<Value>, Value> {
    let validator = Validator::from_value(validator_value)?;

    // Check if field is missing
    let value = match value {
        Some(v) if !matches!(v, Value::Null) => v,
        _ => {
            // Check for default value
            if let Some(default) = &validator.default_value {
                return Ok(Some(default.clone()));
            }
            // Check if required
            if validator.required {
                return Err(create_error(field_name, "is required", "required"));
            }
            // Optional field, not present
            return Ok(None);
        }
    };

    // Check if null is allowed
    if matches!(value, Value::Null) && validator.nullable {
        return Ok(Some(Value::Null));
    }

    // Coerce value to the expected type
    let coerced = coercion::coerce_value(&value, &validator.validator_type)?;

    // Validate against rules
    for rule in &validator.rules {
        rule.validate(field_name, &coerced)?;
    }

    // Handle nested validation for arrays and hashes
    match validator.validator_type {
        ValidatorType::Array => {
            if let Some(element_schema) = &validator.nested_schema {
                let validated_elements = validate_array_elements(&coerced, element_schema)?;
                return Ok(Some(validated_elements));
            }
        }
        ValidatorType::Hash => {
            if let Some(nested_schema) = &validator.nested_schema {
                let result = validate_data(&coerced, nested_schema)
                    .map_err(|e| create_error(field_name, &e, "validation_error"))?;
                // Check if nested validation passed
                if let Value::Hash(h) = &result {
                    for (k, v) in h.borrow().iter() {
                        if let HashKey::String(key) = k {
                            if key == "valid" {
                                if let Value::Bool(false) = v {
                                    // Extract nested errors
                                    if let Some((_, Value::Array(errors))) = h.borrow().iter().find(|(k2, _)| {
                                        matches!(k2, HashKey::String(key2) if key2 == "errors")
                                    }) {
                                        if let Some(err) = errors.borrow().first() {
                                            return Err(prefix_error(field_name, err));
                                        }
                                    }
                                }
                            }
                            if key == "data" {
                                return Ok(Some(v.clone()));
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }

    Ok(Some(coerced))
}

/// Validate array elements against a schema.
fn validate_array_elements(array: &Value, element_schema: &Value) -> Result<Value, Value> {
    let elements = match array {
        Value::Array(arr) => arr.borrow().clone(),
        _ => return Err(create_error("array", "must be an array", "type_error")),
    };

    let mut validated_elements = Vec::new();

    for (i, element) in elements.iter().enumerate() {
        let field_name = format!("[{}]", i);
        match validate_field(&field_name, Some(element.clone()), element_schema) {
            Ok(Some(v)) => validated_elements.push(v),
            Ok(None) => {}
            Err(e) => return Err(e),
        }
    }

    Ok(Value::Array(Rc::new(RefCell::new(validated_elements))))
}

/// Create an error hash.
fn create_error(field: &str, message: &str, code: &str) -> Value {
    let mut pairs: IndexMap<HashKey, Value> = IndexMap::new();
    pairs.insert(
        HashKey::String("field".to_string()),
        Value::String(field.to_string()),
    );
    pairs.insert(
        HashKey::String("message".to_string()),
        Value::String(message.to_string()),
    );
    pairs.insert(
        HashKey::String("code".to_string()),
        Value::String(code.to_string()),
    );
    Value::Hash(Rc::new(RefCell::new(pairs)))
}

/// Prefix an error with a field name for nested validation.
fn prefix_error(prefix: &str, error: &Value) -> Value {
    if let Value::Hash(h) = error {
        let mut pairs: IndexMap<HashKey, Value> = IndexMap::new();
        for (k, v) in h.borrow().iter() {
            if let HashKey::String(key) = k {
                if key == "field" {
                    if let Value::String(field) = v {
                        pairs.insert(k.clone(), Value::String(format!("{}.{}", prefix, field)));
                    } else {
                        pairs.insert(k.clone(), v.clone());
                    }
                } else {
                    pairs.insert(k.clone(), v.clone());
                }
            } else {
                pairs.insert(k.clone(), v.clone());
            }
        }
        return Value::Hash(Rc::new(RefCell::new(pairs)));
    }
    error.clone()
}
