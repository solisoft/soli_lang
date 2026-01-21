//! Validation rules and constraint implementations.

use std::cell::RefCell;
use std::rc::Rc;

use regex::Regex;

use crate::interpreter::value::Value;

use super::create_error;

/// A validation rule that can be applied to a value.
#[derive(Clone, Debug)]
pub enum ValidationRule {
    /// Minimum numeric value
    Min(f64),
    /// Maximum numeric value
    Max(f64),
    /// Minimum string length
    MinLength(usize),
    /// Maximum string length
    MaxLength(usize),
    /// Regex pattern match
    Pattern(String),
    /// Must be a valid email
    Email,
    /// Must be a valid URL
    Url,
    /// Must be one of the allowed values
    OneOf(Vec<Value>),
}

impl ValidationRule {
    /// Validate a value against this rule.
    pub fn validate(&self, field_name: &str, value: &Value) -> Result<(), Value> {
        match self {
            ValidationRule::Min(min) => {
                let num = match value {
                    Value::Int(n) => *n as f64,
                    Value::Float(n) => *n,
                    _ => {
                        return Err(create_error(
                            field_name,
                            "must be a number for min validation",
                            "type_error",
                        ))
                    }
                };
                if num < *min {
                    return Err(create_error(
                        field_name,
                        &format!("must be at least {}", min),
                        "min",
                    ));
                }
            }
            ValidationRule::Max(max) => {
                let num = match value {
                    Value::Int(n) => *n as f64,
                    Value::Float(n) => *n,
                    _ => {
                        return Err(create_error(
                            field_name,
                            "must be a number for max validation",
                            "type_error",
                        ))
                    }
                };
                if num > *max {
                    return Err(create_error(
                        field_name,
                        &format!("must be at most {}", max),
                        "max",
                    ));
                }
            }
            ValidationRule::MinLength(min_len) => {
                let len = match value {
                    Value::String(s) => s.chars().count(),
                    Value::Array(arr) => arr.borrow().len(),
                    _ => {
                        return Err(create_error(
                            field_name,
                            "must be a string or array for min_length validation",
                            "type_error",
                        ))
                    }
                };
                if len < *min_len {
                    return Err(create_error(
                        field_name,
                        &format!("must be at least {} characters", min_len),
                        "min_length",
                    ));
                }
            }
            ValidationRule::MaxLength(max_len) => {
                let len = match value {
                    Value::String(s) => s.chars().count(),
                    Value::Array(arr) => arr.borrow().len(),
                    _ => {
                        return Err(create_error(
                            field_name,
                            "must be a string or array for max_length validation",
                            "type_error",
                        ))
                    }
                };
                if len > *max_len {
                    return Err(create_error(
                        field_name,
                        &format!("must be at most {} characters", max_len),
                        "max_length",
                    ));
                }
            }
            ValidationRule::Pattern(pattern) => {
                let s = match value {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(create_error(
                            field_name,
                            "must be a string for pattern validation",
                            "type_error",
                        ))
                    }
                };
                match Regex::new(pattern) {
                    Ok(re) => {
                        if !re.is_match(&s) {
                            return Err(create_error(
                                field_name,
                                &format!("must match pattern {}", pattern),
                                "pattern",
                            ));
                        }
                    }
                    Err(_) => {
                        return Err(create_error(
                            field_name,
                            &format!("invalid pattern: {}", pattern),
                            "invalid_pattern",
                        ))
                    }
                }
            }
            ValidationRule::Email => {
                let s = match value {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(create_error(
                            field_name,
                            "must be a string for email validation",
                            "type_error",
                        ))
                    }
                };
                // Simple email regex
                let email_re =
                    Regex::new(r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$").unwrap();
                if !email_re.is_match(&s) {
                    return Err(create_error(
                        field_name,
                        "must be a valid email",
                        "invalid_email",
                    ));
                }
            }
            ValidationRule::Url => {
                let s = match value {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(create_error(
                            field_name,
                            "must be a string for URL validation",
                            "type_error",
                        ))
                    }
                };
                // Simple URL regex
                let url_re = Regex::new(r"^https?://[^\s/$.?#].[^\s]*$").unwrap();
                if !url_re.is_match(&s) {
                    return Err(create_error(field_name, "must be a valid URL", "invalid_url"));
                }
            }
            ValidationRule::OneOf(allowed) => {
                let is_match = allowed.iter().any(|allowed_val| {
                    match (value, allowed_val) {
                        (Value::Int(a), Value::Int(b)) => a == b,
                        (Value::Float(a), Value::Float(b)) => (a - b).abs() < f64::EPSILON,
                        (Value::String(a), Value::String(b)) => a == b,
                        (Value::Bool(a), Value::Bool(b)) => a == b,
                        _ => false,
                    }
                });
                if !is_match {
                    let allowed_strs: Vec<String> =
                        allowed.iter().map(|v| format!("{}", v)).collect();
                    return Err(create_error(
                        field_name,
                        &format!("must be one of: {}", allowed_strs.join(", ")),
                        "one_of",
                    ));
                }
            }
        }
        Ok(())
    }

    /// Convert the rule to a Soli Value for serialization.
    pub fn to_value(&self) -> Value {
        let (rule_type, rule_value) = match self {
            ValidationRule::Min(n) => ("min", Value::Float(*n)),
            ValidationRule::Max(n) => ("max", Value::Float(*n)),
            ValidationRule::MinLength(n) => ("min_length", Value::Int(*n as i64)),
            ValidationRule::MaxLength(n) => ("max_length", Value::Int(*n as i64)),
            ValidationRule::Pattern(s) => ("pattern", Value::String(s.clone())),
            ValidationRule::Email => ("email", Value::Bool(true)),
            ValidationRule::Url => ("url", Value::Bool(true)),
            ValidationRule::OneOf(arr) => {
                let values = arr.clone();
                ("one_of", Value::Array(Rc::new(RefCell::new(values))))
            }
        };

        let pairs: Vec<(Value, Value)> = vec![
            (
                Value::String("type".to_string()),
                Value::String(rule_type.to_string()),
            ),
            (Value::String("value".to_string()), rule_value),
        ];
        Value::Hash(Rc::new(RefCell::new(pairs)))
    }

    /// Parse a rule from a Soli Value.
    pub fn from_value(value: &Value) -> Option<Self> {
        let hash = match value {
            Value::Hash(h) => h.borrow().clone(),
            _ => return None,
        };

        let mut rule_type = None;
        let mut rule_value = None;

        for (k, v) in hash.iter() {
            if let Value::String(key) = k {
                match key.as_str() {
                    "type" => {
                        if let Value::String(t) = v {
                            rule_type = Some(t.clone());
                        }
                    }
                    "value" => {
                        rule_value = Some(v.clone());
                    }
                    _ => {}
                }
            }
        }

        let rule_type = rule_type?;
        let rule_value = rule_value?;

        match rule_type.as_str() {
            "min" => {
                let n = match rule_value {
                    Value::Float(f) => f,
                    Value::Int(i) => i as f64,
                    _ => return None,
                };
                Some(ValidationRule::Min(n))
            }
            "max" => {
                let n = match rule_value {
                    Value::Float(f) => f,
                    Value::Int(i) => i as f64,
                    _ => return None,
                };
                Some(ValidationRule::Max(n))
            }
            "min_length" => {
                let n = match rule_value {
                    Value::Int(i) => i as usize,
                    _ => return None,
                };
                Some(ValidationRule::MinLength(n))
            }
            "max_length" => {
                let n = match rule_value {
                    Value::Int(i) => i as usize,
                    _ => return None,
                };
                Some(ValidationRule::MaxLength(n))
            }
            "pattern" => {
                let s = match rule_value {
                    Value::String(s) => s,
                    _ => return None,
                };
                Some(ValidationRule::Pattern(s))
            }
            "email" => Some(ValidationRule::Email),
            "url" => Some(ValidationRule::Url),
            "one_of" => {
                let arr = match rule_value {
                    Value::Array(a) => a.borrow().clone(),
                    _ => return None,
                };
                Some(ValidationRule::OneOf(arr))
            }
            _ => None,
        }
    }
}
