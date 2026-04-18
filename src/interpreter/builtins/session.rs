//! Session management for Solilang.
//!
//! Provides in-memory session storage with cookie-based session IDs.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;
use std::time::{Duration, Instant};

use lazy_static::lazy_static;
use serde_json::Value as JsonValue;
use uuid::Uuid;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

/// Session data with expiration.
/// Stores data as JSON values for thread safety.
#[derive(Clone)]
struct Session {
    data: HashMap<String, JsonValue>,
    #[allow(dead_code)]
    created_at: Instant,
    last_accessed: Instant,
}

impl Session {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            data: HashMap::new(),
            created_at: now,
            last_accessed: now,
        }
    }

    fn touch(&mut self) {
        self.last_accessed = Instant::now();
    }

    fn is_expired(&self, max_age: Duration) -> bool {
        self.last_accessed.elapsed() > max_age
    }
}

/// Thread-safe in-memory session store.
pub struct InMemorySessionStore {
    sessions: RwLock<HashMap<String, Session>>,
    /// Session timeout duration (default: 24 hours)
    max_age: Duration,
    /// Request counter for probabilistic cleanup
    request_counter: AtomicU64,
}

impl InMemorySessionStore {
    fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            max_age: Duration::from_secs(24 * 60 * 60), // 24 hours
            request_counter: AtomicU64::new(0),
        }
    }

    /// Get or create a session by ID.
    fn get_or_create(&self, session_id: &str) -> String {
        // Probabilistic cleanup: only run on 1 in 1000 requests to reduce lock contention
        let count = self.request_counter.fetch_add(1, Ordering::Relaxed);
        if count.is_multiple_of(1000) {
            self.cleanup();
        }

        // Fast path: read lock to check if session exists (skip expiration check —
        // 24h max_age is long enough that probabilistic cleanup handles expired sessions)
        {
            let sessions = self.sessions.read().unwrap();
            if sessions.contains_key(session_id) {
                return session_id.to_string();
            }
        }

        // Slow path: write lock to create session
        let mut sessions = self.sessions.write().unwrap();

        // Double-check after acquiring write lock
        if sessions.contains_key(session_id) {
            return session_id.to_string();
        }

        // Create new session
        let new_id = Uuid::new_v4().to_string();
        sessions.insert(new_id.clone(), Session::new());
        new_id
    }

    /// Create a new session and return its ID.
    fn create_session(&self) -> String {
        let mut sessions = self.sessions.write().unwrap();
        let session_id = Uuid::new_v4().to_string();
        sessions.insert(session_id.clone(), Session::new());
        session_id
    }

    /// Get a value from a session.
    fn get(&self, session_id: &str, key: &str) -> Option<JsonValue> {
        let sessions = self.sessions.read().unwrap();
        sessions
            .get(session_id)
            .and_then(|s| s.data.get(key).cloned())
    }

    /// Set a value in a session.
    fn set(&self, session_id: &str, key: &str, value: JsonValue) {
        let mut sessions = self.sessions.write().unwrap();
        if let Some(session) = sessions.get_mut(session_id) {
            session.touch();
            session.data.insert(key.to_string(), value);
        }
    }

    /// Delete a key from a session.
    fn delete(&self, session_id: &str, key: &str) -> Option<JsonValue> {
        let mut sessions = self.sessions.write().unwrap();
        if let Some(session) = sessions.get_mut(session_id) {
            session.touch();
            return session.data.remove(key);
        }
        None
    }

    /// Destroy a session.
    fn destroy(&self, session_id: &str) {
        let mut sessions = self.sessions.write().unwrap();
        sessions.remove(session_id);
    }

    /// Regenerate session ID (for security after login).
    fn regenerate(&self, old_id: &str) -> String {
        let mut sessions = self.sessions.write().unwrap();
        let new_id = Uuid::new_v4().to_string();

        if let Some(session) = sessions.remove(old_id) {
            sessions.insert(new_id.clone(), session);
        } else {
            sessions.insert(new_id.clone(), Session::new());
        }

        new_id
    }

    /// Clean up expired sessions.
    /// Called lazily on each get_or_create() to avoid background threads.
    fn cleanup(&self) {
        let mut sessions = self.sessions.write().unwrap();
        sessions.retain(|_, session| !session.is_expired(self.max_age));
    }
}

lazy_static! {
    /// Global session store.
    static ref SESSION_STORE: InMemorySessionStore = InMemorySessionStore::new();
}

// Thread-local current session ID (set per-request from cookie).
thread_local! {
    static CURRENT_SESSION_ID: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Set the current session ID for this request.
pub fn set_current_session_id(session_id: Option<String>) {
    CURRENT_SESSION_ID.with(|id| {
        *id.borrow_mut() = session_id;
    });
}

/// Get the current session ID for this request.
pub fn get_current_session_id() -> Option<String> {
    CURRENT_SESSION_ID.with(|id| id.borrow().clone())
}

/// Get or create a session for the given cookie value.
/// Returns the session ID to use (may be new if expired or invalid).
pub fn ensure_session(cookie_session_id: Option<&str>) -> String {
    match cookie_session_id {
        Some(id) if !id.is_empty() => SESSION_STORE.get_or_create(id),
        _ => SESSION_STORE.create_session(),
    }
}

/// Extract session ID from Cookie header.
pub fn extract_session_id_from_cookie(cookie_header: Option<&str>) -> Option<String> {
    cookie_header.and_then(|cookies| {
        for cookie in cookies.split(';') {
            let cookie = cookie.trim();
            if let Some(value) = cookie.strip_prefix("session_id=") {
                return Some(value.to_string());
            }
        }
        None
    })
}

/// Create Set-Cookie header value for session.
pub fn create_session_cookie(session_id: &str, secure: bool) -> String {
    let secure_flag = if secure { "; Secure" } else { "" };
    format!(
        "session_id={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}{}",
        session_id,
        24 * 60 * 60, // 24 hours
        secure_flag
    )
}

/// Return a Set-Cookie header value iff the active session ID differs from
/// the one the client sent — i.e. we created a session lazily, the cookie
/// was expired/invalid and `ensure_session` minted a replacement, or the
/// controller called `session_regenerate`. Returns None when no cookie
/// refresh is needed.
pub fn session_cookie_if_changed(
    current: Option<&str>,
    cookie: Option<&str>,
    secure: bool,
) -> Option<String> {
    match current {
        Some(sid) if Some(sid) != cookie => Some(create_session_cookie(sid, secure)),
        _ => None,
    }
}

/// Convert a Soli Value to JSON for storage.
fn value_to_json(value: &Value) -> Result<JsonValue, String> {
    crate::interpreter::value::value_to_json(value)
}

/// Convert a JSON value back to a Soli Value.
fn json_to_value(json: &JsonValue) -> Value {
    crate::interpreter::value::json_to_value_ref(json).unwrap_or(Value::Null)
}

/// Register session builtins in the given environment.
pub fn register_session_builtins(env: &mut Environment) {
    // session_get(key) -> Value or null
    env.define(
        "session_get".to_string(),
        Value::NativeFunction(NativeFunction::new("session_get", Some(1), |args| {
            let key = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "session_get() expects string key, got {}",
                        other.type_name()
                    ))
                }
            };

            let session_id = get_current_session_id();
            match session_id {
                Some(id) => Ok(SESSION_STORE
                    .get(&id, &key)
                    .map(|json| json_to_value(&json))
                    .unwrap_or(Value::Null)),
                None => Ok(Value::Null),
            }
        })),
    );

    // session_set(key, value) -> null
    env.define(
        "session_set".to_string(),
        Value::NativeFunction(NativeFunction::new("session_set", Some(2), |args| {
            let key = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "session_set() expects string key, got {}",
                        other.type_name()
                    ))
                }
            };

            let value = &args[1];
            let json_value = value_to_json(value)?;

            // Lazily create a session on first write so first-time visitors
            // (no Cookie header) still get a persisted session + Set-Cookie.
            let id = match get_current_session_id() {
                Some(id) => id,
                None => {
                    let id = SESSION_STORE.create_session();
                    set_current_session_id(Some(id.clone()));
                    id
                }
            };
            SESSION_STORE.set(&id, &key, json_value);

            Ok(Value::Null)
        })),
    );

    // session_delete(key) -> deleted value or null
    env.define(
        "session_delete".to_string(),
        Value::NativeFunction(NativeFunction::new("session_delete", Some(1), |args| {
            let key = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "session_delete() expects string key, got {}",
                        other.type_name()
                    ))
                }
            };

            if let Some(id) = get_current_session_id() {
                return Ok(SESSION_STORE
                    .delete(&id, &key)
                    .map(|json| json_to_value(&json))
                    .unwrap_or(Value::Null));
            }

            Ok(Value::Null)
        })),
    );

    // session_destroy() -> null
    env.define(
        "session_destroy".to_string(),
        Value::NativeFunction(NativeFunction::new("session_destroy", Some(0), |_args| {
            if let Some(id) = get_current_session_id() {
                SESSION_STORE.destroy(&id);
            }
            Ok(Value::Null)
        })),
    );

    // session_id() -> String
    env.define(
        "session_id".to_string(),
        Value::NativeFunction(NativeFunction::new("session_id", Some(0), |_args| {
            Ok(get_current_session_id()
                .map(Value::String)
                .unwrap_or(Value::Null))
        })),
    );

    // session_regenerate() -> String (new ID)
    env.define(
        "session_regenerate".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "session_regenerate",
            Some(0),
            |_args| {
                let new_id = match get_current_session_id() {
                    Some(old_id) => SESSION_STORE.regenerate(&old_id),
                    None => SESSION_STORE.create_session(),
                };
                set_current_session_id(Some(new_id.clone()));
                Ok(Value::String(new_id))
            },
        )),
    );

    // session_has(key) -> Bool
    env.define(
        "session_has".to_string(),
        Value::NativeFunction(NativeFunction::new("session_has", Some(1), |args| {
            let key = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "session_has() expects string key, got {}",
                        other.type_name()
                    ))
                }
            };

            if let Some(id) = get_current_session_id() {
                return Ok(Value::Bool(SESSION_STORE.get(&id, &key).is_some()));
            }

            Ok(Value::Bool(false))
        })),
    );
}

#[cfg(test)]
mod tests {
    //! End-to-end integration tests for the session layer.
    //!
    //! These exercise the actual native-function closures that the interpreter
    //! invokes, driving the same SESSION_STORE + CURRENT_SESSION_ID thread-local
    //! that a real HTTP request uses. They simulate the request lifecycle
    //! (resolve cookie → run handler → diff IDs for Set-Cookie) without
    //! standing up a full interpreter + router.
    //!
    //! Regression coverage:
    //! - session_set on a request with no Cookie header must persist and
    //!   cause a Set-Cookie to be emitted (previously a silent no-op).
    //! - session_regenerate must emit a Set-Cookie carrying the new ID,
    //!   otherwise the client's cookie points at a deleted session after
    //!   login (previously the new ID was dropped on the floor).
    use super::*;
    use serde_json::json;

    /// Look up a registered native function by name and invoke it.
    fn call_fn(env: &Environment, name: &str, args: Vec<Value>) -> Result<Value, String> {
        match env.get(name) {
            Some(Value::NativeFunction(f)) => (f.func)(args),
            other => panic!("expected NativeFunction for {name}, got {other:?}"),
        }
    }

    fn fresh_env() -> Environment {
        // Reset the thread-local so tests sharing a thread don't leak state.
        set_current_session_id(None);
        let mut env = Environment::new();
        register_session_builtins(&mut env);
        env
    }

    /// First-time visitor: no Cookie header, handler writes to the session.
    /// session_set must create a session on demand, persist the value, and
    /// leave the thread-local pointing at the new ID so finalize_response
    /// can emit Set-Cookie.
    #[test]
    fn session_set_lazily_creates_session_when_no_cookie() {
        let env = fresh_env();
        let cookie_session_id: Option<String> = None;
        set_current_session_id(cookie_session_id.clone());

        call_fn(
            &env,
            "session_set",
            vec![Value::String("user_id".into()), Value::Int(42)],
        )
        .unwrap();

        let current = get_current_session_id().expect("session should be created lazily");
        assert_eq!(
            SESSION_STORE.get(&current, "user_id"),
            Some(json!(42)),
            "value must persist under the newly created session"
        );

        // Simulate finalize_response: Set-Cookie must carry the new ID.
        let cookie = session_cookie_if_changed(Some(&current), cookie_session_id.as_deref(), false)
            .expect("expected Set-Cookie for lazily created session");
        assert!(cookie.contains(&format!("session_id={current}")));
    }

    /// session_regenerate on login rotates the ID, migrates data, and
    /// destroys the old ID. The response must carry a Set-Cookie for
    /// the new ID, or the browser keeps using the deleted cookie.
    #[test]
    fn session_regenerate_migrates_data_and_emits_new_cookie() {
        let env = fresh_env();

        // Prime a session as if an earlier request had created one.
        let old_id = SESSION_STORE.create_session();
        SESSION_STORE.set(&old_id, "flash", json!("hello"));
        set_current_session_id(Some(old_id.clone()));
        let cookie_session_id = Some(old_id.clone());

        // Login-style flow: regenerate, then write user_id.
        let new_id = match call_fn(&env, "session_regenerate", vec![]).unwrap() {
            Value::String(s) => s,
            other => panic!("expected String session id, got {other:?}"),
        };
        assert_ne!(new_id, old_id, "regenerate must mint a new ID");

        call_fn(
            &env,
            "session_set",
            vec![Value::String("user_id".into()), Value::Int(42)],
        )
        .unwrap();

        assert!(
            SESSION_STORE.get(&old_id, "flash").is_none(),
            "old session ID must be destroyed after regenerate"
        );
        assert_eq!(
            SESSION_STORE.get(&new_id, "flash"),
            Some(json!("hello")),
            "data must move from old ID to new ID"
        );
        assert_eq!(SESSION_STORE.get(&new_id, "user_id"), Some(json!(42)));
        assert_eq!(get_current_session_id().as_deref(), Some(new_id.as_str()));

        let cookie = session_cookie_if_changed(
            get_current_session_id().as_deref(),
            cookie_session_id.as_deref(),
            true,
        )
        .expect("expected Set-Cookie carrying the rotated ID");
        assert!(cookie.contains(&format!("session_id={new_id}")));
        assert!(cookie.contains("Secure"), "secure flag must propagate");
    }

    /// session_regenerate with no prior session (e.g. first-request login)
    /// should still produce a usable, cookie-emitted session.
    #[test]
    fn session_regenerate_creates_session_when_none_active() {
        let env = fresh_env();
        set_current_session_id(None);

        let new_id = match call_fn(&env, "session_regenerate", vec![]).unwrap() {
            Value::String(s) => s,
            other => panic!("expected String, got {other:?}"),
        };
        assert_eq!(get_current_session_id().as_deref(), Some(new_id.as_str()));
        assert!(session_cookie_if_changed(Some(&new_id), None, false).is_some());
    }

    /// Across two simulated requests, a session written on request #1 must
    /// be readable on request #2 when the client echoes the cookie.
    #[test]
    fn session_persists_across_requests_via_cookie() {
        let env = fresh_env();

        // --- Request 1: no cookie, handler writes user_id.
        set_current_session_id(None);
        call_fn(
            &env,
            "session_set",
            vec![Value::String("user_id".into()), Value::Int(42)],
        )
        .unwrap();
        let issued_id = get_current_session_id().expect("request 1 must create a session");
        set_current_session_id(None); // end of request 1

        // --- Request 2: client sends the cookie back.
        let cookie_session_id = Some(issued_id.clone());
        let resolved = ensure_session(cookie_session_id.as_deref());
        assert_eq!(
            resolved, issued_id,
            "ensure_session must reuse an existing cookie ID"
        );
        set_current_session_id(Some(resolved.clone()));

        let got = call_fn(&env, "session_get", vec![Value::String("user_id".into())]).unwrap();
        match got {
            Value::Int(n) => assert_eq!(n, 42),
            other => panic!("expected stored user_id, got {other:?}"),
        }

        // No Set-Cookie on request 2 since the ID didn't change.
        assert!(session_cookie_if_changed(
            get_current_session_id().as_deref(),
            cookie_session_id.as_deref(),
            false,
        )
        .is_none());
    }

    /// ensure_session mints a replacement when the cookie's ID is unknown
    /// (e.g. after a server restart). finalize_response must notice and
    /// refresh the client's cookie.
    #[test]
    fn unknown_cookie_id_triggers_replacement_and_set_cookie() {
        let _env = fresh_env();
        let stale = "00000000-0000-0000-0000-000000000000".to_string();
        let cookie_session_id = Some(stale.clone());

        let resolved = ensure_session(cookie_session_id.as_deref());
        assert_ne!(resolved, stale, "unknown cookie ID must be replaced");
        set_current_session_id(Some(resolved.clone()));

        let cookie =
            session_cookie_if_changed(Some(&resolved), cookie_session_id.as_deref(), false)
                .expect("Set-Cookie must be emitted when the cookie ID was replaced");
        assert!(cookie.contains(&format!("session_id={resolved}")));
    }

    #[test]
    fn session_cookie_if_changed_respects_matches_and_absence() {
        assert!(session_cookie_if_changed(None, None, false).is_none());
        assert!(session_cookie_if_changed(None, Some("a"), false).is_none());
        assert!(session_cookie_if_changed(Some("a"), Some("a"), false).is_none());
        assert!(session_cookie_if_changed(Some("a"), Some("b"), false).is_some());
        assert!(session_cookie_if_changed(Some("a"), None, false).is_some());
    }

    #[test]
    fn extract_session_id_from_cookie_parses_common_shapes() {
        assert_eq!(extract_session_id_from_cookie(None), None);
        assert_eq!(extract_session_id_from_cookie(Some("")), None);
        assert_eq!(
            extract_session_id_from_cookie(Some("session_id=abc")),
            Some("abc".to_string())
        );
        assert_eq!(
            extract_session_id_from_cookie(Some("foo=1; session_id=abc; bar=2")),
            Some("abc".to_string())
        );
        assert_eq!(extract_session_id_from_cookie(Some("foo=1; bar=2")), None);
    }
}
