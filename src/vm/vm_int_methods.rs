//! Native int method dispatch for the VM.
//!
//! Bare member access (`n.abs`) resolves through the tree-walker's
//! `int_member_access` table in `op_get_property`; this module handles the
//! explicit-call form. Pure with-args methods delegate to
//! `call_int_method_impl`, shared with the tree-walker, so both engines
//! behave identically by construction. Only the closure-taking methods
//! (`times`, `upto`, `downto`) have a VM-specific implementation, using the
//! same callable-batch pattern as `vm_array_methods`. A compiled block
//! usually runs in the dispatch loop instead (`vm_callback_loop.rs`); these
//! drivers serve the other callees and shapes, and the two must agree
//! element for element.

use crate::error::RuntimeError;
use crate::interpreter::executor::calls::int_methods::call_int_method_impl;
use crate::interpreter::executor::Interpreter;
use crate::interpreter::value::Value;
use crate::span::Span;

use super::vm::Vm;

impl Vm {
    /// Dispatch an int method call (the explicit-call form — bare member
    /// access resolves through `int_member_access` in `op_get_property`).
    pub fn vm_call_int_method(
        &mut self,
        n: i64,
        name: &str,
        args: &[Value],
        span: Span,
    ) -> Result<Value, RuntimeError> {
        // A block that broke stops the iterator, which answers nil (#651).
        crate::interpreter::executor::stop_on_block_break(
            self.vm_call_int_method_inner(n, name, args, span),
        )
    }

    fn vm_call_int_method_inner(
        &mut self,
        n: i64,
        name: &str,
        args: &[Value],
        span: Span,
    ) -> Result<Value, RuntimeError> {
        if args.is_empty() {
            // `n.to_s()` / `n.to_s`: the member table answers these with a
            // method value, built only to be dropped here before the call
            // below. Without a user method on Int to outrank it, go
            // straight to the digits.
            if matches!(name, "to_s" | "to_string")
                && !crate::interpreter::executor::calls::user_methods::has_user_methods(
                    crate::interpreter::executor::calls::user_methods::PrimType::Int,
                )
            {
                return Ok(crate::interpreter::executor::calls::int_methods::int_to_s_value(n));
            }
            // `n.abs()` — empty parens behave like the bare form (the
            // tree-walker accepts this too, see evaluate_call). With-args
            // methods come back as a ValueMethod; fall through so explicit
            // zero-arg calls like `n.to_s()` still dispatch below.
            match Interpreter::int_member_access(n, name, span)? {
                Value::Method(_) => {}
                value => return Ok(value),
            }
        }
        match name {
            "times" => {
                let [cb] = require_args::<1>(args, span)?;
                let cb = cb.clone();
                let batch = self.enter_callable_batch();
                let outcome: Result<(), RuntimeError> = (|| {
                    for i in 0..n {
                        self.invoke_in_batch_one(&batch, &cb, Value::Int(i), span)?;
                    }
                    Ok(())
                })();
                self.exit_callable_batch(batch);
                outcome?;
                Ok(Value::Int(n))
            }
            "upto" => {
                let [limit, cb] = require_args::<2>(args, span)?;
                let limit = int_arg(limit, "upto expects an integer limit", span)?;
                let cb = cb.clone();
                let batch = self.enter_callable_batch();
                let outcome: Result<(), RuntimeError> = (|| {
                    for i in n..=limit {
                        self.invoke_in_batch_one(&batch, &cb, Value::Int(i), span)?;
                    }
                    Ok(())
                })();
                self.exit_callable_batch(batch);
                outcome?;
                Ok(Value::Int(n))
            }
            "downto" => {
                let [limit, cb] = require_args::<2>(args, span)?;
                let limit = int_arg(limit, "downto expects an integer limit", span)?;
                let cb = cb.clone();
                let batch = self.enter_callable_batch();
                let outcome: Result<(), RuntimeError> = (|| {
                    let mut i = n;
                    while i >= limit {
                        self.invoke_in_batch_one(&batch, &cb, Value::Int(i), span)?;
                        i -= 1;
                    }
                    Ok(())
                })();
                self.exit_callable_batch(batch);
                outcome?;
                Ok(Value::Int(n))
            }
            _ => call_int_method_impl(n, name, args, span),
        }
    }
}

/// Borrow exactly N arguments or fail with a wrong-arity error.
fn require_args<const N: usize>(args: &[Value], span: Span) -> Result<&[Value; N], RuntimeError> {
    args.try_into()
        .map_err(|_| RuntimeError::wrong_arity(N, args.len(), span))
}

fn int_arg(value: &Value, message: &str, span: Span) -> Result<i64, RuntimeError> {
    match value {
        Value::Int(m) => Ok(*m),
        _ => Err(RuntimeError::type_error(message, span)),
    }
}
