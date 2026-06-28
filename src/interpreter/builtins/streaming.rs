//! Handler-driven streaming responses (SSE + chunked).
//!
//! A controller returns `sse(req) do |out| ... end` (or `stream(req, ct) do
//! |out| ... end`); the `sse`/`stream` builtin stashes the block as a pending
//! stream (thread-local) and returns a benign 200 sentinel. The serve worker,
//! after the request handler runs, detects the pending stream: it opens a
//! chunk channel, hands the receiver back to the async hyper task as the
//! response body, then runs the block — `out.emit(...)`/`out.write(...)` push
//! formatted frames into the channel as the block executes. The worker thread
//! is occupied for the stream's lifetime.
//!
//! The chunk sender lives in a process-global registry keyed by an integer id
//! (the same pattern as the Pop3 connection registry); the `out` instance
//! carries that id.

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

/// A stream a controller asked for, captured during the request handler run.
pub struct StreamSpec {
    pub block: Value,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    /// SSE framing (`data:`/`event:`) vs raw chunked bytes.
    pub sse: bool,
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
    });
    Ok(sentinel())
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
