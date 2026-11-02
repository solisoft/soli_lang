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

use std::cell::RefCell;

use ahash::AHashMap;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, SoliStr, Value};

/// Most distinct names one thread remembers before starting over, so a
/// program that builds names (`getenv("FEATURE_#{x}")`) cannot grow it without
/// bound.
const GETENV_CACHE_MAX: usize = 256;

thread_local! {
    /// What `getenv` / `hasenv` answered on this thread, by name — present only
    /// on a thread that called [`cache_reads_on_this_thread`].
    ///
    /// A server worker reads the same few names on every request (the starter
    /// layout's `absolute_url` reads `APP_URL` and `SOLI_APP_HOSTS` once per
    /// page), and each `std::env::var` takes the process environment lock,
    /// searches the whole environment and allocates. A worker starts after
    /// `.env` is loaded and nothing changes the environment after that — Soli
    /// code cannot (`setenv` is gone, SEC-033) — so the first answer stays
    /// right. Everywhere else (scripts, the REPL, the test runner, unit tests
    /// that flip variables) every read goes to the environment.
    static GETENV_CACHE: RefCell<Option<AHashMap<SoliStr, Option<SoliStr>>>> =
        const { RefCell::new(None) };
}

/// Remember `getenv` / `hasenv` answers on this thread from now on. A server
/// worker calls this when it starts, after the environment is final.
pub fn cache_reads_on_this_thread() {
    GETENV_CACHE.with(|cache| {
        cache.borrow_mut().get_or_insert_with(AHashMap::default);
    });
}

/// The value of `name` in the environment, through this thread's cache when it
/// has one.
fn read_env(name: &SoliStr) -> Option<SoliStr> {
    fn uncached(name: &str) -> Option<SoliStr> {
        crate::platform::env::var(name).ok().map(SoliStr::from)
    }
    GETENV_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let Some(map) = cache.as_mut() else {
            return uncached(name);
        };
        if let Some(known) = map.get(name) {
            return known.clone();
        }
        let value = uncached(name);
        if map.len() >= GETENV_CACHE_MAX {
            map.clear();
        }
        map.insert(name.clone(), value.clone());
        value
    })
}

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

            Ok(read_env(&name).map_or(Value::Null, Value::String))
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

            Ok(Value::Bool(read_env(&name).is_some()))
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
        let err = (setenv.func)(&[Value::String("PATH".into()), Value::String("/tmp".into())])
            .unwrap_err();
        assert!(
            err.contains("SEC-033") && err.contains(".env"),
            "expected SEC-033 migration error pointing at .env, got: {}",
            err
        );

        let unsetenv = fetch(&env, "unsetenv");
        let err = (unsetenv.func)(&[Value::String("PATH".into())]).unwrap_err();
        assert!(
            err.contains("SEC-033"),
            "expected SEC-033 migration error, got: {}",
            err
        );

        // The read-only helpers are unchanged.
        let getenv = fetch(&env, "getenv");
        let _ = (getenv.func)(&[Value::String("PATH".into())]).unwrap();
        let hasenv = fetch(&env, "hasenv");
        let _ = (hasenv.func)(&[Value::String("PATH".into())]).unwrap();
    }

    fn call(f: &NativeFunction, name: &str) -> Value {
        (f.func)(&[Value::String(name.into())]).unwrap()
    }

    /// Off a server worker, every `getenv` reads the environment: a change is
    /// seen at once.
    #[test]
    fn getenv_reads_the_environment_each_time_by_default() {
        let mut env = Environment::new();
        register_env_builtins(&mut env);
        let getenv = fetch(&env, "getenv");
        let hasenv = fetch(&env, "hasenv");
        let name = "SOLI_N32_GETENV_UNCACHED";

        std::env::set_var(name, "one");
        assert_eq!(call(&getenv, name), Value::String("one".into()));
        std::env::set_var(name, "two");
        assert_eq!(call(&getenv, name), Value::String("two".into()));
        std::env::remove_var(name);
        assert_eq!(call(&getenv, name), Value::Null);
        assert_eq!(call(&hasenv, name), Value::Bool(false));
    }

    /// On a worker that turned the cache on, the first answer is kept: the
    /// second read does not go to the environment (which a change made behind
    /// its back shows), for a set variable and for an unset one alike.
    #[test]
    fn a_worker_answers_getenv_from_its_cache() {
        let name = "SOLI_N32_GETENV_CACHED";
        let missing = "SOLI_N32_GETENV_CACHED_MISSING";
        std::env::set_var(name, "first");
        std::env::remove_var(missing);

        // Its own thread: the cache is per thread and stays on once enabled.
        // `Value` is not `Send`, so the answers come back compared.
        let outcome = std::thread::spawn(move || {
            let mut env = Environment::new();
            register_env_builtins(&mut env);
            let getenv = fetch(&env, "getenv");
            let hasenv = fetch(&env, "hasenv");

            cache_reads_on_this_thread();
            let before = [call(&getenv, name), call(&hasenv, missing)];
            std::env::set_var(name, "second");
            std::env::set_var(missing, "now set");
            let after = [
                call(&getenv, name),
                call(&hasenv, missing),
                call(&getenv, missing),
            ];
            (
                before == [Value::String("first".into()), Value::Bool(false)],
                after
                    == [
                        Value::String("first".into()),
                        Value::Bool(false),
                        Value::Null,
                    ],
            )
        })
        .join();
        std::env::remove_var(name);
        std::env::remove_var(missing);

        assert_eq!(outcome.unwrap(), (true, true));
    }

    /// Names built at run time cannot grow a worker's cache without bound.
    #[test]
    fn the_getenv_cache_is_bounded() {
        let size = std::thread::spawn(|| {
            cache_reads_on_this_thread();
            for i in 0..(GETENV_CACHE_MAX * 3) {
                read_env(&SoliStr::from(format!("SOLI_N32_BOUND_{i}")));
            }
            GETENV_CACHE.with(|cache| cache.borrow().as_ref().map_or(0, |m| m.len()))
        })
        .join()
        .unwrap();
        assert!(size > 0 && size <= GETENV_CACHE_MAX, "{size}");
    }
}
