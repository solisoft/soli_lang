//! The `/_eui/session/<component>` socket.
//!
//! Mirrors the LiveView socket: one hyper-tungstenite upgrade, one instance in
//! `LIVE_REGISTRY`, events posted to the worker channel. The differences are
//! the wire — binary EUI frames, decoded and validated here — and that a
//! client event is checked against the tree the client was last sent before
//! it becomes a handler call (`spec/06-events.md` §4).

use std::path::PathBuf;
use std::sync::Arc;

use bytes::Bytes;
use eui_proto::{EventFrame, Frame, Welcome, PROTOCOL_VERSION};
use futures_util::{SinkExt, StreamExt};
use hyper::body::Incoming;
use hyper::{Request, Response, StatusCode};
use tokio::sync::oneshot;
use tungstenite::Message;

use crate::live::liveview_instance_id;
use crate::live::view::{LiveViewInstance, LIVE_REGISTRY};

use super::super::{box_full, default_websocket_config, full, LiveViewEventData, ResponseBody};
use super::{trace, tree, with_encoder, RESYNC_EVENT};
use crate::serve::websocket::{WsConnectionSlot, WsRateLimiter};

type WsSender = Arc<async_channel::Sender<Result<Message, tungstenite::Error>>>;

/// How long a fresh socket has to say Hello. A client that connects and
/// says nothing used to hold its task and its file descriptor forever.
const HELLO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// How often the server pings, and how many pings may go unanswered before
/// the client is taken for gone. The client pings too, but a half-open
/// connection is only ever found by the side that asks.
const HEARTBEAT: std::time::Duration = std::time::Duration::from_secs(30);
const MAX_UNANSWERED_PINGS: u32 = 2;

/// What a `Resync` costs against the socket's frame budget, in frames: the
/// handler is skipped, but the whole view runs and the whole tree is sent
/// again — the most expensive thing a two-byte frame can ask for.
const RESYNC_COST: f64 = 20.0;

/// How long the socket waits for the worker to answer one event before
/// it gives the session up as one it can no longer describe.
const HANDLER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// A file the person picked, on its way to disk (EUI 01 §6, 10 §5).
///
/// The budget is "two chunks, whatever the file weighs", so nothing here
/// keeps the file: each chunk is written as it arrives and forgotten. What
/// is held is what the next chunk has to be checked against.
struct Upload {
    /// The name the person's machine gave it — never a path (03 §3.2).
    name: String,
    /// What the `file_pick` said it weighs.
    size: u64,
    /// The chunk index that must come next; a gap ends the session.
    next_seq: u32,
    /// What has actually landed, which is what the ceiling is checked
    /// against — the announced size is the client's word, not a fact.
    written: u64,
    /// Where it is going, relative to the application root, because that is
    /// the shape every path in a Soli view already has.
    rel: String,
    /// Where it is going, in full.
    path: PathBuf,
    file: std::fs::File,
}

/// The largest upload this server will take when the view's `pick` named no
/// ceiling of its own. The client enforces the node's `max` before it sends
/// anything (10 §5); this is the same number again on the side that writes
/// the disk, because a client is not a thing to be trusted about sizes.
const UPLOAD_CEILING: u64 = eui_proto::limits::DEFAULT_UPLOAD_BYTES;

/// Where a session's uploads land: under the application, so the view can
/// name what it was given the way it names everything else, and under one
/// directory per session so what the session leaves behind can go with it.
fn upload_dir(session_id: &str) -> PathBuf {
    crate::live::component::get_app_root()
        .join("tmp/eui-uploads")
        .join(short_session(session_id))
}

/// A session id reduced to something that is safe as a directory name and
/// still tells two sessions apart.
fn short_session(session_id: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(session_id.as_bytes())[..8]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A client-supplied filename reduced to a leaf this server is willing to
/// create. The client already sends a basename (03 §3.2), and this is the
/// second time that is checked, on the side that does the writing: no
/// separators, no `..`, no dot-file, no empty name, and bounded.
fn safe_leaf(name: &str) -> String {
    // The last segment first, and both separators, because the name comes
    // from the person's machine and not from this one. Mapping a `/` to a
    // dash instead would keep the whole path in the filename — harmless,
    // since there is no separator left to walk, but it would name a file
    // `-..-etc-passwd` and leave the next reader wondering.
    let name = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let leaf: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '-'
            }
        })
        .take(96)
        .collect();
    let leaf = leaf.trim_matches('.').to_string();
    if leaf.is_empty() {
        "file".to_string()
    } else {
        leaf
    }
}

/// A client string on its way into the log: control bytes out, length
/// bounded — a client must not get to write our log lines.
fn printable(s: &str) -> String {
    s.chars()
        .take(200)
        .map(|c| if c.is_control() { '?' } else { c })
        .collect()
}

/// Upgrade the request and drive the session on a task. `slot` is the
/// socket's admission, released when the task ends.
pub fn upgrade(
    mut req: Request<Incoming>,
    component: String,
    session_id: String,
    lv_event_tx: crossbeam::channel::Sender<LiveViewEventData>,
    slot: WsConnectionSlot,
) -> Result<Response<ResponseBody>, hyper::Error> {
    if !super::is_eui_component(&component) {
        return Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(full(Bytes::from("no such EUI component")))
            .unwrap());
    }
    let (response, websocket) =
        match hyper_tungstenite::upgrade(&mut req, Some(default_websocket_config())) {
            Ok(r) => r,
            Err(e) => {
                return Ok(Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(full(Bytes::from(format!("WebSocket upgrade error: {e}"))))
                    .unwrap());
            }
        };

    tokio::spawn(async move {
        let _slot = slot;
        let stream = match websocket.await {
            Ok(ws) => ws,
            Err(e) => {
                eprintln!("[EUI] handshake error: {e}");
                return;
            }
        };
        let (mut ws_write, mut ws_read) = stream.split();

        // 1. Hello, or nothing. The viewport it carries goes to the
        //    application with `connect`, and again as `viewport` whenever
        //    the client reports a change: a view that wants to be
        //    responsive keeps it in its state.
        //    Why the opening frame was refused is worth saying. "expected
        //    Hello" on its own names the symptom of every skew there can
        //    be, and the commonest one by far — a client built against a
        //    newer EUI than the `eui-proto` this binary was compiled with,
        //    whose `Hello` this decoder cannot read — looks exactly like a
        //    client that said nothing at all.
        let hello = match tokio::time::timeout(HELLO_TIMEOUT, ws_read.next()).await {
            Ok(Some(Ok(Message::Binary(b)))) => match Frame::decode(&b) {
                Ok(Frame::Hello(h)) if h.version >= 1 => Ok(h),
                Ok(Frame::Hello(h)) => Err(format!("the client's Hello names protocol version {}", h.version)),
                Ok(_) => Err("the first frame was not a Hello".to_string()),
                Err(e) => Err(format!(
                    "the opening frame did not decode ({e}); a client built against a newer EUI than this server sends a Hello this one cannot read — rebuild soli against the same protocol"
                )),
            },
            Ok(Some(Ok(Message::Text(_)))) => Err("the client sent a text frame; an EUI session is binary only".to_string()),
            Ok(Some(Ok(_))) => Err("the first frame was not binary".to_string()),
            Ok(Some(Err(e))) => Err(format!("the socket failed before Hello: {e}")),
            Ok(None) => Err("the socket closed before Hello".to_string()),
            Err(_) => Err(format!("nothing arrived within {HELLO_TIMEOUT:?} of the socket opening")),
        };
        let hello = match hello {
            Ok(h) => h,
            Err(why) => {
                eprintln!("[EUI] refusing the session: {}", printable(&why));
                let _ = ws_write
                    .send(Message::Binary(
                        Frame::Error {
                            code: 1,
                            message: format!("expected Hello: {why}"),
                        }
                        .encode(),
                    ))
                    .await;
                return;
            }
        };
        // The session the Welcome names is a handle, not the cookie: the
        // LiveView socket learned not to hand the raw session id to the
        // client (`live::socket::session_handle`), and this frame used to
        // carry its first sixteen bytes. A synthetic `sess-*` id names no
        // session and goes as it is.
        let session_bytes = welcome_session(&session_id);
        if ws_write
            .send(Message::Binary(
                Frame::Welcome(Welcome {
                    version: PROTOCOL_VERSION,
                    session: session_bytes,
                    // EUI 01 §4.1: a session that survives its socket. A
                    // LiveView session is torn down with the socket under
                    // it, so there is nothing here to pick up again and
                    // the client is told to start clean. A socket that
                    // breaks does now bring the window back by itself —
                    // on a fresh session and a fresh mount, which is the
                    // half of it that needs nothing from this side.
                    resumed: false,
                })
                .encode(),
            ))
            .await
            .is_err()
        {
            return;
        }

        // 2. The instance, shared with LiveView's registry.
        let (tx, rx) = async_channel::bounded::<Result<Message, tungstenite::Error>>(32);
        let sender: WsSender = Arc::new(tx);
        let mut instance = LiveViewInstance::new(
            component.clone(),
            PathBuf::new(),
            serde_json::json!({}),
            session_id.clone(),
            sender.clone(),
        );
        instance.id = liveview_instance_id(&session_id, &component, None);
        let liveview_id = instance.id.clone();
        let already = LIVE_REGISTRY
            .attach_or_register(instance, sender.clone())
            .is_some();

        // 3. First render: `connect` for a fresh instance, a whole-tree resend
        //    for a reconnect that found its state still there.
        let first = if already { RESYNC_EVENT } else { "connect" };
        let first_params = if already {
            serde_json::json!({})
        } else {
            serde_json::json!({"viewport": viewport_json(&hello.viewport)})
        };
        // A `connect` that closes never ran for this client: there is no
        // `disconnect` to post on the way out.
        let connected = post(
            &lv_event_tx,
            &sender,
            &liveview_id,
            &component,
            first,
            first_params,
            &session_id,
        )
        .await;

        // 4. Server frames out.
        let write_task = tokio::spawn(async move {
            while let Ok(msg) = rx.recv().await {
                match msg {
                    Ok(m) => {
                        if ws_write.send(m).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            // The queue was closed on us — a worker gave this socket up as
            // one that does not drain (`SEND_PATIENCE`) — or the write
            // failed. Say goodbye properly, so the reader side sees the
            // close and the session is torn down instead of half-open.
            let _ = ws_write.close().await;
        });

        // 5. Client frames in — unless `connect` closed the session, in
        //    which case the Error frame is on its way and this is over.
        //    Frames are charged against the same budget a `/ws/*` socket
        //    has (`SOLI_WS_MAX_MESSAGES_PER_SEC`), and the server pings on
        //    its own clock.
        let mut budget = WsRateLimiter::from_config();
        let mut heartbeat = tokio::time::interval(HEARTBEAT);
        heartbeat.tick().await; // the first tick is immediate
        let mut unanswered_pings: u32 = 0;
        // Uploads this session has announced and not yet finished. An
        // `Upload` for an id that is not in here is refused: the spec says a
        // server MUST NOT accept one it has not seen announced (01 §6), and
        // that is the whole of what keeps a socket from writing files
        // nobody offered it.
        let mut uploads: std::collections::HashMap<u32, Upload> = std::collections::HashMap::new();
        loop {
            if !connected {
                break;
            }
            let msg = tokio::select! {
                next = ws_read.next() => match next {
                    Some(Ok(msg)) => msg,
                    _ => break,
                },
                _ = heartbeat.tick() => {
                    if unanswered_pings >= MAX_UNANSWERED_PINGS {
                        eprintln!(
                            "[EUI] {component}: {unanswered_pings} pings unanswered; closing"
                        );
                        break;
                    }
                    unanswered_pings += 1;
                    let nonce = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |d| d.as_nanos() as u64)
                        .to_le_bytes();
                    let _ = sender.try_send(Ok(Message::Binary(Frame::Ping(nonce).encode())));
                    continue;
                }
            };
            if !budget.allow() {
                let _ = sender.try_send(Ok(Message::Binary(
                    Frame::Error {
                        code: 429,
                        message: "too many frames".into(),
                    }
                    .encode(),
                )));
                break;
            }
            let bytes = match msg {
                Message::Binary(b) => b,
                Message::Close(_) => break,
                Message::Text(_) => {
                    let _ = sender.try_send(Ok(Message::Binary(
                        Frame::Error {
                            code: 2,
                            message: "binary frames only".into(),
                        }
                        .encode(),
                    )));
                    break;
                }
                _ => continue,
            };
            let frame = match Frame::decode(&bytes) {
                Ok(f) => f,
                Err(e) => {
                    let _ = sender.try_send(Ok(Message::Binary(
                        Frame::Error {
                            code: e.code(),
                            message: e.to_string(),
                        }
                        .encode(),
                    )));
                    break;
                }
            };
            match frame {
                Frame::Event(e) => {
                    if trace() {
                        eprintln!(
                            "[EUI trace] event node={} kind={:?} name={}",
                            e.node, e.event, e.name
                        );
                    }
                    // The encoder lock is a plain mutex a worker may be
                    // holding through a whole render of this session — a
                    // tick, a second tab — so it is not taken on the
                    // reactor thread.
                    let checked = tokio::task::block_in_place(|| validate(&liveview_id, &e));
                    let Some((name, params)) = checked else {
                        // The node carries no handler for this event *now*.
                        // Usually that is a race and not an attack: a
                        // handler that a render removed is still in the
                        // client's tree for the one round trip it takes the
                        // new tree to arrive, and anything the pointer does
                        // in that window arrives naming it. A split whose
                        // drag has just ended is the everyday case -- the
                        // release takes `pointer_move` off the container,
                        // and a hand still moving sends one more.
                        //
                        // Dropping it is the whole of the defence: nothing
                        // is looked up, nothing is run, no handler is
                        // reachable that the tree does not offer. Ending the
                        // session instead cost the person their application
                        // for a mouse movement.
                        if trace() {
                            eprintln!(
                                "[EUI trace] event ignored: node={} kind={:?} is not in the tree we last sent",
                                e.node, e.event
                            );
                        }
                        continue;
                    };
                    // A `file_pick` is the announcement the chunks that
                    // follow are checked against (01 §6). The file is
                    // opened now, before the application has said anything
                    // about it, because the client is already streaming:
                    // there is no round trip between the pick and the
                    // first chunk in which to ask permission.
                    if e.event == eui_proto::EventKind::FilePick {
                        if let Err(why) = announce(&mut uploads, &params, &session_id, &component) {
                            eprintln!("[EUI] {component} file_pick: {}", printable(&why));
                        }
                    }
                    if !post(
                        &lv_event_tx,
                        &sender,
                        &liveview_id,
                        &component,
                        &name,
                        params,
                        &session_id,
                    )
                    .await
                    {
                        break;
                    }
                }
                Frame::Resync => {
                    if !budget.allow_cost(RESYNC_COST - 1.0) {
                        let _ = sender.try_send(Ok(Message::Binary(
                            Frame::Error {
                                code: 429,
                                message: "too many resyncs".into(),
                            }
                            .encode(),
                        )));
                        break;
                    }
                    if !post(
                        &lv_event_tx,
                        &sender,
                        &liveview_id,
                        &component,
                        RESYNC_EVENT,
                        serde_json::json!({}),
                        &session_id,
                    )
                    .await
                    {
                        break;
                    }
                }
                Frame::Ping(n) => {
                    let _ = sender.try_send(Ok(Message::Binary(Frame::Pong(n).encode())));
                }
                Frame::Viewport(v) => {
                    if !post(
                        &lv_event_tx,
                        &sender,
                        &liveview_id,
                        &component,
                        "viewport",
                        serde_json::json!({"viewport": viewport_json(&v)}),
                        &session_id,
                    )
                    .await
                    {
                        break;
                    }
                }
                Frame::Ack { .. } => {}
                Frame::Pong(_) => unanswered_pings = 0,
                Frame::Error { code, message } => {
                    eprintln!("[EUI] client error {code}: {}", printable(&message));
                    break;
                }
                Frame::Hello(_) | Frame::Welcome(_) | Frame::Batch(_) | Frame::Blob(_) => {
                    let _ = sender.try_send(Ok(Message::Binary(
                        Frame::Error {
                            code: 301,
                            message: "server-only frame".into(),
                        }
                        .encode(),
                    )));
                    break;
                }
                // EUI 01 §6. The chunks of a file the person picked, each
                // written as it arrives — the budget for an upload is two
                // chunks however big the file is (10 §5), so nothing is
                // held. What ends the session is what the spec says ends
                // it: an id nobody announced, or a gap in `seq`.
                Frame::Upload(t) => {
                    match take_chunk(&mut uploads, t) {
                        Chunk::More => {}
                        Chunk::Refused { code, why } => {
                            eprintln!("[EUI] {component}: refusing an upload: {}", printable(&why));
                            let _ = sender.try_send(Ok(Message::Binary(
                                Frame::Error { code, message: why }.encode(),
                            )));
                            break;
                        }
                        // Whole or abandoned, the application hears one
                        // event either way, because a file that never
                        // arrives is a thing a view has to be able to say
                        // out loud instead of leaving a spinner turning.
                        Chunk::Done(params) => {
                            if !post(
                                &lv_event_tx,
                                &sender,
                                &liveview_id,
                                &component,
                                "file_upload",
                                params,
                                &session_id,
                            )
                            .await
                            {
                                break;
                            }
                        }
                    }
                }
            }
        }

        // A transfer belongs to its session and does not survive it
        // (01 §6). What the application wanted to keep it has already
        // copied somewhere it owns; the rest goes with the socket.
        uploads.clear();
        let spool = upload_dir(&session_id);
        if spool.exists() {
            let _ = std::fs::remove_dir_all(&spool);
        }

        // The window is gone. The application hears about it before the
        // session is torn down, because what it started on the way in it
        // may have to stop on the way out — a player it spawned, a
        // device it borrowed. The post is awaited like any other event,
        // so the handler runs to its end; a component with no
        // `disconnect` branch simply returns its state unchanged.
        if connected {
            post(
                &lv_event_tx,
                &sender,
                &liveview_id,
                &component,
                "disconnect",
                serde_json::json!({}),
                &session_id,
            )
            .await;
        }

        // Detached, not dropped: the state stays for `DETACHED_GRACE` so a
        // reconnect can resync, then the reaper takes it. Without the detach
        // an instance lived to the hour-long idle timeout — and only if a
        // LiveView socket had ever started the reaper at all.
        if LIVE_REGISTRY.drop_sender(&liveview_id, &sender) {
            LIVE_REGISTRY.detach(&liveview_id);
        }
        // Let what is queued — an Error frame saying why — reach the client
        // before the socket goes; a client that has stopped reading gets
        // two seconds, then the writer is dropped.
        sender.close();
        let mut write_task = write_task;
        if tokio::time::timeout(std::time::Duration::from_secs(2), &mut write_task)
            .await
            .is_err()
        {
            write_task.abort();
        }
        super::drop_encoder(&liveview_id);
        // The memo lives on the worker this session was pinned to; ask it.
        // Fire and forget: a saturated queue means the worker's own sweep
        // finds the encoder gone and drops the memo on its next render.
        let (response_tx, _) = oneshot::channel();
        if let Some(tx) = super::super::lv_sender_for(&liveview_id, &component) {
            let _ = tx.try_send(LiveViewEventData {
                liveview_id: liveview_id.clone(),
                component: component.clone(),
                event: super::FORGET_EVENT.to_string(),
                params: serde_json::json!({}),
                sender_session: None,
                response_tx,
            });
        }
    });

    Ok(box_full(response))
}

/// The sixteen bytes a Welcome names the session by: a SHA-256 of the
/// session id for a real one, so the value the cookie's `HttpOnly` flag
/// keeps from page scripts is not on the socket; the synthetic `sess-*`
/// id of a cookie-less socket as it is, since it names nothing.
fn welcome_session(session_id: &str) -> [u8; 16] {
    let mut out = [0u8; 16];
    if session_id.starts_with("sess-") {
        let raw = session_id.as_bytes();
        let n = raw.len().min(16);
        out[..n].copy_from_slice(&raw[..n]);
    } else {
        use sha2::{Digest, Sha256};
        out.copy_from_slice(&Sha256::digest(session_id.as_bytes())[..16]);
    }
    out
}

/// What one `Upload` frame amounted to.
enum Chunk {
    /// Written; more are owed.
    More,
    /// The transfer ended — whole, or abandoned — and this is the
    /// `file_upload` the application is owed.
    Done(serde_json::Value),
    /// The frame was one this session may not take, and the session ends.
    Refused { code: u32, why: String },
}

/// Open the file a `file_pick` announced, so the chunks that follow have
/// somewhere to go. The payload is `[id, name, size]` (06 §1).
fn announce(
    uploads: &mut std::collections::HashMap<u32, Upload>,
    params: &serde_json::Value,
    session_id: &str,
    component: &str,
) -> Result<(), String> {
    let payload = params
        .get("payload")
        .and_then(serde_json::Value::as_array)
        .ok_or("a file_pick carries [id, name, size]")?;
    let id = payload
        .first()
        .and_then(serde_json::Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .ok_or("a file_pick names an upload id")?;
    let name = payload
        .get(1)
        .and_then(serde_json::Value::as_str)
        .unwrap_or("file");
    let size = payload
        .get(2)
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    if size > UPLOAD_CEILING {
        return Err(format!(
            "{name} is {size} bytes; this server takes {UPLOAD_CEILING}"
        ));
    }
    // One directory per session, and the id in the leaf: two files of the
    // same name picked in one session are two files, and neither is the
    // other's overwrite.
    let dir = upload_dir(session_id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let leaf = format!("{id}-{}", safe_leaf(name));
    let path = dir.join(&leaf);
    let file = std::fs::File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let rel = format!("tmp/eui-uploads/{}/{leaf}", short_session(session_id));
    if trace() {
        eprintln!("[EUI trace] {component} file_pick: {name} ({size} B) -> {rel}");
    }
    uploads.insert(
        id,
        Upload {
            name: name.to_string(),
            size,
            next_seq: 0,
            written: 0,
            rel,
            path,
            file,
        },
    );
    Ok(())
}

/// One `Upload` frame against the transfer it names.
fn take_chunk(
    uploads: &mut std::collections::HashMap<u32, Upload>,
    t: eui_proto::Transfer,
) -> Chunk {
    use std::io::Write;
    let Some(up) = uploads.get_mut(&t.id) else {
        // Either a client inventing ids, or one still streaming a file this
        // server already gave up on. Both are the session's end: the spec
        // allows nothing to be written for an id that was not announced.
        return Chunk::Refused {
            code: 302,
            why: format!("upload {} was never announced", t.id),
        };
    };
    if t.flag == eui_proto::Chunked::Abort {
        let why = String::from_utf8_lossy(&t.bytes)
            .chars()
            .take(256)
            .collect::<String>();
        let done = finish(uploads, t.id, Some(&why));
        return Chunk::Done(done);
    }
    if t.seq != up.next_seq {
        return Chunk::Refused {
            code: 302,
            why: format!(
                "upload {} jumped from chunk {} to {}",
                t.id, up.next_seq, t.seq
            ),
        };
    }
    if t.bytes.len() > eui_proto::limits::MAX_TRANSFER_CHUNK_BYTES {
        return Chunk::Refused {
            code: 302,
            why: format!("upload {} sent a {} byte chunk", t.id, t.bytes.len()),
        };
    }
    let landing = up.written.saturating_add(t.bytes.len() as u64);
    // The announced size is the client's word; this is the fact. A file
    // that outgrows what it said it was is abandoned rather than refused,
    // because the person did pick something and deserves to be told.
    if landing > UPLOAD_CEILING || landing > up.size.max(1) {
        let why = format!("it is longer than the {} bytes it announced", up.size);
        return Chunk::Done(finish(uploads, t.id, Some(&why)));
    }
    if let Err(e) = up.file.write_all(&t.bytes) {
        let why = format!("{e}");
        return Chunk::Done(finish(uploads, t.id, Some(&why)));
    }
    up.written = landing;
    up.next_seq = up.next_seq.saturating_add(1);
    if t.flag == eui_proto::Chunked::Last {
        return Chunk::Done(finish(uploads, t.id, None));
    }
    Chunk::More
}

/// Close a transfer and say what the application gets. `why` is the reason
/// it did not finish, and what a view shows instead of the file.
///
/// The answer is shaped like every other event's: the facts go under
/// `payload`, beside the `kind` that names them. A server-posted event that
/// put its fields at the top level instead would make a handler need to know
/// *which* events came from the server — and the one that did read
/// `params["payload"]` like all the others and found nothing, so every
/// attachment arrived without a file.
fn finish(
    uploads: &mut std::collections::HashMap<u32, Upload>,
    id: u32,
    why: Option<&str>,
) -> serde_json::Value {
    use std::io::Write;
    let Some(mut up) = uploads.remove(&id) else {
        return serde_json::json!({"upload": id, "error": "no such upload"});
    };
    let _ = up.file.flush();
    drop(up.file);
    if let Some(why) = why {
        // 01 §6: everything already received is discarded. A half-written
        // file left on disk is a file some view would eventually show.
        let _ = std::fs::remove_file(&up.path);
        return serde_json::json!({
            "kind": "file_upload",
            "payload": {
                "upload": id,
                "name": up.name,
                "size": up.size,
                "path": "",
                "error": why,
            },
        });
    }
    // The path is relative to the application root, which is what `image`,
    // `File` and the rest of a view's vocabulary already speak. It lives in
    // the session's own directory, so a view that wants to keep the file
    // has to copy it somewhere it owns — a file is not an asset (01 §6),
    // and promoting one is the application's decision, not this server's.
    serde_json::json!({
        "kind": "file_upload",
        "payload": {
            "upload": id,
            "name": up.name,
            "size": up.written,
            "path": up.rel,
            "error": "",
        },
    })
}

/// Check an event against the tree the client was last sent; on success,
/// the server-side event name and the params the handler receives.
fn validate(liveview_id: &str, e: &EventFrame) -> Option<(String, serde_json::Value)> {
    with_encoder(liveview_id, |enc| {
        let (name, props) = enc.event_target(e.node, e.event)?;
        let params = serde_json::json!({
            "node": e.node,
            "kind": tree::event_name(e.event),
            "payload": tree::wire_to_json(enc, &e.payload),
            "props": props,
        });
        Some((name, params))
    })
}

/// Post an event to the session's worker and wait for it to be handled, so
/// that events from one socket are applied in order. `false` when the
/// handler closed the session: the socket has been told why and stops.
///
/// The worker is the one the session is pinned to (`lv_sender_for`): every
/// frame of a session renders on the same thread, where the memo and the
/// application's own kept objects are. The shared queue is the fallback for
/// a process that has no pinned queues, such as a test.
async fn post(
    lv_event_tx: &crossbeam::channel::Sender<LiveViewEventData>,
    sender: &WsSender,
    liveview_id: &str,
    component: &str,
    event: &str,
    params: serde_json::Value,
    session_id: &str,
) -> bool {
    let (response_tx, response_rx) = oneshot::channel();
    let data = LiveViewEventData {
        liveview_id: liveview_id.to_string(),
        component: component.to_string(),
        event: event.to_string(),
        params,
        sender_session: Some(session_id.to_string()),
        response_tx,
    };
    let tx =
        super::super::lv_sender_for(liveview_id, component).unwrap_or_else(|| lv_event_tx.clone());
    // An Error frame ends the session on both sides (spec 01 §4), so it
    // is sent only when the session really is over: the server could not
    // take the event, or cannot say what became of it. A handler that
    // raised is neither — the state is unchanged, the screen still right,
    // and the next click works — so that stays a log line.
    let fail = |code: u32, message: &str| {
        let _ = sender.try_send(Ok(Message::Binary(
            Frame::Error {
                code,
                message: message.into(),
            }
            .encode(),
        )));
        false
    };
    if tx.try_send(data).is_err() {
        eprintln!("[EUI] {component} {event}: worker pool saturated; closing the session");
        return fail(503, "server busy");
    }
    match tokio::time::timeout(HANDLER_TIMEOUT, response_rx).await {
        Ok(Ok(Ok(()))) => {
            if trace() {
                eprintln!("[EUI trace] {component} {event}: handled");
            }
        }
        Ok(Ok(Err(e))) => {
            if let Some(reason) = super::close_reason(&e) {
                eprintln!("[EUI] {component} {event}: closed by the handler: {reason}");
                return false;
            }
            eprintln!("[EUI] {component} {event}: {e}");
        }
        Ok(Err(_)) => {
            eprintln!("[EUI] {component} {event}: the worker went away; closing the session");
            return fail(500, "the handler did not answer");
        }
        Err(_) => {
            eprintln!(
                "[EUI] {component} {event}: no answer in {}s; closing the session",
                HANDLER_TIMEOUT.as_secs()
            );
            return fail(504, "the handler took too long");
        }
    }
    true
}

/// The client's viewport as the application sees it, in `params`.
fn viewport_json(v: &eui_proto::Viewport) -> serde_json::Value {
    serde_json::json!({
        "width": v.width,
        "height": v.height,
        "scale": f64::from(v.scale) / 100.0,
        "mode": match v.mode { eui_proto::ThemeMode::Light => "light", eui_proto::ThemeMode::Dark => "dark", eui_proto::ThemeMode::HighContrast => "high_contrast" },
        "density": match v.density { eui_proto::Density::Compact => "compact", eui_proto::Density::Cozy => "cozy", eui_proto::Density::Comfortable => "comfortable" },
        "font_scale": f64::from(v.font_scale) / 100.0
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_welcome_never_carries_the_cookie() {
        let cookie = "3f2b7c1e-9a4d-4e8f-b6c0-1d2e3f4a5b6c";
        let bytes = welcome_session(cookie);
        assert!(!cookie.as_bytes().starts_with(&bytes));
        assert_eq!(bytes, welcome_session(cookie), "stable per session");
        assert_ne!(bytes, welcome_session("another-session"));
        // A synthetic id names no session and is passed through.
        assert!(welcome_session("sess-abc").starts_with(b"sess-abc"));
    }

    #[test]
    fn a_filename_from_a_client_is_reduced_to_a_leaf() {
        assert_eq!(safe_leaf("notes.txt"), "notes.txt");
        assert_eq!(safe_leaf("../../etc/passwd"), "passwd");
        assert_eq!(safe_leaf("/absolute"), "absolute");
        assert_eq!(safe_leaf(r"C:\Users\x\report.pdf"), "report.pdf");
        assert_eq!(safe_leaf(".."), "file");
        assert_eq!(safe_leaf(""), "file");
        assert_eq!(safe_leaf(".hidden"), "hidden");
        assert_eq!(safe_leaf("a").len(), 1);
        assert!(safe_leaf(&"x".repeat(500)).len() <= 96);
    }

    /// A transfer built by hand, so the state machine can be exercised
    /// without a socket.
    fn spool(dir: &std::path::Path, id: u32, size: u64) -> std::collections::HashMap<u32, Upload> {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(format!("{id}-f"));
        let file = std::fs::File::create(&path).unwrap();
        let mut m = std::collections::HashMap::new();
        m.insert(
            id,
            Upload {
                name: "f".into(),
                size,
                next_seq: 0,
                written: 0,
                rel: format!("tmp/eui-uploads/t/{id}-f"),
                path,
                file,
            },
        );
        m
    }

    fn chunk(id: u32, seq: u32, flag: eui_proto::Chunked, bytes: &[u8]) -> eui_proto::Transfer {
        eui_proto::Transfer {
            id,
            seq,
            flag,
            bytes: bytes.to_vec(),
        }
    }

    #[test]
    fn a_whole_transfer_lands_and_says_where() {
        let dir = std::env::temp_dir().join("eui-upload-test-whole");
        let _ = std::fs::remove_dir_all(&dir);
        let mut ups = spool(&dir, 7, 6);
        assert!(matches!(
            take_chunk(&mut ups, chunk(7, 0, eui_proto::Chunked::More, b"abc")),
            Chunk::More
        ));
        let Chunk::Done(params) =
            take_chunk(&mut ups, chunk(7, 1, eui_proto::Chunked::Last, b"def"))
        else {
            panic!("the last chunk did not finish the transfer");
        };
        // Shaped like every other event: the facts under `payload`, so a
        // handler reads this one exactly as it reads a click. They used to
        // sit at the top level, and the one handler that read
        // `params["payload"]` like all the others found nothing — so every
        // attachment arrived without a file.
        assert_eq!(params["kind"], "file_upload");
        assert_eq!(params["payload"]["error"], "");
        assert_eq!(params["payload"]["size"], 6);
        assert!(
            params["payload"]["path"]
                .as_str()
                .is_some_and(|p| p.contains("7-f")),
            "a finished upload says where it landed: {params}"
        );
        assert_eq!(std::fs::read(dir.join("7-f")).unwrap(), b"abcdef");
        // The transfer is over; a further chunk names an id nobody knows.
        assert!(matches!(
            take_chunk(&mut ups, chunk(7, 2, eui_proto::Chunked::Last, b"g")),
            Chunk::Refused { code: 302, .. }
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_gap_in_the_sequence_ends_the_session_and_an_abort_only_ends_the_file() {
        let dir = std::env::temp_dir().join("eui-upload-test-gap");
        let _ = std::fs::remove_dir_all(&dir);

        let mut ups = spool(&dir, 1, 100);
        assert!(matches!(
            take_chunk(&mut ups, chunk(1, 3, eui_proto::Chunked::More, b"x")),
            Chunk::Refused { code: 302, .. }
        ));

        // An abort is the person's file going away, not the socket's.
        let mut ups = spool(&dir, 2, 100);
        take_chunk(&mut ups, chunk(2, 0, eui_proto::Chunked::More, b"partial"));
        let Chunk::Done(params) = take_chunk(
            &mut ups,
            chunk(2, 1, eui_proto::Chunked::Abort, b"the disk went away"),
        ) else {
            panic!("an abort must still tell the application");
        };
        assert_eq!(params["payload"]["error"], "the disk went away");
        assert_eq!(
            params["payload"]["path"], "",
            "an abandoned file has no path"
        );
        // 01 §6: what arrived is discarded.
        assert!(!dir.join("2-f").exists());

        // An id that was never announced is refused outright.
        let mut ups = spool(&dir, 3, 100);
        assert!(matches!(
            take_chunk(&mut ups, chunk(99, 0, eui_proto::Chunked::Last, b"x")),
            Chunk::Refused { code: 302, .. }
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_longer_than_it_announced_is_abandoned_not_written() {
        let dir = std::env::temp_dir().join("eui-upload-test-long");
        let _ = std::fs::remove_dir_all(&dir);
        let mut ups = spool(&dir, 4, 4);
        let Chunk::Done(params) = take_chunk(
            &mut ups,
            chunk(4, 0, eui_proto::Chunked::Last, b"far too much"),
        ) else {
            panic!("an overlong file must end its transfer");
        };
        assert!(
            params["payload"]["error"]
                .as_str()
                .unwrap()
                .contains("announced"),
            "{params}"
        );
        assert!(!dir.join("4-f").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
