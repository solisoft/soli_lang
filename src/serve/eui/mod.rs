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
pub mod stats;
pub mod tree;

use std::collections::HashMap;
use std::sync::Mutex;

use eui_proto::Frame;
use tungstenite::Message;

use crate::interpreter::value::Value;
use crate::interpreter::Interpreter;
use crate::live::view::{LiveViewInstance, LIVE_REGISTRY};
use crate::span::Span;

use self::stats::Stats;
use super::{json_to_value, unwrap_handler_return, value_to_json, LiveViewEventData};

/// `component -> view action`, filled by `router_eui`.
static EUI_VIEWS: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

/// Per-instance encoder state: tables, node ids, the previous tree.
///
/// A lock per instance inside a lock over the map, rather than one lock held
/// across the encode. The outer one is taken only to look an instance up, so
/// two sessions encoding at the same time — which they now do, the realtime
/// pool having more than one thread in it — do not queue behind each other.
/// Two frames of the *same* session still cannot overlap, and are already
/// held apart before they get here by the per-LiveView frame lock.
type EncoderCell = std::sync::Arc<Mutex<tree::Encoder>>;
static EUI_ENCODERS: Mutex<Option<HashMap<String, EncoderCell>>> = Mutex::new(None);

/// The synthetic event the socket posts when the client asks for a resync:
/// the handler is not run, the tree is re-sent whole.
pub const RESYNC_EVENT: &str = "__eui_resync";

/// The synthetic event the socket posts to the session's worker once the
/// session is gone, so the worker drops what it kept for it. The memo is
/// thread-local — it holds interpreter values — so nobody else can.
pub const FORGET_EVENT: &str = "__eui_forget";

/// Components whose socket needs a session cookie: `router_eui(component,
/// handler, view, {"session": "required"})`.
static EUI_SESSION_REQUIRED: Mutex<Option<std::collections::HashSet<String>>> = Mutex::new(None);

/// Refuse the upgrade for `component` unless the request carries a session.
pub fn require_session(component: &str) {
    let mut g = EUI_SESSION_REQUIRED
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(Default::default)
        .insert(component.to_string());
}

/// True when `router_eui` declared this component needs a session.
pub fn session_required(component: &str) -> bool {
    let g = EUI_SESSION_REQUIRED
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    g.as_ref().is_some_and(|set| set.contains(component))
}

/// A handler answered `{"close": reason}`. The worker's answer to the
/// socket is a `Result<(), String>` shared with LiveView, so the close rides
/// in the error string behind this mark; [`close_reason`] reads it back.
const CLOSE_MARK: &str = "\u{0}eui-close:";

/// The reason behind a close the handler asked for, or `None` for an
/// ordinary error.
pub fn close_reason(err: &str) -> Option<&str> {
    err.strip_prefix(CLOSE_MARK)
}

/// The worker side of [`FORGET_EVENT`].
pub fn forget_on_worker(liveview_id: &str) {
    tree::forget_session(liveview_id);
}

/// True while the instance's encoder is still around: the session is live.
pub fn has_encoder(liveview_id: &str) -> bool {
    let g = EUI_ENCODERS.lock().unwrap_or_else(|e| e.into_inner());
    g.as_ref().is_some_and(|m| m.contains_key(liveview_id))
}

/// Register a component's view action.
pub fn register_view(component: &str, view: &str) {
    let mut g = EUI_VIEWS.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(HashMap::new)
        .insert(component.to_string(), view.to_string());
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
    let cell = {
        let mut g = EUI_ENCODERS.lock().unwrap_or_else(|e| e.into_inner());
        let map = g.get_or_insert_with(HashMap::new);
        std::sync::Arc::clone(map.entry(liveview_id.to_string()).or_default())
    };
    // The map is unlocked here: the encode below is this instance's own.
    let mut encoder = cell.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut encoder)
}

/// Forget an instance's encoder when its socket is gone for good. The memo
/// is not touched here: it lives on the session's worker thread, and this
/// runs on the socket's task — the socket posts [`FORGET_EVENT`] for that.
pub fn drop_encoder(liveview_id: &str) {
    stats::forget(liveview_id);
    let mut g = EUI_ENCODERS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(map) = g.as_mut() {
        map.remove(liveview_id);
    }
}

/// How long one render waits on a socket that is not draining its frames
/// before giving the socket up.
///
/// The socket's queue holds 32 frames. A render that streams more than that
/// — a list of thirty thousand rows is a few dozen batches — used to lose
/// every batch past the queue's end, silently: the client kept a tree the
/// server had moved on from, and every later patch was against nodes it
/// had never been sent. The worker now waits for room, and if the reader
/// still cannot keep up it closes the queue, which closes the socket: the
/// client reconnects and resyncs, which is the honest outcome for a reader
/// that has fallen this far behind. The wait is bounded because the worker
/// holds this session's frame lock the while.
const SEND_PATIENCE: std::time::Duration = std::time::Duration::from_secs(2);

/// Send a frame to every socket attached to an instance: encode once, send to
/// each, and say how many bytes that was — the one number a dev bar cannot
/// work out for itself. `deadline` is the render's, shared by all its frames.
fn send_frame(instance: &LiveViewInstance, frame: &Frame, deadline: std::time::Instant) -> usize {
    let bytes = frame.encode();
    let size = bytes.len();
    for sender in &instance.senders {
        send_or_close(sender, bytes.clone(), deadline);
    }
    size
}

/// Queue one frame on one socket, waiting for room until `deadline`; past
/// it, close the socket rather than skip the frame.
fn send_or_close(
    sender: &async_channel::Sender<Result<Message, tungstenite::Error>>,
    bytes: Vec<u8>,
    deadline: std::time::Instant,
) {
    let mut message = Ok(Message::Binary(bytes));
    loop {
        match sender.try_send(message) {
            Ok(()) => return,
            Err(async_channel::TrySendError::Closed(_)) => return,
            Err(async_channel::TrySendError::Full(back)) => {
                if std::time::Instant::now() >= deadline {
                    eprintln!("[EUI] a socket is not draining its frames; closing it");
                    sender.close();
                    return;
                }
                message = back;
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
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
        eprintln!(
            "[EUI trace] worker: {component} {} state={}",
            data.event, instance.state
        );
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
                if let Some(reason) = unwrapped.close {
                    // The handler will not have this client: say why, with
                    // the code the spec gives a refusal, and end the session
                    // without rendering anything for it.
                    send_frame(
                        instance,
                        &Frame::Error {
                            code: 403,
                            message: reason.clone(),
                        },
                        std::time::Instant::now() + SEND_PATIENCE,
                    );
                    return Err(format!("{CLOSE_MARK}{reason}"));
                }
                if let Some(state) = unwrapped.state {
                    instance.state = state;
                }
                if let Some(update) = unwrapped.update {
                    if let (Some(dst), Some(src)) =
                        (instance.state.as_object_mut(), update.as_object())
                    {
                        for (k, v) in src {
                            dst.insert(k.clone(), v.clone());
                        }
                    }
                }
            }
            Ok(other) => {
                return Err(format!(
                    "EUI: handler '{handler_name}' returned {}, expected a hash",
                    other.type_name()
                ));
            }
            Err(e) => return Err(format!("EUI: handler '{handler_name}' failed: {e}")),
        }
    }

    // Render: the view action maps state to a tree hash.
    let view_name = view_action(&component)
        .ok_or_else(|| format!("EUI: no view for component '{component}'"))?;
    let view = resolve(interpreter, &view_name)?;
    let state_value = json_to_value(&instance.state);
    let t_view = std::time::Instant::now();
    // Which session the view is being called for, so `eui_stats()` inside it
    // can hand back that session's last render and no one else's.
    stats::set_current(Some(instance.id.clone()));
    let tree_value = interpreter
        .call_value(view, vec![state_value], Span::default())
        .map_err(|e| format!("EUI: view '{view_name}' failed: {e}"))?;
    let view_ms = t_view.elapsed().as_secs_f64() * 1e3;

    let resync = data.event == RESYNC_EVENT;
    let t_render = std::time::Instant::now();
    let session_id = instance.id.clone();
    // A handler that raises is one thing — a timeout inside an HTTP call is
    // transient, the session survives it, and the person's next click still
    // works. A view that cannot be *converted* is another: it names an event
    // or a value the protocol does not have, so every later render of it
    // fails the same way, and saying nothing leaves a window that looks
    // alive and answers nothing. Spec 01 §4: `Error` ends the session on
    // both sides, which is the right end for a view that will never encode.
    let mut tables = [0usize; 5];
    let batches = match with_encoder(&instance.id, |enc| {
        let out = enc.render_value(&session_id, &tree_value, resync);
        tables = enc.tables();
        out
    }) {
        Ok(batches) => batches,
        Err(e) => {
            send_frame(
                instance,
                &Frame::Error {
                    code: 400,
                    message: e.clone(),
                },
                std::time::Instant::now() + SEND_PATIENCE,
            );
            return Err(e);
        }
    };
    if std::env::var("EUI_TRACE").is_ok() {
        eprintln!(
            "[EUI trace] worker: view {view_ms:.0} ms, convert+diff+encode {:.0} ms, {} batch(es)",
            t_render.elapsed().as_secs_f64() * 1e3,
            batches.len()
        );
    }

    instance.touch();
    if !LIVE_REGISTRY.commit(instance) {
        if std::env::var("EUI_TRACE").is_ok() {
            eprintln!(
                "[EUI trace] worker: commit refused (socket gone?) for {}",
                instance.id
            );
        }
        return Ok(());
    }
    let encode_ms = t_render.elapsed().as_secs_f64() * 1e3;
    let mut sent = Stats {
        event: data.event.clone(),
        view_ms,
        encode_ms,
        batches: batches.len(),
        nodes: tables[4],
        atoms: tables[0],
        styles: tables[1],
        colors: tables[2],
        chunks: tables[3],
        ..Stats::default()
    };
    let deadline = std::time::Instant::now() + SEND_PATIENCE;
    for batch in batches {
        if std::env::var("EUI_TRACE").is_ok() {
            eprintln!(
                "[EUI trace] worker: sending batch seq={} ops={} to {} sender(s)",
                batch.seq,
                batch.ops.len(),
                instance.senders.len()
            );
        }
        sent.ops += batch.ops.len();
        sent.seq = batch.seq;
        sent.bytes += send_frame(instance, &Frame::Batch(batch), deadline);
    }
    // What this render cost, for the next one to draw. A dev bar reports the
    // work behind what is on the screen, so it is always one render behind —
    // which is the only honest thing it could be.
    stats::record(&instance.id, sent);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_close_is_told_apart_from_an_error() {
        assert_eq!(
            close_reason(&format!("{CLOSE_MARK}not you")),
            Some("not you")
        );
        assert_eq!(close_reason("EUI: view 'x' failed: boom"), None);
    }

    #[test]
    fn a_reader_that_never_drains_is_closed_not_skipped() {
        let (tx, rx) = async_channel::bounded::<Result<Message, tungstenite::Error>>(1);
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(20);
        send_or_close(&tx, vec![1], deadline);
        assert_eq!(rx.len(), 1, "the first frame fits");
        send_or_close(&tx, vec![2], deadline);
        assert!(
            tx.is_closed(),
            "the second could not be queued in time: closed"
        );
        assert_eq!(rx.len(), 1, "and nothing was dropped on the floor");
    }

    #[test]
    fn a_reader_that_drains_gets_every_frame() {
        let (tx, rx) = async_channel::bounded::<Result<Message, tungstenite::Error>>(1);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let drain = std::thread::spawn(move || {
            let mut got = 0;
            while let Ok(_) = rx.recv_blocking() {
                got += 1;
                if got == 3 {
                    break;
                }
            }
            got
        });
        for n in 0..3u8 {
            send_or_close(&tx, vec![n], deadline);
        }
        assert!(!tx.is_closed());
        assert_eq!(drain.join().unwrap(), 3);
    }
}
