//! Every builtin the runtime registers should also be known to the type checker
//! and to the linter — and where one is not, the gap is written down.
//!
//! Soli installs its builtins into one `Environment` at startup, but two other
//! surfaces have to be told about each one by hand:
//!
//!   * `src/types/environment.rs` — or `soli check` fails the program with
//!     `Undefined variable '<name>'`.
//!   * `src/lint/rules/scope.rs` — or `smell/undefined-local` flags every call
//!     site, because `let` is optional and the rule cannot tell a builtin from
//!     a typo.
//!
//! Both lists are maintained by hand, so both drift, and the drift is silent:
//! the builtin works, and only the tooling refuses it. That has now been paid
//! for twice — once for the ten `pdf_*` builtins, once for `Image` / `File` /
//! `Trusted` — and each time it was found by a user rather than by a test.
//!
//! The runtime environment is the authoritative enumerator here, the same
//! choice `examples/builtin_inventory` makes and for the same reason: grepping
//! the registration sites misses whatever is registered in a loop.
//!
//! **This is a ratchet, not a clean bill of health.** A few hundred names are
//! legitimately unknown to one surface or the other — the test DSL, the model
//! DSL, the `__`-prefixed internals — and they are listed in
//! `tests/builtin_registration_baseline.txt`. The test fails when a name shows
//! up that is not in that file, and fails again when a name in that file has
//! since been registered. So the baseline can only shrink, and a new builtin
//! cannot reach `main` without someone deciding, in a commit, which it is.

use std::collections::BTreeSet;

use solilang::interpreter::builtins::register_builtins;
use solilang::interpreter::environment::Environment;
use solilang::interpreter::value::Value;
use solilang::lint::rules::scope::is_likely_global;
use solilang::types::environment::TypeEnvironment;

const BASELINE: &str = include_str!("builtin_registration_baseline.txt");

/// The two sections of the baseline file, as sets.
fn baseline() -> (BTreeSet<String>, BTreeSet<String>) {
    let (mut types, mut lint) = (BTreeSet::new(), BTreeSet::new());
    let mut section = None;
    for raw in BASELINE.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match line {
            "[type-checker]" => section = Some(&mut types),
            "[linter]" => section = Some(&mut lint),
            name => {
                let set = section
                    .as_mut()
                    .expect("baseline: a name before any [section] header");
                set.insert(name.to_string());
            }
        }
    }
    (types, lint)
}

/// Every global the runtime installs, as the runtime itself sees them.
fn registered_globals() -> BTreeSet<String> {
    let env = std::rc::Rc::new(std::cell::RefCell::new(
        Environment::with_builtins_capacity(),
    ));
    register_builtins(&mut env.borrow_mut(), true);

    let bindings = env.borrow().get_all_bindings();
    bindings
        .iter()
        .filter(|(_, v)| matches!(v, Value::NativeFunction(_)))
        .map(|(name, _)| name.clone())
        .collect()
}

/// What the two surfaces do not know about, right now.
fn current_gaps() -> (BTreeSet<String>, BTreeSet<String>) {
    let type_env = TypeEnvironment::new();
    let (mut types, mut lint) = (BTreeSet::new(), BTreeSet::new());

    for name in registered_globals() {
        if type_env.get(&name).is_none() {
            types.insert(name.clone());
        }
        if !is_likely_global(&name) {
            lint.insert(name);
        }
    }
    (types, lint)
}

fn report(surface: &str, fix: &str, added: &BTreeSet<String>, fixed: &BTreeSet<String>) -> String {
    let mut out = String::new();
    if !added.is_empty() {
        out.push_str(&format!(
            "\n{} builtin(s) the runtime registers that {surface} does not know about, \
             and that are not in the baseline:\n  {}\n\n  Register them in {fix} — or, if \
             they are DSL that only makes sense inside a spec, a model body or a \
             migration, add them to tests/builtin_registration_baseline.txt and say so \
             in the commit.\n",
            added.len(),
            added.iter().cloned().collect::<Vec<_>>().join("\n  ")
        ));
    }
    if !fixed.is_empty() {
        out.push_str(&format!(
            "\n{} name(s) in the baseline are now known to {surface}. Good — delete these \
             lines from tests/builtin_registration_baseline.txt so it keeps shrinking:\n  {}\n",
            fixed.len(),
            fixed.iter().cloned().collect::<Vec<_>>().join("\n  ")
        ));
    }
    out
}

#[test]
fn no_builtin_reaches_main_unknown_to_the_type_checker_or_the_linter() {
    let (base_types, base_lint) = baseline();
    let (gap_types, gap_lint) = current_gaps();

    let mut failure = String::new();
    failure.push_str(&report(
        "the type checker",
        "src/types/environment.rs",
        &(&gap_types - &base_types),
        &(&base_types - &gap_types),
    ));
    failure.push_str(&report(
        "the linter",
        "src/lint/rules/scope.rs",
        &(&gap_lint - &base_lint),
        &(&base_lint - &gap_lint),
    ));

    assert!(failure.is_empty(), "{failure}");
}

/// The baseline lists names the runtime registers. One that it no longer does
/// is a line nobody will ever remove on purpose, because nothing points at it.
#[test]
fn the_baseline_only_names_builtins_the_runtime_still_registers() {
    let registered = registered_globals();
    let (types, lint) = baseline();

    let stale: Vec<&String> = types
        .iter()
        .chain(lint.iter())
        .filter(|n| !registered.contains(*n))
        .collect();

    assert!(
        stale.is_empty(),
        "tests/builtin_registration_baseline.txt names {} builtin(s) the runtime no \
         longer registers; delete these lines:\n  {:?}",
        stale.len(),
        stale
    );
}
