//! Reactive live queries.
//!
//! `Model.live_where(filter)` called inside a LiveView handler runs the query
//! like `where(filter).all()` **and** subscribes that LiveView to the queried
//! collection. A later write to the collection wakes every subscribed LiveView
//! to re-render — the LiveView's diff gate drops the frame if nothing visible
//! changed, so an over-broad wake is harmless.
//!
//! v1 semantics are **per-collection**: any write to `posts` wakes every
//! LiveView that subscribed to `posts`. Per-document filter matching (evaluate
//! the `filter` against the changed row before waking) is a documented v2.
//!
//! Everything here is process-global state (workers are threads in one
//! process), so cross-worker subscriptions work for free. **Cross-process is an
//! explicit non-goal for v1** — a multi-process deployment needs an external
//! bus, which this module deliberately does not provide.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

/// `collection` → (`liveview_id` → `component`). The inner map de-dupes so a
/// LiveView re-subscribing on every render (the normal case) stays idempotent.
static SUBSCRIPTIONS: LazyLock<Mutex<HashMap<String, HashMap<String, String>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

thread_local! {
    /// The LiveView currently rendering on this worker thread, as
    /// `(liveview_id, component)`. Set by [`set_current`] around a handler call
    /// so `live_where` knows who to subscribe; `None` in a plain controller.
    static CURRENT_LIVEVIEW: RefCell<Option<(String, String)>> = const { RefCell::new(None) };
}

/// RAII guard that marks the current LiveView for the duration of a handler
/// call. Dropping it clears the thread-local, so a query in a subsequent plain
/// request never accidentally subscribes.
#[must_use]
pub struct CurrentLiveViewGuard;

impl Drop for CurrentLiveViewGuard {
    fn drop(&mut self) {
        CURRENT_LIVEVIEW.with(|c| *c.borrow_mut() = None);
    }
}

/// Mark `(liveview_id, component)` as the LiveView rendering on this thread.
/// Hold the returned guard for the handler call; it clears on drop.
pub fn set_current(liveview_id: String, component: String) -> CurrentLiveViewGuard {
    CURRENT_LIVEVIEW.with(|c| *c.borrow_mut() = Some((liveview_id, component)));
    CurrentLiveViewGuard
}

/// Whether a LiveView render is currently in scope on this thread.
pub fn is_active() -> bool {
    CURRENT_LIVEVIEW.with(|c| c.borrow().is_some())
}

/// Subscribe the current LiveView (if any) to `collection`. A no-op outside a
/// LiveView render, and idempotent within one.
pub fn subscribe(collection: &str) {
    let Some((liveview_id, component)) = CURRENT_LIVEVIEW.with(|c| c.borrow().clone()) else {
        return;
    };
    let mut subs = SUBSCRIPTIONS.lock().unwrap();
    subs.entry(collection.to_string())
        .or_default()
        .insert(liveview_id, component);
}

/// Drop a LiveView from every collection it subscribed to. Called when a
/// LiveView disconnects or is reaped, so a stale id can't keep waking.
pub fn unsubscribe_all(liveview_id: &str) {
    let mut subs = SUBSCRIPTIONS.lock().unwrap();
    for members in subs.values_mut() {
        members.remove(liveview_id);
    }
    subs.retain(|_, members| !members.is_empty());
}

/// The subscribers for a collection, as `(liveview_id, component)` pairs.
/// Empty when nothing is subscribed (the common case — a cheap map lookup).
fn subscribers(collection: &str) -> Vec<(String, String)> {
    let subs = SUBSCRIPTIONS.lock().unwrap();
    match subs.get(collection) {
        Some(members) if !members.is_empty() => members
            .iter()
            .map(|(id, comp)| (id.clone(), comp.clone()))
            .collect(),
        _ => Vec::new(),
    }
}

/// Wake every LiveView subscribed to `collection` after a write, by enqueuing a
/// synthetic `live_query_changed` event onto the LiveView event bus. No-op when
/// nothing is subscribed. Safe to call from the DB write path (sync, no await).
pub fn notify_change(collection: &str) {
    let subscribers = subscribers(collection);
    if subscribers.is_empty() {
        return;
    }
    crate::serve::enqueue_live_query_changed(subscribers);
}

#[cfg(test)]
mod tests {
    use super::*;

    // The subscription map is a process-global static shared across tests; use
    // collection names unique to each test so parallel runs don't interfere.

    #[test]
    fn subscribe_is_noop_without_current_liveview() {
        subscribe("lq_test_no_current");
        assert!(subscribers("lq_test_no_current").is_empty());
    }

    #[test]
    fn subscribe_records_current_liveview_and_dedupes() {
        let guard = set_current("sess:posts".to_string(), "posts".to_string());
        subscribe("lq_test_posts");
        subscribe("lq_test_posts"); // idempotent
        let subs = subscribers("lq_test_posts");
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0], ("sess:posts".to_string(), "posts".to_string()));
        drop(guard);
        // Guard cleared the thread-local: a further subscribe is a no-op.
        subscribe("lq_test_posts_after");
        assert!(subscribers("lq_test_posts_after").is_empty());
    }

    #[test]
    fn unsubscribe_all_removes_from_every_collection() {
        let guard = set_current("sess:multi".to_string(), "multi".to_string());
        subscribe("lq_test_a");
        subscribe("lq_test_b");
        drop(guard);
        assert_eq!(subscribers("lq_test_a").len(), 1);
        assert_eq!(subscribers("lq_test_b").len(), 1);
        unsubscribe_all("sess:multi");
        assert!(subscribers("lq_test_a").is_empty());
        assert!(subscribers("lq_test_b").is_empty());
    }
}
