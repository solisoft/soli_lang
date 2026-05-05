//! `dotenv` / `dotenv!` runtime stubs.
//!
//! SEC-033: these were exposed in earlier versions and called the
//! `unsafe` `std::env::set_var` from worker thread code. That's
//! documented UB on Rust 2024 / glibc the moment another worker reads
//! env (which `getenv` and the `SOLIDB_*` builtins do constantly).
//!
//! `.env` and `.env.{APP_ENV}` are auto-loaded once at single-threaded
//! server boot via `serve::env_loader::load_env_files`, so the runtime
//! stubs are no longer needed. We keep the names registered with
//! migration errors so existing apps fail loud instead of running
//! into `undefined variable`.

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

pub fn register_dotenv_builtins(env: &mut Environment) {
    env.define(
        "dotenv".to_string(),
        Value::NativeFunction(NativeFunction::new("dotenv", None, |_args| {
            Err(
                "dotenv() has been removed (SEC-033). `.env` and `.env.{APP_ENV}` are auto-loaded at single-threaded server boot \
                 — runtime env mutation from a worker thread is unsafe."
                    .to_string(),
            )
        })),
    );

    env.define(
        "dotenv!".to_string(),
        Value::NativeFunction(NativeFunction::new("dotenv!", None, |_args| {
            Err(
                "dotenv!() has been removed (SEC-033). `.env` and `.env.{APP_ENV}` are auto-loaded at single-threaded server boot \
                 — runtime env mutation from a worker thread is unsafe."
                    .to_string(),
            )
        })),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SEC-033: `dotenv` and `dotenv!` are removed but still registered;
    /// callers get a migration error.
    #[test]
    fn dotenv_runtime_callers_get_migration_error() {
        let mut env = Environment::new();
        register_dotenv_builtins(&mut env);

        for name in ["dotenv", "dotenv!"] {
            let f = match env.get(name) {
                Some(Value::NativeFunction(f)) => f.clone(),
                other => panic!("expected NativeFunction for {name}, got {other:?}"),
            };
            let err = (f.func)(vec![]).unwrap_err();
            assert!(
                err.contains("SEC-033") && err.contains(".env"),
                "expected SEC-033 migration error from {}, got: {}",
                name,
                err
            );
        }
    }
}
