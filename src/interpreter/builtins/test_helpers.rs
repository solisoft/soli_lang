//! Test-only helpers that need `&mut Interpreter` to run user blocks.

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

pub fn register_test_helpers(env: &mut Environment) {
    // with_transaction(fn() { ... }) — begin → run block → always rollback.
    // Real work happens in the `evaluate_call` interceptor (needs the
    // interpreter to invoke the block); this placeholder catches misuse.
    env.define(
        "with_transaction".to_string(),
        Value::NativeFunction(NativeFunction::new("with_transaction", Some(1), |_args| {
            Err(
                "with_transaction() expects a function block: with_transaction(fn() { ... })"
                    .to_string(),
            )
        })),
    );
}
