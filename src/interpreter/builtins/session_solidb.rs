use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use uuid::Uuid;

use crate::solidb_http::{SoliDBClient, SoliDBError};

use super::session::SessionStore;

#[derive(Clone, Serialize, Deserialize)]
struct SessionDocument {
    #[serde(rename = "_key")]
    key: String,
    data: HashMap<String, JsonValue>,
    created_at: i64,
    last_accessed: i64,
}

impl SessionDocument {
    fn new(key: String) -> Self {
        let now = chrono::Utc::now().timestamp_millis();
        Self {
            key,
            data: HashMap::new(),
            created_at: now,
            last_accessed: now,
        }
    }

    fn touch(&mut self) {
        self.last_accessed = chrono::Utc::now().timestamp_millis();
    }
}

pub struct SolidbSessionStore {
    host: String,
    database: String,
    collection: String,
    max_age: Duration,
    request_counter: AtomicU64,
}

impl SolidbSessionStore {
    pub fn new(host: String, database: String) -> Self {
        Self {
            host,
            database,
            collection: "sessions".to_string(),
            max_age: Duration::from_secs(24 * 60 * 60),
            request_counter: AtomicU64::new(0),
        }
    }

    pub fn with_collection(mut self, collection: String) -> Self {
        self.collection = collection;
        self
    }

    fn create_client(&self) -> Result<SoliDBClient, SoliDBError> {
        let client = SoliDBClient::connect(&self.host)?;
        let mut client = client;
        client.set_database(&self.database);
        Ok(client)
    }

    fn load_session(&self, session_id: &str) -> Result<Option<SessionDocument>, String> {
        let client = self.create_client().map_err(|e| e.to_string())?;
        let doc = client
            .get(&self.collection, session_id)
            .map_err(|e| e.to_string())?;

        match doc {
            Some(d) => {
                let doc: SessionDocument = serde_json::from_value(d)
                    .map_err(|e| format!("Failed to deserialize session: {}", e))?;
                Ok(Some(doc))
            }
            None => Ok(None),
        }
    }

    fn save_session(&self, session: &SessionDocument) -> Result<(), String> {
        let client = self.create_client().map_err(|e| e.to_string())?;
        let doc_value = serde_json::to_value(session)
            .map_err(|e| format!("Failed to serialize session: {}", e))?;

        client
            .update(&self.collection, &session.key, doc_value, true)
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn insert_session(&self, session: &SessionDocument) -> Result<(), String> {
        let client = self.create_client().map_err(|e| e.to_string())?;
        let doc_value = serde_json::to_value(session)
            .map_err(|e| format!("Failed to serialize session: {}", e))?;

        client
            .insert(&self.collection, Some(&session.key), doc_value)
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

impl SessionStore for SolidbSessionStore {
    fn get_or_create(&self, session_id: &str) -> String {
        let count = self.request_counter.fetch_add(1, Ordering::Relaxed);
        if count.is_multiple_of(1000) {
            self.cleanup();
        }

        if let Ok(Some(session)) = self.load_session(session_id) {
            let now = chrono::Utc::now().timestamp_millis();
            let age = (now - session.last_accessed) as u64;
            if Duration::from_millis(age) < self.max_age {
                return session_id.to_string();
            }
        }

        let new_id = Uuid::new_v4().to_string();
        let session = SessionDocument::new(new_id.clone());
        if let Err(e) = self.insert_session(&session) {
            eprintln!("Failed to create session: {}", e);
        }
        new_id
    }

    fn create_session(&self) -> String {
        let session_id = Uuid::new_v4().to_string();
        let session = SessionDocument::new(session_id.clone());
        if let Err(e) = self.insert_session(&session) {
            eprintln!("Failed to create session: {}", e);
        }
        session_id
    }

    fn get(&self, session_id: &str, key: &str) -> Option<JsonValue> {
        self.load_session(session_id)
            .ok()
            .flatten()
            .and_then(|s| s.data.get(key).cloned())
    }

    fn set(&self, session_id: &str, key: &str, value: JsonValue) {
        if let Ok(Some(mut session)) = self.load_session(session_id) {
            session.touch();
            session.data.insert(key.to_string(), value);
            if let Err(e) = self.save_session(&session) {
                eprintln!("Failed to save session: {}", e);
            }
        }
    }

    fn delete(&self, session_id: &str, key: &str) -> Option<JsonValue> {
        if let Ok(Some(mut session)) = self.load_session(session_id) {
            session.touch();
            let value = session.data.remove(key);
            if let Err(e) = self.save_session(&session) {
                eprintln!("Failed to save session: {}", e);
            }
            value
        } else {
            None
        }
    }

    fn destroy(&self, session_id: &str) {
        if let Ok(client) = self.create_client() {
            if let Err(e) = client.delete(&self.collection, session_id) {
                eprintln!("Failed to destroy session: {}", e);
            }
        }
    }

    fn regenerate(&self, old_id: &str) -> String {
        let old_session = self.load_session(old_id).ok().flatten();
        let new_id = Uuid::new_v4().to_string();

        if let Some(mut session) = old_session {
            session.key = new_id.clone();
            session.touch();
            if let Err(e) = self.insert_session(&session) {
                eprintln!("Failed to create new session during regenerate: {}", e);
            }
        } else {
            let session = SessionDocument::new(new_id.clone());
            if let Err(e) = self.insert_session(&session) {
                eprintln!("Failed to create session during regenerate: {}", e);
            }
        }

        self.destroy(old_id);
        new_id
    }

    fn cleanup(&self) {
        if let Ok(client) = self.create_client() {
            let now = chrono::Utc::now().timestamp_millis();
            let max_age_ms = self.max_age.as_millis() as i64;

            if let Ok(docs) = client.list(&self.collection, 1000, 0) {
                for doc in docs {
                    if let Ok(session) = serde_json::from_value::<SessionDocument>(doc) {
                        let age = now - session.last_accessed;
                        if age > max_age_ms {
                            let _ = client.delete(&self.collection, &session.key);
                        }
                    }
                }
            }
        }
    }

    fn driver_name(&self) -> &'static str {
        "solidb"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_solidb_store_creation() {
        let store = SolidbSessionStore::new("localhost:8080".to_string(), "test_db".to_string());
        assert_eq!(store.driver_name(), "solidb");
    }
}
