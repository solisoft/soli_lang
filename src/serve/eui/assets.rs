//! Content-addressed assets (`spec/01-transport.md` §2.2).
//!
//! An asset is named by the BLAKE3 hash of its bytes, so the same image in
//! ten sessions is one entry, a client can cache it forever, and nothing on
//! the path can substitute content. The store is process-wide and bounded.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use bytes::Bytes;
use hyper::{header, Response, StatusCode};

use crate::live::component::get_app_root;

use super::super::{full, ResponseBody};

/// Hash of an asset's bytes.
pub type Hash = [u8; 32];

/// Largest single asset.
pub const MAX_ASSET_BYTES: usize = 16 * 1024 * 1024;
/// Largest total the store will hold; past it, new assets are refused.
pub const MAX_STORE_BYTES: usize = 256 * 1024 * 1024;

struct Store {
    by_hash: HashMap<Hash, Arc<Vec<u8>>>,
    by_path: HashMap<PathBuf, (SystemTime, Hash)>,
    bytes: usize,
}

static STORE: Mutex<Option<Store>> = Mutex::new(None);

fn with_store<T>(f: impl FnOnce(&mut Store) -> T) -> T {
    let mut g = STORE.lock().unwrap_or_else(|e| e.into_inner());
    let store = g.get_or_insert_with(|| Store {
        by_hash: HashMap::new(),
        by_path: HashMap::new(),
        bytes: 0,
    });
    f(store)
}

/// Store bytes, returning their hash. Refuses an asset over the size limit
/// or one that would push the store over its budget.
pub fn put(bytes: Vec<u8>) -> Result<Hash, String> {
    if bytes.len() > MAX_ASSET_BYTES {
        return Err(format!(
            "EUI: asset of {} bytes exceeds the {} byte limit",
            bytes.len(),
            MAX_ASSET_BYTES
        ));
    }
    let hash: Hash = *blake3::hash(&bytes).as_bytes();
    with_store(|s| {
        if s.by_hash.contains_key(&hash) {
            return Ok(hash);
        }
        if s.bytes.saturating_add(bytes.len()) > MAX_STORE_BYTES {
            return Err("EUI: asset store is full".to_string());
        }
        s.bytes = s.bytes.saturating_add(bytes.len());
        s.by_hash.insert(hash, Arc::new(bytes));
        Ok(hash)
    })
}

/// The bytes for a hash.
pub fn get(hash: &Hash) -> Option<Arc<Vec<u8>>> {
    with_store(|s| s.by_hash.get(hash).cloned())
}

/// Store a file from the application, by a path relative to the app root.
/// The resolved path MUST stay inside the root; anything else is refused
/// before it is read. Re-reads when the file's mtime changes.
pub fn from_file(rel: &str) -> Result<Hash, String> {
    let root = get_app_root()
        .canonicalize()
        .map_err(|e| format!("EUI: app root: {e}"))?;
    let path = root
        .join(rel)
        .canonicalize()
        .map_err(|_| format!("EUI: asset '{rel}' not found"))?;
    if !path.starts_with(&root) {
        return Err(format!(
            "EUI: asset '{rel}' resolves outside the application"
        ));
    }
    let mtime = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .map_err(|e| format!("EUI: asset '{rel}': {e}"))?;
    if let Some(hit) = with_store(|s| s.by_path.get(&path).copied()) {
        if hit.0 == mtime {
            return Ok(hit.1);
        }
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("EUI: asset '{rel}': {e}"))?;
    let hash = put(bytes)?;
    with_store(|s| s.by_path.insert(path, (mtime, hash)));
    Ok(hash)
}

/// Parse `<64 hex chars>` into a hash.
pub fn parse_hex(hex: &str) -> Option<Hash> {
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
        let s = std::str::from_utf8(chunk).ok()?;
        out[i] = u8::from_str_radix(s, 16).ok()?;
    }
    Some(out)
}

/// `GET /_eui/asset/<hex>`: the bytes, immutable, or 404.
pub fn respond(hex: &str) -> Response<ResponseBody> {
    let found = parse_hex(hex.trim_end_matches('/')).and_then(|h| get(&h));
    match found {
        Some(bytes) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")
            .header(header::CONTENT_LENGTH, bytes.len())
            .body(full(Bytes::from(bytes.as_ref().clone())))
            .unwrap_or_else(|_| Response::new(full(Bytes::new()))),
        None => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header(header::CACHE_CONTROL, "no-store")
            .body(full(Bytes::from("no such asset")))
            .unwrap_or_else(|_| Response::new(full(Bytes::new()))),
    }
}
