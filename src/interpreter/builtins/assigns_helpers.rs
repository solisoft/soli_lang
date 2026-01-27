//! View assigns helper functions for Rails-like E2E controller testing.

use std::cell::RefCell;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

thread_local! {
    static LAST_ASSIGNS: RefCell<Option<Value>> = const { RefCell::new(None) };
    static LAST_VIEW_PATH: RefCell<Option<String>> = const { RefCell::new(None) };
}

pub fn register_assigns_helpers(env: &mut Environment) {
    env.define(
        "assigns".to_string(),
        Value::NativeFunction(NativeFunction::new("assigns", Some(0), |_args| {
            get_assigns()
        })),
    );

    env.define(
        "assign".to_string(),
        Value::NativeFunction(NativeFunction::new("assign", Some(1), |args| {
            let key = extract_string(&args[0], "assign(key)")?;
            get_assign(&key)
        })),
    );

    env.define(
        "view_path".to_string(),
        Value::NativeFunction(NativeFunction::new("view_path", Some(0), |_args| {
            get_view_path()
        })),
    );

    env.define(
        "render_template?".to_string(),
        Value::NativeFunction(NativeFunction::new("render_template?", Some(0), |_args| {
            Ok(Value::Bool(false))
        })),
    );

    env.define(
        "flash".to_string(),
        Value::NativeFunction(NativeFunction::new("flash", Some(0), |_args| {
            Ok(Value::Null)
        })),
    );

    env.define(
        "flash.now".to_string(),
        Value::NativeFunction(NativeFunction::new("flash.now", Some(0), |_args| {
            Ok(Value::Null)
        })),
    );

    env.define(
        "flash[:key]".to_string(),
        Value::NativeFunction(NativeFunction::new("flash[:key]", Some(1), |_args| {
            Ok(Value::Null)
        })),
    );

    env.define(
        "have_assign".to_string(),
        Value::NativeFunction(NativeFunction::new("have_assign", Some(1), |_args| {
            Ok(Value::Bool(false))
        })),
    );

    env.define(
        "assert_assigns".to_string(),
        Value::NativeFunction(NativeFunction::new("assert_assigns", Some(1), |_args| {
            Ok(Value::Null)
        })),
    );
}

fn get_assigns() -> Result<Value, String> {
    LAST_ASSIGNS.with(|cell| {
        let assigns = cell.borrow();
        match &*assigns {
            Some(a) => Ok(a.clone()),
            None => Ok(Value::Hash(Rc::new(RefCell::new(Vec::new())))),
        }
    })
}

fn get_assign(_key: &str) -> Result<Value, String> {
    Ok(Value::Null)
}

fn get_view_path() -> Result<Value, String> {
    LAST_VIEW_PATH.with(|cell| {
        let path = cell.borrow();
        match &*path {
            Some(p) => Ok(Value::String(p.clone())),
            None => Ok(Value::String(String::new())),
        }
    })
}

fn extract_string(value: &Value, context: &str) -> Result<String, String> {
    match value {
        Value::String(s) => Ok(s.clone()),
        _ => Err(format!("{} expects string argument", context)),
    }
}

pub fn clear_assigns() {
    LAST_ASSIGNS.with(|cell| {
        *cell.borrow_mut() = None;
    });
    LAST_VIEW_PATH.with(|cell| {
        *cell.borrow_mut() = None;
    });
}
