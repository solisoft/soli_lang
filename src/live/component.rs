//! LiveView component state management.

use serde_json::json;
use serde_json::Value as JsonValue;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::interpreter::builtins::template::inject_template_helpers;
use crate::interpreter::value::json_to_value;
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

/// Render a component and return its HTML.
/// Supports .slv and .html.slv extensions (new), with backward compat for .sliv and .html.erb.
pub fn render_component(component_name: &str, state: &JsonValue) -> Result<String, String> {
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
        return Err(format!(
            "Template not found: {}, {}, {}, or {}",
            html_slv_path.display(),
            slv_path.display(),
            html_erb_path.display(),
            sliv_path.display()
        ));
    };

    let content = std::fs::read_to_string(&template_path).map_err(|e| e.to_string())?;

    // Convert JSON state to interpreter Value
    let data = json_to_value(state)?;

    // Inject template helpers (range, public_path, html_escape, etc.)
    inject_template_helpers(&data);

    // Parse the template using the existing ERB parser
    let nodes = parse_template(&content)?;

    // Render using the existing template renderer
    render_nodes(&nodes, &data, None)
}
