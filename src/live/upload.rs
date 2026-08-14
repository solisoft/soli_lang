//! Temporary store for LiveView HTTP uploads.
//!
//! WebSocket frames are capped at 1 MiB (SEC-047), so file bytes travel
//! over `POST /live/upload` and the socket only carries a small id. The
//! event path hydrates `data` (base64) so handlers see the same shape as
//! a multipart `find_uploaded_file` result.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::{engine::general_purpose, Engine as _};

/// Default per-file cap (bytes). Matches a typical `SOLI_MAX_BODY_SIZE` floor.
pub const DEFAULT_MAX_BYTES: usize = 8 * 1024 * 1024;
/// Global backstop across all sessions.
const MAX_STORED: usize = 64;
/// Per-session slot cap. Without it one client fills the global map — 64 slots
/// × 8 MiB ≈ 512 MiB of RSS — and locks every other user out of uploads.
const MAX_PER_OWNER: usize = 8;
const TTL: Duration = Duration::from_secs(600);

struct Stored {
    /// Session that uploaded the file. `None` when the request carried no
    /// session cookie: such an entry stays takeable by anyone, as before.
    owner: Option<String>,
    name: String,
    filename: String,
    content_type: String,
    data: Vec<u8>,
    created: Instant,
}

fn store() -> &'static Mutex<HashMap<String, Stored>> {
    static STORE: std::sync::OnceLock<Mutex<HashMap<String, Stored>>> = std::sync::OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Put one uploaded file in the store. Returns the public metadata (no bytes).
///
/// `owner` is the uploading session; only that session can take the file back
/// (see [`take`]), and it gets at most [`MAX_PER_OWNER`] pending slots.
pub fn put(
    owner: Option<&str>,
    name: &str,
    filename: &str,
    content_type: &str,
    data: Vec<u8>,
) -> Result<serde_json::Value, String> {
    if data.len() > DEFAULT_MAX_BYTES {
        return Err(format!(
            "file exceeds {} byte LiveView upload limit",
            DEFAULT_MAX_BYTES
        ));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let size = data.len();
    let mut map = store().lock().unwrap_or_else(|e| e.into_inner());
    prune_locked(&mut map);
    let owner_key = owner.map(|s| s.to_string());
    let mine = map.values().filter(|v| v.owner == owner_key).count();
    if mine >= MAX_PER_OWNER {
        return Err("too many pending LiveView uploads for this session".to_string());
    }
    if map.len() >= MAX_STORED {
        return Err("too many pending LiveView uploads".to_string());
    }
    map.insert(
        id.clone(),
        Stored {
            owner: owner_key,
            name: name.to_string(),
            filename: filename.to_string(),
            content_type: content_type.to_string(),
            data,
            created: Instant::now(),
        },
    );
    Ok(serde_json::json!({
        "id": id,
        "name": name,
        "filename": filename,
        "content_type": content_type,
        "size": size,
    }))
}

/// Take a stored file, or `None` if unknown, expired, or owned by another
/// session. A mismatch leaves the entry in place — a wrong guess must not
/// consume someone else's pending upload.
pub fn take(id: &str, taker: Option<&str>) -> Option<serde_json::Value> {
    let mut map = store().lock().unwrap_or_else(|e| e.into_inner());
    prune_locked(&mut map);
    let owner = map.get(id)?.owner.clone();
    if let Some(owner) = owner {
        if taker != Some(owner.as_str()) {
            return None;
        }
    }
    let stored = map.remove(id)?;
    Some(entry_json(&stored))
}

fn entry_json(stored: &Stored) -> serde_json::Value {
    serde_json::json!({
        "name": stored.name,
        "filename": stored.filename,
        "content_type": stored.content_type,
        "size": stored.data.len(),
        "data": general_purpose::STANDARD.encode(&stored.data),
    })
}

fn prune_locked(map: &mut HashMap<String, Stored>) {
    let now = Instant::now();
    map.retain(|_, v| now.duration_since(v.created) < TTL);
}

/// Replace `{ "id": "…" }` file hashes in a LiveView event's params with
/// the stored multipart shape (`filename`, `content_type`, `size`, `data`).
pub fn hydrate_event_params(params: &mut serde_json::Value, session_id: Option<&str>) {
    hydrate_one(params, session_id);
    if let Some(file) = params.get_mut("file") {
        hydrate_one(file, session_id);
    }
    if let Some(serde_json::Value::Array(files)) = params.get_mut("files") {
        for file in files {
            hydrate_one(file, session_id);
        }
    }
}

fn hydrate_one(value: &mut serde_json::Value, session_id: Option<&str>) {
    let Some(id) = value.get("id").and_then(|v| v.as_str()) else {
        return;
    };
    if value.get("data").is_some_and(|d| d.is_string()) {
        return;
    }
    let Some(full) = take(id, session_id) else {
        return;
    };
    *value = full;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_then_take_hydrates_base64_data() {
        let meta = put(None, "avatar", "x.png", "image/png", b"\x89PNG".to_vec()).unwrap();
        let id = meta["id"].as_str().unwrap();
        assert_eq!(meta["filename"], "x.png");
        assert!(meta.get("data").is_none());

        let mut params = serde_json::json!({ "file": { "id": id, "filename": "x.png" } });
        hydrate_event_params(&mut params, None);
        assert_eq!(params["file"]["filename"], "x.png");
        assert_eq!(params["file"]["size"], 4);
        assert!(params["file"]["data"].as_str().unwrap().len() > 4);
        // One-shot: a second hydrate cannot find the id.
        let mut again = serde_json::json!({ "file": { "id": id } });
        hydrate_event_params(&mut again, None);
        assert!(again["file"].get("data").is_none());
    }

    #[test]
    fn missing_id_does_not_invent_data() {
        let mut params =
            serde_json::json!({ "file": { "id": "no-such-upload", "filename": "x.png" } });
        hydrate_event_params(&mut params, None);
        assert!(
            params["file"].get("data").is_none(),
            "unknown id must not grow a data field"
        );
        assert_eq!(params["file"]["id"], "no-such-upload");
        assert_eq!(params["file"]["filename"], "x.png");
    }

    #[test]
    fn consume_round_trips_original_bytes() {
        let bytes = b"hello-live-upload";
        let meta = put(None, "doc", "note.txt", "text/plain", bytes.to_vec()).unwrap();
        let mut params = serde_json::json!({ "file": { "id": meta["id"] } });
        hydrate_event_params(&mut params, None);
        let data = params["file"]["data"].as_str().expect("hydrated data");
        let decoded = general_purpose::STANDARD.decode(data).unwrap();
        assert_eq!(decoded, bytes);
        assert_eq!(params["file"]["filename"], "note.txt");
        assert_eq!(params["file"]["content_type"], "text/plain");
        assert_eq!(params["file"]["size"], bytes.len() as i64);
    }

    #[test]
    fn oversized_put_is_refused() {
        let big = vec![0u8; DEFAULT_MAX_BYTES + 1];
        assert!(put(None, "f", "big.bin", "application/octet-stream", big).is_err());
    }

    /// An upload belongs to the session that made it: another session must not
    /// be able to hydrate it into its own handler params.
    #[test]
    fn another_session_cannot_take_the_file() {
        let meta = put(
            Some("sess-a"),
            "avatar",
            "a.png",
            "image/png",
            b"aaa".to_vec(),
        )
        .unwrap();
        let id = meta["id"].as_str().unwrap().to_string();

        assert!(take(&id, Some("sess-b")).is_none(), "wrong session refused");
        assert!(take(&id, None).is_none(), "no session refused");
        // Refusal must not consume the entry — the owner can still take it.
        let mine = take(&id, Some("sess-a")).expect("owner takes its own upload");
        assert_eq!(mine["filename"], "a.png");
    }

    /// One session must not be able to fill the global store and lock everyone
    /// else out of uploads.
    #[test]
    fn one_session_cannot_exhaust_the_store() {
        let owner = "sess-greedy";
        for i in 0..MAX_PER_OWNER {
            put(
                Some(owner),
                "f",
                &format!("{i}.bin"),
                "application/octet-stream",
                vec![0; 8],
            )
            .unwrap_or_else(|e| panic!("slot {i} should fit: {e}"));
        }
        let over = put(
            Some(owner),
            "f",
            "over.bin",
            "application/octet-stream",
            vec![0; 8],
        );
        assert!(over.is_err(), "past the per-session cap");
        // Another session is unaffected.
        assert!(put(
            Some("sess-other"),
            "f",
            "ok.bin",
            "application/octet-stream",
            vec![0; 8]
        )
        .is_ok());
    }
}
