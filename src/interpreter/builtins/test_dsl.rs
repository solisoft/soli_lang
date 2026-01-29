//! Test DSL built-in functions for Soli.

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};
use std::cell::RefCell;
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

pub fn register_test_builtins(env: &mut Environment) {
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

                // Create new suite
                let new_suite = TestSuite {
                    name: suite_name.clone(),
                    tests: Vec::new(),
                    before_each: None,
                    after_each: None,
                    before_all: None,
                    after_all: None,
                    nested_suites: Vec::new(),
                };

                // Push to stack
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
            Ok(args[0].clone())
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
