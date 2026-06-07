//! Native method dispatch for non-collection primitives in the VM:
//! Float, Bool, Null, Decimal (Int lives in `vm_int_methods` because of its
//! closure-taking methods).
//!
//! Bare member access (`x.abs`) resolves through the tree-walker's
//! `*_member_access` tables in `op_get_property`; this module handles the
//! explicit-call form (`x.round(2)`, `x.is_a?("float")`) by delegating to
//! the `call_*_method_impl` dispatchers shared with the tree-walker — so
//! both engines behave identically by construction.

use crate::error::RuntimeError;
use crate::interpreter::executor::calls::{
    bool_methods::call_bool_method_impl, decimal_methods::call_decimal_method_impl,
    float_methods::call_float_method_impl, null_methods::call_null_method_impl,
};
use crate::interpreter::executor::Interpreter;
use crate::interpreter::value::Value;
use crate::span::Span;

use super::vm::Vm;

impl Vm {
    /// Dispatch a method call on a primitive receiver (Int, Float, Bool,
    /// Null, Decimal). Used by CallMethod/CallMethodById, zero-arg member
    /// auto-invoke, and stored bound methods (`call_builtin_method`).
    ///
    /// Empty-args calls (`f.abs()`) first resolve through the member-access
    /// tables so explicit parens behave like the bare form — matching the
    /// tree-walker's evaluate_call. With-args methods come back as a
    /// ValueMethod and fall through to the shared `call_*_method_impl`
    /// dispatchers.
    pub fn vm_call_primitive_method(
        &mut self,
        receiver: &Value,
        name: &str,
        args: &[Value],
        span: Span,
    ) -> Result<Value, RuntimeError> {
        // Int handles its own empty-args delegation (vm_call_int_method).
        if let Value::Int(n) = receiver {
            return self.vm_call_int_method(*n, name, args, span);
        }
        if args.is_empty() {
            let direct = match receiver {
                Value::Float(n) => Interpreter::float_member_access(*n, name, span)?,
                Value::Bool(b) => Interpreter::bool_member_access(*b, name, span)?,
                Value::Null => Interpreter::null_member_access(name, span)?,
                Value::Decimal(d) => Interpreter::decimal_member_access(d, name, span)?,
                _ => {
                    return Err(RuntimeError::NoSuchProperty {
                        value_type: receiver.type_name(),
                        property: name.to_string(),
                        span,
                    })
                }
            };
            match direct {
                Value::Method(_) => {}
                value => return Ok(value),
            }
        }
        match receiver {
            Value::Float(n) => call_float_method_impl(*n, name, args, span),
            Value::Bool(b) => call_bool_method_impl(*b, name, args, span),
            Value::Null => call_null_method_impl(name, args, span),
            Value::Decimal(d) => call_decimal_method_impl(d, name, args, span),
            _ => Err(RuntimeError::NoSuchProperty {
                value_type: receiver.type_name(),
                property: name.to_string(),
                span,
            }),
        }
    }
}
