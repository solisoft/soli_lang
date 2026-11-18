use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use web_time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use uuid::Uuid;

use super::session::SessionStore;
use super::session_lifetime::{self as lifetime, id_lock};

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
        now.saturating_sub(self.last_accessed) > max_age_secs
    }
}

pub struct DiskSessionStore {
    session_dir: PathBuf,
    max_age: Duration,
    request_counter: AtomicU64,
    /// Unix seconds of the last sweep of expired files (`cleanup_due`).
    last_cleanup: AtomicU64,
    /// `SOLI_SESSION_MAX_LIFETIME` (seconds, `0` = none).
    max_lifetime: u64,
}

impl DiskSessionStore {
    pub fn new(session_dir: PathBuf) -> std::io::Result<Self> {
        fs::create_dir_all(&session_dir)?;
        // Sessions hold who is signed in and the CSRF token: other local
        // users must not read them. Files were 0644 in a 0755 directory.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&session_dir, fs::Permissions::from_mode(0o700))?;
        }
        Ok(Self {
            session_dir,
            max_age: Duration::from_secs(24 * 60 * 60),
            request_counter: AtomicU64::new(0),
            last_cleanup: AtomicU64::new(lifetime::now_secs()),
            max_lifetime: lifetime::max_lifetime_secs(),
        })
    }

    /// Override the absolute lifetime (`0` disables it).
    pub fn with_max_lifetime(mut self, seconds: u64) -> Self {
        self.max_lifetime = seconds;
        self
    }

    /// Live: not idle past the TTL, and not past the absolute lifetime.
    fn is_live(&self, session: &SessionFile) -> bool {
        !session.is_expired(self.max_age.as_secs())
            && !lifetime::outlived(session.created_at, lifetime::now_secs(), self.max_lifetime)
    }

    /// SEC-038: thread `SOLI_SESSION_TTL` into the disk store so files
    /// expire on the operator-configured schedule, not the 24h default.
    pub fn with_max_age(mut self, max_age: Duration) -> Self {
        self.max_age = max_age;
        self
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
        // Written whole under another name, then renamed over the session:
        // `fs::write` truncated the file first, so a request reading it at
        // that moment found no session and dropped the user.
        let temporary = path.with_extension(format!("json.{}.tmp", Uuid::new_v4().simple()));
        {
            use std::io::Write;
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temporary)?;
            file.write_all(content.as_bytes())?;
        }
        fs::rename(&temporary, &path).inspect_err(|_| {
            let _ = fs::remove_file(&temporary);
        })
    }

    /// Change a session that still exists, under its id's lock. A session
    /// destroyed or regenerated meanwhile stays gone.
    fn update<R>(&self, session_id: &str, change: impl FnOnce(&mut SessionFile) -> R) -> Option<R> {
        let _guard = id_lock(session_id);
        let mut session = self.load_session(session_id)?;
        session.touch();
        let result = change(&mut session);
        let _ = self.save_session(session_id, &session);
        Some(result)
    }

    /// Sweep expired files at most once a minute, from the paths the server
    /// takes (creating a session), not only from `get_or_create`.
    fn maybe_cleanup(&self) {
        if lifetime::cleanup_due(&self.last_cleanup, lifetime::now_secs()) {
            self.cleanup();
        }
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
            if self.is_live(&session) {
                return session_id.to_string();
            }
        }

        let new_id = Uuid::new_v4().to_string();
        let session = SessionFile::new();
        let _ = self.save_session(&new_id, &session);
        new_id
    }

    fn create_session(&self) -> String {
        self.maybe_cleanup();
        let session_id = Uuid::new_v4().to_string();
        let session = SessionFile::new();
        let _ = self.save_session(&session_id, &session);
        session_id
    }

    fn get(&self, session_id: &str, key: &str) -> Option<JsonValue> {
        self.load_session(session_id)
            .and_then(|s| s.data.get(key).cloned())
    }

    /// The trait default probes a key with `get`, which for this store reads
    /// one entry of the session's data — a key nobody ever sets — so every
    /// existing session looked missing, and the request resolver (which
    /// refuses to mint a session for an unknown cookie) dropped every login
    /// on the next request.
    fn exists(&self, session_id: &str) -> bool {
        let Some(session) = self.load_session(session_id) else {
            return false;
        };
        if !self.is_live(&session) {
            return false;
        }
        // A read keeps the session alive too, refreshed once a quarter of the
        // TTL has passed rather than rewritten on every request.
        if lifetime::needs_refresh(
            session.last_accessed,
            lifetime::now_secs(),
            self.max_age.as_secs(),
        ) {
            self.update(session_id, |_| ());
        }
        true
    }

    fn set(&self, session_id: &str, key: &str, value: JsonValue) {
        self.update(session_id, |session| {
            session.data.insert(key.to_string(), value);
        });
    }

    fn delete(&self, session_id: &str, key: &str) -> Option<JsonValue> {
        self.update(session_id, |session| session.data.remove(key))
            .flatten()
    }

    fn destroy(&self, session_id: &str) {
        let _guard = id_lock(session_id);
        let _ = self.delete_session_file(session_id);
    }

    fn regenerate(&self, old_id: &str) -> String {
        let _guard = id_lock(old_id);
        let old_session = self.load_session(old_id);
        let new_id = Uuid::new_v4().to_string();

        if let Some(mut session) = old_session {
            // A new session, as the cookie driver's regenerate: its absolute
            // lifetime starts again.
            session.created_at = lifetime::now_secs();
            session.touch();
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
                    if !self.is_live(&session) {
                        let _guard = id_lock(&id);
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

    /// A login is `session_set` then a redirect; the next request resolves
    /// the cookie through `exists`. With the trait default this was always
    /// false for the disk driver.
    #[test]
    fn exists_sees_a_stored_session() {
        let dir = tempdir().unwrap();
        let store = DiskSessionStore::new(dir.path().to_path_buf()).unwrap();
        let id = store.create_session();
        assert!(store.exists(&id), "fresh session");
        store.set(&id, "admin_ui_authenticated", JsonValue::Bool(true));
        assert!(store.exists(&id), "session with data");
        assert!(!store.exists("00000000-0000-4000-8000-000000000000"));
        store.destroy(&id);
        assert!(!store.exists(&id), "destroyed session");

        let expired = DiskSessionStore::new(dir.path().to_path_buf())
            .unwrap()
            .with_max_age(Duration::from_secs(0));
        let id = expired.create_session();
        std::thread::sleep(Duration::from_millis(1100));
        assert!(!expired.exists(&id), "expired session");
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

    /// A write racing a logout used to save the whole session back after the
    /// file was deleted. Now a write whose session is gone does nothing, and
    /// the two cannot interleave (the id's lock).
    #[test]
    fn a_write_after_a_destroy_does_not_bring_the_session_back() {
        let dir = tempdir().unwrap();
        let store = std::sync::Arc::new(DiskSessionStore::new(dir.path().to_path_buf()).unwrap());
        for _ in 0..20 {
            let id = store.create_session();
            store.set(&id, "user_id", JsonValue::from(42));
            let writers: Vec<_> = (0..8)
                .map(|n| {
                    let store = std::sync::Arc::clone(&store);
                    let id = id.clone();
                    std::thread::spawn(move || {
                        for _ in 0..20 {
                            store.set(&id, "touched", JsonValue::from(n));
                        }
                    })
                })
                .collect();
            store.destroy(&id);
            for writer in writers {
                writer.join().unwrap();
            }
            assert!(!store.exists(&id), "the destroyed session came back");
            assert!(store.get(&id, "user_id").is_none());
        }
    }

    #[cfg(unix)]
    #[test]
    fn session_files_are_readable_by_the_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let sessions = dir.path().join("sessions");
        let store = DiskSessionStore::new(sessions.clone()).unwrap();
        let id = store.create_session();
        store.set(&id, "user_id", JsonValue::from(1));
        let file = store.session_path(&id).unwrap();
        assert_eq!(
            fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(&sessions).unwrap().permissions().mode() & 0o777,
            0o700
        );
        // No temporary file is left behind by the atomic write.
        let leftovers = fs::read_dir(&sessions)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn a_session_past_its_absolute_lifetime_is_gone_however_busy() {
        let dir = tempdir().unwrap();
        let store = DiskSessionStore::new(dir.path().to_path_buf())
            .unwrap()
            .with_max_lifetime(3_600);
        let id = store.create_session();
        let mut session = store.load_session(&id).unwrap();
        session.created_at = lifetime::now_secs() - 3_601;
        session.touch();
        store.save_session(&id, &session).unwrap();
        assert!(!store.exists(&id), "busy, but older than the lifetime");

        // Regenerating starts a new lifetime.
        let fresh = store.create_session();
        let mut old = store.load_session(&fresh).unwrap();
        old.created_at = lifetime::now_secs() - 3_000;
        store.save_session(&fresh, &old).unwrap();
        let regenerated = store.regenerate(&fresh);
        let renewed = store.load_session(&regenerated).unwrap();
        assert!(renewed.created_at + 10 >= lifetime::now_secs());
    }

    #[test]
    fn a_read_keeps_the_session_alive() {
        let dir = tempdir().unwrap();
        let store = DiskSessionStore::new(dir.path().to_path_buf())
            .unwrap()
            .with_max_age(Duration::from_secs(100));
        let id = store.create_session();
        let mut session = store.load_session(&id).unwrap();
        session.last_accessed = lifetime::now_secs() - 60;
        store.save_session(&id, &session).unwrap();
        assert!(store.exists(&id));
        let refreshed = store.load_session(&id).unwrap();
        assert!(
            refreshed.last_accessed + 5 >= lifetime::now_secs(),
            "read did not refresh"
        );
    }

    #[test]
    fn creating_sessions_sweeps_the_expired_ones() {
        let dir = tempdir().unwrap();
        let store = DiskSessionStore::new(dir.path().to_path_buf())
            .unwrap()
            .with_max_age(Duration::from_secs(10));
        let stale = store.create_session();
        let mut session = store.load_session(&stale).unwrap();
        session.last_accessed = lifetime::now_secs() - 3_600;
        store.save_session(&stale, &session).unwrap();
        // Make the sweep due, then create a session as the server does.
        store.last_cleanup.store(0, Ordering::Relaxed);
        store.create_session();
        assert!(
            !store.session_path(&stale).unwrap().exists(),
            "expired file kept"
        );
    }
}
