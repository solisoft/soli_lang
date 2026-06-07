//! Null method call implementations.
//!
//! All null methods are pure, so the dispatch lives in
//! `call_null_method_impl`, shared by the tree-walker and the VM
//! (`vm_primitive_methods.rs`).

use crate::error::RuntimeError;
use crate::interpreter::executor::{Interpreter, RuntimeResult};
use crate::interpreter::value::Value;
use crate::span::Span;

impl Interpreter {
    /// Handle null methods that require arguments.
    pub(crate) fn call_null_method(
        &mut self,
        method_name: &str,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        call_null_method_impl(method_name, &arguments, span)
    }
}

/// Null method dispatch shared by the tree-walker and the VM.
pub(crate) fn call_null_method_impl(
    method_name: &str,
    arguments: &[Value],
    span: Span,
) -> RuntimeResult<Value> {
    match method_name {
        "is_a?" => null_is_a(arguments, span),
        _ => Err(RuntimeError::NoSuchProperty {
            value_type: "null".to_string(),
            property: method_name.to_string(),
            span,
        }),
    }
}

fn null_is_a(arguments: &[Value], span: Span) -> RuntimeResult<Value> {
    if arguments.len() != 1 {
        return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
    }
    let class_name = match &arguments[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Err(RuntimeError::type_error(
                "is_a? expects a string argument",
                span,
            ))
        }
    };
    Ok(Value::Bool(class_name == "null" || class_name == "object"))
}
