//! Validator struct and chainable methods.

use std::cell::RefCell;
use std::rc::Rc;

use crate::interpreter::value::{NativeFunction, Value};

use super::rules::ValidationRule;
use super::create_error;

/// The type of value a validator expects.
#[derive(Clone, Debug, PartialEq)]
pub enum ValidatorType {
    String,
    Int,
    Float,
    Bool,
    Array,
    Hash,
}

impl ValidatorType {
    pub fn name(&self) -> &'static str {
        match self {
            ValidatorType::String => "string",
            ValidatorType::Int => "int",
            ValidatorType::Float => "float",
            ValidatorType::Bool => "bool",
            ValidatorType::Array => "array",
            ValidatorType::Hash => "hash",
        }
    }
}

/// A validator with type, constraints, and rules.
#[derive(Clone)]
pub struct Validator {
    pub validator_type: ValidatorType,
    pub required: bool,
    pub nullable: bool,
    pub default_value: Option<Value>,
    pub rules: Vec<ValidationRule>,
    pub nested_schema: Option<Value>,
}

impl Validator {
    pub fn new(validator_type: ValidatorType, nested_schema: Option<Value>) -> Self {
        Self {
            validator_type,
            required: false,
            nullable: false,
            default_value: None,
            rules: Vec::new(),
            nested_schema,
        }
    }

    /// Convert the validator to a Soli Value (Hash with __validator__ marker).
    pub fn to_value(&self) -> Value {
        let validator_rc = Rc::new(RefCell::new(self.clone()));

        // Create a hash with chainable methods
        let mut pairs: Vec<(Value, Value)> = vec![
            (
                Value::String("__validator__".to_string()),
                Value::Bool(true),
            ),
            (
                Value::String("__type__".to_string()),
                Value::String(self.validator_type.name().to_string()),
            ),
            (
                Value::String("__required__".to_string()),
                Value::Bool(self.required),
            ),
            (
                Value::String("__nullable__".to_string()),
                Value::Bool(self.nullable),
            ),
        ];

        if let Some(ref default) = self.default_value {
            pairs.push((Value::String("__default__".to_string()), default.clone()));
        }

        if let Some(ref schema) = self.nested_schema {
            pairs.push((Value::String("__nested_schema__".to_string()), schema.clone()));
        }

        // Serialize rules
        let rules_array: Vec<Value> = self.rules.iter().map(|r| r.to_value()).collect();
        pairs.push((
            Value::String("__rules__".to_string()),
            Value::Array(Rc::new(RefCell::new(rules_array))),
        ));

        // Add chainable methods
        let validator_for_required = validator_rc.clone();
        pairs.push((
            Value::String("required".to_string()),
            Value::NativeFunction(NativeFunction::new("required", Some(0), move |_args| {
                let mut v = validator_for_required.borrow().clone();
                v.required = true;
                Ok(v.to_value())
            })),
        ));

        let validator_for_optional = validator_rc.clone();
        pairs.push((
            Value::String("optional".to_string()),
            Value::NativeFunction(NativeFunction::new("optional", Some(0), move |_args| {
                let mut v = validator_for_optional.borrow().clone();
                v.required = false;
                Ok(v.to_value())
            })),
        ));

        let validator_for_nullable = validator_rc.clone();
        pairs.push((
            Value::String("nullable".to_string()),
            Value::NativeFunction(NativeFunction::new("nullable", Some(0), move |_args| {
                let mut v = validator_for_nullable.borrow().clone();
                v.nullable = true;
                Ok(v.to_value())
            })),
        ));

        let validator_for_default = validator_rc.clone();
        pairs.push((
            Value::String("default".to_string()),
            Value::NativeFunction(NativeFunction::new("default", Some(1), move |args| {
                let mut v = validator_for_default.borrow().clone();
                v.default_value = Some(args[0].clone());
                Ok(v.to_value())
            })),
        ));

        // String-specific methods
        if self.validator_type == ValidatorType::String {
            let validator_for_min_length = validator_rc.clone();
            pairs.push((
                Value::String("min_length".to_string()),
                Value::NativeFunction(NativeFunction::new("min_length", Some(1), move |args| {
                    let min = match &args[0] {
                        Value::Int(n) => *n as usize,
                        _ => return Err("min_length() expects integer".to_string()),
                    };
                    let mut v = validator_for_min_length.borrow().clone();
                    v.rules.push(ValidationRule::MinLength(min));
                    Ok(v.to_value())
                })),
            ));

            let validator_for_max_length = validator_rc.clone();
            pairs.push((
                Value::String("max_length".to_string()),
                Value::NativeFunction(NativeFunction::new("max_length", Some(1), move |args| {
                    let max = match &args[0] {
                        Value::Int(n) => *n as usize,
                        _ => return Err("max_length() expects integer".to_string()),
                    };
                    let mut v = validator_for_max_length.borrow().clone();
                    v.rules.push(ValidationRule::MaxLength(max));
                    Ok(v.to_value())
                })),
            ));

            let validator_for_pattern = validator_rc.clone();
            pairs.push((
                Value::String("pattern".to_string()),
                Value::NativeFunction(NativeFunction::new("pattern", Some(1), move |args| {
                    let pattern = match &args[0] {
                        Value::String(s) => s.clone(),
                        _ => return Err("pattern() expects string regex".to_string()),
                    };
                    let mut v = validator_for_pattern.borrow().clone();
                    v.rules.push(ValidationRule::Pattern(pattern));
                    Ok(v.to_value())
                })),
            ));

            let validator_for_email = validator_rc.clone();
            pairs.push((
                Value::String("email".to_string()),
                Value::NativeFunction(NativeFunction::new("email", Some(0), move |_args| {
                    let mut v = validator_for_email.borrow().clone();
                    v.rules.push(ValidationRule::Email);
                    Ok(v.to_value())
                })),
            ));

            let validator_for_url = validator_rc.clone();
            pairs.push((
                Value::String("url".to_string()),
                Value::NativeFunction(NativeFunction::new("url", Some(0), move |_args| {
                    let mut v = validator_for_url.borrow().clone();
                    v.rules.push(ValidationRule::Url);
                    Ok(v.to_value())
                })),
            ));
        }

        // Numeric methods (Int and Float)
        if self.validator_type == ValidatorType::Int || self.validator_type == ValidatorType::Float
        {
            let validator_for_min = validator_rc.clone();
            pairs.push((
                Value::String("min".to_string()),
                Value::NativeFunction(NativeFunction::new("min", Some(1), move |args| {
                    let min = match &args[0] {
                        Value::Int(n) => *n as f64,
                        Value::Float(n) => *n,
                        _ => return Err("min() expects number".to_string()),
                    };
                    let mut v = validator_for_min.borrow().clone();
                    v.rules.push(ValidationRule::Min(min));
                    Ok(v.to_value())
                })),
            ));

            let validator_for_max = validator_rc.clone();
            pairs.push((
                Value::String("max".to_string()),
                Value::NativeFunction(NativeFunction::new("max", Some(1), move |args| {
                    let max = match &args[0] {
                        Value::Int(n) => *n as f64,
                        Value::Float(n) => *n,
                        _ => return Err("max() expects number".to_string()),
                    };
                    let mut v = validator_for_max.borrow().clone();
                    v.rules.push(ValidationRule::Max(max));
                    Ok(v.to_value())
                })),
            ));
        }

        // one_of() - works for any type
        let validator_for_one_of = validator_rc.clone();
        pairs.push((
            Value::String("one_of".to_string()),
            Value::NativeFunction(NativeFunction::new("one_of", Some(1), move |args| {
                let allowed = match &args[0] {
                    Value::Array(arr) => arr.borrow().clone(),
                    _ => return Err("one_of() expects array".to_string()),
                };
                let mut v = validator_for_one_of.borrow().clone();
                v.rules.push(ValidationRule::OneOf(allowed));
                Ok(v.to_value())
            })),
        ));

        Value::Hash(Rc::new(RefCell::new(pairs)))
    }

    /// Parse a validator from a Soli Value.
    pub fn from_value(value: &Value) -> Result<Self, Value> {
        let hash = match value {
            Value::Hash(h) => h.borrow().clone(),
            _ => return Err(create_error("schema", "invalid validator", "invalid_schema")),
        };

        // Check for __validator__ marker
        let is_validator = hash.iter().any(|(k, v)| {
            if let (Value::String(key), Value::Bool(true)) = (k, v) {
                key == "__validator__"
            } else {
                false
            }
        });

        if !is_validator {
            return Err(create_error("schema", "invalid validator", "invalid_schema"));
        }

        // Extract type
        let validator_type = hash
            .iter()
            .find_map(|(k, v)| {
                if let (Value::String(key), Value::String(type_name)) = (k, v) {
                    if key == "__type__" {
                        return match type_name.as_str() {
                            "string" => Some(ValidatorType::String),
                            "int" => Some(ValidatorType::Int),
                            "float" => Some(ValidatorType::Float),
                            "bool" => Some(ValidatorType::Bool),
                            "array" => Some(ValidatorType::Array),
                            "hash" => Some(ValidatorType::Hash),
                            _ => None,
                        };
                    }
                }
                None
            })
            .ok_or_else(|| create_error("schema", "missing validator type", "invalid_schema"))?;

        // Extract required
        let required = hash.iter().any(|(k, v)| {
            if let (Value::String(key), Value::Bool(true)) = (k, v) {
                key == "__required__"
            } else {
                false
            }
        });

        // Extract nullable
        let nullable = hash.iter().any(|(k, v)| {
            if let (Value::String(key), Value::Bool(true)) = (k, v) {
                key == "__nullable__"
            } else {
                false
            }
        });

        // Extract default value
        let default_value = hash.iter().find_map(|(k, v)| {
            if let Value::String(key) = k {
                if key == "__default__" {
                    return Some(v.clone());
                }
            }
            None
        });

        // Extract nested schema
        let nested_schema = hash.iter().find_map(|(k, v)| {
            if let Value::String(key) = k {
                if key == "__nested_schema__" {
                    return Some(v.clone());
                }
            }
            None
        });

        // Extract rules
        let mut rules = Vec::new();
        for (k, v) in hash.iter() {
            if let Value::String(key) = k {
                if key == "__rules__" {
                    if let Value::Array(arr) = v {
                        for rule_value in arr.borrow().iter() {
                            if let Some(rule) = ValidationRule::from_value(rule_value) {
                                rules.push(rule);
                            }
                        }
                    }
                }
            }
        }

        Ok(Validator {
            validator_type,
            required,
            nullable,
            default_value,
            rules,
            nested_schema,
        })
    }
}
