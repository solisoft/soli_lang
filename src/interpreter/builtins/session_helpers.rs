//! Session and authentication helper functions for Rails-like E2E controller testing.

use std::cell::RefCell;
use std::collections::HashMap;
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
        Value::NativeFunction(NativeFunction::new("as_user", None, |args| {
            if args.is_empty() || args.len() > 2 {
                return Err(format!(
                    "as_user expects 1 or 2 arguments (id, options?), got {}",
                    args.len()
                ));
            }
            let user_id = extract_int(&args[0], "as_user(user_id)")?;
            if args.len() == 1 {
                set_test_user_id(user_id);
                return Ok(Value::Null);
            }
            // 2-arg form: write user_id + options into the server-side session
            // store so the test server's middleware sees them on next request.
            let mut fields = vec![("user_id".to_string(), serde_json::json!(user_id))];
            fields.extend(hash_to_field_pairs(&args[1], "as_user")?);
            write_session_fields(fields, "as_user")?;
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
        "as_role".to_string(),
        Value::NativeFunction(NativeFunction::new("as_role", Some(1), |args| {
            let role = extract_string(&args[0], "as_role(role)")?;
            let sdbql = "FOR u IN users FILTER u.role == @role LIMIT 1 RETURN u".to_string();
            let mut binds: HashMap<String, serde_json::Value> = HashMap::new();
            binds.insert("role".to_string(), serde_json::json!(role));
            let results = super::model::exec_async_query_with_binds(sdbql, Some(binds))
                .map_err(|e| format!("as_role(\"{}\"): users lookup failed: {}", role, e))?;
            if results.is_empty() {
                return Err(format!(
                    "as_role(\"{}\"): no user with role '{}' in the users collection. \
                     Seed one in before_each, or use as_user(id, {{\"role\": \"{}\"}}).",
                    role, role, role
                ));
            }
            let user = &results[0];
            let id = user
                .get("id")
                .or_else(|| user.get("_key"))
                .cloned()
                .ok_or_else(|| format!("as_role(\"{}\"): matching user has no 'id' field", role))?;
            write_session_fields(
                vec![
                    ("user_id".to_string(), id),
                    ("role".to_string(), serde_json::json!(role)),
                ],
                "as_role",
            )?;
            Ok(Value::Null)
        })),
    );

    env.define(
        "sign_in".to_string(),
        Value::NativeFunction(NativeFunction::new("sign_in", None, |args| {
            if args.is_empty() || args.len() > 2 {
                return Err(format!(
                    "sign_in expects 1 or 2 arguments (resource_name, id?), got {}",
                    args.len()
                ));
            }
            let name = extract_string(&args[0], "sign_in(resource_name)")?;
            if name.is_empty() {
                return Err("sign_in: resource_name must not be empty".to_string());
            }
            let session_key = format!("{}_id", name);

            let id_json: serde_json::Value = if args.len() == 2 {
                match &args[1] {
                    Value::Int(n) => serde_json::json!(n),
                    Value::String(s) => serde_json::json!(s),
                    other => {
                        return Err(format!(
                            "sign_in: id must be an integer or string, got {}",
                            other.type_name()
                        ))
                    }
                }
            } else {
                let class_name = pascalize(&name);
                let collection = super::model::class_name_to_collection(&class_name);
                let sdbql = format!("FOR x IN {} LIMIT 1 RETURN x", collection);
                let results =
                    super::model::exec_async_query_with_binds(sdbql, None).map_err(|e| {
                        format!(
                            "sign_in(\"{}\"): lookup in collection '{}' failed: {}. \
                             Is {} a Model with a matching table?",
                            name, collection, e, class_name
                        )
                    })?;
                if results.is_empty() {
                    return Err(format!(
                        "sign_in(\"{}\"): no {} records in DB. Seed one in before_each, \
                         or call sign_in(\"{}\", id) with an explicit id.",
                        name, class_name, name
                    ));
                }
                let rec = &results[0];
                rec.get("id")
                    .or_else(|| rec.get("_key"))
                    .cloned()
                    .ok_or_else(|| {
                        format!("sign_in(\"{}\"): matching record has no 'id' field", name)
                    })?
            };

            write_session_fields(vec![(session_key, id_json)], "sign_in")?;
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
        "with_session".to_string(),
        Value::NativeFunction(NativeFunction::new("with_session", Some(1), |args| {
            let fields = hash_to_field_pairs(&args[0], "with_session")?;
            write_session_fields(fields, "with_session")?;
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

// SEC-040: refuse to write to the live `CURRENT_STORE` outside a test
// context. `register_session_helpers` is gated on `include_test_builtins` at
// the registration point, but `Interpreter::new()` (used by the REPL,
// `soli run`, the dev server boot, jobs, etc.) sets that flag true. Without
// this runtime gate, a Soli-code injection in any of those paths could
// impersonate any user against the live session store. The flag is the same
// one SEC-017 uses for the SSRF bypass — it can only be flipped by
// `soli test` startup or by a test-server child born of a
// `SOLI_INTERNAL_TEST_RUNNER=<uuid-v4>` token.
fn write_session_fields(
    fields: Vec<(String, serde_json::Value)>,
    helper_name: &str,
) -> Result<(), String> {
    if !super::test_server::is_test_runner_process() {
        return Err(format!(
            "{} is a test-only helper; it is not callable outside a test runner context",
            helper_name
        ));
    }
    let store = super::session::get_current_store();
    let session_id = match session_id_from_cookies() {
        Some(id) if !id.is_empty() => store.get_or_create(&id),
        _ => store.create_session(),
    };
    for (key, value) in fields {
        store.set(&session_id, &key, value);
    }
    set_cookie_inner("session_id".to_string(), session_id);
    Ok(())
}

fn hash_to_field_pairs(
    value: &Value,
    helper_name: &str,
) -> Result<Vec<(String, serde_json::Value)>, String> {
    let hash = match value {
        Value::Hash(h) => h.clone(),
        other => {
            return Err(format!(
                "{} expects a hash, got {}",
                helper_name,
                other.type_name()
            ))
        }
    };
    let mut out = Vec::new();
    for (key, val) in hash.borrow().iter() {
        let key_str = match key {
            HashKey::String(s) => s.clone(),
            other => {
                return Err(format!(
                    "{} keys must be strings, got {:?}",
                    helper_name, other
                ))
            }
        };
        let json = crate::interpreter::value::value_to_json(val)
            .map_err(|e| format!("{}: cannot serialize {}: {}", helper_name, key_str, e))?;
        out.push((key_str, json));
    }
    Ok(out)
}

// Convert a snake_case resource name to PascalCase model name.
// "admin" → "Admin", "blog_post" → "BlogPost".
fn pascalize(s: &str) -> String {
    let mut out = String::new();
    let mut capitalize_next = true;
    for c in s.chars() {
        if c == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            out.extend(c.to_uppercase());
            capitalize_next = false;
        } else {
            out.push(c);
        }
    }
    out
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
            let text = crate::interpreter::builtins::http_class::read_capped_text_sync(r)?;
            (code, cookies, text)
        }
        Err(ureq::Error::Status(code, r)) => {
            let cookies: Vec<String> = r.all("Set-Cookie").iter().map(|s| s.to_string()).collect();
            let text = crate::interpreter::builtins::http_class::read_capped_text_sync(r)
                .unwrap_or_default();
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

fn session_id_from_cookies() -> Option<String> {
    let jar = super::request_helpers::current_cookies();
    for pair in jar.split(';') {
        let pair = pair.trim();
        if let Some((k, v)) = pair.split_once('=') {
            if k.trim().eq_ignore_ascii_case("session_id") {
                return Some(v.trim().to_string());
            }
        }
    }
    None
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::value::HashKey;
    use serde_json::json;

    fn call_fn(env: &Environment, name: &str, args: Vec<Value>) -> Result<Value, String> {
        match env.get(name) {
            Some(Value::NativeFunction(f)) => (f.func)(args),
            other => panic!("expected NativeFunction for {name}, got {other:?}"),
        }
    }

    fn fresh_env() -> Environment {
        // SEC-040: `with_session` refuses to operate outside a test
        // runner. The unit tests in this module simulate that context, so
        // flip the same flag the real test runner flips at startup. The
        // flag is process-wide and one-way (no disable), matching the
        // pattern SEC-017 established for the SSRF bypass.
        crate::interpreter::builtins::http_class::enable_ssrf_test_mode();
        clear_cookies_inner();
        clear_test_user();
        let mut env = Environment::new();
        register_session_helpers(&mut env);
        env
    }

    fn make_hash(pairs: Vec<(&str, Value)>) -> Value {
        let mut h: HashPairs = HashPairs::default();
        for (k, v) in pairs {
            h.insert(HashKey::String(k.to_string()), v);
        }
        Value::Hash(Rc::new(RefCell::new(h)))
    }

    /// with_session writes the data into the global session store, sets a
    /// session_id cookie pointing at the new server-side session, and a
    /// follow-up server request reading the cookie sees the data.
    #[test]
    fn with_session_writes_to_store_and_cookie() {
        let env = fresh_env();

        let data = make_hash(vec![
            ("user_id", Value::Int(42)),
            ("role", Value::String("editor".into())),
        ]);
        call_fn(&env, "with_session", vec![data]).unwrap();

        let session_id =
            session_id_from_cookies().expect("with_session must set the session_id cookie");
        assert!(
            !session_id.is_empty(),
            "session_id cookie must be non-empty"
        );

        let store = super::super::session::get_current_store();
        assert_eq!(store.get(&session_id, "user_id"), Some(json!(42)));
        assert_eq!(store.get(&session_id, "role"), Some(json!("editor")));
    }

    /// Calling with_session twice in the same test reuses the cookie's
    /// session_id and merges new keys into the same session — no leak of a
    /// fresh session per call.
    #[test]
    fn with_session_reuses_existing_session_cookie() {
        let env = fresh_env();

        call_fn(
            &env,
            "with_session",
            vec![make_hash(vec![("a", Value::Int(1))])],
        )
        .unwrap();
        let first_id = session_id_from_cookies().unwrap();

        call_fn(
            &env,
            "with_session",
            vec![make_hash(vec![("b", Value::Int(2))])],
        )
        .unwrap();
        let second_id = session_id_from_cookies().unwrap();

        assert_eq!(first_id, second_id, "cookie session id must be reused");
        let store = super::super::session::get_current_store();
        assert_eq!(store.get(&first_id, "a"), Some(json!(1)));
        assert_eq!(store.get(&first_id, "b"), Some(json!(2)));
    }

    #[test]
    fn with_session_rejects_non_hash_argument() {
        let env = fresh_env();
        let err = call_fn(&env, "with_session", vec![Value::Int(7)]).unwrap_err();
        assert!(err.contains("expects a hash"), "got: {err}");
    }

    /// `as_user(id)` (single arg) keeps the legacy thread-local behavior:
    /// it sets `TEST_USER` so `current_user()` returns it, and does NOT touch
    /// the server-side session store or the cookie jar.
    #[test]
    fn as_user_one_arg_sets_thread_local_only() {
        let env = fresh_env();
        call_fn(&env, "as_user", vec![Value::Int(7)]).unwrap();

        // TEST_USER is populated.
        let user = call_fn(&env, "current_user", vec![]).unwrap();
        match user {
            Value::Hash(h) => {
                let pairs = h.borrow();
                let id = pairs.get(&HashKey::String("id".to_string())).cloned();
                assert!(matches!(id, Some(Value::Int(7))), "id should be 7");
            }
            other => panic!("expected hash, got {other:?}"),
        }

        // No session cookie was set, no store write happened.
        assert!(
            session_id_from_cookies().is_none(),
            "single-arg as_user must not set a session cookie"
        );
    }

    /// `as_user(id, {options})` writes `user_id` plus every option key into the
    /// server-side session store and sets a `session_id` cookie pointing at
    /// the new server-side session. It does NOT populate the thread-local
    /// `TEST_USER` — middleware reads from the store on the next request.
    #[test]
    fn as_user_two_arg_writes_to_store() {
        let env = fresh_env();
        let opts = make_hash(vec![
            ("role", Value::String("admin".into())),
            ("tenant", Value::String("acme".into())),
        ]);
        call_fn(&env, "as_user", vec![Value::Int(42), opts]).unwrap();

        let session_id =
            session_id_from_cookies().expect("as_user(id, opts) must set session_id cookie");
        let store = super::super::session::get_current_store();
        assert_eq!(store.get(&session_id, "user_id"), Some(json!(42)));
        assert_eq!(store.get(&session_id, "role"), Some(json!("admin")));
        assert_eq!(store.get(&session_id, "tenant"), Some(json!("acme")));
    }

    #[test]
    fn as_user_rejects_wrong_arity() {
        let env = fresh_env();
        let err = call_fn(&env, "as_user", vec![]).unwrap_err();
        assert!(err.contains("1 or 2 arguments"), "got: {err}");

        let err = call_fn(
            &env,
            "as_user",
            vec![Value::Int(1), Value::Int(2), Value::Int(3)],
        )
        .unwrap_err();
        assert!(err.contains("1 or 2 arguments"), "got: {err}");
    }

    #[test]
    fn as_user_rejects_non_hash_options() {
        let env = fresh_env();
        let err = call_fn(
            &env,
            "as_user",
            vec![Value::Int(1), Value::String("admin".into())],
        )
        .unwrap_err();
        assert!(err.contains("expects a hash"), "got: {err}");
    }

    /// `sign_in(name, id)` with an explicit id writes `{name}_id = id` to the
    /// server-side session store and sets the cookie. No DB lookup happens.
    #[test]
    fn sign_in_with_id_writes_session_key() {
        let env = fresh_env();
        call_fn(
            &env,
            "sign_in",
            vec![Value::String("admin".into()), Value::Int(5)],
        )
        .unwrap();

        let session_id = session_id_from_cookies().expect("sign_in must set the session_id cookie");
        let store = super::super::session::get_current_store();
        assert_eq!(store.get(&session_id, "admin_id"), Some(json!(5)));
        // user_id must NOT have been written — only admin_id.
        assert_eq!(store.get(&session_id, "user_id"), None);
    }

    #[test]
    fn sign_in_supports_string_id() {
        let env = fresh_env();
        call_fn(
            &env,
            "sign_in",
            vec![
                Value::String("staff".into()),
                Value::String("abc-123".into()),
            ],
        )
        .unwrap();
        let session_id = session_id_from_cookies().unwrap();
        let store = super::super::session::get_current_store();
        assert_eq!(store.get(&session_id, "staff_id"), Some(json!("abc-123")));
    }

    #[test]
    fn sign_in_rejects_empty_name() {
        let env = fresh_env();
        let err = call_fn(
            &env,
            "sign_in",
            vec![Value::String("".into()), Value::Int(1)],
        )
        .unwrap_err();
        assert!(err.contains("must not be empty"), "got: {err}");
    }

    #[test]
    fn sign_in_rejects_wrong_arity() {
        let env = fresh_env();
        let err = call_fn(&env, "sign_in", vec![]).unwrap_err();
        assert!(err.contains("1 or 2 arguments"), "got: {err}");
    }

    #[test]
    fn sign_in_rejects_non_string_name() {
        let env = fresh_env();
        let err = call_fn(&env, "sign_in", vec![Value::Int(7), Value::Int(1)]).unwrap_err();
        assert!(err.contains("expects string"), "got: {err}");
    }

    #[test]
    fn sign_in_rejects_non_scalar_id() {
        let env = fresh_env();
        let err = call_fn(
            &env,
            "sign_in",
            vec![
                Value::String("admin".into()),
                Value::Hash(Rc::new(RefCell::new(HashPairs::default()))),
            ],
        )
        .unwrap_err();
        assert!(err.contains("integer or string"), "got: {err}");
    }

    #[test]
    fn pascalize_handles_snake_case() {
        assert_eq!(pascalize("admin"), "Admin");
        assert_eq!(pascalize("user"), "User");
        assert_eq!(pascalize("blog_post"), "BlogPost");
        assert_eq!(pascalize("user_profile"), "UserProfile");
        assert_eq!(pascalize(""), "");
    }
}
