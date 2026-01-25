//! LiveView WebSocket handler.

use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

use serde_json::json;

use crate::live::component::render_component;
use crate::live::view::{LiveRegistry, LiveViewInstance, ServerMessage, LIVE_REGISTRY};

/// Extract session ID from request cookies.
pub fn extract_session_id(cookies: Option<&str>) -> String {
    cookies
        .and_then(|c| {
            c.split(';')
                .find(|c| c.trim_start().starts_with("session_id="))
        })
        .map(|c| {
            let value = c.trim_start().strip_prefix("session_id=").unwrap();
            value.to_string()
        })
        .unwrap_or_else(|| format!("sess-{}", Uuid::new_v4().to_string()))
}

/// Extract component name from URL path.
fn extract_component_from_path(path: &str) -> String {
    path.trim_start_matches("/live/")
        .trim_end_matches("/socket")
        .to_string()
}

/// Handle a LiveView connection.
pub fn handle_live_connection(
    component: String,
    session_id: String,
    sender: Arc<async_channel::Sender<Result<tungstenite::Message, tungstenite::Error>>>,
) {
    let template_path = format!("app/views/live/{}.sliv", component);

    // Get initial state
    let initial_state = json!({
        "id": format!("{}-{}", component, Uuid::new_v4().to_string().split('-').next().unwrap()),
        "count": 0
    });

    let instance = LiveViewInstance::new(
        component.clone(),
        PathBuf::from(template_path),
        initial_state,
        session_id.clone(),
        sender,
    );

    let liveview_id = instance.id.clone();

    // Register the instance (clone to keep original for rendering)
    LIVE_REGISTRY.register(instance.clone());

    // Send initial render
    let initial_html = render_component(&component, &instance.state)
        .unwrap_or_else(|e| format!("<div class='error'>{}</div>", e));

    let _ = instance.send(ServerMessage::Render {
        html: initial_html,
        liveview_id,
    });
}

/// Handle an event from a LiveView client.
pub fn handle_event(
    liveview_id: &str,
    event: String,
    _params: serde_json::Value,
) -> Result<(), String> {
    let mut instance = LIVE_REGISTRY
        .get(liveview_id)
        .ok_or("LiveView not found".to_string())?;

    let component = instance.component.clone();

    // Update state based on event
    match (component.as_str(), event.as_str()) {
        ("counter", "increment") => {
            if let Some(count) = instance.state["count"].as_i64() {
                instance.state["count"] = json!(count + 1);
            }
        }
        ("counter", "decrement") => {
            if let Some(count) = instance.state["count"].as_i64() {
                instance.state["count"] = json!(count - 1);
            }
        }
        _ => return Err(format!("Unknown event: {}", event)),
    }

    // Render new HTML
    let new_html = render_component(&component, &instance.state)?;
    let old_html = instance.last_html.clone();

    // Update registry
    let patch = crate::live::diff::compute_patch(&old_html, &new_html);

    let _ = LIVE_REGISTRY.send(
        liveview_id,
        ServerMessage::Patch {
            liveview_id: liveview_id.to_string(),
            diff: patch,
        },
    );

    Ok(())
}

/// Clean up expired LiveViews.
pub fn cleanup() {
    LIVE_REGISTRY.cleanup();
}
