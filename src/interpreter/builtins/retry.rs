//! Retry — engine-embedded Soli stdlib.
//!
//! Plain natives cannot invoke Soli functions (see `respond_to.rs`), so
//! retry logic lives in `retry.sl`. It is evaluated once per thread and the
//! resulting class is defined into every environment that asks for it — see
//! `RETRY_CLASS` for why it is not evaluated into each of them.

use std::cell::RefCell;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::Value;

pub(crate) const RETRY_SOURCE: &str = include_str!("retry.sl");

thread_local! {
    /// The evaluated Retry class, once per thread.
    ///
    /// Its methods close over the environment they were evaluated in. When
    /// that was the environment receiving the class, the two held each other:
    /// the environment owned `Retry`, `Retry`'s methods owned the environment,
    /// and neither was ever freed. Every `Interpreter::new()` registers Retry,
    /// and one is built for each call of a named scope, a user method on a
    /// primitive, a validator — so a server kept a whole builtins registry
    /// (~350 KB) per call, and a page making a few dozen scope calls grew the
    /// worker by megabytes on every request.
    ///
    /// Evaluated here into an environment of its own that lives as long as the
    /// thread, the class points at that environment and at nothing it is
    /// registered into, so those can be dropped.
    static RETRY_CLASS: RefCell<Option<Value>> = const { RefCell::new(None) };
}

/// Define the embedded Retry class in `env` so `Retry.with_backoff(...)` /
/// `Retry.within(...)` resolve everywhere.
pub fn register_retry_class(env: &Rc<RefCell<Environment>>) -> Result<(), String> {
    let class = retry_class()?;
    env.borrow_mut().define("Retry".to_string(), class);
    Ok(())
}

fn retry_class() -> Result<Value, String> {
    if let Some(class) = RETRY_CLASS.with(|slot| slot.borrow().clone()) {
        return Ok(class);
    }

    let tokens = crate::lexer::Scanner::new(RETRY_SOURCE)
        .scan_tokens()
        .map_err(|e| format!("retry stdlib lexer error: {}", e))?;
    let program = crate::parser::Parser::new(tokens)
        .parse()
        .map_err(|e| format!("retry stdlib parser error: {}", e))?;
    // The class body only needs builtins: the blocks it runs are the caller's
    // closures, which carry their own environment.
    let home = Rc::new(RefCell::new(Environment::with_builtins_capacity()));
    crate::interpreter::builtins::register_builtins(&mut home.borrow_mut(), false);
    let mut interpreter = crate::interpreter::Interpreter::with_environment(home.clone());
    for stmt in &program.statements {
        interpreter
            .execute(stmt)
            .map_err(|e| format!("retry stdlib eval error: {}", e))?;
    }
    let class = home
        .borrow()
        .get("Retry")
        .ok_or_else(|| "retry stdlib did not define Retry".to_string())?;
    RETRY_CLASS.with(|slot| *slot.borrow_mut() = Some(class.clone()));
    Ok(class)
}
