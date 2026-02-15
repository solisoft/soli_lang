//! Literal expression evaluation.

use crate::ast::ExprKind;
use crate::interpreter::value::Value;
use crate::span::Span;

use super::{Interpreter, RuntimeResult};
use crate::error::RuntimeError;

impl Interpreter {
    /// Evaluate an SDBQL query block with interpolations.
    pub(crate) fn evaluate_sdql_block(
        &mut self,
        query: &str,
        interpolations: &Vec<crate::ast::expr::SdqlInterpolation>,
        _span: Span,
    ) -> RuntimeResult<Value> {
        use crate::interpreter::builtins::model::crud::exec_async_query;

        let mut processed_query = query.to_string();
        let mut bind_vars = std::collections::HashMap::new();

        for interp in interpolations {
            let expr_str = interp.expr.trim_start_matches('{').trim_end_matches('}');
            let var_name = expr_str.trim();

            if let Some(value) = self.environment.borrow().get(var_name) {
                let json_value = value_to_json(&value);
                bind_vars.insert(var_name.to_string(), json_value);

                let placeholder = format!("#{{{}}}", var_name);
                let sdbql_var = format!("@{}", var_name);
                processed_query = processed_query.replace(&placeholder, &sdbql_var);
            }
        }

        if bind_vars.is_empty() {
            Ok(exec_async_query(processed_query))
        } else {
            use crate::interpreter::builtins::model::crud::exec_async_query_with_binds;
            match exec_async_query_with_binds(processed_query, Some(bind_vars)) {
                Ok(results) => {
                    use crate::interpreter::builtins::model::crud::json_to_value;
                    use std::cell::RefCell;
                    use std::rc::Rc;
                    let values: Vec<Value> = results.iter().map(json_to_value).collect();
                    Ok(Value::Array(Rc::new(RefCell::new(values))))
                }
                Err(e) => Ok(Value::String(format!("Error: {}", e))),
            }
        }
    }

    /// Evaluate an interpolated string expression.
    pub(crate) fn evaluate_interpolated_string(
        &mut self,
        parts: &Vec<crate::ast::expr::InterpolatedPart>,
        _span: Span,
    ) -> RuntimeResult<Value> {
        let mut result = String::new();
        for part in parts {
            match part {
                crate::ast::expr::InterpolatedPart::Literal(s) => {
                    result.push_str(s);
                }
                crate::ast::expr::InterpolatedPart::Expression(expr) => {
                    let value = self.evaluate(expr)?;
                    result.push_str(&value.to_string());
                }
            }
        }
        Ok(Value::String(result))
    }

    /// Evaluate a literal value from an expression kind.
    /// Used for pattern matching.
    pub(crate) fn evaluate_literal(&self, literal: &ExprKind) -> RuntimeResult<Value> {
        match literal {
            ExprKind::IntLiteral(n) => Ok(Value::Int(*n)),
            ExprKind::FloatLiteral(n) => Ok(Value::Float(*n)),
            ExprKind::DecimalLiteral(s) => {
                use crate::interpreter::value::DecimalValue;
                let decimal: rust_decimal::Decimal = s.parse().map_err(|_| {
                    RuntimeError::type_error("invalid decimal literal", Span::default())
                })?;
                let precision = s.split('.').nth(1).map(|p| p.len() as u32).unwrap_or(0);
                Ok(Value::Decimal(DecimalValue(decimal, precision)))
            }
            ExprKind::StringLiteral(s) => Ok(Value::String(s.clone())),
            ExprKind::BoolLiteral(b) => Ok(Value::Bool(*b)),
            ExprKind::Null => Ok(Value::Null),
            _ => Err(RuntimeError::type_error(
                "expected literal expression",
                Span::default(),
            )),
        }
    }

    /// Compare two values for equality.
    /// Used for pattern matching.
    pub(crate) fn values_equal(&self, a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Null, Value::Null) => true,
            _ => false,
        }
    }
}

/// Convert a Value to serde_json::Value for bind vars
fn value_to_json(value: &Value) -> serde_json::Value {
    match value {
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Int(n) => serde_json::json!(*n),
        Value::Float(f) => serde_json::json!(*f),
        Value::String(s) => serde_json::Value::String(s.clone()),
        Value::Array(arr) => {
            let items: Vec<serde_json::Value> = arr.borrow().iter().map(value_to_json).collect();
            serde_json::Value::Array(items)
        }
        Value::Hash(hash) => {
            let mut obj = serde_json::Map::new();
            for (key, val) in hash.borrow().iter() {
                let key_str = match key {
                    crate::interpreter::value::HashKey::String(s) => s.clone(),
                    crate::interpreter::value::HashKey::Int(i) => i.to_string(),
                    crate::interpreter::value::HashKey::Decimal(d) => d.0.to_string(),
                    crate::interpreter::value::HashKey::Bool(b) => b.to_string(),
                    crate::interpreter::value::HashKey::Null => "null".to_string(),
                };
                obj.insert(key_str, value_to_json(val));
            }
            serde_json::Value::Object(obj)
        }
        _ => serde_json::Value::String(value.to_string()),
    }
}
