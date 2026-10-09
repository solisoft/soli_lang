//! Streaming a SoliDB blob straight to the client, with HTTP `Range`.
//!
//! `solidb_get_blob` hands Soli the whole blob as base64, which is fine for a
//! thumbnail and ruinous for a 150 MB podcast episode: the worker holds the
//! bytes, the base64 copy and the decoded body at once, per download, and an
//! audio player that seeks gets the whole file again for every seek.
//!
//! `solidb_blob_response(client, collection, blob_id, req, headers)` answers
//! instead with a response hash that only *describes* the download. It runs on
//! the worker and does no I/O: it resolves the blob URL and credentials,
//! decides which `Range` to forward, and parks a [`BlobStreamSpec`] in a
//! thread-local, the way `stream(req) do ... end` parks its block. The hash it
//! returns carries [`MARKER_HEADER`] with the spec's token, so the spec is only
//! honoured when the hash the action returned (after any `after_action` and
//! response middleware) is still that response — an action that builds the
//! response and then returns something else gets what it returned.
//!
//! The worker then replies with `WorkerResponse::Blob`, and the request's async
//! task — not the worker — opens `GET /_api/blob/{db}/{collection}/{key}` and
//! pipes SoliDB's body into hyper one frame at a time. Nothing is buffered:
//! the upstream body is polled only when hyper asks for the next frame, so a
//! slow client slows the read from SoliDB (TCP back-pressure end to end) and
//! the memory a download costs is one chunk, whatever the blob's size. A
//! client that hangs up drops the body, which drops the upstream response and
//! closes that connection. The worker is free as soon as the spec is built.
//!
//! SoliDB answers a single `Range: bytes=…` with `206` + `Content-Range`, an
//! unsatisfiable one with `416`, anything else with the whole blob (`200`).
//! Status, `Content-Range`, `Content-Length` and `Accept-Ranges` are taken from
//! that answer; the representation headers (`Content-Type`,
//! `Content-Disposition`, `nosniff`, `Cache-Control`) are the application's.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

use bytes::Bytes;
use futures_util::StreamExt;
use hyper::{Response, StatusCode};

use crate::interpreter::value::{HashKey, HashPairs, StrKey, Value};

use super::{add_header_checked, full, ResponseBody};

/// Internal header tying a response hash to the spec parked for it. Stripped
/// before anything reaches the client.
pub(crate) const MARKER_HEADER: &str = "x-soli-blob-stream";

/// How long SoliDB may take to answer with its response headers.
const UPSTREAM_HEAD_TIMEOUT: Duration = Duration::from_secs(30);

/// How long one chunk of the body may take to arrive from SoliDB. Only the
/// wait on SoliDB counts: time spent waiting for a slow client to take the
/// previous chunk does not, so a paused audio player is not cut off.
const UPSTREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// What the async side needs to fetch and relay one blob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BlobStreamSpec {
    /// Matches the [`MARKER_HEADER`] value of the response it belongs to.
    pub(crate) token: u64,
    /// `{base}/_api/blob/{db}/{collection}/{key}`, every segment encoded.
    pub(crate) url: String,
    /// The same credentials every other `Solidb` call sends, as one header.
    pub(crate) auth: Option<(String, String)>,
    /// The client's `Range`, normalised, when it is a single byte range whose
    /// `If-Range` (if any) still matches. `None` asks for the whole blob.
    pub(crate) range: Option<String>,
    /// A `HEAD` request: send headers only, never the body.
    pub(crate) head: bool,
}

thread_local! {
    static PENDING: RefCell<Option<BlobStreamSpec>> = const { RefCell::new(None) };
}

static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

/// Forget a spec left over from an earlier request on this worker.
pub(crate) fn clear_pending() {
    PENDING.with(|p| *p.borrow_mut() = None);
}

/// Take the spec this request parked, if any.
pub(crate) fn take_pending() -> Option<BlobStreamSpec> {
    PENDING.with(|p| p.borrow_mut().take())
}

fn set_pending(spec: BlobStreamSpec) {
    PENDING.with(|p| *p.borrow_mut() = Some(spec));
}

/// Remove every [`MARKER_HEADER`] from a finished response, and hand back the
/// spec only when the response still carries that spec's token.
pub(crate) fn claim(
    spec: Option<BlobStreamSpec>,
    headers: &mut Vec<(String, String)>,
) -> Option<BlobStreamSpec> {
    let mut token = None;
    headers.retain(|(name, value)| {
        if name.eq_ignore_ascii_case(MARKER_HEADER) {
            token = value.trim().parse::<u64>().ok();
            return false;
        }
        true
    });
    spec.filter(|spec| token == Some(spec.token))
}

// ---------------------------------------------------------------------------
// Request side: which Range to forward
// ---------------------------------------------------------------------------

/// Normalise a `Range` header to `bytes=…` when it asks for exactly one byte
/// range: `bytes=a-b`, `bytes=a-` or `bytes=-n`.
///
/// Anything else — another unit, several ranges, a malformed or reversed one —
/// is `None`, and the blob goes out whole: RFC 9110 lets a server ignore a
/// Range it does not serve, and SoliDB only serves a single one.
pub(crate) fn single_byte_range(raw: &str) -> Option<String> {
    let (unit, spec) = raw.trim().split_once('=')?;
    if !unit.trim().eq_ignore_ascii_case("bytes") {
        return None;
    }
    let spec = spec.trim();
    let (first, last) = spec.split_once('-')?;
    let number = |s: &str| -> Option<u64> {
        if s.is_empty() || s.len() > 19 || !s.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        s.parse().ok()
    };
    match (first.is_empty(), last.is_empty()) {
        // `bytes=-` says nothing.
        (true, true) => None,
        (true, false) => Some(format!("bytes=-{}", number(last)?)),
        (false, true) => Some(format!("bytes={}-", number(first)?)),
        (false, false) => {
            let (a, b) = (number(first)?, number(last)?);
            (a <= b).then(|| format!("bytes={a}-{b}"))
        }
    }
}

/// The Range to send SoliDB, after `If-Range`.
///
/// `If-Range` makes a Range conditional on the representation being the one
/// the client already has part of. The only validator this response carries
/// is its strong `ETag`, so a matching tag keeps the Range and anything else
/// — another tag, a weak one, or a date — drops it and the client gets the
/// whole blob, which is what RFC 9110 §13.1.5 asks for.
pub(crate) fn forwarded_range(
    range: Option<&str>,
    if_range: Option<&str>,
    etag: Option<&str>,
) -> Option<String> {
    let range = single_byte_range(range?)?;
    match if_range.map(str::trim) {
        None => Some(range),
        Some(validator) => (Some(validator) == etag).then_some(range),
    }
}

/// A strong ETag for a blob.
///
/// A blob key names one set of bytes for good — replacing an attachment stores
/// a new blob under a new key — so the key itself is a correct validator.
/// `None` when the key holds a character an entity tag cannot carry.
pub(crate) fn etag_for(blob_id: &str) -> Option<String> {
    let valid = !blob_id.is_empty()
        && blob_id
            .bytes()
            .all(|b| b.is_ascii_graphic() && b != b'"' && b != b'\\');
    valid.then(|| format!("\"{blob_id}\""))
}

/// `If-None-Match` against our ETag, with the weak comparison RFC 9110 §13.1.2
/// prescribes for it.
pub(crate) fn if_none_match_hits(header: &str, etag: &str) -> bool {
    let ours = etag.trim_start_matches("W/");
    header
        .split(',')
        .map(str::trim)
        .any(|candidate| candidate == "*" || candidate.trim_start_matches("W/") == ours)
}

// ---------------------------------------------------------------------------
// The builtin's body: build the response hash and park the spec
// ---------------------------------------------------------------------------

fn hash_str(hash: &Value, key: &str) -> Option<String> {
    match hash {
        Value::Hash(pairs) => match pairs.borrow().get(&StrKey(key)) {
            Some(Value::String(s)) => Some(s.to_string()),
            _ => None,
        },
        _ => None,
    }
}

fn hash_field(hash: &Value, key: &str) -> Option<Value> {
    match hash {
        Value::Hash(pairs) => pairs.borrow().get(&StrKey(key)).cloned(),
        _ => None,
    }
}

/// Headers the relay sets from SoliDB's answer, never from the application.
fn is_framing_header(name: &str) -> bool {
    [
        "content-length",
        "content-range",
        "accept-ranges",
        "transfer-encoding",
        MARKER_HEADER,
    ]
    .iter()
    .any(|framing| name.eq_ignore_ascii_case(framing))
}

fn header_pairs(headers: Vec<(String, String)>) -> Value {
    let mut pairs = HashPairs::default();
    for (name, value) in headers {
        pairs.insert(HashKey::String(name.into()), Value::String(value.into()));
    }
    Value::Hash(std::rc::Rc::new(RefCell::new(pairs)))
}

fn response_hash(status: i64, headers: Vec<(String, String)>) -> Value {
    let mut pairs = HashPairs::default();
    pairs.insert(HashKey::String("status".into()), Value::Int(status));
    pairs.insert(HashKey::String("headers".into()), header_pairs(headers));
    pairs.insert(HashKey::String("body".into()), Value::String("".into()));
    Value::Hash(std::rc::Rc::new(RefCell::new(pairs)))
}

/// Build the response hash for `solidb_blob_response` and park its spec.
///
/// `req` is the request hash (`null` reads as a plain `GET`); `extra` is the
/// application's headers hash. A conditional `GET` whose `If-None-Match`
/// matches is answered `304` here, without touching SoliDB.
pub(crate) fn blob_response(
    url: String,
    auth: Option<(String, String)>,
    blob_id: &str,
    req: &Value,
    extra: Option<&Value>,
) -> Result<Value, String> {
    let method = hash_str(req, "method").unwrap_or_else(|| "GET".to_string());
    let req_headers = hash_field(req, "headers").unwrap_or(Value::Null);
    let header = |name: &str| hash_str(&req_headers, name);

    let mut headers: Vec<(String, String)> = Vec::new();
    match extra {
        None | Some(Value::Null) => {}
        Some(Value::Hash(pairs)) => {
            for (key, value) in pairs.borrow().iter() {
                let HashKey::String(name) = key else { continue };
                if is_framing_header(name) {
                    continue;
                }
                let value = match value {
                    Value::String(s) => s.to_string(),
                    Value::Int(n) => n.to_string(),
                    Value::Null => continue,
                    other => other.to_string(),
                };
                headers.push((name.to_string(), value));
            }
        }
        Some(other) => {
            return Err(format!(
                "solidb_blob_response() expects a headers hash, got {}",
                other.type_name()
            ))
        }
    }

    let etag = etag_for(blob_id);
    if let Some(tag) = etag.as_ref() {
        if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("etag")) {
            headers.push(("ETag".to_string(), tag.clone()));
        }
        if header("if-none-match").is_some_and(|inm| if_none_match_hits(&inm, tag)) {
            // RFC 9110 §15.4.5: a 304 carries the validator and the caching
            // headers, not the representation's.
            headers.retain(|(k, _)| {
                k.eq_ignore_ascii_case("etag")
                    || k.eq_ignore_ascii_case("cache-control")
                    || k.eq_ignore_ascii_case("vary")
            });
            return Ok(response_hash(304, headers));
        }
    }

    let range = forwarded_range(
        header("range").as_deref(),
        header("if-range").as_deref(),
        etag.as_deref(),
    );
    let token = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed);
    set_pending(BlobStreamSpec {
        token,
        url,
        auth,
        range,
        head: method.eq_ignore_ascii_case("HEAD"),
    });
    headers.push((MARKER_HEADER.to_string(), token.to_string()));
    Ok(response_hash(200, headers))
}

// ---------------------------------------------------------------------------
// Response side: relay SoliDB's answer
// ---------------------------------------------------------------------------

/// What to send for one upstream answer.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Mapped {
    pub(crate) status: u16,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: MappedBody,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum MappedBody {
    /// Relay SoliDB's body.
    Upstream,
    /// Headers only (`HEAD`, `416`).
    Empty,
    /// A short plain-text error.
    Text(&'static str),
}

/// `bytes 0-0/1234` or `bytes */1234` → `1234`.
fn complete_length(content_range: &str) -> Option<u64> {
    content_range.rsplit_once('/')?.1.trim().parse().ok()
}

/// An error answer keeps the application's non-representation headers
/// (cookies, security headers) and loses the blob's.
fn error_headers(mut headers: Vec<(String, String)>) -> Vec<(String, String)> {
    headers.retain(|(k, _)| {
        ![
            "content-type",
            "content-disposition",
            "etag",
            "cache-control",
            "last-modified",
        ]
        .iter()
        .any(|h| k.eq_ignore_ascii_case(h))
    });
    headers.push((
        "Content-Type".to_string(),
        "text/plain; charset=utf-8".to_string(),
    ));
    headers.push(("Cache-Control".to_string(), "no-store".to_string()));
    headers
}

/// Map SoliDB's status and headers onto the response to send.
///
/// `app` is the application's headers (marker already gone). `upstream` reads
/// one of SoliDB's response headers by lowercase name. `probe` is set when a
/// `HEAD` without a Range asked SoliDB for `bytes=0-0` to learn the size
/// without its body; the answer is then reported as the whole blob's.
pub(crate) fn map_upstream(
    mut app: Vec<(String, String)>,
    status: u16,
    upstream: &dyn Fn(&str) -> Option<String>,
    head: bool,
    probe: bool,
) -> Mapped {
    app.retain(|(k, _)| !is_framing_header(k));
    let has = |headers: &[(String, String)], name: &str| {
        headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(name))
    };

    match status {
        200 | 206 | 416 if probe => {
            // HEAD: the probe's Content-Range carries the complete length.
            let total = if status == 200 {
                upstream("content-length")
            } else {
                upstream("content-range")
                    .and_then(|cr| complete_length(&cr))
                    .map(|n| n.to_string())
            };
            let mut headers = app;
            if !has(&headers, "content-type") {
                let ct = upstream("content-type")
                    .unwrap_or_else(|| "application/octet-stream".to_string());
                headers.push(("Content-Type".to_string(), ct));
            }
            // A 206 or 416 to a Range means SoliDB serves ranges.
            let accept =
                upstream("accept-ranges").or_else(|| (status != 200).then(|| "bytes".into()));
            if let Some(accept) = accept {
                headers.push(("Accept-Ranges".to_string(), accept));
            }
            if let Some(total) = total {
                headers.push(("Content-Length".to_string(), total));
            }
            Mapped {
                status: 200,
                headers,
                body: MappedBody::Empty,
            }
        }
        200 | 206 => {
            let mut headers = app;
            if !has(&headers, "content-type") {
                let ct = upstream("content-type")
                    .unwrap_or_else(|| "application/octet-stream".to_string());
                headers.push(("Content-Type".to_string(), ct));
            }
            if let Some(accept) = upstream("accept-ranges") {
                headers.push(("Accept-Ranges".to_string(), accept));
            }
            if status == 206 {
                if let Some(cr) = upstream("content-range") {
                    headers.push(("Content-Range".to_string(), cr));
                }
            }
            if let Some(len) = upstream("content-length") {
                headers.push(("Content-Length".to_string(), len));
            }
            Mapped {
                status,
                headers,
                body: if head {
                    MappedBody::Empty
                } else {
                    MappedBody::Upstream
                },
            }
        }
        416 => {
            let mut headers = error_headers(app);
            if let Some(cr) = upstream("content-range") {
                headers.push(("Content-Range".to_string(), cr));
            }
            headers.push(("Accept-Ranges".to_string(), "bytes".to_string()));
            Mapped {
                status: 416,
                headers,
                body: MappedBody::Empty,
            }
        }
        404 => Mapped {
            status: 404,
            headers: error_headers(app),
            body: MappedBody::Text("Not found"),
        },
        _ => Mapped {
            status: 502,
            headers: error_headers(app),
            body: MappedBody::Text("Bad Gateway"),
        },
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// The client blob downloads go through.
///
/// Not the shared DB client: that one caps every request, body included, at
/// 10 s, which would cut a podcast download off mid-episode. This one has no
/// total timeout — the relay times the connect, the response headers and each
/// chunk itself — and never follows a redirect with the credentials on it.
fn blob_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(build_client)
}

#[cfg(not(target_arch = "wasm32"))]
fn build_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .tcp_keepalive(Duration::from_secs(60))
        .pool_idle_timeout(Duration::from_secs(90))
        .redirect(reqwest::redirect::Policy::none())
        .min_tls_version(reqwest::tls::Version::TLS_1_2)
        .build()
        .expect("Failed to create the blob download client")
}

#[cfg(not(target_arch = "wasm32"))]
/// Fetch the blob described by `spec` and relay it under `headers` (the
/// application's response headers, marker removed).
pub(crate) async fn respond(
    headers: Vec<(String, String)>,
    spec: BlobStreamSpec,
) -> Response<ResponseBody> {
    respond_with(blob_client(), headers, spec).await
}

#[cfg(not(target_arch = "wasm32"))]
/// [`respond`] with a given client — tests pass one bound to their runtime.
pub(crate) async fn respond_with(
    client: &reqwest::Client,
    headers: Vec<(String, String)>,
    spec: BlobStreamSpec,
) -> Response<ResponseBody> {
    let probe = spec.head && spec.range.is_none();
    let mut request = client.get(&spec.url);
    if let Some((name, value)) = &spec.auth {
        request = request.header(name.as_str(), value.as_str());
    }
    let range = if probe {
        Some("bytes=0-0".to_string())
    } else {
        spec.range.clone()
    };
    if let Some(range) = range {
        request = request.header("Range", range);
    }

    let upstream = match tokio::time::timeout(UPSTREAM_HEAD_TIMEOUT, request.send()).await {
        Ok(Ok(response)) => response,
        Ok(Err(e)) => {
            eprintln!(
                "[soli] blob stream: SoliDB request failed: {}",
                e.without_url()
            );
            return build(
                Mapped {
                    status: 502,
                    headers: error_headers(headers),
                    body: MappedBody::Text("Bad Gateway"),
                },
                None,
            );
        }
        Err(_) => {
            eprintln!(
                "[soli] blob stream: SoliDB sent no response within {}s",
                UPSTREAM_HEAD_TIMEOUT.as_secs()
            );
            return build(
                Mapped {
                    status: 504,
                    headers: error_headers(headers),
                    body: MappedBody::Text("Gateway Timeout"),
                },
                None,
            );
        }
    };

    let status = upstream.status().as_u16();
    if !matches!(status, 200 | 206 | 404 | 416) {
        eprintln!("[soli] blob stream: SoliDB answered {status}; replying 502");
    }
    let mapped = {
        let up_headers = upstream.headers();
        let read = |name: &str| {
            up_headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        };
        map_upstream(headers, status, &read, spec.head, probe)
    };
    let body = (mapped.body == MappedBody::Upstream).then_some(upstream);
    build(mapped, body)
}

#[cfg(not(target_arch = "wasm32"))]
fn build(mapped: Mapped, upstream: Option<reqwest::Response>) -> Response<ResponseBody> {
    let mut builder = Response::builder()
        .status(StatusCode::from_u16(mapped.status).unwrap_or(StatusCode::BAD_GATEWAY))
        .header("Server", "soliMVC");
    for (name, value) in &mapped.headers {
        builder = add_header_checked(builder, name, value);
    }
    let body = match (mapped.body, upstream) {
        (MappedBody::Upstream, Some(upstream)) => relay_body(upstream),
        (MappedBody::Text(text), _) => full(Bytes::from_static(text.as_bytes())),
        _ => full(Bytes::new()),
    };
    builder.body(body).unwrap_or_else(|_| {
        Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(full(Bytes::from_static(b"Internal Server Error")))
            .expect("static 500 response is always valid")
    })
}

/// SoliDB's body as a hyper body, pulled one chunk per frame hyper asks for.
///
/// `unfold` only polls SoliDB when hyper polls the body, which it does once
/// the previous frame is written — that is the back-pressure. The idle timer
/// wraps only the wait on SoliDB. An error ends the body early, and hyper
/// then aborts the connection rather than pretend a short body was complete.
#[cfg(not(target_arch = "wasm32"))]
fn relay_body(upstream: reqwest::Response) -> ResponseBody {
    let chunks = upstream.bytes_stream();
    let frames = futures_util::stream::unfold(Some(chunks), |state| async move {
        let mut chunks = state?;
        match tokio::time::timeout(UPSTREAM_IDLE_TIMEOUT, chunks.next()).await {
            Ok(Some(Ok(bytes))) => Some((Ok(hyper::body::Frame::data(bytes)), Some(chunks))),
            Ok(None) => None,
            Ok(Some(Err(e))) => {
                eprintln!(
                    "[soli] blob stream: SoliDB body failed: {}",
                    e.without_url()
                );
                Some((Err(std::io::Error::other("upstream body failed")), None))
            }
            Err(_) => {
                eprintln!(
                    "[soli] blob stream: SoliDB stalled for {}s; aborting",
                    UPSTREAM_IDLE_TIMEOUT.as_secs()
                );
                Some((
                    Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "upstream body stalled",
                    )),
                    None,
                ))
            }
        }
    });
    http_body_util::BodyExt::boxed(http_body_util::StreamBody::new(frames))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    fn hash(pairs: &[(&str, Value)]) -> Value {
        let mut h = HashPairs::default();
        for (k, v) in pairs {
            h.insert(HashKey::String((*k).into()), v.clone());
        }
        Value::Hash(Rc::new(RefCell::new(h)))
    }

    fn s(v: &str) -> Value {
        Value::String(v.into())
    }

    fn request(method: &str, headers: &[(&str, &str)]) -> Value {
        let headers: Vec<(&str, Value)> = headers.iter().map(|(k, v)| (*k, s(v))).collect();
        hash(&[("method", s(method)), ("headers", hash(&headers))])
    }

    fn response_headers(response: &Value) -> Vec<(String, String)> {
        let headers = hash_field(response, "headers").expect("headers");
        let Value::Hash(pairs) = headers else {
            panic!("headers is not a hash")
        };
        let pairs = pairs.borrow();
        pairs
            .iter()
            .map(|(k, v)| match (k, v) {
                (HashKey::String(k), Value::String(v)) => (k.to_string(), v.to_string()),
                _ => panic!("non-string header"),
            })
            .collect()
    }

    fn get<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
        headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    // -- Range forwarding --------------------------------------------------

    #[test]
    fn a_single_byte_range_is_forwarded_normalised() {
        assert_eq!(
            single_byte_range("bytes=0-99").as_deref(),
            Some("bytes=0-99")
        );
        assert_eq!(
            single_byte_range("bytes=500-").as_deref(),
            Some("bytes=500-")
        );
        assert_eq!(
            single_byte_range("bytes=-500").as_deref(),
            Some("bytes=-500")
        );
        assert_eq!(
            single_byte_range(" Bytes = 7-7 ").as_deref(),
            Some("bytes=7-7")
        );
        // Unsatisfiable but well formed: SoliDB answers it with a 416.
        assert_eq!(single_byte_range("bytes=-0").as_deref(), Some("bytes=-0"));
    }

    #[test]
    fn anything_but_one_byte_range_is_dropped() {
        for raw in [
            "",
            "bytes=",
            "bytes=-",
            "bytes=0-9,20-29",
            "items=0-9",
            "bytes=9-0",
            "bytes=a-b",
            "bytes=0x10-",
            "bytes=+1-2",
            "bytes=99999999999999999999-",
            "0-99",
        ] {
            assert_eq!(
                single_byte_range(raw),
                None,
                "{raw:?} must not be forwarded"
            );
        }
    }

    #[test]
    fn if_range_keeps_the_range_only_for_our_strong_etag() {
        let etag = Some("\"blob42\"");
        let range = Some("bytes=10-");
        assert_eq!(
            forwarded_range(range, None, etag).as_deref(),
            Some("bytes=10-")
        );
        assert_eq!(
            forwarded_range(range, Some("\"blob42\""), etag).as_deref(),
            Some("bytes=10-")
        );
        // Another representation, a weak tag, or a date: send it all.
        assert_eq!(forwarded_range(range, Some("\"other\""), etag), None);
        assert_eq!(forwarded_range(range, Some("W/\"blob42\""), etag), None);
        assert_eq!(
            forwarded_range(range, Some("Wed, 21 Oct 2015 07:28:00 GMT"), etag),
            None
        );
        // No ETag to compare with: an If-Range can never match.
        assert_eq!(forwarded_range(range, Some("\"blob42\""), None), None);
        assert_eq!(forwarded_range(None, None, etag), None);
    }

    #[test]
    fn the_blob_key_is_the_etag_when_it_can_be_one() {
        assert_eq!(etag_for("abc-123_x").as_deref(), Some("\"abc-123_x\""));
        assert_eq!(etag_for(""), None);
        assert_eq!(etag_for("has space"), None);
        assert_eq!(etag_for("quo\"te"), None);
        assert_eq!(etag_for("é"), None);
    }

    #[test]
    fn if_none_match_uses_the_weak_comparison() {
        let etag = "\"k1\"";
        assert!(if_none_match_hits("\"k1\"", etag));
        assert!(if_none_match_hits("W/\"k1\"", etag));
        assert!(if_none_match_hits("\"a\", \"k1\"", etag));
        assert!(if_none_match_hits("*", etag));
        assert!(!if_none_match_hits("\"k2\"", etag));
    }

    // -- The builtin's response hash ---------------------------------------

    #[test]
    fn the_response_hash_parks_a_spec_and_carries_its_token() {
        clear_pending();
        let req = request("GET", &[("range", "bytes=100-199")]);
        let extra = hash(&[
            ("Content-Type", s("audio/mpeg")),
            ("Content-Disposition", s("inline")),
            // The relay owns framing: an application length would be wrong
            // for every 206.
            ("Content-Length", s("123")),
        ]);
        let response = blob_response(
            "http://db/_api/blob/d/c/k1".into(),
            Some(("Authorization".into(), "Basic eDp5".into())),
            "k1",
            &req,
            Some(&extra),
        )
        .expect("response hash");

        assert!(matches!(
            hash_field(&response, "status"),
            Some(Value::Int(200))
        ));
        let mut headers = response_headers(&response);
        assert_eq!(get(&headers, "content-type"), Some("audio/mpeg"));
        assert_eq!(get(&headers, "etag"), Some("\"k1\""));
        assert_eq!(get(&headers, "content-length"), None);

        let spec = take_pending().expect("a parked spec");
        assert_eq!(spec.url, "http://db/_api/blob/d/c/k1");
        assert_eq!(spec.range.as_deref(), Some("bytes=100-199"));
        assert_eq!(
            spec.auth,
            Some(("Authorization".to_string(), "Basic eDp5".to_string()))
        );
        assert!(!spec.head);
        assert_eq!(
            get(&headers, MARKER_HEADER),
            Some(spec.token.to_string().as_str())
        );

        let claimed = claim(Some(spec.clone()), &mut headers);
        assert_eq!(claimed, Some(spec));
        assert_eq!(
            get(&headers, MARKER_HEADER),
            None,
            "the marker never leaves"
        );
    }

    #[test]
    fn head_is_flagged_and_a_null_request_reads_as_get() {
        clear_pending();
        blob_response("u".into(), None, "k", &request("HEAD", &[]), None).unwrap();
        assert!(take_pending().unwrap().head);

        blob_response("u".into(), None, "k", &Value::Null, None).unwrap();
        let spec = take_pending().unwrap();
        assert!(!spec.head);
        assert_eq!(spec.range, None);
    }

    #[test]
    fn a_matching_if_none_match_is_a_304_without_a_spec() {
        clear_pending();
        let req = request("GET", &[("if-none-match", "\"k9\"")]);
        let extra = hash(&[
            ("Content-Type", s("video/mp4")),
            ("Cache-Control", s("private, max-age=300")),
        ]);
        let response = blob_response("u".into(), None, "k9", &req, Some(&extra)).unwrap();
        assert!(matches!(
            hash_field(&response, "status"),
            Some(Value::Int(304))
        ));
        let headers = response_headers(&response);
        assert_eq!(get(&headers, "etag"), Some("\"k9\""));
        assert_eq!(get(&headers, "cache-control"), Some("private, max-age=300"));
        assert_eq!(get(&headers, "content-type"), None);
        assert_eq!(get(&headers, MARKER_HEADER), None);
        assert_eq!(take_pending(), None, "nothing to fetch for a 304");
    }

    #[test]
    fn a_stale_if_range_fetches_the_whole_blob() {
        clear_pending();
        let req = request(
            "GET",
            &[("range", "bytes=0-1"), ("if-range", "\"an-older-blob\"")],
        );
        blob_response("u".into(), None, "k", &req, None).unwrap();
        assert_eq!(take_pending().unwrap().range, None);
    }

    #[test]
    fn headers_must_be_a_hash() {
        let err = blob_response("u".into(), None, "k", &Value::Null, Some(&s("nope")));
        assert!(err.unwrap_err().contains("headers hash"));
    }

    #[test]
    fn a_spec_is_used_only_by_the_response_carrying_its_token() {
        let spec = BlobStreamSpec {
            token: 7,
            url: "u".into(),
            auth: None,
            range: None,
            head: false,
        };
        // The action returned some other response: no marker.
        let mut other = vec![("Content-Type".to_string(), "text/html".to_string())];
        assert_eq!(claim(Some(spec.clone()), &mut other), None);
        // A marker for another token (or forged by hand) is dropped too.
        let mut forged = vec![(MARKER_HEADER.to_string(), "8".to_string())];
        assert_eq!(claim(Some(spec.clone()), &mut forged), None);
        assert!(forged.is_empty());
        let mut stray = vec![("X-Soli-Blob-Stream".to_string(), "7".to_string())];
        assert_eq!(claim(None, &mut stray), None);
        assert!(stray.is_empty());
    }

    // -- Mapping SoliDB's answer -------------------------------------------

    fn app() -> Vec<(String, String)> {
        vec![
            ("Content-Type".into(), "audio/mpeg".into()),
            ("Content-Disposition".into(), "inline".into()),
            ("X-Content-Type-Options".into(), "nosniff".into()),
            ("Cache-Control".into(), "private, max-age=300".into()),
            ("ETag".into(), "\"k\"".into()),
            ("Set-Cookie".into(), "s=1".into()),
            ("Content-Length".into(), "999".into()),
        ]
    }

    fn upstream(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |name: &str| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn a_full_answer_passes_length_and_accept_ranges() {
        let up = upstream(&[
            ("content-length", "5000000"),
            ("accept-ranges", "bytes"),
            ("content-type", "application/octet-stream"),
        ]);
        let m = map_upstream(app(), 200, &up, false, false);
        assert_eq!(m.status, 200);
        assert_eq!(m.body, MappedBody::Upstream);
        assert_eq!(get(&m.headers, "content-length"), Some("5000000"));
        assert_eq!(get(&m.headers, "accept-ranges"), Some("bytes"));
        // The application's type wins over SoliDB's.
        assert_eq!(get(&m.headers, "content-type"), Some("audio/mpeg"));
        assert_eq!(get(&m.headers, "content-disposition"), Some("inline"));
        assert_eq!(get(&m.headers, "set-cookie"), Some("s=1"));
        assert_eq!(
            m.headers
                .iter()
                .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                .count(),
            1,
            "the application's own Content-Length is dropped"
        );
    }

    #[test]
    fn a_partial_answer_passes_content_range() {
        let up = upstream(&[
            ("content-length", "100"),
            ("content-range", "bytes 100-199/5000000"),
            ("accept-ranges", "bytes"),
        ]);
        let m = map_upstream(app(), 206, &up, false, false);
        assert_eq!(m.status, 206);
        assert_eq!(
            get(&m.headers, "content-range"),
            Some("bytes 100-199/5000000")
        );
        assert_eq!(get(&m.headers, "content-length"), Some("100"));
        assert_eq!(m.body, MappedBody::Upstream);
    }

    #[test]
    fn an_unsatisfiable_range_is_a_bodiless_416() {
        let up = upstream(&[("content-range", "bytes */5000000")]);
        let m = map_upstream(app(), 416, &up, false, false);
        assert_eq!(m.status, 416);
        assert_eq!(m.body, MappedBody::Empty);
        assert_eq!(get(&m.headers, "content-range"), Some("bytes */5000000"));
        assert_eq!(get(&m.headers, "content-disposition"), None);
        assert_eq!(get(&m.headers, "x-content-type-options"), Some("nosniff"));
    }

    #[test]
    fn a_missing_blob_is_404_and_anything_else_is_502() {
        let none = upstream(&[]);
        let m = map_upstream(app(), 404, &none, false, false);
        assert_eq!((m.status, m.body), (404, MappedBody::Text("Not found")));
        assert_eq!(
            get(&m.headers, "content-type"),
            Some("text/plain; charset=utf-8")
        );
        assert_eq!(get(&m.headers, "etag"), None);
        for status in [401, 403, 500, 503, 302] {
            let m = map_upstream(app(), status, &none, false, false);
            assert_eq!(m.status, 502, "SoliDB {status}");
            assert_eq!(m.body, MappedBody::Text("Bad Gateway"));
        }
    }

    #[test]
    fn head_sends_the_headers_without_the_body() {
        let up = upstream(&[
            ("content-length", "100"),
            ("content-range", "bytes 0-99/400"),
        ]);
        let m = map_upstream(app(), 206, &up, true, false);
        assert_eq!(m.status, 206);
        assert_eq!(m.body, MappedBody::Empty);
        assert_eq!(get(&m.headers, "content-length"), Some("100"));
    }

    #[test]
    fn a_head_probe_reports_the_whole_blob() {
        // We asked for bytes=0-0; the client asked for the whole thing.
        let up = upstream(&[
            ("content-length", "1"),
            ("content-range", "bytes 0-0/5000000"),
            ("accept-ranges", "bytes"),
        ]);
        let m = map_upstream(app(), 206, &up, true, true);
        assert_eq!(m.status, 200);
        assert_eq!(m.body, MappedBody::Empty);
        assert_eq!(get(&m.headers, "content-length"), Some("5000000"));
        assert_eq!(get(&m.headers, "content-range"), None);
        assert_eq!(get(&m.headers, "accept-ranges"), Some("bytes"));

        // A SoliDB that ignores Range answers 200 with the real length.
        let up = upstream(&[("content-length", "5000000")]);
        let m = map_upstream(app(), 200, &up, true, true);
        assert_eq!((m.status, m.body), (200, MappedBody::Empty));
        assert_eq!(get(&m.headers, "content-length"), Some("5000000"));
        assert_eq!(get(&m.headers, "accept-ranges"), None);

        // An empty blob has no byte 0.
        let up = upstream(&[("content-range", "bytes */0")]);
        let m = map_upstream(app(), 416, &up, true, true);
        assert_eq!(m.status, 200);
        assert_eq!(get(&m.headers, "content-length"), Some("0"));
    }

    // -- Against a stand-in SoliDB -----------------------------------------

    use http_body_util::BodyExt;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::sync::Notify;

    const MB: usize = 1024 * 1024;

    fn blob(len: usize) -> Arc<Vec<u8>> {
        Arc::new((0..len).map(|i| (i % 251) as u8).collect())
    }

    /// A SoliDB blob endpoint honouring one `Range`: 206 + Content-Range,
    /// 416 when unsatisfiable, 200 otherwise; `/missing` is a 404. The body
    /// goes out 64 KiB at a time, and when `gate` is set the rest waits on it
    /// after the first chunk. Records each request head it reads.
    struct StandIn {
        base: String,
        heads: Arc<Mutex<Vec<String>>>,
    }

    async fn stand_in(data: Arc<Vec<u8>>, gate: Option<Arc<Notify>>) -> StandIn {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let heads = Arc::new(Mutex::new(Vec::new()));
        let seen = heads.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut conn, _)) = listener.accept().await else {
                    return;
                };
                let (data, gate, seen) = (data.clone(), gate.clone(), seen.clone());
                tokio::spawn(async move {
                    let mut head = Vec::new();
                    let mut byte = [0u8; 1];
                    while !head.ends_with(b"\r\n\r\n") {
                        if conn.read(&mut byte).await.unwrap_or(0) == 0 {
                            return;
                        }
                        head.push(byte[0]);
                    }
                    let head = String::from_utf8_lossy(&head).to_string();
                    seen.lock().unwrap().push(head.clone());
                    let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
                    let range = head
                        .lines()
                        .find_map(|l| {
                            l.strip_prefix("range: ")
                                .or_else(|| l.strip_prefix("Range: "))
                        })
                        .map(str::to_string);
                    let len = data.len();
                    let (status, extra, span) = if path.ends_with("/missing") {
                        ("404 Not Found", String::new(), None)
                    } else {
                        match range.as_deref().and_then(|r| {
                            crate::serve::server_constants::parse_range_header(r, len as u64)
                        }) {
                            Some((a, b)) => (
                                "206 Partial Content",
                                format!("Content-Range: bytes {a}-{b}/{len}\r\n"),
                                Some((a as usize, b as usize + 1)),
                            ),
                            None if range.is_some() => (
                                "416 Range Not Satisfiable",
                                format!("Content-Range: bytes */{len}\r\n"),
                                None,
                            ),
                            None => ("200 OK", String::new(), Some((0, len))),
                        }
                    };
                    let body: &[u8] = match span {
                        Some((a, b)) => &data[a..b],
                        None => b"",
                    };
                    let reply = format!(
                        "HTTP/1.1 {status}\r\nContent-Type: application/octet-stream\r\n\
                         Accept-Ranges: bytes\r\n{extra}Content-Length: {}\r\n\
                         Connection: close\r\n\r\n",
                        body.len()
                    );
                    if conn.write_all(reply.as_bytes()).await.is_err() {
                        return;
                    }
                    for (i, chunk) in body.chunks(64 * 1024).enumerate() {
                        if i == 1 {
                            if let Some(gate) = &gate {
                                gate.notified().await;
                            }
                        }
                        if conn.write_all(chunk).await.is_err() {
                            return;
                        }
                    }
                    let _ = conn.shutdown().await;
                });
            }
        });
        StandIn { base, heads }
    }

    fn spec(url: String, range: Option<&str>, head: bool) -> BlobStreamSpec {
        BlobStreamSpec {
            token: 1,
            url,
            auth: Some(("Authorization".into(), "Basic dTpw".into())),
            range: range.map(str::to_string),
            head,
        }
    }

    fn app_headers() -> Vec<(String, String)> {
        vec![
            ("Content-Type".into(), "audio/mpeg".into()),
            ("Content-Disposition".into(), "inline".into()),
            ("X-Content-Type-Options".into(), "nosniff".into()),
        ]
    }

    /// The whole body, and how many frames it came in.
    async fn drain(response: Response<ResponseBody>) -> (Vec<u8>, usize) {
        let mut body = response.into_body();
        let (mut bytes, mut frames) = (Vec::new(), 0);
        while let Some(frame) = body.frame().await {
            if let Ok(data) = frame.expect("body frame").into_data() {
                bytes.extend_from_slice(&data);
                frames += 1;
            }
        }
        (bytes, frames)
    }

    fn header(response: &Response<ResponseBody>, name: &str) -> Option<String> {
        response
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    }

    #[tokio::test]
    async fn streams_a_multi_megabyte_blob_whole_and_by_range() {
        let data = blob(5 * MB + 123);
        let solidb = stand_in(data.clone(), None).await;
        let client = build_client();
        let url = format!("{}/_api/blob/db/media/ep1", solidb.base);

        let whole = respond_with(&client, app_headers(), spec(url.clone(), None, false)).await;
        assert_eq!(whole.status(), 200);
        assert_eq!(
            header(&whole, "content-length"),
            Some(data.len().to_string())
        );
        assert_eq!(header(&whole, "accept-ranges").as_deref(), Some("bytes"));
        assert_eq!(
            header(&whole, "content-type").as_deref(),
            Some("audio/mpeg")
        );
        assert_eq!(
            header(&whole, "content-disposition").as_deref(),
            Some("inline")
        );
        let (bytes, frames) = drain(whole).await;
        assert!(bytes == *data, "the whole blob, byte for byte");
        assert!(
            frames > 1,
            "relayed in chunks, not as one buffer ({frames} frame)"
        );

        let start = 3 * MB + 7;
        let end = start + 2 * MB;
        let range = format!("bytes={start}-{end}");
        let part = respond_with(
            &client,
            app_headers(),
            spec(url.clone(), Some(&range), false),
        )
        .await;
        assert_eq!(part.status(), 206);
        assert_eq!(
            header(&part, "content-range"),
            Some(format!("bytes {start}-{end}/{}", data.len()))
        );
        assert_eq!(
            header(&part, "content-length"),
            Some((end - start + 1).to_string())
        );
        let (bytes, _) = drain(part).await;
        assert!(bytes == data[start..=end], "exactly the requested span");

        let tail = respond_with(
            &client,
            app_headers(),
            spec(url.clone(), Some("bytes=-10"), false),
        )
        .await;
        assert_eq!(tail.status(), 206);
        assert!(drain(tail).await.0 == data[data.len() - 10..]);

        let past = format!("bytes={}-", data.len() + 1);
        let unsatisfiable = respond_with(
            &client,
            app_headers(),
            spec(url.clone(), Some(&past), false),
        )
        .await;
        assert_eq!(unsatisfiable.status(), 416);
        assert_eq!(
            header(&unsatisfiable, "content-range"),
            Some(format!("bytes */{}", data.len()))
        );
        assert!(drain(unsatisfiable).await.0.is_empty());

        // Every request carried the client's credentials and its Range only.
        let heads = solidb.heads.lock().unwrap().clone();
        assert_eq!(heads.len(), 4);
        for head in &heads {
            assert!(
                head.to_ascii_lowercase()
                    .contains("authorization: basic dtpw"),
                "{head}"
            );
        }
        assert!(
            !heads[0].to_ascii_lowercase().contains("range:"),
            "{}",
            heads[0]
        );
        assert!(heads[1]
            .to_ascii_lowercase()
            .contains(&format!("range: {range}")));
    }

    /// The first chunk reaches the client while SoliDB is still holding the
    /// rest back — a relay that buffered the body would never yield it.
    #[tokio::test]
    async fn the_first_chunk_arrives_before_solidb_finishes() {
        let data = blob(4 * MB);
        let gate = Arc::new(Notify::new());
        let solidb = stand_in(data.clone(), Some(gate.clone())).await;
        let client = build_client();
        let url = format!("{}/_api/blob/db/media/ep1", solidb.base);

        let response = respond_with(&client, app_headers(), spec(url, None, false)).await;
        assert_eq!(response.status(), 200);
        let mut body = response.into_body();
        let first = tokio::time::timeout(Duration::from_secs(5), body.frame())
            .await
            .expect("a frame while SoliDB is still sending")
            .expect("a frame")
            .expect("not an error")
            .into_data()
            .expect("data");
        assert!(!first.is_empty() && first.len() <= 64 * 1024);

        gate.notify_one();
        let mut rest = first.to_vec();
        while let Some(frame) = body.frame().await {
            if let Ok(chunk) = frame.unwrap().into_data() {
                rest.extend_from_slice(&chunk);
            }
        }
        assert!(rest == *data);
    }

    #[tokio::test]
    async fn head_probes_the_size_without_the_body() {
        let data = blob(2 * MB);
        let solidb = stand_in(data.clone(), None).await;
        let client = build_client();
        let url = format!("{}/_api/blob/db/media/ep1", solidb.base);

        let response = respond_with(&client, app_headers(), spec(url, None, true)).await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            header(&response, "content-length"),
            Some(data.len().to_string())
        );
        assert_eq!(header(&response, "accept-ranges").as_deref(), Some("bytes"));
        assert!(drain(response).await.0.is_empty());
        let heads = solidb.heads.lock().unwrap().clone();
        assert!(heads[0].to_ascii_lowercase().contains("range: bytes=0-0"));
    }

    #[tokio::test]
    async fn a_missing_blob_or_an_unreachable_solidb_is_an_error_status() {
        let solidb = stand_in(blob(10), None).await;
        let client = build_client();

        let missing = format!("{}/_api/blob/db/media/missing", solidb.base);
        let response = respond_with(&client, app_headers(), spec(missing, None, false)).await;
        assert_eq!(response.status(), 404);
        assert_eq!(drain(response).await.0, b"Not found");

        // Nothing listens on a port we just released.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let down = format!("http://127.0.0.1:{port}/_api/blob/db/media/k");
        let response = respond_with(&client, app_headers(), spec(down, None, false)).await;
        assert_eq!(response.status(), 502);
        assert_eq!(header(&response, "content-disposition"), None);
    }

    /// Against a real SoliDB, when one is configured (`SOLIDB_DRIVER_TEST_HOST`,
    /// as for the driver tests): store a multi-megabyte blob, then stream it
    /// back whole and by range through the relay. A server that predates Range
    /// support answers the range with the whole blob, which is still correct.
    #[test]
    fn against_a_real_solidb_when_configured() {
        let Some(host) = crate::platform::env::var("SOLIDB_DRIVER_TEST_HOST")
            .ok()
            .filter(|h| !h.is_empty())
        else {
            eprintln!("skip: SOLIDB_DRIVER_TEST_HOST unset (no SoliDB to stream from)");
            return;
        };
        let user = crate::platform::env::var("SOLIDB_USERNAME").unwrap_or_else(|_| "admin".into());
        let pass = crate::platform::env::var("SOLIDB_PASSWORD").unwrap_or_else(|_| "admin".into());
        let database = format!("soli_blob_stream_test_{}", std::process::id());

        let admin = crate::solidb_http::SoliDBClient::connect(&host)
            .unwrap()
            .with_basic_auth(&user, &pass);
        admin
            .create_database(&database)
            .expect("create test database");
        // `SoliDBClient::delete_database` targets the plural `/_api/databases/`
        // path, which SoliDB answers 404; the test runner drops through the
        // singular one, and so does this guard — on a failed assertion too.
        struct DropDatabase(String, String);
        impl Drop for DropDatabase {
            fn drop(&mut self) {
                let _ = ureq::delete(&self.0).set("Authorization", &self.1).call();
            }
        }
        let _cleanup = DropDatabase(
            format!("{}/_api/database/{}", admin.base_url(), database),
            {
                use base64::Engine as _;
                let token =
                    base64::engine::general_purpose::STANDARD.encode(format!("{user}:{pass}"));
                format!("Basic {token}")
            },
        );
        let mut client = crate::solidb_http::SoliDBClient::connect(&host)
            .unwrap()
            .with_basic_auth(&user, &pass);
        client.set_database(&database);
        client
            .create_collection("media", Some("blob"))
            .expect("create blob collection");
        let data = blob(3 * MB + 5);
        let key = client
            .store_blob("media", &data, "episode.mp3", "audio/mpeg")
            .expect("store blob");
        let (url, auth) = client.blob_download_target("media", &key).unwrap();

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let http = build_client();
            let mut whole_spec = spec(url.clone(), None, false);
            whole_spec.auth = auth.clone();
            let whole = respond_with(&http, app_headers(), whole_spec).await;
            assert_eq!(whole.status(), 200);
            let (bytes, _) = drain(whole).await;
            assert!(bytes == *data, "whole blob from SoliDB");

            let mut part_spec = spec(url.clone(), Some("bytes=1048576-1048675"), false);
            part_spec.auth = auth.clone();
            let part = respond_with(&http, app_headers(), part_spec).await;
            let status = part.status().as_u16();
            let (bytes, _) = drain(part).await;
            match status {
                206 => assert!(bytes == data[MB..MB + 100]),
                200 => {
                    eprintln!("note: this SoliDB ignores Range (answered 200)");
                    assert!(bytes == *data);
                }
                other => panic!("unexpected status {other} for a range"),
            }
        });
    }
}
