//! LiveView WebSocket handler.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

use serde_json::json;
use tungstenite::Message;

use crate::live::component::{render_component, render_error_html};
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
    /// Per-instance tick task abort handles, keyed by liveview_id.
    /// Lets us cancel/replace a running tick when the handler asks for a new
    /// interval, when the WS connection closes, or when the instance expires.
    pub static ref LIVEVIEW_TICK_TASKS: std::sync::Mutex<HashMap<String, tokio::task::AbortHandle>> = std::sync::Mutex::new(HashMap::new());
}

/// Install (or replace) the tick task for a LiveView instance. Aborts any
/// previously-installed task for the same `liveview_id`.
pub fn set_tick_task(liveview_id: &str, handle: tokio::task::AbortHandle) {
    let mut tasks = LIVEVIEW_TICK_TASKS.lock().unwrap();
    if let Some(old) = tasks.insert(liveview_id.to_string(), handle) {
        old.abort();
    }
}

/// Cancel and remove the tick task for a LiveView instance, if any.
pub fn cancel_tick_task(liveview_id: &str) {
    let mut tasks = LIVEVIEW_TICK_TASKS.lock().unwrap();
    if let Some(old) = tasks.remove(liveview_id) {
        old.abort();
    }
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

/// Extract session ID from request cookies. SEC-077: delegates to the shared
/// `interpreter::builtins::session::extract_session_id_from_cookie` so the
/// LiveView socket reads both `session_id` and `__Host-session_id` cookie
/// names, matching whatever the HTTP layer issued. Falls back to a synthetic
/// `sess-<uuid>` handle when no cookie is present (LiveView needs a stable
/// per-socket identifier; it does not have to be a real session UUID).
pub fn extract_session_id(cookies: Option<&str>) -> String {
    crate::interpreter::builtins::session::extract_session_id_from_cookie(cookies)
        .unwrap_or_else(|| format!("sess-{}", Uuid::new_v4()))
}

/// Extract component name from URL path.
#[allow(dead_code)]
fn _extract_component_from_path(path: &str) -> String {
    path.trim_start_matches("/live/")
        .trim_end_matches("/socket")
        .to_string()
}

/// First-paint state for the Field Desk sample. The connect handler reseeds
/// the same shape; this exists so the initial Render is not an error page
/// (unknown components only get `{ id }` before `connect` runs).
fn desk_initial_state(component: &str) -> serde_json::Value {
    let notes = json!([
        {"id": "n1", "title": "Inspect north hatch", "body": "Seal weeps after last storm. Photo the gasket before you pull it.", "status": "open", "priority": 2, "pinned": true, "file": null},
        {"id": "n2", "title": "Swap radio battery", "body": "Unit 4 is under 20%. Spare pack is in the van, second drawer.", "status": "doing", "priority": 3, "pinned": false, "file": null},
        {"id": "n3", "title": "Log pump hours", "body": "Write the hour-meter before you leave the pad.", "status": "open", "priority": 1, "pinned": false, "file": null},
        {"id": "n4", "title": "Close the east gate", "body": "Latch checked, chain on, photo filed.", "status": "done", "priority": 1, "pinned": false, "file": {"name": "gate.jpg", "size": 184320, "processed": true}}
    ]);
    json!({
        "id": format!("{}-{}", component, Uuid::new_v4().to_string().split('-').next().unwrap()),
        "notes": notes,
        "visible": notes,
        "tab": "all",
        "q": "",
        "selected_id": "n1",
        "selected": notes[0],
        "counts": {"all": 4, "open": 2, "doing": 1, "done": 1},
        "draft_title": "",
        "draft_body": "",
        "composer_open": false,
        "menu_id": null,
        "focus": 3,
        "flash": "",
        "pending": 0
    })
}

/// Build the registry key for a LiveView instance.
///
/// Default is `session:component` (one board per browser session). A
/// `room` query param / `data-live-room` becomes `room:{room}:{component}`
/// so every tab and every visitor on that room shares one instance —
/// needed for public demos when the WebSocket upgrade has no cookie
/// (each socket would otherwise mint a unique `sess-<uuid>`).
pub fn liveview_instance_id(session_id: &str, component: &str, room: Option<&str>) -> String {
    match room {
        Some(name) if !name.is_empty() => format!("room:{name}:{component}"),
        _ => format!("{session_id}:{component}"),
    }
}

/// Accept a client `mid` / `room` only when it is a short token (UUID / slug).
pub fn sanitize_mount_id(raw: &str) -> Option<String> {
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(64)
        .collect();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

/// Handle a LiveView connection.
pub fn handle_live_connection(
    component: String,
    session_id: String,
    sender: Arc<async_channel::Sender<Result<tungstenite::Message, tungstenite::Error>>>,
    room: Option<String>,
) {
    let template_path = format!("app/views/live/{}.sliv", component);

    // Get initial state based on component
    let initial_state = match component.as_str() {
        "counter" => json!({
            "id": format!("{}-{}", component, Uuid::new_v4().to_string().split('-').next().unwrap()),
            "count": 0,
            "typed": ""
        }),
        "metrics" => json!({
            "id": format!("{}-{}", component, Uuid::new_v4().to_string().split('-').next().unwrap()),
            "hours_str": "00", "minutes_str": "00", "seconds_str": "00",
            "milliseconds": 0,
            "milliseconds_str": "000",
            "h4": 0, "h3": 0, "h2": 0, "h1": 0, "h0": 0,
            "m5": 0, "m4": 0, "m3": 0, "m2": 0, "m1": 0, "m0": 0,
            "s5": 0, "s4": 0, "s3": 0, "s2": 0, "s1": 0, "s0": 0,
            "ms9": 0, "ms8": 0, "ms7": 0, "ms6": 0, "ms5": 0,
            "ms4": 0, "ms3": 0, "ms2": 0, "ms1": 0, "ms0": 0,
        }),
        "desk" => desk_initial_state(&component),
        "desk_pulse" => json!({
            "id": format!("{}-{}", component, Uuid::new_v4().to_string().split('-').next().unwrap()),
            "series": [4, 6, 5, 8, 7, 9, 6, 10],
            "series_json": "[4,6,5,8,7,9,6,10]",
            "stamp": "--:--:--",
            "peak": 10
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
        sender.clone(),
    );
    // Same session shares `session:component`. A room overrides that so
    // every socket on the room sees one board.
    instance.id = liveview_instance_id(&session_id, &component, room.as_deref());

    let liveview_id = instance.id.clone();

    // Already mounted (second tab, or a reconnect): keep state, attach this
    // socket, and send the current HTML only to the new connection.
    if let Some(prev) = LIVE_REGISTRY.get(&liveview_id) {
        if !prev.is_expired(std::time::Duration::from_secs(3600)) {
            LIVE_REGISTRY.add_sender(&liveview_id, sender.clone());
            let html = if prev.last_html.is_empty() {
                render_component(&component, &prev.state).unwrap_or_else(|e| render_error_html(&e))
            } else {
                prev.last_html
            };
            if let Ok(payload) = serde_json::to_string(&ServerMessage::Render {
                html,
                liveview_id: liveview_id.clone(),
            }) {
                let _ = sender.try_send(Ok(Message::text(payload)));
            }
            return;
        }
    }

    // Render initial HTML
    let initial_html =
        render_component(&component, &instance.state).unwrap_or_else(|e| render_error_html(&e));

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::view::LIVE_REGISTRY;

    #[test]
    fn reconnect_reuses_in_flight_state() {
        let session = format!("sess-restore-{}", uuid::Uuid::new_v4());
        let (tx, _rx) = async_channel::bounded(8);
        let sender = Arc::new(tx);
        handle_live_connection("counter".into(), session.clone(), sender.clone(), None);
        let id = format!("{session}:counter");
        let mut inst = LIVE_REGISTRY.get(&id).expect("first connect");
        inst.state["count"] = json!(7);
        inst.state["typed"] = json!("kept");
        LIVE_REGISTRY.update(inst);

        handle_live_connection("counter".into(), session, sender, None);
        let again = LIVE_REGISTRY.get(&id).expect("reconnect");
        assert_eq!(again.state["count"], 7);
        assert_eq!(again.state["typed"], "kept");
    }

    #[test]
    fn second_tab_attaches_to_the_same_instance() {
        let session = format!("sess-tabs-{}", uuid::Uuid::new_v4());
        let (tx_a, rx_a) = async_channel::bounded(8);
        let (tx_b, rx_b) = async_channel::bounded(8);
        handle_live_connection("counter".into(), session.clone(), Arc::new(tx_a), None);
        let id = liveview_instance_id(&session, "counter", None);
        handle_live_connection("counter".into(), session, Arc::new(tx_b), None);
        let inst = LIVE_REGISTRY.get(&id).expect("shared instance");
        assert_eq!(inst.senders.len(), 2, "both tabs stay attached");
        drop(rx_a);
        drop(rx_b);
    }

    #[test]
    fn room_joins_the_same_instance_across_sessions() {
        let room = Some("field-desk".to_string());
        let (tx_a, rx_a) = async_channel::bounded(8);
        let (tx_b, rx_b) = async_channel::bounded(8);
        handle_live_connection("desk".into(), "sess-a".into(), Arc::new(tx_a), room.clone());
        handle_live_connection("desk".into(), "sess-b".into(), Arc::new(tx_b), room);
        let id = liveview_instance_id("sess-a", "desk", Some("field-desk"));
        assert_eq!(id, "room:field-desk:desk");
        let inst = LIVE_REGISTRY.get(&id).expect("shared room");
        assert_eq!(inst.senders.len(), 2, "both sessions stay attached");
        drop(rx_a);
        drop(rx_b);
    }
}
