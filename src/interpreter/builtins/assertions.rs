//! Test assertions for the Soli test DSL.

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{HashKey, NativeFunction, Value};
use std::cell::RefCell;
use std::rc::Rc;

thread_local! {
    static ASSERTION_COUNT: Rc<RefCell<i64>> = Rc::new(RefCell::new(0));
}

pub fn register_assertions(env: &mut Environment) {
    env.define(
        "assert".to_string(),
        Value::NativeFunction(NativeFunction::new("assert", Some(1), |args| {
            match &args[0] {
                Value::Bool(true) => {
                    ASSERTION_COUNT.with(|count| {
                        *count.borrow_mut() += 1;
                    });
                    Ok(Value::Int(1))
                }
                Value::Bool(false) => Err("assertion failed".to_string()),
                _ => Err("assert expects boolean".to_string()),
            }
        })),
    );

    env.define(
        "assert_not".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "assert_not",
            Some(1),
            |args| match &args[0] {
                Value::Bool(false) => {
                    ASSERTION_COUNT.with(|count| {
                        *count.borrow_mut() += 1;
                    });
                    Ok(Value::Int(1))
                }
                Value::Bool(true) => Err("assertion failed".to_string()),
                _ => Err("assert_not expects boolean".to_string()),
            },
        )),
    );

    env.define(
        "assert_eq".to_string(),
        Value::NativeFunction(NativeFunction::new("assert_eq", Some(2), |args| {
            if args[0] == args[1] {
                ASSERTION_COUNT.with(|count| {
                    *count.borrow_mut() += 1;
                });
                Ok(Value::Int(1))
            } else {
                Err("values not equal".to_string())
            }
        })),
    );

    env.define(
        "assert_ne".to_string(),
        Value::NativeFunction(NativeFunction::new("assert_ne", Some(2), |args| {
            if args[0] != args[1] {
                ASSERTION_COUNT.with(|count| {
                    *count.borrow_mut() += 1;
                });
                Ok(Value::Int(1))
            } else {
                Err("values should not be equal".to_string())
            }
        })),
    );

    env.define(
        "assert_null".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "assert_null",
            Some(1),
            |args| match &args[0] {
                Value::Null => {
                    ASSERTION_COUNT.with(|count| {
                        *count.borrow_mut() += 1;
                    });
                    Ok(Value::Int(1))
                }
                _ => Err("expected null".to_string()),
            },
        )),
    );

    env.define(
        "assert_not_null".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "assert_not_null",
            Some(1),
            |args| match &args[0] {
                Value::Null => Err("expected non-null".to_string()),
                _ => {
                    ASSERTION_COUNT.with(|count| {
                        *count.borrow_mut() += 1;
                    });
                    Ok(Value::Int(1))
                }
            },
        )),
    );

    env.define(
        "assert_gt".to_string(),
        Value::NativeFunction(NativeFunction::new("assert_gt", Some(2), |args| {
            match (&args[0], &args[1]) {
                (Value::Int(a), Value::Int(b)) if a > b => {
                    ASSERTION_COUNT.with(|count| {
                        *count.borrow_mut() += 1;
                    });
                    Ok(Value::Int(1))
                }
                (Value::Float(a), Value::Float(b)) if a > b => {
                    ASSERTION_COUNT.with(|count| {
                        *count.borrow_mut() += 1;
                    });
                    Ok(Value::Int(1))
                }
                _ => Err("assert_gt failed".to_string()),
            }
        })),
    );

    env.define(
        "assert_lt".to_string(),
        Value::NativeFunction(NativeFunction::new("assert_lt", Some(2), |args| {
            match (&args[0], &args[1]) {
                (Value::Int(a), Value::Int(b)) if a < b => {
                    ASSERTION_COUNT.with(|count| {
                        *count.borrow_mut() += 1;
                    });
                    Ok(Value::Int(1))
                }
                (Value::Float(a), Value::Float(b)) if a < b => {
                    ASSERTION_COUNT.with(|count| {
                        *count.borrow_mut() += 1;
                    });
                    Ok(Value::Int(1))
                }
                _ => Err("assert_lt failed".to_string()),
            }
        })),
    );

    env.define(
        "assert_match".to_string(),
        Value::NativeFunction(NativeFunction::new("assert_match", Some(2), |args| {
            if let (Value::String(s), Value::String(pattern)) = (&args[0], &args[1]) {
                match crate::regex_cache::get_regex(pattern) {
                    Ok(re) if re.is_match(s) => {
                        ASSERTION_COUNT.with(|count| {
                            *count.borrow_mut() += 1;
                        });
                        Ok(Value::Int(1))
                    }
                    _ => Err("assert_match failed".to_string()),
                }
            } else {
                Err("assert_match expects strings".to_string())
            }
        })),
    );

    env.define(
        "assert_contains".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "assert_contains",
            Some(2),
            |args| match &args[0] {
                Value::Array(arr) if arr.borrow().contains(&args[1]) => {
                    ASSERTION_COUNT.with(|count| {
                        *count.borrow_mut() += 1;
                    });
                    Ok(Value::Int(1))
                }
                Value::String(s) => {
                    if let Value::String(sub) = &args[1] {
                        if s.contains(&**(sub)) {
                            ASSERTION_COUNT.with(|count| {
                                *count.borrow_mut() += 1;
                            });
                            Ok(Value::Int(1))
                        } else {
                            Err("assert_contains failed".to_string())
                        }
                    } else {
                        Err("assert_contains expects string as second argument".to_string())
                    }
                }
                _ => Err("assert_contains failed".to_string()),
            },
        )),
    );

    env.define(
        "assert_hash_has_key".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "assert_hash_has_key",
            Some(2),
            |args| {
                if let Value::Hash(h) = &args[0] {
                    let key = &args[1];
                    let found = if let Some(hash_key) = HashKey::from_value(key) {
                        h.borrow().contains_key(&hash_key)
                    } else {
                        false
                    };
                    if found {
                        ASSERTION_COUNT.with(|count| {
                            *count.borrow_mut() += 1;
                        });
                        Ok(Value::Int(1))
                    } else {
                        Err("hash does not contain key".to_string())
                    }
                } else {
                    Err("assert_hash_has_key expects hash".to_string())
                }
            },
        )),
    );

    env.define(
        "assert_json".to_string(),
        Value::NativeFunction(NativeFunction::new("assert_json", Some(1), |args| {
            if let Value::String(s) = &args[0] {
                match serde_json::from_str::<serde_json::Value>(s) {
                    Ok(_) => {
                        ASSERTION_COUNT.with(|count| {
                            *count.borrow_mut() += 1;
                        });
                        Ok(Value::Int(1))
                    }
                    Err(_) => Err("invalid JSON".to_string()),
                }
            } else {
                Err("assert_json expects string".to_string())
            }
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
                    ASSERTION_COUNT.with(|count| {
                        *count.borrow_mut() += 1;
                    });
                    Ok(Value::Int(1))
                } else {
                    let detail = groups
                        .iter()
                        .map(|(template, count)| format!("  {}x  {}", count, template))
                        .collect::<Vec<_>>()
                        .join("\n");
                    Err(format!(
                        "N+1 detected: {} template(s) fired in a loop (batch with `FILTER doc.field IN @ids`):\n{}",
                        groups.len(),
                        detail
                    ))
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
                ASSERTION_COUNT.with(|count| {
                    *count.borrow_mut() += 1;
                });
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
                ASSERTION_COUNT.with(|count| {
                    *count.borrow_mut() += 1;
                });
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
fn n_plus_one_of(value: &Value) -> Result<Vec<(String, i64)>, String> {
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

const NO_INSTRUMENTATION: &str =
    "response has no query instrumentation — run request specs via `soli test` \
(the test server runs in --dev, which records the AQL query log)";

pub fn get_and_reset_assertion_count() -> i64 {
    ASSERTION_COUNT.with(|count| {
        let result = *count.borrow();
        *count.borrow_mut() = 0;
        result
    })
}

pub fn increment_assertion_count() {
    ASSERTION_COUNT.with(|count| {
        *count.borrow_mut() += 1;
    });
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
    fn builtin(name: &str) -> Rc<dyn Fn(Vec<Value>) -> Result<Value, String>> {
        let mut env = Environment::new();
        register_assertions(&mut env);
        match env.get(name) {
            Some(Value::NativeFunction(nf)) => nf.func.clone(),
            _ => panic!("{name} not registered"),
        }
    }

    #[test]
    fn no_n_plus_one_passes_when_clean() {
        let f = builtin("assert_no_n_plus_one");
        assert!(f(vec![response(3, &[])]).is_ok());
    }

    #[test]
    fn no_n_plus_one_fails_and_names_the_template() {
        let f = builtin("assert_no_n_plus_one");
        let err = f(vec![response(
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
        assert!(f(vec![response(3, &[]), Value::Int(3)]).is_ok());
        let err = f(vec![response(3, &[]), Value::Int(1)]).unwrap_err();
        assert!(
            err.contains("expected 1 query but 3 ran"),
            "message was: {err}"
        );
    }

    #[test]
    fn query_count_accepts_bare_int() {
        let f = builtin("assert_query_count");
        assert!(f(vec![Value::Int(4), Value::Int(4)]).is_ok());
    }

    #[test]
    fn max_queries_bound() {
        let f = builtin("assert_max_queries");
        assert!(f(vec![response(3, &[]), Value::Int(5)]).is_ok());
        assert!(f(vec![response(3, &[]), Value::Int(3)]).is_ok());
        let err = f(vec![response(7, &[]), Value::Int(5)]).unwrap_err();
        assert!(
            err.contains("at most 5 queries but 7 ran"),
            "message was: {err}"
        );
    }

    #[test]
    fn missing_instrumentation_is_a_clear_error() {
        // A response with no query_count/n_plus_one keys (e.g. not a request
        // spec, or a non-dev server) should explain itself, not pass silently.
        let bare = Value::Hash(Rc::new(RefCell::new(HashPairs::default())));
        let no_n1 = builtin("assert_no_n_plus_one");
        assert!(no_n1(vec![bare.clone()])
            .unwrap_err()
            .contains("no query instrumentation"));
        let count = builtin("assert_query_count");
        assert!(count(vec![bare, Value::Int(0)])
            .unwrap_err()
            .contains("no query instrumentation"));
    }
}
