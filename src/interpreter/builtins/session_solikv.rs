use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use uuid::Uuid;

use super::resp::{RespPool, RespValue};
use super::session::SessionStore;

const DEFAULT_TTL_SECONDS: u64 = 86400;

#[derive(Clone, Serialize, Deserialize)]
struct SessionData {
    data: HashMap<String, JsonValue>,
    created_at: i64,
    last_accessed: i64,
}

impl SessionData {
    fn new() -> Self {
        let now = chrono::Utc::now().timestamp_millis();
        Self {
            data: HashMap::new(),
            created_at: now,
            last_accessed: now,
        }
    }

    fn touch(&mut self) {
        self.last_accessed = chrono::Utc::now().timestamp_millis();
    }
}

pub struct SolikvSessionStore {
    pool: RespPool,
    prefix: String,
    ttl: u64,
    request_counter: AtomicU64,
}

impl SolikvSessionStore {
    pub fn new(host: String, port: u16, token: Option<String>) -> Self {
        Self {
            pool: RespPool::new(host, port, token),
            prefix: "soli:session:".to_string(),
            ttl: DEFAULT_TTL_SECONDS,
            request_counter: AtomicU64::new(0),
        }
    }

    fn session_key(&self, session_id: &str) -> String {
        format!("{}{}", self.prefix, session_id)
    }

    fn load_session(&self, session_id: &str) -> Option<SessionData> {
        let key = self.session_key(session_id);
        let val = self.pool.execute(&["GET", &key]).ok()?;

        match val {
            RespValue::BulkString(s) => serde_json::from_str(&s).ok(),
            _ => None,
        }
    }

    fn save_session(&self, session_id: &str, session: &SessionData) -> Result<(), String> {
        let key = self.session_key(session_id);
        let value = serde_json::to_string(session)
            .map_err(|e| format!("Failed to serialize session: {}", e))?;

        let ttl_str = self.ttl.to_string();
        self.pool
            .execute(&["SET", &key, &value, "EX", &ttl_str])
            .map_err(|e| format!("Failed to save session: {}", e))?;
        Ok(())
    }

    fn delete_session(&self, session_id: &str) {
        let key = self.session_key(session_id);
        let _ = self.pool.execute(&["DEL", &key]);
    }
}

impl SessionStore for SolikvSessionStore {
    fn get_or_create(&self, session_id: &str) -> String {
        let count = self.request_counter.fetch_add(1, Ordering::Relaxed);
        if count.is_multiple_of(1000) {
            self.cleanup();
        }

        if let Some(session) = self.load_session(session_id) {
            let now = chrono::Utc::now().timestamp_millis();
            let age_ms = (now - session.last_accessed) as u64;
            if Duration::from_millis(age_ms) < Duration::from_secs(self.ttl) {
                return session_id.to_string();
            }
        }

        let new_id = Uuid::new_v4().to_string();
        let session = SessionData::new();
        if let Err(e) = self.save_session(&new_id, &session) {
            eprintln!("Failed to create session: {}", e);
        }
        new_id
    }

    fn create_session(&self) -> String {
        let session_id = Uuid::new_v4().to_string();
        let session = SessionData::new();
        if let Err(e) = self.save_session(&session_id, &session) {
            eprintln!("Failed to create session: {}", e);
        }
        session_id
    }

    fn get(&self, session_id: &str, key: &str) -> Option<JsonValue> {
        self.load_session(session_id)
            .and_then(|s| s.data.get(key).cloned())
    }

    fn set(&self, session_id: &str, key: &str, value: JsonValue) {
        if let Some(mut session) = self.load_session(session_id) {
            session.touch();
            session.data.insert(key.to_string(), value);
            if let Err(e) = self.save_session(session_id, &session) {
                eprintln!("Failed to save session: {}", e);
            }
        }
    }

    fn delete(&self, session_id: &str, key: &str) -> Option<JsonValue> {
        if let Some(mut session) = self.load_session(session_id) {
            session.touch();
            let value = session.data.remove(key);
            if let Err(e) = self.save_session(session_id, &session) {
                eprintln!("Failed to save session: {}", e);
            }
            value
        } else {
            None
        }
    }

    fn destroy(&self, session_id: &str) {
        self.delete_session(session_id);
    }

    fn regenerate(&self, old_id: &str) -> String {
        let old_session = self.load_session(old_id);
        let new_id = Uuid::new_v4().to_string();

        if let Some(session) = old_session {
            if let Err(e) = self.save_session(&new_id, &session) {
                eprintln!("Failed to create new session during regenerate: {}", e);
            }
        } else {
            let session = SessionData::new();
            if let Err(e) = self.save_session(&new_id, &session) {
                eprintln!("Failed to create session during regenerate: {}", e);
            }
        }

        self.delete_session(old_id);
        new_id
    }

    fn cleanup(&self) {
        // SoliKV handles TTL automatically via EX, so cleanup is a no-op
        // But we can iterate and delete expired sessions if needed
    }

    fn driver_name(&self) -> &'static str {
        "solikv"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_solikv_store_creation() {
        let store = SolikvSessionStore::new("localhost".to_string(), 6380, None);
        assert_eq!(store.driver_name(), "solikv");
    }
}
