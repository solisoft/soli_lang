//! Per-component-render storage for declared props (`props("a", "b")`).
//!
//! A component template can declare the props it expects with a `props(...)`
//! call; after rendering, `render_include` diffs the declarations against the
//! data the component was given and warns about missing ones. Unlike
//! `content_for` (which nested partials *join*), declarations must stay isolated
//! per component render — a nested component's `props(...)` must not leak into
//! its parent — so this is a **stack** of frames, not a shared store. Like the
//! engine's other per-render state it lives in a thread-local (rendering is
//! synchronous and single-threaded per worker).

use std::cell::RefCell;
use std::collections::HashSet;

thread_local! {
    static DECLARED_PROPS: RefCell<Vec<HashSet<String>>> = const { RefCell::new(Vec::new()) };
}

/// RAII guard for one component render's declared-props frame. Pushes a fresh
/// set on construction and pops it on drop, keeping nested declarations isolated.
pub struct DeclaredPropsFrame;

/// Push a fresh frame for the component render about to begin.
pub fn push_frame() -> DeclaredPropsFrame {
    DECLARED_PROPS.with(|s| s.borrow_mut().push(HashSet::new()));
    DeclaredPropsFrame
}

impl Drop for DeclaredPropsFrame {
    fn drop(&mut self) {
        DECLARED_PROPS.with(|s| {
            s.borrow_mut().pop();
        });
    }
}

/// Record declared prop names into the current (innermost) frame. No-op when no
/// frame is active (a `props(...)` call outside a component render).
pub fn declare<I: IntoIterator<Item = String>>(names: I) {
    DECLARED_PROPS.with(|s| {
        if let Some(top) = s.borrow_mut().last_mut() {
            top.extend(names);
        }
    });
}

/// The names declared in the current frame (cloned). Empty when no frame is active.
pub fn current() -> HashSet<String> {
    DECLARED_PROPS.with(|s| s.borrow().last().cloned().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declare_and_read_within_frame() {
        let _frame = push_frame();
        declare(["title".to_string(), "value".to_string()]);
        let cur = current();
        assert!(cur.contains("title") && cur.contains("value"));
    }

    #[test]
    fn frame_pops_on_drop() {
        {
            let _frame = push_frame();
            declare(["a".to_string()]);
            assert!(current().contains("a"));
        }
        assert!(current().is_empty());
    }

    #[test]
    fn nested_frames_are_isolated() {
        let _outer = push_frame();
        declare(["outer".to_string()]);
        {
            let _inner = push_frame();
            declare(["inner".to_string()]);
            // Inner frame sees only its own declaration.
            let cur = current();
            assert!(cur.contains("inner") && !cur.contains("outer"));
        }
        // Back to the outer frame after the inner drops.
        let cur = current();
        assert!(cur.contains("outer") && !cur.contains("inner"));
    }

    #[test]
    fn declare_without_frame_is_noop() {
        declare(["lost".to_string()]);
        assert!(current().is_empty());
    }
}
