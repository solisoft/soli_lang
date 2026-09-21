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
pub mod fonts;
pub mod local;
pub mod manifest;
pub mod notify;
pub mod session;
pub mod snapshot;
pub mod stats;
pub mod tree;

use std::collections::HashMap;
use std::sync::Mutex;

use crossbeam::channel;
use eui_proto::Frame;
use hyper::{HeaderMap, Response};
use tungstenite::Message;

use crate::interpreter::value::Value;
use crate::interpreter::Interpreter;
use crate::live::view::{live_registry, LiveViewInstance};
use crate::serve::tenant::TenantValue;
use crate::span::Span;

use self::stats::Stats;
use super::{
    handler_return_is_bare, json_to_value, unwrap_handler_return, value_to_json, LiveViewEventData,
    ResponseBody,
};

/// The three EUI things a plain `GET` can ask for, or `None` for anything
/// else.
///
/// Lifted out of `handle_hyper_request`, from between the same-origin gate and
/// the WebSocket upgrade branch. It stays on that side of the gate: the
/// comment on `/_eui/view/` below records that its lack of origin protection
/// is deliberate, and moving the trio ahead of the gate would have changed
/// what `check_csrf_origin` sees rather than only where the code lives.
pub(super) async fn http_get(
    path: &str,
    method: &str,
    raw_query: Option<&str>,
    headers: &HeaderMap,
    lv_event_tx: &channel::Sender<LiveViewEventData>,
) -> Option<Response<ResponseBody>> {
    if method != "GET" {
        return None;
    }
    // Content-addressed and immutable, so a plain GET with no session and no
    // cookie is the whole protocol.
    if let Some(hex) = path.strip_prefix("/_eui/asset/") {
        return Some(assets::respond(hex));
    }
    if path == "/.well-known/eui" {
        return Some(manifest::respond());
    }
    // A one-shot render, for a component that declared it may be served
    // this way. No socket, no session, nothing resident afterwards, and
    // a strong ETag so a cache in front answers the second reader.
    //
    // No origin protection, deliberately: `check_csrf_origin` exempts
    // `GET`, and a resource that a CDN is meant to hold cannot have a
    // same-origin check. What stands in for it is that the render runs as
    // nobody and that `{"static": ...}` is the application promising its
    // `connect` is a read — `<img src=".../_eui/view/x">` on any page
    // anywhere reaches this.
    if let Some(component) = path.strip_prefix("/_eui/view/") {
        return Some(
            snapshot::respond(
                component.trim_end_matches('/'),
                raw_query,
                headers,
                lv_event_tx,
            )
            .await,
        );
    }
    None
}

/// `component -> view action`, filled by `router_eui`. Per application: two
/// apps may both register a component called `counter`, and this is what
/// decides whose view renders it.
static EUI_VIEWS: TenantValue<HashMap<String, String>> = TenantValue::new(HashMap::new);

/// Per-instance encoder state: tables, node ids, the previous tree.
///
/// A lock per instance inside a lock over the map, rather than one lock held
/// across the encode. The outer one is taken only to look an instance up, so
/// two sessions encoding at the same time — which they now do, the realtime
/// pool having more than one thread in it — do not queue behind each other.
/// Two frames of the *same* session still cannot overlap, and are already
/// held apart before they get here by the per-LiveView frame lock.
type EncoderCell = std::sync::Arc<Mutex<tree::Encoder>>;
/// Per application as well as per instance: the keys are session ids minted by
/// one application, and an encoder holds that session's interned tables and
/// previous tree.
static EUI_ENCODERS: TenantValue<HashMap<String, EncoderCell>> = TenantValue::new(HashMap::new);

/// EUI 01 §4.1: what a broken socket leaves behind, so the socket after it
/// can pick the session up instead of mounting a new one.
///
/// Keyed by instance id, which is what every write here has in hand: a batch
/// is remembered on each render and an `Ack` trims the buffer, while a handle
/// is looked up once per reconnect. The rare road pays the scan.
///
/// The **handle** is what a `Welcome` names the session by — sixteen random
/// bytes minted per EUI session, not the cookie's digest. Two tabs of one
/// person share a cookie and must not share a session, so the handle cannot
/// be derived from it; and because it is a bearer (01 §4.1), resuming also
/// requires the socket to carry the same cookie session and name the same
/// component, so possession of the bytes alone is not possession of the
/// session.
struct Resumable {
    /// The handle this session was opened with. A field rather than the key:
    /// a batch is remembered on every render and a handle is looked up once
    /// per reconnect, so the map is keyed by the thing the hot path has.
    handle: [u8; 16],
    /// The cookie session that opened it. A resume must match it.
    session_id: String,
    /// Which component, so a handle cannot be pointed at another one.
    component: String,
    /// Batches sent and not yet acknowledged, oldest first, capped at
    /// [`MAX_REPLAY`]: the client names the last one it applied and is sent
    /// what came after it.
    replay: std::collections::VecDeque<(u64, Vec<u8>)>,
    /// When the socket went. `None` while one is attached.
    detached_at: Option<std::time::Instant>,
}

/// How many unacknowledged batches a session keeps for a reconnect.
///
/// The same number the reference server keeps, and a ceiling rather than a
/// target: a client that has applied everything keeps none of them. What
/// bounds it is a session that renders while nobody is reading the socket —
/// past this the replay is incomplete, and an incomplete replay is refused
/// rather than half-applied, because 01 §4.1's whole promise is that a
/// resumed session is the session and not something like it.
const MAX_REPLAY: usize = 64;

/// How long a detached session is kept. The same two minutes the LiveView
/// registry holds an instance for, because the two have to expire together:
/// a handle whose instance the reaper took names nothing.
const RESUME_GRACE: std::time::Duration = std::time::Duration::from_secs(120);

static EUI_RESUMABLE: TenantValue<HashMap<String, Resumable>> = TenantValue::new(HashMap::new);

/// Sixteen bytes that name one EUI session and nothing else.
///
/// Random, because a handle derived from anything a client can see is a
/// handle a client can guess, and 01 §4.1 makes it a bearer.
pub fn mint_handle() -> [u8; 16] {
    let mut out = [0u8; 16];
    out.copy_from_slice(&uuid::Uuid::new_v4().as_bytes()[..16]);
    out
}

/// Drop every detached session whose grace has run out, taking its encoder
/// with it.
///
/// The encoder is no longer dropped when the socket goes — that is what made
/// every reconnect a fresh mount — so this is the only thing that frees it,
/// and it runs on every path that touches the map.
fn sweep(m: &mut HashMap<String, Resumable>) {
    let now = std::time::Instant::now();
    let gone: Vec<String> = m
        .iter()
        .filter(|(_, r)| {
            r.detached_at
                .is_some_and(|t| now.duration_since(t) >= RESUME_GRACE)
        })
        .map(|(id, _)| id.clone())
        .collect();
    m.retain(|_, r| {
        r.detached_at
            .is_none_or(|t| now.duration_since(t) < RESUME_GRACE)
    });
    for id in gone {
        drop_encoder(&id);
    }
}

/// Mint a handle for a new session and remember what it names.
pub fn open_resumable(handle: [u8; 16], liveview_id: &str, session_id: &str, component: &str) {
    EUI_RESUMABLE.write(|m| {
        sweep(m);
        m.insert(
            liveview_id.to_owned(),
            Resumable {
                handle,
                session_id: session_id.to_owned(),
                component: component.to_owned(),
                replay: std::collections::VecDeque::new(),
                detached_at: None,
            },
        );
    });
}

/// The socket went. Start the clock; the state stays until it runs out.
pub fn detach_resumable(liveview_id: &str) {
    EUI_RESUMABLE.write(|m| {
        sweep(m);
        if let Some(r) = m.get_mut(liveview_id) {
            r.detached_at = Some(std::time::Instant::now());
        }
    });
}

/// Forget a session outright: it ended, rather than lost its socket.
pub fn close_resumable(liveview_id: &str) {
    EUI_RESUMABLE.write(|m| {
        sweep(m);
        m.remove(liveview_id);
    });
    drop_encoder(liveview_id);
}

/// What an offered handle may be picked up as: the instance to re-attach to
/// and the batches owed after `acked`.
///
/// `None` unless the handle names a live detached session of **this** cookie
/// session and **this** component, and the replay still reaches back to what
/// the client says it applied. A gap is a refusal: 01 §4.1 says a resumed
/// session is the session, and a client sent a tree with a hole in it would
/// be told nothing was wrong.
pub fn take_resumable(
    handle: [u8; 16],
    session_id: &str,
    component: &str,
    acked: u64,
) -> Option<(String, Vec<Vec<u8>>)> {
    EUI_RESUMABLE.write(|m| {
        sweep(m);
        // By handle, which is the rare road: once per reconnect, against a
        // map holding one entry per live session.
        let id = m
            .iter()
            .find(|(_, r)| r.handle == handle)
            .map(|(id, _)| id.clone())?;
        let r = m.get_mut(&id)?;
        if r.session_id != session_id || r.component != component || r.detached_at.is_none() {
            return None;
        }
        // Everything after what the client applied. `acked` of zero is a
        // client that applied nothing, and the buffer must then hold the
        // mount itself for this to be honest.
        let oldest = r.replay.front().map(|(seq, _)| *seq);
        if let Some(first) = oldest {
            if acked.saturating_add(1) < first {
                return None;
            }
        }
        let owed: Vec<Vec<u8>> = r
            .replay
            .iter()
            .filter(|(seq, _)| *seq > acked)
            .map(|(_, b)| b.clone())
            .collect();
        r.detached_at = None;
        Some((id, owed))
    })
}

/// Keep a batch for a reconnect, and drop what the client has applied.
fn remember_batch(liveview_id: &str, seq: u64, bytes: &[u8]) {
    EUI_RESUMABLE.write(|m| {
        if let Some(r) = m.get_mut(liveview_id) {
            r.replay.push_back((seq, bytes.to_vec()));
            while r.replay.len() > MAX_REPLAY {
                r.replay.pop_front();
            }
        }
    });
}

/// The client applied everything up to `seq`; the rest is no longer owed.
pub fn note_acked(liveview_id: &str, seq: u64) {
    EUI_RESUMABLE.write(|m| {
        if let Some(r) = m.get_mut(liveview_id) {
            r.replay.retain(|(s, _)| *s > seq);
        }
    });
}

/// The synthetic event the socket posts when the client asks for a resync:
/// the handler is not run, the tree is re-sent whole.
pub const RESYNC_EVENT: &str = "__eui_resync";

/// The synthetic event the socket posts to the session's worker once the
/// session is gone, so the worker drops what it kept for it. The memo is
/// thread-local — it holds interpreter values — so nobody else can.
pub const FORGET_EVENT: &str = "__eui_forget";

/// Components whose socket needs a session cookie: `router_eui(component,
/// handler, view, {"session": "required"})`.
/// Per application: whether a component's socket needs a session cookie is one
/// app's policy, and refusing the upgrade is a security decision.
static EUI_SESSION_REQUIRED: TenantValue<std::collections::HashSet<String>> =
    TenantValue::new(std::collections::HashSet::new);

/// Components an application will also serve as a one-shot render over
/// `GET /_eui/view/<component>`, and the `Cache-Control` each wants on it:
/// `router_eui(component, handler, view, {"static": "public, max-age=60"})`.
///
/// Per application, like the two above. Whether a component's first render
/// may be handed to anybody who asks is one application's decision about one
/// application's pages, and a co-hosted neighbour must not inherit it.
static EUI_STATIC: TenantValue<HashMap<String, String>> = TenantValue::new(HashMap::new);

/// Which component a bare origin opens: the manifest's `entry` (EUI 01 §2.1).
///
/// The protocol has one entry and a server here has many components, so one
/// of them has to be the one `wss://host` means. The first `router_eui` in
/// the routes file is it, which is the one a person reading that file top to
/// bottom would say, and `{"default": true}` on any of them says otherwise.
static EUI_DEFAULT: TenantValue<Option<String>> = TenantValue::new(|| None);

/// Name `component` as the one a bare origin opens. `first` is a
/// registration claiming it only because nothing else has.
pub fn set_default(component: &str, first: bool) {
    EUI_DEFAULT.write(|d| {
        if !first || d.is_none() {
            *d = Some(component.to_string());
        }
    });
}

/// The component a bare origin opens, if this application registered any.
///
/// `None` is how the rest of the server asks "is this an EUI application at
/// all", which is the question behind the page a browser gets when nothing
/// else answers.
pub fn default_component() -> Option<String> {
    EUI_DEFAULT.read(Clone::clone)
}

/// The session path the manifest advertises: the default component's, or
/// the bare endpoint when no component has been registered at all.
pub fn entry_path() -> String {
    EUI_DEFAULT.read(|d| {
        d.as_ref().map_or_else(
            || "/_eui/session".to_string(),
            |c| format!("/_eui/session/{c}"),
        )
    })
}

/// Serve `component` as a one-shot render too, with `cache_control` on it.
pub fn allow_static(component: &str, cache_control: &str) {
    EUI_STATIC.write(|map| {
        map.insert(component.to_string(), cache_control.to_string());
    });
}

/// The `Cache-Control` for a component that may be served as a one-shot
/// render, or `None` when it may not.
///
/// One lookup answers both questions the endpoint has — is this a component,
/// and did it opt in — because the answer to either on its own is a 404.
pub fn static_cache_control(component: &str) -> Option<String> {
    EUI_STATIC.read(|map| map.get(component).cloned())
}

/// Refuse the upgrade for `component` unless the request carries a session.
pub fn require_session(component: &str) {
    EUI_SESSION_REQUIRED.write(|set| set.insert(component.to_string()));
}

/// True when `router_eui` declared this component needs a session.
pub fn session_required(component: &str) -> bool {
    EUI_SESSION_REQUIRED.read(|set| set.contains(component))
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

/// `EUI_TRACE=1` in the environment: a line per frame on stderr. Read once;
/// it was read from the environment several times per event.
pub(super) fn trace() -> bool {
    static TRACE: std::sync::LazyLock<bool> =
        std::sync::LazyLock::new(|| std::env::var_os("EUI_TRACE").is_some());
    *TRACE
}

/// The worker side of [`FORGET_EVENT`].
pub fn forget_on_worker(liveview_id: &str) {
    tree::forget_session(liveview_id);
}

/// True while the instance's encoder is still around: the session is live.
pub fn has_encoder(liveview_id: &str) -> bool {
    EUI_ENCODERS.read(|m| m.contains_key(liveview_id))
}

/// Register a component's view action.
pub fn register_view(component: &str, view: &str) {
    set_default(component, true);
    EUI_VIEWS.write(|m| m.insert(component.to_string(), view.to_string()));
}

/// True when `router_eui` registered this component.
pub fn is_eui_component(component: &str) -> bool {
    EUI_VIEWS.read(|m| m.contains_key(component))
}

/// Render every live session of a component, now, because something the
/// server knows changed.
///
/// Spec 06 §1.1 gives an application one clock and it belongs to the client:
/// a node carrying `wake` is sent an event on a period, and the server can
/// answer no faster than that period. That is the right answer for watching
/// something that expires — a presence that goes stale, a countdown — and the
/// wrong one for something that *happened*: a message written in one window
/// reaches the next one after up to a whole period, and every window pays a
/// render a period to find out that nothing did.
///
/// The socket is bidirectional and a `Batch` is already S→C (01 §3): nothing
/// in the protocol ties one to an `Event`, and the client applies whatever
/// arrives. What was missing was the trigger, and the machinery for it was
/// all here — the send side is a queue drained by its own task, a rendered
/// batch goes to the instance's senders rather than back along the event, and
/// the worker a session is pinned to is `lv_sender_for`'s to find. So this is
/// that trigger and nothing more: one synthetic event per session, posted to
/// the worker that owns it, which then runs the handler and the view exactly
/// as a client's event would.
///
/// `except` is the session that asked — the one already rendering, whose own
/// batch is on its way. Waking it would render it twice for one change.
///
/// Returns how many sessions were told. Each is a render, so a caller that
/// wakes a busy component on every keystroke has moved the cost rather than
/// removed it: this is for the moments that change what other people see.
///
/// A worker whose queue is full drops the wake rather than blocking on it,
/// and that is the right failure: the window's own clock still brings it
/// round, a little later, which is exactly where it was before this existed.
///
/// A handler that *writes* when it is woken wakes everyone else in turn, who
/// write, and so on: that is a loop, and dropping wakes at a full queue only
/// slows it down. Wake for what was written; do not write for what was woken.
pub fn wake_component(component: &str, event: &str, except: Option<&str>) -> usize {
    if !is_eui_component(component) {
        return 0;
    }
    let mut woken = 0;
    for (id, session) in live_registry().attached_of_component(component) {
        if except.is_some_and(|self_id| self_id == id) {
            continue;
        }
        let Some(tx) = super::lv_sender_for(&id, component) else {
            continue;
        };
        // Fire and forget. The oneshot's other end is dropped here: a
        // server-side wake has nobody to report to, and a worker that
        // answers a closed channel is not an error — the socket path uses
        // the reply only to decide whether to close the session, and a
        // session is not closed because a wake was not taken.
        let (response_tx, _) = tokio::sync::oneshot::channel();
        if tx
            .try_send(LiveViewEventData {
                liveview_id: id.clone(),
                component: component.to_string(),
                event: event.to_string(),
                params: serde_json::json!({}),
                // As the window's own person: a render woken from outside is
                // still that session's render, and a handler that asks who is
                // looking must not get a different answer for it.
                sender_session: Some(session),
                response_tx,
            })
            .is_ok()
        {
            woken += 1;
        }
    }
    woken
}

fn view_action(component: &str) -> Option<String> {
    EUI_VIEWS.read(|m| m.get(component).cloned())
}

/// Run `f` against the instance's encoder, creating it on first use.
pub fn with_encoder<T>(liveview_id: &str, f: impl FnOnce(&mut tree::Encoder) -> T) -> T {
    let cell = EUI_ENCODERS
        .write(|map| std::sync::Arc::clone(map.entry(liveview_id.to_string()).or_default()));
    // The map is unlocked here: the encode below is this instance's own.
    let mut encoder = cell.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut encoder)
}

/// Forget an instance's encoder when its socket is gone for good. The memo
/// is not touched here: it lives on the session's worker thread, and this
/// runs on the socket's task — the socket posts [`FORGET_EVENT`] for that.
pub fn drop_encoder(liveview_id: &str) {
    stats::forget(liveview_id);
    EUI_ENCODERS.write(|map| map.remove(liveview_id));
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
    send_bytes(instance, frame.encode(), deadline)
}

/// The same, for a frame already encoded — a `Batch` is encoded once and kept
/// for a reconnect (01 §4.1) before it is sent, and encoding it twice to do
/// both would put the cost of resume on every render.
fn send_bytes(instance: &LiveViewInstance, bytes: Vec<u8>, deadline: std::time::Instant) -> usize {
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
    if trace() {
        // The state is the application's, secrets included: printed in
        // --dev only, whatever the environment says.
        if crate::interpreter::builtins::template::is_dev_mode() {
            eprintln!(
                "[EUI trace] worker: {component} {} state={}",
                data.event, instance.state
            );
        } else {
            eprintln!("[EUI trace] worker: {component} {}", data.event);
        }
    }

    // The state as the view will see it. A handler that returned the bare
    // state hash — the common shape — hands its own value on to the view,
    // instead of that value going to JSON for the instance and back to a
    // value for the view: two deep copies of the whole state per event.
    // Which session this thread is working for, from here rather than from
    // just before the view: `eui_stats()` is read in a view, but `eui_wake()`
    // is called from a handler, and it needs to know which window is already
    // being answered so it can leave that one out.
    let _current = stats::Current::enter(instance.id.clone());

    let mut state_for_view: Option<Value> = None;
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
                let json = value_to_json(&result);
                let bare = json.as_object().is_some_and(handler_return_is_bare);
                let unwrapped = unwrap_handler_return(json);
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
                    if bare {
                        state_for_view = Some(result);
                    }
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
    let state_value = state_for_view.unwrap_or_else(|| json_to_value(&instance.state));
    let t_view = std::time::Instant::now();
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
    if trace() {
        eprintln!(
            "[EUI trace] worker: view {view_ms:.1} ms, convert+diff+encode {:.1} ms, {} batch(es)",
            t_render.elapsed().as_secs_f64() * 1e3,
            batches.len()
        );
    }

    instance.touch();
    if !live_registry().commit(instance) {
        if trace() {
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
        if trace() {
            eprintln!(
                "[EUI trace] worker: sending batch seq={} ops={} to {} sender(s)",
                batch.seq,
                batch.ops.len(),
                instance.senders.len()
            );
        }
        sent.ops += batch.ops.len();
        sent.seq = batch.seq;
        // 01 §4.1: kept before it is sent, because the send is what may fail.
        // A batch the client never received is exactly the one a resume owes
        // it, and the encoder has already moved on — this is the only copy.
        let bytes = Frame::Batch(batch).encode();
        remember_batch(&instance.id, sent.seq, &bytes);
        sent.bytes += send_bytes(instance, bytes, deadline);
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
        // Room for one frame and three to send, so the sender waits for
        // room at least twice: that is the path under test.
        let (tx, rx) = async_channel::bounded::<Result<Message, tungstenite::Error>>(1);
        // A second receiver, held here and never read from. Without it the
        // channel counts as closed the moment the draining thread has its
        // three frames and drops its own — and `is_closed` below would be
        // reading that, not whether the sender gave the socket up. In the
        // server the socket's receiver outlives every send the same way.
        let still_open = rx.clone();
        // A minute, because the deadline is not what this tests: a machine
        // busy enough that the reading thread waits seconds to be
        // scheduled must not read as a sender giving up on it. What a
        // deadline does to a reader that never drains is the test above.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        let drain = std::thread::spawn(move || {
            let mut got = Vec::new();
            while let Ok(Ok(Message::Binary(bytes))) = rx.recv_blocking() {
                got.push(bytes[0]);
                if got.len() == 3 {
                    break;
                }
            }
            got
        });
        for n in 0..3u8 {
            send_or_close(&tx, vec![n], deadline);
        }
        assert_eq!(drain.join().unwrap(), vec![0, 1, 2], "in order, none lost");
        assert!(!tx.is_closed(), "a draining reader is never given up on");
        drop(still_open);
    }
}

#[cfg(test)]
mod resume_tests {
    use super::*;

    fn batch_bytes(seq: u64) -> Vec<u8> {
        Frame::Batch(eui_proto::Batch {
            seq,
            ops: Vec::new(),
        })
        .encode()
    }

    /// 01 §4.1: a socket that breaks is not an application that ended. The
    /// handle names the session, and what comes back is the batches after
    /// the one the client says it applied — not the whole tree.
    #[test]
    fn a_resumed_session_is_owed_only_what_it_did_not_apply() {
        let h = mint_handle();
        open_resumable(h, "view-1", "sess-a", "counter");
        for seq in 1..=5 {
            remember_batch("view-1", seq, &batch_bytes(seq));
        }
        detach_resumable("view-1");

        let (id, owed) = take_resumable(h, "sess-a", "counter", 3).expect("still here");
        assert_eq!(id, "view-1");
        assert_eq!(owed.len(), 2, "four and five, not the tree");
        assert_eq!(owed[0], batch_bytes(4));
        assert_eq!(owed[1], batch_bytes(5));
        close_resumable("view-1");
    }

    /// The handle is a bearer (01 §4.1), so it is not the whole of the check.
    /// Another person's cookie, or another component, resumes nothing — the
    /// alternative is that sixteen bytes are enough to be handed somebody
    /// else's tree, with their data in it.
    #[test]
    fn a_handle_alone_does_not_open_somebody_elses_session() {
        let h = mint_handle();
        open_resumable(h, "view-2", "sess-owner", "inbox");
        remember_batch("view-2", 1, &batch_bytes(1));
        detach_resumable("view-2");

        assert!(
            take_resumable(h, "sess-thief", "inbox", 0).is_none(),
            "another cookie"
        );
        assert!(
            take_resumable(h, "sess-owner", "settings", 0).is_none(),
            "another component"
        );
        assert!(
            take_resumable(mint_handle(), "sess-owner", "inbox", 0).is_none(),
            "a handle nobody minted"
        );
        // And the real one still works afterwards: a refusal consumes nothing.
        assert!(take_resumable(h, "sess-owner", "inbox", 0).is_some());
        close_resumable("view-2");
    }

    /// A session whose socket is still attached is not resumable: two live
    /// sockets on one session would both be told they own its tree.
    #[test]
    fn a_session_that_never_lost_its_socket_is_not_picked_up() {
        let h = mint_handle();
        open_resumable(h, "view-3", "sess-a", "counter");
        assert!(
            take_resumable(h, "sess-a", "counter", 0).is_none(),
            "still attached"
        );
        detach_resumable("view-3");
        assert!(take_resumable(h, "sess-a", "counter", 0).is_some());
        close_resumable("view-3");
    }

    /// A gap is a refusal. The buffer holds at most `MAX_REPLAY`, and a
    /// client that fell further behind than that cannot be caught up — being
    /// told it was resumed and handed a tree with a hole in it is worse than
    /// being told to start again, because nothing would say so.
    #[test]
    fn a_client_too_far_behind_is_refused_rather_than_half_filled() {
        let h = mint_handle();
        open_resumable(h, "view-4", "sess-a", "counter");
        for seq in 1..=(MAX_REPLAY as u64 + 10) {
            remember_batch("view-4", seq, &batch_bytes(seq));
        }
        detach_resumable("view-4");
        assert!(
            take_resumable(h, "sess-a", "counter", 2).is_none(),
            "the buffer no longer reaches back that far"
        );
        close_resumable("view-4");
    }

    /// What the client acknowledged is no longer owed, or the buffer would
    /// hold the last sixty-four batches of every session for ever.
    #[test]
    fn an_ack_drops_what_it_names() {
        let h = mint_handle();
        open_resumable(h, "view-5", "sess-a", "counter");
        for seq in 1..=4 {
            remember_batch("view-5", seq, &batch_bytes(seq));
        }
        note_acked("view-5", 4);
        detach_resumable("view-5");
        let (_, owed) = take_resumable(h, "sess-a", "counter", 4).expect("still here");
        assert!(owed.is_empty(), "{owed:?}");
        close_resumable("view-5");
    }
}
