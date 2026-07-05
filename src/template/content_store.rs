//! Per-render storage for `content_for` captured blocks.
//!
//! A view (or partial) captures named HTML fragments with
//! `<% content_for "name" do %> ... <% end %>`; the layout later reads them
//! back via `<%= yield "name" %>` (or the `content_for("name")` read-form)
//! and the `content_for?("name")` predicate. The fragments must survive from
//! the view render into the layout render without threading a store through
//! every render-function signature — rendering is synchronous and
//! single-threaded per request, so the store lives in a thread-local,
//! matching the engine's other per-render state (lenient-vars mode,
//! `BUILTINS_RC`).

use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    static CONTENT_FOR: RefCell<Option<HashMap<String, String>>> = const { RefCell::new(None) };
}

/// RAII guard for the per-render `content_for` store.
///
/// The frame that installed the store clears it on drop, so captures never
/// leak into the next render on the same worker thread. Nested frames
/// (partials rendered inside a view) join the active store and drop as no-ops.
pub struct ContentForFrame {
    owned: bool,
}

/// Install a fresh store if none is active (the returned guard owns it and
/// clears it on drop). If a store is already active — a partial rendered
/// inside a view — join it so captures land in the outer render's store.
pub fn ensure_frame() -> ContentForFrame {
    CONTENT_FOR.with(|store| {
        let mut store = store.borrow_mut();
        if store.is_none() {
            *store = Some(HashMap::new());
            ContentForFrame { owned: true }
        } else {
            ContentForFrame { owned: false }
        }
    })
}

impl Drop for ContentForFrame {
    fn drop(&mut self) {
        if self.owned {
            CONTENT_FOR.with(|store| store.borrow_mut().take());
        }
    }
}

/// Append captured HTML under a name. Repeated captures for the same name
/// concatenate in document order (Rails semantics). No-op when no frame is
/// active (node-level rendering outside a `TemplateCache` render).
pub fn append(name: &str, html: &str) {
    CONTENT_FOR.with(|store| {
        if let Some(map) = store.borrow_mut().as_mut() {
            map.entry(name.to_string()).or_default().push_str(html);
        }
    });
}

/// Captured content for a name, if any.
pub fn get(name: &str) -> Option<String> {
    CONTENT_FOR.with(|store| store.borrow().as_ref().and_then(|m| m.get(name).cloned()))
}

/// Whether non-empty content was captured under a name. An empty capture
/// counts as absent, mirroring Rails' `content_for?` blank check.
pub fn has(name: &str) -> bool {
    CONTENT_FOR.with(|store| {
        store
            .borrow()
            .as_ref()
            .is_some_and(|m| m.get(name).is_some_and(|s| !s.is_empty()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owned_frame_clears_on_drop() {
        {
            let _frame = ensure_frame();
            append("head", "<script></script>");
            assert_eq!(get("head").as_deref(), Some("<script></script>"));
        }
        assert_eq!(get("head"), None);
    }

    #[test]
    fn nested_frame_joins_and_does_not_clear() {
        let _outer = ensure_frame();
        {
            let _inner = ensure_frame();
            append("head", "from-partial");
        }
        // Inner drop must not wipe the outer frame's store.
        assert_eq!(get("head").as_deref(), Some("from-partial"));
    }

    #[test]
    fn append_concatenates_in_order() {
        let _frame = ensure_frame();
        append("head", "one");
        append("head", "two");
        assert_eq!(get("head").as_deref(), Some("onetwo"));
    }

    #[test]
    fn append_without_frame_is_noop() {
        append("head", "lost");
        assert_eq!(get("head"), None);
        assert!(!has("head"));
    }

    #[test]
    fn has_requires_non_empty_content() {
        let _frame = ensure_frame();
        assert!(!has("head"));
        append("head", "");
        assert!(!has("head"));
        append("head", "x");
        assert!(has("head"));
    }
}
