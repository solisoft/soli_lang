//! From the request to a worker and back.
//!
//! The other half of `handle_hyper_request`: what happens once the dispatch
//! cascade has decided that this request is the application's to answer.
//! Reading the body, handing the work to an interpreter thread, waiting for
//! its reply, and turning that reply into an HTTP response.
//!
//! Four stages, and the seam between each pair is a value rather than a set
//! of live locals — which is what kept them in one function: [`intake`]
//! produces nine of them, and two of those ([`Intake::if_none_match`],
//! [`Intake::is_prefetch`]) exist only because the headers they come from are
//! moved to the worker before the reply is built.

use std::borrow::Cow;
use std::time::{Duration, Instant};

use bytes::Bytes;
use futures_util::StreamExt;
use http_body_util::{BodyExt, StreamBody};
use hyper::body::Incoming;
use hyper::{Request, Response, StatusCode};
use tokio::sync::oneshot;

use super::file_upload::parse_multipart_body;
use super::{
    add_header_checked, apply_form_method_override, finish_response, full, header_str, live_reload,
    parse_query_pairs, server_constants, RequestData, ResponseBody, UploadedFile, WorkerResponse,
    WorkerSender, NO_INJECT_HEADER,
};

/// The response to send instead of carrying on: a stage that cannot produce
/// its output produces one of these — 413, 408 or 503 from [`intake`], 503
/// from [`enqueue`], 504 or 500 from [`await_worker`].
///
/// Boxed because a `Response<ResponseBody>` is 128 bytes and clippy refuses
/// to see one inside a `Result`. The box costs an allocation only on the path
/// that is already failing; `upgrade::admit_websocket` boxes its refusal for
/// the same reason.
type EarlyResponse = Box<Response<ResponseBody>>;

/// Everything the worker needs from the wire, plus the two things the *reply*
/// will still need once the headers have gone with it.
///
/// Returned as one struct and destructured at the call site, the way
/// `TenantRuntime` is: these are nine locals the tail of the request handler
/// reads by name, and threading them back as a tuple would have renamed them
/// all at the seam.
pub(super) struct Intake {
    /// The verb the application sees. Not always the one on the wire: a form
    /// may have asked for another (see `apply_form_method_override`).
    pub method: Cow<'static, str>,
    pub query: Vec<(String, String)>,
    pub headers: hyper::header::HeaderMap,
    pub body: String,
    pub body_reservation: Option<crate::interpreter::builtins::body_limit::BodyReservation>,
    pub multipart_form: Option<Vec<(String, String)>>,
    pub multipart_files: Option<Vec<UploadedFile>>,
    /// The conditional-GET validator, kept because [`assemble`] needs it after
    /// `headers` has been moved into the `RequestData`.
    pub if_none_match: Option<String>,
    /// Likewise: whether this was a browser speculative prefetch.
    pub is_prefetch: bool,
}

/// Read the request into the shape a worker takes, or the response to send
/// instead — 413 for an oversize body, 503 when the in-flight body budget (or
/// this client's share of it) is exhausted, 408 for one that never finishes
/// arriving. `peer_ip` is the TCP peer; see [`body_budget_client`] for how it
/// becomes the client the budget is charged to.
pub(super) async fn intake(
    req: Request<Incoming>,
    method: Cow<'static, str>,
    raw_query: Option<&str>,
    peer_ip: std::net::IpAddr,
) -> Result<Intake, EarlyResponse> {
    // Parse query string into ordered pairs (order matters for bracket
    // arrays like tags[]=a&tags[]=b — the worker nests them Rack-style).
    let query = parse_query_pairs(raw_query.unwrap_or(""));

    // Split the request: take ownership of the wire headers (moved into
    // RequestData as-is — `HeaderMap` is Send) and the body stream. No
    // per-header String copies happen here; the worker converts the map to
    // Soli HashPairs exactly once when it builds `req["headers"]`.
    let (parts, req_body) = req.into_parts();
    let headers = parts.headers;

    // Keep the conditional-GET validator around so `assemble` can
    // short-circuit to 304 when the controller's rendered ETag matches the
    // browser's cached copy. Read here because `headers` is about to become
    // the worker's.
    let if_none_match = header_str(&headers, "if-none-match").map(|v| v.to_owned());

    // Is this a browser speculative prefetch (hover-preload)? If so `assemble`
    // relaxes the HTML `Cache-Control` so the eventual click reuses the
    // prefetched bytes without a revalidation round-trip — see
    // `prefetch::prefetch_cache_control`. Read here for the same reason.
    let is_prefetch =
        crate::serve::prefetch::is_prefetch_request(|name| header_str(&headers, name));

    // Content headers used by the body-reading block below.
    let declared_content_length =
        header_str(&headers, "content-length").and_then(|v| v.parse::<usize>().ok());
    let req_content_type = header_str(&headers, "content-type").map(|v| v.to_owned());

    // Read body - skip for GET/HEAD requests (usually empty). Cap the
    // read so a hostile client can't exhaust worker memory by streaming
    // an unbounded body. Content-Length lets us short-circuit before any
    // bytes are buffered; chunked uploads (no Content-Length) are caught
    // mid-stream by `read_body`.
    let max_body = crate::interpreter::builtins::body_limit::get_max_body_size();
    let mut body_reservation = None;
    if method != "GET" && method != "HEAD" {
        if let Some(declared) = declared_content_length {
            if declared > max_body {
                return Err(Box::new(
                    Response::builder()
                        .status(StatusCode::PAYLOAD_TOO_LARGE)
                        .header("Content-Type", "text/plain; charset=utf-8")
                        .body(full(Bytes::from("Request body too large")))
                        .unwrap(),
                ));
            }
        }
        // Claim the memory *before* buffering, not after: the point is to stop
        // many connections each collecting their own body. Only a first slice
        // is claimed here — the declared length when it is small, at most
        // `INITIAL_BODY_RESERVATION` — and `read_body` grows the claim as bytes
        // actually arrive. Reserving the whole cap for every chunked upload
        // (which declares no length) let sixteen idle connections exhaust the
        // default budget and 503 every other POST; a declared-but-never-sent
        // `Content-Length` did the same.
        let want = declared_content_length
            .unwrap_or(INITIAL_BODY_RESERVATION)
            .min(INITIAL_BODY_RESERVATION)
            .min(max_body);
        // Charged to this client's share as well as to the whole, so one client
        // cannot hold the entire budget and 503 everyone else.
        let client = body_budget_client(peer_ip, &headers);
        match crate::interpreter::builtins::body_limit::BodyReservation::try_acquire_for(
            want,
            Some(client),
        ) {
            Some(reservation) => body_reservation = Some(reservation),
            None => return Err(Box::new(server_busy_response())),
        }
    }
    let (body, multipart_form, multipart_files) = if method == "GET" || method == "HEAD" {
        (String::new(), None, None)
    } else {
        // Bounded in time as well as size: a cap on how much can arrive says
        // nothing about how long it may take, so a trickled body held a
        // connection and its buffer indefinitely. Two clocks: the whole body
        // within `body_read_timeout_secs`, and no gap between frames longer
        // than `body_idle_timeout_secs`.
        let reservation = body_reservation
            .as_mut()
            .expect("a reservation is taken for every body-bearing method");
        let body_bytes = match read_body(
            req_body,
            max_body,
            reservation,
            Duration::from_secs(server_constants::body_idle_timeout_secs()),
            Duration::from_secs(server_constants::body_read_timeout_secs()),
        )
        .await
        {
            Ok(bytes) => bytes,
            Err(BodyReadError::TooLarge) => {
                return Err(Box::new(
                    Response::builder()
                        .status(StatusCode::PAYLOAD_TOO_LARGE)
                        .header("Content-Type", "text/plain; charset=utf-8")
                        .body(full(Bytes::from("Request body too large")))
                        .unwrap(),
                ));
            }
            Err(BodyReadError::Busy) => return Err(Box::new(server_busy_response())),
            Err(BodyReadError::TimedOut) => {
                return Err(Box::new(
                    Response::builder()
                        .status(StatusCode::REQUEST_TIMEOUT)
                        .header("Content-Type", "text/plain; charset=utf-8")
                        .body(full(Bytes::from("Request body read timed out")))
                        .unwrap(),
                ));
            }
            Err(BodyReadError::Transport) => {
                // The connection failed mid-body (reset, or fewer bytes than
                // the declared `Content-Length`). Never proceed with a
                // partial body.
                return Err(Box::new(
                    Response::builder()
                        .status(StatusCode::BAD_REQUEST)
                        .header("Content-Type", "text/plain; charset=utf-8")
                        .body(full(Bytes::from("Request body could not be read")))
                        .unwrap(),
                ));
            }
        };

        // Check if this is a multipart form
        let content_type = req_content_type.as_deref();
        if let Some(ct) = content_type {
            if ct.starts_with("multipart/form-data") {
                // Structured view only: form fields + files. Do not also
                // allocate a lossy UTF-8 `String` of the raw multipart
                // bytes (binary boundary noise) — that triple-buffered
                // an 8 MiB body as bytes + string + parsed parts.
                // CSRF / `_method` read `multipart_form`.
                //
                // The raw body is moved into the parser and not retained.
                // It used to ride along in `RequestData.body_bytes`, which
                // nothing in the tree ever read (the field was
                // `#[allow(dead_code)]`), so every upload carried a second
                // full copy of itself across the worker queue.
                let (form_fields, files) = parse_multipart_body(body_bytes, ct).await;
                (String::new(), Some(form_fields), Some(files))
            } else {
                (body_into_string(body_bytes), None, None)
            }
        } else {
            (body_into_string(body_bytes), None, None)
        }
    };

    // HTML forms can only express GET and POST. Rails-style method override:
    // a POST whose form body carries `_method=PUT|PATCH|DELETE` (the hidden
    // input `form_with` / `button_to` and the scaffold emit) is routed and
    // dispatched to the app as that verb. Applied after the CSRF origin gate
    // in `handle_hyper_request` — the overridden verbs are state-changing
    // either way.
    let method = apply_form_method_override(
        method,
        &body,
        req_content_type.as_deref(),
        multipart_form.as_deref(),
    );

    Ok(Intake {
        method,
        query,
        headers,
        body,
        body_reservation,
        multipart_form,
        multipart_files,
        if_none_match,
        is_prefetch,
    })
}

/// The first slice of the in-flight body budget a request claims before any
/// of its body has arrived. Small bodies (the declared length, when it is
/// below this) claim exactly what they are; anything larger grows its claim as
/// bytes arrive, so a connection that sends nothing holds almost nothing.
const INITIAL_BODY_RESERVATION: usize = 64 * 1024;

/// The client a request body's budget is charged to.
///
/// The TCP peer, unless the application trusts its proxy — then the
/// right-most `X-Forwarded-For` entry, the address the trusted hop recorded,
/// exactly as the rate limiter (`rate_limit::extract_client_ip`) derives it.
/// Without that, every client behind a reverse proxy would share the proxy's
/// one share of the budget. An unparsable or absent header falls back to the
/// peer.
///
/// `is_trust_proxy_enabled` narrows to `SOLI_TRUSTED_PROXIES` by the peer
/// recorded in a thread-local that only worker threads set; it is set here for
/// the one call and cleared again, so a direct client cannot pass for a
/// trusted hop by sending the header itself.
fn body_budget_client(
    peer_ip: std::net::IpAddr,
    headers: &hyper::header::HeaderMap,
) -> std::net::IpAddr {
    use crate::interpreter::builtins::trust_proxy;
    trust_proxy::set_current_peer_ip(Some(peer_ip));
    let trusted = trust_proxy::is_trust_proxy_enabled();
    trust_proxy::set_current_peer_ip(None);
    if !trusted {
        return peer_ip;
    }
    header_str(headers, "x-forwarded-for")
        .and_then(|xff| xff.rsplit(',').map(str::trim).find(|s| !s.is_empty()))
        .and_then(|ip| ip.parse().ok())
        .unwrap_or(peer_ip)
}

fn server_busy_response() -> Response<ResponseBody> {
    Response::builder()
        .status(StatusCode::SERVICE_UNAVAILABLE)
        .header("Content-Type", "text/plain; charset=utf-8")
        .header("Retry-After", "1")
        .body(full(Bytes::from("Server busy: too many uploads in flight")))
        .unwrap()
}

/// Why [`read_body`] gave up.
#[derive(Debug, PartialEq, Eq)]
enum BodyReadError {
    /// More than `max_body` bytes arrived.
    TooLarge,
    /// The in-flight budget could not cover the bytes that arrived.
    Busy,
    /// The body stalled past the idle timeout, or overran the total one.
    TimedOut,
    /// The transport failed mid-body.
    Transport,
}

/// The request body as a `String`. Valid UTF-8 — nearly every body — takes
/// the bytes over without a second pass through `from_utf8_lossy`'s copy
/// (`Vec::from(Bytes)` reuses the allocation when the buffer is uniquely
/// owned); invalid input is still replaced lossily, as before.
fn body_into_string(body: Bytes) -> String {
    String::from_utf8(Vec::from(body))
        .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

/// Buffer a request body, charging the in-flight budget for the bytes as they
/// arrive rather than for the most they could be.
///
/// The reservation grows by doubling (capped at `max_body`), so a large upload
/// takes a handful of budget updates, not one per frame. Every frame must
/// arrive within `idle` of the previous one and the whole body within `total`.
async fn read_body<B>(
    mut body: B,
    max_body: usize,
    reservation: &mut crate::interpreter::builtins::body_limit::BodyReservation,
    idle: Duration,
    total: Duration,
) -> Result<Bytes, BodyReadError>
where
    B: hyper::body::Body<Data = Bytes> + Unpin,
{
    let deadline = tokio::time::Instant::now() + total;
    // A body that arrives as one frame (most JSON and form posts) is kept as
    // that frame's `Bytes`, with no copy; only a second frame starts a buffer.
    let mut first: Option<Bytes> = None;
    let mut buffer: Option<bytes::BytesMut> = None;
    let mut received: usize = 0;
    loop {
        let frame_deadline = std::cmp::min(deadline, tokio::time::Instant::now() + idle);
        let frame = match tokio::time::timeout_at(frame_deadline, body.frame()).await {
            Err(_) => return Err(BodyReadError::TimedOut),
            Ok(None) => break,
            Ok(Some(Err(_))) => return Err(BodyReadError::Transport),
            Ok(Some(Ok(frame))) => frame,
        };
        // Trailers carry no body bytes.
        let Ok(data) = frame.into_data() else {
            continue;
        };
        if data.is_empty() {
            continue;
        }
        received = received.saturating_add(data.len());
        if received > max_body {
            return Err(BodyReadError::TooLarge);
        }
        if received > reservation.bytes() {
            let grown = reservation
                .bytes()
                .saturating_mul(2)
                .max(received)
                .min(max_body);
            if !reservation.try_grow_to(grown) {
                return Err(BodyReadError::Busy);
            }
        }
        if let Some(buf) = buffer.as_mut() {
            buf.extend_from_slice(&data);
        } else if let Some(previous) = first.take() {
            let mut buf = bytes::BytesMut::with_capacity(received);
            buf.extend_from_slice(&previous);
            buf.extend_from_slice(&data);
            buffer = Some(buf);
        } else {
            first = Some(data);
        }
    }
    Ok(match (buffer, first) {
        (Some(buf), _) => buf.freeze(),
        (None, Some(only)) => only,
        (None, None) => Bytes::new(),
    })
}

/// Signalled once per request a worker takes off the queue, so an [`enqueue`]
/// that found the queue full wakes when there is room instead of polling.
///
/// Per application: each tenant has its own worker pool and its own queue, and
/// a wakeup for one tenant's free slot handed to a request waiting on another
/// tenant's full queue would be a wakeup lost for the first. The async side
/// reads it from inside the request's tenant scope; workers are bound to their
/// tenant for life (`tenant::bind_current`).
static QUEUE_SPACE: super::tenant::TenantValue<std::sync::Arc<tokio::sync::Notify>> =
    super::tenant::TenantValue::new(new_queue_space_signal);

fn new_queue_space_signal() -> std::sync::Arc<tokio::sync::Notify> {
    std::sync::Arc::new(tokio::sync::Notify::new())
}

/// How many [`enqueue`] calls, across every tenant, are waiting for a slot.
///
/// Lets a worker skip the notify — and the tenant lookup in front of it — on
/// every dequeue while nobody is waiting, which is all the time the queue is
/// not full.
static QUEUE_WAITERS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Holds one count in [`QUEUE_WAITERS`] for as long as an [`enqueue`] waits,
/// however that wait ends (success, timeout, or the request future dropped).
struct QueueWaiter;

impl QueueWaiter {
    fn register() -> Self {
        QUEUE_WAITERS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        QueueWaiter
    }
}

impl Drop for QueueWaiter {
    fn drop(&mut self) {
        QUEUE_WAITERS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Called by a worker thread each time it takes a request off the queue: a
/// slot is free, so wake one [`enqueue`] waiting for it.
///
/// Callable from a plain thread — `Notify::notify_one` needs no runtime. The
/// fence pairs with the one in [`enqueue`] (a Dekker handshake): either this
/// load sees the waiter's registration, or the waiter's retry sees the slot
/// this dequeue freed. Never neither, so a wakeup cannot be lost between the
/// waiter's failed `try_send` and its wait.
pub(super) fn queue_slot_freed() {
    std::sync::atomic::fence(std::sync::atomic::Ordering::SeqCst);
    if QUEUE_WAITERS.load(std::sync::atomic::Ordering::Relaxed) > 0 {
        QUEUE_SPACE.read(|space| space.notify_one());
    }
}

fn queue_busy_response() -> EarlyResponse {
    Box::new(
        Response::builder()
            .status(StatusCode::SERVICE_UNAVAILABLE)
            .body(full(Bytes::from("Server busy")))
            .unwrap(),
    )
}

/// Hand a request to the worker pool, or the 503 to send instead.
///
/// Shared with the dev bar's request replay, which used to carry its own copy
/// of this loop under a comment saying it mirrored this one.
pub(super) async fn enqueue(
    request_tx: &WorkerSender,
    data: RequestData,
) -> Result<(), EarlyResponse> {
    // Non-blocking send: try_send, and await room when the queue is full.
    // Blocking send() here would deadlock under high concurrency because:
    // - Full queues block tokio worker threads on send()
    // - Workers' Handle::block_on() futures need the tokio I/O driver to complete
    // - Blocked tokio threads can't drive the I/O driver → permanent deadlock
    //
    // The wait used to be a 1 ms sleep-and-retry loop: under sustained
    // overload every queued request woke a thousand times a second to find the
    // queue still full. It now parks on `QUEUE_SPACE`, which a worker signals
    // per dequeue (`queue_slot_freed`).
    let mut data = match request_tx.try_send(data) {
        Ok(()) => return Ok(()),
        Err(crossbeam::channel::TrySendError::Full(returned)) => returned,
        Err(crossbeam::channel::TrySendError::Disconnected(_)) => return Err(queue_busy_response()),
    };

    let deadline =
        tokio::time::Instant::now() + Duration::from_secs(server_constants::REQUEST_TIMEOUT_SECS);
    let space = QUEUE_SPACE.read(|space| space.clone());
    let _waiting = QueueWaiter::register();
    loop {
        // Register interest *before* the retry: a slot freed between a failed
        // `try_send` and the wait then still reaches this waiter (or leaves a
        // permit that completes the wait at once). The fence is this side of
        // the handshake described on `queue_slot_freed`.
        let notified = space.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        std::sync::atomic::fence(std::sync::atomic::Ordering::SeqCst);

        match request_tx.try_send(data) {
            Ok(()) => return Ok(()),
            Err(crossbeam::channel::TrySendError::Full(returned)) => data = returned,
            Err(crossbeam::channel::TrySendError::Disconnected(_)) => {
                return Err(queue_busy_response())
            }
        }

        let now = tokio::time::Instant::now();
        if now >= deadline {
            return Err(queue_busy_response());
        }
        // The signal is the wakeup; the recheck interval is only a floor on
        // how stale a missed one could leave this wait. A woken waiter whose
        // slot was taken by a newer arrival simply waits again. Dropping a
        // `Notified` that was signalled but not yet polled passes the wakeup
        // on to the next waiter (tokio forwards `notify_one`), so a timeout
        // racing a signal loses nothing either.
        let wake_by = std::cmp::min(
            deadline,
            now + Duration::from_millis(server_constants::QUEUE_SPACE_RECHECK_MS),
        );
        let _ = tokio::time::timeout_at(wake_by, notified).await;
    }
}

/// Wait for the worker's reply, or the response to send instead.
///
/// Bounded by `RESPONSE_WAIT_TIMEOUT_SECS`. The worker
/// reply is otherwise awaited with no timeout: a worker parked in a
/// blocking DB/HTTP call or a lock would hang this request forever
/// ("pending" in the browser, system idle). On timeout we free the
/// connection with a 504 and log which route stalled. Dropping
/// `response_rx` here is safe — the worker's reply send is a discarded
/// `let _ = ...send(...)`, so it won't panic on a closed receiver.
pub(super) async fn await_worker(
    response_rx: oneshot::Receiver<WorkerResponse>,
    method: &str,
    path: &str,
    request_start: Instant,
) -> Result<WorkerResponse, EarlyResponse> {
    match tokio::time::timeout(
        Duration::from_secs(server_constants::RESPONSE_WAIT_TIMEOUT_SECS),
        response_rx,
    )
    .await
    {
        Ok(Ok(worker_response)) => Ok(worker_response),
        Err(_) => {
            eprintln!(
                "[WARN] layer=lang_serve method={} path={} timeout_secs={} elapsed_ms={} \
                 worker response timed out; returning 504",
                method,
                path,
                server_constants::RESPONSE_WAIT_TIMEOUT_SECS,
                request_start.elapsed().as_millis(),
            );
            Err(Box::new(
                Response::builder()
                    .status(StatusCode::GATEWAY_TIMEOUT)
                    .header("Server", "soliMVC")
                    .body(full(Bytes::from("Gateway Timeout")))
                    .unwrap(),
            ))
        }
        Ok(Err(_)) => Err(Box::new(
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(full(Bytes::from("Internal Server Error")))
                .unwrap(),
        )),
    }
}

/// Turn the worker's reply into the HTTP response.
///
/// `if_none_match` and `is_prefetch` come from [`Intake`]: the request headers
/// they were read from are the worker's by now. `live_reload` is whether the
/// server has a reload channel at all — the dev-only script injection below
/// is gated on it.
pub(super) fn assemble(
    worker_response: WorkerResponse,
    if_none_match: Option<&str>,
    is_prefetch: bool,
    dev_mode: bool,
    live_reload: bool,
) -> Response<ResponseBody> {
    // Streaming responses (SSE / chunked) bypass the buffered path
    // entirely: build a chunked body fed by the worker's channel.
    let resp_data = match worker_response {
        WorkerResponse::Stream {
            status,
            headers,
            rx,
        } => {
            let stream = tokio_stream::wrappers::ReceiverStream::new(rx)
                .map(|chunk| Ok::<_, std::io::Error>(hyper::body::Frame::data(Bytes::from(chunk))));
            let body = BodyExt::boxed(StreamBody::new(stream));
            let mut builder = Response::builder()
                .status(StatusCode::from_u16(status).unwrap_or(StatusCode::OK))
                .header("Server", "soliMVC");
            for (key, value) in &headers {
                builder = builder.header(key, value);
            }
            return builder
                .body(body)
                .unwrap_or_else(|_| Response::new(full(Bytes::from("stream init error"))));
        }
        WorkerResponse::Buffered(rd) => rd,
    };
    // Conditional-GET short-circuit: if the controller produced an
    // ETag matching the browser's If-None-Match, return 304 with
    // just the validator headers. Enables the hover-prefetch feature
    // to deliver "instant navigation" — the body is already in the
    // prefetched-resources cache; revalidation costs one tiny round
    // trip instead of re-sending tens of KB of HTML.
    //
    // Skipped in --dev: the dev bar is injected after the ETag is
    // computed, so a 304 would replay an HTML snapshot with stale
    // bar contents (old timings, old query log, old req counter).
    if !dev_mode {
        if let Some(client_etag) = if_none_match {
            if let Some(server_etag) = resp_data.headers.iter().find_map(|(k, v)| {
                if k.eq_ignore_ascii_case("etag") {
                    Some(v.as_str())
                } else {
                    None
                }
            }) {
                fn strip_weak(s: &str) -> &str {
                    s.trim_start_matches("W/").trim()
                }
                if strip_weak(client_etag) == strip_weak(server_etag) {
                    let mut b304 = Response::builder()
                        .status(StatusCode::NOT_MODIFIED)
                        .header("Server", "soliMVC");
                    // RFC 7232 §4.1: 304 MUST include the ETag it validated
                    // against and SHOULD include Cache-Control so the
                    // browser knows the freshness semantics for the next
                    // reuse.
                    for (key, value) in &resp_data.headers {
                        if key.eq_ignore_ascii_case("etag")
                            || key.eq_ignore_ascii_case("cache-control")
                            || key.eq_ignore_ascii_case("vary")
                        {
                            b304 = add_header_checked(b304, key.as_str(), value.as_str());
                        }
                    }
                    return finish_response(b304, Bytes::new());
                }
            }
        }
    }

    let mut builder = Response::builder()
        .status(StatusCode::from_u16(resp_data.status).unwrap_or(StatusCode::OK))
        .header("Server", "soliMVC");

    // For a speculative prefetch of an HTML page, swap the page's
    // `private, no-cache` for a short `private, max-age=N` so the click
    // serves the prefetched bytes straight from the browser cache — no
    // conditional GET, so a CDN that won't relay a 304 (Cloudflare et
    // al.) can't break instant navigation. The ETag still rides along
    // for revalidation once the window lapses.
    let prefetch_cache_control = if is_prefetch
        && resp_data
            .headers
            .iter()
            .any(|(k, v)| k.eq_ignore_ascii_case("content-type") && v.contains("text/html"))
    {
        Some(crate::serve::prefetch::prefetch_cache_control())
    } else {
        None
    };

    let no_inject = resp_data
        .headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case(NO_INJECT_HEADER));
    for (key, value) in &resp_data.headers {
        if key.eq_ignore_ascii_case(NO_INJECT_HEADER) {
            continue;
        }
        if let Some(ref cache_control) = prefetch_cache_control {
            if key.eq_ignore_ascii_case("cache-control") {
                builder = add_header_checked(builder, key.as_str(), cache_control.as_str());
                continue;
            }
        }
        builder = add_header_checked(builder, key.as_str(), value.as_str());
    }

    // Inject live reload script for HTML responses (only in dev mode).
    // HTML is UTF-8, so we can safely view the body as &str for injection.
    // Binary responses (images/files) skip this path via the content-type guard.
    let body: Vec<u8> = if live_reload && !no_inject {
        let is_html = resp_data
            .headers
            .iter()
            .any(|(k, v)| k.eq_ignore_ascii_case("content-type") && v.contains("text/html"));
        if is_html {
            match std::str::from_utf8(&resp_data.body) {
                Ok(html) => live_reload::inject_live_reload_script(html).into_bytes(),
                Err(_) => resp_data.body,
            }
        } else {
            resp_data.body
        }
    } else {
        resp_data.body
    };

    finish_response(builder, Bytes::from(body))
}

#[cfg(test)]
mod read_body_tests {
    use super::*;
    use crate::interpreter::builtins::body_limit::tests::BUDGET_LOCK;
    use crate::interpreter::builtins::body_limit::BodyReservation;
    use futures_util::stream;
    use hyper::body::Frame;

    type FrameResult = Result<Frame<Bytes>, std::convert::Infallible>;

    fn frames(chunks: &[&'static str]) -> Vec<FrameResult> {
        chunks
            .iter()
            .map(|c| Ok(Frame::data(Bytes::from_static(c.as_bytes()))))
            .collect()
    }

    /// Run `read_body` on a private runtime while holding the budget lock:
    /// the in-flight counter is process-global, and the lock must not be held
    /// across an `.await`.
    fn read<B>(body: B, max_body: usize, idle: Duration) -> (Result<Bytes, BodyReadError>, usize)
    where
        B: hyper::body::Body<Data = Bytes> + Unpin,
    {
        let _serial = BUDGET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("test runtime");
        let mut reservation = BodyReservation::try_acquire(4).expect("tiny first slice");
        let outcome = runtime.block_on(read_body(
            body,
            max_body,
            &mut reservation,
            idle,
            Duration::from_secs(30),
        ));
        (outcome, reservation.bytes())
    }

    /// A chunked body is reassembled in order, and the budget it holds tracks
    /// what actually arrived rather than the per-request cap.
    #[test]
    fn a_chunked_body_is_charged_for_what_arrives() {
        let body = StreamBody::new(stream::iter(frames(&["hello ", "chunked ", "world"])));
        let (outcome, reserved) = read(body, 1024, Duration::from_secs(5));
        let bytes = outcome.expect("body reads");
        assert_eq!(&bytes[..], b"hello chunked world");
        assert!(reserved >= bytes.len());
        assert!(reserved <= 1024, "never more than the cap");
    }

    #[test]
    fn a_body_over_the_cap_is_refused_mid_stream() {
        let body = StreamBody::new(stream::iter(frames(&["0123456789", "0123456789"])));
        let (outcome, _) = read(body, 15, Duration::from_secs(5));
        assert_eq!(outcome.unwrap_err(), BodyReadError::TooLarge);
    }

    /// A body that stops sending is dropped after the idle timeout, well
    /// before the total one.
    #[test]
    fn a_stalled_body_times_out_on_the_idle_clock() {
        let body = StreamBody::new(
            stream::iter(frames(&["partial"])).chain(stream::pending::<FrameResult>()),
        );
        let started = std::time::Instant::now();
        let (outcome, _) = read(body, 1024, Duration::from_millis(50));
        assert_eq!(outcome.unwrap_err(), BodyReadError::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}

#[cfg(test)]
mod enqueue_tests {
    use super::*;
    use crate::serve::worker_pool::WorkerQueues;

    fn request_data() -> RequestData {
        let (response_tx, _rx) = oneshot::channel();
        RequestData {
            method: Cow::Borrowed("POST"),
            path: "/queued".to_string(),
            query: Vec::new(),
            headers: hyper::HeaderMap::new(),
            body: String::new(),
            body_reservation: None,
            multipart_form: None,
            multipart_files: None,
            peer_ip: "127.0.0.1".to_string(),
            enqueued_at: None,
            replay: false,
            file_template: None,
            response_tx,
        }
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("test runtime")
    }

    /// A request that finds the queue full is woken by the worker's dequeue
    /// signal, not by the recheck floor: on a single-threaded runtime the
    /// waiter completes as soon as it is scheduled after the signal, well
    /// inside the recheck interval.
    #[test]
    fn a_full_queue_wakes_the_waiter_when_a_worker_dequeues() {
        let queues = WorkerQueues::new(1, 1);
        let sender = queues.get_sender();
        let receiver = queues.get_receiver(0);
        runtime().block_on(async move {
            assert!(
                enqueue(&sender, request_data()).await.is_ok(),
                "fills the queue"
            );
            let waiting = {
                let sender = sender.clone();
                tokio::spawn(async move { enqueue(&sender, request_data()).await.is_ok() })
            };
            // Let the waiter find the queue full and park on the signal.
            for _ in 0..4 {
                tokio::task::yield_now().await;
            }
            assert!(!waiting.is_finished(), "the queue is still full");

            // What a worker does: take one off the queue, then signal.
            let _taken = receiver.try_recv().expect("the queued request");
            queue_slot_freed();

            let woke = tokio::time::timeout(
                Duration::from_millis(server_constants::QUEUE_SPACE_RECHECK_MS / 2),
                waiting,
            )
            .await
            .expect("woken by the signal, not the recheck")
            .expect("task ran");
            assert!(woke, "the waiter's request went into the freed slot");
            assert!(receiver.try_recv().is_ok());
        });
    }

    /// A queue nobody drains any more answers 503 at once rather than waiting
    /// out the timeout.
    #[test]
    fn a_disconnected_queue_is_refused_immediately() {
        let queues = WorkerQueues::new(1, 1);
        let sender = queues.get_sender();
        drop(queues); // the last receiver
        let refused = runtime().block_on(enqueue(&sender, request_data()));
        let response = refused.expect_err("nobody is draining");
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}

#[cfg(test)]
mod body_budget_client_tests {
    use super::*;
    use crate::interpreter::builtins::trust_proxy::TRUST_PROXY_ENABLED;
    use crate::serve::tenant;

    fn forwarded(value: &str) -> hyper::header::HeaderMap {
        let mut headers = hyper::header::HeaderMap::new();
        headers.insert("x-forwarded-for", value.parse().unwrap());
        headers
    }

    /// Run in a tenant of its own, so flipping trust-proxy here cannot race
    /// another test reading the primary tenant's flag.
    fn with_trust_proxy<R>(on: bool, f: impl FnOnce() -> R) -> R {
        let id = tenant::register(std::env::temp_dir());
        tenant::scoped(id, || {
            TRUST_PROXY_ENABLED.write(|enabled| *enabled = on);
            f()
        })
    }

    #[test]
    fn without_trust_proxy_the_peer_is_the_client() {
        let peer: std::net::IpAddr = "192.0.2.1".parse().unwrap();
        let client = with_trust_proxy(false, || {
            body_budget_client(peer, &forwarded("203.0.113.9"))
        });
        assert_eq!(client, peer, "a spoofed header must not pick the bucket");
    }

    #[test]
    fn behind_a_trusted_proxy_the_rightmost_forwarded_entry_is_the_client() {
        let peer: std::net::IpAddr = "10.0.0.2".parse().unwrap();
        let (client, garbage, absent) = with_trust_proxy(true, || {
            (
                body_budget_client(peer, &forwarded("1.1.1.1, 203.0.113.9 ")),
                body_budget_client(peer, &forwarded("not-an-ip")),
                body_budget_client(peer, &hyper::header::HeaderMap::new()),
            )
        });
        assert_eq!(client, "203.0.113.9".parse::<std::net::IpAddr>().unwrap());
        assert_eq!(garbage, peer);
        assert_eq!(absent, peer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;

    fn buffered(status: u16, headers: &[(&str, &str)], body: &str) -> WorkerResponse {
        WorkerResponse::Buffered(super::super::ResponseData {
            status,
            headers: headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            body: body.as_bytes().to_vec(),
        })
    }

    fn header_of(response: &Response<ResponseBody>, name: &str) -> Option<String> {
        response
            .headers()
            .get(name)
            .map(|v| v.to_str().unwrap().to_string())
    }

    async fn body_of(response: Response<ResponseBody>) -> String {
        String::from_utf8(
            response
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn a_buffered_reply_keeps_its_status_headers_and_body() {
        let response = assemble(
            buffered(
                201,
                &[("Content-Type", "text/plain"), ("X-Own", "1")],
                "made",
            ),
            None,
            false,
            false,
            false,
        );
        assert_eq!(response.status(), StatusCode::CREATED);
        assert_eq!(header_of(&response, "server").as_deref(), Some("soliMVC"));
        assert_eq!(header_of(&response, "x-own").as_deref(), Some("1"));
        assert_eq!(body_of(response).await, "made");
    }

    /// RFC 7232 §4.1: the 304 carries the validators and nothing else — no
    /// body, and none of the other headers the 200 would have had.
    #[tokio::test]
    async fn a_matching_etag_is_answered_304_with_only_the_validators() {
        let response = assemble(
            buffered(
                200,
                &[
                    ("ETag", "\"v1\""),
                    ("Cache-Control", "private, no-cache"),
                    ("Vary", "Accept-Encoding"),
                    ("Content-Type", "text/html"),
                ],
                "<html><body>big</body></html>",
            ),
            Some("\"v1\""),
            false,
            false,
            false,
        );
        assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(header_of(&response, "etag").as_deref(), Some("\"v1\""));
        assert_eq!(
            header_of(&response, "cache-control").as_deref(),
            Some("private, no-cache")
        );
        assert_eq!(
            header_of(&response, "vary").as_deref(),
            Some("Accept-Encoding")
        );
        assert!(header_of(&response, "content-type").is_none());
        assert_eq!(body_of(response).await, "");
    }

    /// A proxy in front may weaken either side's validator, so the comparison
    /// strips `W/` from both before comparing.
    #[test]
    fn the_weak_form_of_an_etag_still_validates() {
        for (client, server) in [
            ("W/\"v1\"", "\"v1\""),
            ("\"v1\"", "W/\"v1\""),
            ("W/\"v1\"", "W/\"v1\""),
        ] {
            let response = assemble(
                buffered(200, &[("ETag", server)], "body"),
                Some(client),
                false,
                false,
                false,
            );
            assert_eq!(
                response.status(),
                StatusCode::NOT_MODIFIED,
                "{client} against {server}"
            );
        }
        let response = assemble(
            buffered(200, &[("ETag", "\"v2\"")], "body"),
            Some("\"v1\""),
            false,
            false,
            false,
        );
        assert_eq!(response.status(), StatusCode::OK);
    }

    /// Under `--dev` the dev bar is spliced in after the ETag is computed, so
    /// a 304 would send the browser back to an HTML snapshot whose bar shows
    /// the timings of some earlier request.
    #[test]
    fn dev_mode_never_short_circuits_to_304() {
        let response = assemble(
            buffered(200, &[("ETag", "\"v1\"")], "body"),
            Some("\"v1\""),
            false,
            true,
            false,
        );
        assert_eq!(response.status(), StatusCode::OK);
    }

    /// A speculative prefetch of an HTML page gets a short `max-age` in place
    /// of the page's `no-cache`, so the click is served from the browser
    /// cache without a conditional GET a CDN might not relay.
    #[test]
    fn a_prefetched_html_page_gets_a_cacheable_window() {
        let response = assemble(
            buffered(
                200,
                &[
                    ("Content-Type", "text/html; charset=utf-8"),
                    ("Cache-Control", "private, no-cache"),
                ],
                "<html></html>",
            ),
            None,
            true,
            false,
            false,
        );
        let cache_control = header_of(&response, "cache-control").unwrap();
        assert_ne!(cache_control, "private, no-cache");
        assert_eq!(
            cache_control,
            crate::serve::prefetch::prefetch_cache_control()
        );
    }

    /// Only HTML: a prefetched JSON endpoint keeps whatever the controller
    /// said about caching.
    #[test]
    fn a_prefetch_of_something_that_is_not_html_is_left_alone() {
        let response = assemble(
            buffered(
                200,
                &[
                    ("Content-Type", "application/json"),
                    ("Cache-Control", "private, no-cache"),
                ],
                "{}",
            ),
            None,
            true,
            false,
            false,
        );
        assert_eq!(
            header_of(&response, "cache-control").as_deref(),
            Some("private, no-cache")
        );
    }

    /// The marker is an instruction to this function, not something a client
    /// should ever see.
    #[tokio::test]
    async fn the_no_inject_marker_suppresses_the_splice_and_never_ships() {
        let response = assemble(
            buffered(
                200,
                &[("Content-Type", "text/html"), (NO_INJECT_HEADER, "1")],
                "<html><body>compiled in</body></html>",
            ),
            None,
            false,
            true,
            true,
        );
        assert!(header_of(&response, NO_INJECT_HEADER).is_none());
        assert_eq!(
            body_of(response).await,
            "<html><body>compiled in</body></html>"
        );
    }

    #[tokio::test]
    async fn live_reload_splices_html_and_only_html() {
        let html = assemble(
            buffered(
                200,
                &[("Content-Type", "text/html")],
                "<html><body>x</body></html>",
            ),
            None,
            false,
            true,
            true,
        );
        let spliced = body_of(html).await;
        assert!(spliced.starts_with("<html><body>x"), "{spliced}");
        assert!(spliced.ends_with("</body></html>"), "{spliced}");
        assert!(spliced.contains("<script"), "{spliced}");

        let json = assemble(
            buffered(200, &[("Content-Type", "application/json")], "{\"a\":1}"),
            None,
            false,
            true,
            true,
        );
        assert_eq!(body_of(json).await, "{\"a\":1}");

        let off = assemble(
            buffered(
                200,
                &[("Content-Type", "text/html")],
                "<html><body>x</body></html>",
            ),
            None,
            false,
            true,
            false,
        );
        assert_eq!(body_of(off).await, "<html><body>x</body></html>");
    }
}
