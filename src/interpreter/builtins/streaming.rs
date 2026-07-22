//! Streaming responses — two flavors:
//!
//! 1. **Handler-driven** (`sse(req) do |out| ... end`, `stream(req, ct) do
//!    |out| ... end`): the builtin stashes the block as a pending stream
//!    (thread-local) and returns a 200 sentinel. The worker opens a chunk
//!    channel, hands the receiver to the async hyper task as the body, then
//!    **runs the block on the worker thread** — `out.emit`/`out.write` push
//!    frames as it executes. Good for finite, active streams (an agent run, an
//!    export); the worker is held for the stream's lifetime.
//!
//! 2. **Async pub/sub** (`sse_subscribe(req, topic)` + `sse_broadcast(topic,
//!    data, event?)`): the worker registers the chunk sender under the topic
//!    and **returns immediately** — the connection lives only as the async
//!    StreamBody, so thousands of mostly-idle subscribers cost async-task
//!    memory, not threads. Events are pushed from anywhere via `sse_broadcast`.
//!    Good for fan-out (dashboards, notifications).
//!
//! Senders live in process-global registries (the Pop3-connection pattern):
//! id→sender for the handler-driven path, topic→[senders] for pub/sub.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use lazy_static::lazy_static;
use tokio::sync::mpsc::Sender;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, Instance, NativeFunction, Value};

lazy_static! {
    /// id -> chunk sender. `tokio::mpsc::Sender` is `Send + Sync + Clone`.
    static ref SENDERS: Mutex<HashMap<usize, Sender<Vec<u8>>>> = Mutex::new(HashMap::new());
}
static NEXT_ID: AtomicUsize = AtomicUsize::new(1);

/// Register a chunk sender, returning its id (stored on the `out` instance).
pub fn register_sender(tx: Sender<Vec<u8>>) -> usize {
    let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
    SENDERS.lock().unwrap().insert(id, tx);
    id
}

/// Drop the sender for `id` — closes the stream (the receiver sees end-of-body).
pub fn unregister_sender(id: usize) {
    SENDERS.lock().unwrap().remove(&id);
}

/// Push one body chunk. Returns false if the client has disconnected (the
/// receiver was dropped) so the block can stop early.
fn send_chunk(id: usize, bytes: Vec<u8>) -> bool {
    let tx = SENDERS.lock().unwrap().get(&id).cloned();
    match tx {
        // Safe: the worker thread is not inside a tokio runtime when running
        // the stream block, so blocking_send won't panic. It blocks (back-
        // pressure) when the client is slow and errs when the client is gone.
        Some(tx) => tx.blocking_send(bytes).is_ok(),
        None => false,
    }
}

// ---------------------------------------------------------------------------
// Async pub/sub subscribers (topic -> live chunk senders)
//
// Unlike the handler-driven path above (which holds a worker thread for the
// stream's lifetime), a subscription's connection lives as the async hyper
// StreamBody; the worker only registers the sender and returns. Events are
// pushed from anywhere via `sse_broadcast`. Thousands of mostly-idle
// connections cost async-task memory, not OS threads.
// ---------------------------------------------------------------------------

lazy_static! {
    static ref SUBSCRIBERS: Mutex<HashMap<String, Vec<Sender<Vec<u8>>>>> =
        Mutex::new(HashMap::new());
}

/// Register a subscriber's chunk sender under `topic`.
pub fn register_subscriber(topic: &str, tx: Sender<Vec<u8>>) {
    SUBSCRIBERS
        .lock()
        .unwrap()
        .entry(topic.to_string())
        .or_default()
        .push(tx);
}

/// Fan out `bytes` to every live subscriber of `topic`, pruning disconnected
/// ones. Non-blocking: a slow client whose buffer is full drops this message
/// (but keeps its subscription). Returns the number delivered.
fn broadcast_bytes(topic: &str, bytes: Vec<u8>) -> usize {
    use tokio::sync::mpsc::error::TrySendError;
    let mut subs = SUBSCRIBERS.lock().unwrap();
    let Some(list) = subs.get_mut(topic) else {
        return 0;
    };
    let mut delivered = 0;
    list.retain(|tx| {
        if tx.is_closed() {
            return false; // client disconnected
        }
        match tx.try_send(bytes.clone()) {
            Ok(()) => {
                delivered += 1;
                true
            }
            Err(TrySendError::Full(_)) => true, // slow client: drop msg, keep sub
            Err(TrySendError::Closed(_)) => false,
        }
    });
    if list.is_empty() {
        subs.remove(topic);
    }
    delivered
}

/// Broadcast `data` as an SSE frame to every subscriber of `topic`. Public
/// entry point for cross-module broadcasters (e.g. the `broadcast(...)` builtin
/// / `Model.broadcast`). Non-blocking; returns the number delivered.
pub fn broadcast_sse(topic: &str, data: &str, event: Option<&str>) -> usize {
    broadcast_bytes(topic, format_sse(data, event).into_bytes())
}

/// Number of live subscribers on `topic`, for cross-module callers (the
/// `Native.*` builtins ask this to decide whether a push fallback is needed).
pub fn subscriber_count_for(topic: &str) -> usize {
    subscriber_count(topic)
}

/// Number of live subscribers on `topic` (best-effort; prunes closed ones).
fn subscriber_count(topic: &str) -> usize {
    let mut subs = SUBSCRIBERS.lock().unwrap();
    let Some(list) = subs.get_mut(topic) else {
        return 0;
    };
    list.retain(|tx| !tx.is_closed());
    let n = list.len();
    if n == 0 {
        subs.remove(topic);
    }
    n
}

/// A stream a controller asked for, captured during the request handler run.
pub struct StreamSpec {
    /// The handler block for `sse`/`stream` (run on the worker). Unused — and
    /// `Value::Null` — for `sse_subscribe`, which has no block.
    pub block: Value,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    /// SSE framing (`data:`/`event:`) vs raw chunked bytes.
    pub sse: bool,
    /// When set, this is an async pub/sub subscription: the worker registers
    /// the chunk sender under this topic and returns immediately (no block,
    /// no worker held). Events arrive via `sse_broadcast`.
    pub subscribe_topic: Option<String>,
}

thread_local! {
    static PENDING: RefCell<Option<StreamSpec>> = const { RefCell::new(None) };
    // The StreamOut class for this worker, built at builtin registration.
    static OUT_CLASS: RefCell<Option<Rc<Class>>> = const { RefCell::new(None) };
}

/// Take (and clear) the pending stream set by `sse`/`stream` during this
/// request, if any.
pub fn take_pending_stream() -> Option<StreamSpec> {
    PENDING.with(|p| p.borrow_mut().take())
}

/// Clear any stale pending stream (defensive, at request start).
pub fn clear_pending_stream() {
    PENDING.with(|p| *p.borrow_mut() = None);
}

fn set_pending(spec: StreamSpec) {
    PENDING.with(|p| *p.borrow_mut() = Some(spec));
}

// ---------------------------------------------------------------------------
// `out` emitter
// ---------------------------------------------------------------------------

fn out_instance(id: usize, sse: bool) -> Value {
    let class = OUT_CLASS.with(|c| c.borrow().clone());
    let class = match class {
        Some(c) => c,
        // Should never happen (registered at boot); fall back to a bare class.
        None => Rc::new(Class {
            name: "StreamOut".to_string(),
            ..Default::default()
        }),
    };
    let mut inst = Instance::new(class);
    inst.set("_id".to_string(), Value::Int(id as i64));
    inst.set("_sse".to_string(), Value::Bool(sse));
    Value::Instance(Rc::new(RefCell::new(inst)))
}

fn instance_fields(args: &[Value]) -> Option<(usize, bool)> {
    if let Some(Value::Instance(inst)) = args.first() {
        let inst = inst.borrow();
        let id = match inst.get("_id") {
            Some(Value::Int(n)) => n as usize,
            _ => return None,
        };
        let sse = matches!(inst.get("_sse"), Some(Value::Bool(true)));
        Some((id, sse))
    } else {
        None
    }
}

fn as_text(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.to_string(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

/// Format one SSE event: optional `event:` line, one `data:` line per payload
/// line, terminated by a blank line.
fn format_sse(data: &str, event: Option<&str>) -> String {
    let mut out = String::new();
    if let Some(ev) = event {
        if !ev.is_empty() {
            out.push_str("event: ");
            out.push_str(ev);
            out.push('\n');
        }
    }
    for line in data.split('\n') {
        out.push_str("data: ");
        out.push_str(line);
        out.push('\n');
    }
    out.push('\n');
    out
}

/// `out.emit(data, event?)` — emit an SSE event (or raw text when not in SSE
/// mode). Named `emit`, not `send`, because `send` is the universal
/// metaprogramming method (it would treat the payload as a method name).
/// Returns false if the client has disconnected.
fn out_emit(args: Vec<Value>) -> Result<Value, String> {
    let (id, sse) = instance_fields(&args)
        .ok_or_else(|| "out.emit() must be called on a stream emitter".to_string())?;
    let data = as_text(args.get(1));
    let bytes = if sse {
        let event = match args.get(2) {
            Some(Value::String(s)) => Some(s.to_string()),
            _ => None,
        };
        format_sse(&data, event.as_deref()).into_bytes()
    } else {
        data.into_bytes()
    };
    Ok(Value::Bool(send_chunk(id, bytes)))
}

/// `out.write(data)` — emit a raw body chunk (no SSE framing).
fn out_write(args: Vec<Value>) -> Result<Value, String> {
    let (id, _sse) = instance_fields(&args)
        .ok_or_else(|| "out.write() must be called on a stream emitter".to_string())?;
    let data = as_text(args.get(1));
    Ok(Value::Bool(send_chunk(id, data.into_bytes())))
}

// ---------------------------------------------------------------------------
// `sse` / `stream` builtins
// ---------------------------------------------------------------------------

/// The trailing block argument (native fns receive a block as the last arg).
fn take_block(args: &[Value]) -> Option<Value> {
    args.iter()
        .rev()
        .find(|v| matches!(v, Value::Function(_)))
        .cloned()
}

fn sse_builtin(args: Vec<Value>) -> Result<Value, String> {
    let block = take_block(&args).ok_or_else(|| {
        "sse(req) requires a block: sse(req) do |out| out.emit(...) end".to_string()
    })?;
    set_pending(StreamSpec {
        block,
        status: 200,
        headers: vec![
            ("Content-Type".to_string(), "text/event-stream".to_string()),
            ("Cache-Control".to_string(), "no-cache".to_string()),
            ("Connection".to_string(), "keep-alive".to_string()),
            // Disable proxy buffering so events flush promptly (nginx).
            ("X-Accel-Buffering".to_string(), "no".to_string()),
        ],
        sse: true,
        subscribe_topic: None,
    });
    Ok(sentinel())
}

fn stream_builtin(args: Vec<Value>) -> Result<Value, String> {
    let block = take_block(&args).ok_or_else(|| {
        "stream(req, content_type) requires a block: stream(req, \"text/csv\") do |out| ... end"
            .to_string()
    })?;
    let content_type = match args.get(1) {
        Some(Value::String(s)) => s.to_string(),
        _ => "application/octet-stream".to_string(),
    };
    set_pending(StreamSpec {
        block,
        status: 200,
        headers: vec![
            ("Content-Type".to_string(), content_type),
            ("Cache-Control".to_string(), "no-cache".to_string()),
            ("X-Accel-Buffering".to_string(), "no".to_string()),
        ],
        sse: false,
        subscribe_topic: None,
    });
    Ok(sentinel())
}

/// The last string argument (for `sse_subscribe(req, topic)` / the topic).
fn last_string(args: &[Value]) -> Option<String> {
    args.iter().rev().find_map(|v| match v {
        Value::String(s) => Some(s.to_string()),
        _ => None,
    })
}

/// `sse_subscribe(req, topic)` — open a long-lived SSE connection subscribed to
/// `topic`. The worker registers the connection and returns immediately (no
/// worker held); events arrive via `sse_broadcast`.
fn sse_subscribe_builtin(args: Vec<Value>) -> Result<Value, String> {
    let topic = last_string(&args)
        .ok_or_else(|| "sse_subscribe(req, topic) requires a topic string".to_string())?;
    set_pending(StreamSpec {
        block: Value::Null,
        status: 200,
        headers: vec![
            ("Content-Type".to_string(), "text/event-stream".to_string()),
            ("Cache-Control".to_string(), "no-cache".to_string()),
            ("Connection".to_string(), "keep-alive".to_string()),
            ("X-Accel-Buffering".to_string(), "no".to_string()),
        ],
        sse: true,
        subscribe_topic: Some(topic),
    });
    Ok(sentinel())
}

/// `sse_broadcast(topic, data, event?)` — push an SSE event to every live
/// subscriber of `topic` (from a controller, job, or callback). Returns the
/// number of clients reached. Safe to call from any thread.
fn sse_broadcast_builtin(args: Vec<Value>) -> Result<Value, String> {
    let topic = match args.first() {
        Some(Value::String(s)) => s.to_string(),
        _ => return Err("sse_broadcast(topic, data, event?) requires a topic string".to_string()),
    };
    let data = as_text(args.get(1));
    let event = match args.get(2) {
        Some(Value::String(s)) => Some(s.to_string()),
        _ => None,
    };
    let frame = format_sse(&data, event.as_deref());
    Ok(Value::Int(
        broadcast_bytes(&topic, frame.into_bytes()) as i64
    ))
}

/// `sse_subscribers(topic)` — count live subscribers on `topic`.
fn sse_subscribers_builtin(args: Vec<Value>) -> Result<Value, String> {
    let topic = match args.first() {
        Some(Value::String(s)) => s.to_string(),
        _ => return Err("sse_subscribers(topic) requires a topic string".to_string()),
    };
    Ok(Value::Int(subscriber_count(&topic) as i64))
}

/// Benign value the request handler turns into a 200 — discarded by the worker
/// once it sees the pending stream.
fn sentinel() -> Value {
    use crate::interpreter::value::{hash_from_pairs, Value as V};
    hash_from_pairs([
        ("status".to_string(), V::Int(200)),
        ("body".to_string(), V::String("".into())),
    ])
}

/// Run the stream block to completion, emitting chunks through sender `id`.
pub fn run_stream_block(
    interpreter: &mut crate::interpreter::Interpreter,
    spec: &StreamSpec,
    id: usize,
) {
    let func = match &spec.block {
        Value::Function(f) => f.clone(),
        _ => return,
    };
    let out = out_instance(id, spec.sse);
    if let Err(e) = interpreter.call_function(&func, vec![out]) {
        eprintln!("[stream] block error: {e}");
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// `out.llm_stream(system, user)` — stream an LLM completion token-by-token
/// into this SSE/chunked response. Each content delta is emitted as it arrives
/// (an SSE `data:` frame in SSE mode, raw text otherwise); returns the full
/// accumulated answer so the handler can persist it. Stops early if the client
/// disconnects. Errors when the LLM isn't configured.
fn out_llm_stream(args: Vec<Value>) -> Result<Value, String> {
    let (id, sse) = instance_fields(&args).ok_or_else(|| {
        "out.llm_stream(system, user) must be called on a stream emitter".to_string()
    })?;
    let system = as_text(args.get(1));
    let user = match args.get(2) {
        Some(_) => as_text(args.get(2)),
        None => return Err("out.llm_stream(system, user) requires a user prompt".to_string()),
    };
    let full = crate::generation::generate_completion_stream(&system, &user, |token| {
        let bytes = if sse {
            format_sse(token, None).into_bytes()
        } else {
            token.as_bytes().to_vec()
        };
        send_chunk(id, bytes)
    });
    match full {
        Some(text) => Ok(Value::String(text.into())),
        None => Err("out.llm_stream: LLM not configured or the request failed \
             (set SOLI_LLM_API_KEY / SOLI_LLM_URL)"
            .to_string()),
    }
}

pub fn register_streaming_builtins(env: &mut Environment) {
    let mut methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();
    methods.insert(
        "emit".to_string(),
        Rc::new(NativeFunction::new("StreamOut.emit", None, out_emit)),
    );
    methods.insert(
        "write".to_string(),
        Rc::new(NativeFunction::new("StreamOut.write", Some(1), out_write)),
    );
    methods.insert(
        "llm_stream".to_string(),
        Rc::new(NativeFunction::new(
            "StreamOut.llm_stream",
            None,
            out_llm_stream,
        )),
    );
    let class = Rc::new(Class {
        name: "StreamOut".to_string(),
        native_methods: methods,
        ..Default::default()
    });
    OUT_CLASS.with(|c| *c.borrow_mut() = Some(class));

    env.define(
        "sse".to_string(),
        Value::NativeFunction(NativeFunction::new("sse", None, sse_builtin)),
    );
    env.define(
        "stream".to_string(),
        Value::NativeFunction(NativeFunction::new("stream", None, stream_builtin)),
    );
    env.define(
        "sse_subscribe".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "sse_subscribe",
            None,
            sse_subscribe_builtin,
        )),
    );
    env.define(
        "sse_broadcast".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "sse_broadcast",
            None,
            sse_broadcast_builtin,
        )),
    );
    env.define(
        "sse_subscribers".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "sse_subscribers",
            Some(1),
            sse_subscribers_builtin,
        )),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_framing_handles_event_and_multiline() {
        assert_eq!(
            format_sse("hello", Some("tick")),
            "event: tick\ndata: hello\n\n"
        );
        assert_eq!(format_sse("a\nb", None), "data: a\ndata: b\n\n");
        // an empty event name is omitted
        assert_eq!(format_sse("x", Some("")), "data: x\n\n");
    }

    #[test]
    fn broadcast_fans_out_and_prunes_dead_subscribers() {
        let topic = "unit_topic_broadcast";
        let (tx1, mut rx1) = tokio::sync::mpsc::channel::<Vec<u8>>(4);
        let (tx2, mut rx2) = tokio::sync::mpsc::channel::<Vec<u8>>(4);
        register_subscriber(topic, tx1);
        register_subscriber(topic, tx2);
        assert_eq!(subscriber_count(topic), 2);

        assert_eq!(broadcast_bytes(topic, b"x".to_vec()), 2);
        assert_eq!(rx1.try_recv().unwrap(), b"x".to_vec());
        assert_eq!(rx2.try_recv().unwrap(), b"x".to_vec());

        drop(rx2); // client 2 disconnects
        assert_eq!(broadcast_bytes(topic, b"y".to_vec()), 1); // only rx1; rx2 pruned
        assert_eq!(subscriber_count(topic), 1);
    }

    #[test]
    fn broadcast_sse_delivers_a_framed_event() {
        let topic = "unit_topic_broadcast_sse";
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(4);
        register_subscriber(topic, tx);
        // Public wrapper: frames the payload and fans out. Returns the count.
        assert_eq!(broadcast_sse(topic, "{\"id\":7}", None), 1);
        let frame = String::from_utf8(rx.try_recv().unwrap()).unwrap();
        assert_eq!(frame, "data: {\"id\":7}\n\n");
        // No subscribers on an unknown topic -> zero delivered.
        assert_eq!(broadcast_sse("unit_topic_none", "x", None), 0);
    }

    #[test]
    fn sender_registry_round_trips_and_drops() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(4);
        let id = register_sender(tx);
        assert!(send_chunk(id, b"chunk".to_vec()));
        assert_eq!(rx.try_recv().unwrap(), b"chunk".to_vec());
        unregister_sender(id);
        // sender gone -> reports disconnect
        assert!(!send_chunk(id, b"x".to_vec()));
    }
}
