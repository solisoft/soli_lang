//! What the last render cost, for an application's own dev bar.
//!
//! The HTML dev bar is spliced into the document on its way out (`dev_bar`).
//! EUI has no document to splice into: the application composes its own tree,
//! so a bar there is a widget like any other, and this is where it gets its
//! numbers. They are the *previous* render's, which is what a bar reports —
//! the work that produced what you are looking at.
//!
//! What it counts is what EUI costs and nothing else: the view, the diff, the
//! ops and bytes that went on the wire, and the four tables a session interns
//! once. Nothing here reaches the client, and nothing the client knows —
//! frame time, quads, memory — reaches here: spec 08 says the window reports
//! nothing about the machine beyond its viewport, and a dev bar is not a
//! reason to change that.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Mutex;

/// One session's last render.
#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub event: String,
    pub view_ms: f64,
    pub encode_ms: f64,
    pub batches: usize,
    pub ops: usize,
    pub bytes: usize,
    pub seq: u64,
    pub nodes: usize,
    pub atoms: usize,
    pub styles: usize,
    pub colors: usize,
    pub chunks: usize,
    /// Renders since the session connected, this one included.
    pub renders: u64,
}

static STATS: Mutex<Option<HashMap<String, Stats>>> = Mutex::new(None);

thread_local! {
    /// Which session this thread is rendering, so `eui_stats()` called from
    /// inside a view knows whose numbers to hand back.
    static CURRENT: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Mark the session a view is about to be called for.
pub fn set_current(session: Option<String>) {
    CURRENT.with(|c| *c.borrow_mut() = session);
}

/// The session this thread is rendering, for as long as the guard lives —
/// then none. Left set, the next view on this worker, EUI or not, read the
/// last session's numbers.
pub struct Current;

impl Current {
    pub fn enter(session: String) -> Self {
        set_current(Some(session));
        Self
    }
}

impl Drop for Current {
    fn drop(&mut self) {
        set_current(None);
    }
}

/// Keep what a render cost. `renders` counts up from whatever was there.
pub fn record(session: &str, mut stats: Stats) {
    let mut guard = STATS.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    stats.renders = map
        .get(session)
        .map_or(1, |old| old.renders.saturating_add(1));
    map.insert(session.to_owned(), stats);
}

/// The last render of the session this thread is rendering, if any.
pub fn current() -> Option<Stats> {
    let session = CURRENT.with(|c| c.borrow().clone())?;
    let guard = STATS.lock().unwrap_or_else(|e| e.into_inner());
    guard.as_ref()?.get(&session).cloned()
}

/// A session that has gone: its numbers go with it.
pub fn forget(session: &str) {
    let mut guard = STATS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(map) = guard.as_mut() {
        map.remove(session);
    }
}
