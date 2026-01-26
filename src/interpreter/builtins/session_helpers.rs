//! Session and authentication helper functions for Rails-like E2E controller testing.

use std::cell::RefCell;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

use super::request_helpers::{
    clear_authorization_inner, clear_cookies_inner, set_authorization_inner, set_cookie_inner,
};

type HashPairs = Vec<(Value, Value)>;

thread_local! {
    static TEST_USER: RefCell<Option<Value>> = RefCell::new(None);
}

/// Register session helper built-in functions.
pub fn register_session_helpers(env: &mut Environment) {
    env.define(
        "as_guest".to_string(),
        Value::NativeFunction(NativeFunction::new("as_guest", Some(0), |_args| {
            clear_auth();
            Ok(Value::Null)
        })),
    );

    env.define(
        "as_user".to_string(),
        Value::NativeFunction(NativeFunction::new("as_user", Some(1), |args| {
            let user_id = extract_int(&args[0], "as_user(user_id)")?;
            set_test_user_id(user_id);
            Ok(Value::Null)
        })),
    );

    env.define(
        "as_admin".to_string(),
        Value::NativeFunction(NativeFunction::new("as_admin", Some(0), |_args| {
            set_test_user_id(1);
            Ok(Value::Null)
        })),
    );

    env.define(
        "with_token".to_string(),
        Value::NativeFunction(NativeFunction::new("with_token", Some(1), |args| {
            let token = extract_string(&args[0], "with_token(token)")?;
            set_authorization_inner(token);
            Ok(Value::Null)
        })),
    );

    env.define(
        "login".to_string(),
        Value::NativeFunction(NativeFunction::new("login", Some(2), |args| {
            let email = extract_string(&args[0], "login(email, password)")?;
            let password = extract_string(&args[1], "login(email, password)")?;
            perform_login(&email, &password)
        })),
    );

    env.define(
        "logout".to_string(),
        Value::NativeFunction(NativeFunction::new("logout", Some(0), |_args| {
            clear_auth();
            Ok(Value::Null)
        })),
    );

    env.define(
        "current_user".to_string(),
        Value::NativeFunction(NativeFunction::new("current_user", Some(0), |_args| {
            get_current_user_value()
        })),
    );

    env.define(
        "signed_in?".to_string(),
        Value::NativeFunction(NativeFunction::new("signed_in?", Some(0), |_args| {
            Ok(Value::Bool(is_signed_in()))
        })),
    );

    env.define(
        "signed_out?".to_string(),
        Value::NativeFunction(NativeFunction::new("signed_out?", Some(0), |_args| {
            Ok(Value::Bool(!is_signed_in()))
        })),
    );

    env.define(
        "create_session".to_string(),
        Value::NativeFunction(NativeFunction::new("create_session", Some(1), |args| {
            let user_id = extract_int(&args[0], "create_session(user_id)")?;
            create_test_session(user_id)
        })),
    );

    env.define(
        "destroy_session".to_string(),
        Value::NativeFunction(NativeFunction::new("destroy_session", Some(0), |_args| {
            destroy_test_session();
            Ok(Value::Null)
        })),
    );
}

fn clear_auth() {
    clear_authorization_inner();
    clear_cookies_inner();
    clear_test_user();
}

fn extract_int(value: &Value, context: &str) -> Result<i64, String> {
    match value {
        Value::Int(n) => Ok(*n),
        _ => Err(format!("{} expects integer argument", context)),
    }
}

fn extract_string(value: &Value, context: &str) -> Result<String, String> {
    match value {
        Value::String(s) => Ok(s.clone()),
        _ => Err(format!("{} expects string argument", context)),
    }
}

fn set_test_user_id(user_id: i64) {
    TEST_USER.with(|cell| {
        let mut user = cell.borrow_mut();
        let pairs: HashPairs = vec![(Value::String("id".to_string()), Value::Int(user_id))];
        *user = Some(Value::Hash(Rc::new(RefCell::new(pairs))));
    });
}

fn clear_test_user() {
    TEST_USER.with(|cell| {
        *cell.borrow_mut() = None;
    });
}

fn get_current_user_value() -> Result<Value, String> {
    TEST_USER.with(|cell| {
        let user = cell.borrow();
        match &*user {
            Some(u) => Ok(u.clone()),
            None => Ok(Value::Null),
        }
    })
}

fn perform_login(email: &str, password: &str) -> Result<Value, String> {
    let login_data = format!(r#"{{"email":"{}","password":"{}"}}"#, email, password);

    let client = reqwest::blocking::Client::new();
    let port = super::test_server::get_test_server_port().ok_or("Test server is not running")?;
    let url = format!("http://127.0.0.1:{}/login", port);

    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .body(login_data)
        .send()
        .map_err(|e| e.to_string())?;

    let status = response.status().as_u16();
    let body = response.text().map_err(|e| e.to_string())?;

    let response_hash: HashPairs = vec![
        (
            Value::String("status".to_string()),
            Value::Int(status as i64),
        ),
        (Value::String("body".to_string()), Value::String(body)),
    ];

    if status == 200 {
        clear_auth();
        set_cookie_inner("session_id".to_string(), "test_session_123".to_string());
        let pairs: HashPairs = vec![(
            Value::String("email".to_string()),
            Value::String(email.to_string()),
        )];
        TEST_USER.with(|cell| {
            *cell.borrow_mut() = Some(Value::Hash(Rc::new(RefCell::new(pairs))));
        });
    }

    Ok(Value::Hash(Rc::new(RefCell::new(response_hash))))
}

fn is_signed_in() -> bool {
    TEST_USER.with(|cell| cell.borrow().is_some())
}

fn create_test_session(user_id: i64) -> Result<Value, String> {
    set_test_user_id(user_id);
    let session_id = format!("session_test_{}", user_id);
    set_cookie_inner("session_id".to_string(), session_id.clone());
    Ok(Value::String(session_id))
}

fn destroy_test_session() {
    clear_auth();
}
