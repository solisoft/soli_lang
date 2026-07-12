//! Reactive live queries.
//!
//! `Model.live_where(filter)` called inside a LiveView handler runs the query
//! like `where(filter).all()` **and** subscribes that LiveView to the queried
//! collection. A later write to the collection wakes subscribed LiveViews to
//! re-render — the LiveView's diff gate drops the frame if nothing visible
//! changed, so an over-broad wake is harmless.
//!
//! **Per-row matching.** A flat-equality hash filter (`{"published": true}`) is
//! kept alongside the subscription as its field→value map. On a write we test
//! the changed row against it and wake only subscribers it satisfies. Filters we
//! can't decompose — the string form (`live_where("doc.x > @y", ...)`), deletes
//! (no row), and transaction commits (only collection names survive) — record a
//! `None` matcher and wake conservatively, exactly as the old per-collection v1.
//!
//! Everything here is process-global state (workers are threads in one
//! process), so cross-worker subscriptions work for free. **Cross-process is an
//! explicit non-goal for v1** — a multi-process deployment needs an external
//! bus, which this module deliberately does not provide.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use serde_json::Value as Json;

/// A flat field→value equality filter (the `bind_vars` of a hash-form query).
/// `None` means "not decomposable — wake unconditionally".
pub type Matcher = Option<HashMap<String, Json>>;

/// One LiveView's subscription to a collection: which component to re-render,
/// plus the filters it registered this session. A LiveView can `live_where` the
/// same collection more than once, so matchers is a (deduped, bounded) list —
/// wake if ANY of them matches.
struct Sub {
    component: String,
    matchers: Vec<Matcher>,
}

/// Cap on retained matchers per (collection, liveview). A handler with a stable
/// filter dedups to one; this only bounds the pathological case of a filter that
/// changes every render, whose stale entries would otherwise accumulate (each is
/// harmless — an extra wake the diff gate absorbs).
const MAX_MATCHERS: usize = 16;

/// `collection` → (`liveview_id` → `Sub`). The inner map de-dupes by liveview so
/// re-subscribing on every render (the normal case) stays idempotent.
static SUBSCRIPTIONS: LazyLock<Mutex<HashMap<String, HashMap<String, Sub>>>> =
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

/// Subscribe the current LiveView (if any) to `collection` with an optional
/// per-row `matcher`. A no-op outside a LiveView render. Idempotent: an
/// identical matcher on the same (collection, liveview) is not duplicated.
pub fn subscribe(collection: &str, matcher: Matcher) {
    let Some((liveview_id, component)) = CURRENT_LIVEVIEW.with(|c| c.borrow().clone()) else {
        return;
    };
    let mut subs = SUBSCRIPTIONS.lock().unwrap();
    let sub = subs
        .entry(collection.to_string())
        .or_default()
        .entry(liveview_id)
        .or_insert_with(|| Sub {
            component,
            matchers: Vec::new(),
        });
    if !sub.matchers.contains(&matcher) {
        if sub.matchers.len() >= MAX_MATCHERS {
            sub.matchers.remove(0);
        }
        sub.matchers.push(matcher);
    }
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

/// Numeric-aware scalar equality between a document field and a matcher value.
/// `null` in the matcher matches a missing or null field; numbers compare by
/// value (so a bound `5` matches a stored `5.0`).
fn field_matches(field: Option<&Json>, expected: &Json) -> bool {
    match (field, expected) {
        (None, Json::Null) => true,
        (Some(Json::Null), Json::Null) => true,
        (Some(a), b) if a.is_number() && b.is_number() => a.as_f64() == b.as_f64(),
        (Some(a), b) => a == b,
        (None, _) => false,
    }
}

/// True if `doc` satisfies every field of `matcher` (a conjunction of equalities).
fn doc_matches(matcher: &HashMap<String, Json>, doc: &Json) -> bool {
    matcher
        .iter()
        .all(|(field, expected)| field_matches(doc.get(field), expected))
}

/// True if a subscription with this matcher should wake for `changed`.
/// Conservative (wake) when the matcher is `None` or there's no changed row.
fn matcher_wakes(matcher: &Matcher, changed: Option<&Json>) -> bool {
    match (matcher, changed) {
        (None, _) => true,
        (_, None) => true,
        (Some(m), Some(doc)) => doc_matches(m, doc),
    }
}

/// The `(liveview_id, component)` subscribers of `collection` that should wake
/// for `changed`. Empty when nothing is subscribed (the common case).
fn subscribers_to_wake(collection: &str, changed: Option<&Json>) -> Vec<(String, String)> {
    let subs = SUBSCRIPTIONS.lock().unwrap();
    let Some(members) = subs.get(collection) else {
        return Vec::new();
    };
    members
        .iter()
        .filter(|(_, sub)| sub.matchers.iter().any(|m| matcher_wakes(m, changed)))
        .map(|(id, sub)| (id.clone(), sub.component.clone()))
        .collect()
}

/// Wake the LiveViews subscribed to `collection` whose live query matches the
/// written row `changed` (all of them when `changed` is `None`), by enqueuing a
/// synthetic `live_query_changed` event onto the LiveView event bus. No-op when
/// nothing matches. Safe to call from the DB write path (sync, no await).
pub fn notify_change(collection: &str, changed: Option<&Json>) {
    let subscribers = subscribers_to_wake(collection, changed);
    if subscribers.is_empty() {
        return;
    }
    crate::serve::enqueue_live_query_changed(subscribers);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // The subscription map is a process-global static shared across tests; use
    // collection names unique to each test so parallel runs don't interfere.

    fn eq_matcher(pairs: &[(&str, Json)]) -> Matcher {
        Some(
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
        )
    }

    #[test]
    fn subscribe_is_noop_without_current_liveview() {
        subscribe("lq_test_no_current", None);
        assert!(subscribers_to_wake("lq_test_no_current", None).is_empty());
    }

    #[test]
    fn subscribe_records_current_liveview_and_dedupes() {
        let guard = set_current("sess:posts".to_string(), "posts".to_string());
        subscribe("lq_test_posts", None);
        subscribe("lq_test_posts", None); // idempotent
        let subs = subscribers_to_wake("lq_test_posts", None);
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0], ("sess:posts".to_string(), "posts".to_string()));
        drop(guard);
        subscribe("lq_test_posts_after", None);
        assert!(subscribers_to_wake("lq_test_posts_after", None).is_empty());
    }

    #[test]
    fn per_row_matcher_wakes_only_matching_rows() {
        let guard = set_current("sess:pr".to_string(), "board".to_string());
        subscribe("lq_test_pr", eq_matcher(&[("published", json!(true))]));
        drop(guard);
        // A published row wakes; an unpublished one does not.
        assert_eq!(
            subscribers_to_wake("lq_test_pr", Some(&json!({"published": true, "id": 1}))).len(),
            1
        );
        assert!(subscribers_to_wake("lq_test_pr", Some(&json!({"published": false}))).is_empty());
        // No changed row (delete / tx) -> conservative wake.
        assert_eq!(subscribers_to_wake("lq_test_pr", None).len(), 1);
    }

    #[test]
    fn none_matcher_always_wakes() {
        let guard = set_current("sess:any".to_string(), "any".to_string());
        subscribe("lq_test_any", None); // string-form filter -> conservative
        drop(guard);
        assert_eq!(
            subscribers_to_wake("lq_test_any", Some(&json!({"whatever": 1}))).len(),
            1
        );
    }

    #[test]
    fn numeric_equality_is_value_based() {
        let guard = set_current("sess:num".to_string(), "num".to_string());
        subscribe("lq_test_num", eq_matcher(&[("author_id", json!(5))]));
        drop(guard);
        // stored 5.0 matches bound 5
        assert_eq!(
            subscribers_to_wake("lq_test_num", Some(&json!({"author_id": 5.0}))).len(),
            1
        );
        assert!(subscribers_to_wake("lq_test_num", Some(&json!({"author_id": 6}))).is_empty());
    }

    #[test]
    fn unsubscribe_all_removes_from_every_collection() {
        let guard = set_current("sess:multi".to_string(), "multi".to_string());
        subscribe("lq_test_a", None);
        subscribe("lq_test_b", None);
        drop(guard);
        assert_eq!(subscribers_to_wake("lq_test_a", None).len(), 1);
        assert_eq!(subscribers_to_wake("lq_test_b", None).len(), 1);
        unsubscribe_all("sess:multi");
        assert!(subscribers_to_wake("lq_test_a", None).is_empty());
        assert!(subscribers_to_wake("lq_test_b", None).is_empty());
    }
}
