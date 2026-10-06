//! Test DSL built-in functions for Soli.

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, HashKey, NativeFunction, Value};
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::collections::HashSet;
use std::rc::Rc;

#[derive(Clone)]
pub struct TestDefinition {
    pub name: String,
    pub body: Value,
}

#[derive(Clone)]
pub struct TestSuite {
    pub name: String,
    pub tests: Vec<TestDefinition>,
    pub before_each: Option<Value>,
    pub after_each: Option<Value>,
    pub before_all: Option<Value>,
    pub after_all: Option<Value>,
    pub nested_suites: Vec<TestSuite>,
    /// Browser viewport declared with `viewport(...)` in the suite body, which
    /// every test in it (and every suite nested in it) starts in. `None` means
    /// the default size.
    pub viewport: Option<crate::interpreter::builtins::browser::Viewport>,
}

thread_local! {
    pub static TEST_SUITES: Rc<RefCell<Vec<TestSuite>>> = Rc::new(RefCell::new(Vec::new()));
}

thread_local! {
    static EXPECTATION_CLASS: Rc<RefCell<Option<Rc<Class>>>> = Rc::new(RefCell::new(None));
}

thread_local! {
    /// Set by `pending()` / `skip()` while a test body runs: how the test was
    /// left, and why (`"pending"` or `"skipped"`, then the optional reason).
    static NOT_RUN: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn mark_not_run(kind: &str, args: &[Value]) -> Result<Value, String> {
    let note = match args.first() {
        Some(Value::String(reason)) if !reason.is_empty() => format!("{kind}: {reason}"),
        Some(Value::Null) | None => kind.to_string(),
        Some(other) => format!("{kind}: {other}"),
    };
    NOT_RUN.with(|n| *n.borrow_mut() = Some(note.clone()));
    Err(note)
}

/// `skip(reason)` from Rust: stop the running test and count it as skipped.
pub fn skip_test(reason: &str) -> Result<Value, String> {
    mark_not_run("skipped", &[Value::String(reason.into())])
}

/// Forget a mark left by a previous test. The runner calls it before each.
pub fn clear_not_run() {
    NOT_RUN.with(|n| *n.borrow_mut() = None);
}

/// Whether `pending()` / `skip()` marked the running test, without clearing it.
pub fn not_run_marked() -> bool {
    NOT_RUN.with(|n| n.borrow().is_some())
}

/// The mark `pending()` / `skip()` left on the test that just ran, if any.
pub fn take_not_run() -> Option<String> {
    NOT_RUN.with(|n| n.borrow_mut().take())
}

// Compact, single-line representation of a Value for assertion error messages.
// Long strings are truncated so a 5KB HTML body doesn't fill the test output.
//
// Everything but a string goes through `Display`, never the derived `Debug`:
// `{:?}` on an instance walks its class, the class's methods and their
// closure environments, which point back at the class — a failed `expect` on
// a model instance overflowed the stack and took the whole test runner down.
// `Display` prints only the instance's fields and is depth-guarded.
pub(crate) fn fmt_value(v: &Value) -> String {
    const MAX_STR: usize = 80;
    const MAX_OTHER: usize = 200;
    match v {
        Value::String(s) => {
            if s.chars().count() <= MAX_STR {
                format!("{:?}", s)
            } else {
                let prefix: String = s.chars().take(MAX_STR).collect();
                format!("{:?}… ({} chars)", prefix, s.len())
            }
        }
        _ => {
            let shown = v.to_string().replace(",\n ", ", ");
            if shown.chars().count() <= MAX_OTHER {
                shown
            } else {
                let prefix: String = shown.chars().take(MAX_OTHER).collect();
                format!("{}…", prefix)
            }
        }
    }
}

/// Mark an expectation failure for `assert_raises`, like the `assert_*` family.
fn failure(message: String) -> String {
    crate::interpreter::builtins::assertions::assertion_failure(message)
}

fn get_actual(args: &[Value]) -> Result<Value, String> {
    if args.is_empty() {
        return Err("Missing self argument".to_string());
    }
    let this = &args[0];

    if let Value::Instance(inst) = this {
        if let Some(actual) = inst.borrow().get("actual") {
            return Ok(actual.clone());
        }
        return Err("expect() instance has no 'actual' field".to_string());
    }
    if let Value::Hash(hash) = this {
        let borrowed = hash.borrow();

        if let Some(actual) = borrowed.get(&HashKey::String("actual".into())) {
            return Ok(actual.clone());
        }
    }
    Err("expect() must be called first".to_string())
}

pub fn register_expectation_class(env: &mut Environment) {
    let mut expectation_native_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    expectation_native_methods.insert(
        "to_be".to_string(),
        Rc::new(NativeFunction::new("Expectation.to_be", Some(1), |args| {
            let actual = get_actual(args)?;
            let expected = &args[1];
            if actual == *expected {
                crate::interpreter::builtins::assertions::increment_assertion_count();
                Ok(Value::Bool(true))
            } else {
                Err(failure(format!(
                    "Expected {} to be {}",
                    fmt_value(&actual),
                    fmt_value(expected)
                )))
            }
        })),
    );

    expectation_native_methods.insert(
        "to_equal".to_string(),
        Rc::new(NativeFunction::new(
            "Expectation.to_equal",
            Some(1),
            |args| {
                let actual = get_actual(args)?;
                let expected = &args[1];
                if actual == *expected {
                    crate::interpreter::builtins::assertions::increment_assertion_count();
                    Ok(Value::Bool(true))
                } else {
                    Err(failure(format!(
                        "Expected {} to equal {}",
                        fmt_value(&actual),
                        fmt_value(expected)
                    )))
                }
            },
        )),
    );

    expectation_native_methods.insert(
        "to_not_be".to_string(),
        Rc::new(NativeFunction::new(
            "Expectation.to_not_be",
            Some(1),
            |args| {
                let actual = get_actual(args)?;
                let expected = &args[1];
                if actual != *expected {
                    crate::interpreter::builtins::assertions::increment_assertion_count();
                    Ok(Value::Bool(true))
                } else {
                    Err(failure(format!(
                        "Expected {} to not be {}",
                        fmt_value(&actual),
                        fmt_value(expected)
                    )))
                }
            },
        )),
    );

    expectation_native_methods.insert(
        "to_not_equal".to_string(),
        Rc::new(NativeFunction::new(
            "Expectation.to_not_equal",
            Some(1),
            |args| {
                let actual = get_actual(args)?;
                let expected = &args[1];
                if actual != *expected {
                    crate::interpreter::builtins::assertions::increment_assertion_count();
                    Ok(Value::Bool(true))
                } else {
                    Err(failure(format!(
                        "Expected {} to not equal {}",
                        fmt_value(&actual),
                        fmt_value(expected)
                    )))
                }
            },
        )),
    );

    expectation_native_methods.insert(
        "to_be_null".to_string(),
        Rc::new(NativeFunction::new(
            "Expectation.to_be_null",
            Some(0),
            |args| {
                let actual = get_actual(args)?;
                if matches!(actual, Value::Null) {
                    crate::interpreter::builtins::assertions::increment_assertion_count();
                    Ok(Value::Bool(true))
                } else {
                    Err(failure(format!(
                        "Expected {} to be null",
                        fmt_value(&actual)
                    )))
                }
            },
        )),
    );

    expectation_native_methods.insert(
        "to_not_be_null".to_string(),
        Rc::new(NativeFunction::new(
            "Expectation.to_not_be_null",
            Some(0),
            |args| {
                let actual = get_actual(args)?;
                if !matches!(actual, Value::Null) {
                    crate::interpreter::builtins::assertions::increment_assertion_count();
                    Ok(Value::Bool(true))
                } else {
                    Err(failure("Expected value to not be null".to_string()))
                }
            },
        )),
    );

    for (matcher, relation, accepts) in [
        (
            "to_be_greater_than",
            "to be greater than",
            (|o| o == Ordering::Greater) as fn(Ordering) -> bool,
        ),
        ("to_be_less_than", "to be less than", |o| {
            o == Ordering::Less
        }),
        ("to_be_greater_than_or_equal", "to be >=", |o| {
            o != Ordering::Less
        }),
        ("to_be_less_than_or_equal", "to be <=", |o| {
            o != Ordering::Greater
        }),
    ] {
        expectation_native_methods.insert(
            matcher.to_string(),
            Rc::new(NativeFunction::new(
                format!("Expectation.{matcher}"),
                Some(1),
                move |args| {
                    let actual = get_actual(args)?;
                    let expected = &args[1];
                    let ordering = crate::interpreter::builtins::assertions::compare_numbers(
                        matcher, &actual, expected,
                    )?;
                    if accepts(ordering) {
                        crate::interpreter::builtins::assertions::increment_assertion_count();
                        Ok(Value::Bool(true))
                    } else {
                        Err(failure(format!(
                            "Expected {} {} {}",
                            fmt_value(&actual),
                            relation,
                            fmt_value(expected)
                        )))
                    }
                },
            )),
        );
    }

    expectation_native_methods.insert(
        "to_have_key".to_string(),
        Rc::new(NativeFunction::new(
            "Expectation.to_have_key",
            Some(1),
            |args| {
                let actual = get_actual(args)?;
                let key = &args[1];
                let Value::Hash(hash) = &actual else {
                    return Err(format!(
                        "to_have_key expects a Hash, got {}",
                        actual.type_name()
                    ));
                };
                let found =
                    HashKey::from_value(key).is_some_and(|key| hash.borrow().contains_key(&key));
                if found {
                    crate::interpreter::builtins::assertions::increment_assertion_count();
                    Ok(Value::Bool(true))
                } else {
                    Err(failure(format!(
                        "Expected {} to have key {}",
                        fmt_value(&actual),
                        fmt_value(key)
                    )))
                }
            },
        )),
    );

    expectation_native_methods.insert(
        "to_contain".to_string(),
        Rc::new(NativeFunction::new(
            "Expectation.to_contain",
            Some(1),
            |args| {
                let actual = get_actual(args)?;
                let expected = &args[1];
                let contains = match &actual {
                    Value::String(s) => {
                        if let Value::String(sub) = expected {
                            s.contains(&**(sub))
                        } else {
                            return Err("to_contain expects string argument".to_string());
                        }
                    }
                    Value::Array(arr) => arr.borrow().contains(expected),
                    Value::Hash(hash) => hash.borrow().values().any(|v| v == expected),
                    _ => return Err("to_contain expects string, array, or hash".to_string()),
                };
                if contains {
                    crate::interpreter::builtins::assertions::increment_assertion_count();
                    Ok(Value::Bool(true))
                } else {
                    Err(failure(format!(
                        "Expected {} to contain {}",
                        fmt_value(&actual),
                        fmt_value(expected)
                    )))
                }
            },
        )),
    );

    expectation_native_methods.insert(
        "to_match".to_string(),
        Rc::new(NativeFunction::new(
            "Expectation.to_match",
            Some(1),
            |args| {
                let actual = get_actual(args)?;
                let expected = &args[1];
                let matches = match (&actual, expected) {
                    (Value::String(s), Value::String(pat)) => s.contains(pat.as_ref()),
                    _ => {
                        return Err("to_match expects string actual and string pattern".to_string())
                    }
                };
                if matches {
                    crate::interpreter::builtins::assertions::increment_assertion_count();
                    Ok(Value::Bool(true))
                } else {
                    Err(failure(format!(
                        "Expected {} to match {}",
                        fmt_value(&actual),
                        fmt_value(expected)
                    )))
                }
            },
        )),
    );

    expectation_native_methods.insert(
        "to_be_valid_json".to_string(),
        Rc::new(NativeFunction::new(
            "Expectation.to_be_valid_json",
            Some(0),
            |args| {
                let actual = get_actual(args)?;
                if let Value::String(ref s) = actual {
                    if serde_json::from_str::<serde_json::Value>(s.as_ref()).is_ok() {
                        crate::interpreter::builtins::assertions::increment_assertion_count();
                        Ok(Value::Bool(true))
                    } else {
                        Err(failure(format!("Expected valid JSON, got: {}", s)))
                    }
                } else {
                    Err("to_be_valid_json expects string".to_string())
                }
            },
        )),
    );

    let expectation_class = Class {
        name: "Expectation".to_string(),
        superclass: None,
        methods: Rc::new(RefCell::new(HashMap::new())),
        static_methods: HashMap::new(),
        native_static_methods: HashMap::new(),
        native_methods: expectation_native_methods,
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        const_fields: HashSet::new(),
        static_const_fields: HashSet::new(),
        all_methods_cache: RefCell::new(None),
        all_native_methods_cache: RefCell::new(None),
        primitive: None,
        vm_methods: Rc::new(RefCell::new(HashMap::default())),
        vm_static_methods: Rc::new(RefCell::new(HashMap::default())),
        model_subclass_memo: std::cell::Cell::new(None),
        is_module: false,
        included_modules: Rc::new(RefCell::new(Vec::new())),
        mixin_static_methods: Rc::new(RefCell::new(HashMap::new())),
        included_hook_stmts: Rc::new(RefCell::new(Vec::new())),
        extended_hook_stmts: Rc::new(RefCell::new(Vec::new())),
        concern_static_methods: Rc::new(RefCell::new(HashMap::new())),
        concern_method_names: Rc::new(RefCell::new(Vec::new())),
        private_members: Rc::new(RefCell::new(HashSet::new())),
        protected_members: Rc::new(RefCell::new(HashSet::new())),
    };

    let expectation_class_rc = Rc::new(expectation_class);

    EXPECTATION_CLASS.with(|cell| {
        *cell.borrow_mut() = Some(expectation_class_rc.clone());
    });

    env.define(
        "Expectation".to_string(),
        Value::Class(expectation_class_rc.clone()),
    );
}

pub fn register_test_builtins(env: &mut Environment) {
    register_expectation_class(env);

    env.define(
        "test".to_string(),
        Value::NativeFunction(NativeFunction::new("test", Some(2), |args| {
            if args.len() >= 2 {
                let test_name = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => return Err("test requires string name".to_string()),
                };
                let test_body = args[1].clone();

                TEST_SUITES.with(|suites| {
                    let mut suites = suites.borrow_mut();
                    if let Some(current) = suites.last_mut() {
                        current.tests.push(TestDefinition {
                            name: test_name.to_string(),
                            body: test_body,
                        });
                    }
                });
            }
            Ok(Value::Null)
        })),
    );

    env.define(
        "describe".to_string(),
        Value::NativeFunction(NativeFunction::new("describe", Some(2), |args| {
            if args.len() >= 2 {
                let suite_name = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => return Err("describe requires string name".to_string()),
                };

                let new_suite = TestSuite {
                    name: suite_name.clone().to_string(),
                    tests: Vec::new(),
                    before_each: None,
                    after_each: None,
                    before_all: None,
                    after_all: None,
                    nested_suites: Vec::new(),
                    viewport: None,
                };

                TEST_SUITES.with(|suites| {
                    suites.borrow_mut().push(new_suite);
                });
            }
            Ok(Value::Null)
        })),
    );

    env.define(
        "context".to_string(),
        Value::NativeFunction(NativeFunction::new("context", Some(2), |args| {
            if args.len() >= 2 {
                let suite_name = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => return Err("context requires string name".to_string()),
                };

                let new_suite = TestSuite {
                    name: suite_name.clone().to_string(),
                    tests: Vec::new(),
                    before_each: None,
                    after_each: None,
                    before_all: None,
                    after_all: None,
                    nested_suites: Vec::new(),
                    viewport: None,
                };

                TEST_SUITES.with(|suites| {
                    suites.borrow_mut().push(new_suite);
                });
            }
            Ok(Value::Null)
        })),
    );

    env.define(
        "before_each".to_string(),
        Value::NativeFunction(NativeFunction::new("before_each", Some(1), |args| {
            if let Some(current) = args.first() {
                TEST_SUITES.with(|suites| {
                    let mut suites = suites.borrow_mut();
                    if let Some(suite) = suites.last_mut() {
                        suite.before_each = Some(current.clone());
                    }
                });
            }
            Ok(Value::Null)
        })),
    );

    env.define(
        "after_each".to_string(),
        Value::NativeFunction(NativeFunction::new("after_each", Some(1), |args| {
            if let Some(current) = args.first() {
                TEST_SUITES.with(|suites| {
                    let mut suites = suites.borrow_mut();
                    if let Some(suite) = suites.last_mut() {
                        suite.after_each = Some(current.clone());
                    }
                });
            }
            Ok(Value::Null)
        })),
    );

    env.define(
        "before_all".to_string(),
        Value::NativeFunction(NativeFunction::new("before_all", Some(1), |args| {
            if let Some(current) = args.first() {
                TEST_SUITES.with(|suites| {
                    let mut suites = suites.borrow_mut();
                    if let Some(suite) = suites.last_mut() {
                        suite.before_all = Some(current.clone());
                    }
                });
            }
            Ok(Value::Null)
        })),
    );

    env.define(
        "after_all".to_string(),
        Value::NativeFunction(NativeFunction::new("after_all", Some(1), |args| {
            if let Some(current) = args.first() {
                TEST_SUITES.with(|suites| {
                    let mut suites = suites.borrow_mut();
                    if let Some(suite) = suites.last_mut() {
                        suite.after_all = Some(current.clone());
                    }
                });
            }
            Ok(Value::Null)
        })),
    );

    // `pending(reason?)` / `skip(reason?)` stop the test body and mark the
    // test as not run. They raise to unwind the body, but the mark is what the
    // runner reads: before it existed the raise was the whole implementation,
    // so a pending test counted as a failure and failed the run.
    env.define(
        "pending".to_string(),
        Value::NativeFunction(NativeFunction::new("pending", None, |args| {
            mark_not_run("pending", args)
        })),
    );

    env.define(
        "skip".to_string(),
        Value::NativeFunction(NativeFunction::new("skip", None, |args| {
            mark_not_run("skipped", args)
        })),
    );

    env.define(
        "it".to_string(),
        Value::NativeFunction(NativeFunction::new("it", Some(2), |args| {
            if args.len() >= 2 {
                let test_name = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => return Err("it requires string name".to_string()),
                };
                let test_body = args[1].clone();

                TEST_SUITES.with(|suites| {
                    let mut suites = suites.borrow_mut();
                    if let Some(current) = suites.last_mut() {
                        current.tests.push(TestDefinition {
                            name: test_name.to_string(),
                            body: test_body,
                        });
                    }
                });
            }
            Ok(Value::Null)
        })),
    );

    env.define(
        "specify".to_string(),
        Value::NativeFunction(NativeFunction::new("specify", Some(2), |args| {
            if args.len() >= 2 {
                let test_name = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => return Err("specify requires string name".to_string()),
                };
                let test_body = args[1].clone();

                TEST_SUITES.with(|suites| {
                    let mut suites = suites.borrow_mut();
                    if let Some(current) = suites.last_mut() {
                        current.tests.push(TestDefinition {
                            name: test_name.to_string(),
                            body: test_body,
                        });
                    }
                });
            }
            Ok(Value::Null)
        })),
    );

    env.define(
        "expect".to_string(),
        Value::NativeFunction(NativeFunction::new("expect", Some(1), |args| {
            if args.is_empty() {
                return Err("expect requires 1 argument".to_string());
            }
            let actual = args[0].clone();

            // Try to create Expectation instance
            let class_rc = EXPECTATION_CLASS.with(|cell| cell.borrow().clone());

            if let Some(class_rc) = class_rc {
                let mut instance = crate::interpreter::value::Instance::new(class_rc.clone());

                instance.set("actual", actual);
                let result = Value::Instance(Rc::new(RefCell::new(instance)));

                return Ok(result);
            }

            Err("Expectation class not initialized".to_string())
        })),
    );
}

pub fn get_and_reset_test_suites() -> Vec<TestSuite> {
    TEST_SUITES.with(|suites| {
        let mut suites = suites.borrow_mut();
        let result = suites.clone();
        suites.clear();
        result
    })
}

pub fn clear_test_suites() {
    TEST_SUITES.with(|suites| {
        suites.borrow_mut().clear();
    });
}
