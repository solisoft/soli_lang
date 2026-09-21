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
use http_body_util::{BodyExt, Limited, StreamBody};
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
/// instead — 413 for an oversize body, 503 when the in-flight body budget is
/// exhausted, 408 for one that never finishes arriving.
pub(super) async fn intake(
    req: Request<Incoming>,
    method: Cow<'static, str>,
    raw_query: Option<&str>,
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
    // mid-stream by `Limited`.
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
        // many connections each collecting their own body. A chunked request
        // declares no length, so it reserves the per-request cap — the most it
        // could turn out to be.
        let want = declared_content_length.map_or(max_body, |n| n.min(max_body));
        match crate::interpreter::builtins::body_limit::BodyReservation::try_acquire(want) {
            Some(reservation) => body_reservation = Some(reservation),
            None => {
                return Err(Box::new(
                    Response::builder()
                        .status(StatusCode::SERVICE_UNAVAILABLE)
                        .header("Content-Type", "text/plain; charset=utf-8")
                        .header("Retry-After", "1")
                        .body(full(Bytes::from("Server busy: too many uploads in flight")))
                        .unwrap(),
                ));
            }
        }
    }
    let (body, multipart_form, multipart_files) = if method == "GET" || method == "HEAD" {
        (String::new(), None, None)
    } else {
        // Bounded in time as well as size: `Limited` caps how much can
        // arrive, not how long it may take, so a trickled body held a
        // connection and its buffer indefinitely.
        let collected = match tokio::time::timeout(
            Duration::from_secs(server_constants::body_read_timeout_secs()),
            BodyExt::collect(Limited::new(req_body, max_body)),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => {
                return Err(Box::new(
                    Response::builder()
                        .status(StatusCode::REQUEST_TIMEOUT)
                        .header("Content-Type", "text/plain; charset=utf-8")
                        .body(full(Bytes::from("Request body read timed out")))
                        .unwrap(),
                ));
            }
        };
        // Keep the collected body as `Bytes`. It is refcounted, so the
        // multipart parser can take it by value without copying, and the
        // non-multipart branch only borrows it to build the String. The
        // previous `.to_vec()` was a full extra copy of every request body.
        let body_bytes = match collected {
            Ok(b) => b.to_bytes(),
            Err(_) => {
                // `Limited` returns an error once the running total
                // crosses `max_body`. Treat any failure here as oversize:
                // we can't reliably distinguish a transport error from a
                // length-limit hit, but in either case we don't want to
                // proceed with a partial body.
                return Err(Box::new(
                    Response::builder()
                        .status(StatusCode::PAYLOAD_TOO_LARGE)
                        .header("Content-Type", "text/plain; charset=utf-8")
                        .body(full(Bytes::from("Request body too large")))
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
                let body_str = String::from_utf8_lossy(&body_bytes).to_string();
                (body_str, None, None)
            }
        } else {
            let body_str = String::from_utf8_lossy(&body_bytes).to_string();
            (body_str, None, None)
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

/// Hand a request to the worker pool, or the 503 to send instead.
///
/// Shared with the dev bar's request replay, which used to carry its own copy
/// of this loop under a comment saying it mirrored this one.
pub(super) async fn enqueue(
    request_tx: &WorkerSender,
    data: RequestData,
) -> Result<(), EarlyResponse> {
    // Non-blocking send: use try_send + async yield to avoid blocking tokio threads.
    // Blocking send() here would deadlock under high concurrency because:
    // - Full queues block tokio worker threads on send()
    // - Workers' Handle::block_on() futures need the tokio I/O driver to complete
    // - Blocked tokio threads can't drive the I/O driver → permanent deadlock
    let mut pending_data = Some(data);
    let deadline =
        tokio::time::Instant::now() + Duration::from_secs(server_constants::REQUEST_TIMEOUT_SECS);
    let send_ok = loop {
        if let Some(data) = pending_data.take() {
            match request_tx.try_send(data) {
                Ok(()) => break true,
                Err(crossbeam::channel::TrySendError::Full(returned)) => {
                    if tokio::time::Instant::now() >= deadline {
                        break false;
                    }
                    pending_data = Some(returned);
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
                Err(crossbeam::channel::TrySendError::Disconnected(_)) => {
                    break false;
                }
            }
        }
    };

    if !send_ok {
        return Err(Box::new(
            Response::builder()
                .status(StatusCode::SERVICE_UNAVAILABLE)
                .body(full(Bytes::from("Server busy")))
                .unwrap(),
        ));
    }

    Ok(())
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
