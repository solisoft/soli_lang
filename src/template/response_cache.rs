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
use std::collections::HashSet;
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

    /// `(template, layout)` pairs observed to be uncacheable — see
    /// [`is_known_uncacheable`].
    static UNCACHEABLE: RefCell<HashSet<PairKey>> = RefCell::new(HashSet::new());
}

/// A `(template, layout)` pair, without the data signature. Identifies a
/// *render site* rather than a specific render.
type PairKey = (Arc<PathBuf>, Option<Arc<str>>);

/// Whether this render site has already been shown to be uncacheable.
///
/// The dirty flags are set *during* a render, not before it: the canonical
/// case is `csrf_meta_tag()` in the layout, which calls `csrf_token()` and
/// marks the response dirty — and the layout renders after the cache lookup
/// but before the store. So a render site whose layout embeds a per-session
/// token looks clean on entry, pays a full `data_signature` walk, misses (the
/// store was refused last time for the same reason), renders, and is refused
/// again. Every request, forever.
///
/// The default `soli new` layout calls `csrf_meta_tag()`, so this is the
/// common case for real apps rather than an edge case. Remembering the answer
/// turns an O(data) hash per request into a set lookup.
///
/// Deliberately sticky: a site that is dirty *sometimes* (a controller that
/// only sets a cookie for first-time visitors) stays marked and stops being
/// cached. That is the safe direction to be wrong in — it forfeits a cache,
/// never serves a stale body — and it keeps this free of a re-check schedule
/// that would reintroduce the cost it removes.
pub fn is_known_uncacheable(template_path: &Arc<PathBuf>, layout: Option<&str>) -> bool {
    UNCACHEABLE.with(|c| {
        let key: PairKey = (template_path.clone(), layout.map(Arc::from));
        c.borrow().contains(&key)
    })
}

fn mark_uncacheable(template_path: Arc<PathBuf>, layout: Option<&str>) {
    UNCACHEABLE.with(|c| {
        c.borrow_mut()
            .insert((template_path, layout.map(Arc::from)));
    });
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
    // An edited view can change whether it is cacheable at all (adding or
    // removing a `csrf_meta_tag()`), so the negative cache must go with it.
    UNCACHEABLE.with(|c| c.borrow_mut().clear());
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
        // The render itself tripped a dirty flag, so this site cannot be
        // cached. Record that, so the next request skips the `data_signature`
        // walk instead of paying it to reach this same line again.
        mark_uncacheable(template_path, layout);
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

    /// A render site whose render trips a dirty flag must be remembered as
    /// uncacheable, so `render()` can skip the `data_signature` walk instead
    /// of paying it every request to rediscover the same answer.
    ///
    /// This is the `csrf_meta_tag()`-in-the-layout case, which is the default
    /// for `soli new`: the flag is set *during* the render, so the site looks
    /// clean on entry and only `put` ever learns otherwise.
    #[test]
    fn a_dirty_render_marks_the_site_uncacheable() {
        reset_for_new_request();
        clear_cache();
        let path = Arc::new(PathBuf::from("app/views/items/index.html.erb"));

        assert!(
            !is_known_uncacheable(&path, Some("application")),
            "a site starts out assumed cacheable"
        );

        // Mid-render, something like `csrf_token()` trips the flag; `put` is
        // then reached with the response already dirty.
        mark_response_dirty();
        put(
            path.clone(),
            Some("application"),
            7,
            "body".to_string(),
            String::new(),
        );

        assert!(
            is_known_uncacheable(&path, Some("application")),
            "the refused store must be remembered"
        );
        // The refusal itself must still hold: nothing was cached.
        reset_for_new_request();
        assert!(get(path.clone(), Some("application"), 7).is_none());

        // The mark is per (template, layout) — a different layout is its own
        // decision, since cacheability usually comes from the layout.
        assert!(!is_known_uncacheable(&path, Some("bare")));
        assert!(!is_known_uncacheable(&path, None));

        // An edited view can change whether it is cacheable, so a hot reload
        // must clear the negative cache along with the bodies.
        clear_cache();
        assert!(
            !is_known_uncacheable(&path, Some("application")),
            "clear_cache must reset the negative cache too"
        );
    }

    /// The negative cache must not touch sites that render cleanly — those
    /// still cache normally.
    #[test]
    fn a_clean_render_stays_cacheable() {
        reset_for_new_request();
        clear_cache();
        let path = Arc::new(PathBuf::from("app/views/docs/page.html.slv"));

        put(
            path.clone(),
            Some("docs"),
            99,
            "rendered".to_string(),
            String::new(),
        );

        assert!(!is_known_uncacheable(&path, Some("docs")));
        assert_eq!(
            get(path, Some("docs"), 99).map(|c| c.body),
            Some("rendered".to_string())
        );
    }
}
