//! Per-thread response cache for deterministic template renders.
//!
//! For routes whose output depends only on the data passed to `render()`
//! — the default `soli new` `HomeController#index` is the canonical
//! example — every request would otherwise re-walk the template AST and
//! re-derive the same ETag. This cache short-circuits that work by
//! indexing the rendered body by `(template_path, layout_name, locale,
//! data_signature)`.
//!
//! The data signature is a 64-bit FNV-1a hash of the data `Value`
//! passed to `render()`, computed recursively over its plain values. Data
//! the signature cannot stand for — an object such as a model record from
//! `Post.all`, a query not yet run — has no signature, and the render is
//! not cached: see [`data_signature`]. Only truly static input hits.
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
//! * The active locale is part of the key. A view's `t()` calls read it
//!   from a thread-local, not from the data, so without it a worker that
//!   first rendered a page in `fr` served that `fr` body to every later
//!   `en` request with the same data.
//! * A request with a session never touches the cache
//!   ([`request_has_session`]): a layout that reads the session
//!   (`current_user`, `signed_in?`) renders per person, and the data
//!   signature cannot see that.
//! * Data holding an object never touches it either ([`data_signature`]):
//!   the view calls the object's methods, and a model's relations read the
//!   database. Hashed by the length of its class name, a record made the
//!   page of `Post.all` outlive an edit, and a mailer view send one user's
//!   mail to the next.
//! * The cache is per-thread, sized at 64 entries, LRU-evicted. It
//!   never spans workers, so the same request could be served a
//!   freshly-cached body or re-rendered on first hit depending on
//!   which worker takes it — fine for read-only responses.

use std::cell::RefCell;
use std::collections::HashSet;
use std::num::NonZero;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use lru::LruCache;

use crate::interpreter::value::{HashKey, SoliStr, Value};

const MAX_CACHE_SIZE: NonZero<usize> = NonZero::new(64).unwrap();

/// `(template path, layout name, locale, data signature, finished)`.
/// `Arc<PathBuf>` keeps the key cheap to construct on a hit (no path clone per
/// request). `finished` separates the two things stored here: the raw body of
/// a `TemplateCache::render`, and the HTML response built from it, with its
/// scripts injected and its ETag computed. The same site can hold both.
#[derive(Clone)]
struct CacheKey {
    template_path: Arc<PathBuf>,
    layout: Option<Arc<str>>,
    locale: String,
    data_sig: u64,
    finished: bool,
}

impl PartialEq for CacheKey {
    fn eq(&self, other: &Self) -> bool {
        self.data_sig == other.data_sig
            && self.finished == other.finished
            && self.layout == other.layout
            && self.locale == other.locale
            && self.template_path == other.template_path
    }
}

impl Eq for CacheKey {}

impl std::hash::Hash for CacheKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.template_path.hash(state);
        self.layout.hash(state);
        self.locale.hash(state);
        self.data_sig.hash(state);
        self.finished.hash(state);
    }
}

/// Cached `(body, etag)` for a given `(template, layout, data)`. Both are
/// `SoliStr`, so a hit hands out a reference-counted clone instead of copying
/// the page. A raw render stores an empty `etag`; a finished HTML response
/// stores the ETag of the exact bytes it sends.
#[derive(Clone)]
pub struct CachedResponse {
    pub body: SoliStr,
    pub etag: SoliStr,
}

thread_local! {
    static RESPONSE_CACHE: RefCell<LruCache<CacheKey, CachedResponse>> =
        RefCell::new(LruCache::new(MAX_CACHE_SIZE));

    /// `(template, layout)` pairs observed to be uncacheable — see
    /// [`is_known_uncacheable`].
    static UNCACHEABLE: RefCell<HashSet<PairKey>> = RefCell::new(HashSet::new());
}

crate::request_local! {

    static DATA_DIRTY: RefCell<bool> = const { RefCell::new(false) };
    static RESPONSE_DIRTY: RefCell<bool> = const { RefCell::new(false) };
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

/// Whether the current request carries a session, in which case its render
/// may depend on who is asking and must not be cached or served from cache.
pub fn request_has_session() -> bool {
    crate::interpreter::builtins::session::get_current_session_id().is_some()
}

/// Reset per-request cacheability state. Called at the top of
/// `handle_request` so each request starts with both dirty flags
/// cleared.
pub fn reset_for_new_request() {
    DATA_DIRTY.with(|c| *c.borrow_mut() = false);
    RESPONSE_DIRTY.with(|c| *c.borrow_mut() = false);
    crate::interpreter::builtins::spam::reset_honeypot_ids();
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
    finished: bool,
) -> Option<CachedResponse> {
    if is_response_dirty() || is_data_dirty() {
        return None;
    }
    let key = CacheKey {
        template_path,
        layout: layout.map(Arc::from),
        locale: crate::interpreter::builtins::i18n::helpers::get_locale(),
        data_sig,
        finished,
    };
    RESPONSE_CACHE.with(|c| c.borrow_mut().get(&key).cloned())
}

/// Store a freshly-rendered response so subsequent identical requests
/// can skip the render.
pub fn put(
    template_path: Arc<PathBuf>,
    layout: Option<&str>,
    data_sig: u64,
    finished: bool,
    body: SoliStr,
    etag: SoliStr,
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
        locale: crate::interpreter::builtins::i18n::helpers::get_locale(),
        data_sig,
        finished,
    };
    let value = CachedResponse { body, etag };
    RESPONSE_CACHE.with(|c| c.borrow_mut().put(key, value));
}

// ---------------------------------------------------------------------------
// Data signature
// ---------------------------------------------------------------------------

const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

/// Containers nested deeper than this are not hashed, and the render is not
/// cached. Far deeper than a page's data goes; it bounds the walk on data
/// built deep on purpose.
const MAX_DEPTH: usize = 128;

/// The response cache's signature for the data passed to `render()`, or
/// `None` when the data cannot stand for the page and the render must not be
/// cached.
///
/// Values are hashed by what they hold — scalars, `Decimal` (its digits and
/// its precision), `DateTime` (the instant and the zone it is shown in),
/// strings, arrays and hashes, recursively — so the same data gives the same
/// signature on every worker. Code (a function, a class) is hashed by its
/// name: the cache trusts it as it trusts the helpers a view calls.
///
/// `None` for anything else:
///
/// * an object: a model record from `Post.all`, any class instance. A view
///   calls its methods, and they read what the signature cannot see — a
///   relation (`post.author`) queries the database. Hashed by the length of
///   its class name until 2.29.3, a list of records kept the first request's
///   page after a record changed, and a mailer view sent one user's mail to
///   the next.
/// * a query not yet run (`Post.where(...)`, iterated by the view), a future
///   or a grouped read still pending, an image.
/// * data nested deeper than `MAX_DEPTH` (128) containers, or that contains
///   itself.
///
/// The walk stops at the first such value. The refusal is not remembered
/// for the render site: the next request's data may differ (a lookup that
/// found nothing), and its depth is the client's to choose.
pub fn data_signature(value: &Value) -> Option<u64> {
    Walk::new(Objects::Refuse).signature(value)
}

/// [`data_signature`], except that an object is hashed by its class and its
/// fields, in whatever order the fields come. For the key of a cache the
/// application asked for — a component's `"cache": true` — whose HTML may be
/// kept as long as the data it was rendered from is unchanged, whatever the
/// object's methods would read now.
pub fn content_signature(value: &Value) -> Option<u64> {
    Walk::new(Objects::ByFields).signature(value)
}

/// What the walk does when it meets an object.
#[derive(Clone, Copy)]
enum Objects {
    Refuse,
    ByFields,
}

struct Walk {
    objects: Objects,
    /// The containers between the root and the value being hashed, by
    /// address: meeting one of them again means the data contains itself.
    path: [usize; MAX_DEPTH],
    depth: usize,
}

impl Walk {
    fn new(objects: Objects) -> Self {
        Walk {
            objects,
            path: [0; MAX_DEPTH],
            depth: 0,
        }
    }

    fn signature(mut self, value: &Value) -> Option<u64> {
        let mut h = FNV_OFFSET;
        self.value(&mut h, value)?;
        Some(h)
    }

    /// Step into `container`, unless it is already on the path or the path is
    /// as deep as it may go. A refusal ends the whole walk, so nothing has to
    /// step back out after one.
    fn enter<T>(&mut self, container: &Rc<T>) -> Option<()> {
        let address = Rc::as_ptr(container) as usize;
        if self.depth == MAX_DEPTH || self.path[..self.depth].contains(&address) {
            return None;
        }
        self.path[self.depth] = address;
        self.depth += 1;
        Some(())
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }

    fn value(&mut self, h: &mut u64, v: &Value) -> Option<()> {
        match v {
            Value::Null => mix(h, b'N' as u64),
            Value::Bool(b) => {
                mix(h, b'B' as u64);
                mix(h, *b as u64);
            }
            Value::Int(n) => {
                mix(h, b'I' as u64);
                mix(h, *n as u64);
            }
            Value::Float(f) => {
                mix(h, b'F' as u64);
                mix(h, f.to_bits());
            }
            Value::Decimal(d) => {
                // The precision too: `1.50D` and `1.5D` are equal and print
                // differently.
                mix(h, b'M' as u64);
                mix_bytes(h, &d.0.serialize());
                mix(h, d.1 as u64);
            }
            Value::String(s) => {
                mix(h, b'S' as u64);
                mix_str(h, s.as_bytes());
            }
            Value::Symbol(s) => {
                mix(h, b'Y' as u64);
                mix_str(h, s.as_bytes());
            }
            Value::DateTime(nanos, zone) => {
                mix(h, b'D' as u64);
                mix(h, *nanos as u64);
                mix(h, zone.bits() as u32 as u64);
            }
            Value::Array(arr) => {
                self.enter(arr)?;
                let items = arr.try_borrow().ok()?;
                mix(h, b'A' as u64);
                mix(h, items.len() as u64);
                for item in items.iter() {
                    self.value(h, item)?;
                }
                self.leave();
            }
            Value::Hash(map) => {
                self.enter(map)?;
                let pairs = map.try_borrow().ok()?;
                mix(h, b'H' as u64);
                mix(h, pairs.len() as u64);
                for (k, v) in pairs.iter() {
                    fnv_hash_key(h, k);
                    self.value(h, v)?;
                }
                self.leave();
            }
            // An XML node is hashed by its content (`to_xml`): two renders that
            // pass different trees must not share a cache entry.
            Value::Instance(inst)
                if crate::interpreter::builtins::xml::value::is_node_class(&inst.borrow().class) =>
            {
                let xml = crate::interpreter::builtins::xml::value::fingerprint(&inst.borrow())?;
                mix(h, b'X' as u64);
                mix_str(h, xml.as_bytes());
            }
            Value::Instance(_) if matches!(self.objects, Objects::Refuse) => return None,
            Value::Instance(object) => {
                self.enter(object)?;
                let object = object.try_borrow().ok()?;
                mix(h, b'O' as u64);
                mix_str(h, object.class.name.as_bytes());
                mix(h, object.fields.len() as u64);
                // A field map has no order of its own, so each field is hashed
                // apart and the results are summed: the same fields give the
                // same key in every process.
                let mut fields = 0u64;
                for (name, field) in object.fields.iter() {
                    let mut fh = FNV_OFFSET;
                    mix_str(&mut fh, name.as_bytes());
                    self.value(&mut fh, field)?;
                    fields = fields.wrapping_add(avalanche(fh));
                }
                mix(h, fields);
                self.leave();
            }
            // A grouped read once its batch has run is its result. Read
            // without forcing the batch: a pending one refuses.
            Value::Deferred(cell) => {
                self.enter(cell)?;
                let cell = cell.try_borrow().ok()?;
                self.value(h, cell.resolved.as_ref()?)?;
                self.leave();
            }
            Value::Function(f) => {
                mix(h, b'f' as u64);
                mix_str(h, f.name.as_bytes());
            }
            Value::NativeFunction(f) => {
                mix(h, b'n' as u64);
                mix_str(h, f.name.as_bytes());
            }
            Value::VmClosure(c) => {
                mix(h, b'v' as u64);
                mix_str(h, c.proto.name.as_bytes());
            }
            Value::Class(c) => {
                mix(h, b'C' as u64);
                mix_str(h, c.name.as_bytes());
            }
            Value::Method(m) => {
                self.enter(m)?;
                mix(h, b'm' as u64);
                mix_str(h, m.method_name.as_bytes());
                self.value(h, &m.receiver)?;
                self.leave();
            }
            Value::QueryBuilder(_)
            | Value::Future(_)
            | Value::Image(_)
            | Value::ImagePlan(_)
            | Value::Super(_)
            // An `XmlReader`'s stream (a node handle is hashed above).
            | Value::Xml(_)
            | Value::Breakpoint
            | Value::Continue => return None,
        }
        Some(())
    }
}

fn mix(h: &mut u64, word: u64) {
    *h ^= word;
    *h = h.wrapping_mul(FNV_PRIME);
}

fn mix_bytes(h: &mut u64, bytes: &[u8]) {
    for &b in bytes {
        mix(h, b as u64);
    }
}

/// A string with its length first, so that where one string ends and the
/// next value starts is part of the hash: without it `{"aS": ""}` and
/// `{"a": "S"}`, or the query strings `?qS=x` and `?q=Sx`, hashed the same.
fn mix_str(h: &mut u64, bytes: &[u8]) {
    mix(h, bytes.len() as u64);
    mix_bytes(h, bytes);
}

/// murmur3's 64-bit finalizer: spreads every input bit over the output, so
/// that summing the hashes of an object's fields does not cancel out.
fn avalanche(mut x: u64) -> u64 {
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51afd7ed558ccd);
    x ^= x >> 33;
    x = x.wrapping_mul(0xc4ceb9fe1a85ec53);
    x ^= x >> 33;
    x
}

fn fnv_hash_key(h: &mut u64, k: &HashKey) {
    // Match the per-variant tag bytes used by `HashKey::hash` so the
    // signature agrees with std's Hash impl on the same key.
    match k {
        HashKey::Int(n) => {
            mix(h, 0);
            mix(h, *n as u64);
        }
        HashKey::Decimal(d) => {
            mix(h, 1);
            // Decimals stringify deterministically across processes.
            mix_str(h, d.to_string().as_bytes());
        }
        HashKey::String(s) => {
            mix(h, 2);
            mix_str(h, s.as_bytes());
        }
        HashKey::Bool(b) => {
            mix(h, 3);
            mix(h, *b as u64);
        }
        HashKey::Null => mix(h, 4),
        HashKey::Symbol(s) => {
            mix(h, 5);
            mix_str(h, s.as_bytes());
        }
        HashKey::ValueObject(key) => {
            mix(h, 6);
            mix_str(h, key.class.as_bytes());
            mix_str(h, key.fields.as_bytes());
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
            false,
            SoliStr::from("old body"),
            SoliStr::new(),
        );
        assert_eq!(
            get(path.clone(), Some("application"), 42, false).map(|c| c.body),
            Some(SoliStr::from("old body"))
        );
        clear_cache();
        assert!(get(path, Some("application"), 42, false).is_none());
    }

    /// `t()` reads the locale from a thread-local, not from the render data,
    /// so a body rendered in one locale must not answer a request in another.
    #[test]
    fn the_locale_is_part_of_the_key() {
        use crate::interpreter::builtins::i18n::helpers::set_locale;
        reset_for_new_request();
        clear_cache();
        let path = Arc::new(PathBuf::from("app/views/mesures/index.html.slv"));

        set_locale("fr");
        put(
            path.clone(),
            Some("application"),
            7,
            false,
            SoliStr::from("Mesures"),
            SoliStr::new(),
        );
        set_locale("en");
        assert!(
            get(path.clone(), Some("application"), 7, false).is_none(),
            "an en request must not get the fr body"
        );
        set_locale("fr");
        assert_eq!(
            get(path, Some("application"), 7, false).map(|c| c.body),
            Some(SoliStr::from("Mesures"))
        );
        set_locale("");
    }

    /// A raw render body and the finished HTML response built from it share a
    /// site and a signature. Each must answer only its own kind of lookup: a
    /// raw hit served as a response would ship a page without its scripts or
    /// ETag, and a response served to `render()` would inject them twice.
    #[test]
    fn raw_and_finished_entries_do_not_collide() {
        reset_for_new_request();
        clear_cache();
        let path = Arc::new(PathBuf::from("app/views/home/index.html.slv"));

        put(
            path.clone(),
            Some("application"),
            5,
            false,
            SoliStr::from("raw"),
            SoliStr::new(),
        );
        assert!(get(path.clone(), Some("application"), 5, true).is_none());

        put(
            path.clone(),
            Some("application"),
            5,
            true,
            SoliStr::from("finished"),
            SoliStr::from("W/\"0123456789abcdef\""),
        );
        let Some(raw) = get(path.clone(), Some("application"), 5, false) else {
            panic!("the raw entry was evicted by the finished one");
        };
        let Some(finished) = get(path, Some("application"), 5, true) else {
            panic!("the finished entry is missing");
        };
        assert_eq!(raw.body, "raw");
        assert_eq!(finished.body, "finished");
        assert_eq!(finished.etag, "W/\"0123456789abcdef\"");
    }

    #[test]
    fn a_session_marks_the_request_personal() {
        use crate::interpreter::builtins::session::set_current_session_id;
        set_current_session_id(None);
        assert!(!request_has_session());
        set_current_session_id(Some("abc".into()));
        assert!(request_has_session());
        set_current_session_id(None);
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
            false,
            SoliStr::from("body"),
            SoliStr::new(),
        );

        assert!(
            is_known_uncacheable(&path, Some("application")),
            "the refused store must be remembered"
        );
        // The refusal itself must still hold: nothing was cached.
        reset_for_new_request();
        assert!(get(path.clone(), Some("application"), 7, false).is_none());

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
            false,
            SoliStr::from("rendered"),
            SoliStr::new(),
        );

        assert!(!is_known_uncacheable(&path, Some("docs")));
        assert_eq!(
            get(path, Some("docs"), 99, false).map(|c| c.body),
            Some(SoliStr::from("rendered"))
        );
    }

    // -- data signatures ------------------------------------------------------

    use crate::interpreter::value::{
        Class, DecimalValue, DeferredCell, HashPairs, Instance, NativeFunction,
    };

    fn string(s: &str) -> Value {
        Value::String(s.into())
    }

    fn array(items: Vec<Value>) -> Value {
        Value::Array(Rc::new(RefCell::new(items)))
    }

    fn hash(pairs: Vec<(&str, Value)>) -> Value {
        let mut map = HashPairs::default();
        for (key, value) in pairs {
            map.insert(HashKey::String(key.into()), value);
        }
        Value::Hash(Rc::new(RefCell::new(map)))
    }

    fn object(class: &str, fields: Vec<(&str, Value)>) -> Value {
        let class = Rc::new(Class {
            name: class.to_string(),
            ..Default::default()
        });
        let mut instance = Instance::new(class);
        for (name, value) in fields {
            instance.set(name, value);
        }
        Value::Instance(Rc::new(RefCell::new(instance)))
    }

    fn decimal(text: &str) -> Value {
        let precision = text
            .split('.')
            .nth(1)
            .map_or(0, |digits| digits.len() as u32);
        let parsed = text
            .parse()
            .unwrap_or_else(|e| panic!("{text} is a decimal: {e}"));
        Value::Decimal(DecimalValue(parsed, precision))
    }

    /// A record from `Post.all` (any object) has no signature, so its page is
    /// rendered and never answered from the cache. Two cards with different
    /// titles used to hash the same, by the length of their class name.
    #[test]
    fn an_object_in_the_data_makes_the_render_uncacheable() {
        let card = |title: &str| object("Card", vec![("title", string(title))]);
        assert_eq!(data_signature(&card("first")), None);
        assert_eq!(
            data_signature(&hash(vec![("cards", array(vec![card("first")]))])),
            None
        );
        assert_eq!(
            data_signature(&array(vec![string("x"), card("first"), string("y")])),
            None
        );

        // The same page from plain values keeps a signature that follows them.
        let plain = |title: &str| {
            hash(vec![(
                "cards",
                array(vec![hash(vec![("title", string(title))])]),
            )])
        };
        assert!(data_signature(&plain("first")).is_some());
        assert_eq!(
            data_signature(&plain("first")),
            data_signature(&plain("first"))
        );
        assert_ne!(
            data_signature(&plain("first")),
            data_signature(&plain("second"))
        );
    }

    /// `Decimal` and `DateTime` are values. They fell into the catch-all arm
    /// with the objects, so `@amount = 6.25D` was answered with the page of
    /// `3.25D`.
    #[test]
    fn decimals_and_datetimes_are_hashed_by_value() {
        let sig = |value: Value| {
            data_signature(&hash(vec![("v", value)]))
                .unwrap_or_else(|| panic!("a value has a signature"))
        };
        assert_ne!(sig(decimal("3.25")), sig(decimal("6.25")));
        assert_eq!(sig(decimal("3.25")), sig(decimal("3.25")));
        // Equal, but printed differently.
        assert_ne!(sig(decimal("1.5")), sig(decimal("1.50")));

        let day = 86_400_000_000_000i64;
        assert_ne!(
            sig(Value::DateTime(
                3 * day,
                crate::interpreter::value::DtZone::UTC
            )),
            sig(Value::DateTime(
                6 * day,
                crate::interpreter::value::DtZone::UTC
            ))
        );
        assert_eq!(
            sig(Value::DateTime(
                3 * day,
                crate::interpreter::value::DtZone::UTC
            )),
            sig(Value::DateTime(
                3 * day,
                crate::interpreter::value::DtZone::UTC
            ))
        );
        // One instant, shown in UTC or in the local zone.
        assert_ne!(
            sig(Value::DateTime(
                3 * day,
                crate::interpreter::value::DtZone::UTC
            )),
            sig(Value::DateTime(
                3 * day,
                crate::interpreter::value::DtZone::LOCAL
            ))
        );
    }

    /// A query the view will run and a grouped read still pending are not data
    /// yet. A grouped read whose batch has run is its result, read without
    /// forcing the batch (the old arm's `type_name()` flushed it).
    #[test]
    fn a_query_not_yet_run_or_a_pending_read_has_no_signature() {
        use crate::interpreter::builtins::model::QueryBuilder;

        let query = QueryBuilder::new("Post".to_string(), "posts".to_string());
        let query = Value::QueryBuilder(Rc::new(RefCell::new(query)));
        assert_eq!(data_signature(&hash(vec![("posts", query)])), None);

        let pending = Value::Deferred(Rc::new(RefCell::new(DeferredCell::default())));
        assert_eq!(data_signature(&hash(vec![("count", pending)])), None);

        let resolved = |n: i64| {
            Value::Deferred(Rc::new(RefCell::new(DeferredCell {
                resolved: Some(Value::Int(n)),
            })))
        };
        let sig = |value: Value| data_signature(&hash(vec![("count", value)]));
        assert!(sig(resolved(5)).is_some());
        assert_eq!(sig(resolved(5)), sig(Value::Int(5)));
        assert_ne!(sig(resolved(5)), sig(resolved(6)));
    }

    /// Code is hashed by name, and a page whose data holds it stays cacheable:
    /// the dev-mode and auto-render paths put every view helper in the data.
    #[test]
    fn code_is_hashed_by_name_and_stays_cacheable() {
        let helper = |name: &str| {
            Value::NativeFunction(NativeFunction::new(name, Some(0), |_| Ok(Value::Null)))
        };
        let class = |name: &str| {
            Value::Class(Rc::new(Class {
                name: name.to_string(),
                ..Default::default()
            }))
        };
        let sig = |value: Value| data_signature(&hash(vec![("code", value)]));
        assert!(sig(helper("link_to")).is_some());
        assert_eq!(sig(helper("link_to")), sig(helper("link_to")));
        assert_ne!(sig(helper("link_to")), sig(helper("pluralize")));
        assert!(sig(class("Post")).is_some());
        assert_ne!(sig(class("Post")), sig(class("User")));
    }

    /// Strings are hashed with their length. Without it, where a key ended and
    /// its value began was lost: `{"aS": ""}` and `{"a": "S"}`, or the query
    /// strings `?qS=x` and `?q=Sx`, shared a signature, so one request could be
    /// answered with the page rendered for the other.
    #[test]
    fn where_a_string_ends_is_part_of_the_signature() {
        let pair = |key: &str, value: &str| data_signature(&hash(vec![(key, string(value))]));
        assert_ne!(pair("aS", ""), pair("a", "S"));
        assert_ne!(pair("qS", "x"), pair("q", "Sx"));

        let symbol_key = |key: &str, value: &str| {
            let mut map = HashPairs::default();
            map.insert(HashKey::Symbol(key.into()), string(value));
            data_signature(&Value::Hash(Rc::new(RefCell::new(map))))
        };
        assert_ne!(symbol_key("aS", ""), symbol_key("a", "S"));
    }

    /// Data that contains itself is refused right away. Before, it recursed
    /// until the stack overflowed and the process aborted, and with two ways
    /// back at every level a depth limit alone would take 2^128 steps.
    #[test]
    fn data_that_contains_itself_is_refused_not_walked_forever() {
        let list = array(vec![]);
        if let Value::Array(items) = &list {
            items.borrow_mut().push(list.clone());
        }
        assert_eq!(data_signature(&list), None);

        let node = hash(vec![]);
        if let Value::Hash(map) = &node {
            let mut map = map.borrow_mut();
            map.insert(HashKey::String("left".into()), node.clone());
            map.insert(HashKey::String("right".into()), node.clone());
        }
        assert_eq!(data_signature(&node), None);
        assert_eq!(content_signature(&node), None);

        // Met twice, but never inside itself: that is not a cycle.
        let shared = array(vec![Value::Int(1)]);
        assert!(data_signature(&hash(vec![("a", shared.clone()), ("b", shared)])).is_some());

        // Rc cycles outlive the test unless broken.
        if let Value::Array(items) = &list {
            items.borrow_mut().clear();
        }
        if let Value::Hash(map) = &node {
            map.borrow_mut().clear();
        }
    }

    #[test]
    fn nesting_past_the_limit_is_refused() {
        let mut deep = Value::Int(1);
        for _ in 0..MAX_DEPTH {
            deep = array(vec![deep]);
        }
        assert!(
            data_signature(&deep).is_some(),
            "{MAX_DEPTH} nested containers still hash"
        );
        assert_eq!(data_signature(&array(vec![deep])), None);
    }

    /// The key of a component cached with `"cache": true` hashes an object by
    /// its class and its fields, whatever order its field map iterates in. The
    /// KV store is shared by every worker and process, so the key has to be the
    /// same everywhere.
    #[test]
    fn a_content_signature_hashes_an_object_by_class_and_fields() {
        let card = |title: &str| {
            object(
                "Card",
                vec![("title", string(title)), ("views", Value::Int(3))],
            )
        };
        let sig =
            |value: &Value| content_signature(value).unwrap_or_else(|| panic!("an object has one"));
        assert_eq!(sig(&card("alpha")), sig(&card("alpha")));
        assert_ne!(sig(&card("alpha")), sig(&card("beta")));

        // Each instance's field map has its own random iteration order: with
        // eight fields, an order-dependent hash would differ almost always.
        let names = ["a", "b", "c", "d", "e", "f", "g", "h"];
        let forward = names.iter().map(|name| (*name, string(name))).collect();
        let backward = names
            .iter()
            .rev()
            .map(|name| (*name, string(name)))
            .collect();
        assert_eq!(
            sig(&object("Card", forward)),
            sig(&object("Card", backward))
        );

        // Another class renders through other methods; a field under another
        // name is another record.
        let badge = object(
            "Badge",
            vec![("title", string("alpha")), ("views", Value::Int(3))],
        );
        assert_ne!(sig(&card("alpha")), sig(&badge));
        let renamed = object(
            "Card",
            vec![("name", string("alpha")), ("views", Value::Int(3))],
        );
        assert_ne!(sig(&card("alpha")), sig(&renamed));

        // A nested record counts by its fields as well.
        let post = |author: &str| {
            object(
                "Post",
                vec![("author", object("User", vec![("name", string(author))]))],
            )
        };
        assert_ne!(sig(&post("Ana")), sig(&post("Bo")));

        // A parent and child pointing at each other are refused, not walked.
        let parent = object("Node", vec![]);
        let child = object("Node", vec![("parent", parent.clone())]);
        if let Value::Instance(instance) = &parent {
            instance
                .borrow_mut()
                .set("children", array(vec![child.clone(), child]));
        }
        assert_eq!(content_signature(&parent), None);
        if let Value::Instance(instance) = &parent {
            instance.borrow_mut().fields.clear();
        }

        // The response cache still refuses every one of them.
        assert_eq!(data_signature(&card("alpha")), None);
    }
}
