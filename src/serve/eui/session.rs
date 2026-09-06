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
use super::{tree, with_encoder, RESYNC_EVENT};

type WsSender = Arc<async_channel::Sender<Result<Message, tungstenite::Error>>>;

/// Upgrade the request and drive the session on a task.
pub fn upgrade(
    mut req: Request<Incoming>,
    component: String,
    session_id: String,
    lv_event_tx: crossbeam::channel::Sender<LiveViewEventData>,
) -> Result<Response<ResponseBody>, hyper::Error> {
    if !super::is_eui_component(&component) {
        return Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(full(Bytes::from("no such EUI component")))
            .unwrap());
    }
    let (response, websocket) = match hyper_tungstenite::upgrade(&mut req, Some(default_websocket_config())) {
        Ok(r) => r,
        Err(e) => {
            return Ok(Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(full(Bytes::from(format!("WebSocket upgrade error: {e}"))))
                .unwrap());
        }
    };

    tokio::spawn(async move {
        let stream = match websocket.await {
            Ok(ws) => ws,
            Err(e) => {
                eprintln!("[EUI] handshake error: {e}");
                return;
            }
        };
        let (mut ws_write, mut ws_read) = stream.split();

        // 1. Hello, or nothing.
        let hello_ok = match ws_read.next().await {
            Some(Ok(Message::Binary(b))) => matches!(Frame::decode(&b), Ok(Frame::Hello(h)) if h.version >= 1),
            _ => false,
        };
        if !hello_ok {
            let _ = ws_write.send(Message::Binary(Frame::Error { code: 1, message: "expected Hello".into() }.encode())).await;
            return;
        }
        let mut session_bytes = [0u8; 16];
        let raw = session_id.as_bytes();
        session_bytes[..raw.len().min(16)].copy_from_slice(&raw[..raw.len().min(16)]);
        if ws_write
            .send(Message::Binary(Frame::Welcome(Welcome { version: PROTOCOL_VERSION, session: session_bytes }).encode()))
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
        let already = LIVE_REGISTRY.attach_or_register(instance, sender.clone()).is_some();

        // 3. First render: `connect` for a fresh instance, a whole-tree resend
        //    for a reconnect that found its state still there.
        let first = if already { RESYNC_EVENT } else { "connect" };
        post(&lv_event_tx, &liveview_id, &component, first, serde_json::json!({}), &session_id).await;

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
        });

        // 5. Client frames in.
        while let Some(Ok(msg)) = ws_read.next().await {
            let bytes = match msg {
                Message::Binary(b) => b,
                Message::Close(_) => break,
                Message::Text(_) => {
                    let _ = sender.try_send(Ok(Message::Binary(Frame::Error { code: 2, message: "binary frames only".into() }.encode())));
                    break;
                }
                _ => continue,
            };
            let frame = match Frame::decode(&bytes) {
                Ok(f) => f,
                Err(e) => {
                    let _ = sender.try_send(Ok(Message::Binary(Frame::Error { code: e.code(), message: e.to_string() }.encode())));
                    break;
                }
            };
            match frame {
                Frame::Event(e) => {
                    if std::env::var("EUI_TRACE").is_ok() {
                        eprintln!("[EUI trace] event node={} kind={:?} name={}", e.node, e.event, e.name);
                    }
                    let Some((name, params)) = validate(&liveview_id, &e) else {
                        let _ = sender.try_send(Ok(Message::Binary(
                            Frame::Error { code: 300, message: "event does not match the tree".into() }.encode(),
                        )));
                        break;
                    };
                    post(&lv_event_tx, &liveview_id, &component, &name, params, &session_id).await;
                }
                Frame::Resync => {
                    post(&lv_event_tx, &liveview_id, &component, RESYNC_EVENT, serde_json::json!({}), &session_id).await;
                }
                Frame::Ping(n) => {
                    let _ = sender.try_send(Ok(Message::Binary(Frame::Pong(n).encode())));
                }
                Frame::Ack { .. } | Frame::Pong(_) | Frame::Viewport(_) => {}
                Frame::Error { code, message } => {
                    eprintln!("[EUI] client error {code}: {message}");
                    break;
                }
                Frame::Hello(_) | Frame::Welcome(_) | Frame::Batch(_) => {
                    let _ = sender.try_send(Ok(Message::Binary(Frame::Error { code: 301, message: "server-only frame".into() }.encode())));
                    break;
                }
            }
        }

        LIVE_REGISTRY.drop_sender(&liveview_id, &sender);
        write_task.abort();
        super::drop_encoder(&liveview_id);
    });

    Ok(box_full(response))
}

/// Check an event against the tree the client was last sent; on success,
/// the server-side event name and the params the handler receives.
fn validate(liveview_id: &str, e: &EventFrame) -> Option<(String, serde_json::Value)> {
    with_encoder(liveview_id, |enc| {
        let name = enc.handler_name(e.node, e.event)?;
        let params = serde_json::json!({
            "node": e.node,
            "kind": tree::event_name(e.event),
            "payload": tree::wire_to_json(enc, &e.payload),
            "props": enc.props_of(e.node),
        });
        Some((name, params))
    })
}

/// Post an event to the worker pool and wait for it to be handled, so that
/// events from one socket are applied in order.
async fn post(
    lv_event_tx: &crossbeam::channel::Sender<LiveViewEventData>,
    liveview_id: &str,
    component: &str,
    event: &str,
    params: serde_json::Value,
    session_id: &str,
) {
    let (response_tx, response_rx) = oneshot::channel();
    let data = LiveViewEventData {
        liveview_id: liveview_id.to_string(),
        component: component.to_string(),
        event: event.to_string(),
        params,
        sender_session: Some(session_id.to_string()),
        response_tx,
    };
    if lv_event_tx.try_send(data).is_err() {
        eprintln!("[EUI] worker pool saturated; dropping event {event}");
        return;
    }
    match tokio::time::timeout(std::time::Duration::from_secs(30), response_rx).await {
        Ok(Ok(Ok(()))) => {
            if std::env::var("EUI_TRACE").is_ok() {
                eprintln!("[EUI trace] {component} {event}: handled");
            }
        }
        Ok(Ok(Err(e))) => eprintln!("[EUI] {component} {event}: {e}"),
        Ok(Err(_)) => eprintln!("[EUI] {component} {event}: worker dropped the response"),
        Err(_) => eprintln!("[EUI] {component} {event}: timed out"),
    }
}
