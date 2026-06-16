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
        // Resolve each `#{var}` interpolation to its current value. Only
        // variables that actually exist in scope are bound (matching the
        // original behavior); a missing one leaves its placeholder in the
        // query, which the store rejects.
        let mut binds: Vec<(String, Value)> = Vec::new();
        for interp in interpolations {
            let expr_str = interp.expr.trim_start_matches('{').trim_end_matches('}');
            let var_name = expr_str.trim();
            if let Some(value) = self.environment.borrow().get(var_name) {
                binds.push((var_name.to_string(), value));
            }
        }
        Ok(run_sdql_block(query, &binds))
    }

    /// Evaluate an interpolated string expression.
    pub(crate) fn evaluate_interpolated_string(
        &mut self,
        parts: &Vec<crate::ast::expr::InterpolatedPart>,
        _span: Span,
    ) -> RuntimeResult<Value> {
        // Fast path: single literal (no interpolation)
        if parts.len() == 1 {
            if let crate::ast::expr::InterpolatedPart::Literal(s) = &parts[0] {
                return Ok(Value::String(s.clone().into()));
            }
        }

        // Pre-allocate with estimated capacity
        let capacity: usize = parts
            .iter()
            .map(|p| match p {
                crate::ast::expr::InterpolatedPart::Literal(s) => s.len(),
                _ => 16, // estimate for expression results
            })
            .sum();
        let mut result = String::with_capacity(capacity);

        for part in parts {
            match part {
                crate::ast::expr::InterpolatedPart::Literal(s) => {
                    result.push_str(s);
                }
                crate::ast::expr::InterpolatedPart::Expression(expr) => {
                    let value = self.evaluate(expr)?;
                    value.append_to_string(&mut result);
                }
            }
        }
        Ok(Value::String(result.into()))
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
            ExprKind::StringLiteral(s) => Ok(Value::String(s.clone().into())),
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

/// Execute an `@sdbql{ ... }` block. Shared by the tree-walking interpreter
/// (`evaluate_sdql_block`) and the VM's `__sdql_exec` builtin so both runtimes
/// behave identically. `binds` maps each bare interpolation variable name to
/// its runtime value; every `#{name}` placeholder in `query` is rewritten to a
/// `@name` AQL bind param. Empty binds → no-bind execution. On a query error
/// the result is a `"Error: ..."` string (callers guard with `type(rows)`).
pub(crate) fn run_sdql_block(query: &str, binds: &[(String, Value)]) -> Value {
    use crate::interpreter::builtins::model::crud::{
        exec_async_query, exec_async_query_with_binds, json_to_value,
    };
    use std::cell::RefCell;
    use std::rc::Rc;

    let mut processed_query = query.to_string();
    let mut bind_vars = std::collections::HashMap::new();
    for (var_name, value) in binds {
        bind_vars.insert(var_name.clone(), value_to_json(value));
        let placeholder = format!("#{{{}}}", var_name);
        let sdbql_var = format!("@{}", var_name);
        processed_query = processed_query.replace(&placeholder, &sdbql_var);
    }

    if bind_vars.is_empty() {
        exec_async_query(processed_query)
    } else {
        match exec_async_query_with_binds(processed_query, Some(bind_vars)) {
            Ok(results) => {
                let values: Vec<Value> = results.iter().map(json_to_value).collect();
                Value::Array(Rc::new(RefCell::new(values)))
            }
            Err(e) => Value::String(format!("Error: {}", e).into()),
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
        Value::String(s) => serde_json::Value::String(s.clone().to_string()),
        Value::Array(arr) => {
            let items: Vec<serde_json::Value> = arr.borrow().iter().map(value_to_json).collect();
            serde_json::Value::Array(items)
        }
        Value::Hash(hash) => {
            let mut obj = serde_json::Map::new();
            for (key, val) in hash.borrow().iter() {
                let key_str = match key {
                    crate::interpreter::value::HashKey::String(s) => s.clone(),
                    crate::interpreter::value::HashKey::Symbol(s) => format!(":{}", s).into(),
                    crate::interpreter::value::HashKey::Int(i) => i.to_string().into(),
                    crate::interpreter::value::HashKey::Decimal(d) => d.0.to_string().into(),
                    crate::interpreter::value::HashKey::Bool(b) => b.to_string().into(),
                    crate::interpreter::value::HashKey::Null => "null".into(),
                };
                obj.insert(key_str.to_string(), value_to_json(val));
            }
            serde_json::Value::Object(obj)
        }
        _ => serde_json::Value::String(value.to_string()),
    }
}
