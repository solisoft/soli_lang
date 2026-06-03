//! Environment variable built-in functions.
//!
//! `getenv` and `hasenv` are read-only — safe to call from any worker.
//! `setenv` and `unsetenv` were exposed in earlier versions but were
//! removed in SEC-033 because `std::env::set_var` is `unsafe` (a worker
//! thread mutating env while another worker reads it via `getenv` or
//! the `SOLIDB_*` builtins is documented UB on Rust 2024 / glibc).
//! User-controlled input flowing into `setenv("PATH", ...)` followed by
//! `System.run` was also a confused-deputy hijack vector.
//!
//! Apps that need per-environment configuration should use `.env` /
//! `.env.{APP_ENV}` (auto-loaded once at single-threaded server boot)
//! or `SOLI_PROTECT_ENV` for the parallel test runner.

use std::env;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

pub fn register_env_builtins(env: &mut Environment) {
    env.define(
        "getenv".to_string(),
        Value::NativeFunction(NativeFunction::new("getenv", Some(1), |args| {
            let name = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "getenv() expects string name, got {}",
                        other.type_name()
                    ))
                }
            };

            match env::var(&*name) {
                Ok(value) => Ok(Value::String(value.into())),
                Err(_) => Ok(Value::Null),
            }
        })),
    );

    // SEC-033: `setenv` and `unsetenv` are removed. They wrapped the
    // `unsafe` `std::env::set_var` / `remove_var` calls and were
    // callable from any worker thread, opening a multi-thread UB
    // window when other workers read env via `getenv` or the
    // `SOLIDB_*` builtins. They also let user-controlled input flow
    // into `setenv("PATH", req["x"])` ahead of `System.run`.
    //
    // Keep the names in the registry so existing code fails with a
    // clear migration error instead of `undefined variable`.
    env.define(
        "setenv".to_string(),
        Value::NativeFunction(NativeFunction::new("setenv", None, |_args| {
            Err(
                "setenv() has been removed (SEC-033). Mutating process env from a worker thread is unsafe. \
                 Use `.env` / `.env.{APP_ENV}` (auto-loaded at single-threaded server boot) or SOLI_PROTECT_ENV for test isolation."
                    .to_string(),
            )
        })),
    );

    env.define(
        "unsetenv".to_string(),
        Value::NativeFunction(NativeFunction::new("unsetenv", None, |_args| {
            Err(
                "unsetenv() has been removed (SEC-033). Mutating process env from a worker thread is unsafe. \
                 Configure via `.env` / `.env.{APP_ENV}` and `SOLI_PROTECT_ENV` instead."
                    .to_string(),
            )
        })),
    );

    env.define(
        "hasenv".to_string(),
        Value::NativeFunction(NativeFunction::new("hasenv", Some(1), |args| {
            let name = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "hasenv() expects string name, got {}",
                        other.type_name()
                    ))
                }
            };

            Ok(Value::Bool(env::var(&*name).is_ok()))
        })),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fetch(env: &Environment, name: &str) -> NativeFunction {
        match env.get(name) {
            Some(Value::NativeFunction(f)) => f.clone(),
            other => panic!("expected NativeFunction for {name}, got {other:?}"),
        }
    }

    /// SEC-033: `setenv`/`unsetenv` are still registered (so the lookup
    /// resolves and Soli code gets a clear migration error rather than
    /// `undefined variable`), but every call returns an error naming
    /// SEC-033 and pointing at the safe alternatives.
    #[test]
    fn setenv_and_unsetenv_are_removed_with_migration_error() {
        let mut env = Environment::new();
        register_env_builtins(&mut env);

        let setenv = fetch(&env, "setenv");
        let err = (setenv.func)(vec![
            Value::String("PATH".into()),
            Value::String("/tmp".into()),
        ])
        .unwrap_err();
        assert!(
            err.contains("SEC-033") && err.contains(".env"),
            "expected SEC-033 migration error pointing at .env, got: {}",
            err
        );

        let unsetenv = fetch(&env, "unsetenv");
        let err = (unsetenv.func)(vec![Value::String("PATH".into())]).unwrap_err();
        assert!(
            err.contains("SEC-033"),
            "expected SEC-033 migration error, got: {}",
            err
        );

        // The read-only helpers are unchanged.
        let getenv = fetch(&env, "getenv");
        let _ = (getenv.func)(vec![Value::String("PATH".into())]).unwrap();
        let hasenv = fetch(&env, "hasenv");
        let _ = (hasenv.func)(vec![Value::String("PATH".into())]).unwrap();
    }
}
