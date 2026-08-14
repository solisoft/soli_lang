//! Phoenix-style `send_update` for nested LiveView components.
//!
//! A handler (or any code running under the LiveView frame lock) calls
//! `send_update("desk_focus", { "focus": 3 })`. Assigns are queued on this
//! thread and merged onto parent state after the handler returns, then the
//! usual render/diff patches every attached socket.

use std::cell::RefCell;

use serde_json::Value as JsonValue;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{value_to_json, NativeFunction, Value};

thread_local! {
    static PENDING: RefCell<Vec<JsonValue>> = const { RefCell::new(Vec::new()) };
}

/// Queue an assigns hash to merge after the current handler returns.
pub fn queue(assigns: JsonValue) {
    if assigns.as_object().is_none() {
        return;
    }
    PENDING.with(|p| p.borrow_mut().push(assigns));
}

/// Take every queued update (clears the thread-local).
pub fn take() -> Vec<JsonValue> {
    PENDING.with(|p| std::mem::take(&mut *p.borrow_mut()))
}

/// Merge queued updates (and an optional handler `update:` hash) into state.
pub fn apply_to(state: &mut JsonValue, extra: Option<JsonValue>) {
    let mut pending = take();
    if let Some(extra) = extra {
        pending.push(extra);
    }
    for patch in pending {
        crate::live::nested::merge_child_assigns(state, &patch);
    }
}

/// Flatten a Soli hash `Value` into JSON for the queue.
pub fn queue_from_json(assigns: JsonValue) {
    queue(assigns);
}

/// `send_update(component?, assigns)` — merge `assigns` onto the current
/// LiveView after the handler returns. The component name is accepted for
/// Phoenix familiarity and is ignored: nested `live_component`s share parent
/// state, so the patch is the parent hash.
pub fn register(env: &mut Environment) {
    env.define(
        "send_update".to_string(),
        Value::NativeFunction(NativeFunction::new("send_update", None, |args| {
            let assigns = match args.len() {
                1 => args.first(),
                n if n >= 2 => args.get(1),
                _ => {
                    return Err(
                        "send_update(assigns) or send_update(component, assigns)".to_string()
                    )
                }
            };
            match assigns {
                Some(v @ Value::Hash(_)) => {
                    queue(value_to_json(v).unwrap_or(JsonValue::Null));
                    Ok(Value::Null)
                }
                _ => Err("send_update expects an assigns hash".to_string()),
            }
        })),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn queued_assigns_merge_onto_parent_state() {
        let _ = take();
        queue(json!({ "focus": 4, "flash": "ok" }));
        let mut state = json!({ "focus": 1, "tab": "all" });
        apply_to(&mut state, None);
        assert_eq!(state["focus"], 4);
        assert_eq!(state["flash"], "ok");
        assert_eq!(state["tab"], "all");
        assert!(take().is_empty());
    }

    #[test]
    fn handler_update_key_merges_too() {
        let _ = take();
        let mut state = json!({ "focus": 1 });
        apply_to(&mut state, Some(json!({ "focus": 9 })));
        assert_eq!(state["focus"], 9);
    }
}
