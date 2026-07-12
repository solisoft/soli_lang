//! Per-request collector for developer warnings raised during template render
//! (e.g. a component declared a prop that wasn't provided). Mirrors `view_log`
//! in shape: a thread-local `Vec` gated on an `ENABLED` cell so production pays
//! nothing. The dev bar drains `snapshot()` into its Warnings panel.

use std::cell::{Cell, RefCell};

thread_local! {
    static ENABLED: Cell<bool> = const { Cell::new(false) };
    static WARNINGS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

pub fn set_enabled(enabled: bool) {
    ENABLED.with(|e| e.set(enabled));
}

#[inline]
pub fn is_enabled() -> bool {
    ENABLED.with(|e| e.get())
}

pub fn clear() {
    WARNINGS.with(|w| w.borrow_mut().clear());
}

/// Record a developer warning for this request. No-op unless enabled (`--dev`).
/// De-duplicates, so a component rendered in a loop warns once, not per item.
pub fn record(message: String) {
    if !is_enabled() {
        return;
    }
    WARNINGS.with(|w| {
        let mut list = w.borrow_mut();
        if !list.contains(&message) {
            list.push(message);
        }
    });
}

pub fn snapshot() -> Vec<String> {
    WARNINGS.with(|w| w.borrow().clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_only_when_enabled_and_dedupes() {
        clear();
        set_enabled(false);
        record("a".to_string());
        assert!(snapshot().is_empty());

        set_enabled(true);
        record("a".to_string());
        record("a".to_string());
        record("b".to_string());
        assert_eq!(snapshot(), vec!["a".to_string(), "b".to_string()]);

        clear();
        set_enabled(false);
    }
}
