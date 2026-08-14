//! LiveView component state management.

use serde_json::json;
use serde_json::Value as JsonValue;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::interpreter::builtins::template::inject_template_helpers;
use crate::interpreter::value::{json_to_value_ref, value_to_json, HashKey, NativeFunction, Value};
use crate::template::is_safe_template_name;
use crate::template::parser::parse_template;
use crate::template::renderer::render_nodes;
use uuid::Uuid;

lazy_static::lazy_static! {
    /// Global app root directory for LiveView template resolution.
    pub static ref APP_ROOT: Mutex<PathBuf> = Mutex::new(PathBuf::from("."));
}

/// Set the app root directory for LiveView templates.
pub fn set_app_root(path: PathBuf) {
    if let Ok(mut root) = APP_ROOT.lock() {
        *root = path;
    }
}

/// Get the app root directory.
pub fn get_app_root() -> PathBuf {
    APP_ROOT
        .lock()
        .map(|r| r.clone())
        .unwrap_or_else(|_| PathBuf::from("."))
}

/// Component state wrapper.
#[derive(Clone, Default)]
pub struct ComponentState {
    state: JsonValue,
}

impl ComponentState {
    pub fn new(state: JsonValue) -> Self {
        Self { state }
    }

    pub fn get(&self, key: &str) -> JsonValue {
        self.state.get(key).cloned().unwrap_or(JsonValue::Null)
    }

    pub fn set(&mut self, key: &str, value: JsonValue) {
        if let JsonValue::Object(map) = &mut self.state {
            map.insert(key.to_string(), value);
        }
    }

    pub fn as_value(&self) -> JsonValue {
        self.state.clone()
    }
}

/// Component instance with state.
pub struct ComponentInstance {
    #[allow(dead_code)]
    name: String,
    state: JsonValue,
}

impl ComponentInstance {
    pub fn new(name: String, state: JsonValue) -> Self {
        Self { name, state }
    }

    pub fn mount(_session: JsonValue, params: JsonValue) -> Result<Self, String> {
        let id = params
            .get("id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                format!(
                    "counter-{}",
                    Uuid::new_v4().to_string().split('-').next().unwrap()
                )
            });

        let initial = params.get("initial").and_then(|v| v.as_i64()).unwrap_or(0);

        let state = json!({
            "id": id,
            "count": initial
        });

        Ok(Self {
            name: "counter".to_string(),
            state,
        })
    }

    pub fn handle_event(&mut self, event: String, _params: JsonValue) -> Result<(), String> {
        match event.as_str() {
            "increment" | "decrement" => {
                if let Some(count) = self.state["count"].as_i64() {
                    let delta = if event == "increment" { 1 } else { -1 };
                    self.state["count"] = json!(count + delta);
                }
                Ok(())
            }
            _ => Err(format!("Unknown event: {}", event)),
        }
    }

    pub fn state(&self) -> &JsonValue {
        &self.state
    }

    pub fn state_mut(&mut self) -> &mut JsonValue {
        &mut self.state
    }
}

/// Get the counter component instance.
pub fn get_counter_component() -> Result<ComponentInstance, String> {
    Ok(ComponentInstance::new(
        "counter".to_string(),
        json!({
            "id": format!("counter-{}", Uuid::new_v4().to_string().split('-').next().unwrap()),
            "count": 0
        }),
    ))
}

/// Wrap a render/mount failure as the markup the client morphs into the DOM.
///
/// The text is escaped: it can carry a component name taken from the socket URL,
/// and unescaped it would be injected into the page verbatim.
pub fn render_error_html(error: &str) -> String {
    format!(
        "<div class=\"error\">{}</div>",
        crate::interpreter::builtins::html::html_escape(error)
    )
}

/// Render a component and return its HTML.
/// Supports .slv and .html.slv extensions (new), with backward compat for .sliv and .html.erb.
pub fn render_component(component_name: &str, state: &JsonValue) -> Result<String, String> {
    if !is_safe_template_name(component_name) {
        return Err(format!("Invalid component name: {}", component_name));
    }

    let app_root = get_app_root();

    // Try .html.slv first (new), then .slv, then fall back to .html.erb and .sliv (backward compat)
    let html_slv_path = app_root.join(format!("app/views/live/{}.html.slv", component_name));
    let slv_path = app_root.join(format!("app/views/live/{}.slv", component_name));
    let html_erb_path = app_root.join(format!("app/views/live/{}.html.erb", component_name));
    let sliv_path = app_root.join(format!("app/views/live/{}.sliv", component_name));

    let template_path = if html_slv_path.exists() {
        html_slv_path
    } else if slv_path.exists() {
        slv_path
    } else if html_erb_path.exists() {
        html_erb_path
    } else if sliv_path.exists() {
        sliv_path
    } else {
        // The paths stay on the server log; the returned message is shown in the
        // browser (see `render_error_html`) and must not disclose the filesystem
        // layout of the host.
        eprintln!(
            "[LiveView] template not found for component {}: tried {}, {}, {}, {}",
            component_name,
            html_slv_path.display(),
            slv_path.display(),
            html_erb_path.display(),
            sliv_path.display()
        );
        return Err(format!(
            "Template not found for live component '{}'",
            component_name
        ));
    };

    let content = std::fs::read_to_string(&template_path).map_err(|e| e.to_string())?;

    // Convert JSON state to interpreter Value
    let data = json_to_value_ref(state)?;

    // Inject template helpers (range, public_path, html_escape, etc.)
    inject_template_helpers(&data);
    bind_live_component_helper(&data);

    let nodes = parse_template(&content)?;

    // Push this render's state so `live_component` inside the template
    // resolves assigns from *this* parent — and pop it when we leave so a
    // sibling `live_component` on the caller does not see our spec.
    push_live_parent(state.clone());
    let rendered = render_nodes(&nodes, &data, None);
    pop_live_parent();
    rendered
}

thread_local! {
    static LIVE_PARENT: std::cell::RefCell<Vec<JsonValue>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

fn push_live_parent(parent: JsonValue) {
    LIVE_PARENT.with(|stack| stack.borrow_mut().push(parent));
}

fn pop_live_parent() {
    LIVE_PARENT.with(|stack| {
        stack.borrow_mut().pop();
    });
}

fn current_live_parent() -> JsonValue {
    LIVE_PARENT.with(|stack| {
        stack
            .borrow()
            .last()
            .cloned()
            .unwrap_or_else(|| JsonValue::Object(serde_json::Map::new()))
    })
}

fn bind_live_component_helper(data: &Value) {
    let Value::Hash(hash) = data else {
        return;
    };
    hash.borrow_mut().insert(
        HashKey::String("live_component".into()),
        Value::NativeFunction(NativeFunction::new(
            "live_component",
            None,
            live_component_helper,
        )),
    );
}

fn live_component_helper(args: &[Value]) -> Result<Value, String> {
    let name = match args.first() {
        Some(Value::String(s)) => s.to_string(),
        _ => {
            return Err(
                "live_component(name, assigns?) requires a string component name".to_string(),
            )
        }
    };
    let spec = match args.get(1) {
        Some(v) => value_to_json(v)?,
        None => JsonValue::Object(serde_json::Map::new()),
    };
    let parent = current_live_parent();
    let html = crate::live::nested::render_nested(&name, &parent, &spec)?;
    Ok(Value::String(html.into()))
}

#[cfg(test)]
pub(crate) fn with_app_root_lock<T>(f: impl FnOnce() -> T) -> T {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    f()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The browser-visible error must not disclose server paths, and must not
    /// carry markup through from the component name.
    #[test]
    fn render_errors_are_escaped_and_path_free() {
        with_app_root_lock(|| {
            let dir = tempdir().unwrap();
            fs::create_dir_all(dir.path().join("app/views/live")).unwrap();
            set_app_root(dir.path().to_path_buf());

            let missing = render_component("no_such_component", &json!({})).unwrap_err();
            assert!(
                !missing.contains(dir.path().to_str().unwrap()) && !missing.contains(".slv"),
                "message leaks the filesystem: {missing}"
            );
            assert!(missing.contains("no_such_component"));

            let html = render_error_html("Invalid component name: <img src=x onerror=alert(1)>");
            assert!(!html.contains("<img"), "markup survived escaping: {html}");
            assert!(html.contains("&lt;img"));
        });
    }
    use std::fs;
    use tempfile::tempdir;

    /// `render_component` is the only path-touching entry; gate it against
    /// the component name leaving `app/views/live/`.
    #[test]
    fn render_component_rejects_path_traversal() {
        with_app_root_lock(|| {
            let dir = tempdir().unwrap();
            let live = dir.path().join("app/views/live");
            fs::create_dir_all(&live).unwrap();

            // Plant a sibling file with one of the recognised suffixes outside
            // the live dir; if traversal worked, render_component would happily
            // read and parse it.
            let secret = dir.path().join("app/views/secret.html.slv");
            fs::create_dir_all(secret.parent().unwrap()).unwrap();
            fs::write(&secret, "<h1>secret</h1>").unwrap();

            // And a real component to confirm the legitimate path still works.
            fs::write(live.join("ok.html.slv"), "<h1>ok</h1>").unwrap();

            set_app_root(dir.path().to_path_buf());

            // Sanity: the legitimate name still renders.
            assert!(render_component("ok", &json!({})).is_ok());

            for bad in [
                "../secret",
                "../../app/views/secret",
                "..",
                "/etc/passwd",
                "./secret",
                "",
                "foo\0bar",
                "foo\\..\\secret",
            ] {
                let err = render_component(bad, &json!({}))
                    .expect_err(&format!("expected rejection for {:?}", bad));
                assert!(
                    err.contains("Invalid component name") || err.contains("not found"),
                    "unexpected error for {:?}: {}",
                    bad,
                    err
                );
            }
        });
    }

    #[test]
    fn sibling_live_components_keep_the_parent_assign_scope() {
        with_app_root_lock(|| {
            let dir = tempdir().unwrap();
            let live = dir.path().join("app/views/live");
            fs::create_dir_all(&live).unwrap();
            fs::write(
            live.join("board.html.slv"),
            r#"<%- live_component("left", { "n": true }) %><%- live_component("right", { "m": true }) %>"#,
        )
        .unwrap();
            fs::write(
                live.join("left.html.slv"),
                r#"<span id="l"><%= n %></span>"#,
            )
            .unwrap();
            fs::write(
                live.join("right.html.slv"),
                r#"<span id="r"><%= m %></span>"#,
            )
            .unwrap();
            set_app_root(dir.path().to_path_buf());

            let html = render_component("board", &json!({ "n": 1, "m": 2 })).unwrap();
            assert!(
                html.contains(">1<") && html.contains(">2<"),
                "second live_component must still read parent assigns, got: {html}"
            );
            assert!(html.contains("soli-component=\"left\""));
            assert!(html.contains("soli-component=\"right\""));
        });
    }
}
