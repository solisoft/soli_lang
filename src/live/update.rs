//! Phoenix-style `send_update` / `update` for nested LiveView components.
//!
//! `send_update("score", { "score": 5 })` queues a named update. After the
//! parent handler returns, assigns are stored under `_components` and — when
//! the child has a `router_live` handler — that handler runs with
//! `event == "update"` (Soli's `update/2`). A bare `send_update({ ... })`
//! still merges onto the parent.

use std::cell::RefCell;

use serde_json::Value as JsonValue;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{value_to_json, NativeFunction, Value};
use crate::live::nested::{component_cid, merge_child_assigns, put_component_state};

/// One queued `send_update`.
#[derive(Clone, Debug)]
pub struct PendingUpdate {
    pub component: Option<String>,
    pub assigns: JsonValue,
}

thread_local! {
    static PENDING: RefCell<Vec<PendingUpdate>> = const { RefCell::new(Vec::new()) };
}

pub fn queue(assigns: JsonValue) {
    queue_named(None, assigns);
}

pub fn queue_named(component: Option<String>, assigns: JsonValue) {
    if assigns.as_object().is_none() {
        return;
    }
    PENDING.with(|p| {
        p.borrow_mut().push(PendingUpdate { component, assigns });
    });
}

pub fn take() -> Vec<PendingUpdate> {
    PENDING.with(|p| std::mem::take(&mut *p.borrow_mut()))
}

/// Apply queued updates (and an optional handler `update:` hash) into state.
/// Named targets also write `_components[cid]` so the next render uses
/// child-owned state. Parent keys are still merged so existing boards that
/// share assigns keep working when the child has no handler.
///
/// Returns `(component_name, cid, assigns)` for each named target so the
/// event loop can run the child's `update` handler.
pub fn apply_to(
    state: &mut JsonValue,
    extra: Option<JsonValue>,
) -> Vec<(String, String, JsonValue)> {
    let mut pending = take();
    if let Some(extra) = extra {
        pending.push(PendingUpdate {
            component: extra
                .get("_component")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            assigns: extra,
        });
    }
    let mut named = Vec::new();
    for patch in pending {
        let mut assigns = patch.assigns;
        if let Some(name) = patch.component.as_deref() {
            let cid = component_cid(name, &assigns);
            let mut child = crate::live::nested::get_component_state(state, &cid)
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            merge_child_assigns(&mut child, &assigns);
            put_component_state(state, &cid, child);
            named.push((name.to_string(), cid, assigns.clone()));
            if let Some(obj) = assigns.as_object_mut() {
                obj.remove("id");
                obj.remove("_component");
            }
        }
        merge_child_assigns(state, &assigns);
    }
    named
}

/// `send_update(assigns)` or `send_update(component, assigns)`.
pub fn register(env: &mut Environment) {
    env.define(
        "send_update".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "send_update",
            None,
            |args| match args.len() {
                1 => match args.first() {
                    Some(v @ Value::Hash(_)) => {
                        queue(value_to_json(v).unwrap_or(JsonValue::Null));
                        Ok(Value::Null)
                    }
                    _ => Err("send_update expects an assigns hash".to_string()),
                },
                n if n >= 2 => {
                    let name = match args.first() {
                        Some(Value::String(s)) => s.to_string(),
                        _ => {
                            return Err(
                                "send_update(component, assigns) expects a string name".into()
                            )
                        }
                    };
                    match args.get(1) {
                        Some(v @ Value::Hash(_)) => {
                            queue_named(Some(name), value_to_json(v).unwrap_or(JsonValue::Null));
                            Ok(Value::Null)
                        }
                        _ => Err("send_update expects an assigns hash".to_string()),
                    }
                }
                _ => Err("send_update(assigns) or send_update(component, assigns)".to_string()),
            },
        )),
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

    #[test]
    fn named_send_update_writes_component_bag() {
        let _ = take();
        queue_named(Some("score".into()), json!({ "score": 7, "id": "a" }));
        let mut state = json!({ "score": 1 });
        apply_to(&mut state, None);
        assert_eq!(state["score"], 7, "parent still sees the assign");
        assert_eq!(state["_components"]["score:a"]["score"], 7);
    }
}
