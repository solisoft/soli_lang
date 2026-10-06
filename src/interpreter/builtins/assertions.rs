//! Test assertions for the Soli test DSL.

use crate::error::RuntimeError;
use crate::interpreter::builtins::test_dsl::fmt_value;
use crate::interpreter::environment::Environment;
use crate::interpreter::value::{HashKey, NativeFunction, Value};
use crate::span::Span;
use std::cell::RefCell;
use std::rc::Rc;

thread_local! {
    static ASSERTION_COUNT: Rc<RefCell<i64>> = Rc::new(RefCell::new(0));
}

thread_local! {
    /// Set when an assertion fails, so `assert_raises` can tell a failed
    /// assertion inside its block — which must fail the test — from the error
    /// the block was expected to raise.
    static ASSERTION_FAILED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Build the error an assertion fails with, and mark the failure.
pub(crate) fn assertion_failure(message: impl Into<String>) -> String {
    ASSERTION_FAILED.with(|failed| failed.set(true));
    message.into()
}

/// Forget an earlier failure mark; `assert_raises` calls it before its block.
pub(crate) fn clear_assertion_failed() {
    ASSERTION_FAILED.with(|failed| failed.set(false));
}

/// Whether an assertion failed since the last [`clear_assertion_failed`].
pub(crate) fn assertion_failed() -> bool {
    ASSERTION_FAILED.with(|failed| failed.get())
}

/// Count the assertion and return what every assertion returns on success.
fn pass() -> Result<Value, String> {
    increment_assertion_count();
    Ok(Value::Int(1))
}

/// Check the argument count of an assertion that takes an optional trailing
/// message, and return that message.
fn split_message<'a>(
    name: &str,
    args: &'a [Value],
    required: usize,
) -> Result<Option<&'a str>, String> {
    if args.len() < required || args.len() > required + 1 {
        return Err(format!(
            "{name} expects {required} argument{} and an optional message, got {}",
            if required == 1 { "" } else { "s" },
            args.len()
        ));
    }
    match args.get(required) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(message)) => Ok(Some(message.as_ref())),
        Some(other) => Err(format!(
            "{name}: the message must be a String, got {}",
            other.type_name()
        )),
    }
}

/// Prefix a failure with the caller's message, when one was given.
fn with_message(message: Option<&str>, failure: String) -> String {
    match message {
        Some(message) if !message.is_empty() => assertion_failure(format!("{message}: {failure}")),
        _ => assertion_failure(failure),
    }
}

/// Compare two numbers, Int and Float mixed freely.
pub(crate) fn compare_numbers(
    name: &str,
    left: &Value,
    right: &Value,
) -> Result<std::cmp::Ordering, String> {
    let as_f64 = |value: &Value| match value {
        Value::Int(n) => Some(*n as f64),
        Value::Float(f) => Some(*f),
        _ => None,
    };
    if let (Value::Int(a), Value::Int(b)) = (left, right) {
        return Ok(a.cmp(b));
    }
    match (as_f64(left), as_f64(right)) {
        (Some(a), Some(b)) => a.partial_cmp(&b).ok_or_else(|| {
            format!(
                "{name}: cannot order {} and {}",
                fmt_value(left),
                fmt_value(right)
            )
        }),
        _ => Err(format!(
            "{name} expects two numbers, got {} and {}",
            left.type_name(),
            right.type_name()
        )),
    }
}

pub fn register_assertions(env: &mut Environment) {
    env.define(
        "assert".to_string(),
        Value::NativeFunction(NativeFunction::new("assert", None, |args| {
            let message = split_message("assert", args, 1)?;
            match &args[0] {
                Value::Bool(true) => pass(),
                Value::Bool(false) => Err(with_message(message, "assertion failed".to_string())),
                other => Err(format!(
                    "assert expects a Bool, got {} ({})",
                    other.type_name(),
                    fmt_value(other)
                )),
            }
        })),
    );

    env.define(
        "assert_not".to_string(),
        Value::NativeFunction(NativeFunction::new("assert_not", None, |args| {
            let message = split_message("assert_not", args, 1)?;
            match &args[0] {
                Value::Bool(false) => pass(),
                Value::Bool(true) => Err(with_message(
                    message,
                    "expected false, got true".to_string(),
                )),
                other => Err(format!(
                    "assert_not expects a Bool, got {} ({})",
                    other.type_name(),
                    fmt_value(other)
                )),
            }
        })),
    );

    // assert_eq(actual, expected[, message])
    env.define(
        "assert_eq".to_string(),
        Value::NativeFunction(NativeFunction::new("assert_eq", None, |args| {
            let message = split_message("assert_eq", args, 2)?;
            if args[0] == args[1] {
                pass()
            } else {
                Err(with_message(
                    message,
                    format!(
                        "expected {}, got {}",
                        fmt_value(&args[1]),
                        fmt_value(&args[0])
                    ),
                ))
            }
        })),
    );

    env.define(
        "assert_ne".to_string(),
        Value::NativeFunction(NativeFunction::new("assert_ne", None, |args| {
            let message = split_message("assert_ne", args, 2)?;
            if args[0] != args[1] {
                pass()
            } else {
                Err(with_message(
                    message,
                    format!("expected a value other than {}", fmt_value(&args[1])),
                ))
            }
        })),
    );

    env.define(
        "assert_null".to_string(),
        Value::NativeFunction(NativeFunction::new("assert_null", None, |args| {
            let message = split_message("assert_null", args, 1)?;
            match &args[0] {
                Value::Null => pass(),
                other => Err(with_message(
                    message,
                    format!("expected nil, got {}", fmt_value(other)),
                )),
            }
        })),
    );

    env.define(
        "assert_not_null".to_string(),
        Value::NativeFunction(NativeFunction::new("assert_not_null", None, |args| {
            let message = split_message("assert_not_null", args, 1)?;
            match &args[0] {
                Value::Null => Err(with_message(
                    message,
                    "expected a value, got nil".to_string(),
                )),
                _ => pass(),
            }
        })),
    );

    env.define(
        "assert_gt".to_string(),
        Value::NativeFunction(NativeFunction::new("assert_gt", None, |args| {
            let message = split_message("assert_gt", args, 2)?;
            if compare_numbers("assert_gt", &args[0], &args[1])? == std::cmp::Ordering::Greater {
                pass()
            } else {
                Err(with_message(
                    message,
                    format!(
                        "expected {} to be greater than {}",
                        fmt_value(&args[0]),
                        fmt_value(&args[1])
                    ),
                ))
            }
        })),
    );

    env.define(
        "assert_lt".to_string(),
        Value::NativeFunction(NativeFunction::new("assert_lt", None, |args| {
            let message = split_message("assert_lt", args, 2)?;
            if compare_numbers("assert_lt", &args[0], &args[1])? == std::cmp::Ordering::Less {
                pass()
            } else {
                Err(with_message(
                    message,
                    format!(
                        "expected {} to be less than {}",
                        fmt_value(&args[0]),
                        fmt_value(&args[1])
                    ),
                ))
            }
        })),
    );

    env.define(
        "assert_match".to_string(),
        Value::NativeFunction(NativeFunction::new("assert_match", None, |args| {
            let message = split_message("assert_match", args, 2)?;
            let (Value::String(text), Value::String(pattern)) = (&args[0], &args[1]) else {
                return Err(format!(
                    "assert_match expects a String and a pattern String, got {} and {}",
                    args[0].type_name(),
                    args[1].type_name()
                ));
            };
            let regex = crate::regex_cache::get_regex(pattern)
                .map_err(|e| format!("assert_match: invalid pattern /{pattern}/: {e}"))?;
            if regex.is_match(text) {
                pass()
            } else {
                Err(with_message(
                    message,
                    format!("expected {} to match /{}/", fmt_value(&args[0]), pattern),
                ))
            }
        })),
    );

    env.define(
        "assert_contains".to_string(),
        Value::NativeFunction(NativeFunction::new("assert_contains", None, |args| {
            let message = split_message("assert_contains", args, 2)?;
            let contains = match (&args[0], &args[1]) {
                (Value::Array(items), needle) => items.borrow().contains(needle),
                (Value::String(text), Value::String(needle)) => text.contains(needle.as_ref()),
                (Value::String(_), other) => {
                    return Err(format!(
                        "assert_contains on a String expects a String to look for, got {}",
                        other.type_name()
                    ))
                }
                (other, _) => {
                    return Err(format!(
                        "assert_contains expects an Array or a String, got {}",
                        other.type_name()
                    ))
                }
            };
            if contains {
                pass()
            } else {
                Err(with_message(
                    message,
                    format!(
                        "expected {} to contain {}",
                        fmt_value(&args[0]),
                        fmt_value(&args[1])
                    ),
                ))
            }
        })),
    );

    env.define(
        "assert_hash_has_key".to_string(),
        Value::NativeFunction(NativeFunction::new("assert_hash_has_key", None, |args| {
            let message = split_message("assert_hash_has_key", args, 2)?;
            let Value::Hash(hash) = &args[0] else {
                return Err(format!(
                    "assert_hash_has_key expects a Hash, got {}",
                    args[0].type_name()
                ));
            };
            let found =
                HashKey::from_value(&args[1]).is_some_and(|key| hash.borrow().contains_key(&key));
            if found {
                pass()
            } else {
                Err(with_message(
                    message,
                    format!(
                        "expected {} to have key {}",
                        fmt_value(&args[0]),
                        fmt_value(&args[1])
                    ),
                ))
            }
        })),
    );

    env.define(
        "assert_json".to_string(),
        Value::NativeFunction(NativeFunction::new("assert_json", None, |args| {
            let message = split_message("assert_json", args, 1)?;
            let Value::String(text) = &args[0] else {
                return Err(format!(
                    "assert_json expects a String, got {}",
                    args[0].type_name()
                ));
            };
            match serde_json::from_str::<serde_json::Value>(text) {
                Ok(_) => pass(),
                Err(e) => Err(with_message(
                    message,
                    format!("expected valid JSON, got {} ({e})", fmt_value(&args[0])),
                )),
            }
        })),
    );

    // `assert_raises(fn() { ... })` / `assert_raises("part of the message") do ... end`.
    // The block has to run with `&mut Interpreter` (or the VM), so the real
    // work is in the call interceptors; this placeholder only explains misuse.
    env.define(
        "assert_raises".to_string(),
        Value::NativeFunction(NativeFunction::new("assert_raises", None, |_args| {
            Err(
                "assert_raises expects a block: assert_raises() do ... end, \
assert_raises(\"part of the message\") do ... end, or assert_raises(fn() { ... })"
                    .to_string(),
            )
        })),
    );

    // Fails when the request that produced `res` triggered an N+1 query pattern
    // (the same AQL template fired >= 2x). Uses the exact detection behind the
    // dev-bar N+1 badge. Pass the response from get()/post()/etc.
    env.define(
        "assert_no_n_plus_one".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "assert_no_n_plus_one",
            Some(1),
            |args| {
                let groups = n_plus_one_of(&args[0])?;
                if groups.is_empty() {
                    increment_assertion_count();
                    Ok(Value::Int(1))
                } else {
                    Err(n_plus_one_failure_message(&groups))
                }
            },
        )),
    );

    // Fails when the request left reads uncoalesced: >= 3 distinct read
    // templates, each run once, none inside a `grouped(fn() { ... })` block.
    // Complements assert_no_n_plus_one, which only sees *repeated* templates.
    env.define(
        "assert_no_ungrouped_reads".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "assert_no_ungrouped_reads",
            Some(1),
            |args| {
                let reads = ungrouped_reads_of(&args[0])?;
                if reads.is_empty() {
                    increment_assertion_count();
                    Ok(Value::Int(1))
                } else {
                    Err(ungrouped_reads_failure_message(&reads))
                }
            },
        )),
    );

    // Asserts the exact number of AQL queries the request executed.
    env.define(
        "assert_query_count".to_string(),
        Value::NativeFunction(NativeFunction::new("assert_query_count", Some(2), |args| {
            let actual = query_count_of(&args[0])?;
            let expected = match &args[1] {
                Value::Int(n) => *n,
                _ => return Err("assert_query_count expects an Int second argument".to_string()),
            };
            if actual == expected {
                increment_assertion_count();
                Ok(Value::Int(1))
            } else {
                Err(format!(
                    "expected {} quer{} but {} ran",
                    expected,
                    if expected == 1 { "y" } else { "ies" },
                    actual
                ))
            }
        })),
    );

    // Asserts the request executed no more than `max` AQL queries. Friendlier
    // than assert_query_count for endpoints whose baseline count can shift.
    env.define(
        "assert_max_queries".to_string(),
        Value::NativeFunction(NativeFunction::new("assert_max_queries", Some(2), |args| {
            let actual = query_count_of(&args[0])?;
            let max = match &args[1] {
                Value::Int(n) => *n,
                _ => return Err("assert_max_queries expects an Int second argument".to_string()),
            };
            if actual <= max {
                increment_assertion_count();
                Ok(Value::Int(1))
            } else {
                Err(format!(
                    "expected at most {} quer{} but {} ran",
                    max,
                    if max == 1 { "y" } else { "ies" },
                    actual
                ))
            }
        })),
    );
}

/// Split the evaluated arguments of `assert_raises` into the expected message
/// fragment and the block. `None` when the call is not one of the two shapes,
/// so the caller falls through to the placeholder and its usage error.
pub(crate) fn assert_raises_args(mut args: Vec<Value>) -> Option<(Option<String>, Value)> {
    let callable = |value: &Value| {
        matches!(
            value,
            Value::Function(_) | Value::NativeFunction(_) | Value::VmClosure(_)
        )
    };
    match args.len() {
        1 if callable(&args[0]) => args.pop().map(|block| (None, block)),
        2 if callable(&args[1]) => {
            let block = args.pop()?;
            match args.pop()? {
                Value::String(fragment) => Some((Some(fragment.to_string()), block)),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Judge what an `assert_raises` block did.
///
/// The error it was expected to raise passes (and is returned as its message,
/// so a spec can assert more about it); a block that returns normally fails.
/// A failed assertion, a `skip`/`pending`, or an error the runtime must route
/// rather than catch (breakpoint, engine fallback) is not "the error the block
/// raised": it propagates untouched, or `assert_raises` would turn a failing
/// spec inside the block into a passing one.
pub(crate) fn judge_raised(
    outcome: Result<Value, RuntimeError>,
    fragment: Option<&str>,
    span: Span,
) -> Result<Value, RuntimeError> {
    let failure = |message: String| RuntimeError::General {
        message: assertion_failure(message),
        span,
    };
    let error = match outcome {
        Ok(_) => {
            return Err(failure(match fragment {
                Some(fragment) => format!(
                    "expected the block to raise an error containing {:?}, but it raised nothing",
                    fragment
                ),
                None => "expected the block to raise, but it raised nothing".to_string(),
            }))
        }
        Err(error) => error,
    };
    if assertion_failed()
        || crate::interpreter::builtins::test_dsl::not_run_marked()
        || error.is_breakpoint()
        || error.is_engine_fallback()
    {
        return Err(error);
    }
    let message = match &error {
        RuntimeError::Thrown { value, .. } => crate::error::render_thrown(value),
        other => other.catchable_message(),
    };
    if let Some(fragment) = fragment {
        if !message.contains(fragment) {
            return Err(failure(format!(
                "expected an error containing {:?}, got {:?}",
                fragment, message
            )));
        }
    }
    increment_assertion_count();
    Ok(Value::String(message.into()))
}

/// Read the AQL query count off a response hash (the `query_count` key set by
/// the test-runner server), or accept a bare Int for direct assertions.
fn query_count_of(value: &Value) -> Result<i64, String> {
    match value {
        Value::Int(n) => Ok(*n),
        Value::Hash(h) => match h.borrow().get(&HashKey::String("query_count".into())) {
            Some(Value::Int(n)) => Ok(*n),
            _ => Err(NO_INSTRUMENTATION.to_string()),
        },
        _ => Err("expected a response hash (from get()/post()) or an Int".to_string()),
    }
}

/// Read the N+1 groups off a response hash's `n_plus_one` array as
/// `(template, count)` pairs.
///
/// `pub(crate)` so the `--fail-on-n1` guard in `request_helpers` runs the
/// exact same detection as the `assert_no_n_plus_one` assertion.
pub(crate) fn n_plus_one_of(value: &Value) -> Result<Vec<(String, i64)>, String> {
    let hash = match value {
        Value::Hash(h) => h,
        _ => {
            return Err(
                "assert_no_n_plus_one expects a response hash (from get()/post())".to_string(),
            )
        }
    };
    let borrowed = hash.borrow();
    let entries = match borrowed.get(&HashKey::String("n_plus_one".into())) {
        Some(Value::Array(arr)) => arr,
        // Instrumented responses always carry `n_plus_one` alongside
        // `query_count`; its absence means the response was never instrumented.
        _ => return Err(NO_INSTRUMENTATION.to_string()),
    };
    let mut groups = Vec::new();
    for entry in entries.borrow().iter() {
        if let Value::Hash(g) = entry {
            let g = g.borrow();
            let template = match g.get(&HashKey::String("query".into())) {
                Some(Value::String(s)) => s.to_string(),
                _ => String::new(),
            };
            let count = match g.get(&HashKey::String("count".into())) {
                Some(Value::Int(n)) => *n,
                _ => 0,
            };
            groups.push((template, count));
        }
    }
    Ok(groups)
}

/// Format the `(template, count)` N+1 groups into the failure message shared
/// by `assert_no_n_plus_one` and the `--fail-on-n1` global guard, so both
/// report an N+1 identically.
pub(crate) fn n_plus_one_failure_message(groups: &[(String, i64)]) -> String {
    let detail = groups
        .iter()
        .map(|(template, count)| format!("  {}x  {}", count, template))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "N+1 detected: {} template(s) fired in a loop (batch with `FILTER doc.field IN @ids`):\n{}",
        groups.len(),
        detail
    )
}

/// Read the coalescing candidates off a response hash's `ungrouped_reads` array.
///
/// Each entry is `{query: <template>}` — no count, because every one of these
/// ran exactly once (that is what makes them invisible to the N+1 scan).
fn ungrouped_reads_of(value: &Value) -> Result<Vec<String>, String> {
    let hash = match value {
        Value::Hash(h) => h,
        _ => {
            return Err(
                "assert_no_ungrouped_reads expects a response hash (from get()/post())".to_string(),
            )
        }
    };
    let borrowed = hash.borrow();
    let entries = match borrowed.get(&HashKey::String("ungrouped_reads".into())) {
        Some(Value::Array(arr)) => arr,
        _ => return Err(NO_INSTRUMENTATION.to_string()),
    };
    let mut reads = Vec::new();
    for entry in entries.borrow().iter() {
        if let Value::Hash(g) = entry {
            if let Some(Value::String(s)) = g.borrow().get(&HashKey::String("query".into())) {
                reads.push(s.to_string());
            }
        }
    }
    Ok(reads)
}

/// Failure message for `assert_no_ungrouped_reads`. States the precondition the
/// runtime cannot verify — dependent reads genuinely cannot share a round-trip —
/// so a spec author can tell a real finding from an unavoidable one.
fn ungrouped_reads_failure_message(reads: &[String]) -> String {
    let detail = reads
        .iter()
        .map(|template| format!("  {}", template))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "{} reads each cost a round-trip outside any `grouped` block. If they do not \
depend on each other, wrap them in `grouped(fn() {{ ... }})` to ship them as one \
request:\n{}",
        reads.len(),
        detail
    )
}

const NO_INSTRUMENTATION: &str =
    "response has no query instrumentation — run request specs via `soli test` \
(the test server runs in --dev, which records the AQL query log)";

/// The assertions counted so far in the running file, without resetting them —
/// the runner reads it around each test to find tests that asserted nothing.
pub fn assertion_count() -> i64 {
    ASSERTION_COUNT.with(|count| *count.borrow())
}

pub fn get_and_reset_assertion_count() -> i64 {
    ASSERTION_COUNT.with(|count| {
        let result = *count.borrow();
        *count.borrow_mut() = 0;
        result
    })
}

/// Count one assertion.
///
/// Two counters, on purpose. The thread-local is per file and is drained by
/// `get_and_reset_assertion_count` when the file ends — that is the number the
/// report prints. The global is per suite and readable from any thread, which
/// is what lets the runner's progress bar move while a file is still running
/// rather than jumping once at the end.
pub fn increment_assertion_count() {
    ASSERTION_COUNT.with(|count| {
        *count.borrow_mut() += 1;
    });
    crate::interpreter::builtins::test_progress::record_assertion();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::value::HashPairs;

    /// Build a response-shaped hash like the test client produces, with the
    /// given query count and N+1 groups (`(template, count)`).
    fn response(query_count: i64, n1: &[(&str, i64)]) -> Value {
        let mut pairs = HashPairs::default();
        pairs.insert(
            HashKey::String("query_count".into()),
            Value::Int(query_count),
        );
        let groups: Vec<Value> = n1
            .iter()
            .map(|(template, count)| {
                let mut g = HashPairs::default();
                g.insert(
                    HashKey::String("query".into()),
                    Value::String((*template).into()),
                );
                g.insert(HashKey::String("count".into()), Value::Int(*count));
                Value::Hash(Rc::new(RefCell::new(g)))
            })
            .collect();
        pairs.insert(
            HashKey::String("n_plus_one".into()),
            Value::Array(Rc::new(RefCell::new(groups))),
        );
        Value::Hash(Rc::new(RefCell::new(pairs)))
    }

    /// Pull a registered assertion builtin out of a fresh environment.
    fn builtin(name: &str) -> crate::interpreter::value::NativeFn {
        let mut env = Environment::new();
        register_assertions(&mut env);
        match env.get(name) {
            Some(Value::NativeFunction(nf)) => nf.func.clone(),
            _ => panic!("{name} not registered"),
        }
    }

    /// Attach an `ungrouped_reads` array to a response hash built by [`response`].
    fn with_ungrouped(response: Value, reads: &[&str]) -> Value {
        let entries: Vec<Value> = reads
            .iter()
            .map(|template| {
                let mut g = HashPairs::default();
                g.insert(
                    HashKey::String("query".into()),
                    Value::String((*template).into()),
                );
                Value::Hash(Rc::new(RefCell::new(g)))
            })
            .collect();
        if let Value::Hash(h) = &response {
            h.borrow_mut().insert(
                HashKey::String("ungrouped_reads".into()),
                Value::Array(Rc::new(RefCell::new(entries))),
            );
        }
        response
    }

    #[test]
    fn no_ungrouped_reads_passes_when_clean() {
        let f = builtin("assert_no_ungrouped_reads");
        assert!(f(&[with_ungrouped(response(3, &[]), &[])]).is_ok());
    }

    #[test]
    fn no_ungrouped_reads_fails_and_states_the_precondition() {
        let f = builtin("assert_no_ungrouped_reads");
        let err = f(&[with_ungrouped(
            response(3, &[]),
            &[
                "FOR doc IN posts RETURN doc",
                "FOR doc IN accounts RETURN doc",
                "FOR doc IN tags RETURN doc",
            ],
        )])
        .unwrap_err();
        assert!(err.contains("3 reads"), "message was: {err}");
        assert!(err.contains("grouped(fn() { ... })"), "message was: {err}");
        // The runtime cannot prove independence, so the message must not claim
        // a defect outright — a dependent chain is a legitimate false positive.
        assert!(
            err.contains("do not depend on each other"),
            "message should state the precondition: {err}"
        );
        assert!(err.contains("FOR doc IN accounts RETURN doc"));
    }

    #[test]
    fn no_ungrouped_reads_rejects_an_uninstrumented_response() {
        let f = builtin("assert_no_ungrouped_reads");
        // `response()` alone has no `ungrouped_reads` key — same contract as
        // assert_no_n_plus_one: say the response was never instrumented rather
        // than silently passing.
        let err = f(&[response(3, &[])]).unwrap_err();
        assert!(err.contains("no query instrumentation"), "message: {err}");
    }

    #[test]
    fn no_n_plus_one_passes_when_clean() {
        let f = builtin("assert_no_n_plus_one");
        assert!(f(&[response(3, &[])]).is_ok());
    }

    #[test]
    fn no_n_plus_one_fails_and_names_the_template() {
        let f = builtin("assert_no_n_plus_one");
        let err = f(&[response(
            6,
            &[("FOR d IN posts FILTER d._key == @k RETURN d", 5)],
        )])
        .unwrap_err();
        assert!(err.contains("N+1 detected"), "message was: {err}");
        assert!(
            err.contains("5x"),
            "message should include the count: {err}"
        );
        assert!(
            err.contains("FILTER d._key == @k"),
            "message should quote the template: {err}"
        );
    }

    #[test]
    fn query_count_exact_match() {
        let f = builtin("assert_query_count");
        assert!(f(&[response(3, &[]), Value::Int(3)]).is_ok());
        let err = f(&[response(3, &[]), Value::Int(1)]).unwrap_err();
        assert!(
            err.contains("expected 1 query but 3 ran"),
            "message was: {err}"
        );
    }

    #[test]
    fn query_count_accepts_bare_int() {
        let f = builtin("assert_query_count");
        assert!(f(&[Value::Int(4), Value::Int(4)]).is_ok());
    }

    #[test]
    fn max_queries_bound() {
        let f = builtin("assert_max_queries");
        assert!(f(&[response(3, &[]), Value::Int(5)]).is_ok());
        assert!(f(&[response(3, &[]), Value::Int(3)]).is_ok());
        let err = f(&[response(7, &[]), Value::Int(5)]).unwrap_err();
        assert!(
            err.contains("at most 5 queries but 7 ran"),
            "message was: {err}"
        );
    }

    #[test]
    fn fail_on_n1_guard_shares_detection_and_message() {
        // The `--fail-on-n1` guard (in request_helpers) reads the response via
        // `n_plus_one_of` and formats with `n_plus_one_failure_message` — the
        // exact pair `assert_no_n_plus_one` uses. A dirty response yields
        // groups + the shared message; a clean one yields no groups so the
        // guard stays quiet.
        let dirty = response(6, &[("FOR d IN comments FILTER d.post == @k RETURN d", 3)]);
        let groups = n_plus_one_of(&dirty).expect("instrumented response");
        assert_eq!(groups.len(), 1);
        let msg = n_plus_one_failure_message(&groups);
        assert!(msg.contains("N+1 detected"), "message was: {msg}");
        assert!(
            msg.contains("3x"),
            "message should include the count: {msg}"
        );

        let clean = response(2, &[]);
        assert!(
            n_plus_one_of(&clean)
                .expect("instrumented response")
                .is_empty(),
            "a clean response must produce no N+1 groups"
        );
    }

    #[test]
    fn missing_instrumentation_is_a_clear_error() {
        // A response with no query_count/n_plus_one keys (e.g. not a request
        // spec, or a non-dev server) should explain itself, not pass silently.
        let bare = Value::Hash(Rc::new(RefCell::new(HashPairs::default())));
        let no_n1 = builtin("assert_no_n_plus_one");
        assert!(no_n1(std::slice::from_ref(&bare))
            .unwrap_err()
            .contains("no query instrumentation"));
        let count = builtin("assert_query_count");
        assert!(count(&[bare, Value::Int(0)])
            .unwrap_err()
            .contains("no query instrumentation"));
    }
}
