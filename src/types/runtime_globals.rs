//! What the runtime defines, read from the runtime.
//!
//! The checker models the names it knows precisely — `len`, `str`, `HTTP`,
//! `I18n`. Everything else the interpreter installs used to be invisible to
//! it, and every reference to one read as `Undefined variable` on code that
//! runs: `describe` in a spec, `RateLimiter` in a model, `middleware` in
//! `config/routes.sl`. On one real application that was 140 of 237 reported
//! errors, all false, which is how `soli check` ends up outside a project's
//! verification gate — and a gate one has learnt to ignore protects nothing.
//!
//! A second hand-written list would drift the same way (`lint::rules::scope`'s
//! `WELL_KNOWN_GLOBALS` is already one, and it too was missing `middleware`
//! and `RateLimiter`). So the list is *derived*: register the builtins into a
//! throwaway environment, ask it what it now holds, and parse the preludes the
//! runtime evaluates as Soli source — the routing DSL, the mailer, the form
//! builder — for what they declare. A builtin added anywhere is known here on
//! the next build, with nothing to maintain.
//!
//! Two sets, not one, because the runtime has two. `register_builtins` takes a
//! flag: a served application does **not** get `describe`, `test`, `expect` or
//! `as_guest`, and a `describe(...)` in a controller is therefore a real error
//! worth reporting. So the served namespace is seeded always, and the test DSL
//! only for a file that `soli test` would run — see `test_dsl_globals`.

use std::sync::OnceLock;

/// One global the runtime installs.
pub(crate) struct RuntimeGlobal {
    /// The name the global is bound to.
    pub name: String,
    /// When the global is a class, the verbs reachable on it — native statics
    /// and native instance methods. `None` for anything that is not a class.
    pub class_verbs: Option<Vec<String>>,
}

/// The runtime's global namespace. Computed once per process.
pub(crate) fn runtime_globals() -> &'static [RuntimeGlobal] {
    static GLOBALS: OnceLock<Vec<RuntimeGlobal>> = OnceLock::new();
    GLOBALS.get_or_init(collect)
}

fn collect() -> Vec<RuntimeGlobal> {
    use crate::interpreter::value::Value;

    // `false`: what a served application holds. The test DSL is seeded
    // separately, per file (`test_dsl_globals`).
    let mut globals: Vec<RuntimeGlobal> = registered(false)
        .into_iter()
        .map(|(name, value)| {
            let class_verbs = match &value {
                Value::Class(class) => {
                    let mut verbs: Vec<String> = class
                        .native_static_methods
                        .keys()
                        .chain(class.native_methods.keys())
                        .cloned()
                        .collect();
                    verbs.sort();
                    verbs.dedup();
                    Some(verbs)
                }
                _ => None,
            };
            RuntimeGlobal { name, class_verbs }
        })
        .collect();

    // What the runtime installs as Soli source rather than as Rust: the
    // routing verbs, the mailer prelude (`Mailer`, `Message`), the form
    // builder (`form_with`, `csrf_field`, `button_to`), the upload helpers.
    // Parsed rather than transcribed, for the same reason as everything else
    // here.
    for source in SOLI_PRELUDES {
        globals.extend(prelude_names(source).into_iter().map(|name| RuntimeGlobal {
            name,
            class_verbs: None,
        }));
    }

    // Request-scope names are seeded per file, like the test DSL.
    globals.retain(|g| !REQUEST_ONLY.contains(&g.name.as_str()));

    globals
}

/// Names that only mean something while a request is being served.
///
/// Two kinds, one rule. The server injects the first kind into the handler's
/// scope just before it runs (`call_handler`'s `define_or_update` calls in
/// `serve::mod`), so they exist in no table to read — this is the one list in
/// this module that is not derived, because there is nothing to derive it
/// from. The second kind is `render` and `redirect`: they *are* registered
/// builtins, and are deliberately kept out of the served namespace anyway,
/// because a standalone script that calls them cannot work — a decision pinned
/// by `render_and_redirect_stay_unknown_to_the_checker` in
/// `tests/type_check_test.rs`. The same argument covers their cousins
/// (`render_template`, `res_redirect`); the decision names these two, so the
/// set does too.
///
/// Seeded for a file that belongs to an application — under `app/`, `config/`
/// or `stdlib/` — and for a spec, and for nothing else. A loose script still
/// gets `Undefined variable 'render'`, which is the point.
const REQUEST_ONLY: &[&str] = &[
    "req",
    "request",
    "response",
    "params",
    "query",
    "body",
    "headers",
    "cookies",
    "session",
    "flash",
    "current_user",
    "errors",
    "render",
    "redirect",
];

/// The request-scope names, for a checker that knows it is reading
/// application code. See [`REQUEST_ONLY`].
pub fn request_scope_globals() -> &'static [&'static str] {
    REQUEST_ONLY
}

/// Every global `register_builtins` installs, with its value.
fn registered(include_test_builtins: bool) -> Vec<(String, crate::interpreter::value::Value)> {
    use crate::interpreter::environment::Environment;

    let mut env = Environment::with_builtins_capacity();
    crate::interpreter::builtins::register_builtins(&mut env, include_test_builtins);
    env.get_all_bindings().into_iter().collect()
}

/// The test DSL: what the runtime installs **only** when it is about to run
/// tests — `describe`, `test`, `expect`, the session helpers (`as_guest`,
/// `login`), the factories, the browser verbs.
///
/// Derived as the difference between the two registration modes, so it needs
/// no list of its own either. Seeded by the checker for a file `soli test`
/// would run, and for no other — that is what keeps `describe(...)` in a
/// controller an error.
pub fn test_dsl_globals() -> &'static [String] {
    static NAMES: OnceLock<Vec<String>> = OnceLock::new();
    NAMES.get_or_init(|| {
        let served: std::collections::HashSet<String> =
            registered(false).into_iter().map(|(n, _)| n).collect();
        let mut names: Vec<String> = registered(true)
            .into_iter()
            .map(|(n, _)| n)
            .filter(|n| !served.contains(n))
            .collect();
        names.sort();
        names
    })
}

/// Preludes the runtime evaluates as Soli source.
const SOLI_PRELUDES: &[&str] = &[
    crate::serve::app_loader::ROUTES_DSL_SOURCE,
    crate::interpreter::builtins::mailer::MAILER_PRELUDE,
    crate::interpreter::builtins::template::FORM_BUILDER_SOURCE,
    crate::interpreter::builtins::retry::RETRY_SOURCE,
    crate::serve::uploads_prelude::UPLOADS_PRELUDE_SOURCE,
    crate::serve::uploads_prelude::UPLOADS_HELPERS_SOURCE,
];

/// The top-level functions and classes one Soli prelude declares.
///
/// A prelude that stops parsing contributes nothing rather than failing the
/// check: it is the runtime's own source, and if it were broken the server
/// would not start.
fn prelude_names(source: &str) -> Vec<String> {
    use crate::ast::stmt::StmtKind;

    let Ok(tokens) = crate::lexer::Scanner::new(source).scan_tokens() else {
        return Vec::new();
    };
    let Ok(program) = crate::parser::Parser::new(tokens).parse() else {
        return Vec::new();
    };
    program
        .statements
        .iter()
        .filter_map(|stmt| match &stmt.kind {
            StmtKind::Function(decl) => Some(decl.name.clone()),
            StmtKind::Class(decl) => Some(decl.name.clone()),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has(name: &str) -> bool {
        runtime_globals().iter().any(|g| g.name == name)
    }

    #[test]
    fn the_test_dsl_is_held_apart_from_the_served_namespace() {
        // `describe` alone accounted for one error per spec file: the outer
        // call failed, and its body — every `test`, every assertion — went
        // unchecked behind it. It belongs to a spec, though, not to a served
        // application, so it is not in the always-seeded set.
        assert!(!has("describe"));
        let dsl = test_dsl_globals();
        assert!(dsl.iter().any(|n| n == "describe"));
        assert!(dsl.iter().any(|n| n == "test"));
        assert!(dsl.iter().any(|n| n == "as_guest"));
        // And nothing a served application needs leaked into it. `res_body`
        // reads a response and is registered unconditionally — a spec is where
        // it is *used*, not where it comes from.
        assert!(!dsl.iter().any(|n| n == "render"));
        assert!(!dsl.iter().any(|n| n == "len"));
        assert!(has("res_body"));
    }

    #[test]
    fn covers_the_routing_verbs() {
        // Declared in Soli, not in Rust: without parsing the DSL source,
        // `middleware(...)` in config/routes.sl is an undefined variable.
        assert!(has("middleware"));
        assert!(has("resources"));
        assert!(has("uploads"));
    }

    #[test]
    fn a_builtin_class_carries_its_verbs() {
        let i18n = runtime_globals()
            .iter()
            .find(|g| g.name == "I18n")
            .expect("I18n is registered by register_builtins");
        let verbs = i18n
            .class_verbs
            .as_ref()
            .expect("a class global reports its verbs");
        assert!(verbs.iter().any(|v| v == "translate"));
        // The two the hand-written surface was missing.
        assert!(verbs.iter().any(|v| v == "cache_table"));
        assert!(verbs.iter().any(|v| v == "cached_table"));
    }

    #[test]
    fn covers_the_preludes_written_in_soli() {
        // `Mailer.deliver(...)` is called from jobs and controllers; the class
        // is a Soli prelude the server evaluates at boot.
        assert!(has("Mailer"));
        assert!(has("Message"));
        assert!(has("form_with"));
        assert!(has("csrf_field"));
    }

    #[test]
    fn the_request_scope_is_held_apart_too() {
        // Every controller action reads `req`; helpers read `cookies`. They
        // mean nothing outside a request, so they are seeded for application
        // code and not for a loose script.
        assert!(!has("req"));
        assert!(!has("render"));
        let scope = request_scope_globals();
        assert!(scope.contains(&"req"));
        assert!(scope.contains(&"cookies"));
        assert!(scope.contains(&"current_user"));
        // The decision `render_and_redirect_stay_unknown_to_the_checker` pins.
        assert!(scope.contains(&"render"));
        assert!(scope.contains(&"redirect"));
    }

    #[test]
    fn a_plain_function_reports_no_verbs() {
        let len = runtime_globals()
            .iter()
            .find(|g| g.name == "len")
            .expect("len is a builtin");
        assert!(len.class_verbs.is_none());
    }
}
