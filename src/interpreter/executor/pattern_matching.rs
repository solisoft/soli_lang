//! Pattern matching evaluation.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use crate::ast::expr::MatchPattern;
use crate::interpreter::value::{HashKey, Value};

use indexmap::IndexMap;

use super::{Interpreter, RuntimeResult};

impl Interpreter {
    /// Match a value against a pattern.
    /// Returns Some(bindings) if the pattern matches, None otherwise.
    /// Bindings are (name, value) pairs for variable patterns.
    pub(crate) fn match_pattern(
        &mut self,
        value: &Value,
        pattern: &MatchPattern,
    ) -> RuntimeResult<Option<Vec<(String, Value)>>> {
        match pattern {
            MatchPattern::Wildcard => Ok(Some(Vec::new())),

            MatchPattern::Variable(name) => Ok(Some(vec![(name.clone(), value.clone())])),

            MatchPattern::Typed { name, type_name } => {
                let matches = match type_name.as_str() {
                    "Int" => matches!(value, Value::Int(_)),
                    "Float" => matches!(value, Value::Float(_)),
                    "Bool" => matches!(value, Value::Bool(_)),
                    "String" => matches!(value, Value::String(_)),
                    "Void" => matches!(value, Value::Null),
                    _ => {
                        if let Value::Instance(inst) = value {
                            inst.borrow().class.name == *type_name
                        } else {
                            false
                        }
                    }
                };

                if matches {
                    Ok(Some(vec![(name.clone(), value.clone())]))
                } else {
                    Ok(None)
                }
            }

            MatchPattern::Literal(literal) => {
                let literal_value = self.evaluate_literal(literal)?;
                if self.values_equal(&literal_value, value) {
                    Ok(Some(Vec::new()))
                } else {
                    Ok(None)
                }
            }

            MatchPattern::Array { elements, rest } => {
                let arr = match value {
                    Value::Array(arr) => arr.borrow().clone(),
                    _ => return Ok(None),
                };

                if rest.is_none() {
                    if elements.len() != arr.len() {
                        return Ok(None);
                    }
                } else if elements.len() > arr.len() {
                    return Ok(None);
                }

                let mut bindings = Vec::new();

                for (i, elem_pattern) in elements.iter().enumerate() {
                    if i >= arr.len() {
                        return Ok(None);
                    }
                    match self.match_pattern(&arr[i], elem_pattern)? { Some(elem_bindings) => {
                        bindings.extend(elem_bindings);
                    } _ => {
                        return Ok(None);
                    }}
                }

                if let Some(rest_name) = rest {
                    let rest_values =
                        Value::Array(Rc::new(RefCell::new(arr[elements.len()..].to_vec())));
                    bindings.push((rest_name.clone(), rest_values));
                }

                Ok(Some(bindings))
            }

            MatchPattern::Hash { fields, rest } => {
                let hash = match value {
                    Value::Hash(hash) => hash.borrow().clone(),
                    _ => return Ok(None),
                };

                let mut bindings = Vec::new();

                for (field_name, field_pattern) in fields {
                    let hash_key = HashKey::String(field_name.clone());
                    if let Some(val) = hash.get(&hash_key) {
                        match self.match_pattern(val, field_pattern)? { Some(field_bindings) => {
                            bindings.extend(field_bindings);
                        } _ => {
                            return Ok(None);
                        }}
                    } else {
                        return Ok(None);
                    }
                }

                if let Some(rest_name) = rest {
                    let matched_keys: HashSet<HashKey> = fields
                        .iter()
                        .map(|(f, _)| HashKey::String(f.clone()))
                        .collect();
                    let rest_map: IndexMap<HashKey, Value> = hash
                        .into_iter()
                        .filter(|(k, _)| !matched_keys.contains(k))
                        .collect();
                    let rest_values = Value::Hash(Rc::new(RefCell::new(rest_map)));
                    bindings.push((rest_name.clone(), rest_values));
                }

                Ok(Some(bindings))
            }

            MatchPattern::Destructuring { type_name, fields } => {
                let instance = match value {
                    Value::Instance(inst) => inst.clone(),
                    _ => return Ok(None),
                };

                if instance.borrow().class.name != *type_name {
                    return Ok(None);
                }

                let mut bindings = Vec::new();

                for (field_name, field_pattern) in fields {
                    match instance.borrow().fields.get(field_name) { Some(field_value) => {
                        match self.match_pattern(field_value, field_pattern)?
                        { Some(field_bindings) => {
                            bindings.extend(field_bindings);
                        } _ => {
                            return Ok(None);
                        }}
                    } _ => {
                        return Ok(None);
                    }}
                }

                Ok(Some(bindings))
            }

            MatchPattern::And(patterns) => {
                let mut all_bindings = Vec::new();
                for p in patterns {
                    match self.match_pattern(value, p)? {
                        Some(bindings) => all_bindings.extend(bindings),
                        None => return Ok(None),
                    }
                }
                Ok(Some(all_bindings))
            }

            MatchPattern::Or(patterns) => {
                for p in patterns {
                    if let Some(bindings) = self.match_pattern(value, p)? {
                        return Ok(Some(bindings));
                    }
                }
                Ok(None)
            }
        }
    }
}
