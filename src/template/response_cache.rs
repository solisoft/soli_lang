//! Per-thread response cache for deterministic template renders.
//!
//! For routes whose output depends only on the data passed to `render()`
//! — the default `soli new` `HomeController#index` is the canonical
//! example — every request would otherwise re-walk the template AST and
//! re-derive the same ETag. This cache short-circuits that work by
//! indexing the rendered body by `(template_path, layout_name,
//! data_signature)`.
//!
//! The data signature is a 64-bit FNV-1a hash of the data `Value`
//! passed to `render()`. The signature is computed recursively, so
//! deeply-nested hashes / arrays with mutable content (e.g. `Post.all()`)
//! naturally miss the cache — only truly static input hits.
//!
//! Safety / correctness:
//! * Builtins that mutate request-visible state (`set_cookie`,
//!   `session_set`, `session_regenerate`, etc.) trip the
//!   [`mark_response_dirty`] flag, which disables the cache for the
//!   current request — otherwise a cached response would strip a
//!   `Set-Cookie` header the controller just set.
//! * Builtins that introduce non-determinism into the data hash
//!   (`clock`, `random_*`) trip [`mark_data_dirty`] for the same
//!   reason — caching by data signature would be unsound when the
//!   data changes between requests.
//! * Both flags reset on the next call to [`reset_for_new_request`]
//!   (wired into `handle_request`).
//! * The cache is per-thread, sized at 64 entries, LRU-evicted. It
//!   never spans workers, so the same request could be served a
//!   freshly-cached body or re-rendered on first hit depending on
//!   which worker takes it — fine for read-only responses.

use std::cell::RefCell;
use std::num::NonZero;
use std::path::PathBuf;
use std::sync::Arc;

use lru::LruCache;

const MAX_CACHE_SIZE: NonZero<usize> = NonZero::new(64).unwrap();

/// `(template path, layout name, data signature)`. `Arc<PathBuf>` keeps
/// the key cheap to construct on a hit (no path clone per request).
#[derive(Clone)]
struct CacheKey {
    template_path: Arc<PathBuf>,
    layout: Option<Arc<str>>,
    data_sig: u64,
}

impl PartialEq for CacheKey {
    fn eq(&self, other: &Self) -> bool {
        self.data_sig == other.data_sig
            && self.layout == other.layout
            && self.template_path == other.template_path
    }
}

impl Eq for CacheKey {}

impl std::hash::Hash for CacheKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.template_path.hash(state);
        self.layout.hash(state);
        self.data_sig.hash(state);
    }
}

/// Cached `(body, etag)` for a given `(template, layout, data)`. The
/// `etag` field is currently unused by `render` — we keep it on the
/// struct so a future change can store the pre-computed ETag without
/// a cache-format break. Callers that pass `etag: ""` to `put` are
/// saying "don't bother computing the ETag now; `html_response` will
/// derive it on every request".
#[derive(Clone)]
pub struct CachedResponse {
    pub body: String,
    pub etag: String,
}

thread_local! {
    static RESPONSE_CACHE: RefCell<LruCache<CacheKey, CachedResponse>> =
        RefCell::new(LruCache::new(MAX_CACHE_SIZE));

    static DATA_DIRTY: RefCell<bool> = const { RefCell::new(false) };
    static RESPONSE_DIRTY: RefCell<bool> = const { RefCell::new(false) };
}

/// Reset per-request cacheability state. Called at the top of
/// `handle_request` so each request starts with both dirty flags
/// cleared.
pub fn reset_for_new_request() {
    DATA_DIRTY.with(|c| *c.borrow_mut() = false);
    RESPONSE_DIRTY.with(|c| *c.borrow_mut() = false);
}

/// True when the current request's response depends on a
/// request-specific cookie, session mutation, or other side effect
/// that would be lost if we returned a cached body. False (default)
/// means the response is safe to memoize.
pub fn is_response_dirty() -> bool {
    RESPONSE_DIRTY.with(|c| *c.borrow())
}

/// True when the data passed to `render()` may have varied (clock,
/// random, etc.) and a cache hit would mask that variation.
pub fn is_data_dirty() -> bool {
    DATA_DIRTY.with(|c| *c.borrow())
}

/// Trip when the controller called a mutating builtin
/// (`set_cookie`, `session_set`, etc.) that would otherwise be lost
/// on a cache hit.
pub fn mark_response_dirty() {
    RESPONSE_DIRTY.with(|c| *c.borrow_mut() = true);
}

/// Trip when the controller pulled a non-deterministic value
/// (`clock`, `random_*`) that ends up inside the data hash and
/// would invalidate any data-signature key.
pub fn mark_data_dirty() {
    DATA_DIRTY.with(|c| *c.borrow_mut() = true);
}

/// Drop the entire response cache. Used on hot-reload of the
/// `views/` tree so a stale `(template, layout) → body` mapping
/// can't outlive the AST that produced it.
pub fn clear_cache() {
    RESPONSE_CACHE.with(|c| c.borrow_mut().clear());
}

/// Look up a cached response. Returns `None` if the request is
/// marked dirty or the entry is missing.
pub fn get(
    template_path: Arc<PathBuf>,
    layout: Option<&str>,
    data_sig: u64,
) -> Option<CachedResponse> {
    if is_response_dirty() || is_data_dirty() {
        return None;
    }
    let key = CacheKey {
        template_path,
        layout: layout.map(Arc::from),
        data_sig,
    };
    RESPONSE_CACHE.with(|c| c.borrow_mut().get(&key).cloned())
}

/// Store a freshly-rendered response so subsequent identical requests
/// can skip the render.
pub fn put(
    template_path: Arc<PathBuf>,
    layout: Option<&str>,
    data_sig: u64,
    body: String,
    etag: String,
) {
    if is_response_dirty() || is_data_dirty() {
        return;
    }
    let key = CacheKey {
        template_path,
        layout: layout.map(Arc::from),
        data_sig,
    };
    let value = CachedResponse { body, etag };
    RESPONSE_CACHE.with(|c| c.borrow_mut().put(key, value));
}

// ---------------------------------------------------------------------------
// Data signature
// ---------------------------------------------------------------------------

const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

/// Compute a stable 64-bit signature for a `Value` so the response
/// cache can key off the data alone. Walks the structure
/// recursively; cheap (O(n) over the data) and always-deterministic
/// so two workers with the same data produce the same sig.
pub fn data_signature(value: &crate::interpreter::value::Value) -> u64 {
    let mut h = FNV_OFFSET;
    fnv_value(&mut h, value);
    h
}

fn fnv_value(h: &mut u64, v: &crate::interpreter::value::Value) {
    use crate::interpreter::value::Value;
    match v {
        Value::Null => {
            *h ^= b'N' as u64;
            *h = h.wrapping_mul(FNV_PRIME);
        }
        Value::Bool(b) => {
            *h ^= b'B' as u64;
            *h = h.wrapping_mul(FNV_PRIME);
            *h ^= *b as u64;
            *h = h.wrapping_mul(FNV_PRIME);
        }
        Value::Int(n) => {
            *h ^= b'I' as u64;
            *h = h.wrapping_mul(FNV_PRIME);
            *h ^= *n as u64;
            *h = h.wrapping_mul(FNV_PRIME);
        }
        Value::Float(f) => {
            *h ^= b'F' as u64;
            *h = h.wrapping_mul(FNV_PRIME);
            *h ^= f.to_bits();
            *h = h.wrapping_mul(FNV_PRIME);
        }
        Value::String(s) => {
            *h ^= b'S' as u64;
            *h = h.wrapping_mul(FNV_PRIME);
            for &b in s.as_bytes() {
                *h ^= b as u64;
                *h = h.wrapping_mul(FNV_PRIME);
            }
        }
        Value::Symbol(s) => {
            *h ^= b'Y' as u64;
            *h = h.wrapping_mul(FNV_PRIME);
            for &b in s.as_bytes() {
                *h ^= b as u64;
                *h = h.wrapping_mul(FNV_PRIME);
            }
        }
        Value::Array(arr) => {
            *h ^= b'A' as u64;
            *h = h.wrapping_mul(FNV_PRIME);
            let borrowed = arr.borrow();
            *h ^= borrowed.len() as u64;
            *h = h.wrapping_mul(FNV_PRIME);
            for elem in borrowed.iter() {
                fnv_value(h, elem);
            }
        }
        Value::Hash(map) => {
            *h ^= b'H' as u64;
            *h = h.wrapping_mul(FNV_PRIME);
            let borrowed = map.borrow();
            *h ^= borrowed.len() as u64;
            *h = h.wrapping_mul(FNV_PRIME);
            for (k, v) in borrowed.iter() {
                fnv_hash_key(h, k);
                fnv_value(h, v);
            }
        }
        // Other variants (Function, Class, Instance, NativeFunction,
        // Method, QueryBuilder, Future, Super, VmClosure, Image, ...)
        // are not seen in render data — but hash them defensively by
        // a type tag so a future change that passes one through
        // doesn't accidentally collide with a primitive.
        _ => {
            *h ^= b'?' as u64;
            *h = h.wrapping_mul(FNV_PRIME);
            *h ^= v.type_name().len() as u64;
            *h = h.wrapping_mul(FNV_PRIME);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The dev-mode hot-reload path relies on `clear_cache()` to drop
    /// stale rendered bodies after a view edit (src/serve/mod.rs worker
    /// loop). Guard the put → get → clear → miss contract.
    #[test]
    fn clear_cache_drops_cached_bodies() {
        reset_for_new_request();
        let path = Arc::new(PathBuf::from("app/views/home/index.html.slv"));
        put(
            path.clone(),
            Some("application"),
            42,
            "old body".to_string(),
            String::new(),
        );
        assert_eq!(
            get(path.clone(), Some("application"), 42).map(|c| c.body),
            Some("old body".to_string())
        );
        clear_cache();
        assert!(get(path, Some("application"), 42).is_none());
    }
}

fn fnv_hash_key(h: &mut u64, k: &crate::interpreter::value::HashKey) {
    use crate::interpreter::value::HashKey;
    // Match the per-variant tag bytes used by `HashKey::hash` so the
    // signature agrees with std's Hash impl on the same key.
    match k {
        HashKey::Int(n) => {
            *h ^= 0u8 as u64;
            *h = h.wrapping_mul(FNV_PRIME);
            *h ^= *n as u64;
            *h = h.wrapping_mul(FNV_PRIME);
        }
        HashKey::Decimal(d) => {
            *h ^= 1u8 as u64;
            *h = h.wrapping_mul(FNV_PRIME);
            // Best-effort: hash the bytes. Decimals stringify
            // deterministically across processes so this is safe.
            for &b in d.to_string().as_bytes() {
                *h ^= b as u64;
                *h = h.wrapping_mul(FNV_PRIME);
            }
        }
        HashKey::String(s) => {
            *h ^= 2u8 as u64;
            *h = h.wrapping_mul(FNV_PRIME);
            for &b in s.as_bytes() {
                *h ^= b as u64;
                *h = h.wrapping_mul(FNV_PRIME);
            }
        }
        HashKey::Bool(b) => {
            *h ^= 3u8 as u64;
            *h = h.wrapping_mul(FNV_PRIME);
            *h ^= *b as u64;
            *h = h.wrapping_mul(FNV_PRIME);
        }
        HashKey::Null => {
            *h ^= 4u8 as u64;
            *h = h.wrapping_mul(FNV_PRIME);
        }
        HashKey::Symbol(s) => {
            *h ^= 5u8 as u64;
            *h = h.wrapping_mul(FNV_PRIME);
            for &b in s.as_bytes() {
                *h ^= b as u64;
                *h = h.wrapping_mul(FNV_PRIME);
            }
        }
    }
}
