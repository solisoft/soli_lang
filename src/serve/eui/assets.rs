//! Content-addressed assets (`spec/01-transport.md` §2.2).
//!
//! An asset is named by the BLAKE3 hash of its bytes, so the same image in
//! ten sessions is one entry, a client can cache it forever, and nothing on
//! the path can substitute content. The store is process-wide and bounded.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Instant, SystemTime};

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

/// How long a file's mtime is trusted before it is stat'ed again. A grid of
/// a hundred thumbnails used to cost three hundred syscalls per render.
const RECHECK_AFTER: std::time::Duration = std::time::Duration::from_secs(1);

struct Store {
    /// The bytes, shared: a response is a refcount, not a copy.
    by_hash: HashMap<Hash, Bytes>,
    /// When each asset was last asked for, so the store can let go of the
    /// least recently used when it is full instead of refusing forever.
    last_used: HashMap<Hash, Instant>,
    /// `path -> (mtime, hash, when the mtime was last checked)`.
    by_path: HashMap<PathBuf, (SystemTime, Hash, Instant)>,
    /// `app root -> canonical app root`, so the root is resolved once.
    roots: HashMap<PathBuf, PathBuf>,
    bytes: usize,
}

static STORE: Mutex<Option<Store>> = Mutex::new(None);

fn with_store<T>(f: impl FnOnce(&mut Store) -> T) -> T {
    let mut g = STORE.lock().unwrap_or_else(|e| e.into_inner());
    let store = g.get_or_insert_with(|| Store {
        by_hash: HashMap::new(),
        last_used: HashMap::new(),
        by_path: HashMap::new(),
        roots: HashMap::new(),
        bytes: 0,
    });
    f(store)
}

impl Store {
    /// Keep `bytes` under `budget`, letting go of what was asked for least
    /// recently. The store used to refuse new assets once full, for the
    /// life of the process; an application that rotates images filled it
    /// and could then serve nothing new.
    fn put(&mut self, hash: Hash, bytes: Bytes, budget: usize) -> Result<(), String> {
        if self.by_hash.contains_key(&hash) {
            self.last_used.insert(hash, Instant::now());
            return Ok(());
        }
        if bytes.len() > budget {
            return Err("EUI: asset store is full".to_string());
        }
        while self.bytes.saturating_add(bytes.len()) > budget {
            let Some(oldest) = self
                .last_used
                .iter()
                .min_by_key(|(_, at)| **at)
                .map(|(h, _)| *h)
            else {
                break;
            };
            self.evict(&oldest);
        }
        self.bytes = self.bytes.saturating_add(bytes.len());
        self.by_hash.insert(hash, bytes);
        self.last_used.insert(hash, Instant::now());
        Ok(())
    }

    fn evict(&mut self, hash: &Hash) {
        if let Some(gone) = self.by_hash.remove(hash) {
            self.bytes = self.bytes.saturating_sub(gone.len());
        }
        self.last_used.remove(hash);
        self.by_path.retain(|_, (_, h, _)| h != hash);
    }
}

/// Store bytes, returning their hash. Refuses an asset over the size limit;
/// one that does not fit the store's budget pushes out the least recently
/// used.
pub fn put(bytes: Vec<u8>) -> Result<Hash, String> {
    if bytes.len() > MAX_ASSET_BYTES {
        return Err(format!(
            "EUI: asset of {} bytes exceeds the {} byte limit",
            bytes.len(),
            MAX_ASSET_BYTES
        ));
    }
    let hash: Hash = *blake3::hash(&bytes).as_bytes();
    with_store(|s| s.put(hash, Bytes::from(bytes), MAX_STORE_BYTES))?;
    Ok(hash)
}

/// The bytes for a hash — a handle on them, not a copy.
pub fn get(hash: &Hash) -> Option<Bytes> {
    with_store(|s| {
        let found = s.by_hash.get(hash).cloned();
        if found.is_some() {
            s.last_used.insert(*hash, Instant::now());
        }
        found
    })
}

/// Where an asset may come from, relative to the app root. A `src` is a
/// path the view wrote, and a view writes what its data says —
/// `avatar(user["photo"], 40)` — so the path is not trusted past these.
/// Everything under them is served to anyone who has the hash, session or
/// not, which is right for an image and wrong for `config/`, `.env` or the
/// publisher key next to them.
pub const ASSET_DIRS: [&str; 2] = ["public", "app/assets"];

/// Store a file from the application, by a path relative to the app root.
/// The resolved path MUST be inside one of [`ASSET_DIRS`]; anything else is
/// refused before it is read. Re-reads when the file's mtime changes.
pub fn from_file(rel: &str) -> Result<Hash, String> {
    from_file_in(&get_app_root(), rel)
}

/// [`from_file`] against an explicit application root.
pub fn from_file_in(app_root: &Path, rel: &str) -> Result<Hash, String> {
    let root = match with_store(|s| s.roots.get(app_root).cloned()) {
        Some(root) => root,
        None => {
            let root = app_root
                .canonicalize()
                .map_err(|e| format!("EUI: app root: {e}"))?;
            with_store(|s| s.roots.insert(app_root.to_path_buf(), root.clone()));
            root
        }
    };
    // A path seen lately is trusted for a second before the file is
    // stat'ed again; the joined path is looked up as written, so the
    // canonicalisation below is only paid when the cache misses.
    let joined = root.join(rel);
    if let Some((_, hash, checked)) = with_store(|s| s.by_path.get(&joined).copied()) {
        if checked.elapsed() < RECHECK_AFTER {
            return Ok(hash);
        }
    }
    let path = joined
        .canonicalize()
        .map_err(|_| format!("EUI: asset '{rel}' not found"))?;
    let allowed = ASSET_DIRS
        .iter()
        .map(|dir| root.join(dir))
        .any(|dir| path.starts_with(&dir) && path != dir);
    if !allowed {
        return Err(format!(
            "EUI: asset '{rel}' is outside {} — an asset is served to anyone with its hash",
            ASSET_DIRS.join("/ and ")
        ));
    }
    let mtime = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .map_err(|e| format!("EUI: asset '{rel}': {e}"))?;
    let now = Instant::now();
    if let Some((seen, hash, _)) = with_store(|s| s.by_path.get(&joined).copied()) {
        if seen == mtime && with_store(|s| s.by_hash.contains_key(&hash)) {
            with_store(|s| s.by_path.insert(joined, (mtime, hash, now)));
            return Ok(hash);
        }
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("EUI: asset '{rel}': {e}"))?;
    let hash = put(bytes)?;
    with_store(|s| s.by_path.insert(joined, (mtime, hash, now)));
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
            .body(full(bytes))
            .unwrap_or_else(|_| Response::new(full(Bytes::new()))),
        None => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header(header::CACHE_CONTROL, "no-store")
            .body(full(Bytes::from("no such asset")))
            .unwrap_or_else(|_| Response::new(full(Bytes::new()))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "soli-eui-assets-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        std::fs::create_dir_all(root.join("public/images")).unwrap();
        std::fs::create_dir_all(root.join("app/assets")).unwrap();
        std::fs::create_dir_all(root.join("config")).unwrap();
        std::fs::write(root.join("public/images/a.png"), b"png").unwrap();
        std::fs::write(root.join("app/assets/b.png"), b"png2").unwrap();
        std::fs::write(root.join("config/eui_publisher.pkcs8"), b"secret").unwrap();
        std::fs::write(root.join("notes.txt"), b"root").unwrap();
        root
    }

    #[test]
    fn a_full_store_lets_go_of_the_least_recently_used() {
        let mut store = Store {
            by_hash: HashMap::new(),
            last_used: HashMap::new(),
            by_path: HashMap::new(),
            roots: HashMap::new(),
            bytes: 0,
        };
        let asset = |n: u8| (([n; 32]) as Hash, Bytes::from(vec![n; 10]));
        let (a, ab) = asset(1);
        let (b, bb) = asset(2);
        let (c, cb) = asset(3);
        store.put(a, ab, 25).unwrap();
        store.put(b, bb, 25).unwrap();
        // Touch `a`, so `b` is the least recently used.
        store.last_used.insert(a, Instant::now());
        store.put(c, cb, 25).unwrap();
        assert!(store.by_hash.contains_key(&a));
        assert!(!store.by_hash.contains_key(&b), "b was pushed out");
        assert!(store.by_hash.contains_key(&c));
        assert_eq!(store.bytes, 20);
        // Nothing fits a budget smaller than itself.
        assert!(store.put([9; 32], Bytes::from(vec![0; 30]), 25).is_err());
    }

    #[test]
    fn only_the_asset_directories_are_served() {
        let root = app_root();
        assert!(from_file_in(&root, "public/images/a.png").is_ok());
        assert!(from_file_in(&root, "app/assets/b.png").is_ok());
        for refused in [
            "config/eui_publisher.pkcs8",
            "notes.txt",
            "public/../config/eui_publisher.pkcs8",
            "public",
        ] {
            let err = from_file_in(&root, refused).unwrap_err();
            assert!(err.contains("outside"), "{refused}: {err}");
        }
        let outside = root.join("../soli-eui-assets-outside.png");
        let err = from_file_in(&root, outside.to_str().unwrap()).unwrap_err();
        assert!(
            err.contains("not found") || err.contains("outside"),
            "{err}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
