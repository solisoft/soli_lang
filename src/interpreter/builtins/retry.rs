//! Retry — engine-embedded Soli stdlib.
//!
//! Plain natives cannot invoke Soli functions (see `respond_to.rs`), so
//! retry logic lives in `retry.sl` and is evaluated into the global
//! environment at startup, exactly like the template form builder.

use std::cell::RefCell;
use std::rc::Rc;

use crate::interpreter::environment::Environment;

const RETRY_SOURCE: &str = include_str!("retry.sl");

/// Evaluate the embedded Retry class into the builtins environment so
/// `Retry.with_backoff(...)` / `Retry.within(...)` resolve everywhere.
pub fn register_retry_class(env: &Rc<RefCell<Environment>>) -> Result<(), String> {
    let tokens = crate::lexer::Scanner::new(RETRY_SOURCE)
        .scan_tokens()
        .map_err(|e| format!("retry stdlib lexer error: {}", e))?;
    let program = crate::parser::Parser::new(tokens)
        .parse()
        .map_err(|e| format!("retry stdlib parser error: {}", e))?;
    let mut interpreter = crate::interpreter::Interpreter::with_environment(env.clone());
    for stmt in &program.statements {
        interpreter
            .execute(stmt)
            .map_err(|e| format!("retry stdlib eval error: {}", e))?;
    }
    Ok(())
}
