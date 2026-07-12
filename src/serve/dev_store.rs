//! Process-wide ring buffer of recent per-request dev snapshots.
//!
//! The dev bar's "requests" panel lists every route a page touched. Clicking a
//! row inspects that request's panels (db / http / kv / flame), but those
//! XHR/HTMx sub-requests are separate responses that never carry a dev bar — so
//! their per-request detail would otherwise be discarded. In `--dev`, each
//! request stashes its [`DevBarContext`] snapshot here keyed by a request id,
//! and the `/__solidev/request/:id` endpoint re-renders it on demand.
//!
//! Bounded to [`CAP`] entries (oldest evicted) so a long-lived dev server can't
//! grow without limit. Dev-only: nothing writes here unless `--dev` is on.

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

use crate::serve::dev_bar::DevBarContext;

/// Max snapshots retained. A page plus its handful of XHR/HTMx calls fits many
/// times over; older requests age out.
const CAP: usize = 64;

fn store() -> &'static Mutex<VecDeque<(String, DevBarContext)>> {
    static STORE: OnceLock<Mutex<VecDeque<(String, DevBarContext)>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(VecDeque::with_capacity(CAP)))
}

/// Record a request's snapshot, evicting the oldest once at capacity.
pub fn put(id: String, ctx: DevBarContext) {
    let mut q = store().lock().unwrap();
    if q.len() >= CAP {
        q.pop_front();
    }
    q.push_back((id, ctx));
}

/// Fetch a stored snapshot by request id (clone; `None` if aged out / unknown).
pub fn get(id: &str) -> Option<DevBarContext> {
    let q = store().lock().unwrap();
    q.iter()
        .rev()
        .find(|(rid, _)| rid == id)
        .map(|(_, ctx)| ctx.clone())
}

/// The raw wire fields of a captured request, enough to re-dispatch it through
/// the real worker path. Stored alongside the [`DevBarContext`] snapshot (keyed
/// by the same request id) so the dev bar's "replay" button can reproduce a bug
/// server-side. Dev-only; nothing captures this in production.
#[derive(Clone)]
pub struct RawRequest {
    pub method: String,
    pub path: String,
    pub query: Vec<(String, String)>,
    pub headers: hyper::header::HeaderMap,
    pub body: String,
    pub peer_ip: String,
}

fn raw_store() -> &'static Mutex<VecDeque<(String, RawRequest)>> {
    static STORE: OnceLock<Mutex<VecDeque<(String, RawRequest)>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(VecDeque::with_capacity(CAP)))
}

/// Record a request's raw fields for later replay, evicting the oldest at capacity.
pub fn put_raw(id: String, raw: RawRequest) {
    let mut q = raw_store().lock().unwrap();
    if q.len() >= CAP {
        q.pop_front();
    }
    q.push_back((id, raw));
}

/// Fetch a captured raw request by id (clone; `None` if aged out / unknown).
pub fn get_raw(id: &str) -> Option<RawRequest> {
    let q = raw_store().lock().unwrap();
    q.iter()
        .rev()
        .find(|(rid, _)| rid == id)
        .map(|(_, raw)| raw.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_raw(path: &str) -> RawRequest {
        RawRequest {
            method: "POST".to_string(),
            path: path.to_string(),
            query: vec![("q".to_string(), "1".to_string())],
            headers: hyper::header::HeaderMap::new(),
            body: "hello".to_string(),
            peer_ip: "127.0.0.1".to_string(),
        }
    }

    #[test]
    fn raw_round_trips_by_id() {
        put_raw("replay-round-trip-id".to_string(), sample_raw("/posts"));
        let got = get_raw("replay-round-trip-id").expect("stored raw request");
        assert_eq!(got.method, "POST");
        assert_eq!(got.path, "/posts");
        assert_eq!(got.body, "hello");
        assert_eq!(got.query, vec![("q".to_string(), "1".to_string())]);
    }

    #[test]
    fn raw_unknown_id_is_none() {
        assert!(get_raw("replay-never-stored-id").is_none());
    }
}
