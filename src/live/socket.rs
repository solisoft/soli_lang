//! LiveView WebSocket handler.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

use serde_json::json;

use crate::live::component::render_component;
use crate::live::view::{LiveViewInstance, ServerMessage, LIVE_REGISTRY};

/// A LiveView route with its handler reference.
#[derive(Clone, Debug)]
pub struct LiveViewRoute {
    /// Component name (e.g., "counter")
    pub component: String,
    /// Controller#action string for handler lookup (e.g., "live#counter")
    pub handler_name: String,
}

// Global registry of LiveView routes
lazy_static::lazy_static! {
    pub static ref LIVEVIEW_ROUTES: std::sync::Mutex<HashMap<String, LiveViewRoute>> = std::sync::Mutex::new(HashMap::new());
}

/// Register a LiveView route.
/// `component` is the component name (e.g., "counter")
/// `handler_name` is "controller#action" (e.g., "live#counter")
pub fn register_liveview_route(component: &str, handler_name: &str) {
    let mut routes = LIVEVIEW_ROUTES.lock().unwrap();
    routes.insert(
        component.to_string(),
        LiveViewRoute {
            component: component.to_string(),
            handler_name: handler_name.to_string(),
        },
    );
}

/// Get the handler for a LiveView component.
pub fn get_liveview_handler(component: &str) -> Option<String> {
    let routes = LIVEVIEW_ROUTES.lock().unwrap();
    routes.get(component).map(|r| r.handler_name.clone())
}

/// Clear all LiveView routes (for hot reload).
pub fn clear_liveview_routes() {
    let mut routes = LIVEVIEW_ROUTES.lock().unwrap();
    routes.clear();
}

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
        .unwrap_or_else(|| format!("sess-{}", Uuid::new_v4()))
}

/// Extract component name from URL path.
#[allow(dead_code)]
fn _extract_component_from_path(path: &str) -> String {
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

    // Get initial state based on component
    let initial_state = match component.as_str() {
        "counter" => json!({
            "id": format!("{}-{}", component, Uuid::new_v4().to_string().split('-').next().unwrap()),
            "count": 0
        }),
        "metrics" => json!({
            "id": format!("{}-{}", component, Uuid::new_v4().to_string().split('-').next().unwrap()),
            "hours_str": "00", "minutes_str": "00", "seconds_str": "00",
            "h4": 0, "h3": 0, "h2": 0, "h1": 0, "h0": 0,
            "m5": 0, "m4": 0, "m3": 0, "m2": 0, "m1": 0, "m0": 0,
            "s5": 0, "s4": 0, "s3": 0, "s2": 0, "s1": 0, "s0": 0
        }),
        _ => json!({
            "id": format!("{}-{}", component, Uuid::new_v4().to_string().split('-').next().unwrap())
        }),
    };

    let mut instance = LiveViewInstance::new(
        component.clone(),
        PathBuf::from(template_path),
        initial_state,
        session_id.clone(),
        sender,
    );

    let liveview_id = instance.id.clone();

    // Render initial HTML
    let initial_html = render_component(&component, &instance.state)
        .unwrap_or_else(|e| format!("<div class='error'>{}</div>", e));

    // Save last_html for future diffs
    instance.last_html = initial_html.clone();

    // Register the instance
    LIVE_REGISTRY.register(instance.clone());

    // Send initial render
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
        ("metrics", "tick") => {
            // Generate simulated metrics
            use std::time::SystemTime;
            let now = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos();

            let time_secs = (now / 1_000_000_000) % 86400;
            let milliseconds = ((now / 1_000_000) % 1000) as i64;
            let hours = (time_secs / 3600) as i64;
            let minutes = ((time_secs % 3600) / 60) as i64;
            let seconds = (time_secs % 60) as i64;

            eprintln!(
                "DEBUG metrics: now={}, ms={}, h={}, m={}, s={}",
                now, milliseconds, hours, minutes, seconds
            );

            // Simulated fluctuating metrics
            let base = (now as f64 / 1000.0).sin();
            let cpu = (30.0 + base * 20.0 + (now % 100) as f64 * 0.15) as i64;
            let cpu = cpu.clamp(5, 95);

            let memory = (512.0 + (now as f64 / 2000.0).sin() * 100.0 + (now % 50) as f64) as i64;
            let memory_pct = (memory as f64 / 1024.0 * 100.0) as i64;

            let requests =
                (1500.0 + (now as f64 / 500.0).sin() * 500.0 + (now % 200) as f64) as i64;
            let requests_pct = (requests as f64 / 3000.0 * 100.0) as i64;

            let latency =
                (5.0 + (now as f64 / 800.0).sin() * 3.0 + (now % 3) as f64).max(1.0) as i64;
            let latency_pct = (latency as f64 / 20.0 * 100.0) as i64;

            // Format time with leading zeros
            let hours_str = format!("{:02}", hours);
            let minutes_str = format!("{:02}", minutes);
            let seconds_str = format!("{:02}", seconds);
            let milliseconds_str = format!("{:03}", milliseconds);

            instance.state["hours"] = json!(hours);
            instance.state["minutes"] = json!(minutes);
            instance.state["seconds"] = json!(seconds);
            instance.state["milliseconds"] = json!(milliseconds);
            instance.state["milliseconds_str"] = json!(milliseconds_str);
            instance.state["hours_str"] = json!(hours_str);
            instance.state["minutes_str"] = json!(minutes_str);
            instance.state["seconds_str"] = json!(seconds_str);

            // Binary clock bits (pre-computed for template)
            // Hours: 5 bits (0-23)
            instance.state["h4"] = json!((hours >> 4) & 1); // 16
            instance.state["h3"] = json!((hours >> 3) & 1); // 8
            instance.state["h2"] = json!((hours >> 2) & 1); // 4
            instance.state["h1"] = json!((hours >> 1) & 1); // 2
            instance.state["h0"] = json!(hours & 1); // 1

            // Minutes: 6 bits (0-59)
            instance.state["m5"] = json!((minutes >> 5) & 1); // 32
            instance.state["m4"] = json!((minutes >> 4) & 1); // 16
            instance.state["m3"] = json!((minutes >> 3) & 1); // 8
            instance.state["m2"] = json!((minutes >> 2) & 1); // 4
            instance.state["m1"] = json!((minutes >> 1) & 1); // 2
            instance.state["m0"] = json!(minutes & 1); // 1

            // Seconds: 6 bits (0-59)
            instance.state["s5"] = json!((seconds >> 5) & 1); // 32
            instance.state["s4"] = json!((seconds >> 4) & 1); // 16
            instance.state["s3"] = json!((seconds >> 3) & 1); // 8
            instance.state["s2"] = json!((seconds >> 2) & 1); // 4
            instance.state["s1"] = json!((seconds >> 1) & 1); // 2
            instance.state["s0"] = json!(seconds & 1); // 1

            // Milliseconds: 10 bits (0-999)
            instance.state["ms9"] = json!((milliseconds >> 9) & 1); // 512
            instance.state["ms8"] = json!((milliseconds >> 8) & 1); // 256
            instance.state["ms7"] = json!((milliseconds >> 7) & 1); // 128
            instance.state["ms6"] = json!((milliseconds >> 6) & 1); // 64
            instance.state["ms5"] = json!((milliseconds >> 5) & 1); // 32
            instance.state["ms4"] = json!((milliseconds >> 4) & 1); // 16
            instance.state["ms3"] = json!((milliseconds >> 3) & 1); // 8
            instance.state["ms2"] = json!((milliseconds >> 2) & 1); // 4
            instance.state["ms1"] = json!((milliseconds >> 1) & 1); // 2
            instance.state["ms0"] = json!(milliseconds & 1); // 1

            instance.state["cpu"] = json!(cpu);
            instance.state["memory"] = json!(memory);
            instance.state["memory_pct"] = json!(memory_pct);
            instance.state["requests"] = json!(requests);
            instance.state["requests_pct"] = json!(requests_pct);
            instance.state["latency"] = json!(latency);
            instance.state["latency_pct"] = json!(latency_pct);
        }
        _ => return Err(format!("Unknown event: {}", event)),
    }

    // Render new HTML
    let new_html = render_component(&component, &instance.state)?;
    let old_html = instance.last_html.clone();

    // Compute patch
    let patch = crate::live::diff::compute_patch(&old_html, &new_html);

    // Update last_html and save instance back to registry
    instance.last_html = new_html;
    instance.touch();
    LIVE_REGISTRY.update(instance);

    // Send patch to client
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
