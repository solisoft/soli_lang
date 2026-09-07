//! EUI: a LiveView component whose view is a node tree rather than HTML,
//! served as binary frames over `/_eui/session/<component>`.
//!
//! Everything that already works for LiveView is reused untouched: route
//! registration, the instance registry, the frame lock, the worker channel,
//! the handler contract `{event, params, state} -> state`. What this module
//! adds is a second *render*: a view action returning a hash, which
//! [`tree`] turns into nodes, [`diff`] turns into patch ops against the
//! session's previous tree, and `eui-proto` encodes. Nothing here runs unless
//! the `eui` cargo feature is on.

pub mod assets;
pub mod diff;
pub mod local;
pub mod manifest;
pub mod session;
pub mod tree;

use std::collections::HashMap;
use std::sync::Mutex;

use eui_proto::Frame;
use tungstenite::Message;

use crate::interpreter::value::Value;
use crate::interpreter::Interpreter;
use crate::live::view::{LiveViewInstance, LIVE_REGISTRY};
use crate::span::Span;

use super::{json_to_value, unwrap_handler_return, value_to_json, LiveViewEventData};

/// `component -> view action`, filled by `router_eui`.
static EUI_VIEWS: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

/// Per-instance encoder state: tables, node ids, the previous tree.
static EUI_ENCODERS: Mutex<Option<HashMap<String, tree::Encoder>>> = Mutex::new(None);

/// The synthetic event the socket posts when the client asks for a resync:
/// the handler is not run, the tree is re-sent whole.
pub const RESYNC_EVENT: &str = "__eui_resync";

/// Register a component's view action.
pub fn register_view(component: &str, view: &str) {
    let mut g = EUI_VIEWS.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(HashMap::new).insert(component.to_string(), view.to_string());
}

/// True when `router_eui` registered this component.
pub fn is_eui_component(component: &str) -> bool {
    let g = EUI_VIEWS.lock().unwrap_or_else(|e| e.into_inner());
    g.as_ref().is_some_and(|m| m.contains_key(component))
}

fn view_action(component: &str) -> Option<String> {
    let g = EUI_VIEWS.lock().unwrap_or_else(|e| e.into_inner());
    g.as_ref().and_then(|m| m.get(component).cloned())
}

/// Run `f` against the instance's encoder, creating it on first use.
pub fn with_encoder<T>(liveview_id: &str, f: impl FnOnce(&mut tree::Encoder) -> T) -> T {
    let mut g = EUI_ENCODERS.lock().unwrap_or_else(|e| e.into_inner());
    let map = g.get_or_insert_with(HashMap::new);
    f(map.entry(liveview_id.to_string()).or_default())
}

/// Forget an instance's encoder when its socket is gone for good.
pub fn drop_encoder(liveview_id: &str) {
    let mut g = EUI_ENCODERS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(map) = g.as_mut() {
        map.remove(liveview_id);
    }
}

/// Send a frame to every socket attached to an instance.
fn send_frame(instance: &LiveViewInstance, frame: &Frame) {
    let bytes = frame.encode();
    for sender in &instance.senders {
        let _ = sender.try_send(Ok(Message::Binary(bytes.clone())));
    }
}

fn resolve(interpreter: &Interpreter, action: &str) -> Result<Value, String> {
    if let Ok(h) = crate::interpreter::builtins::router::resolve_handler(action, None) {
        return Ok(h);
    }
    let name = action.split('#').next_back().unwrap_or(action);
    interpreter
        .environment
        .borrow()
        .get(name)
        .ok_or_else(|| format!("EUI: action '{action}' is not defined"))
}

/// The worker-side entry: run the handler (unless resyncing), then the view,
/// then diff and send. Called from `handle_liveview_event` with the frame
/// lock already held.
pub fn handle_eui_event(
    interpreter: &mut Interpreter,
    data: &LiveViewEventData,
    instance: &mut LiveViewInstance,
) -> Result<(), String> {
    let component = instance.component.clone();
    if std::env::var("EUI_TRACE").is_ok() {
        eprintln!("[EUI trace] worker: {component} {} state={}", data.event, instance.state);
    }

    if data.event != RESYNC_EVENT {
        let handler_name = crate::live::socket::get_liveview_handler(&component)
            .ok_or_else(|| format!("EUI: no handler for component '{component}'"))?;
        let handler = resolve(interpreter, &handler_name)?;

        let mut event_map = crate::interpreter::value::HashPairs::default();
        let key = |k: &str| crate::interpreter::value::HashKey::String(k.into());
        event_map.insert(key("event"), Value::String(data.event.clone().into()));
        event_map.insert(key("params"), json_to_value(&data.params));
        event_map.insert(key("state"), json_to_value(&instance.state));
        let event_value = Value::Hash(std::rc::Rc::new(std::cell::RefCell::new(event_map)));

        match interpreter.call_value(handler, vec![event_value], Span::default()) {
            Ok(Value::Null) => {}
            Ok(result @ Value::Hash(_)) => {
                let unwrapped = unwrap_handler_return(value_to_json(&result));
                if let Some(state) = unwrapped.state {
                    instance.state = state;
                }
                if let Some(update) = unwrapped.update {
                    if let (Some(dst), Some(src)) = (instance.state.as_object_mut(), update.as_object()) {
                        for (k, v) in src {
                            dst.insert(k.clone(), v.clone());
                        }
                    }
                }
            }
            Ok(other) => {
                return Err(format!("EUI: handler '{handler_name}' returned {}, expected a hash", other.type_name()));
            }
            Err(e) => return Err(format!("EUI: handler '{handler_name}' failed: {e}")),
        }
    }

    // Render: the view action maps state to a tree hash.
    let view_name = view_action(&component).ok_or_else(|| format!("EUI: no view for component '{component}'"))?;
    let view = resolve(interpreter, &view_name)?;
    let state_value = json_to_value(&instance.state);
    let t_view = std::time::Instant::now();
    let tree_value = interpreter
        .call_value(view, vec![state_value], Span::default())
        .map_err(|e| format!("EUI: view '{view_name}' failed: {e}"))?;
    let view_ms = t_view.elapsed().as_secs_f64() * 1e3;
    let t_json = std::time::Instant::now();
    let tree_json = value_to_json(&tree_value);
    let json_ms = t_json.elapsed().as_secs_f64() * 1e3;

    let resync = data.event == RESYNC_EVENT;
    let t_render = std::time::Instant::now();
    let batches = with_encoder(&instance.id, |enc| enc.render(&tree_json, resync))?;
    if std::env::var("EUI_TRACE").is_ok() {
        eprintln!(
            "[EUI trace] worker: view {view_ms:.0} ms, to json {json_ms:.0} ms, convert+diff+encode {:.0} ms, {} batch(es)",
            t_render.elapsed().as_secs_f64() * 1e3,
            batches.len()
        );
    }

    instance.touch();
    if !LIVE_REGISTRY.commit(instance) {
        if std::env::var("EUI_TRACE").is_ok() {
            eprintln!("[EUI trace] worker: commit refused (socket gone?) for {}", instance.id);
        }
        return Ok(());
    }
    for batch in batches {
        if std::env::var("EUI_TRACE").is_ok() {
            eprintln!("[EUI trace] worker: sending batch seq={} ops={} to {} sender(s)", batch.seq, batch.ops.len(), instance.senders.len());
        }
        send_frame(instance, &Frame::Batch(batch));
    }
    Ok(())
}
