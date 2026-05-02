//! Per-template timing for the dev bar.
//!
//! `phase_log` already tracks the *total* time spent in views so the
//! render breakdown can compute "controller = total - middleware - view -
//! db - http". When a request renders multiple templates (the main view,
//! its layout, any partials), we want the dev bar to break that aggregate
//! down per template (name + duration), so this module captures one entry
//! per render call instead of a single rolling sum.
//!
//! Mirrors `middleware_log` in shape: thread-local Vec, gated on an
//! `ENABLED` atomic so production has zero cost.
//!
//! Note: nested calls (partials inside layouts inside the main view) each
//! record their own entry. Their durations therefore overlap and will
//! sum to *more* than the aggregate "view" phase — that's expected; the
//! sub-rows are a flat list of *what was rendered*, not a partition.
//!
//! Each render also gets a stable per-request `id` (assigned at *start*
//! of render via `next_id`). The template engine wraps the rendered
//! output in `<!--solidev:KIND:start id=ID …-->` HTML comments using
//! that same id; the dev bar emits `data-solidev-view-idx="ID"` on the
//! corresponding sub-row. The hover-overlay JS pairs them up to outline
//! the template's region in the page.
//!
//! `next_id` also pushes the new id onto an open-stack and `record` pops
//! it, so each entry carries its lexical parent — letting the dev bar
//! render the breakdown as a tree (layout → view → partial → …) instead
//! of a flat list.

use std::cell::{Cell, RefCell};

thread_local! {
    // Thread-local rather than a process-wide atomic: each worker thread
    // sets its own gate at startup (see `serve::mod::set_enabled`), and
    // unit tests that flip the flag on their test thread don't leak the
    // setting into parallel tests running on other threads.
    static ENABLED: Cell<bool> = const { Cell::new(false) };
    static LOG: RefCell<Vec<(u32, Option<u32>, String, u64)>> = const { RefCell::new(Vec::new()) };
    static NEXT_ID: Cell<u32> = const { Cell::new(0) };
    // Open-render stack — pushed by `next_id`, popped by `record`. Used
    // to discover the *parent* render at record time (the new top after
    // popping our own id).
    static OPEN_STACK: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
}

pub fn set_enabled(enabled: bool) {
    ENABLED.with(|e| e.set(enabled));
}

#[inline]
pub fn is_enabled() -> bool {
    ENABLED.with(|e| e.get())
}

pub fn clear() {
    LOG.with(|l| l.borrow_mut().clear());
    NEXT_ID.with(|c| c.set(0));
    OPEN_STACK.with(|s| s.borrow_mut().clear());
}

/// Allocate the next render id for this request. Assigned at the *start*
/// of a render call so the wrapping HTML comment markers and the
/// eventual `record(...)` entry agree, even when nested partials cause
/// completion order to differ from start order. Pushes the id onto the
/// open-stack so nested `next_id` calls inherit this id as their parent.
pub fn next_id() -> u32 {
    let id = NEXT_ID.with(|c| {
        let v = c.get();
        c.set(v + 1);
        v
    });
    OPEN_STACK.with(|s| s.borrow_mut().push(id));
    id
}

pub fn record(id: u32, name: &str, dur_us: u64) {
    if !is_enabled() {
        // Still keep the open-stack in sync: `next_id` always pushes,
        // so `record` must always pop, even when the gate is flipped
        // mid-request.
        OPEN_STACK.with(|s| {
            let mut stack = s.borrow_mut();
            if stack.last().copied() == Some(id) {
                stack.pop();
            }
        });
        return;
    }
    let parent = OPEN_STACK.with(|s| {
        let mut stack = s.borrow_mut();
        if stack.last().copied() == Some(id) {
            stack.pop();
        }
        stack.last().copied()
    });
    LOG.with(|l| l.borrow_mut().push((id, parent, name.to_string(), dur_us)));
}

pub fn snapshot() -> Vec<(u32, Option<u32>, String, u64)> {
    LOG.with(|l| l.borrow().clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() {
        clear();
        set_enabled(true);
    }

    #[test]
    fn nested_renders_link_parent() {
        fresh();
        // Simulate: render(view) { render_partial(card); render_layout(app) { render_partial(menu) } }
        let view = next_id();
        let card = next_id();
        record(card, "things/card", 100);
        let layout = next_id();
        let menu = next_id();
        record(menu, "shared/menu", 50);
        record(layout, "layouts/application", 300);
        record(view, "things/show", 500);

        let snap = snapshot();
        // Records are appended in close-order.
        let by_id: std::collections::HashMap<u32, Option<u32>> =
            snap.iter().map(|(id, p, _, _)| (*id, *p)).collect();
        assert_eq!(by_id[&view], None);
        assert_eq!(by_id[&card], Some(view));
        assert_eq!(by_id[&layout], Some(view));
        assert_eq!(by_id[&menu], Some(layout));

        clear();
        set_enabled(false);
    }
}
