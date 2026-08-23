//! Method dispatch for `Value::DateTime`.
//!
//! A DateTime is a native `Value::DateTime(i64)` rather than an `Instance`, so
//! it has no class to look methods up on. Both engines route here, and both
//! read the same registered map, so they cannot drift apart.
//!
//! ## Why the argument list is built on the stack
//!
//! The methods were written when the receiver *was* an object, so each reads
//! `args[0]` as the instant and `args[1..]` as its own arguments. Prepending
//! the receiver through a `Vec` costs a heap allocation on every DateTime
//! method call — measured at a meaningful share of the ~75ns `d.year()` takes,
//! against ~20ns for a minimal builtin like `n.abs()` that dispatches through a
//! compile-time `match` and allocates nothing.
//!
//! Every DateTime method takes at most two user arguments (`format` takes a
//! format string and an optional locale), so the fixed-size arms below cover
//! every real call and the `Vec` is only a correctness fallback.

use crate::error::RuntimeError;
use crate::interpreter::builtins::datetime_class::datetime_method;
use crate::interpreter::executor::RuntimeResult;
use crate::interpreter::value::Value;
use crate::span::Span;

/// Call one of DateTime's instance methods. `ts` is the receiver's instant in
/// nanoseconds; it is passed as `args[0]`, matching how the methods were
/// written when the receiver was an object.
pub(crate) fn call_datetime_method_impl(
    ts: i64,
    method_name: &str,
    arguments: &[Value],
    span: Span,
) -> RuntimeResult<Value> {
    // `is_a?` is universal rather than a registered DateTime method, so it is
    // answered here. Lowercase names, matching every other type's `is_a?` and
    // the `"datetime"` that `.class` reports.
    if method_name == "is_a?" {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let Value::String(class_name) = &arguments[0] else {
            return Err(RuntimeError::type_error(
                "is_a? expects a string argument",
                span,
            ));
        };
        let name = class_name.as_ref();
        return Ok(Value::Bool(name == "datetime" || name == "object"));
    }
    let Some(func) = datetime_method(method_name) else {
        return Err(RuntimeError::type_error(
            format!("DateTime has no method '{}'", method_name),
            span,
        ));
    };
    let recv = Value::DateTime(ts);
    let call = |args: &[Value]| (func.func)(args).map_err(|e| RuntimeError::type_error(e, span));
    match arguments {
        [] => call(&[recv]),
        [a] => call(&[recv, a.clone()]),
        [a, b] => call(&[recv, a.clone(), b.clone()]),
        rest => {
            let mut args = Vec::with_capacity(rest.len() + 1);
            args.push(recv);
            args.extend_from_slice(rest);
            call(&args)
        }
    }
}
