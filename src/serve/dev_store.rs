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
