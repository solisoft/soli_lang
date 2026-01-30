//! Test assertions for the Soli test DSL.

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{HashKey, NativeFunction, Value};
use regex::Regex;
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
                match Regex::new(pattern) {
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
                        if s.contains(sub) {
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
}

pub fn get_and_reset_assertion_count() -> i64 {
    ASSERTION_COUNT.with(|count| {
        let result = *count.borrow();
        *count.borrow_mut() = 0;
        result
    })
}
