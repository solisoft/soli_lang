use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use uuid::Uuid;

use super::session::SessionStore;

/// Reject session IDs that could escape the session directory or contain
/// unexpected bytes. UUIDs (the only IDs we generate) consist of hex digits and
/// dashes, so this allowlist passes them through while blocking `/`, `\`, `..`,
/// null bytes, and any other path-shaping characters that could be smuggled in
/// via the `session_id` cookie.
fn is_safe_session_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

#[derive(Clone, Serialize, Deserialize)]
struct SessionFile {
    data: HashMap<String, JsonValue>,
    created_at: u64,
    last_accessed: u64,
}

impl SessionFile {
    fn new() -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        Self {
            data: HashMap::new(),
            created_at: now,
            last_accessed: now,
        }
    }

    fn touch(&mut self) {
        self.last_accessed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
    }

    fn is_expired(&self, max_age_secs: u64) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        now - self.last_accessed > max_age_secs
    }
}

pub struct DiskSessionStore {
    session_dir: PathBuf,
    max_age: Duration,
    request_counter: AtomicU64,
}

impl DiskSessionStore {
    pub fn new(session_dir: PathBuf) -> std::io::Result<Self> {
        fs::create_dir_all(&session_dir)?;
        Ok(Self {
            session_dir,
            max_age: Duration::from_secs(24 * 60 * 60),
            request_counter: AtomicU64::new(0),
        })
    }

    fn session_path(&self, session_id: &str) -> Option<PathBuf> {
        if !is_safe_session_id(session_id) {
            return None;
        }
        Some(self.session_dir.join(format!("{}.json", session_id)))
    }

    fn load_session(&self, session_id: &str) -> Option<SessionFile> {
        let path = self.session_path(session_id)?;
        fs::read_to_string(&path)
            .ok()
            .and_then(|content| serde_json::from_str(&content).ok())
    }

    fn save_session(&self, session_id: &str, session: &SessionFile) -> std::io::Result<()> {
        let path = self.session_path(session_id).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid session id")
        })?;
        let content = serde_json::to_string_pretty(session)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        fs::write(path, content)
    }

    fn delete_session_file(&self, session_id: &str) -> std::io::Result<()> {
        let Some(path) = self.session_path(session_id) else {
            return Ok(());
        };
        if path.exists() {
            fs::remove_file(path)
        } else {
            Ok(())
        }
    }

    fn list_sessions(&self) -> std::io::Result<Vec<String>> {
        let mut ids = Vec::new();
        for entry in fs::read_dir(&self.session_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                if let Some(stem) = path.file_stem() {
                    if let Some(id) = stem.to_str() {
                        ids.push(id.to_string());
                    }
                }
            }
        }
        Ok(ids)
    }
}

impl SessionStore for DiskSessionStore {
    fn get_or_create(&self, session_id: &str) -> String {
        let count = self.request_counter.fetch_add(1, Ordering::Relaxed);
        if count.is_multiple_of(1000) {
            self.cleanup();
        }

        if let Some(session) = self.load_session(session_id) {
            if !session.is_expired(self.max_age.as_secs()) {
                return session_id.to_string();
            }
        }

        let new_id = Uuid::new_v4().to_string();
        let session = SessionFile::new();
        let _ = self.save_session(&new_id, &session);
        new_id
    }

    fn create_session(&self) -> String {
        let session_id = Uuid::new_v4().to_string();
        let session = SessionFile::new();
        let _ = self.save_session(&session_id, &session);
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
            let _ = self.save_session(session_id, &session);
        }
    }

    fn delete(&self, session_id: &str, key: &str) -> Option<JsonValue> {
        if let Some(mut session) = self.load_session(session_id) {
            session.touch();
            let value = session.data.remove(key);
            let _ = self.save_session(session_id, &session);
            value
        } else {
            None
        }
    }

    fn destroy(&self, session_id: &str) {
        let _ = self.delete_session_file(session_id);
    }

    fn regenerate(&self, old_id: &str) -> String {
        let old_session = self.load_session(old_id);
        let new_id = Uuid::new_v4().to_string();

        if let Some(session) = old_session {
            let _ = self.save_session(&new_id, &session);
        } else {
            let session = SessionFile::new();
            let _ = self.save_session(&new_id, &session);
        }

        let _ = self.delete_session_file(old_id);
        new_id
    }

    fn cleanup(&self) {
        if let Ok(ids) = self.list_sessions() {
            for id in ids {
                if let Some(session) = self.load_session(&id) {
                    if session.is_expired(self.max_age.as_secs()) {
                        let _ = self.delete_session_file(&id);
                    }
                }
            }
        }
    }

    fn driver_name(&self) -> &'static str {
        "disk"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_disk_session_store_basic() {
        let dir = tempdir().unwrap();
        let store = DiskSessionStore::new(dir.path().to_path_buf()).unwrap();

        let session_id = store.create_session();
        assert!(!session_id.is_empty());

        store.set(&session_id, "user_id", JsonValue::Number(42.into()));
        assert_eq!(
            store.get(&session_id, "user_id"),
            Some(JsonValue::Number(42.into()))
        );
        assert_eq!(store.get(&session_id, "missing"), None);

        store.delete(&session_id, "user_id");
        assert_eq!(store.get(&session_id, "user_id"), None);

        store.destroy(&session_id);
        assert_eq!(store.get(&session_id, "user_id"), None);
    }

    #[test]
    fn test_disk_session_regenerate() {
        let dir = tempdir().unwrap();
        let store = DiskSessionStore::new(dir.path().to_path_buf()).unwrap();

        let old_id = store.create_session();
        store.set(&old_id, "flash", JsonValue::String("hello".to_string()));

        let new_id = store.regenerate(&old_id);
        assert_ne!(new_id, old_id);
        assert_eq!(
            store.get(&new_id, "flash"),
            Some(JsonValue::String("hello".to_string()))
        );
        assert_eq!(store.get(&old_id, "flash"), None);
    }

    #[test]
    fn test_disk_session_get_or_create() {
        let dir = tempdir().unwrap();
        let store = DiskSessionStore::new(dir.path().to_path_buf()).unwrap();

        let new_id = store.get_or_create("nonexistent");
        assert!(!new_id.is_empty());
        assert!(store.load_session(&new_id).is_some());

        let same_id = store.get_or_create(&new_id);
        assert_eq!(same_id, new_id);
    }

    #[test]
    fn test_disk_session_multiple_values() {
        let dir = tempdir().unwrap();
        let store = DiskSessionStore::new(dir.path().to_path_buf()).unwrap();

        let session_id = store.create_session();
        store.set(
            &session_id,
            "string_val",
            JsonValue::String("hello".to_string()),
        );
        store.set(&session_id, "number_val", JsonValue::Number(100.into()));
        store.set(&session_id, "bool_val", JsonValue::Bool(true));
        store.set(
            &session_id,
            "array_val",
            JsonValue::Array(vec![1.into(), 2.into()]),
        );
        store.set(&session_id, "null_val", JsonValue::Null);

        assert_eq!(
            store.get(&session_id, "string_val"),
            Some(JsonValue::String("hello".to_string()))
        );
        assert_eq!(
            store.get(&session_id, "number_val"),
            Some(JsonValue::Number(100.into()))
        );
        assert_eq!(
            store.get(&session_id, "bool_val"),
            Some(JsonValue::Bool(true))
        );
        assert_eq!(
            store.get(&session_id, "array_val"),
            Some(JsonValue::Array(vec![1.into(), 2.into()]))
        );
        assert_eq!(store.get(&session_id, "null_val"), Some(JsonValue::Null));

        assert_eq!(
            store.get(&session_id, "string_val"),
            Some(JsonValue::String("hello".to_string()))
        );
        store.delete(&session_id, "string_val");
        assert_eq!(store.get(&session_id, "string_val"), None);
        assert_eq!(
            store.get(&session_id, "number_val"),
            Some(JsonValue::Number(100.into()))
        );
    }

    #[test]
    fn test_disk_session_persistence_across_reload() {
        let dir = tempdir().unwrap();

        {
            let store = DiskSessionStore::new(dir.path().to_path_buf()).unwrap();
            let session_id = store.create_session();
            store.set(
                &session_id,
                "persistent",
                JsonValue::String("data".to_string()),
            );
        }

        {
            let _store = DiskSessionStore::new(dir.path().to_path_buf()).unwrap();
            let files = std::fs::read_dir(dir.path()).unwrap().count();
            assert!(files >= 1, "Session file should exist after reload");

            let mut session_id = None;
            for entry in std::fs::read_dir(dir.path()).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "json") {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        if let Ok(session) = serde_json::from_str::<SessionFile>(&content) {
                            if session.data.get("persistent")
                                == Some(&JsonValue::String("data".to_string()))
                            {
                                session_id = path
                                    .file_stem()
                                    .and_then(|s| s.to_str())
                                    .map(|s| s.to_string());
                                break;
                            }
                        }
                    }
                }
            }
            assert!(
                session_id.is_some(),
                "Should find session with persistent data"
            );
        }
    }

    #[test]
    fn rejects_path_traversal_in_session_id() {
        let dir = tempdir().unwrap();
        let store = DiskSessionStore::new(dir.path().to_path_buf()).unwrap();

        // Plant a sentinel file outside the session dir; if traversal worked,
        // destroy() would unlink it.
        let outside = dir.path().parent().unwrap().join("sentinel.json");
        std::fs::write(&outside, "{}").unwrap();

        // Build the relative traversal that lands on the sentinel:
        //   <session_dir>/<traverse>.json -> <parent>/sentinel.json
        let parent_name = dir.path().file_name().unwrap().to_str().unwrap();
        let traversal = format!("../{}/../sentinel", parent_name);

        assert!(store.session_path(&traversal).is_none());
        store.destroy(&traversal);
        assert!(
            outside.exists(),
            "destroy must not delete files outside the session dir"
        );

        // Other suspect inputs all map to None (no path constructed).
        for bad in [
            "../etc/passwd",
            "..\\windows\\system32",
            "/etc/passwd",
            "abc/def",
            "abc\0def",
            "",
        ] {
            assert!(
                store.session_path(bad).is_none(),
                "should reject: {:?}",
                bad
            );
        }

        // get_or_create with a bad id falls back to a fresh UUID.
        let new_id = store.get_or_create("../escape");
        assert_ne!(new_id, "../escape");
        assert!(is_safe_session_id(&new_id));

        // set/delete on a bad id are no-ops, no file written.
        store.set("../escape", "k", JsonValue::String("v".into()));
        store.delete("../escape", "k");
        assert!(!outside.with_file_name("escape.json").exists());

        // regenerate with a bad old_id still returns a fresh, safe id and
        // doesn't touch anything outside the session dir.
        let regen_id = store.regenerate("../escape");
        assert!(is_safe_session_id(&regen_id));
        assert!(outside.exists());
    }

    #[test]
    fn test_disk_session_destroy_removes_file() {
        let dir = tempdir().unwrap();
        let store = DiskSessionStore::new(dir.path().to_path_buf()).unwrap();

        let session_id = store.create_session();
        store.set(&session_id, "data", JsonValue::String("value".to_string()));

        let path = store.session_path(&session_id).expect("valid session id");
        assert!(path.exists(), "Session file should exist before destroy");

        store.destroy(&session_id);
        assert!(
            !path.exists(),
            "Session file should be removed after destroy"
        );
    }
}
