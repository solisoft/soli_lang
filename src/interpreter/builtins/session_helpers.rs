//! Session and authentication helper functions for Rails-like E2E controller testing.

use std::cell::RefCell;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{HashKey, HashPairs, NativeFunction, Value};

use super::request_helpers::{
    clear_authorization_inner, clear_cookies_inner, set_authorization_inner, set_cookie_inner,
};

thread_local! {
    static TEST_USER: RefCell<Option<Value>> = const { RefCell::new(None) };
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
        "signed_in".to_string(),
        Value::NativeFunction(NativeFunction::new("signed_in", Some(0), |_args| {
            Ok(Value::Bool(is_signed_in()))
        })),
    );

    env.define(
        "signed_out".to_string(),
        Value::NativeFunction(NativeFunction::new("signed_out", Some(0), |_args| {
            Ok(Value::Bool(!is_signed_in()))
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
        let mut pairs: HashPairs = HashPairs::default();
        pairs.insert(HashKey::String("id".to_string()), Value::Int(user_id));
        *user = Some(Value::Hash(Rc::new(RefCell::new(pairs))));
    });
}

fn clear_test_user() {
    TEST_USER.with(|cell| {
        *cell.borrow_mut() = None;
    });
}

/// Clear the "signed-in" test-user marker from any module. Used by the HTTP
/// helper when a test hits `/logout` so `signed_in()` reflects the
/// server-side state transition.
pub fn clear_test_user_public() {
    clear_test_user();
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

    let port = super::test_server::get_test_server_port().ok_or("Test server is not running")?;
    let url = format!("http://127.0.0.1:{}/login", port);

    // Login typically returns 302 (redirect) on success — we don't want ureq
    // to follow the redirect because the redirect target usually requires a
    // cookie that hasn't been installed yet. And we need the Set-Cookie
    // header from this exact response.
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(10))
        .redirects(0)
        .build();

    let response = agent
        .post(&url)
        .set("Content-Type", "application/json")
        .send_string(&login_data);

    let (status, set_cookies, body) = match response {
        Ok(r) => {
            let code = r.status();
            let cookies: Vec<String> = r.all("Set-Cookie").iter().map(|s| s.to_string()).collect();
            let text = r.into_string().map_err(|e| e.to_string())?;
            (code, cookies, text)
        }
        Err(ureq::Error::Status(code, r)) => {
            let cookies: Vec<String> = r.all("Set-Cookie").iter().map(|s| s.to_string()).collect();
            let text = r.into_string().unwrap_or_default();
            (code, cookies, text)
        }
        Err(e) => return Err(e.to_string()),
    };

    let mut response_hash: HashPairs = HashPairs::default();
    response_hash.insert(
        HashKey::String("status".to_string()),
        Value::Int(status as i64),
    );
    response_hash.insert(HashKey::String("body".to_string()), Value::String(body));

    // Successful login is a redirect (typically 302 to the dashboard). A
    // 200 means the server re-rendered the login form — usually because
    // credentials were wrong. Don't install cookies or mark TEST_USER in
    // that case, or subsequent requests will think they're logged in.
    let logged_in = (300..400).contains(&status);
    if logged_in {
        clear_cookies_inner();
        clear_auth();
        for raw in &set_cookies {
            // Set-Cookie: name=value; Path=/; ...
            let kv = raw.split(';').next().unwrap_or(raw).trim();
            if let Some((name, value)) = kv.split_once('=') {
                set_cookie_inner(name.trim().to_string(), value.trim().to_string());
            }
        }
        let mut pairs: HashPairs = HashPairs::default();
        pairs.insert(
            HashKey::String("email".to_string()),
            Value::String(email.to_string()),
        );
        TEST_USER.with(|cell| {
            *cell.borrow_mut() = Some(Value::Hash(Rc::new(RefCell::new(pairs))));
        });
    }

    Ok(Value::Hash(Rc::new(RefCell::new(response_hash))))
}

fn is_signed_in() -> bool {
    // Rails-style test helper: consider the session authenticated when the
    // cookie jar contains a session_id value that we haven't cleared.
    // Controllers set session_id on successful login and clear it on logout.
    if TEST_USER.with(|cell| cell.borrow().is_some()) {
        return true;
    }
    let jar = super::request_helpers::current_cookies();
    for pair in jar.split(';') {
        let pair = pair.trim();
        if let Some((k, v)) = pair.split_once('=') {
            if k.trim().eq_ignore_ascii_case("session_id") && !v.trim().is_empty() {
                return true;
            }
        }
    }
    false
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
