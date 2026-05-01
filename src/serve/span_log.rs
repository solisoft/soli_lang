//! Per-request hierarchical span store for the dev-bar flamegraph.
//!
//! Every other instrumentation log (`phase_log`, `middleware_log`,
//! `query_log`, `http_log`) is flat — it tells you "middleware took 12 ms"
//! and "this AQL took 3 ms" but nothing about *what was nested inside what*.
//! `span_log` is the tree on top: each entry has a `parent` link, so the
//! dev bar can render a flame chart showing partials nested inside views
//! nested inside controller actions, etc.
//!
//! Cheap when dev mode is off — every public entry point checks the
//! `ENABLED` atomic before doing any work, mirroring `phase_log`.
//!
//! Design notes:
//! - All state is thread-local. Soli's worker model owns one interpreter +
//!   one set of these thread-locals per worker; `clear()` is called at the
//!   start of every request.
//! - `SpanGuard` is the RAII entry point — construction pushes the new
//!   span's id onto the open-span stack; drop pops it and emits the record.
//!   This mirrors `phase_log::PhaseTimer` and is the standard way to
//!   instrument paired enter/exit points.
//! - `record(...)` is a one-shot variant for callers that already have
//!   start/end times in hand (e.g. `query_log` and `http_log` already
//!   captured a duration; we just shadow that as a span without re-timing).
//! - `open_fn` / `close_fn` are the deep-mode pair used inside
//!   `Interpreter::push_frame` / `pop_frame`, where the call stack
//!   already enforces strict pairing so a Drop guard is unnecessary.

use std::cell::{Cell, RefCell};
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpanKind {
    Request,
    Middleware,
    BeforeAction,
    Action,
    AfterAction,
    View,
    Partial,
    Db,
    Http,
    Fn,
}

impl SpanKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SpanKind::Request => "request",
            SpanKind::Middleware => "middleware",
            SpanKind::BeforeAction => "before_action",
            SpanKind::Action => "action",
            SpanKind::AfterAction => "after_action",
            SpanKind::View => "view",
            SpanKind::Partial => "partial",
            SpanKind::Db => "db",
            SpanKind::Http => "http",
            SpanKind::Fn => "fn",
        }
    }
}

#[derive(Clone, Debug)]
pub struct SpanRecord {
    pub id: u32,
    pub parent: Option<u32>,
    pub name: String,
    pub kind: SpanKind,
    /// Microseconds since the request started.
    pub start_us: u64,
    pub end_us: u64,
    /// Optional one-line detail (AQL template, URL, …). Shown in tooltip
    /// and exported as a `args.meta` field in the trace JSON.
    pub meta: Option<String>,
}

// Worker-thread-local enable flag. Each worker calls `set_enabled` once
// at boot based on `--dev`; tests can flip it freely without racing
// other tests on other threads.
thread_local! {
    static ENABLED: Cell<bool> = const { Cell::new(false) };
    static LOG: RefCell<Vec<SpanRecord>> = const { RefCell::new(Vec::new()) };
    static STACK: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
    static REQ_START: Cell<Option<Instant>> = const { Cell::new(None) };
    static NEXT_ID: Cell<u32> = const { Cell::new(0) };
}

pub fn set_enabled(enabled: bool) {
    ENABLED.with(|e| e.set(enabled));
}

#[inline]
pub fn is_enabled() -> bool {
    ENABLED.with(|e| e.get())
}

/// Start a new request: zero the log, the open-span stack, the id
/// counter, and stash the request-start instant so every span can encode
/// its time as microseconds-since-request-start.
pub fn begin_request(start: Instant) {
    LOG.with(|l| l.borrow_mut().clear());
    STACK.with(|s| s.borrow_mut().clear());
    FN_STACK.with(|f| f.borrow_mut().clear());
    NEXT_ID.with(|n| n.set(0));
    REQ_START.with(|t| t.set(Some(start)));
}

pub fn clear() {
    LOG.with(|l| l.borrow_mut().clear());
    STACK.with(|s| s.borrow_mut().clear());
    FN_STACK.with(|f| f.borrow_mut().clear());
    NEXT_ID.with(|n| n.set(0));
    REQ_START.with(|t| t.set(None));
}

pub fn snapshot() -> Vec<SpanRecord> {
    LOG.with(|l| l.borrow().clone())
}

fn next_id() -> u32 {
    NEXT_ID.with(|n| {
        let id = n.get();
        n.set(id.wrapping_add(1));
        id
    })
}

fn current_parent() -> Option<u32> {
    STACK.with(|s| s.borrow().last().copied())
}

fn us_since_start(t: Instant) -> u64 {
    REQ_START.with(|r| match r.get() {
        Some(start) => t.saturating_duration_since(start).as_micros() as u64,
        // No request-start anchor — fall back to 0. Caller still gets
        // sensible duration via end_us - start_us.
        None => 0,
    })
}

/// One-shot record for callers that already have a start instant + duration
/// (e.g. query_log / http_log fire after the operation completes). Honours
/// the current open-span stack for parent linkage.
pub fn record(name: &str, kind: SpanKind, start: Instant, dur_us: u64, meta: Option<String>) {
    if !is_enabled() {
        return;
    }
    let start_us = us_since_start(start);
    let end_us = start_us.saturating_add(dur_us);
    let id = next_id();
    let parent = current_parent();
    LOG.with(|l| {
        l.borrow_mut().push(SpanRecord {
            id,
            parent,
            name: name.to_string(),
            kind,
            start_us,
            end_us,
            meta,
        })
    });
}

// Deep-mode (`fn:<name>` per Soli function call) hooks. Mirrors
// `Interpreter::push_frame` / `pop_frame` strict pairing: every
// `push_fn(name)` must be followed by exactly one `pop_fn()`.
// Cheaper than SpanGuard because the call sites already guarantee
// matched push/pop, so no Drop guard is needed.

thread_local! {
    static FN_STACK: RefCell<Vec<OpenFn>> = const { RefCell::new(Vec::new()) };
}

struct OpenFn {
    id: u32,
    parent: Option<u32>,
    name: String,
    meta: Option<String>,
    start: Instant,
}

pub fn push_fn(name: &str, meta: Option<String>) {
    if !is_enabled() {
        return;
    }
    let id = next_id();
    let parent = current_parent();
    // Anonymous lambdas / closures push frames with an empty `func.name`
    // — surface them as `<anonymous>` so the flamegraph shows something
    // useful instead of a blank rectangle.
    let display_name = if name.is_empty() {
        "<anonymous>".to_string()
    } else {
        name.to_string()
    };
    STACK.with(|s| s.borrow_mut().push(id));
    FN_STACK.with(|f| {
        f.borrow_mut().push(OpenFn {
            id,
            parent,
            name: display_name,
            meta,
            start: Instant::now(),
        })
    });
}

pub fn pop_fn() {
    if !is_enabled() {
        return;
    }
    let Some(open) = FN_STACK.with(|f| f.borrow_mut().pop()) else {
        return;
    };
    let end_us = us_since_start(Instant::now());
    let start_us = us_since_start(open.start);
    STACK.with(|s| {
        let mut stack = s.borrow_mut();
        if stack.last().copied() == Some(open.id) {
            stack.pop();
        }
    });
    LOG.with(|l| {
        l.borrow_mut().push(SpanRecord {
            id: open.id,
            parent: open.parent,
            name: open.name,
            kind: SpanKind::Fn,
            start_us,
            end_us,
            meta: open.meta,
        })
    });
}

/// RAII span guard. Construction pushes a new id onto the open-span
/// stack; drop pops it and emits the SpanRecord. No-op when dev mode is
/// off — `start` returns a guard whose `id` is `u32::MAX` and whose
/// `start` is `None`, and Drop short-circuits.
pub struct SpanGuard {
    id: u32,
    parent: Option<u32>,
    name: String,
    kind: SpanKind,
    meta: Option<String>,
    start: Option<Instant>,
}

impl SpanGuard {
    pub fn start(name: &str, kind: SpanKind) -> Self {
        Self::start_with_meta(name, kind, None)
    }

    pub fn start_with_meta(name: &str, kind: SpanKind, meta: Option<String>) -> Self {
        if !is_enabled() {
            return Self {
                id: u32::MAX,
                parent: None,
                name: String::new(),
                kind,
                meta: None,
                start: None,
            };
        }
        let id = next_id();
        let parent = current_parent();
        STACK.with(|s| s.borrow_mut().push(id));
        Self {
            id,
            parent,
            name: name.to_string(),
            kind,
            meta,
            start: Some(Instant::now()),
        }
    }
}

impl Drop for SpanGuard {
    fn drop(&mut self) {
        let Some(start) = self.start else { return };
        if !is_enabled() {
            return;
        }
        let start_us = us_since_start(start);
        let end_us = us_since_start(Instant::now());
        STACK.with(|s| {
            let mut stack = s.borrow_mut();
            if stack.last().copied() == Some(self.id) {
                stack.pop();
            }
        });
        LOG.with(|l| {
            l.borrow_mut().push(SpanRecord {
                id: self.id,
                parent: self.parent,
                name: std::mem::take(&mut self.name),
                kind: self.kind,
                start_us,
                end_us,
                meta: std::mem::take(&mut self.meta),
            })
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() {
        clear();
        set_enabled(true);
        begin_request(Instant::now());
    }

    #[test]
    fn no_op_when_disabled() {
        clear();
        set_enabled(false);
        begin_request(Instant::now());
        let _g = SpanGuard::start("noop", SpanKind::Action);
        record(
            "also-noop",
            SpanKind::Db,
            Instant::now(),
            42,
            Some("x".into()),
        );
        push_fn("foo", None);
        pop_fn();
        drop(_g);
        assert!(snapshot().is_empty(), "no spans should be recorded");
    }

    #[test]
    fn nested_guards_link_parent() {
        fresh();
        let outer = SpanGuard::start("outer", SpanKind::Action);
        {
            let _inner = SpanGuard::start("inner", SpanKind::View);
        }
        drop(outer);

        let snap = snapshot();
        assert_eq!(snap.len(), 2);
        // Inner is dropped first, so it's recorded first.
        let inner = &snap[0];
        let outer = &snap[1];
        assert_eq!(inner.name, "inner");
        assert_eq!(outer.name, "outer");
        assert_eq!(inner.parent, Some(outer.id));
        assert_eq!(outer.parent, None);
    }

    #[test]
    fn record_one_shot_uses_current_parent() {
        fresh();
        let _g = SpanGuard::start("controller", SpanKind::Action);
        record(
            "FOR doc IN posts RETURN doc",
            SpanKind::Db,
            Instant::now(),
            1500,
            Some("posts".into()),
        );
        drop(_g);

        let snap = snapshot();
        assert_eq!(snap.len(), 2);
        let db = snap.iter().find(|s| s.kind == SpanKind::Db).unwrap();
        let action = snap.iter().find(|s| s.kind == SpanKind::Action).unwrap();
        assert_eq!(db.parent, Some(action.id));
        assert_eq!(db.meta.as_deref(), Some("posts"));
        assert!(db.end_us >= db.start_us + 1500);
    }

    #[test]
    fn push_pop_fn_pairs_correctly() {
        fresh();
        let _outer = SpanGuard::start("controller", SpanKind::Action);
        push_fn("helper_one", Some("foo.sl:10".into()));
        push_fn("helper_two", None);
        pop_fn(); // helper_two
        pop_fn(); // helper_one
        drop(_outer);

        let snap = snapshot();
        assert_eq!(snap.len(), 3);
        let h1 = snap.iter().find(|s| s.name == "helper_one").unwrap();
        let h2 = snap.iter().find(|s| s.name == "helper_two").unwrap();
        assert_eq!(h2.parent, Some(h1.id));
        assert_eq!(h1.kind, SpanKind::Fn);
        assert_eq!(h2.kind, SpanKind::Fn);
    }

    #[test]
    fn begin_request_resets_id_counter() {
        clear();
        set_enabled(true);
        begin_request(Instant::now());
        let _a = SpanGuard::start("a", SpanKind::Action);
        drop(_a);
        let first = snapshot()[0].id;

        begin_request(Instant::now());
        let _b = SpanGuard::start("b", SpanKind::Action);
        drop(_b);
        let second = snapshot()[0].id;

        assert_eq!(first, second, "ids restart from 0 each request");
    }
}
