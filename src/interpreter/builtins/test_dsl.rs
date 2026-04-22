//! Test DSL built-in functions for Soli.

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, HashKey, NativeFunction, Value};
use std::cell::RefCell;
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
}

thread_local! {
    pub static TEST_SUITES: Rc<RefCell<Vec<TestSuite>>> = Rc::new(RefCell::new(Vec::new()));
}

thread_local! {
    static EXPECTATION_CLASS: Rc<RefCell<Option<Rc<Class>>>> = Rc::new(RefCell::new(None));
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

        if let Some(actual) = borrowed.get(&HashKey::String("actual".to_string())) {
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
            let actual = get_actual(&args)?;
            let expected = &args[1];
            if actual == *expected {
                crate::interpreter::builtins::assertions::increment_assertion_count();
                Ok(Value::Bool(true))
            } else {
                Err(format!("Expected {:?} to be {:?}", actual, expected))
            }
        })),
    );

    expectation_native_methods.insert(
        "to_equal".to_string(),
        Rc::new(NativeFunction::new(
            "Expectation.to_equal",
            Some(1),
            |args| {
                let actual = get_actual(&args)?;
                let expected = &args[1];
                if actual == *expected {
                    crate::interpreter::builtins::assertions::increment_assertion_count();
                    Ok(Value::Bool(true))
                } else {
                    Err(format!("Expected {:?} to equal {:?}", actual, expected))
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
                let actual = get_actual(&args)?;
                let expected = &args[1];
                if actual != *expected {
                    crate::interpreter::builtins::assertions::increment_assertion_count();
                    Ok(Value::Bool(true))
                } else {
                    Err(format!("Expected {:?} to not be {:?}", actual, expected))
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
                let actual = get_actual(&args)?;
                let expected = &args[1];
                if actual != *expected {
                    crate::interpreter::builtins::assertions::increment_assertion_count();
                    Ok(Value::Bool(true))
                } else {
                    Err(format!("Expected {:?} to not equal {:?}", actual, expected))
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
                let actual = get_actual(&args)?;
                if matches!(actual, Value::Null) {
                    crate::interpreter::builtins::assertions::increment_assertion_count();
                    Ok(Value::Bool(true))
                } else {
                    Err(format!("Expected {:?} to be null", actual))
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
                let actual = get_actual(&args)?;
                if !matches!(actual, Value::Null) {
                    crate::interpreter::builtins::assertions::increment_assertion_count();
                    Ok(Value::Bool(true))
                } else {
                    Err("Expected value to not be null".to_string())
                }
            },
        )),
    );

    expectation_native_methods.insert(
        "to_be_greater_than".to_string(),
        Rc::new(NativeFunction::new(
            "Expectation.to_be_greater_than",
            Some(1),
            |args| {
                let actual = get_actual(&args)?;
                let expected = &args[1];
                match (&actual, expected) {
                    (Value::Int(a), Value::Int(b)) => {
                        if a > b {
                            crate::interpreter::builtins::assertions::increment_assertion_count();
                            Ok(Value::Bool(true))
                        } else {
                            Err(format!("Expected {:?} to be greater than {:?}", a, b))
                        }
                    }
                    (Value::Float(a), Value::Float(b)) => {
                        if a > b {
                            crate::interpreter::builtins::assertions::increment_assertion_count();
                            Ok(Value::Bool(true))
                        } else {
                            Err(format!("Expected {:?} to be greater than {:?}", a, b))
                        }
                    }
                    (Value::Int(a), Value::Float(b)) => {
                        if (*a as f64) > *b {
                            crate::interpreter::builtins::assertions::increment_assertion_count();
                            Ok(Value::Bool(true))
                        } else {
                            Err(format!("Expected {:?} to be greater than {:?}", a, b))
                        }
                    }
                    (Value::Float(a), Value::Int(b)) => {
                        if *a > (*b as f64) {
                            crate::interpreter::builtins::assertions::increment_assertion_count();
                            Ok(Value::Bool(true))
                        } else {
                            Err(format!("Expected {:?} to be greater than {:?}", a, b))
                        }
                    }
                    _ => Err("to_be_greater_than expects numbers".to_string()),
                }
            },
        )),
    );

    expectation_native_methods.insert(
        "to_be_less_than".to_string(),
        Rc::new(NativeFunction::new(
            "Expectation.to_be_less_than",
            Some(1),
            |args| {
                let actual = get_actual(&args)?;
                let expected = &args[1];
                match (&actual, expected) {
                    (Value::Int(a), Value::Int(b)) => {
                        if a < b {
                            crate::interpreter::builtins::assertions::increment_assertion_count();
                            Ok(Value::Bool(true))
                        } else {
                            Err(format!("Expected {:?} to be less than {:?}", a, b))
                        }
                    }
                    (Value::Float(a), Value::Float(b)) => {
                        if a < b {
                            crate::interpreter::builtins::assertions::increment_assertion_count();
                            Ok(Value::Bool(true))
                        } else {
                            Err(format!("Expected {:?} to be less than {:?}", a, b))
                        }
                    }
                    _ => Err("to_be_less_than expects numbers".to_string()),
                }
            },
        )),
    );

    expectation_native_methods.insert(
        "to_be_greater_than_or_equal".to_string(),
        Rc::new(NativeFunction::new(
            "Expectation.to_be_greater_than_or_equal",
            Some(1),
            |args| {
                let actual = get_actual(&args)?;
                let expected = &args[1];
                match (&actual, expected) {
                    (Value::Int(a), Value::Int(b)) => {
                        if a >= b {
                            crate::interpreter::builtins::assertions::increment_assertion_count();
                            Ok(Value::Bool(true))
                        } else {
                            Err(format!("Expected {:?} to be >= {:?}", a, b))
                        }
                    }
                    (Value::Float(a), Value::Float(b)) => {
                        if a >= b {
                            crate::interpreter::builtins::assertions::increment_assertion_count();
                            Ok(Value::Bool(true))
                        } else {
                            Err(format!("Expected {:?} to be >= {:?}", a, b))
                        }
                    }
                    _ => Err("to_be_greater_than_or_equal expects numbers".to_string()),
                }
            },
        )),
    );

    expectation_native_methods.insert(
        "to_be_less_than_or_equal".to_string(),
        Rc::new(NativeFunction::new(
            "Expectation.to_be_less_than_or_equal",
            Some(1),
            |args| {
                let actual = get_actual(&args)?;
                let expected = &args[1];
                match (&actual, expected) {
                    (Value::Int(a), Value::Int(b)) => {
                        if a <= b {
                            crate::interpreter::builtins::assertions::increment_assertion_count();
                            Ok(Value::Bool(true))
                        } else {
                            Err(format!("Expected {:?} to be <= {:?}", a, b))
                        }
                    }
                    (Value::Float(a), Value::Float(b)) => {
                        if a <= b {
                            crate::interpreter::builtins::assertions::increment_assertion_count();
                            Ok(Value::Bool(true))
                        } else {
                            Err(format!("Expected {:?} to be <= {:?}", a, b))
                        }
                    }
                    _ => Err("to_be_less_than_or_equal expects numbers".to_string()),
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
                let actual = get_actual(&args)?;
                let expected = &args[1];
                let contains = match &actual {
                    Value::String(s) => {
                        if let Value::String(sub) = expected {
                            s.contains(sub)
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
                    Err(format!("Expected {:?} to contain {:?}", actual, expected))
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
                let actual = get_actual(&args)?;
                let expected = &args[1];
                let matches = match (&actual, expected) {
                    (Value::String(s), Value::String(pat)) => s.contains(pat.as_str()),
                    _ => {
                        return Err("to_match expects string actual and string pattern".to_string())
                    }
                };
                if matches {
                    crate::interpreter::builtins::assertions::increment_assertion_count();
                    Ok(Value::Bool(true))
                } else {
                    Err(format!("Expected {:?} to match {:?}", actual, expected))
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
                let actual = get_actual(&args)?;
                if let Value::String(ref s) = actual {
                    if serde_json::from_str::<serde_json::Value>(s.as_str()).is_ok() {
                        crate::interpreter::builtins::assertions::increment_assertion_count();
                        Ok(Value::Bool(true))
                    } else {
                        Err(format!("Expected valid JSON, got: {}", s))
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
                            name: test_name,
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
                    name: suite_name.clone(),
                    tests: Vec::new(),
                    before_each: None,
                    after_each: None,
                    before_all: None,
                    after_all: None,
                    nested_suites: Vec::new(),
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
                    name: suite_name.clone(),
                    tests: Vec::new(),
                    before_each: None,
                    after_each: None,
                    before_all: None,
                    after_all: None,
                    nested_suites: Vec::new(),
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

    env.define(
        "pending".to_string(),
        Value::NativeFunction(NativeFunction::new("pending", Some(0), |_args| {
            Err("PENDING".to_string())
        })),
    );

    env.define(
        "skip".to_string(),
        Value::NativeFunction(NativeFunction::new("skip", Some(0), |_args| {
            Err("SKIPPED".to_string())
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
                            name: test_name,
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
                            name: test_name,
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

                instance.set("actual".to_string(), actual);
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
