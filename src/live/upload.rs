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

/// Global cap on *in-progress* chunked uploads. The completed-file store has
/// had `MAX_STORED` since it was written, but the partial-assembly map had
/// nothing: only a TTL sweep. `POST /live/upload` is reachable without a
/// session, so anyone could mint a fresh `X-Soli-Upload-Id` per request,
/// send one chunk of a declared 512, and park up to 8 MiB per id for the
/// full TTL — unbounded RSS growth from an unauthenticated client.
const MAX_PARTIAL: usize = 32;
/// Per-session in-progress cap. Anonymous callers (no session cookie) all
/// share the `None` bucket, exactly like [`MAX_PER_OWNER`] does in [`put`],
/// so an unauthenticated flood can never claim more than one client's worth.
const MAX_PARTIAL_PER_OWNER: usize = 4;
/// Ceiling on the bytes held across *all* partial uploads at once. The
/// per-entry cap alone would still allow `MAX_PARTIAL` × 8 MiB = 256 MiB;
/// this is the number that actually bounds resident memory.
const MAX_PARTIAL_BYTES: usize = 64 * 1024 * 1024;
/// Idle timeout for a partial upload, refreshed by every accepted chunk.
/// A real upload sends chunks continuously and keeps renewing it; an entry
/// parked to hold memory is swept two minutes later instead of ten.
const PARTIAL_IDLE_TTL: Duration = Duration::from_secs(120);

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

/// In-progress chunked upload. Assembled into [`put`] when the last chunk lands.
struct PartialUpload {
    owner: Option<String>,
    name: String,
    filename: String,
    content_type: String,
    total: usize,
    received: Vec<Option<Vec<u8>>>,
    /// Last time a chunk was accepted — this is an *idle* deadline, not a
    /// total-duration one, so a slow but live upload is never cut off.
    touched: Instant,
}

impl PartialUpload {
    /// Bytes currently held, optionally ignoring one slot (the one a retry
    /// is about to overwrite — counting both copies would falsely trip the
    /// size cap on a resent chunk).
    fn held_bytes(&self, ignoring: Option<usize>) -> usize {
        self.received
            .iter()
            .enumerate()
            .filter(|(i, _)| Some(*i) != ignoring)
            .filter_map(|(_, c)| c.as_ref().map(|b| b.len()))
            .sum()
    }
}

fn chunks() -> &'static Mutex<HashMap<String, PartialUpload>> {
    static CHUNKS: std::sync::OnceLock<Mutex<HashMap<String, PartialUpload>>> =
        std::sync::OnceLock::new();
    CHUNKS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Accept one chunk of a resumable upload. `index` is 0-based. When every
/// slot is filled the file is stored with [`put`] and the usual metadata
/// (including `id`) is returned. Incomplete uploads return `{ "id", "pending": true, "received" }`.
#[allow(clippy::too_many_arguments)]
pub fn put_chunk(
    owner: Option<&str>,
    upload_id: &str,
    index: usize,
    total: usize,
    name: &str,
    filename: &str,
    content_type: &str,
    data: Vec<u8>,
) -> Result<serde_json::Value, String> {
    if total == 0 || total > 512 {
        return Err("invalid chunk count".to_string());
    }
    if index >= total {
        return Err("chunk index out of range".to_string());
    }
    if data.len() > DEFAULT_MAX_BYTES {
        return Err(format!(
            "file exceeds {} byte LiveView upload limit",
            DEFAULT_MAX_BYTES
        ));
    }
    let mut map = chunks().lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    map.retain(|_, v| now.duration_since(v.touched) < PARTIAL_IDLE_TTL);

    let owner_key = owner.map(|s| s.to_string());
    // Admission control, before the entry exists: an unknown `upload_id` is a
    // new allocation and has to fit under every cap. A chunk for an upload
    // already in flight skips this — it consumes a slot that was granted.
    if !map.contains_key(upload_id) {
        let mine = map.values().filter(|v| v.owner == owner_key).count();
        if mine >= MAX_PARTIAL_PER_OWNER {
            return Err("too many in-progress LiveView uploads for this session".to_string());
        }
        if map.len() >= MAX_PARTIAL {
            return Err("too many in-progress LiveView uploads".to_string());
        }
    }
    let held: usize = map.values().map(|v| v.held_bytes(None)).sum();

    let entry = map
        .entry(upload_id.to_string())
        .or_insert_with(|| PartialUpload {
            owner: owner_key,
            name: name.to_string(),
            filename: filename.to_string(),
            content_type: content_type.to_string(),
            total,
            received: vec![None; total],
            touched: now,
        });
    if entry.total != total {
        return Err("chunk count changed mid-upload".to_string());
    }
    if let Some(owner_id) = &entry.owner {
        if owner != Some(owner_id.as_str()) {
            return Err("upload belongs to another session".to_string());
        }
    }
    // Ignore the slot being written: a client retrying chunk 3 must not be
    // charged for both the old copy and the new one.
    let replaced = entry.received[index].as_ref().map(|b| b.len()).unwrap_or(0);
    let so_far = entry.held_bytes(Some(index));
    if so_far + data.len() > DEFAULT_MAX_BYTES {
        return Err(format!(
            "file exceeds {} byte LiveView upload limit",
            DEFAULT_MAX_BYTES
        ));
    }
    if held - replaced + data.len() > MAX_PARTIAL_BYTES {
        return Err("LiveView upload staging is full, retry shortly".to_string());
    }
    entry.received[index] = Some(data);
    entry.touched = now;
    let got = entry.received.iter().filter(|c| c.is_some()).count();
    if got < total {
        return Ok(serde_json::json!({
            "id": upload_id,
            "pending": true,
            "received": got,
            "total": total,
        }));
    }
    let mut assembled = Vec::new();
    for part in &entry.received {
        assembled.extend_from_slice(part.as_ref().unwrap());
    }
    let name = entry.name.clone();
    let filename = entry.filename.clone();
    let content_type = entry.content_type.clone();
    map.remove(upload_id);
    drop(map);
    put(owner, &name, &filename, &content_type, assembled)
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
    fn chunks_assemble_in_order() {
        let id = format!("up-{}", uuid::Uuid::new_v4());
        let a = put_chunk(
            Some("sess"),
            &id,
            0,
            2,
            "doc",
            "note.txt",
            "text/plain",
            b"hel".to_vec(),
        )
        .unwrap();
        assert_eq!(a["pending"], true);
        let b = put_chunk(
            Some("sess"),
            &id,
            1,
            2,
            "doc",
            "note.txt",
            "text/plain",
            b"lo".to_vec(),
        )
        .unwrap();
        assert!(b.get("pending").is_none());
        let mut params = serde_json::json!({ "file": { "id": b["id"] } });
        hydrate_event_params(&mut params, Some("sess"));
        let data = general_purpose::STANDARD
            .decode(params["file"]["data"].as_str().unwrap())
            .unwrap();
        assert_eq!(data, b"hello");
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

    /// The partial-assembly map used to have no cap at all: only a TTL sweep.
    /// One caller minting a fresh `upload_id` per request could park memory
    /// without bound — and `POST /live/upload` needs no session.
    #[test]
    fn one_session_cannot_exhaust_the_chunk_staging_area() {
        let owner = "sess-chunk-greedy";
        for i in 0..MAX_PARTIAL_PER_OWNER {
            put_chunk(
                Some(owner),
                &format!("{owner}-{i}"),
                0,
                2,
                "f",
                "a.bin",
                "application/octet-stream",
                vec![0; 8],
            )
            .unwrap_or_else(|e| panic!("slot {i} should fit: {e}"));
        }
        let over = put_chunk(
            Some(owner),
            &format!("{owner}-over"),
            0,
            2,
            "f",
            "a.bin",
            "application/octet-stream",
            vec![0; 8],
        );
        assert!(over.is_err(), "past the per-session in-progress cap");
        // A chunk for an upload already in flight is still accepted — the cap
        // gates new allocations, not progress on granted ones.
        assert!(put_chunk(
            Some(owner),
            &format!("{owner}-0"),
            1,
            2,
            "f",
            "a.bin",
            "application/octet-stream",
            vec![0; 8],
        )
        .is_ok());
    }

    /// Anonymous callers share the `None` bucket, so an unauthenticated flood
    /// can never claim more than one client's worth of staging slots.
    #[test]
    fn resending_a_chunk_is_not_charged_twice() {
        let id = format!("up-retry-{}", uuid::Uuid::new_v4());
        let half = DEFAULT_MAX_BYTES / 2;
        put_chunk(
            Some("sess-retry"),
            &id,
            0,
            2,
            "f",
            "a.bin",
            "application/octet-stream",
            vec![0; half],
        )
        .expect("first copy fits");
        // The retry replaces slot 0 rather than adding to it; charging both
        // copies would push the total over the per-file limit and reject a
        // perfectly ordinary network retry.
        put_chunk(
            Some("sess-retry"),
            &id,
            0,
            2,
            "f",
            "a.bin",
            "application/octet-stream",
            vec![0; half],
        )
        .expect("a resent chunk must not be double-counted");
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
