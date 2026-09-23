//! Per-request memo of the session documents a network session store loaded.
//!
//! The `solidb` and `solikv` stores keep the whole session in one remote
//! document and used to fetch it on every call: resolving the cookie's session,
//! reading the stored locale, the CSRF token and each `session_get` were a
//! network round-trip apiece, all for the same document within one request.
//! While a request is being served, the first load is remembered here and the
//! later reads answer from it.
//!
//! - **Active only inside a request** ([`RequestSessionCache::begin`], bound by
//!   the request handler). Outside one — a WebSocket frame, a job, a background
//!   thread — nothing is remembered and every call reaches the store as before,
//!   so a thread reused across requests never sees another request's data.
//! - **Writes still read and write the store.** `set`/`delete` keep their fresh
//!   load-modify-save (so the window for losing a concurrent request's write is
//!   what it was), then either refresh the memo with what they saved (SoliKV,
//!   whose `SET` stores exactly that) or drop the entry so the next read
//!   reloads it (SoliDB, whose update merges). `destroy` drops the entry;
//!   `regenerate` remembers the session it created.
//! - Only successful loads are remembered: a network error is never mistaken
//!   for "no such session".
//! - Entries are keyed by the store's address as well as the session id, so a
//!   store swapped by `session_configure` mid-request starts cold.

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;

type Entries = HashMap<(usize, String), Option<Box<dyn Any>>>;

thread_local! {
    /// `None` outside a request: the memo is off.
    static CACHE: RefCell<Option<Entries>> = const { RefCell::new(None) };
}

/// Turns the memo on for the life of a request. Bind it for the whole request
/// and let it drop at the end; the drop empties the memo and turns it off.
pub struct RequestSessionCache {
    _private: (),
}

impl RequestSessionCache {
    pub fn begin() -> Self {
        CACHE.with(|cache| *cache.borrow_mut() = Some(HashMap::new()));
        RequestSessionCache { _private: () }
    }
}

impl Drop for RequestSessionCache {
    fn drop(&mut self) {
        CACHE.with(|cache| *cache.borrow_mut() = None);
    }
}

/// Look the session up in the memo. `None`: the memo is off or has not seen
/// this session — load it. `Some(r)`: `f` ran against the remembered document
/// (`None` when the session is known not to exist).
pub(crate) fn with_cached<T: 'static, R>(
    store: usize,
    session_id: &str,
    f: impl FnOnce(Option<&T>) -> R,
) -> Option<R> {
    CACHE.with(|cache| {
        let cache = cache.borrow();
        let entries = cache.as_ref()?;
        let entry = entries.get(&(store, session_id.to_string()))?;
        match entry {
            None => Some(f(None)),
            Some(doc) => (**doc).downcast_ref::<T>().map(|doc| f(Some(doc))),
        }
    })
}

/// Remember what the store holds for this session (`None`: it has none).
/// A no-op while the memo is off.
pub(crate) fn remember<T: 'static>(store: usize, session_id: &str, doc: Option<T>) {
    CACHE.with(|cache| {
        if let Some(entries) = cache.borrow_mut().as_mut() {
            entries.insert(
                (store, session_id.to_string()),
                doc.map(|doc| Box::new(doc) as Box<dyn Any>),
            );
        }
    });
}

/// Drop what the memo holds for this session, so the next read loads it.
pub(crate) fn forget(store: usize, session_id: &str) {
    CACHE.with(|cache| {
        if let Some(entries) = cache.borrow_mut().as_mut() {
            entries.remove(&(store, session_id.to_string()));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remembers_only_inside_a_request() {
        remember(1, "a", Some(5_i32));
        assert!(with_cached::<i32, _>(1, "a", |doc| doc.copied()).is_none());
        {
            let _scope = RequestSessionCache::begin();
            remember(1, "a", Some(5_i32));
            remember::<i32>(1, "gone", None);
            assert_eq!(
                with_cached::<i32, _>(1, "a", |doc| doc.copied()),
                Some(Some(5))
            );
            assert_eq!(
                with_cached::<i32, _>(1, "gone", |doc| doc.copied()),
                Some(None)
            );
            assert!(with_cached::<i32, _>(2, "a", |doc| doc.copied()).is_none());
            forget(1, "a");
            assert!(with_cached::<i32, _>(1, "a", |doc| doc.copied()).is_none());
        }
        assert!(with_cached::<i32, _>(1, "gone", |doc| doc.copied()).is_none());
    }
}
