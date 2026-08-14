//! Shared-state nested LiveView components.
//!
//! Parent state is the source of truth. A child is *not* its own socket:
//! [`resolve_child_assigns`] picks the child's view of parent state, and
//! [`merge_child_assigns`] writes a child event's `_assigns` patch back
//! onto the parent. [`render_nested`] renders the child template with
//! those assigns and wraps it in `soli-component`.

use serde_json::{json, Map, Value as JsonValue};

/// Resolve a child's assign hash from parent state and a spec.
///
/// Each spec entry is either:
/// - `true`, `null`, or `"from_parent"` — copy `parent[key]`
/// - any other JSON value — used as a literal override
pub fn resolve_child_assigns(parent: &JsonValue, spec: &JsonValue) -> JsonValue {
    let Some(spec_map) = spec.as_object() else {
        return json!({});
    };
    let parent_map = parent.as_object();
    let mut out = Map::new();
    for (key, spec_val) in spec_map {
        let from_parent = spec_val.as_bool() == Some(true)
            || spec_val.is_null()
            || spec_val.as_str() == Some("from_parent");
        if from_parent {
            let taken = parent_map
                .and_then(|m| m.get(key))
                .cloned()
                .unwrap_or(JsonValue::Null);
            out.insert(key.clone(), taken);
        } else {
            out.insert(key.clone(), spec_val.clone());
        }
    }
    JsonValue::Object(out)
}

/// Apply a LiveView event's `_assigns` patch onto parent state.
/// No-op when the key is absent — the real dispatch path in `serve`.
pub fn apply_event_assigns(parent: &mut JsonValue, params: &JsonValue) {
    if let Some(updates) = params.get("_assigns") {
        merge_child_assigns(parent, updates);
    }
}

/// The serve event path: hydrate upload ids, merge child `_assigns`, then
/// return the state snapshot the handler receives as `event["state"]`.
/// Snapshotting *before* the merge would let a handler that returns that
/// hash wipe the child's patch.
pub fn prepare_handler_state(state: &mut JsonValue, params: &mut JsonValue) -> JsonValue {
    crate::live::upload::hydrate_event_params(params);
    apply_event_assigns(state, params);
    state.clone()
}

/// Merge a child-originated assign patch into parent state (in place).
///
/// When the parent already holds a number/bool for a key, a string value
/// is coerced to that type so HTML `soli-assign-*` attributes stay typed.
pub fn merge_child_assigns(parent: &mut JsonValue, updates: &JsonValue) {
    let Some(updates) = updates.as_object() else {
        return;
    };
    let JsonValue::Object(parent_map) = parent else {
        return;
    };
    for (key, raw) in updates {
        let coerced = match parent_map.get(key) {
            Some(existing) => coerce_to(existing, raw),
            None => coerce_loose(raw),
        };
        parent_map.insert(key.clone(), coerced);
    }
}

fn coerce_to(existing: &JsonValue, raw: &JsonValue) -> JsonValue {
    match existing {
        JsonValue::Number(_) => match raw {
            JsonValue::Number(_) => raw.clone(),
            JsonValue::String(s) => parse_number(s).unwrap_or_else(|| raw.clone()),
            _ => raw.clone(),
        },
        JsonValue::Bool(_) => match raw {
            JsonValue::Bool(_) => raw.clone(),
            JsonValue::String(s) if s.eq_ignore_ascii_case("true") => JsonValue::Bool(true),
            JsonValue::String(s) if s.eq_ignore_ascii_case("false") => JsonValue::Bool(false),
            _ => raw.clone(),
        },
        _ => raw.clone(),
    }
}

fn coerce_loose(raw: &JsonValue) -> JsonValue {
    match raw {
        JsonValue::String(s) => parse_number(s)
            .or_else(|| match s.to_ascii_lowercase().as_str() {
                "true" => Some(JsonValue::Bool(true)),
                "false" => Some(JsonValue::Bool(false)),
                _ => None,
            })
            .unwrap_or_else(|| raw.clone()),
        other => other.clone(),
    }
}

fn parse_number(s: &str) -> Option<JsonValue> {
    if let Ok(n) = s.parse::<i64>() {
        return Some(json!(n));
    }
    s.parse::<f64>().ok().map(|f| json!(f))
}

/// Wrap child markup so the client stamps events with `_component`.
pub fn wrap_component(name: &str, inner: &str) -> String {
    format!("<div soli-component=\"{}\">{}</div>", name, inner)
}

/// Render a nested component: resolve assigns from the parent, render the
/// child's LiveView template, wrap with `soli-component`.
pub fn render_nested(name: &str, parent: &JsonValue, spec: &JsonValue) -> Result<String, String> {
    if !crate::template::is_safe_template_name(name) {
        return Err(format!("Invalid component name: {name}"));
    }
    let assigns = resolve_child_assigns(parent, spec);
    let inner = crate::live::component::render_component(name, &assigns)?;
    Ok(wrap_component(name, &inner))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::component::{render_component, set_app_root};
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn parent_assign_change_is_visible_on_the_child() {
        let parent = json!({ "score": 4, "label": "pts" });
        let spec = json!({ "score": true, "label": "score" });
        let child = resolve_child_assigns(&parent, &spec);
        assert_eq!(child["score"], 4);
        assert_eq!(child["label"], "score");

        let parent = json!({ "score": 9, "label": "pts" });
        let child = resolve_child_assigns(&parent, &spec);
        assert_eq!(child["score"], 9, "child must see the parent's new assign");
    }

    #[test]
    fn child_event_assigns_update_parent_state() {
        let mut parent = json!({ "score": 4, "open": true });
        apply_event_assigns(
            &mut parent,
            &json!({ "_component": "score", "_assigns": { "score": "5", "open": "false" } }),
        );
        assert_eq!(parent["score"], 5);
        assert_eq!(parent["open"], false);
    }

    #[test]
    fn event_without_assigns_leaves_parent_untouched() {
        let mut parent = json!({ "score": 4 });
        apply_event_assigns(&mut parent, &json!({ "_component": "score" }));
        assert_eq!(parent["score"], 4);
    }

    #[test]
    fn prepare_handler_state_snapshots_after_assign_merge() {
        let mut state = json!({ "score": 1, "label": "pts" });
        let mut params = json!({
            "_component": "score",
            "_assigns": { "score": "9" },
            "file": { "id": "missing-upload-id" }
        });
        let snapshot = prepare_handler_state(&mut state, &mut params);
        assert_eq!(
            snapshot["score"], 9,
            "handler event[\"state\"] must see merged child assigns"
        );
        assert_eq!(state["score"], 9);
        assert_eq!(snapshot["label"], "pts");
        assert!(
            params["file"].get("data").is_none(),
            "unknown upload id must not invent data"
        );
    }

    #[test]
    fn render_nested_fans_parent_assigns_into_child_markup() {
        crate::live::component::with_app_root_lock(|| {
            let dir = tempdir().unwrap();
            let live = dir.path().join("app/views/live");
            fs::create_dir_all(&live).unwrap();
            fs::write(
                live.join("score.html.slv"),
                r#"<span id="s"><%= score %></span>"#,
            )
            .unwrap();
            set_app_root(dir.path().to_path_buf());

            let spec = json!({ "score": true });
            let html = render_nested("score", &json!({ "score": 3 }), &spec).unwrap();
            assert!(
                html.contains("soli-component=\"score\""),
                "wrapper missing: {html}"
            );
            assert!(html.contains(">3<"), "parent assign not in child: {html}");

            let mut parent = json!({ "score": 3 });
            merge_child_assigns(&mut parent, &json!({ "score": 8 }));
            let html = render_nested("score", &parent, &spec).unwrap();
            assert!(
                html.contains(">8<"),
                "child event did not update child render: {html}"
            );
            // Sibling render path still works.
            assert!(render_component("score", &json!({ "score": 1 })).is_ok());
        });
    }
}
