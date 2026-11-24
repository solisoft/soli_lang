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
//!
//! The same relay serves the other attachment services ([`Source`]): an `s3`
//! or `r2` blob is read through a presigned `GET` URL, which S3 answers with
//! the same `206`/`416`/`200`; a `disk` blob is read from its file, which
//! [`open_file`] answers the same way. `AttachmentsController#show` parks the
//! spec through `__soli_attachment_blob_response`.

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
    /// Where the stored bytes are read from.
    pub(crate) source: Source,
    /// The client's `Range`, normalised, when it is a single byte range whose
    /// `If-Range` (if any) still matches. `None` asks for the whole blob.
    pub(crate) range: Option<String>,
    /// A `HEAD` request: send headers only, never the body.
    pub(crate) head: bool,
    /// The blob is SBE-sealed (an `encrypted` attachment): open it segment by
    /// segment on the way out. `None` relays SoliDB's bytes as they are.
    pub(crate) sealed: Option<Sealed>,
}

/// Where a blob's stored bytes come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Source {
    /// An HTTP `GET` that honours one `Range`: SoliDB's blob API
    /// (`{base}/_api/blob/{db}/{collection}/{key}`, with the credentials every
    /// other `Solidb` call sends, as one header), or a presigned S3/R2 URL.
    Http {
        url: String,
        auth: Option<(String, String)>,
    },
    /// A file on this server's disk (the `disk` attachment service), read
    /// with the same semantics: `206`/`416` for a Range, `404` when absent.
    File(std::path::PathBuf),
}

/// How to open a sealed blob: the purpose it was sealed for and the keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Sealed {
    pub(crate) purpose: Vec<u8>,
    pub(crate) keys: Vec<crate::interpreter::builtins::blob_crypto::BlobKey>,
}

crate::request_local! {
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
    source: Source,
    blob_id: &str,
    req: &Value,
    extra: Option<&Value>,
    sealed: Option<Sealed>,
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
                "a blob response expects a headers hash, got {}",
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
        source,
        range,
        head: method.eq_ignore_ascii_case("HEAD"),
        sealed,
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
    mut spec: BlobStreamSpec,
) -> Response<ResponseBody> {
    match spec.sealed.take() {
        Some(sealed) => respond_sealed(client, headers, spec, sealed).await,
        None => respond_plain(client, headers, spec).await,
    }
}

// ---------------------------------------------------------------------------
// The store: SoliDB, an S3/R2 bucket, or this server's disk
// ---------------------------------------------------------------------------

/// The stored bytes as they arrive, whatever the source.
#[cfg(not(target_arch = "wasm32"))]
type Chunks =
    std::pin::Pin<Box<dyn futures_util::Stream<Item = Result<Bytes, String>> + Send + Sync>>;

/// How much of a file on disk one frame carries.
#[cfg(not(target_arch = "wasm32"))]
const FILE_CHUNK: u64 = 64 * 1024;

/// One answer from the store: its status, the headers the relay reads
/// (lowercase names), and its body.
#[cfg(not(target_arch = "wasm32"))]
struct Upstream {
    status: u16,
    headers: Vec<(String, String)>,
    chunks: Chunks,
}

#[cfg(not(target_arch = "wasm32"))]
impl Upstream {
    fn header(&self, name: &str) -> Option<String> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
    }

    fn bodiless(status: u16, headers: Vec<(String, String)>) -> Self {
        Upstream {
            status,
            headers,
            chunks: Box::pin(futures_util::stream::empty()),
        }
    }
}

/// One read of the stored blob with an optional Range. `Err` is the status
/// to answer with.
#[cfg(not(target_arch = "wasm32"))]
async fn fetch(
    client: &reqwest::Client,
    spec: &BlobStreamSpec,
    range: Option<String>,
) -> Result<Upstream, u16> {
    match &spec.source {
        Source::Http { url, auth } => fetch_http(client, url, auth.as_ref(), range).await,
        Source::File(path) => open_file(path, range.as_deref()).await,
    }
}

/// One GET under the head timeout. A presigned URL carries its signature in
/// the query string, so errors are logged without the URL.
#[cfg(not(target_arch = "wasm32"))]
async fn fetch_http(
    client: &reqwest::Client,
    url: &str,
    auth: Option<&(String, String)>,
    range: Option<String>,
) -> Result<Upstream, u16> {
    let mut request = client.get(url);
    if let Some((name, value)) = auth {
        request = request.header(name.as_str(), value.as_str());
    }
    if let Some(range) = range {
        request = request.header("Range", range);
    }
    let response = match tokio::time::timeout(UPSTREAM_HEAD_TIMEOUT, request.send()).await {
        Ok(Ok(response)) => response,
        Ok(Err(e)) => {
            eprintln!(
                "[soli] blob stream: the store request failed: {}",
                e.without_url()
            );
            return Err(502);
        }
        Err(_) => {
            eprintln!(
                "[soli] blob stream: the store sent no response within {}s",
                UPSTREAM_HEAD_TIMEOUT.as_secs()
            );
            return Err(504);
        }
    };
    let headers = [
        "content-range",
        "content-length",
        "content-type",
        "accept-ranges",
    ]
    .iter()
    .filter_map(|name| {
        response
            .headers()
            .get(*name)
            .and_then(|v| v.to_str().ok())
            .map(|v| (name.to_string(), v.to_string()))
    })
    .collect();
    Ok(Upstream {
        status: response.status().as_u16(),
        headers,
        chunks: Box::pin(
            response
                .bytes_stream()
                .map(|chunk| chunk.map_err(|e| e.without_url().to_string())),
        ),
    })
}

/// A file on disk answered the way SoliDB answers: `206` + `Content-Range`
/// for a Range, `416` + `bytes */len` when no byte of it is there, `200`
/// otherwise, `404` when the file is gone. The body is read `FILE_CHUNK`
/// bytes at a time, each read only when the previous frame was taken.
#[cfg(not(target_arch = "wasm32"))]
async fn open_file(path: &std::path::Path, range: Option<&str>) -> Result<Upstream, u16> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};

    let mut file = match tokio::fs::File::open(path).await {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Upstream::bodiless(404, Vec::new()))
        }
        Err(e) => {
            eprintln!("[soli] blob stream: cannot open a stored file: {e}");
            return Err(500);
        }
    };
    let len = match file.metadata().await {
        Ok(meta) => meta.len(),
        Err(e) => {
            eprintln!("[soli] blob stream: cannot stat a stored file: {e}");
            return Err(500);
        }
    };
    let mut headers = vec![("accept-ranges".to_string(), "bytes".to_string())];
    let (status, start, count) = match range {
        None => (200, 0, len),
        Some(range) => match resolve_range(range, len) {
            Some((a, b)) => {
                headers.push(("content-range".to_string(), format!("bytes {a}-{b}/{len}")));
                (206, a, b - a + 1)
            }
            None => {
                headers.push(("content-range".to_string(), format!("bytes */{len}")));
                return Ok(Upstream::bodiless(416, headers));
            }
        },
    };
    headers.push(("content-length".to_string(), count.to_string()));
    if start > 0 {
        if let Err(e) = file.seek(std::io::SeekFrom::Start(start)).await {
            eprintln!("[soli] blob stream: cannot seek in a stored file: {e}");
            return Err(500);
        }
    }
    let chunks = futures_util::stream::unfold(Some((file, count)), |state| async move {
        let (mut file, left) = state?;
        if left == 0 {
            return None;
        }
        let mut buf = vec![0u8; left.min(FILE_CHUNK) as usize];
        match file.read(&mut buf).await {
            Ok(0) => Some((Err("the stored file ended early".to_string()), None)),
            Ok(n) => {
                buf.truncate(n);
                Some((Ok(Bytes::from(buf)), Some((file, left - n as u64))))
            }
            Err(e) => Some((Err(e.to_string()), None)),
        }
    });
    Ok(Upstream {
        status,
        headers,
        chunks: Box::pin(chunks),
    })
}

/// A short error answer.
#[cfg(not(target_arch = "wasm32"))]
fn error_response(headers: Vec<(String, String)>, status: u16) -> Response<ResponseBody> {
    let text = match status {
        404 => "Not found",
        500 => "Internal Server Error",
        504 => "Gateway Timeout",
        _ => "Bad Gateway",
    };
    build(
        Mapped {
            status,
            headers: error_headers(headers),
            body: MappedBody::Text(text),
        },
        None,
    )
}

#[cfg(not(target_arch = "wasm32"))]
async fn respond_plain(
    client: &reqwest::Client,
    headers: Vec<(String, String)>,
    spec: BlobStreamSpec,
) -> Response<ResponseBody> {
    let probe = spec.head && spec.range.is_none();
    let range = if probe {
        Some("bytes=0-0".to_string())
    } else {
        spec.range.clone()
    };
    let upstream = match fetch(client, &spec, range).await {
        Ok(upstream) => upstream,
        Err(status) => return error_response(headers, status),
    };
    let status = upstream.status;
    if !matches!(status, 200 | 206 | 404 | 416) {
        eprintln!("[soli] blob stream: the store answered {status}; replying 502");
    }
    let mapped = map_upstream(
        headers,
        status,
        &|name| upstream.header(name),
        spec.head,
        probe,
    );
    let body = (mapped.body == MappedBody::Upstream).then_some(upstream);
    build(mapped, body)
}

// ---------------------------------------------------------------------------
// Sealed blobs: open segment by segment on the way out
// ---------------------------------------------------------------------------

/// The plaintext span a normalised Range (see [`single_byte_range`]) asks for
/// in a file of `len` bytes, or `None` when no byte of it is there (`416`).
/// A suffix longer than the file is the whole file (RFC 9110 §14.1.3).
pub(crate) fn resolve_range(range: &str, len: u64) -> Option<(u64, u64)> {
    let spec = range.strip_prefix("bytes=")?;
    let (first, last) = spec.split_once('-')?;
    if len == 0 {
        return None;
    }
    if first.is_empty() {
        let suffix: u64 = last.parse().ok()?;
        return (suffix > 0).then(|| (len - suffix.min(len), len - 1));
    }
    let start: u64 = first.parse().ok()?;
    if start >= len {
        return None;
    }
    let end = if last.is_empty() {
        len - 1
    } else {
        last.parse::<u64>().ok()?.min(len - 1)
    };
    (start <= end).then_some((start, end))
}

/// Read up to `want` bytes of a body (the sealed header), then let the rest go.
#[cfg(not(target_arch = "wasm32"))]
async fn read_prefix(upstream: Upstream, want: usize) -> Result<Vec<u8>, u16> {
    let mut chunks = upstream.chunks;
    let mut out = Vec::with_capacity(want);
    while out.len() < want {
        match tokio::time::timeout(UPSTREAM_IDLE_TIMEOUT, chunks.next()).await {
            Ok(Some(Ok(bytes))) => {
                let take = (want - out.len()).min(bytes.len());
                out.extend_from_slice(&bytes[..take]);
            }
            Ok(None) => break,
            Ok(Some(Err(_))) | Err(_) => return Err(502),
        }
    }
    Ok(out)
}

/// Relay a sealed (`encrypted`) blob as the file it seals, with `Range`.
///
/// A first read takes the blob's header and its sealed length
/// (`Range: bytes=0-18`). From those the file's length follows, and from the
/// client's Range the segments that hold it: one more read fetches exactly
/// those segments, and each is authenticated and decrypted as it completes,
/// then cut to the requested span. The memory a download costs is about one
/// segment, whatever the file's size, and a slow client still slows the read
/// from the store. A segment that does not authenticate ends the body with an
/// error before any of its bytes are sent: hyper then aborts the connection.
///
/// A blob that does not start with an SBE header was stored before the
/// attachment was `encrypted`, and is relayed as it is.
#[cfg(not(target_arch = "wasm32"))]
async fn respond_sealed(
    client: &reqwest::Client,
    headers: Vec<(String, String)>,
    spec: BlobStreamSpec,
    sealed: Sealed,
) -> Response<ResponseBody> {
    use crate::interpreter::builtins::blob_crypto::{is_sealed, Opener, HEADER_LEN};

    let probe = match fetch(client, &spec, Some(format!("bytes=0-{}", HEADER_LEN - 1))).await {
        Ok(response) => response,
        Err(status) => return error_response(headers, status),
    };
    let total = match probe.status {
        206 => probe
            .header("content-range")
            .and_then(|v| complete_length(&v)),
        200 => probe
            .header("content-length")
            .and_then(|v| v.trim().parse().ok()),
        // An empty blob: nothing sealed there.
        416 => return respond_plain(client, headers, spec).await,
        404 => return error_response(headers, 404),
        other => {
            eprintln!("[soli] blob stream: the store answered {other}; replying 502");
            return error_response(headers, 502);
        }
    };
    let Some(total) = total else {
        eprintln!("[soli] blob stream: the store gave no length for a sealed blob");
        return error_response(headers, 502);
    };
    let head = match read_prefix(probe, HEADER_LEN).await {
        Ok(head) => head,
        Err(status) => return error_response(headers, status),
    };
    if !is_sealed(&head) {
        return respond_plain(client, headers, spec).await;
    }
    let opener = match Opener::new(&head, total, &sealed.purpose, &sealed.keys) {
        Ok(opener) => opener,
        Err(e) => {
            eprintln!("[soli] blob stream: {e}");
            return error_response(headers, 500);
        }
    };

    let len = opener.plain_len();
    let mut app = headers;
    app.retain(|(k, _)| !is_framing_header(k));
    if !app
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("content-type"))
    {
        app.push((
            "Content-Type".to_string(),
            "application/octet-stream".to_string(),
        ));
    }
    app.push(("Accept-Ranges".to_string(), "bytes".to_string()));

    let span = match spec.range.as_deref() {
        None => (len > 0).then(|| (0, len - 1)),
        Some(range) => match resolve_range(range, len) {
            Some(span) => Some(span),
            None => {
                let mut headers = error_headers(app);
                headers.push(("Content-Range".to_string(), format!("bytes */{len}")));
                return build(
                    Mapped {
                        status: 416,
                        headers,
                        body: MappedBody::Empty,
                    },
                    None,
                );
            }
        },
    };
    let status = if spec.range.is_some() { 206 } else { 200 };
    let sent = span.map_or(0, |(a, b)| b - a + 1);
    app.push(("Content-Length".to_string(), sent.to_string()));
    if let (206, Some((a, b))) = (status, span) {
        app.push(("Content-Range".to_string(), format!("bytes {a}-{b}/{len}")));
    }
    let empty = |headers| {
        build(
            Mapped {
                status,
                headers,
                body: MappedBody::Empty,
            },
            None,
        )
    };
    let Some((start, end)) = span.filter(|_| !spec.head) else {
        return empty(app);
    };

    let first = start / opener.segment_size();
    let last = end / opener.segment_size();
    let (from, to) = opener.sealed_span(first, last);
    let upstream = match fetch(client, &spec, Some(format!("bytes={from}-{to}"))).await {
        Ok(response) => response,
        Err(status) => return error_response(app, status),
    };
    // A server that ignores the Range sends the whole blob: skip to `from`.
    let skip = match upstream.status {
        206 => 0,
        200 => from,
        404 => return error_response(app, 404),
        other => {
            eprintln!("[soli] blob stream: the store answered {other}; replying 502");
            return error_response(app, 502);
        }
    };
    let body = sealed_body(upstream, opener, skip, first, last, start, end);
    let mut builder = Response::builder()
        .status(StatusCode::from_u16(status).unwrap_or(StatusCode::OK))
        .header("Server", "soliMVC");
    for (name, value) in &app {
        builder = add_header_checked(builder, name, value);
    }
    builder.body(body).unwrap_or_else(|_| {
        Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(full(Bytes::from_static(b"Internal Server Error")))
            .expect("static 500 response is always valid")
    })
}

/// Where a sealed relay is.
#[cfg(not(target_arch = "wasm32"))]
struct SealedState {
    chunks: Chunks,
    opener: crate::interpreter::builtins::blob_crypto::Opener,
    /// Sealed bytes of the segment being assembled.
    pending: Vec<u8>,
    /// Upstream bytes still to drop before segment `index` starts.
    skip: u64,
    index: u64,
    last: u64,
    /// The plaintext span to send, inclusive.
    start: u64,
    end: u64,
}

/// The store's sealed bytes, from the start of segment `first`, as the file's
/// bytes `start..=end`: one frame per segment, each sent only once its tag
/// has checked out.
#[cfg(not(target_arch = "wasm32"))]
fn sealed_body(
    upstream: Upstream,
    opener: crate::interpreter::builtins::blob_crypto::Opener,
    skip: u64,
    first: u64,
    last: u64,
    start: u64,
    end: u64,
) -> ResponseBody {
    let state = SealedState {
        chunks: upstream.chunks,
        pending: Vec::with_capacity(opener.sealed_len(first) as usize),
        opener,
        skip,
        index: first,
        last,
        start,
        end,
    };
    let frames = futures_util::stream::unfold(Some(state), |state| async move {
        let mut state = state?;
        if state.index > state.last {
            return None;
        }
        let need = state.opener.sealed_len(state.index) as usize;
        while state.pending.len() < need {
            match tokio::time::timeout(UPSTREAM_IDLE_TIMEOUT, state.chunks.next()).await {
                Ok(Some(Ok(mut bytes))) => {
                    if state.skip > 0 {
                        let drop = state.skip.min(bytes.len() as u64) as usize;
                        state.skip -= drop as u64;
                        bytes = bytes.slice(drop..);
                    }
                    state.pending.extend_from_slice(&bytes);
                }
                Ok(None) => {
                    eprintln!("[soli] blob stream: the store ended a sealed blob early");
                    return Some((Err(std::io::Error::other("sealed blob truncated")), None));
                }
                Ok(Some(Err(e))) => {
                    eprintln!("[soli] blob stream: the store's body failed: {e}");
                    return Some((Err(std::io::Error::other("upstream body failed")), None));
                }
                Err(_) => {
                    eprintln!(
                        "[soli] blob stream: the store stalled for {}s; aborting",
                        UPSTREAM_IDLE_TIMEOUT.as_secs()
                    );
                    return Some((
                        Err(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            "upstream body stalled",
                        )),
                        None,
                    ));
                }
            }
        }
        // What came past this segment belongs to the next one.
        let rest = state.pending.split_off(need);
        let plain = match state.opener.open_segment(state.index, &state.pending) {
            Ok(plain) => plain,
            Err(e) => {
                eprintln!("[soli] blob stream: {e}");
                return Some((Err(std::io::Error::other("sealed segment rejected")), None));
            }
        };
        state.pending = rest;
        let segment_start = state.index * state.opener.segment_size();
        let from = state.start.saturating_sub(segment_start) as usize;
        let to = ((state.end + 1 - segment_start) as usize).min(plain.len());
        let frame = Bytes::from(plain).slice(from..to);
        state.index += 1;
        Some((Ok(hyper::body::Frame::data(frame)), Some(state)))
    });
    http_body_util::BodyExt::boxed(http_body_util::StreamBody::new(frames))
}

#[cfg(not(target_arch = "wasm32"))]
fn build(mapped: Mapped, upstream: Option<Upstream>) -> Response<ResponseBody> {
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

/// The store's body as a hyper body, pulled one chunk per frame hyper asks for.
///
/// `unfold` only polls the store when hyper polls the body, which it does once
/// the previous frame is written — that is the back-pressure. The idle timer
/// wraps only the wait on the store. An error ends the body early, and hyper
/// then aborts the connection rather than pretend a short body was complete.
#[cfg(not(target_arch = "wasm32"))]
fn relay_body(upstream: Upstream) -> ResponseBody {
    let frames = futures_util::stream::unfold(Some(upstream.chunks), |state| async move {
        let mut chunks = state?;
        match tokio::time::timeout(UPSTREAM_IDLE_TIMEOUT, chunks.next()).await {
            Ok(Some(Ok(bytes))) => Some((Ok(hyper::body::Frame::data(bytes)), Some(chunks))),
            Ok(None) => None,
            Ok(Some(Err(e))) => {
                eprintln!("[soli] blob stream: the store's body failed: {e}");
                Some((Err(std::io::Error::other("upstream body failed")), None))
            }
            Err(_) => {
                eprintln!(
                    "[soli] blob stream: the store stalled for {}s; aborting",
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
            Source::Http {
                url: "http://db/_api/blob/d/c/k1".into(),
                auth: Some(("Authorization".into(), "Basic eDp5".into())),
            },
            "k1",
            &req,
            Some(&extra),
            None,
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
        assert_eq!(
            spec.source,
            Source::Http {
                url: "http://db/_api/blob/d/c/k1".to_string(),
                auth: Some(("Authorization".to_string(), "Basic eDp5".to_string())),
            }
        );
        assert_eq!(spec.range.as_deref(), Some("bytes=100-199"));
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
        blob_response(
            Source::File("u".into()),
            "k",
            &request("HEAD", &[]),
            None,
            None,
        )
        .unwrap();
        assert!(take_pending().unwrap().head);

        blob_response(Source::File("u".into()), "k", &Value::Null, None, None).unwrap();
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
        let response =
            blob_response(Source::File("u".into()), "k9", &req, Some(&extra), None).unwrap();
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
        blob_response(Source::File("u".into()), "k", &req, None, None).unwrap();
        assert_eq!(take_pending().unwrap().range, None);
    }

    #[test]
    fn headers_must_be_a_hash() {
        let err = blob_response(
            Source::File("u".into()),
            "k",
            &Value::Null,
            Some(&s("nope")),
            None,
        );
        assert!(err.unwrap_err().contains("headers hash"));
    }

    #[test]
    fn a_spec_is_used_only_by_the_response_carrying_its_token() {
        let spec = BlobStreamSpec {
            token: 7,
            source: Source::File("u".into()),
            range: None,
            head: false,
            sealed: None,
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
            source: Source::Http {
                url,
                auth: Some(("Authorization".into(), "Basic dTpw".into())),
            },
            range: range.map(str::to_string),
            head,
            sealed: None,
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

    // -- Sealed (encrypted) blobs -------------------------------------------

    use crate::interpreter::builtins::blob_crypto::{self, BlobKey};

    const SEGMENT: usize = blob_crypto::DEFAULT_SEGMENT as usize;

    fn test_key() -> BlobKey {
        BlobKey::from_material(b"blob-stream-test-key")
    }

    fn sealed_spec(url: String, range: Option<&str>, head: bool) -> BlobStreamSpec {
        let mut spec = spec(url, range, head);
        spec.sealed = Some(Sealed {
            purpose: blob_crypto::purpose("media"),
            keys: vec![test_key()],
        });
        spec
    }

    /// `plain` sealed, served by a stand-in; the relay's URL for it.
    async fn sealed_stand_in(plain: &[u8]) -> (StandIn, String) {
        let sealed = blob_crypto::seal_with(
            plain,
            &blob_crypto::purpose("media"),
            &test_key(),
            blob_crypto::DEFAULT_SEGMENT,
        )
        .unwrap();
        let solidb = stand_in(Arc::new(sealed), None).await;
        let url = format!("{}/_api/blob/db/media/secret", solidb.base);
        (solidb, url)
    }

    #[test]
    fn a_range_resolves_against_the_file_length() {
        assert_eq!(resolve_range("bytes=0-9", 100), Some((0, 9)));
        assert_eq!(resolve_range("bytes=90-200", 100), Some((90, 99)));
        assert_eq!(resolve_range("bytes=95-", 100), Some((95, 99)));
        assert_eq!(resolve_range("bytes=-10", 100), Some((90, 99)));
        // A suffix longer than the file is the whole file, not a 416.
        assert_eq!(resolve_range("bytes=-500", 100), Some((0, 99)));
        assert_eq!(resolve_range("bytes=100-", 100), None);
        assert_eq!(resolve_range("bytes=-0", 100), None);
        assert_eq!(resolve_range("bytes=0-0", 0), None);
    }

    #[tokio::test]
    async fn a_sealed_blob_streams_as_the_file_whole_and_by_range() {
        let plain = blob(5 * MB + 123);
        let (solidb, url) = sealed_stand_in(&plain).await;
        let client = build_client();

        let whole = respond_with(
            &client,
            app_headers(),
            sealed_spec(url.clone(), None, false),
        )
        .await;
        assert_eq!(whole.status(), 200);
        assert_eq!(
            header(&whole, "content-length"),
            Some(plain.len().to_string())
        );
        assert_eq!(header(&whole, "accept-ranges").as_deref(), Some("bytes"));
        assert_eq!(
            header(&whole, "content-type").as_deref(),
            Some("audio/mpeg")
        );
        let (bytes, frames) = drain(whole).await;
        assert!(bytes == *plain, "the file, byte for byte");
        assert!(frames > 1, "one frame per segment, not one buffer");

        // Across segment boundaries, starting and ending inside full
        // segments (`bytes=-10` below ends in the short last one).
        let start = 3 * MB + 7;
        let end = start + MB;
        let range = format!("bytes={start}-{end}");
        let part = respond_with(
            &client,
            app_headers(),
            sealed_spec(url.clone(), Some(&range), false),
        )
        .await;
        assert_eq!(part.status(), 206);
        assert_eq!(
            header(&part, "content-range"),
            Some(format!("bytes {start}-{end}/{}", plain.len()))
        );
        assert_eq!(
            header(&part, "content-length"),
            Some((end - start + 1).to_string())
        );
        assert!(drain(part).await.0 == plain[start..=end]);

        // Inside one segment, and the file's last bytes.
        let inside = format!("bytes={}-{}", SEGMENT + 10, SEGMENT + 20);
        let small = respond_with(
            &client,
            app_headers(),
            sealed_spec(url.clone(), Some(&inside), false),
        )
        .await;
        assert_eq!(small.status(), 206);
        assert!(drain(small).await.0 == plain[SEGMENT + 10..=SEGMENT + 20]);
        let tail = respond_with(
            &client,
            app_headers(),
            sealed_spec(url.clone(), Some("bytes=-10"), false),
        )
        .await;
        assert!(drain(tail).await.0 == plain[plain.len() - 10..]);

        let past = format!("bytes={}-", plain.len());
        let unsatisfiable = respond_with(
            &client,
            app_headers(),
            sealed_spec(url.clone(), Some(&past), false),
        )
        .await;
        assert_eq!(unsatisfiable.status(), 416);
        assert_eq!(
            header(&unsatisfiable, "content-range"),
            Some(format!("bytes */{}", plain.len()))
        );

        // The range asked SoliDB for exactly the segments that hold it.
        let heads = solidb.heads.lock().unwrap().clone();
        let (first, last) = (start / SEGMENT, end / SEGMENT);
        let sealed_segment = SEGMENT + 16;
        let from = blob_crypto::HEADER_LEN + first * sealed_segment;
        let to = blob_crypto::HEADER_LEN + (last + 1) * sealed_segment - 1;
        let asked = format!("range: bytes={from}-{to}");
        assert!(
            heads
                .iter()
                .any(|h| h.to_ascii_lowercase().contains(&asked)),
            "{asked} in {heads:?}"
        );
        assert!(heads[0].to_ascii_lowercase().contains("range: bytes=0-18"));
    }

    #[tokio::test]
    async fn head_of_a_sealed_blob_reports_the_file_length() {
        let plain = blob(MB + 1);
        let (solidb, url) = sealed_stand_in(&plain).await;
        let response =
            respond_with(&build_client(), app_headers(), sealed_spec(url, None, true)).await;
        assert_eq!(response.status(), 200);
        assert_eq!(
            header(&response, "content-length"),
            Some(plain.len().to_string())
        );
        assert!(drain(response).await.0.is_empty());
        // The header was enough: no request for the body.
        assert_eq!(solidb.heads.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn an_empty_sealed_file_is_an_empty_body() {
        let (_solidb, url) = sealed_stand_in(b"").await;
        let response = respond_with(
            &build_client(),
            app_headers(),
            sealed_spec(url, None, false),
        )
        .await;
        assert_eq!(response.status(), 200);
        assert_eq!(header(&response, "content-length").as_deref(), Some("0"));
        assert!(drain(response).await.0.is_empty());
    }

    /// A segment that does not authenticate ends the body before any of its
    /// bytes: what came before it went out, nothing of it does.
    #[tokio::test]
    async fn a_tampered_segment_ends_the_body_before_its_bytes() {
        let plain = blob(3 * SEGMENT + 50);
        let mut sealed = blob_crypto::seal_with(
            &plain,
            &blob_crypto::purpose("media"),
            &test_key(),
            blob_crypto::DEFAULT_SEGMENT,
        )
        .unwrap();
        let in_third = blob_crypto::HEADER_LEN + 2 * (SEGMENT + 16) + 100;
        sealed[in_third] ^= 1;
        let solidb = stand_in(Arc::new(sealed), None).await;
        let url = format!("{}/_api/blob/db/media/secret", solidb.base);

        let response = respond_with(
            &build_client(),
            app_headers(),
            sealed_spec(url, None, false),
        )
        .await;
        assert_eq!(response.status(), 200);
        let mut body = response.into_body();
        let mut got = Vec::new();
        let mut failed = false;
        while let Some(frame) = body.frame().await {
            match frame {
                Ok(frame) => got.extend_from_slice(&frame.into_data().unwrap()),
                Err(_) => {
                    failed = true;
                    break;
                }
            }
        }
        assert!(failed, "the body ends in an error");
        assert!(
            got == plain[..2 * SEGMENT],
            "the two good segments and nothing more"
        );
    }

    #[tokio::test]
    async fn plaintext_under_an_encrypted_uploader_is_relayed_as_is() {
        let data = blob(200_000);
        let solidb = stand_in(data.clone(), None).await;
        let url = format!("{}/_api/blob/db/media/legacy", solidb.base);
        let response = respond_with(
            &build_client(),
            app_headers(),
            sealed_spec(url, Some("bytes=10-19"), false),
        )
        .await;
        assert_eq!(response.status(), 206);
        assert!(drain(response).await.0 == data[10..20]);
    }

    #[tokio::test]
    async fn a_sealed_blob_without_its_key_is_a_server_error() {
        let (_solidb, url) = sealed_stand_in(b"secret").await;
        let mut spec = sealed_spec(url, None, false);
        spec.sealed.as_mut().unwrap().keys = vec![BlobKey::from_material(b"another key")];
        let response = respond_with(&build_client(), app_headers(), spec).await;
        assert_eq!(response.status(), 500);
        assert_eq!(header(&response, "content-disposition"), None);
        assert_eq!(drain(response).await.0, b"Internal Server Error");
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
            whole_spec.source = Source::Http {
                url: url.clone(),
                auth: auth.clone(),
            };
            let whole = respond_with(&http, app_headers(), whole_spec).await;
            assert_eq!(whole.status(), 200);
            let (bytes, _) = drain(whole).await;
            assert!(bytes == *data, "whole blob from SoliDB");

            let mut part_spec = spec(url.clone(), Some("bytes=1048576-1048675"), false);
            part_spec.source = Source::Http {
                url: url.clone(),
                auth: auth.clone(),
            };
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

    // -- A disk file -------------------------------------------------------

    fn file_spec(path: &std::path::Path, range: Option<&str>, head: bool) -> BlobStreamSpec {
        BlobStreamSpec {
            token: 1,
            source: Source::File(path.to_path_buf()),
            range: range.map(str::to_string),
            head,
            sealed: None,
        }
    }

    #[tokio::test]
    async fn a_disk_file_streams_whole_by_range_and_head() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data");
        let data = blob(3 * MB + 77);
        std::fs::write(&path, &*data).unwrap();
        let client = build_client();

        let whole = respond_with(&client, app_headers(), file_spec(&path, None, false)).await;
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
        let (bytes, frames) = drain(whole).await;
        assert!(bytes == *data, "the whole file, byte for byte");
        assert!(frames > 1, "read in chunks, not as one buffer");

        let (start, end) = (MB + 5, 2 * MB + 9);
        let range = format!("bytes={start}-{end}");
        let part = respond_with(
            &client,
            app_headers(),
            file_spec(&path, Some(&range), false),
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
        assert!(drain(part).await.0 == data[start..=end]);

        let tail = respond_with(
            &client,
            app_headers(),
            file_spec(&path, Some("bytes=-10"), false),
        )
        .await;
        assert_eq!(tail.status(), 206);
        assert!(drain(tail).await.0 == data[data.len() - 10..]);

        let past = format!("bytes={}-", data.len());
        let unsatisfiable =
            respond_with(&client, app_headers(), file_spec(&path, Some(&past), false)).await;
        assert_eq!(unsatisfiable.status(), 416);
        assert_eq!(
            header(&unsatisfiable, "content-range"),
            Some(format!("bytes */{}", data.len()))
        );

        let head = respond_with(&client, app_headers(), file_spec(&path, None, true)).await;
        assert_eq!(head.status(), 200);
        assert_eq!(
            header(&head, "content-length"),
            Some(data.len().to_string())
        );
        assert!(drain(head).await.0.is_empty());
    }

    #[tokio::test]
    async fn an_empty_or_missing_disk_file() {
        let dir = tempfile::tempdir().unwrap();
        let empty = dir.path().join("empty");
        std::fs::write(&empty, b"").unwrap();
        let client = build_client();

        let response = respond_with(&client, app_headers(), file_spec(&empty, None, false)).await;
        assert_eq!(response.status(), 200);
        assert_eq!(header(&response, "content-length").as_deref(), Some("0"));
        assert!(drain(response).await.0.is_empty());
        let head = respond_with(&client, app_headers(), file_spec(&empty, None, true)).await;
        assert_eq!(head.status(), 200);
        assert_eq!(header(&head, "content-length").as_deref(), Some("0"));

        let gone = dir.path().join("gone");
        let missing = respond_with(&client, app_headers(), file_spec(&gone, None, false)).await;
        assert_eq!(missing.status(), 404);
        let mut sealed = file_spec(&gone, None, false);
        sealed.sealed = Some(Sealed {
            purpose: blob_crypto::purpose("media"),
            keys: vec![test_key()],
        });
        assert_eq!(
            respond_with(&client, app_headers(), sealed).await.status(),
            404
        );
    }

    #[tokio::test]
    async fn a_sealed_disk_file_streams_as_the_file_by_range() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data");
        let plain = blob(4 * SEGMENT + 321);
        let sealed = blob_crypto::seal_with(
            &plain,
            &blob_crypto::purpose("media"),
            &test_key(),
            blob_crypto::DEFAULT_SEGMENT,
        )
        .unwrap();
        std::fs::write(&path, &sealed).unwrap();
        let client = build_client();
        let with_key = |range: Option<&str>, head: bool| {
            let mut spec = file_spec(&path, range, head);
            spec.sealed = Some(Sealed {
                purpose: blob_crypto::purpose("media"),
                keys: vec![test_key()],
            });
            spec
        };

        let whole = respond_with(&client, app_headers(), with_key(None, false)).await;
        assert_eq!(whole.status(), 200);
        assert_eq!(
            header(&whole, "content-length"),
            Some(plain.len().to_string())
        );
        assert!(drain(whole).await.0 == *plain);

        let (start, end) = (SEGMENT - 3, 3 * SEGMENT + 10);
        let range = format!("bytes={start}-{end}");
        let part = respond_with(&client, app_headers(), with_key(Some(&range), false)).await;
        assert_eq!(part.status(), 206);
        assert_eq!(
            header(&part, "content-range"),
            Some(format!("bytes {start}-{end}/{}", plain.len()))
        );
        assert!(drain(part).await.0 == plain[start..=end]);

        let head = respond_with(&client, app_headers(), with_key(None, true)).await;
        assert_eq!(
            header(&head, "content-length"),
            Some(plain.len().to_string())
        );

        // A plaintext file under an encrypted uploader predates the option.
        let legacy = dir.path().join("legacy");
        std::fs::write(&legacy, b"stored before encryption").unwrap();
        let mut spec = file_spec(&legacy, Some("bytes=7-12"), false);
        spec.sealed = with_key(None, false).sealed;
        let old = respond_with(&client, app_headers(), spec).await;
        assert_eq!(old.status(), 206);
        assert_eq!(drain(old).await.0, b"before");
    }
}
