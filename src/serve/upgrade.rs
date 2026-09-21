//! The four sockets this server will upgrade a request to, and the helpers
//! that admit them: live reload, an EUI session, a LiveView, and the
//! application's own `websocket_routes`.
//!
//! Lifted out of `handle_hyper_request` whole. It is a move and not a merge
//! for a security reason worth stating twice: the same-origin gate in
//! `handle_hyper_request` tests `!is_upgrade_request(&req)` *precisely so this
//! branch runs ungated*, and each of the four sub-branches compensates with
//! its own `websocket_origin_allowed`. Those checks travel with their
//! branches; nothing here has been folded together, and the gate that lets
//! them through is not touched.
//!
//! `websocket_origin_allowed` itself stays in `serve::mod`: the `/__livereload`
//! SSE endpoint calls it too, and it is not a property of upgrading.
//!
//! Not here: `handle_websocket_event`, which takes `&mut Interpreter` and runs
//! on a *worker* thread. Dispatching an event is a different concern from
//! accepting the socket it arrives on.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use crossbeam::channel;
use futures_util::{SinkExt, StreamExt};
use hyper::body::Incoming;
use hyper::{Request, Response, StatusCode};
use tokio::sync::{broadcast, oneshot};

use crate::interpreter::builtins::session::{parse_cookie_pairs, session_id_from_cookie_pairs};
use crate::live::socket::{
    extract_session_id as extract_live_session_id, handle_live_connection, liveview_instance_id,
    sanitize_mount_id,
};

use super::tenant::TenantCell;
use super::{
    box_full, full, header_str, server_constants, LiveViewEventData, ResponseBody,
    WebSocketConnection, WebSocketContext, WebSocketEventData, WebSocketRegistry,
};

/// Every socket the binary knows how to open. Called only for a request that
/// `hyper_tungstenite::is_upgrade_request` already accepted, and every path
/// through it returns — an upgrade request for an unknown path is a 404 here,
/// never a fall-through to route matching.
pub(super) async fn handle(
    mut req: Request<Incoming>,
    path: &str,
    raw_query: Option<&str>,
    peer_addr: SocketAddr,
    reload_tx: Option<&broadcast::Sender<()>>,
    ws_event_tx: &channel::Sender<WebSocketEventData>,
    lv_event_tx: &channel::Sender<LiveViewEventData>,
) -> Result<Response<ResponseBody>, hyper::Error> {
    // Handle live reload WebSocket endpoint
    if path == "/__livereload_ws" {
        if !super::websocket_origin_allowed(req.headers()) {
            return Ok(forbidden_websocket_origin_response());
        }
        if let Some(tx) = reload_tx {
            return super::live_reload_ws::handle_live_reload_websocket(req, tx.subscribe())
                .await
                .map(box_full);
        } else {
            // Live reload disabled
            return Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(full(Bytes::from("Live reload is disabled")))
                .unwrap());
        }
    }

    // EUI session socket: binary frames, one component per path. Same
    // registry and worker channel as LiveView; only the wire differs.
    #[cfg(feature = "eui")]
    if let Some(component) = path.strip_prefix("/_eui/session/") {
        if !super::websocket_origin_allowed(req.headers()) {
            return Ok(forbidden_websocket_origin_response());
        }
        let component = component.trim_end_matches('/').to_string();
        let cookies = req
            .headers()
            .get("cookie")
            .map(|v| v.to_str().unwrap_or(""));
        let session_id = extract_live_session_id(cookies);
        // `router_eui(..., {"session": "required"})`: no session cookie,
        // no socket. A cookie-less upgrade gets a synthetic `sess-*` id
        // and would run `connect` as nobody; a component that declared
        // it needs someone is refused before that.
        if super::eui::session_required(&component) && session_id.starts_with("sess-") {
            return Ok(Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .body(full(Bytes::from("this component needs a session")))
                .unwrap());
        }
        let slot = match admit_websocket(&peer_addr.ip().to_string()) {
            Ok(slot) => slot,
            Err(refused) => return Ok(*refused),
        };
        // Same registry as LiveView, same reaper — an application that
        // only ever serves EUI used to keep every instance it ever made.
        start_liveview_reaper();
        // 01 §2.7: an island is addressed by a path *with a query* —
        // `?for=1042` is how two regions of one component tell the
        // application which of them is being rendered. It reaches
        // `connect` as ordinary params.
        return super::eui::session::upgrade(
            req,
            component,
            raw_query.map(str::to_string),
            session_id,
            lv_event_tx.clone(),
            slot,
        );
    }

    // Handle LiveView WebSocket endpoint
    if path == "/live/socket" || path.starts_with("/live/socket/") {
        if !super::websocket_origin_allowed(req.headers()) {
            return Ok(forbidden_websocket_origin_response());
        }

        // Extract component name from path
        let component = if path == "/live/socket" {
            "counter".to_string()
        } else {
            path.trim_start_matches("/live/socket/")
                .trim_end_matches("/socket")
                .to_string()
        };

        // An EUI component lives on its own binary socket, where every
        // event is checked against the tree the client was last sent
        // before it becomes a handler call. `router_eui` registers the
        // handler in the same registry this socket reads, so the
        // component used to be reachable here too — with a JSON event
        // naming any handler event and carrying whatever `params` and
        // `props` the client cared to write. Not a LiveView; not here.
        #[cfg(feature = "eui")]
        if super::eui::is_eui_component(&component) {
            return Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(full(Bytes::from("no such LiveView component")))
                .unwrap());
        }

        // Extract session ID from cookies
        let cookies = req
            .headers()
            .get("cookie")
            .map(|v| v.to_str().unwrap_or(""));
        let session_id = extract_live_session_id(cookies);
        let room = req
            .uri()
            .query()
            .and_then(|q| {
                q.split('&').find_map(|pair| {
                    let (key, value) = pair.split_once('=')?;
                    (key == "room").then_some(value)
                })
            })
            .and_then(sanitize_mount_id)
            // Rooms are opt-in per component (`live_rooms("desk")` in
            // routes.sl). Undeclared, `?room=` is ignored and the socket
            // gets its own per-session instance, which is what an
            // unshared component always meant.
            .filter(|_| {
                let allowed = crate::live::socket::is_room_component(&component);
                if !allowed {
                    eprintln!(
                        "[LiveView] ignoring ?room= for component {component:?}: not declared \
                         room-shareable (add live_rooms({component:?}) to config/routes.sl)"
                    );
                }
                allowed
            });

        start_liveview_reaper();

        // The same admission every socket gets; the slot lives as long
        // as the connection task below.
        let slot = match admit_websocket(&peer_addr.ip().to_string()) {
            Ok(slot) => slot,
            Err(refused) => return Ok(*refused),
        };

        // Perform the WebSocket upgrade
        let ws_config = default_websocket_config();
        let (response, websocket) = match hyper_tungstenite::upgrade(&mut req, Some(ws_config)) {
            Ok(result) => result,
            Err(e) => {
                eprintln!("[LiveView] Upgrade error: {}", e);
                return Ok(Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(full(Bytes::from(format!("WebSocket upgrade error: {}", e))))
                    .unwrap());
            }
        };

        // The connection itself outlives this request; the handshake
        // completes inside it. See `liveview_connection_loop`.
        super::tenant::spawn(liveview_connection_loop(
            websocket,
            slot,
            component,
            session_id,
            room,
            lv_event_tx.clone(),
        ));

        return Ok(box_full(response));
    }

    // Check if there's a WebSocket route for this path
    let routes = crate::serve::websocket::get_websocket_routes();
    let has_ws_route = routes.iter().any(|r| r.path_pattern == path);

    if has_ws_route {
        // Get the global WebSocket registry
        let ws_registry = crate::serve::websocket::get_ws_registry();
        handle_websocket_upgrade(
            req,
            ws_registry,
            path.to_string(),
            ws_event_tx.clone(),
            peer_addr.ip().to_string(),
        )
        .await
    } else {
        // No WebSocket route found
        Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(full(Bytes::from("WebSocket endpoint not found")))
            .unwrap())
    }
}

/// One LiveView socket, for as long as it is open.
///
/// Spawned by `handle` and outliving the request that made it: the handshake
/// finishes here, then client events are forwarded to a worker and the
/// worker's messages back to the wire.
async fn liveview_connection_loop(
    websocket: hyper_tungstenite::HyperWebsocket,
    // Held for the life of the connection; dropping it frees the admission
    // slot on every exit path below, handshake failure included.
    _slot: crate::serve::websocket::WsConnectionSlot,
    component: String,
    session_id: String,
    room: Option<String>,
    lv_event_tx: channel::Sender<LiveViewEventData>,
) {
    let stream = match websocket.await {
        Ok(ws) => ws,
        Err(e) => {
            eprintln!("[LiveView] WebSocket handshake error: {}", e);
            return;
        }
    };

    // Create async channel for LiveView messages
    let (tx, rx) = async_channel::bounded::<Result<tungstenite::Message, tungstenite::Error>>(32);
    let tx_arc = Arc::new(tx);

    // Initialize the LiveView connection
    let liveview_id = liveview_instance_id(&session_id, &component, room.as_deref());
    // Kept for the event sites below: `handle_live_connection` takes
    // ownership, and a room instance's own session is the creator's,
    // not this socket's.
    let socket_session = session_id.clone();
    handle_live_connection(component.clone(), session_id, tx_arc.clone(), room);

    // Fire a synthetic `connect` event so user handlers can seed
    // initial state and request a tick interval. We fire-and-forget
    // here — the worker still posts a response, but no one needs to
    // await it (the receiver gets dropped).
    if crate::live::socket::get_liveview_handler(&component).is_some() {
        let (response_tx, _response_rx) = oneshot::channel();
        let _ = lv_event_tx.try_send(LiveViewEventData {
            liveview_id: liveview_id.clone(),
            component: component.clone(),
            event: "connect".to_string(),
            params: serde_json::json!({}),
            sender_session: Some(socket_session.clone()),
            response_tx,
        });
    }

    // Split the WebSocket stream
    let (mut ws_write, mut ws_read) = stream.split();

    // Spawn task to forward messages from channel to WebSocket
    let write_task = super::tenant::spawn(async move {
        while let Ok(msg_result) = rx.recv().await {
            match msg_result {
                Ok(msg) => {
                    if ws_write.send(msg).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Handle incoming messages (events from client)
    let liveview_id_owned = liveview_id.clone();
    while let Some(msg_result) = ws_read.next().await {
        match msg_result {
            Ok(msg) => {
                if msg.is_close() {
                    break;
                }
                if msg.is_text() {
                    if let Ok(text) = msg.to_text() {
                        // Parse the event message
                        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(text) {
                            let event_type =
                                parsed.get("type").and_then(|v| v.as_str()).unwrap_or("");
                            let liveview_id = parsed
                                .get("liveview_id")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());
                            let event_name = parsed
                                .get("event")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());
                            let params = parsed
                                .get("params")
                                .cloned()
                                .unwrap_or(serde_json::json!({}));

                            if event_type == "event" {
                                // Dispatch against *this* socket's
                                // instance. A client naming another id
                                // would otherwise drive a component
                                // whose `connect` never ran — or, with a
                                // known session id, another user's view.
                                if !addressed_to_this_socket(
                                    liveview_id.as_deref(),
                                    &liveview_id_owned,
                                ) {
                                    continue;
                                }
                                {
                                    let id = liveview_id_owned.clone();
                                    if let Some(event) = event_name {
                                        // Send event to worker thread for controller dispatch
                                        let (response_tx, response_rx) = oneshot::channel();
                                        let event_data = LiveViewEventData {
                                            liveview_id: id.clone(),
                                            component: component.clone(),
                                            event: event.clone(),
                                            params,
                                            sender_session: Some(socket_session.clone()),
                                            response_tx,
                                        };

                                        // try_send, like every other
                                        // enqueue site: a blocking send
                                        // parks a tokio worker thread
                                        // when the pool is saturated.
                                        if lv_event_tx.try_send(event_data).is_ok() {
                                            // Wait for response (with timeout)
                                            match tokio::time::timeout(
                                                std::time::Duration::from_secs(
                                                    super::server_constants::HEARTBEAT_TIMEOUT_SECS,
                                                ),
                                                response_rx,
                                            )
                                            .await
                                            {
                                                Ok(Ok(Ok(()))) => {
                                                    // Event handled successfully
                                                }
                                                Ok(Ok(Err(e))) => {
                                                    eprintln!("[LiveView] Event error: {}", e);
                                                }
                                                Ok(Err(_)) => {
                                                    eprintln!("[LiveView] Response channel closed");
                                                }
                                                Err(_) => {
                                                    eprintln!(
                                                        "[LiveView] Event handling timed out"
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }
                            } else if event_type == "heartbeat" {
                                // `send` here returns a future that was
                                // dropped, so the ack was never actually
                                // written. `try_send` is non-blocking and
                                // the enum owns the wire shape.
                                if let Ok(ack) = serde_json::to_string(
                                    &crate::live::view::ServerMessage::HeartbeatAck,
                                ) {
                                    let _ = tx_arc.try_send(Ok(tungstenite::Message::text(ack)));
                                }
                            } else if event_type == "resync" {
                                // The client lost its shadow copy (or a splice
                                // failed to apply): replay the last full render
                                // so it can rebuild without resetting server state.
                                if addressed_to_this_socket(
                                    liveview_id.as_deref(),
                                    &liveview_id_owned,
                                ) {
                                    use crate::live::view::{live_registry, ServerMessage};
                                    let id = liveview_id_owned.clone();
                                    if let Some(instance) = live_registry().get(&id) {
                                        let _ = live_registry().send(
                                            &id,
                                            ServerMessage::Render {
                                                html: instance.last_html,
                                                liveview_id: id.clone(),
                                            },
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Err(_) => {
                break;
            }
        }
    }

    write_task.abort();
    // Other tabs may still be attached to this instance.
    if crate::live::view::live_registry().drop_sender(&liveview_id, &tx_arc) {
        crate::live::socket::cancel_tick_task(&liveview_id);
        crate::live::view::live_registry().detach(&liveview_id);
    }
}

/// Handle WebSocket upgrade request.
async fn handle_websocket_upgrade(
    mut req: Request<Incoming>,
    ws_registry: Arc<WebSocketRegistry>,
    path: String,
    ws_event_tx: channel::Sender<WebSocketEventData>,
    peer_ip: String,
) -> Result<Response<ResponseBody>, hyper::Error> {
    // Check if this is a valid WebSocket upgrade request
    if !hyper_tungstenite::is_upgrade_request(&req) {
        return Ok(Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(full(Bytes::from("Not a WebSocket upgrade request")))
            .unwrap());
    }

    if !super::websocket_origin_allowed(req.headers()) {
        return Ok(forbidden_websocket_origin_response());
    }

    // Capture the socket's identity from the upgrade request. This is the only
    // point where cookies and headers exist — after the upgrade the connection
    // is a raw frame stream — so anything a handler needs to know about *who*
    // is connected has to be taken now.
    let ws_context = Arc::new(build_websocket_context(req.uri(), req.headers(), &peer_ip));

    // Admission control before the upgrade: every accepted socket costs a tokio
    // task, a 32-slot channel and a registry entry, none of which were bounded.
    let connection_slot = match admit_websocket(&peer_ip) {
        Ok(slot) => slot,
        Err(refused) => return Ok(*refused),
    };

    // Perform the WebSocket upgrade
    let ws_config = default_websocket_config();
    let (response, websocket) = match hyper_tungstenite::upgrade(&mut req, Some(ws_config)) {
        Ok(result) => result,
        Err(e) => {
            eprintln!("[WS] Upgrade error: {}", e);
            return Ok(Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(full(Bytes::from(format!("WebSocket upgrade error: {}", e))))
                .unwrap());
        }
    };

    // Spawn a task to handle the WebSocket connection
    let ws_registry = ws_registry.clone();
    let ws_event_tx = ws_event_tx.clone();
    let path = path.clone();

    super::tenant::spawn(async move {
        // Held for the life of the connection; dropping it frees the slot on
        // every exit path below, handshake failure included.
        let _connection_slot = connection_slot;

        // Wait for the WebSocket handshake to complete
        let stream = match websocket.await {
            Ok(ws) => ws,
            Err(e) => {
                eprintln!("[WS] WebSocket handshake error: {}", e);
                return;
            }
        };

        // Split the WebSocket stream into read and write halves
        let (mut ws_write, mut ws_read) = stream.split();

        // Create connection in registry
        let (ws_tx, mut ws_rx) =
            tokio::sync::mpsc::channel::<Result<tungstenite::Message, tungstenite::Error>>(32);
        let ws_tx_arc = Arc::new(ws_tx);
        let connection = WebSocketConnection::new(ws_tx_arc.clone());
        let connection_id = connection.id;

        ws_registry.register(connection).await;

        // Send connect event
        let (response_tx, _) = oneshot::channel();
        let connect_event = WebSocketEventData {
            path: path.clone(),
            connection_id,
            event_type: "connect".to_string(),
            message: None,
            channel: None,
            context: ws_context.clone(),
            response_tx,
        };
        if !enqueue_ws_event(&ws_event_tx, connect_event).await {
            eprintln!("[WS] realtime queue saturated, dropping connection {connection_id}");
            ws_registry.unregister(&connection_id).await;
            return;
        }

        let mut rate_limiter = crate::serve::websocket::WsRateLimiter::from_config();

        // Spawn task to forward messages from channel to WebSocket
        let write_task = super::tenant::spawn(async move {
            while let Some(msg_result) = ws_rx.recv().await {
                match msg_result {
                    Ok(msg) => {
                        if ws_write.send(msg).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        // Handle incoming messages
        while let Some(msg_result) = ws_read.next().await {
            match msg_result {
                Ok(msg) => {
                    if msg.is_close() {
                        break;
                    }
                    if msg.is_text() || msg.is_binary() {
                        // Per-connection budget: one socket must not be able to
                        // monopolise the shared realtime queue.
                        if !rate_limiter.allow() {
                            eprintln!(
                                "[WS] connection {connection_id} exceeded its message rate, closing"
                            );
                            break;
                        }
                        if let Ok(text) = msg.to_text() {
                            let (response_tx, _) = oneshot::channel();
                            let msg_event = WebSocketEventData {
                                path: path.clone(),
                                connection_id,
                                event_type: "message".to_string(),
                                message: Some(text.to_string()),
                                channel: None,
                                context: ws_context.clone(),
                                response_tx,
                            };
                            if !enqueue_ws_event(&ws_event_tx, msg_event).await {
                                eprintln!(
                                    "[WS] realtime queue saturated, closing connection {connection_id}"
                                );
                                break;
                            }
                        }
                    }
                }
                Err(_) => {
                    break;
                }
            }
        }

        // Send disconnect event
        let (response_tx, _) = oneshot::channel();
        let disconnect_event = WebSocketEventData {
            path: path.clone(),
            connection_id,
            event_type: "disconnect".to_string(),
            message: None,
            channel: None,
            context: ws_context.clone(),
            response_tx,
        };
        enqueue_ws_event(&ws_event_tx, disconnect_event).await;

        ws_registry.unregister(&connection_id).await;
        write_task.abort();
    });

    // Return the upgrade response directly
    Ok(box_full(response))
}

/// Snapshot the identity of an upgrading WebSocket: session, headers, query,
/// peer.
fn build_websocket_context(
    uri: &hyper::Uri,
    headers: &hyper::HeaderMap,
    peer_ip: &str,
) -> WebSocketContext {
    let cookie_pairs = parse_cookie_pairs(header_str(headers, "cookie"));
    let session_id = session_id_from_cookie_pairs(&cookie_pairs);

    let header_pairs: Vec<(String, String)> = headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|v| (name.as_str().to_ascii_lowercase(), v.to_string()))
        })
        .collect();

    let query = uri
        .query()
        .map(crate::interpreter::builtins::server::parse_query_pairs)
        .unwrap_or_default();

    WebSocketContext {
        session_id,
        headers: header_pairs,
        query,
        peer_ip: peer_ip.to_string(),
    }
}

/// Take a connection slot for a socket about to be upgraded, or the response
/// that refuses it. The one gate for every kind of socket — `/ws/*`, LiveView
/// and EUI — since each costs the same task, queue and registry entry; the
/// caps are `SOLI_WS_MAX_CONNECTIONS` and `SOLI_WS_MAX_CONNECTIONS_PER_IP`.
fn admit_websocket(
    peer_ip: &str,
) -> Result<crate::serve::websocket::WsConnectionSlot, Box<Response<ResponseBody>>> {
    crate::serve::websocket::ws_connection_limiter()
        .try_acquire(peer_ip)
        .map_err(|reason| {
            let (status, message) = match reason {
                crate::serve::websocket::WsAdmission::ServerFull => (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "WebSocket connection limit reached",
                ),
                crate::serve::websocket::WsAdmission::PerIpFull => (
                    StatusCode::TOO_MANY_REQUESTS,
                    "Too many WebSocket connections from this address",
                ),
            };
            Box::new(
                Response::builder()
                    .status(status)
                    .body(full(Bytes::from(message)))
                    .unwrap(),
            )
        })
}

/// Hand a WebSocket event to the realtime workers without ever blocking the
/// calling tokio task.
///
/// The blocking `send()` this replaces was a whole-server denial of service: a
/// few sockets flooding a `/ws/*` route filled the bounded event channel, and
/// every reader task then parked an OS thread of the tokio pool inside
/// `send()`. The interpreter workers drive their DB and HTTP futures with
/// `Handle::block_on` on that same pool, so once its threads were parked
/// nothing could make progress — accepts, HTTP requests and health checks
/// included. `try_send` with an async sleep keeps the thread free; when the
/// queue stays full past the deadline we shed the connection instead.
async fn enqueue_ws_event(
    tx: &channel::Sender<WebSocketEventData>,
    event: WebSocketEventData,
) -> bool {
    let deadline = tokio::time::Instant::now()
        + Duration::from_secs(server_constants::ws_enqueue_timeout_secs());
    let mut pending = Some(event);
    loop {
        let Some(data) = pending.take() else {
            return false;
        };
        match tx.try_send(data) {
            Ok(()) => return true,
            Err(crossbeam::channel::TrySendError::Full(returned)) => {
                if tokio::time::Instant::now() >= deadline {
                    return false;
                }
                pending = Some(returned);
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            Err(crossbeam::channel::TrySendError::Disconnected(_)) => return false,
        }
    }
}

/// SEC-047: tungstenite's default caps are 64 MiB per message and 16 MiB
/// per frame. Combined with the per-connection mpsc channel of 32
/// pending messages, a few hundred connections drip-feeding maximum-size
/// payloads can pin tens of GiB of worker memory. 1 MiB is plenty for
/// live-reload signals, LiveView event payloads, and form-shaped data;
/// large blobs (file uploads, images) belong on HTTP, not WS.
pub(crate) fn default_websocket_config() -> hyper_tungstenite::tungstenite::protocol::WebSocketConfig
{
    hyper_tungstenite::tungstenite::protocol::WebSocketConfig {
        max_message_size: Some(1 << 20),
        max_frame_size: Some(1 << 20),
        ..Default::default()
    }
}

/// The flat 403 every one of the four sockets answers a foreign origin with.
fn forbidden_websocket_origin_response() -> Response<ResponseBody> {
    Response::builder()
        .status(StatusCode::FORBIDDEN)
        .body(full(Bytes::from("Forbidden WebSocket origin")))
        .unwrap()
}

/// Is a client message addressed to the instance this socket owns?
///
/// A message with no `liveview_id` is accepted (it can only mean "mine"); one
/// naming a different instance is dropped — the client has no legitimate reason
/// to address another view, and honouring it would let it drive a component of
/// another session.
fn addressed_to_this_socket(claimed: Option<&str>, own: &str) -> bool {
    match claimed {
        None => true,
        Some(id) if id == own => true,
        Some(id) => {
            eprintln!("[LiveView] ignoring event addressed to {id} from the socket owning {own}");
            false
        }
    }
}

/// Reap LiveView instances whose socket closed (or that idled out). Started on
/// the first LiveView upgrade; `cleanup` is cheap and runs off the hot path.
fn start_liveview_reaper() {
    // One reaper per application: it sweeps that application's LiveView
    // registry, and runs as that tenant so it finds it. A process-wide
    // `OnceLock` here meant no second application could ever get a reaper.
    static STARTED: TenantCell<()> = TenantCell::new();
    if !STARTED.set_once(()) {
        return;
    }
    super::tenant::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            ticker.tick().await;
            crate::live::view::live_registry().cleanup();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_websocket_config_caps_message_and_frame_size() {
        // SEC-047: tungstenite's defaults (64 MiB / 16 MiB) are too
        // generous; lock the caps in so a future refactor can't quietly
        // restore them.
        let cfg = default_websocket_config();
        assert_eq!(cfg.max_message_size, Some(1 << 20));
        assert_eq!(cfg.max_frame_size, Some(1 << 20));
    }
}
