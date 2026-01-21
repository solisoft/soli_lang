//! Session management for Solilang.
//!
//! Provides in-memory session storage with cookie-based session IDs.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
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
}

impl InMemorySessionStore {
    fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            max_age: Duration::from_secs(24 * 60 * 60), // 24 hours
        }
    }

    /// Get or create a session by ID.
    fn get_or_create(&self, session_id: &str) -> String {
        let mut sessions = self.sessions.write().unwrap();

        if let Some(session) = sessions.get_mut(session_id) {
            if !session.is_expired(self.max_age) {
                session.touch();
                return session_id.to_string();
            }
            // Session expired, remove it
            sessions.remove(session_id);
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

    /// Clean up expired sessions (should be called periodically).
    #[allow(dead_code)]
    fn cleanup(&self) {
        let mut sessions = self.sessions.write().unwrap();
        sessions.retain(|_, session| !session.is_expired(self.max_age));
    }
}

lazy_static! {
    /// Global session store.
    static ref SESSION_STORE: InMemorySessionStore = InMemorySessionStore::new();
}

/// Thread-local current session ID (set per-request from cookie).
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
pub fn create_session_cookie(session_id: &str) -> String {
    format!(
        "session_id={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}",
        session_id,
        24 * 60 * 60 // 24 hours
    )
}

/// Convert a Soli Value to JSON for storage.
fn value_to_json(value: &Value) -> Result<JsonValue, String> {
    match value {
        Value::Null => Ok(JsonValue::Null),
        Value::Bool(b) => Ok(JsonValue::Bool(*b)),
        Value::Int(n) => Ok(JsonValue::Number((*n).into())),
        Value::Float(n) => serde_json::Number::from_f64(*n)
            .map(JsonValue::Number)
            .ok_or_else(|| "Cannot convert float to JSON".to_string()),
        Value::String(s) => Ok(JsonValue::String(s.clone())),
        Value::Array(arr) => {
            let items: Result<Vec<JsonValue>, String> =
                arr.borrow().iter().map(value_to_json).collect();
            Ok(JsonValue::Array(items?))
        }
        Value::Hash(hash) => {
            let mut map = serde_json::Map::new();
            for (k, v) in hash.borrow().iter() {
                let key = match k {
                    Value::String(s) => s.clone(),
                    _ => format!("{}", k),
                };
                map.insert(key, value_to_json(v)?);
            }
            Ok(JsonValue::Object(map))
        }
        other => Err(format!("Cannot store {} in session", other.type_name())),
    }
}

/// Convert a JSON value back to a Soli Value.
fn json_to_value(json: &JsonValue) -> Value {
    match json {
        JsonValue::Null => Value::Null,
        JsonValue::Bool(b) => Value::Bool(*b),
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else if let Some(f) = n.as_f64() {
                Value::Float(f)
            } else {
                Value::Null
            }
        }
        JsonValue::String(s) => Value::String(s.clone()),
        JsonValue::Array(arr) => {
            let items: Vec<Value> = arr.iter().map(json_to_value).collect();
            Value::Array(Rc::new(RefCell::new(items)))
        }
        JsonValue::Object(obj) => {
            let pairs: Vec<(Value, Value)> = obj
                .iter()
                .map(|(k, v)| (Value::String(k.clone()), json_to_value(v)))
                .collect();
            Value::Hash(Rc::new(RefCell::new(pairs)))
        }
    }
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

            if let Some(id) = get_current_session_id() {
                SESSION_STORE.set(&id, &key, json_value);
            }

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
        Value::NativeFunction(NativeFunction::new("session_regenerate", Some(0), |_args| {
            if let Some(old_id) = get_current_session_id() {
                let new_id = SESSION_STORE.regenerate(&old_id);
                set_current_session_id(Some(new_id.clone()));
                return Ok(Value::String(new_id));
            }
            Ok(Value::Null)
        })),
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
