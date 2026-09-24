//! `GET /_eui/view/<component>` — one render, no session, nothing kept.
//!
//! The socket at [`super::session`] costs the server a resident session per
//! reader: an instance, an encoder holding the four interned tables *and the
//! whole previous tree*, and a memo on the worker that drew it. Measured
//! against `examples/eui-site`, that is 50–60 kB for a reader who is doing
//! nothing, which is the wrong shape for the pages that have the most
//! readers.
//!
//! This is the other way to be served. The body is exactly the bytes a fresh
//! socket would have sent — a `Welcome`, then the `Batch`es through the first
//! `Mount` — so the client needs no second codec, and a cache or a CDN in
//! front of it answers the second reader without the origin rendering again.
//! Everything the render created is gone before the response is written.
//!
//! Named `snapshot` rather than `view` because `view` already means the view
//! *action* throughout this module (`EUI_VIEWS`, `register_view`,
//! `view_action`), and a second meaning would be a tax on every later reader.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use bytes::Bytes;
use eui_proto::{Frame, Start, Viewport, Welcome, PROTOCOL_VERSION};
use hyper::header;
use hyper::{Response, StatusCode};
use tokio::sync::{oneshot, Semaphore};
use tungstenite::Message;

use crate::interpreter::value::Value;
use crate::live::view::{live_registry, LiveViewInstance};

use super::super::{full, LiveViewEventData, ResponseBody};
use super::{trace, with_encoder};

type WsSender = Arc<async_channel::Sender<Result<Message, tungstenite::Error>>>;

/// How long a one-shot render waits for the worker.
///
/// Deliberately not the socket's 30 s `HANDLER_TIMEOUT`. A socket holding a
/// slow handler costs one connection that is already open; an HTTP request
/// holding one costs a hyper task, a connection slot, and a reader looking at
/// nothing. Five seconds is longer than any first render has a right to be.
const VIEW_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// The largest body this endpoint will write, matching one frame's ceiling.
const MAX_SNAPSHOT_BYTES: usize = eui_proto::limits::MAX_FRAME_BYTES;

/// The nominal viewport a one-shot render is given when the request names
/// none. There is nobody to ask — that is the whole point of the endpoint —
/// and `Viewport::default()` is 0×0, which collapses any view that divides by
/// a width. A desktop-shaped default is the least surprising answer.
const NOMINAL_WIDTH: u32 = 1280;
const NOMINAL_HEIGHT: u32 = 800;

/// How many one-shot renders may be in flight at once.
///
/// Every request here runs a `connect` handler on a realtime worker, with no
/// session, no cookie and — a `GET` being exempt from the origin gate by
/// design — no same-origin check. Without a ceiling that is an unauthenticated
/// "occupy a worker" button. `SOLI_EUI_VIEW_CONCURRENCY` moves it.
fn admit() -> &'static Semaphore {
    static ADMIT: OnceLock<Semaphore> = OnceLock::new();
    ADMIT.get_or_init(|| {
        let n = std::env::var("SOLI_EUI_VIEW_CONCURRENCY")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|n| *n > 0)
            .unwrap_or(16);
        Semaphore::new(n)
    })
}

// ----------------------------------------------------------- the capture

/// Somewhere for one render's frames to go that is not a socket.
///
/// **Unbounded, and that is load-bearing.** The socket's queue holds 32, and
/// `send_or_close` waits `SEND_PATIENCE` for room before closing it. Here the
/// render runs on a worker that will not return until every frame is queued,
/// and the task that would drain the queue is the one waiting on that return
/// — so a bounded queue is a deadlock with a two-second fuse, and what comes
/// out of it is a **200 carrying a valid-looking prefix of the tree**. A tree
/// of a few thousand rows streams more than 32 batches, so this is the common
/// case rather than the pathological one.
struct Capture {
    sender: WsSender,
    frames: async_channel::Receiver<Result<Message, tungstenite::Error>>,
}

impl Capture {
    fn new() -> Self {
        let (tx, rx) = async_channel::unbounded();
        Self {
            sender: Arc::new(tx),
            frames: rx,
        }
    }

    fn sender(&self) -> WsSender {
        Arc::clone(&self.sender)
    }

    /// Every binary frame queued so far, in order. A render posts nothing
    /// else, so anything that is not binary is dropped rather than guessed at.
    fn drain(&self) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        while let Ok(Ok(Message::Binary(bytes))) = self.frames.try_recv() {
            out.push(bytes);
        }
        out
    }
}

/// BLAKE3 over a run of already-encoded frames, in order.
///
/// This is the identity of a *tree*, and it is deliberately not the ETag: the
/// ETag covers the whole body, `Welcome` included, and that frame carries a
/// session handle on a socket and sixteen zeroes here. Hash the body for the
/// cache; hash the batches for anything that has to recognise the same tree
/// arriving by another road.
pub fn frames_hash(frames: &[Vec<u8>]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    for frame in frames {
        hasher.update(frame);
    }
    *hasher.finalize().as_bytes()
}

// --------------------------------------------------------- the instance

/// Why a one-shot render produced no frames.
enum Refusal {
    /// The worker pool would not take it, or did not answer in time.
    Busy,
    /// The worker dropped the reply without sending one.
    Gone,
    /// The handler answered `{"close": reason}`.
    Closed(String),
    /// The handler raised, or the view would not encode.
    Failed(String),
}

/// A LiveView instance that exists for one render.
///
/// Everything it creates is named in [`Ephemeral::drop`], because everything
/// it creates leaks otherwise — and a leak here is the exact cost this
/// endpoint was built to remove.
struct Ephemeral {
    id: String,
    component: String,
    capture: Capture,
}

impl Ephemeral {
    fn new(component: &str, protocol: u32) -> Self {
        // Not `liveview_instance_id`: that is `{session_handle}:{component}`,
        // one per session and component, so a reader who also has a socket
        // open would collide with their own live session — `with_encoder`
        // would hand this render the live encoder, whose `prev` is the tree
        // that reader is looking at, so it would *diff* instead of mounting,
        // and the cleanup below would delete a live session out from under
        // its socket. A fresh id per request cannot. It also spreads renders
        // across the realtime workers, since `lv_sender_for` hashes it: a
        // constant id would funnel every static render of a documentation
        // site onto one worker.
        let id = format!("view-{}:{}", uuid::Uuid::new_v4(), component);
        let capture = Capture::new();
        let mut instance = LiveViewInstance::new(
            component.to_string(),
            PathBuf::new(),
            serde_json::json!({}),
            id.clone(),
            capture.sender(),
        );
        instance.id = id.clone();
        with_encoder(&id, |enc| {
            enc.set_protocol(protocol);
            // Nothing this render converts is kept on the worker: there is no
            // second render to be warm for, and the memo is a thread-local
            // that only that worker could free.
            enc.set_ephemeral(true);
        });
        // `register`, not `attach_or_register`: nothing can already hold an
        // id minted a line ago, and the difference matters — the latter would
        // quietly hand back somebody else's instance.
        live_registry().register(instance);
        Self {
            id,
            component: component.to_string(),
            capture,
        }
    }

    /// Post `connect` to the worker that owns this id and wait for it.
    async fn connect(
        &self,
        lv_event_tx: &crossbeam::channel::Sender<LiveViewEventData>,
        viewport: Viewport,
    ) -> Result<(), Refusal> {
        let (response_tx, response_rx) = oneshot::channel();
        let tx = crate::serve::lv_sender_for(&self.id, &self.component)
            .unwrap_or_else(|| lv_event_tx.clone());
        tx.try_send(LiveViewEventData {
            liveview_id: self.id.clone(),
            component: self.component.clone(),
            event: "connect".to_string(),
            params: serde_json::json!({"viewport": super::session::viewport_json(&viewport)}),
            // Anonymous, and that is the truth: this body will be served to
            // whoever asks for it, so nothing in the render may depend on who
            // did. It is also what lets a shared cache hold it without a
            // `Vary: Cookie`.
            sender_session: None,
            response_tx,
        })
        .map_err(|_| Refusal::Busy)?;
        match tokio::time::timeout(VIEW_TIMEOUT, response_rx).await {
            Ok(Ok(Ok(()))) => Ok(()),
            Ok(Ok(Err(e))) => match super::close_reason(&e) {
                Some(reason) => Err(Refusal::Closed(reason.to_string())),
                None => Err(Refusal::Failed(e)),
            },
            Ok(Err(_)) => Err(Refusal::Gone),
            Err(_) => Err(Refusal::Busy),
        }
    }

    fn frames(&self) -> Vec<Vec<u8>> {
        self.capture.drain()
    }
}

impl Drop for Ephemeral {
    fn drop(&mut self) {
        // In `Drop` rather than at the end of the happy path, so that a
        // timeout, an oversize body or a `?` anywhere cannot skip it.
        //
        // `unregister`, emphatically not `detach`: `detach` keeps the instance
        // for `DETACHED_GRACE` — two minutes — so that a reconnect can reclaim
        // its state. A one-shot has no reconnect, so `detach` would hold an
        // instance per reader for two minutes: a slower version of the leak
        // this endpoint exists to fix, invisible in testing and fatal under
        // load. `unregister` also drops the frame lock and the live-query
        // subscriptions.
        live_registry().unregister(&self.id);
        // After the registry, not before: `tree::with_memo`'s backstop sweep
        // keeps a memo while `has_encoder` is true, so the encoder going is
        // what tells a worker the session is over. Nothing was put in the memo
        // anyway — `set_ephemeral` saw to that — and this also takes the
        // `Stats` row with it.
        super::drop_encoder(&self.id);
        self.capture.sender.close();
    }
}

// ----------------------------------------------------------- the handler

/// Does an `Accept` ask for frames rather than a page?
///
/// Exact rather than a substring of the whole header, because a browser's
/// `Accept` ends in `*/*` and everything matches that. What is being asked
/// is whether the caller named this type, not whether it would tolerate it.
pub fn accepts_frames(accept: Option<&str>) -> bool {
    accept.is_some_and(|a| {
        a.split(',').any(|part| {
            part.split(';')
                .next()
                .unwrap_or("")
                .trim()
                .eq_ignore_ascii_case(FRAMES_MEDIA_TYPE)
        })
    })
}

/// The media type one render is answered with.
pub const FRAMES_MEDIA_TYPE: &str = "application/vnd.eui.frames";

/// `eui_render(tree)`: one render of a view value, as a response hash.
///
/// The encoder is built here, used, and dropped when this returns. There is
/// no instance, no registry entry and nothing to clean up, because an action
/// is already running on a worker with an interpreter — which is why a page
/// on an ordinary route is cheaper to serve than one behind
/// `GET /_eui/view/<component>`, not dearer.
pub fn render_once(
    tree: &Value,
    version: u32,
    if_none_match: Option<&str>,
) -> Result<Value, String> {
    let mut encoder = super::tree::Encoder::default();
    encoder.set_protocol(version);
    // Nothing kept on the worker: this render has no second render to be
    // warm for.
    encoder.set_ephemeral(true);
    let batches = encoder.render_value("", tree, false)?;
    if batches.is_empty() {
        return Err("eui_render: the view produced nothing to mount".to_string());
    }

    // Sixteen zero bytes: this body names no session, because it is
    // everyone's. A real handle here would give every response its own ETag
    // and cache nothing while appearing to.
    let mut body = Frame::Welcome(Welcome {
        version,
        session: [0u8; 16],
        start: Start::Fresh,
    })
    .encode();
    for batch in &batches {
        body.extend_from_slice(&Frame::Batch(batch.clone()).encode());
    }
    if body.len() > MAX_SNAPSHOT_BYTES {
        return Err(format!(
            "eui_render: {} bytes is past what one render may be",
            body.len()
        ));
    }
    let etag = format!("\"{}\"", blake3::hash(&body).to_hex());

    let key = |k: &str| crate::interpreter::value::HashKey::String(k.into());
    let mut headers = crate::interpreter::value::HashPairs::default();
    headers.insert(key("Content-Type"), Value::String(FRAMES_MEDIA_TYPE.into()));
    headers.insert(key("ETag"), Value::String(etag.clone().into()));
    // The application decides how long this may be held; saying nothing here
    // leaves that to whatever it sets itself.
    let hashed = |pairs| Value::Hash(std::rc::Rc::new(std::cell::RefCell::new(pairs)));

    let mut out = crate::interpreter::value::HashPairs::default();
    if if_none_match.is_some_and(|inm| inm.split(',').map(str::trim).any(|c| c == "*" || c == etag))
    {
        out.insert(key("status"), Value::Int(304));
        out.insert(key("headers"), hashed(headers));
        out.insert(key("body"), Value::String("".into()));
        return Ok(hashed(out));
    }
    out.insert(key("status"), Value::Int(200));
    out.insert(key("headers"), hashed(headers));
    // Soli has no bytes type, so a binary body travels as base64 and is
    // decoded once on the way out (`body_base64` in `extract_response`).
    out.insert(key("body_base64"), Value::String(b64(&body).into()));
    Ok(hashed(out))
}

/// Standard base64, no padding omitted — what `extract_response` decodes.
fn b64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk.first().copied().unwrap_or(0),
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let idx = [(n >> 18) & 63, (n >> 12) & 63, (n >> 6) & 63, n & 63];
        for (i, part) in idx.iter().enumerate() {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[*part as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// A `Cache-Control` an application asked for, checked before it is ever
/// written into a header.
///
/// One load-bearing rule and one cosmetic one: printable ASCII only, and at
/// most 128 bytes of it. The value goes into a response header verbatim, and
/// a `\r\n` in it is header injection. `HeaderValue::from_str` would refuse
/// too, but refusing at boot with the line number of the `router_eui` that
/// wrote it beats refusing once per request with nothing to go on.
///
/// Deliberately no attempt to parse the directives: an invalid `Cache-Control`
/// is the application's business, an injected header is not.
pub fn validate_cache_control(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("router_eui: static's Cache-Control is empty".to_string());
    }
    if value.len() > 128 {
        return Err(format!(
            "router_eui: static's Cache-Control is {} bytes; 128 is the most a directive list needs",
            value.len()
        ));
    }
    if !value.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
        return Err(
            "router_eui: static's Cache-Control may hold printable ASCII only; a newline in a \
             header value is header injection"
                .to_string(),
        );
    }
    Ok(value.to_string())
}

fn refuse(status: StatusCode, message: &str) -> Response<ResponseBody> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .body(full(Bytes::from(message.to_string())))
        .unwrap_or_else(|_| Response::new(full(Bytes::new())))
}

/// One query parameter, as a borrowed value.
fn param<'a>(query: Option<&'a str>, name: &str) -> Option<&'a str> {
    query?.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == name).then_some(v)
    })
}

/// Does `If-None-Match` name `etag`? A list, `*`, or nothing.
fn matches_etag(headers: &hyper::HeaderMap, etag: &str) -> bool {
    headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(',')
                .map(str::trim)
                .any(|candidate| candidate == "*" || candidate == etag)
        })
}

/// The page a browser gets when it opens an address that speaks EUI.
///
/// Not an error: arriving here in a browser is a reasonable thing to have
/// done, and a 404 would teach nobody anything. It says what the address is,
/// what the protocol saves, and how to open it — with this server's own
/// address already written out, because the useful thing to hand somebody is
/// the line they are going to need.
const BROWSER_PAGE: &str = include_str!("browser.html");

/// Does the caller want HTML rather than frames?
///
/// Only an explicit `text/html` counts. A browser always sends it; `*/*` from
/// curl does not, and an EUI client asks for `application/vnd.eui.frames` —
/// so the default, when nobody says, stays the protocol's own answer.
fn wants_html(headers: &hyper::HeaderMap) -> bool {
    accepts_html(headers.get(header::ACCEPT).and_then(|v| v.to_str().ok()))
}

/// The address to open, as this server would be reached: `wss://` everywhere
/// but loopback, where there is no certificate and the client is told so.
fn open_address(host: Option<&str>, forwarded_host: Option<&str>, proto: Option<&str>) -> String {
    let sane = |h: &&str| !h.is_empty() && h.len() < 256 && !h.contains(|c: char| c.is_control());
    // A forwarded list is `client, proxy1, proxy2`; the first is the one the
    // reader typed.
    let first = |v: &str| v.split(',').next().unwrap_or(v).trim().to_owned();
    let forwarded = forwarded_host.map(first);
    let host = forwarded
        .as_deref()
        .filter(sane)
        .or_else(|| host.filter(sane))
        .unwrap_or("localhost");
    let loopback =
        host.starts_with("127.0.0.1") || host.starts_with("localhost") || host.starts_with("[::1]");
    // What the proxy says beats what the socket looks like: TLS in front of a
    // loopback upstream is the ordinary deployment, and it is `wss://` to
    // everyone who is not this machine. With nothing forwarded, the host is
    // the only evidence there is.
    let secure = match proto.map(first) {
        Some(p) => p.eq_ignore_ascii_case("https"),
        None => !loopback,
    };
    // The origin as the reader's browser shows it, and nothing more.
    //
    // It used to print the whole session address — `wss://host/_eui/session/
    // <component>` — which is correct and is not what anybody would type.
    // The client now takes `https://` as a spelling of `wss://` and completes
    // a bare origin from the manifest's `entry` (01 §2.1: "the protocol's own
    // prefix is the part nobody should have to type"), so the shortest thing
    // that works is the address already in the address bar.
    let scheme = if secure { "https" } else { "http" };
    // The opt-out is loopback-only in the client, so offering it anywhere
    // else would be a command that cannot work. A plain-http origin that is
    // not loopback is refused outright (01 §1), and the honest thing is to
    // print the address without a spell that will not lift the refusal.
    let prefix = if !secure && loopback {
        "EUI_ALLOW_INSECURE_LOOPBACK=1 "
    } else {
        ""
    };
    format!("{prefix}eui {scheme}://{host}/")
}

/// The browser page, with this server's own address written into it.
pub fn browser_body(
    host: Option<&str>,
    forwarded_host: Option<&str>,
    proto: Option<&str>,
) -> String {
    let address = open_address(host, forwarded_host, proto);
    BROWSER_PAGE.replace("__ADDRESS__", &escape(&address))
}

/// Is `accept` a browser asking for a page?
///
/// Only an explicit `text/html` counts, so `*/*` from curl and an EUI
/// client's own `Accept` both fall through to the protocol's answer.
pub fn accepts_html(accept: Option<&str>) -> bool {
    accept.is_some_and(|a| a.contains("text/html"))
}

/// The browser page as a response.
pub fn browser_page(headers: &hyper::HeaderMap) -> Response<ResponseBody> {
    let get = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    let body = browser_body(
        get("host"),
        get("x-forwarded-host"),
        get("x-forwarded-proto"),
    );
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(header::CONTENT_LENGTH, body.len())
        // The address in it comes from the request, so it is this reader's
        // page and not a shared one.
        .header(header::CACHE_CONTROL, "no-store")
        .header(header::VARY, "Accept")
        .body(full(Bytes::from(body)))
        .unwrap_or_else(|_| Response::new(full(Bytes::new())))
}

/// The four characters that could turn a `Host` header into markup.
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// `GET /_eui/view/<component>?v=<protocol>&w=<width>` — the frames a fresh
/// socket would have sent, for a component that said it could be.
pub async fn respond(
    component: &str,
    query: Option<&str>,
    headers: &hyper::HeaderMap,
    lv_event_tx: &crossbeam::channel::Sender<LiveViewEventData>,
) -> Response<ResponseBody> {
    // One lookup answers both questions: is this a component, and did it opt
    // in. A 404 for either, and the same 404, so the endpoint tells a prober
    // nothing it could not already learn from the session path.
    // A browser at this address is not an error, and the protocol's own
    // answer would be a screenful of binary. Content negotiation, and the
    // only place this server has two representations of one thing.
    if wants_html(headers) && super::is_eui_component(component) {
        return browser_page(headers);
    }
    let Some(cache_control) = super::static_cache_control(component) else {
        return refuse(StatusCode::NOT_FOUND, "no such static view");
    };
    // Belt and braces: `router_eui` refuses the pair at registration, so this
    // can only fire if some later path registers the two separately.
    if super::session_required(component) {
        return refuse(StatusCode::NOT_FOUND, "no such static view");
    }

    // The client's version is in the query because it is part of the cache
    // key: a client at 3 and a client at 5 get different bytes, and a shared
    // cache must not hand one the other's.
    let version = match param(query, "v") {
        None => PROTOCOL_VERSION,
        Some(raw) => match raw.parse::<u32>() {
            // A `Welcome` naming version 0 makes the client close rather than
            // draw, so this is a 400 and not a clamp.
            Ok(v) if (1..=1000).contains(&v) => v,
            _ => return refuse(StatusCode::BAD_REQUEST, "?v= names a protocol version"),
        },
    };
    let negotiated = version.min(PROTOCOL_VERSION);
    let width = param(query, "w")
        .and_then(|raw| raw.parse::<u32>().ok())
        .filter(|w| *w >= 128 && *w <= 8192)
        .unwrap_or(NOMINAL_WIDTH);

    let Ok(_permit) = admit().try_acquire() else {
        return Response::builder()
            .status(StatusCode::SERVICE_UNAVAILABLE)
            .header(header::RETRY_AFTER, "1")
            .header(header::CACHE_CONTROL, "no-store")
            .body(full(Bytes::from("server busy")))
            .unwrap_or_else(|_| Response::new(full(Bytes::new())));
    };

    let shot = Ephemeral::new(component, negotiated);
    let viewport = Viewport {
        width,
        height: NOMINAL_HEIGHT,
        ..Viewport::default()
    };
    if let Err(refusal) = shot.connect(lv_event_tx, viewport).await {
        return match refusal {
            Refusal::Busy => Response::builder()
                .status(StatusCode::SERVICE_UNAVAILABLE)
                .header(header::RETRY_AFTER, "1")
                .header(header::CACHE_CONTROL, "no-store")
                .body(full(Bytes::from("server busy")))
                .unwrap_or_else(|_| Response::new(full(Bytes::new()))),
            // The application saying "not you" to a request that carries no
            // identity is still a meaningful answer, and 403 is the code the
            // session path already gives a handler close.
            Refusal::Closed(reason) => refuse(StatusCode::FORBIDDEN, &reason),
            Refusal::Gone => refuse(
                StatusCode::INTERNAL_SERVER_ERROR,
                "the handler did not answer",
            ),
            Refusal::Failed(e) => {
                eprintln!("[EUI] view {component}: {e}");
                refuse(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "the view could not be rendered",
                )
            }
        };
    }

    let batches = shot.frames();
    if batches.is_empty() {
        // A fresh encoder always mounts, so this is not a view that had
        // nothing to say — it is a render whose frames went somewhere else.
        return refuse(
            StatusCode::INTERNAL_SERVER_ERROR,
            "the view rendered no frames",
        );
    }
    let total: usize = batches.iter().map(Vec::len).sum();
    if total > MAX_SNAPSHOT_BYTES {
        return refuse(
            StatusCode::INTERNAL_SERVER_ERROR,
            "the view is too large to serve at once",
        );
    }

    // The session handle is sixteen zeroes, and that is what makes this body
    // cacheable at all: `welcome_session` would put a per-request value in the
    // body's fourth byte, every ETag would be unique, and the endpoint would
    // cache nothing while looking as though it did. It names no session
    // because there is none — this body is everyone's.
    let mut body = Frame::Welcome(Welcome {
        version: negotiated,
        session: [0u8; 16],
        start: Start::Fresh,
    })
    .encode();
    for batch in &batches {
        body.extend_from_slice(batch);
    }
    let etag = format!("\"{}\"", blake3::hash(&body).to_hex());

    if trace() {
        eprintln!(
            "[EUI trace] view {component}: v{negotiated} w{width}, {} batch(es), {} bytes, {etag}",
            batches.len(),
            body.len()
        );
    }

    if matches_etag(headers, &etag) {
        return Response::builder()
            .status(StatusCode::NOT_MODIFIED)
            .header(header::ETAG, &etag)
            .header(header::CACHE_CONTROL, &cache_control)
            .body(full(Bytes::new()))
            .unwrap_or_else(|_| Response::new(full(Bytes::new())));
    }

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/vnd.eui.frames")
        .header(header::CONTENT_LENGTH, body.len())
        .header(header::ETAG, &etag)
        .header(header::CACHE_CONTROL, &cache_control)
        .body(full(Bytes::from(body)))
        .unwrap_or_else(|_| Response::new(full(Bytes::new())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_one_shot_render_is_sent_the_space_steps_its_version_has() {
        // 01 §2.4 puts the client's version in the query, and 05 §2 says a
        // session below 6 is sent a fallback for `space` 13-17. `eui_render`
        // and `GET /_eui/view` both come through here with that version.
        use base64::Engine as _;
        let tree = crate::interpreter::value::json_to_value(
            serde_json::json!({"k": "box", "s": {"pad": [13, 14, 15, 17]}}),
        )
        .unwrap();
        let styles = |version: u32| -> (u32, Vec<eui_proto::StyleRecord>) {
            let out = render_once(&tree, version, None).unwrap();
            let Value::Hash(h) = out else {
                panic!("a response hash")
            };
            let key = crate::interpreter::value::HashKey::String("body_base64".into());
            let Some(Value::String(b)) = h.borrow().get(&key).cloned() else {
                panic!("a body")
            };
            let body = base64::engine::general_purpose::STANDARD
                .decode(b.as_bytes())
                .unwrap();
            let (mut at, mut welcomed, mut records) = (0, 0, Vec::new());
            while at < body.len() {
                let (frame, used) = Frame::decode_prefix(&body[at..]).unwrap();
                at += used;
                match frame {
                    Frame::Welcome(w) => welcomed = w.version,
                    Frame::Batch(b) => {
                        records.extend(b.ops.into_iter().filter_map(|op| match op {
                            eui_proto::Op::DefStyle { record, .. } => Some(record),
                            _ => None,
                        }))
                    }
                    _ => {}
                }
            }
            (welcomed, records)
        };
        let (v, old) = styles(5);
        assert_eq!(v, 5);
        assert_eq!(old[0].padding, [2, 3, 4, 12], "the older steps at 5");
        let (v, new) = styles(6);
        assert_eq!(v, 6);
        assert_eq!(
            new[0].padding,
            [13, 14, 15, 17],
            "the steps as written at 6"
        );
    }

    #[test]
    fn a_cache_control_with_a_newline_in_it_is_refused() {
        // Header injection. The value is written into a response header
        // verbatim, so this is the one check that is not cosmetic.
        assert!(validate_cache_control("public\r\nSet-Cookie: a=1").is_err());
        assert!(validate_cache_control("public\nX: y").is_err());
        assert!(validate_cache_control("public, max-age=60").is_ok());
        assert!(validate_cache_control("  no-cache  ").unwrap() == "no-cache");
        assert!(validate_cache_control("").is_err());
        assert!(validate_cache_control(&"a".repeat(129)).is_err());
    }

    #[test]
    fn the_query_is_read_one_parameter_at_a_time() {
        assert_eq!(param(Some("v=4&w=800"), "v"), Some("4"));
        assert_eq!(param(Some("v=4&w=800"), "w"), Some("800"));
        assert_eq!(param(Some("v=4"), "w"), None);
        assert_eq!(param(None, "v"), None);
        // A bare flag names no value, and must not be read as one.
        assert_eq!(param(Some("v"), "v"), None);
    }

    #[test]
    fn if_none_match_is_read_as_a_list() {
        let mut headers = hyper::HeaderMap::new();
        let etag = "\"abc\"";
        assert!(!matches_etag(&headers, etag));
        headers.insert(header::IF_NONE_MATCH, "\"abc\"".parse().unwrap());
        assert!(matches_etag(&headers, etag));
        headers.insert(header::IF_NONE_MATCH, "*".parse().unwrap());
        assert!(matches_etag(&headers, etag));
        headers.insert(
            header::IF_NONE_MATCH,
            "\"x\", \"abc\", \"y\"".parse().unwrap(),
        );
        assert!(matches_etag(&headers, etag));
        headers.insert(header::IF_NONE_MATCH, "\"ab\"".parse().unwrap());
        assert!(!matches_etag(&headers, etag));
    }

    #[test]
    fn the_tree_hash_is_not_the_body_hash() {
        // The trap worth pinning: an adopt offer names the *tree*, and the
        // `Welcome` differs between the two roads it can arrive by — sixteen
        // zeroes over HTTP, a session handle on a socket. Hash the body for
        // the cache and the batches for recognising the same tree.
        let welcome_http = Frame::Welcome(Welcome {
            version: 4,
            session: [0u8; 16],
            start: Start::Fresh,
        })
        .encode();
        let welcome_sock = Frame::Welcome(Welcome {
            version: 4,
            session: [7u8; 16],
            start: Start::Fresh,
        })
        .encode();
        let batches = vec![vec![0x03, 0x02, 0x01, 0x00], vec![0x03, 0x01, 0x20]];

        assert_eq!(frames_hash(&batches), frames_hash(&batches));

        let mut body_http = welcome_http.clone();
        let mut body_sock = welcome_sock.clone();
        for b in &batches {
            body_http.extend_from_slice(b);
            body_sock.extend_from_slice(b);
        }
        assert_ne!(
            blake3::hash(&body_http).as_bytes(),
            blake3::hash(&body_sock).as_bytes(),
            "the two bodies differ, which is why the ETag cannot be the offer"
        );
    }

    #[test]
    fn the_address_is_the_one_the_reader_typed() {
        // Behind the proxy the `Host` header is the upstream this process
        // listens on, so a line built from it hands somebody
        // `ws://localhost:20059/...` — right for the proxy, unreachable for
        // everyone else. The whole job of that line is to be copied.
        let direct = open_address(Some("127.0.0.1:5190"), None, None);
        assert!(direct.contains("http://127.0.0.1:5190/"), "{direct}");
        assert!(direct.starts_with("EUI_ALLOW_INSECURE_LOOPBACK=1 "));

        let proxied = open_address(
            Some("localhost:20059"),
            Some("eui-site.solisoft.test"),
            Some("https"),
        );
        assert_eq!(proxied, "eui https://eui-site.solisoft.test/");

        // A forwarded list names the reader first.
        let chained = open_address(None, Some("a.example, proxy.internal"), Some("https, http"));
        assert_eq!(chained, "eui https://a.example/");

        // Plain http through a proxy stays http, and the loopback spell is
        // not offered where it cannot work.
        let plain = open_address(Some("localhost:9"), Some("box.local"), Some("http"));
        assert_eq!(plain, "eui http://box.local/");
    }

    #[tokio::test]
    async fn a_capture_takes_every_frame_however_many() {
        // The socket's queue holds 32 and closes when a reader will not drain
        // it. Here nobody drains until the render has returned, so a bounded
        // queue would be a 200 carrying a prefix of the tree.
        let capture = Capture::new();
        let sender = capture.sender();
        for i in 0..200u32 {
            sender
                .try_send(Ok(Message::Binary(vec![0x03, i as u8])))
                .expect("an unbounded queue never refuses");
        }
        let drained = capture.drain();
        assert_eq!(drained.len(), 200);
        assert_eq!(drained[199], vec![0x03, 199]);
    }
}
