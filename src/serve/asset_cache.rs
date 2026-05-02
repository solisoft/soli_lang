//! In-memory snapshot of `public/` CSS and JS files for production mode.
//!
//! At server startup we walk `public/`, load every `.css` and `.js` file
//! into memory, and serve those bytes for the lifetime of the process. This
//! prevents the deploy-race where new asset bytes appear on disk before the
//! binary restarts: the running server keeps serving the bytes it loaded at
//! boot, so HTML still references assets whose content matches.
//!
//! Dev mode never builds the cache (returns an empty map) so live-reloaded
//! files are always read fresh.
use bytes::Bytes;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;
const WARN_TOTAL_BYTES: u64 = 100 * 1024 * 1024;

#[derive(Clone)]
pub struct CachedAsset {
    pub bytes: Bytes,
    pub etag: String,
    pub content_type: &'static str,
}

pub type AssetCache = Arc<HashMap<PathBuf, CachedAsset>>;

pub fn build(public_dir: &Path, dev_mode: bool) -> AssetCache {
    if dev_mode || !public_dir.exists() {
        return Arc::new(HashMap::new());
    }
    let canonical_root = match std::fs::canonicalize(public_dir) {
        Ok(p) => p,
        Err(_) => return Arc::new(HashMap::new()),
    };
    let mut map = HashMap::new();
    let mut total: u64 = 0;
    walk(&canonical_root, &mut map, &mut total);
    if total > WARN_TOTAL_BYTES {
        eprintln!(
            "Warning: prod asset cache exceeds 100 MB ({} bytes across {} files)",
            total,
            map.len()
        );
    }
    if !map.is_empty() {
        println!(
            "Cached {} CSS/JS assets ({} bytes) for prod-mode serving",
            map.len(),
            total
        );
    }
    Arc::new(map)
}

fn walk(dir: &Path, out: &mut HashMap<PathBuf, CachedAsset>, total: &mut u64) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let path = entry.path();
        if meta.is_dir() {
            walk(&path, out, total);
            continue;
        }
        if !meta.is_file() {
            continue;
        }
        let ext = match path.extension().and_then(|s| s.to_str()) {
            Some(e) => e,
            None => continue,
        };
        let content_type: &'static str = match ext {
            "css" => "text/css",
            "js" => "application/javascript",
            _ => continue,
        };
        if meta.len() > MAX_FILE_BYTES {
            eprintln!(
                "Skipping asset cache for {} ({} bytes > {} cap)",
                path.display(),
                meta.len(),
                MAX_FILE_BYTES
            );
            continue;
        }
        let data = match std::fs::read(&path) {
            Ok(d) => d,
            Err(_) => continue,
        };
        let digest = Sha256::digest(&data);
        let mut h = 0u64;
        for byte in &digest[..8] {
            h = (h << 8) | *byte as u64;
        }
        let etag = format!("\"{:016x}\"", h);
        let canonical = std::fs::canonicalize(&path).unwrap_or(path);
        *total += data.len() as u64;
        out.insert(
            canonical,
            CachedAsset {
                bytes: Bytes::from(data),
                etag,
                content_type,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    fn write_file(path: &Path, contents: &[u8]) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut f = fs::File::create(path).unwrap();
        f.write_all(contents).unwrap();
    }

    #[test]
    fn caches_css_and_js_only() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write_file(&root.join("css/app.css"), b"body { color: red }");
        write_file(&root.join("js/app.js"), b"console.log(1)");
        write_file(&root.join("images/logo.png"), b"\x89PNG fake");
        write_file(&root.join("readme.txt"), b"plain");

        let cache = build(root, false);
        assert_eq!(cache.len(), 2);
        let css_key = fs::canonicalize(root.join("css/app.css")).unwrap();
        let js_key = fs::canonicalize(root.join("js/app.js")).unwrap();
        assert!(cache.contains_key(&css_key));
        assert!(cache.contains_key(&js_key));
        assert_eq!(cache.get(&css_key).unwrap().content_type, "text/css");
        assert_eq!(
            cache.get(&js_key).unwrap().content_type,
            "application/javascript"
        );
    }

    #[test]
    fn dev_mode_returns_empty() {
        let tmp = tempfile::tempdir().unwrap();
        write_file(&tmp.path().join("css/app.css"), b"body{}");
        let cache = build(tmp.path(), true);
        assert!(cache.is_empty());
    }

    #[test]
    fn missing_dir_returns_empty() {
        let cache = build(Path::new("/definitely/does/not/exist/abc123"), false);
        assert!(cache.is_empty());
    }

    #[test]
    fn etag_is_content_hash_stable() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("a.css");
        write_file(&path, b"body{color:red}");
        let cache1 = build(tmp.path(), false);
        let cache2 = build(tmp.path(), false);
        let key = fs::canonicalize(&path).unwrap();
        assert_eq!(
            cache1.get(&key).unwrap().etag,
            cache2.get(&key).unwrap().etag
        );

        // Different content -> different etag
        write_file(&path, b"body{color:blue}");
        let cache3 = build(tmp.path(), false);
        assert_ne!(
            cache1.get(&key).unwrap().etag,
            cache3.get(&key).unwrap().etag
        );
    }

    #[test]
    fn skips_files_above_size_cap() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("huge.css");
        let big = vec![b'x'; (MAX_FILE_BYTES + 1) as usize];
        write_file(&path, &big);
        let cache = build(tmp.path(), false);
        assert!(cache.is_empty());
    }
}
