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
    /// SEC-025: authenticate worker → SoliDB calls so a network attacker
    /// (or an unrelated tenant on the same DB) can't read or forge
    /// session documents. At most one of {api_key} or {username+password}
    /// is honored (JWT priority is applied inside `SoliDBClient`).
    api_key: Option<String>,
    username: Option<String>,
    password: Option<String>,
}

impl SolidbSessionStore {
    pub fn new(host: String, database: String) -> Self {
        Self {
            host,
            database,
            collection: "sessions".to_string(),
            max_age: Duration::from_secs(24 * 60 * 60),
            request_counter: AtomicU64::new(0),
            api_key: None,
            username: None,
            password: None,
        }
    }

    pub fn with_collection(mut self, collection: String) -> Self {
        self.collection = collection;
        self
    }

    /// SEC-038: thread `SOLI_SESSION_TTL` into the SoliDB store so the
    /// document-age expiry matches the operator-configured TTL instead
    /// of falling back to 24h.
    pub fn with_max_age(mut self, max_age: Duration) -> Self {
        self.max_age = max_age;
        self
    }

    /// SEC-025: install auth credentials. Pass `(Some(key), None, None)`
    /// for API-key auth or `(None, Some(user), Some(pass))` for basic
    /// auth. Other combinations leave the request unauthenticated and
    /// are intended for loopback / test runs only.
    pub fn with_auth(
        mut self,
        api_key: Option<String>,
        username: Option<String>,
        password: Option<String>,
    ) -> Self {
        self.api_key = api_key;
        self.username = username;
        self.password = password;
        self
    }

    fn create_client(&self) -> Result<SoliDBClient, SoliDBError> {
        Self::build_client(
            &self.host,
            &self.database,
            &self.api_key,
            &self.username,
            &self.password,
        )
    }

    /// Build a client from owned config, so it can be called off the
    /// request thread (where `&self` isn't available) — see `spawn_cleanup`.
    fn build_client(
        host: &str,
        database: &str,
        api_key: &Option<String>,
        username: &Option<String>,
        password: &Option<String>,
    ) -> Result<SoliDBClient, SoliDBError> {
        let mut client = SoliDBClient::connect(host)?;
        client.set_database(database);
        if let Some(key) = api_key {
            client = client.with_api_key(key);
        } else if let (Some(u), Some(p)) = (username, password) {
            client = client.with_basic_auth(u, p);
        }
        Ok(client)
    }

    /// Delete every expired session in one server-side bulk `REMOVE`,
    /// instead of listing up to 1000 docs and firing a separate blocking
    /// HTTP DELETE per expired key. The collection name is operator-set
    /// (never user input), so interpolating it is safe and matches how the
    /// model layer builds SDBQL; the cutoff is a bind var.
    fn purge_expired(client: &SoliDBClient, collection: &str, cutoff_ms: i64) {
        let sdbql = format!(
            "FOR doc IN {collection} FILTER doc.last_accessed < @cutoff REMOVE doc IN {collection}"
        );
        let mut bind_vars = HashMap::new();
        bind_vars.insert("cutoff".to_string(), serde_json::json!(cutoff_ms));
        if let Err(e) = client.query(&sdbql, Some(bind_vars)) {
            eprintln!("Session cleanup failed: {}", e);
        }
    }

    /// Run expired-session GC off the request path. Previously `cleanup`
    /// ran synchronously inside `get_or_create` on every 1000th request,
    /// firing up to 1000 sequential blocking SoliDB round-trips — which
    /// stalled whichever request happened to be the 1000th for seconds
    /// with the CPU otherwise idle. Now it's one bulk query on a detached
    /// thread, so no user request ever waits on GC.
    fn spawn_cleanup(&self) {
        let host = self.host.clone();
        let database = self.database.clone();
        let collection = self.collection.clone();
        let api_key = self.api_key.clone();
        let username = self.username.clone();
        let password = self.password.clone();
        let cutoff_ms = chrono::Utc::now().timestamp_millis() - self.max_age.as_millis() as i64;

        std::thread::spawn(move || {
            match Self::build_client(&host, &database, &api_key, &username, &password) {
                Ok(client) => Self::purge_expired(&client, &collection, cutoff_ms),
                Err(e) => eprintln!("Session cleanup: failed to connect to SoliDB: {}", e),
            }
        });
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
            self.spawn_cleanup();
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
            let cutoff_ms = chrono::Utc::now().timestamp_millis() - self.max_age.as_millis() as i64;
            Self::purge_expired(&client, &self.collection, cutoff_ms);
        }
    }

    fn driver_name(&self) -> &'static str {
        "solidb"
    }

    /// Read-only `RETURN 1` over the pooled connection — keeps the shared
    /// HTTP client's keep-alive to the session host from idling out. See
    /// `session::spawn_session_keep_warm`.
    fn warm_ping(&self) -> Result<(), String> {
        let client = self.create_client().map_err(|e| e.to_string())?;
        // Short per-attempt timeout: the readiness/keep-warm loops retry, so a
        // probe that stalls should fail fast and let the next attempt catch the
        // store once reachable — rather than riding the shared client's ~10s
        // default timeout on a single attempt.
        client
            .ping_with_timeout(Some(std::time::Duration::from_secs(2)))
            .map(|_| ())
            .map_err(|e| e.to_string())
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
