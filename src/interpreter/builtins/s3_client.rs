//! A small S3 client: Signature V4 over `reqwest`, path-style addressing.
//!
//! It covers what `S3.*` and the `s3` attachment service use — bucket list /
//! create / delete, object put (from memory or streamed from a file), get,
//! head, delete, copy, a paginated `ListObjectsV2`, and a presigned `PUT` —
//! and nothing else. It replaced rusoto, which is unmaintained and pulled
//! hyper 0.14 and a few megabytes of generated bindings into the build.
//!
//! Configuration is the environment `S3.*` has always read:
//! `AWS_ACCESS_KEY_ID` / `S3_ACCESS_KEY`, `AWS_SECRET_ACCESS_KEY` /
//! `S3_SECRET_KEY`, `AWS_REGION` / `S3_REGION` (default `us-east-1`) and
//! `S3_ENDPOINT` for MinIO, Garage, SeaweedFS, R2 and the like. Requests are
//! path-style (`https://host/bucket/key`), as rusoto's were.

use std::collections::BTreeMap;
use std::io::Read;
use std::sync::OnceLock;

use bytes::Bytes;
use hmac::{Hmac, Mac};
use quick_xml::events::Event;
use quick_xml::Reader;
use sha2::{Digest, Sha256};

use super::hex::encode as hex;
use crate::serve::get_tokio_handle;

/// SHA-256 of the empty string: the payload hash of every bodyless request.
const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// Credentials and where to send requests.
#[derive(Debug, Clone)]
pub(crate) struct S3Config {
    pub access_key: String,
    pub secret_key: String,
    pub region: String,
    /// `scheme://host[:port]`, no trailing slash.
    pub base_url: String,
    /// Whether `base_url` is the AWS default rather than `S3_ENDPOINT`.
    pub aws: bool,
}

impl S3Config {
    pub fn from_env() -> Result<Self, String> {
        let access_key = crate::platform::env::var("AWS_ACCESS_KEY_ID")
            .or_else(|_| crate::platform::env::var("S3_ACCESS_KEY"))
            .map_err(|_| "S3_ACCESS_KEY or AWS_ACCESS_KEY_ID not set".to_string())?;
        let secret_key = crate::platform::env::var("AWS_SECRET_ACCESS_KEY")
            .or_else(|_| crate::platform::env::var("S3_SECRET_KEY"))
            .map_err(|_| "S3_SECRET_KEY or AWS_SECRET_ACCESS_KEY not set".to_string())?;
        let region = crate::platform::env::var("AWS_REGION")
            .or_else(|_| crate::platform::env::var("S3_REGION"))
            .ok()
            .filter(|r| !r.is_empty())
            .unwrap_or_else(|| "us-east-1".to_string());
        let endpoint = crate::platform::env::var("S3_ENDPOINT")
            .ok()
            .filter(|e| !e.is_empty());
        Ok(Self::new(
            access_key,
            secret_key,
            region,
            endpoint.as_deref(),
        ))
    }

    pub fn new(
        access_key: String,
        secret_key: String,
        region: String,
        endpoint: Option<&str>,
    ) -> Self {
        let (base_url, aws) = match endpoint {
            Some(ep) => {
                let ep = ep.trim_end_matches('/');
                let url = if ep.starts_with("http://") || ep.starts_with("https://") {
                    ep.to_string()
                } else {
                    format!("https://{ep}")
                };
                // Scheme and authority only: requests are `/bucket/key` from
                // the root, and a path here would be sent but not signed.
                let authority_end = url
                    .find("://")
                    .and_then(|i| url[i + 3..].find('/').map(|j| i + 3 + j))
                    .unwrap_or(url.len());
                (url[..authority_end].to_string(), false)
            }
            None if region == "us-east-1" => ("https://s3.amazonaws.com".to_string(), true),
            None => (format!("https://s3.{region}.amazonaws.com"), true),
        };
        S3Config {
            access_key,
            secret_key,
            region,
            base_url,
            aws,
        }
    }
}

/// What `head_object` / `get_object` report about an object.
#[derive(Debug, Clone, Default)]
pub(crate) struct ObjectInfo {
    pub content_type: Option<String>,
    pub content_length: Option<u64>,
    /// `x-amz-meta-*` headers, keyed without the prefix.
    pub metadata: BTreeMap<String, String>,
}

/// One page of `ListObjectsV2`.
pub(crate) struct ListPage {
    pub keys: Vec<String>,
    pub next_token: Option<String>,
}

pub(crate) struct S3Client {
    config: S3Config,
    http: reqwest::Client,
}

/// The body of a request, and the payload hash its signature carries.
enum Payload {
    Empty,
    Bytes(Bytes),
    /// A file streamed in one-megabyte chunks; its hash and size are taken
    /// by a first read so the signature can cover the content, as it does for
    /// an in-memory body.
    File {
        path: std::path::PathBuf,
        sha256: String,
        size: u64,
    },
}

impl Payload {
    fn sha256(&self) -> String {
        match self {
            Payload::Empty => EMPTY_SHA256.to_string(),
            Payload::Bytes(b) => hex(&Sha256::digest(b)),
            Payload::File { sha256, .. } => sha256.clone(),
        }
    }
}

impl S3Client {
    pub fn from_env() -> Result<Self, String> {
        Ok(Self::new(S3Config::from_env()?))
    }

    pub fn new(config: S3Config) -> Self {
        S3Client {
            config,
            http: http_client(),
        }
    }

    pub async fn list_buckets(&self) -> Result<Vec<String>, String> {
        let body = self
            .send_text("GET", "", "", &[], &[], Payload::Empty)
            .await?;
        Ok(xml_texts(&body, "Name"))
    }

    pub async fn create_bucket(&self, bucket: &str) -> Result<(), String> {
        // AWS refuses a bare create outside us-east-1: the region has to be
        // named. An S3-compatible endpoint takes the region it is given.
        let payload = if self.config.aws && self.config.region != "us-east-1" {
            Payload::Bytes(Bytes::from(format!(
                "<CreateBucketConfiguration xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\
                 <LocationConstraint>{}</LocationConstraint></CreateBucketConfiguration>",
                self.config.region
            )))
        } else {
            Payload::Empty
        };
        self.send_text("PUT", bucket, "", &[], &[], payload)
            .await
            .map(|_| ())
    }

    pub async fn delete_bucket(&self, bucket: &str) -> Result<(), String> {
        self.send_text("DELETE", bucket, "", &[], &[], Payload::Empty)
            .await
            .map(|_| ())
    }

    pub async fn put_object(
        &self,
        bucket: &str,
        key: &str,
        body: Bytes,
        content_type: &str,
        metadata: &[(&str, &str)],
    ) -> Result<(), String> {
        let headers = object_headers(content_type, metadata);
        self.send_text("PUT", bucket, key, &[], &headers, Payload::Bytes(body))
            .await
            .map(|_| ())
    }

    /// Upload a file without holding it in memory.
    pub async fn put_object_file(
        &self,
        bucket: &str,
        key: &str,
        path: &std::path::Path,
        content_type: &str,
        metadata: &[(&str, &str)],
    ) -> Result<(), String> {
        let (sha256, size) = file_sha256(path)?;
        let headers = object_headers(content_type, metadata);
        let payload = Payload::File {
            path: path.to_path_buf(),
            sha256,
            size,
        };
        self.send_text("PUT", bucket, key, &[], &headers, payload)
            .await
            .map(|_| ())
    }

    pub async fn get_object(&self, bucket: &str, key: &str) -> Result<(ObjectInfo, Bytes), String> {
        let resp = self
            .send("GET", bucket, key, &[], &[], Payload::Empty)
            .await?;
        let info = object_info(resp.headers());
        let body = resp
            .bytes()
            .await
            .map_err(|e| format!("S3 read error: {e}"))?;
        Ok((info, body))
    }

    /// The object's headers, or `None` when there is no such object.
    pub async fn head_object(&self, bucket: &str, key: &str) -> Result<Option<ObjectInfo>, String> {
        match self
            .send("HEAD", bucket, key, &[], &[], Payload::Empty)
            .await
        {
            Ok(resp) => Ok(Some(object_info(resp.headers()))),
            Err(e) if e.starts_with("S3 404") => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub async fn delete_object(&self, bucket: &str, key: &str) -> Result<(), String> {
        self.send_text("DELETE", bucket, key, &[], &[], Payload::Empty)
            .await
            .map(|_| ())
    }

    /// `copy_source` is `bucket/key`, already percent-encoded.
    pub async fn copy_object(
        &self,
        copy_source: &str,
        bucket: &str,
        key: &str,
    ) -> Result<(), String> {
        let headers = [("x-amz-copy-source".to_string(), copy_source.to_string())];
        let body = self
            .send_text("PUT", bucket, key, &[], &headers, Payload::Empty)
            .await?;
        // A copy can fail after the 200 has been sent: the error is the body.
        match xml_error(&body) {
            Some(err) => Err(format!("S3 copy failed: {err}")),
            None => Ok(()),
        }
    }

    pub async fn list_objects_page(
        &self,
        bucket: &str,
        prefix: Option<&str>,
        continuation_token: Option<&str>,
    ) -> Result<ListPage, String> {
        let mut query = vec![("list-type".to_string(), "2".to_string())];
        if let Some(p) = prefix {
            query.push(("prefix".to_string(), p.to_string()));
        }
        if let Some(t) = continuation_token {
            query.push(("continuation-token".to_string(), t.to_string()));
        }
        let body = self
            .send_text("GET", bucket, "", &query, &[], Payload::Empty)
            .await?;
        let truncated = xml_texts(&body, "IsTruncated").first().map(String::as_str) == Some("true");
        let next_token = xml_texts(&body, "NextContinuationToken")
            .into_iter()
            .next()
            .filter(|t| truncated && !t.is_empty());
        Ok(ListPage {
            keys: xml_texts(&body, "Key"),
            next_token,
        })
    }

    /// A presigned `PUT` URL valid `expires_in` seconds. `signed_headers` must
    /// be sent, with exactly these values, by whoever uses the URL; anything
    /// else (`Content-Type`, `Content-Length`) is not covered.
    pub fn presign_put(
        &self,
        bucket: &str,
        key: &str,
        expires_in: u64,
        signed_headers: &[(&str, &str)],
    ) -> String {
        let now = chrono::Utc::now();
        let host = self.host();
        let path = object_path(bucket, key);
        let mut headers: Vec<(String, String)> = vec![("host".to_string(), host)];
        headers.extend(
            signed_headers
                .iter()
                .map(|(n, v)| (n.to_ascii_lowercase(), v.trim().to_string())),
        );
        let query = presign_query(&self.config, &now, expires_in, &headers);
        let signature = signature(
            &self.config,
            &now,
            "PUT",
            &path,
            &query,
            &headers,
            "UNSIGNED-PAYLOAD",
        );
        format!(
            "{}{}?{}&X-Amz-Signature={}",
            self.config.base_url,
            path,
            canonical_query(&query),
            signature
        )
    }

    fn host(&self) -> String {
        host_of(&self.config.base_url)
    }

    async fn send_text(
        &self,
        method: &str,
        bucket: &str,
        key: &str,
        query: &[(String, String)],
        headers: &[(String, String)],
        payload: Payload,
    ) -> Result<String, String> {
        let resp = self
            .send(method, bucket, key, query, headers, payload)
            .await?;
        resp.text().await.map_err(|e| format!("S3 read error: {e}"))
    }

    /// Sign and send one request; a non-2xx answer is an `Err` that starts
    /// `S3 <status>` and carries the error document's code and message.
    async fn send(
        &self,
        method: &str,
        bucket: &str,
        key: &str,
        query: &[(String, String)],
        extra_headers: &[(String, String)],
        payload: Payload,
    ) -> Result<reqwest::Response, String> {
        let now = chrono::Utc::now();
        let path = if bucket.is_empty() {
            "/".to_string()
        } else {
            object_path(bucket, key)
        };
        let payload_hash = payload.sha256();
        let mut headers: Vec<(String, String)> = vec![
            ("host".to_string(), self.host()),
            ("x-amz-content-sha256".to_string(), payload_hash.clone()),
            (
                "x-amz-date".to_string(),
                now.format("%Y%m%dT%H%M%SZ").to_string(),
            ),
        ];
        headers.extend(
            extra_headers
                .iter()
                .map(|(n, v)| (n.to_ascii_lowercase(), v.trim().to_string())),
        );
        let signature = signature(
            &self.config,
            &now,
            method,
            &path,
            query,
            &headers,
            &payload_hash,
        );
        let authorization = format!(
            "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
            self.config.access_key,
            scope(&self.config, &now),
            signed_header_names(&headers),
            signature
        );

        let mut url = format!("{}{}", self.config.base_url, path);
        if !query.is_empty() {
            url.push('?');
            url.push_str(&canonical_query(query));
        }
        let method = reqwest::Method::from_bytes(method.as_bytes()).map_err(|e| e.to_string())?;
        let mut request = self
            .http
            .request(method, &url)
            .header("authorization", authorization);
        for (name, value) in &headers {
            // reqwest writes Host from the URL, the same string signed above.
            if name != "host" {
                request = request.header(name.as_str(), value.as_str());
            }
        }
        request = match payload {
            Payload::Empty => request,
            Payload::Bytes(b) => request.body(b),
            Payload::File { path, size, .. } => request
                .header("content-length", size)
                .body(reqwest::Body::wrap_stream(file_chunks(&path)?)),
        };

        let resp = request
            .send()
            .await
            .map_err(|e| format!("S3 request failed: {}", without_url(e)))?;
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let body = resp.text().await.unwrap_or_default();
        Err(match xml_error(&body) {
            Some(err) => format!("S3 {} {}", status.as_u16(), err),
            None => format!(
                "S3 {} {}",
                status.as_u16(),
                status.canonical_reason().unwrap_or("")
            ),
        })
    }
}

/// Run an S3 future to completion from synchronous builtin code: on the
/// server's runtime when this thread has one, else on this thread's own.
pub(crate) fn run<F, T>(future: F) -> T
where
    F: std::future::Future<Output = T>,
{
    match get_tokio_handle() {
        Some(handle) => handle.block_on(future),
        None => FALLBACK.with(|(rt, _)| rt.block_on(future)),
    }
}

thread_local! {
    /// A runtime, and a client whose connections live on it, for threads the
    /// server did not start (scripts, `soli test`, jobs run inline). A client
    /// shared with another runtime would hand this one pooled connections
    /// whose tasks are driven elsewhere.
    static FALLBACK: (tokio::runtime::Runtime, reqwest::Client) = (
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to create the S3 fallback runtime"),
        build_http_client(),
    );
}

fn http_client() -> reqwest::Client {
    if get_tokio_handle().is_some() {
        static SHARED: OnceLock<reqwest::Client> = OnceLock::new();
        SHARED.get_or_init(build_http_client).clone()
    } else {
        FALLBACK.with(|(_, client)| client.clone())
    }
}

/// Not the user-facing `HTTP` client: its SSRF resolver refuses loopback and
/// private addresses, which is exactly where a MinIO or Garage often lives.
/// The endpoint is the operator's configuration, not request input.
fn build_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// A reqwest error without the URL, which names the bucket and key but
/// nothing secret; kept short for error messages.
fn without_url(e: reqwest::Error) -> String {
    let e = e.without_url();
    let mut s = e.to_string();
    let mut source = std::error::Error::source(&e);
    while let Some(inner) = source {
        s.push_str(": ");
        s.push_str(&inner.to_string());
        source = inner.source();
    }
    s
}

fn object_headers(content_type: &str, metadata: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut headers = vec![("content-type".to_string(), content_type.to_string())];
    headers.extend(metadata.iter().map(|(k, v)| {
        (
            format!("x-amz-meta-{}", k.to_ascii_lowercase()),
            metadata_value(v),
        )
    }));
    headers
}

/// A metadata value as an HTTP header can carry it: printable ASCII as is,
/// anything else as an RFC 2047 encoded word (`=?UTF-8?B?…?=`) — the form S3
/// itself returns non-ASCII metadata in. A browser refuses to send a raw
/// non-ASCII header, and `HeaderValue::to_str` refuses to read one.
pub(crate) fn metadata_value(value: &str) -> String {
    if value.bytes().all(|b| (0x20..0x7f).contains(&b)) && !value.starts_with("=?") {
        return value.to_string();
    }
    use base64::Engine;
    format!(
        "=?UTF-8?B?{}?=",
        base64::engine::general_purpose::STANDARD.encode(value.as_bytes())
    )
}

/// The inverse of [`metadata_value`], and lenient: a raw UTF-8 header (some
/// S3-compatible servers echo one) is read as UTF-8, and an RFC 2047 word in
/// either `B` or `Q` encoding is decoded.
fn read_metadata_value(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    let Some(inner) = text.strip_prefix("=?").and_then(|t| t.strip_suffix("?=")) else {
        return text.into_owned();
    };
    let mut parts = inner.splitn(3, '?');
    let (Some(charset), Some(encoding), Some(payload)) = (parts.next(), parts.next(), parts.next())
    else {
        return text.into_owned();
    };
    if !charset.eq_ignore_ascii_case("utf-8") {
        return text.into_owned();
    }
    let bytes = if encoding.eq_ignore_ascii_case("b") {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(payload)
            .ok()
    } else if encoding.eq_ignore_ascii_case("q") {
        Some(
            urlencoding::decode_binary(payload.replace('_', " ").replace('=', "%").as_bytes())
                .into_owned(),
        )
    } else {
        None
    };
    bytes
        .and_then(|b| String::from_utf8(b).ok())
        .unwrap_or_else(|| text.into_owned())
}

fn object_info(headers: &reqwest::header::HeaderMap) -> ObjectInfo {
    let text = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    let metadata = headers
        .iter()
        .filter_map(|(name, value)| {
            let key = name.as_str().strip_prefix("x-amz-meta-")?;
            Some((key.to_string(), read_metadata_value(value.as_bytes())))
        })
        .collect();
    ObjectInfo {
        content_type: text("content-type"),
        content_length: text("content-length").and_then(|v| v.parse().ok()),
        metadata,
    }
}

fn file_sha256(path: &std::path::Path) -> Result<(String, u64), String> {
    let mut file = std::fs::File::open(path).map_err(|e| format!("S3 upload open: {e}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut size = 0u64;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|e| format!("S3 upload read: {e}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        size += read as u64;
    }
    Ok((hex(&hasher.finalize()), size))
}

/// The file in one-megabyte chunks, so it is never resident whole.
fn file_chunks(
    path: &std::path::Path,
) -> Result<impl futures_util::Stream<Item = std::io::Result<Bytes>> + Send + 'static, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("S3 upload open: {e}"))?;
    Ok(futures_util::stream::unfold(file, |mut file| async move {
        let mut buffer = vec![0u8; 1024 * 1024];
        match file.read(&mut buffer) {
            Ok(0) => None,
            Ok(read) => {
                buffer.truncate(read);
                Some((Ok(Bytes::from(buffer)), file))
            }
            Err(e) => Some((Err(e), file)),
        }
    }))
}

// ---------------------------------------------------------------------------
// Signature V4
// ---------------------------------------------------------------------------

/// `/bucket/key`, each key segment percent-encoded and its slashes kept.
fn object_path(bucket: &str, key: &str) -> String {
    let mut path = format!("/{}", uri_encode(bucket, true));
    if !key.is_empty() {
        path.push('/');
        path.push_str(&uri_encode(key, false));
    }
    path
}

fn host_of(base_url: &str) -> String {
    let rest = base_url
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(base_url);
    let authority = rest.split('/').next().unwrap_or(rest);
    // reqwest leaves a default port out of `Host`; so must the signature.
    authority
        .strip_suffix(":443")
        .filter(|_| base_url.starts_with("https://"))
        .or_else(|| {
            authority
                .strip_suffix(":80")
                .filter(|_| base_url.starts_with("http://"))
        })
        .unwrap_or(authority)
        .to_string()
}

/// RFC 3986 percent-encoding as SigV4 wants it: unreserved characters kept,
/// everything else `%XX` uppercase; `/` kept unless `encode_slash`.
fn uri_encode(s: &str, encode_slash: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b'/' if !encode_slash => out.push('/'),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

fn canonical_query(query: &[(String, String)]) -> String {
    let mut pairs: Vec<(String, String)> = query
        .iter()
        .map(|(k, v)| (uri_encode(k, true), uri_encode(v, true)))
        .collect();
    pairs.sort();
    pairs
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&")
}

fn sorted_headers(headers: &[(String, String)]) -> Vec<(String, String)> {
    let mut sorted: Vec<(String, String)> = headers
        .iter()
        .map(|(n, v)| {
            (
                n.to_ascii_lowercase(),
                v.split_whitespace().collect::<Vec<_>>().join(" "),
            )
        })
        .collect();
    sorted.sort();
    sorted
}

fn signed_header_names(headers: &[(String, String)]) -> String {
    sorted_headers(headers)
        .iter()
        .map(|(n, _)| n.as_str())
        .collect::<Vec<_>>()
        .join(";")
}

fn scope(config: &S3Config, now: &chrono::DateTime<chrono::Utc>) -> String {
    format!("{}/{}/s3/aws4_request", now.format("%Y%m%d"), config.region)
}

/// The query of a presigned URL, without its signature.
fn presign_query(
    config: &S3Config,
    now: &chrono::DateTime<chrono::Utc>,
    expires_in: u64,
    headers: &[(String, String)],
) -> Vec<(String, String)> {
    vec![
        (
            "X-Amz-Algorithm".to_string(),
            "AWS4-HMAC-SHA256".to_string(),
        ),
        (
            "X-Amz-Credential".to_string(),
            format!("{}/{}", config.access_key, scope(config, now)),
        ),
        (
            "X-Amz-Date".to_string(),
            now.format("%Y%m%dT%H%M%SZ").to_string(),
        ),
        ("X-Amz-Expires".to_string(), expires_in.to_string()),
        (
            "X-Amz-SignedHeaders".to_string(),
            signed_header_names(headers),
        ),
    ]
}

fn hmac_sha256(key: &[u8], data: &str) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

/// The hex signature of one request (`path` already encoded).
fn signature(
    config: &S3Config,
    now: &chrono::DateTime<chrono::Utc>,
    method: &str,
    path: &str,
    query: &[(String, String)],
    headers: &[(String, String)],
    payload_hash: &str,
) -> String {
    let sorted = sorted_headers(headers);
    let canonical_headers: String = sorted.iter().map(|(n, v)| format!("{n}:{v}\n")).collect();
    let canonical_request = format!(
        "{method}\n{path}\n{}\n{canonical_headers}\n{}\n{payload_hash}",
        canonical_query(query),
        signed_header_names(headers),
    );
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{}\n{}\n{}",
        now.format("%Y%m%dT%H%M%SZ"),
        scope(config, now),
        hex(&Sha256::digest(canonical_request.as_bytes()))
    );
    let date_key = hmac_sha256(
        format!("AWS4{}", config.secret_key).as_bytes(),
        &now.format("%Y%m%d").to_string(),
    );
    let region_key = hmac_sha256(&date_key, &config.region);
    let service_key = hmac_sha256(&region_key, "s3");
    let signing_key = hmac_sha256(&service_key, "aws4_request");
    hex(&hmac_sha256(&signing_key, &string_to_sign))
}

// ---------------------------------------------------------------------------
// XML
// ---------------------------------------------------------------------------

/// The text of every element named `tag`, in document order. Entities are
/// resolved; an unknown entity is dropped (there is no DTD to define one).
fn xml_texts(xml: &str, tag: &str) -> Vec<String> {
    let mut reader = Reader::from_str(xml);
    let mut out = Vec::new();
    let mut current: Option<String> = None;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) if e.local_name().as_ref() == tag.as_bytes() => {
                current = Some(String::new());
            }
            Ok(Event::Text(t)) => {
                if let (Some(buf), Ok(text)) = (current.as_mut(), t.decode()) {
                    if let Ok(text) = quick_xml::escape::unescape(&text) {
                        buf.push_str(&text);
                    }
                }
            }
            Ok(Event::GeneralRef(r)) => {
                if let Some(buf) = current.as_mut() {
                    if r.is_char_ref() {
                        if let Ok(Some(c)) = r.resolve_char_ref() {
                            buf.push(c);
                        }
                    } else if let Ok(name) = r.decode() {
                        if let Some(s) = quick_xml::escape::resolve_predefined_entity(&name) {
                            buf.push_str(s);
                        }
                    }
                }
            }
            Ok(Event::End(e)) if e.local_name().as_ref() == tag.as_bytes() => {
                if let Some(text) = current.take() {
                    out.push(text);
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

/// `Code: Message` from an S3 `<Error>` document, if `xml` is one.
fn xml_error(xml: &str) -> Option<String> {
    if !xml.contains("<Error") {
        return None;
    }
    let code = xml_texts(xml, "Code").into_iter().next()?;
    match xml_texts(xml, "Message").into_iter().next() {
        Some(message) if !message.is_empty() => Some(format!("{code}: {message}")),
        _ => Some(code),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aws_example() -> S3Config {
        // The credentials AWS uses in its Signature V4 examples.
        S3Config {
            access_key: "AKIAIOSFODNN7EXAMPLE".to_string(),
            secret_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string(),
            region: "us-east-1".to_string(),
            base_url: "https://examplebucket.s3.amazonaws.com".to_string(),
            aws: true,
        }
    }

    fn example_time() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339("2013-05-24T00:00:00Z")
            .map(|t| t.with_timezone(&chrono::Utc))
            .unwrap_or_default()
    }

    /// AWS's "GET Object" header-signing example.
    #[test]
    fn signs_the_aws_get_object_example() {
        let headers = vec![
            (
                "host".to_string(),
                "examplebucket.s3.amazonaws.com".to_string(),
            ),
            ("range".to_string(), "bytes=0-9".to_string()),
            ("x-amz-content-sha256".to_string(), EMPTY_SHA256.to_string()),
            ("x-amz-date".to_string(), "20130524T000000Z".to_string()),
        ];
        let sig = signature(
            &aws_example(),
            &example_time(),
            "GET",
            "/test.txt",
            &[],
            &headers,
            EMPTY_SHA256,
        );
        assert_eq!(
            sig,
            "f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
    }

    /// AWS's presigned-URL example (a GET, but the arithmetic is the same).
    #[test]
    fn signs_the_aws_presigned_url_example() {
        let config = aws_example();
        let now = example_time();
        let headers = vec![(
            "host".to_string(),
            "examplebucket.s3.amazonaws.com".to_string(),
        )];
        let query = presign_query(&config, &now, 86400, &headers);
        let sig = signature(
            &config,
            &now,
            "GET",
            "/test.txt",
            &query,
            &headers,
            "UNSIGNED-PAYLOAD",
        );
        assert_eq!(
            sig,
            "aeeed9bbccd4d02ee5c0109b86d86835f995330da4c265957d157751f604d404"
        );
    }

    #[test]
    fn keys_are_encoded_once_and_keep_their_slashes() {
        assert_eq!(
            object_path("b", "a dir/é+x.txt"),
            "/b/a%20dir/%C3%A9%2Bx.txt"
        );
        assert_eq!(object_path("b", ""), "/b");
        assert_eq!(uri_encode("a/b~c", true), "a%2Fb~c");
    }

    /// The whole client against a live S3-compatible server — SeaweedFS,
    /// Garage, MinIO or AWS itself. Not part of the normal run:
    ///
    /// ```text
    /// S3_E2E_ENDPOINT=http://127.0.0.1:8333 S3_E2E_ACCESS_KEY=… S3_E2E_SECRET_KEY=… \
    ///   cargo test --lib s3_client -- --ignored
    /// ```
    #[test]
    #[ignore = "needs a live S3-compatible endpoint (S3_E2E_ENDPOINT, _ACCESS_KEY, _SECRET_KEY)"]
    fn round_trip_against_a_live_endpoint() {
        let env =
            |name: &str| crate::platform::env::var(format!("S3_E2E_{name}")).unwrap_or_default();
        let region = Some(env("REGION")).filter(|r| !r.is_empty());
        let config = S3Config::new(
            env("ACCESS_KEY"),
            env("SECRET_KEY"),
            region.unwrap_or_else(|| "us-east-1".to_string()),
            Some(env("ENDPOINT").as_str()),
        );
        let client = S3Client::new(config);
        let bucket = format!("soli-e2e-{}", uuid::Uuid::new_v4().simple());
        assert!(matches!(run(client.create_bucket(&bucket)), Ok(())));

        // Three and a half chunks of the one-megabyte stream, with a key and
        // metadata that need encoding.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("big.bin");
        let content: Vec<u8> = (0..3_500_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&path, &content).expect("write");
        let key = "streamed/é & +.bin";
        let put = run(client.put_object_file(
            &bucket,
            key,
            &path,
            "application/x-test",
            &[("original-filename", "grand é.bin")],
        ));
        assert!(put.is_ok(), "{put:?}");

        let head = run(client.head_object(&bucket, key)).expect("head");
        let head = head.expect("the object exists");
        assert_eq!(head.content_length, Some(3_500_000));
        assert_eq!(head.content_type.as_deref(), Some("application/x-test"));
        assert_eq!(
            head.metadata.get("original-filename").map(String::as_str),
            Some("grand é.bin")
        );
        let (_, body) = run(client.get_object(&bucket, key)).expect("get");
        assert!(body.as_ref() == content.as_slice());

        assert!(matches!(
            run(client.head_object(&bucket, "absent")),
            Ok(None)
        ));
        assert!(run(client.delete_object(&bucket, key)).is_ok());
        assert!(run(client.delete_bucket(&bucket)).is_ok());
    }

    #[test]
    fn non_ascii_metadata_travels_as_an_encoded_word() {
        assert_eq!(metadata_value("plain.txt"), "plain.txt");
        let encoded = metadata_value("r é.txt");
        assert_eq!(encoded, "=?UTF-8?B?ciDDqS50eHQ=?=");
        assert_eq!(read_metadata_value(encoded.as_bytes()), "r é.txt");
        assert_eq!(read_metadata_value("r é.txt".as_bytes()), "r é.txt");
        assert_eq!(read_metadata_value(b"=?UTF-8?Q?r_=C3=A9.txt?="), "r é.txt");
        assert_eq!(
            read_metadata_value(b"=?ISO-8859-1?B?6Q==?="),
            "=?ISO-8859-1?B?6Q==?="
        );
    }

    #[test]
    fn the_host_drops_a_default_port_only() {
        assert_eq!(host_of("https://s3.amazonaws.com"), "s3.amazonaws.com");
        assert_eq!(host_of("http://127.0.0.1:9000"), "127.0.0.1:9000");
        assert_eq!(host_of("https://minio.local:443"), "minio.local");
        assert_eq!(host_of("http://minio.local:443"), "minio.local:443");
    }

    #[test]
    fn endpoints_default_to_https_and_aws_hosts_follow_the_region() {
        let cfg = |region: &str, ep: Option<&str>| {
            S3Config::new("a".into(), "s".into(), region.into(), ep).base_url
        };
        assert_eq!(cfg("us-east-1", None), "https://s3.amazonaws.com");
        assert_eq!(cfg("eu-west-3", None), "https://s3.eu-west-3.amazonaws.com");
        assert_eq!(
            cfg("auto", Some("acct.r2.cloudflarestorage.com/")),
            "https://acct.r2.cloudflarestorage.com"
        );
        assert_eq!(
            cfg("us-east-1", Some("http://localhost:9000")),
            "http://localhost:9000"
        );
        assert_eq!(
            cfg("us-east-1", Some("https://s3.example.com/ignored/path")),
            "https://s3.example.com"
        );
    }

    #[test]
    fn list_and_error_documents_are_read_with_entities_resolved() {
        let list = "<ListBucketResult><IsTruncated>true</IsTruncated>\
            <Contents><Key>a &amp; b.txt</Key></Contents><Contents><Key>c&#47;d</Key></Contents>\
            <NextContinuationToken>tok</NextContinuationToken></ListBucketResult>";
        assert_eq!(xml_texts(list, "Key"), vec!["a & b.txt", "c/d"]);
        assert_eq!(xml_texts(list, "NextContinuationToken"), vec!["tok"]);
        let err = "<?xml version=\"1.0\"?><Error><Code>NoSuchBucket</Code>\
            <Message>The specified bucket does not exist</Message></Error>";
        assert_eq!(
            xml_error(err).as_deref(),
            Some("NoSuchBucket: The specified bucket does not exist")
        );
        assert_eq!(xml_error("<CopyObjectResult/>"), None);
    }
}
