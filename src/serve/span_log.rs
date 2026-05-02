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
    /// For View/Partial spans, the matching `view_log::next_id()` value.
    /// The dev bar emits this as `data-solidev-view-idx` on the flame row
    /// so the hover-overlay JS pairs the row with the
    /// `<!--solidev:KIND:start id=…-->` markers around the rendered region.
    pub render_id: Option<u32>,
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
    static ROOT: RefCell<Option<RootRequest>> = const { RefCell::new(None) };
}

/// State for the synthetic root span that wraps the entire request.
/// Stored separately from `STACK` so we can close it explicitly in
/// `finalize_response` (right before snapshotting) without relying on
/// SpanGuard drop ordering — the dev-bar injection happens *inside* the
/// `handle_request` stack frame, so a normal RAII guard would still be
/// alive at snapshot time and the root would be missing from the trace.
struct RootRequest {
    id: u32,
    name: String,
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
    ROOT.with(|r| r.borrow_mut().take());
}

pub fn clear() {
    LOG.with(|l| l.borrow_mut().clear());
    STACK.with(|s| s.borrow_mut().clear());
    FN_STACK.with(|f| f.borrow_mut().clear());
    NEXT_ID.with(|n| n.set(0));
    REQ_START.with(|t| t.set(None));
    ROOT.with(|r| r.borrow_mut().take());
}

/// Returns true for the small whitelist of request-path native
/// builtins worth instrumenting in the flamegraph. Anything that turns
/// a controller action into a response goes here — `render`, `redirect`,
/// `halt`, `render_json`, `render_text`, `render_partial`. Other
/// builtins (`len`, `str`, `print`, `t`, `h`, …) are deliberately left
/// out: instrumenting them would generate one Fn span per call inside
/// hot iteration loops and drown out signal in the chart.
///
/// Kept as a `match` so the compiler optimizes it down to a jump table
/// and so the whitelist is trivially auditable in one place.
fn is_request_path_native(name: &str) -> bool {
    matches!(
        name,
        "render" | "render_partial" | "render_json" | "render_text" | "redirect" | "halt"
    )
}

/// Open a `Fn` span for a native builtin call when (a) dev mode is on
/// AND (b) the native is on the request-path whitelist. Returns `None`
/// otherwise, so the caller pays only a thread-local read + a `match`
/// in the cold path. The returned guard closes the span on drop, just
/// like `SpanGuard::start`.
///
/// This is the bridge that fills the "where did the time go between the
/// controller `Fn` span and the `View` span?" gap — `render(...)` is a
/// `NativeFunction`, not a Soli function, so it doesn't go through
/// `push_frame` / `push_fn` and would otherwise produce no span at all.
pub fn maybe_instrument_native(name: &str) -> Option<SpanGuard> {
    if !is_enabled() {
        return None;
    }
    if !is_request_path_native(name) {
        return None;
    }
    Some(SpanGuard::start(name, SpanKind::Fn))
}

/// Open the synthetic root span for the current request. Pushes its id
/// onto the open-span stack so every other span captured during the
/// request becomes a (transitive) child of this root, giving the
/// flamegraph a single top-level rectangle (e.g. `GET /docs/getting_started`)
/// instead of a forest of disconnected action / middleware spans.
///
/// No-op when dev mode is off, or if a root has already been opened for
/// this request (defensive against double-open paths).
pub fn open_request_root(name: String) {
    if !is_enabled() {
        return;
    }
    if ROOT.with(|r| r.borrow().is_some()) {
        return;
    }
    let id = next_id();
    STACK.with(|s| s.borrow_mut().push(id));
    ROOT.with(|r| *r.borrow_mut() = Some(RootRequest { id, name }));
}

/// Close the synthetic root span. Must be called right before
/// `snapshot()` so the root entry actually ends up in the recorded log.
/// Idempotent — safe to call when no root is open.
pub fn close_request_root() {
    if !is_enabled() {
        return;
    }
    let Some(root) = ROOT.with(|r| r.borrow_mut().take()) else {
        return;
    };
    STACK.with(|s| {
        let mut stack = s.borrow_mut();
        if stack.last().copied() == Some(root.id) {
            stack.pop();
        }
    });
    let end_us = REQ_START.with(|r| match r.get() {
        Some(start) => start.elapsed().as_micros() as u64,
        None => 0,
    });
    LOG.with(|l| {
        l.borrow_mut().push(SpanRecord {
            id: root.id,
            parent: None,
            name: root.name,
            kind: SpanKind::Request,
            start_us: 0,
            end_us,
            meta: None,
            render_id: None,
        })
    });
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
            render_id: None,
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
            render_id: None,
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
    render_id: Option<u32>,
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
                render_id: None,
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
            render_id: None,
            start: Some(Instant::now()),
        }
    }

    /// Stamp the render id assigned by `view_log::next_id()` onto this
    /// span so the dev bar can pair the flame row with the marker
    /// comments wrapped around the rendered template's region.
    pub fn set_render_id(&mut self, id: u32) {
        self.render_id = Some(id);
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
        let render_id = self.render_id;
        LOG.with(|l| {
            l.borrow_mut().push(SpanRecord {
                id: self.id,
                parent: self.parent,
                name: std::mem::take(&mut self.name),
                kind: self.kind,
                start_us,
                end_us,
                meta: std::mem::take(&mut self.meta),
                render_id,
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
    fn maybe_instrument_native_whitelisted_emits_span() {
        fresh();
        let outer = SpanGuard::start("controller_action", SpanKind::Action);
        {
            let _native = maybe_instrument_native("render");
            assert!(_native.is_some(), "render should be instrumented");
            // Inside the render span, an inner View span nests under it.
            let _view = SpanGuard::start("docs/getting_started", SpanKind::View);
        }
        drop(outer);

        let snap = snapshot();
        let render = snap
            .iter()
            .find(|s| s.name == "render")
            .expect("render span missing");
        assert_eq!(render.kind, SpanKind::Fn);
        let view = snap.iter().find(|s| s.kind == SpanKind::View).unwrap();
        assert_eq!(
            view.parent,
            Some(render.id),
            "view should nest under the render native"
        );
        let action = snap.iter().find(|s| s.kind == SpanKind::Action).unwrap();
        assert_eq!(render.parent, Some(action.id));
    }

    #[test]
    fn maybe_instrument_native_skips_non_whitelisted() {
        fresh();
        let _outer = SpanGuard::start("a", SpanKind::Action);
        assert!(maybe_instrument_native("len").is_none());
        assert!(maybe_instrument_native("str").is_none());
        assert!(maybe_instrument_native("print").is_none());
        assert!(maybe_instrument_native("h").is_none());
        // No extra spans emitted from the cold native checks.
        drop(_outer);
        let snap = snapshot();
        assert_eq!(snap.len(), 1, "only the action span should be recorded");
    }

    #[test]
    fn maybe_instrument_native_no_op_when_disabled() {
        clear();
        set_enabled(false);
        begin_request(Instant::now());
        assert!(maybe_instrument_native("render").is_none());
        assert!(snapshot().is_empty());
    }

    #[test]
    fn maybe_instrument_native_covers_response_primitives() {
        // Lock in the whitelist so accidentally narrowing it triggers
        // a test failure instead of silently dropping spans.
        for name in [
            "render",
            "render_partial",
            "render_json",
            "render_text",
            "redirect",
            "halt",
        ] {
            fresh();
            assert!(
                maybe_instrument_native(name).is_some(),
                "{name} should be instrumented"
            );
        }
    }

    #[test]
    fn root_request_wraps_other_spans() {
        fresh();
        open_request_root("GET /docs/getting_started".to_string());
        let action = SpanGuard::start("docs#getting_started", SpanKind::Action);
        {
            let _view = SpanGuard::start("docs/getting_started", SpanKind::View);
        }
        drop(action);
        close_request_root();

        let snap = snapshot();
        assert_eq!(snap.len(), 3, "view + action + root request");

        let root = snap
            .iter()
            .find(|s| s.kind == SpanKind::Request)
            .expect("root request span missing");
        assert_eq!(root.name, "GET /docs/getting_started");
        assert!(root.parent.is_none());
        assert_eq!(root.start_us, 0);

        let action = snap.iter().find(|s| s.kind == SpanKind::Action).unwrap();
        assert_eq!(
            action.parent,
            Some(root.id),
            "action should nest under the root request"
        );
        let view = snap.iter().find(|s| s.kind == SpanKind::View).unwrap();
        assert_eq!(view.parent, Some(action.id));
    }

    #[test]
    fn close_request_root_is_idempotent_when_no_root_open() {
        fresh();
        // Should not panic / not push a phantom record.
        close_request_root();
        assert!(snapshot().is_empty());
    }

    #[test]
    fn open_request_root_no_op_when_disabled() {
        clear();
        set_enabled(false);
        begin_request(Instant::now());
        open_request_root("GET /".to_string());
        close_request_root();
        assert!(snapshot().is_empty());
    }

    #[test]
    fn begin_request_clears_root_state() {
        fresh();
        open_request_root("GET /one".to_string());
        // Don't close — start a fresh request.
        begin_request(Instant::now());
        // New root should be allowed (previous one was discarded by begin_request).
        open_request_root("GET /two".to_string());
        close_request_root();

        let snap = snapshot();
        let roots: Vec<_> = snap
            .iter()
            .filter(|s| s.kind == SpanKind::Request)
            .collect();
        assert_eq!(roots.len(), 1, "only the second root should be recorded");
        assert_eq!(roots[0].name, "GET /two");
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
