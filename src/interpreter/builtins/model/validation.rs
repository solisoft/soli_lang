//! Validation types and execution logic.

use indexmap::IndexMap;
use std::cell::RefCell;
use std::rc::Rc;

use crate::interpreter::value::{HashKey, Value};

use super::core::MODEL_REGISTRY;

/// A single validation rule for a field.
#[derive(Debug, Clone, Default)]
pub struct ValidationRule {
    pub field: String,
    pub presence: bool,
    pub uniqueness: bool,
    pub min_length: Option<usize>,
    pub max_length: Option<usize>,
    pub format: Option<String>, // regex pattern
    pub numericality: bool,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub custom: Option<String>, // method name for custom validation
}

impl ValidationRule {
    pub fn new(field: String) -> Self {
        Self {
            field,
            ..Default::default()
        }
    }
}

/// A validation error.
#[derive(Debug, Clone)]
pub struct ValidationError {
    pub field: String,
    pub message: String,
}

impl ValidationError {
    pub fn new(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
        }
    }

    pub fn to_value(&self) -> Value {
        let mut pairs: IndexMap<HashKey, Value> = IndexMap::new();
        pairs.insert(
            HashKey::String("field".into()),
            Value::String(self.field.clone()),
        );
        pairs.insert(
            HashKey::String("message".into()),
            Value::String(self.message.clone()),
        );
        Value::Hash(Rc::new(RefCell::new(pairs)))
    }
}

/// Register a validation rule for a model class.
pub fn register_validation(class_name: &str, rule: ValidationRule) {
    let mut registry = MODEL_REGISTRY.write().unwrap();
    let metadata = registry.entry(class_name.to_string()).or_default();
    metadata.validations.push(rule);
}

/// Run validations on data and return any errors.
pub fn run_validations(class_name: &str, data: &Value, _is_create: bool) -> Vec<ValidationError> {
    let registry = MODEL_REGISTRY.read().unwrap();
    let metadata = match registry.get(class_name) {
        Some(m) => m,
        None => return vec![],
    };

    let hash = match data {
        Value::Hash(h) => h.borrow(),
        _ => return vec![ValidationError::new("_base", "Data must be a hash")],
    };

    let mut errors = Vec::new();

    for rule in &metadata.validations {
        // Find the field value
        let field_value = hash
            .iter()
            .find(|(k, _)| matches!(k, HashKey::String(s) if s == &rule.field))
            .map(|(_, v)| v.clone());

        // Presence validation
        if rule.presence {
            match &field_value {
                None => errors.push(ValidationError::new(&rule.field, "can't be blank")),
                Some(Value::Null) => {
                    errors.push(ValidationError::new(&rule.field, "can't be blank"))
                }
                Some(Value::String(s)) if s.is_empty() => {
                    errors.push(ValidationError::new(&rule.field, "can't be blank"))
                }
                _ => {}
            }
        }

        // Min length validation
        if let Some(min_len) = rule.min_length {
            if let Some(Value::String(s)) = &field_value {
                if s.len() < min_len {
                    errors.push(ValidationError::new(
                        &rule.field,
                        format!("is too short (minimum is {} characters)", min_len),
                    ));
                }
            }
        }

        // Max length validation
        if let Some(max_len) = rule.max_length {
            if let Some(Value::String(s)) = &field_value {
                if s.len() > max_len {
                    errors.push(ValidationError::new(
                        &rule.field,
                        format!("is too long (maximum is {} characters)", max_len),
                    ));
                }
            }
        }

        // Format validation (regex)
        if let Some(pattern) = &rule.format {
            if let Some(Value::String(s)) = &field_value {
                if let Ok(re) = regex::Regex::new(pattern) {
                    if !re.is_match(s) {
                        errors.push(ValidationError::new(&rule.field, "is invalid"));
                    }
                }
            }
        }

        // Numericality validation
        if rule.numericality {
            match &field_value {
                Some(Value::Int(_)) | Some(Value::Float(_)) => {}
                Some(_) => errors.push(ValidationError::new(&rule.field, "is not a number")),
                None => {} // Skip if field is not present (presence handles required)
            }
        }

        // Min value validation
        if let Some(min_val) = rule.min {
            match &field_value {
                Some(Value::Int(n)) if (*n as f64) < min_val => {
                    errors.push(ValidationError::new(
                        &rule.field,
                        format!("must be greater than or equal to {}", min_val),
                    ));
                }
                Some(Value::Float(n)) if *n < min_val => {
                    errors.push(ValidationError::new(
                        &rule.field,
                        format!("must be greater than or equal to {}", min_val),
                    ));
                }
                _ => {}
            }
        }

        // Max value validation
        if let Some(max_val) = rule.max {
            match &field_value {
                Some(Value::Int(n)) if (*n as f64) > max_val => {
                    errors.push(ValidationError::new(
                        &rule.field,
                        format!("must be less than or equal to {}", max_val),
                    ));
                }
                Some(Value::Float(n)) if *n > max_val => {
                    errors.push(ValidationError::new(
                        &rule.field,
                        format!("must be less than or equal to {}", max_val),
                    ));
                }
                _ => {}
            }
        }
    }

    errors
}

/// Build a validation result hash.
pub fn build_validation_result(
    valid: bool,
    errors: Vec<ValidationError>,
    record: Option<Value>,
) -> Value {
    let mut pairs: IndexMap<HashKey, Value> = IndexMap::new();
    pairs.insert(HashKey::String("valid".into()), Value::Bool(valid));

    if !valid {
        let error_values: Vec<Value> = errors.iter().map(|e| e.to_value()).collect();
        pairs.insert(
            HashKey::String("errors".into()),
            Value::Array(Rc::new(RefCell::new(error_values))),
        );
    }

    if let Some(rec) = record {
        pairs.insert(HashKey::String("record".into()), rec);
    }

    Value::Hash(Rc::new(RefCell::new(pairs)))
}
