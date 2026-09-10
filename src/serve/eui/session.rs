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
        let hello = match tokio::time::timeout(HELLO_TIMEOUT, ws_read.next()).await {
            Ok(Some(Ok(Message::Binary(b)))) => match Frame::decode(&b) {
                Ok(Frame::Hello(h)) if h.version >= 1 => Some(h),
                _ => None,
            },
            _ => None,
        };
        let Some(hello) = hello else {
            let _ = ws_write
                .send(Message::Binary(
                    Frame::Error {
                        code: 1,
                        message: "expected Hello".into(),
                    }
                    .encode(),
                ))
                .await;
            return;
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
                // EUI 01 §6. A file only ever leaves a client for a node
                // that declared `pick`, and nothing here can declare one
                // yet (`tree::event_kind` takes neither file event), so an
                // upload on this session did not come from a tree this
                // server sent.
                Frame::Upload(_) => {
                    let _ = sender.try_send(Ok(Message::Binary(
                        Frame::Error {
                            code: 302,
                            message: "this server takes no uploads".into(),
                        }
                        .encode(),
                    )));
                    break;
                }
            }
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
}
