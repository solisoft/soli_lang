//! LiveView component state management.

use serde_json::json;
use serde_json::Value as JsonValue;
use std::collections::HashMap;

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
    name: String,
    state: JsonValue,
}

impl ComponentInstance {
    pub fn new(name: String, state: JsonValue) -> Self {
        Self { name, state }
    }

    pub fn mount(session: JsonValue, params: JsonValue) -> Result<Self, String> {
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

use std::sync::{Mutex, OnceLock};
use uuid::Uuid;

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
pub fn render_component(component_name: &str, state: &JsonValue) -> Result<String, String> {
    let template_path = format!("app/views/live/{}.sliv", component_name);
    let template_path = std::path::Path::new(&template_path);

    if !template_path.exists() {
        return Err(format!("Template not found: {}", template_path.display()));
    }

    let content = std::fs::read_to_string(template_path).map_err(|e| e.to_string())?;

    let mut html = String::new();
    let mut i = 0;
    let chars: Vec<char> = content.chars().collect();

    while i < chars.len() {
        if chars[i] == '<' && i + 1 < chars.len() {
            if i + 4 < chars.len()
                && chars[i + 1] == '%'
                && chars[i + 2] == '='
                && chars[i + 3] == ' '
            {
                let start = i + 4;
                let mut end = start;
                while end < chars.len() {
                    if end >= 2
                        && chars[end - 2] == ' '
                        && chars[end - 1] == '%'
                        && chars[end] == '>'
                    {
                        break;
                    }
                    end += 1;
                }
                let expr = chars[start..end - 3].iter().collect::<String>();
                let value = evaluate_expression(expr.trim(), state);
                html.push_str(&value);
                i = end + 1;
                continue;
            }
        }
        html.push(chars[i]);
        i += 1;
    }

    Ok(html)
}

fn evaluate_expression(expr: &str, state: &JsonValue) -> String {
    if expr.starts_with("@") {
        let key = &expr[1..];
        if let Some(value) = state.get(key) {
            return value.to_string();
        }
    }
    expr.to_string()
}
