//! Native-driver transport for the model layer: the default, `SOLI_DB_DRIVER=0` opts out.
//!
//! SoliDB multiplexes three protocols on one port, chosen by magic bytes in its
//! accept loop: `solidb-sync-v1` (replication), `solidb-drv-v1\0` (this one) and
//! HTTP for anything else. The driver protocol is MessagePack with a 4-byte
//! big-endian length prefix on a persistent authenticated connection, so a write
//! costs a few hundred bytes down an open socket instead of a full HTTP
//! request/response cycle.
//!
//! That difference is why this module exists. On the framework benchmark Soli
//! lost all three write rows to Phoenix, and the diagnosis was transport: Soli's
//! inserts leave the process as HTTP while Ecto's go down a pooled binary
//! connection. Measured standalone on the same box, the driver protocol runs
//! **2.2x** HTTP on inserts and **3.8x** on single-document reads.
//!
//! ## Why a thread-local client
//!
//! `SoliDBClient` owns its sockets and needs `&mut self`, so it cannot be
//! shared. Soli's workers are threads, so one client per worker is the natural
//! fit — and every call goes through `block_on_db`, the same per-worker runtime
//! the reqwest path uses. That last part is not optional: tokio sockets are
//! bound to the reactor that created them, and driving them from a second
//! runtime is what produced the exactly-10s stalls that `SOLI_DB_SHARED_REACTOR`
//! exists to work around.
//!
//! ## Scope
//!
//! The driver routes document CRUD **and** multi-row queries. Anything not
//! covered (transactions, blobs, a `https://` host) falls back to HTTP by
//! returning `None` from `try_*`, so the transport can change, never the
//! semantics. It used to be opt-in (`SOLI_DB_DRIVER=1`); the published
//! benchmarks had been measured on it all along.
//!
//! Measured on the framework benchmark, against a SoliDB carrying the
//! driver-side cache fix:
//!
//! * writes — **+37% throughput, 0.57x system-wide CPU per request**
//! * 50-row read + render — **+40% throughput, 0.43x SoliDB CPU per request**
//!
//! Queries need that server fix; see `query_enabled()`.

use std::cell::RefCell;

use serde_json::Value;
use solidb_client::{DriverError, SoliDBClient};

use crate::interpreter::builtins::http_class::block_on_db;
use crate::interpreter::builtins::model::db_config::{db_scheme_and_host, get_database_name};

/// Connections per worker: one. A worker runs one request at a time and every
/// call blocks on `block_on_db`, so it never has two commands in flight; the
/// four others of the old pool of five (sized to match the reqwest pool for a
/// benchmark) sat idle. Each connection is authenticated with a full password
/// check on the server, one after the other, on the worker's first query: five
/// of them put ~100 ms on the first request after a wake, and 80 password
/// checks on SoliDB for 16 workers.
const POOL_SIZE: usize = 1;

thread_local! {
    /// `None` until the first use; `Some(Err)` once a connection attempt has
    /// failed, so a broken driver setup degrades to HTTP instead of retrying a
    /// dead endpoint on every query.
    static CLIENT: RefCell<Option<Result<SoliDBClient, String>>> = const { RefCell::new(None) };
}

/// Is the native driver transport switched on? It is unless `SOLI_DB_DRIVER`
/// says `0`, `false`, `off` or `no`.
pub fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| driver_setting() != Some(false))
}

/// `SOLI_DB_DRIVER` as written: `Some(true)` asked for the driver, `Some(false)`
/// turned it off, `None` left the default.
fn driver_setting() -> Option<bool> {
    let raw = crate::platform::env::var("SOLI_DB_DRIVER").ok()?;
    match raw.trim().to_ascii_lowercase().as_str() {
        "0" | "false" | "off" | "no" => Some(false),
        "" => None,
        _ => Some(true),
    }
}

/// Why this worker stays on HTTP, when that is worth a line on stderr: always
/// for a driver that was asked for, and for a default one only when SoliDB could
/// not be reached by it. A `https://` host is HTTP's by design, not a failure.
fn report_fallback(error: &DriverUnavailable) {
    let asked = driver_setting() == Some(true);
    match error {
        DriverUnavailable::TlsHost if !asked => {}
        DriverUnavailable::TlsHost => {
            eprintln!("[solidb_driver] SOLI_DB_DRIVER does not support TLS hosts; using HTTP")
        }
        DriverUnavailable::Failed(e) => eprintln!("[solidb_driver] {e}; falling back to HTTP"),
    }
}

enum DriverUnavailable {
    TlsHost,
    Failed(String),
}

/// Route multi-row SDBQL queries over the driver as well? **On** with the driver,
/// opt out with `SOLI_DB_DRIVER_QUERY=0`.
///
/// This needs a SoliDB carrying the driver-side cache fix. The driver's
/// `handle_query` originally had neither of the two layers the HTTP `/cursor`
/// handler uses — a prepared-statement cache and a read-result cache — so it
/// executed every query for real while HTTP replayed a memoized result. Measured
/// that way it looked like the binary protocol was bad at queries (0.84x
/// throughput, SoliDB CPU 127 -> 216us). It was not: one handler cached and the
/// other did not.
///
/// With both layers added to the driver handler, the same cell measures:
///
/// | arm    | req/s  | SoliDB CPU/req |
/// |--------|-------:|---------------:|
/// | HTTP   | 35,726 |     106-123us  |
/// | driver | 50,106 |      49-50us   |
///
/// **1.40x the throughput on under half the server CPU** — MessagePack costs
/// SoliDB less to produce than the HTTP JSON response, and there is no HTTP
/// framing to build. Against an *older* SoliDB without that fix, set
/// `SOLI_DB_DRIVER_QUERY=0` to keep queries on the cursor endpoint.
fn query_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| crate::platform::env::var("SOLI_DB_DRIVER_QUERY").as_deref() != Ok("0"))
}

/// Opt out of SoliDB's read-result memoization on the driver path too.
/// Mirrors the HTTP `payload["cache"] = false` in `crud.rs`.
fn no_query_cache() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| crate::platform::env::var("SOLI_DB_NO_QUERY_CACHE").as_deref() == Ok("1"))
}

/// `host:port` for the driver, taken from the same `SOLIDB_HOST` the HTTP path
/// uses. The driver speaks raw TCP, so the scheme is dropped — and a TLS host is
/// refused rather than silently downgraded to plaintext.
fn driver_addr() -> Result<String, DriverUnavailable> {
    let (scheme, host) = db_scheme_and_host();
    if scheme.starts_with("https") {
        return Err(DriverUnavailable::TlsHost);
    }
    Ok(host)
}

/// Authenticate every pooled socket, matching the credentials the HTTP model
/// path would use.
///
/// Preference order:
/// 1. `SOLIDB_API_KEY` — the common production path
/// 2. `SOLIDB_USERNAME` / `SOLIDB_PASSWORD` when either is set
/// 3. `admin` / `admin` for the local-dev default (same as SoliDB's bootstrap)
async fn authenticate(client: &mut SoliDBClient, database: &str) -> Result<(), String> {
    if let Ok(api_key) = crate::platform::env::var("SOLIDB_API_KEY") {
        if !api_key.is_empty() {
            return client
                .auth_with_api_key(database, &api_key)
                .await
                .map_err(|e| format!("driver auth (api key) failed: {e}"));
        }
    }

    let username = crate::platform::env::var("SOLIDB_USERNAME").unwrap_or_else(|_| "admin".into());
    let password = crate::platform::env::var("SOLIDB_PASSWORD").unwrap_or_else(|_| "admin".into());
    client
        .auth(database, &username, &password)
        .await
        .map_err(|e| format!("driver auth failed: {e}"))
}

/// Why a driver call failed, kept typed until the caller words it the way the
/// HTTP path words the same failure. Callers read those words: a 409 becomes a
/// uniqueness error in `_errors`, a missing collection is created and the call
/// retried, `Cron` reports `HTTP 404`. Phrased differently, the same refusal
/// behaved differently depending on the transport.
#[derive(Debug)]
pub enum DriverFailure {
    /// SoliDB refused the command; its HTTP API answers the same error with
    /// `status`.
    Server {
        status: reqwest::StatusCode,
        message: String,
    },
    /// The connection failed. The worker's client is dropped so the next call
    /// connects again.
    Connection(String),
}

impl DriverFailure {
    /// In the words of the HTTP document path (`exec_document_request`).
    pub fn document_error(&self, url: &str) -> String {
        match self {
            DriverFailure::Server { status, message } => format!(
                "HTTP {} {}: {}",
                status,
                url,
                serde_json::json!({ "error": message })
            ),
            DriverFailure::Connection(message) => format!("HTTP error: {message}"),
        }
    }

    /// In the words of the HTTP cursor path (`format_query_http_failure`).
    pub fn query_error(&self) -> String {
        match self {
            DriverFailure::Server { status, message } => format!(
                "Query failed: {} - {}",
                status,
                serde_json::json!({ "error": message })
            ),
            DriverFailure::Connection(message) => format!("HTTP error: {message}"),
        }
    }
}

fn classify(error: DriverError) -> DriverFailure {
    use reqwest::StatusCode;
    let (status, message) = match error {
        DriverError::DatabaseError(message)
        | DriverError::ServerError(message)
        | DriverError::InvalidCommand(message)
        | DriverError::TransactionError(message) => status_for(&message),
        DriverError::AuthError(message) => (StatusCode::UNAUTHORIZED, message),
        DriverError::MessageTooLarge => (
            StatusCode::PAYLOAD_TOO_LARGE,
            "message too large".to_string(),
        ),
        DriverError::ConnectionError(message) | DriverError::ProtocolError(message) => {
            return DriverFailure::Connection(message)
        }
    };
    DriverFailure::Server { status, message }
}

/// The status SoliDB's HTTP API gives an error (`DbError::into_response`), and
/// the message its body carries, read back from the message the driver
/// carries: the same `DbError`, printed. The variant's prefix decides first —
/// a unique-index violation is an `Invalid document: … already exists`, a 400
/// over HTTP, not a 409 — and HTTP's body holds the message without it.
fn status_for(message: &str) -> (reqwest::StatusCode, String) {
    use reqwest::StatusCode;
    let prefixed = [
        ("Invalid document: ", StatusCode::BAD_REQUEST),
        ("Parse error: ", StatusCode::BAD_REQUEST),
        ("Bad Request: ", StatusCode::BAD_REQUEST),
        ("Conflict: ", StatusCode::CONFLICT),
    ];
    for (prefix, status) in prefixed {
        if let Some(rest) = message.strip_prefix(prefix) {
            return (status, rest.to_string());
        }
    }
    let lower = message.to_ascii_lowercase();
    let status = if lower.contains("not found") {
        StatusCode::NOT_FOUND
    } else if lower.contains("already exists") {
        StatusCode::CONFLICT
    } else if lower.starts_with("schema validation") {
        StatusCode::BAD_REQUEST
    } else if lower.starts_with("unauthorized") {
        StatusCode::UNAUTHORIZED
    } else if lower.starts_with("forbidden") {
        StatusCode::FORBIDDEN
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };
    (status, message.to_string())
}

/// Run `f` against this worker's driver client, connecting on first use.
///
/// Returns `None` when the driver is off or unusable, which is the caller's
/// signal to take the HTTP path. A call whose connection failed drops the
/// client: the next one connects afresh.
fn with_client<T>(
    f: impl FnOnce(&mut SoliDBClient) -> Result<T, DriverFailure>,
) -> Option<Result<T, DriverFailure>> {
    if !enabled() {
        return None;
    }
    CLIENT.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            let connected = block_on_db(async {
                let addr = driver_addr()?;
                let mut client = SoliDBClient::connect_with_pool(&addr, POOL_SIZE)
                    .await
                    .map_err(|e| {
                        DriverUnavailable::Failed(format!("driver connect failed: {e}"))
                    })?;
                let db = get_database_name();
                authenticate(&mut client, &db)
                    .await
                    .map_err(DriverUnavailable::Failed)?;
                Ok::<_, DriverUnavailable>(client)
            });
            let connected = connected.map_err(|e| {
                report_fallback(&e);
                match e {
                    DriverUnavailable::TlsHost => "TLS host: HTTP only".to_string(),
                    DriverUnavailable::Failed(message) => message,
                }
            });
            *slot = Some(connected);
        }
        let outcome = match slot.as_mut() {
            Some(Ok(client)) => f(client),
            // Connection failed earlier: stay on HTTP for this worker's lifetime.
            _ => return None,
        };
        if matches!(outcome, Err(DriverFailure::Connection(_))) {
            *slot = None;
        }
        Some(outcome)
    })
}

/// A read whose connection failed is asked again over HTTP. A write is not:
/// the server may have applied it before the socket went, and HTTP reports a
/// dropped connection as an error too.
fn read_or_http<T>(outcome: Option<Result<T, DriverFailure>>) -> Option<Result<T, DriverFailure>> {
    match outcome {
        Some(Err(DriverFailure::Connection(_))) => None,
        other => other,
    }
}

/// A document key as the HTTP path addresses it: a composite `_id`
/// (`products/abc`) names the document by its last segment.
fn document_key(key: &str) -> String {
    crate::interpreter::builtins::model::crud::normalize_key(key).to_string()
}

/// Insert many documents in one driver command. `None` means "not handled — use HTTP".
pub fn try_bulk_insert(
    collection: &str,
    documents: Vec<Value>,
) -> Option<Result<usize, DriverFailure>> {
    let coll = collection.to_string();
    let db = get_database_name();
    with_client(move |client| {
        block_on_db(async move {
            client
                .bulk_insert(&db, &coll, documents)
                .await
                .map_err(classify)
        })
    })
}

/// Insert one document. `None` means "not handled — use HTTP".
pub fn try_insert(
    collection: &str,
    document: &Value,
    key: Option<&str>,
) -> Option<Result<Value, DriverFailure>> {
    let (coll, doc, key) = (
        collection.to_string(),
        document.clone(),
        key.map(document_key),
    );
    let db = get_database_name();
    with_client(move |client| {
        block_on_db(async move {
            client
                .insert(&db, &coll, key.as_deref(), doc)
                .await
                .map_err(classify)
        })
    })
}

pub fn try_update(
    collection: &str,
    key: &str,
    document: &Value,
) -> Option<Result<Value, DriverFailure>> {
    let (coll, k, doc) = (collection.to_string(), document_key(key), document.clone());
    let db = get_database_name();
    with_client(move |client| {
        block_on_db(async move {
            // `merge` is passed false to mirror the HTTP path's PUT, but the two
            // agree either way: SoliDB merges on both. Verified directly —
            // `PUT /document/wposts/k` with `{"views":42}` leaves the row's other
            // fields intact, and so does this command with merge = false.
            //
            // Worth knowing because `crud.rs` asserts the opposite ("PUT is a
            // full replace, so `document` holds every field") and feeds
            // `document` to `live_query::notify_change` on that basis. If PUT
            // merges, a partial update hands live-query matching a partial row.
            // Pre-existing on the HTTP path; this transport does not change it.
            client
                .update(&db, &coll, &k, doc, false)
                .await
                .map_err(classify)
        })
    })
}

pub fn try_delete(collection: &str, key: &str) -> Option<Result<Value, DriverFailure>> {
    let (coll, k) = (collection.to_string(), document_key(key));
    let db = get_database_name();
    with_client(move |client| {
        block_on_db(async move {
            client
                .delete(&db, &coll, &k)
                .await
                .map(|()| Value::Null)
                .map_err(classify)
        })
    })
}

pub fn try_get(collection: &str, key: &str) -> Option<Result<Value, DriverFailure>> {
    let (coll, k) = (collection.to_string(), document_key(key));
    let db = get_database_name();
    read_or_http(with_client(move |client| {
        block_on_db(async move { client.get(&db, &coll, &k).await.map_err(classify) })
    }))
}

/// Run an SDBQL query and return its rows.
///
/// The driver hands back the whole result set in one response, so unlike the
/// HTTP cursor path there is no `has_more` batch to drain — which is also why
/// the 1,000-row cursor truncation that bit the HTTP path cannot happen here.
/// Could `try_query` handle a query at all? `false` guarantees it returns
/// `None`, which lets the caller skip copying the binds it would pass.
pub fn query_may_handle() -> bool {
    enabled() && query_enabled()
}

pub fn try_query(
    sdbql: &str,
    bind_vars: Option<std::collections::HashMap<String, Value>>,
) -> Option<Result<Vec<Value>, DriverFailure>> {
    // On with the driver; `SOLI_DB_DRIVER_QUERY=0` keeps queries on HTTP. See query_enabled().
    if !query_enabled() {
        return None;
    }
    let q = sdbql.to_string();
    let db = get_database_name();
    // Honour SOLI_DB_NO_QUERY_CACHE on the driver path the same way the HTTP
    // cursor payload does — otherwise the diagnostic only uncached one arm.
    let cache = !no_query_cache();
    read_or_http(with_client(move |client| {
        block_on_db(async move {
            client
                .query_with_cache(&db, &q, bind_vars, cache)
                .await
                .map_err(classify)
        })
    }))
}

/// [`try_query`], but the rows are decoded straight from the wire into Soli
/// values — no `serde_json::Value` in between. For a plain read that is one
/// decode of the result set instead of two, the dominant cost of a small
/// query on this transport.
pub fn try_query_values(
    sdbql: &str,
    bind_vars: Option<std::collections::HashMap<String, Value>>,
) -> Option<Result<Vec<crate::interpreter::value::Value>, DriverFailure>> {
    if !query_enabled() {
        return None;
    }
    let q = sdbql.to_string();
    let db = get_database_name();
    let cache = !no_query_cache();
    read_or_http(with_client(move |client| {
        block_on_db(async move {
            client
                .query_as::<crate::interpreter::value::Value>(&db, &q, bind_vars, cache)
                .await
                .map_err(classify)
        })
    }))
}

// ---------------------------------------------------------------------------
// Result ordering: no difference (an earlier note here said otherwise)
// ---------------------------------------------------------------------------
//
// The ORM's read query has no `ORDER BY` (`FOR doc IN posts RETURN {id: doc.id,
// …}`), so the storage engine's scan order decides the sequence. Both transports
// return the *same* sequence — verified by flipping only
// `SOLI_DB_DRIVER_QUERY` against the same collection: `[1, 3, 4, 2, 5, 6, 8, 12]`
// either way, and identical to a direct HTTP cursor call.
//
// A first pass here recorded this as a driver-side difference, on the strength of
// the driver's output no longer matching Django's row order. That was a wrong
// attribution: **SoliDB's own scan order for the collection had changed** in the
// meantime (rebuild/compaction between the two observations), and the HTTP path
// had moved with it. Worth keeping as a note because the underlying caveat is
// real and belongs to the benchmark rather than to this module: an unordered
// SDBQL query's row order is not stable across time, so a suite that asserts
// byte-identical payloads needs an `ORDER BY` to stay honest.

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::StatusCode;

    fn server(message: &str) -> DriverFailure {
        classify(DriverError::DatabaseError(message.to_string()))
    }

    /// The messages are SoliDB's `DbError` as printed; the statuses are the ones
    /// its HTTP API gives the same errors.
    #[test]
    fn a_refusal_gets_the_status_http_gives_it() {
        let cases = [
            ("Document with key 'k' not found", StatusCode::NOT_FOUND),
            ("Collection 'posts' not found", StatusCode::NOT_FOUND),
            (
                "Conflict: Document with _key 'k' already exists",
                StatusCode::CONFLICT,
            ),
            ("Collection 'posts' already exists", StatusCode::CONFLICT),
            ("Parse error: Unexpected token", StatusCode::BAD_REQUEST),
            ("Invalid document: not an object", StatusCode::BAD_REQUEST),
            (
                "Invalid document: Unique constraint violated: fields '[\"email\"]' with value \"a\" already exists in index 'idx'",
                StatusCode::BAD_REQUEST,
            ),
            (
                "Query execution error: division by zero",
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        ];
        for (message, expected) in cases {
            match server(message) {
                DriverFailure::Server { status, .. } => assert_eq!(status, expected, "{message}"),
                DriverFailure::Connection(_) => panic!("{message} read as a connection failure"),
            }
        }
    }

    /// What the callers look for in an error, found in the driver's too.
    #[test]
    fn a_refusal_reads_as_it_does_over_http() {
        use crate::interpreter::builtins::model::validation::is_unique_violation;

        let url = "http://127.0.0.1:6745/_api/database/app/document/users";
        let duplicate =
            server("Conflict: Document with _key 'joe' already exists").document_error(url);
        assert!(
            duplicate.starts_with("HTTP 409 Conflict http://"),
            "{duplicate}"
        );
        assert!(is_unique_violation(&duplicate), "{duplicate}");

        let missing = server("Document with key 'joe' not found").document_error(url);
        assert!(missing.starts_with("HTTP 404 Not Found"), "{missing}");

        let collection = server("Collection 'users' already exists").document_error(url);
        assert!(!is_unique_violation(&collection), "{collection}");

        let query = server("Parse error: Unexpected token").query_error();
        assert!(
            query.starts_with("Query failed: 400 Bad Request - "),
            "{query}"
        );
    }

    #[test]
    fn a_dropped_connection_is_not_a_refusal() {
        let failure = classify(DriverError::ConnectionError("reset by peer".into()));
        assert!(matches!(failure, DriverFailure::Connection(_)));
        assert_eq!(failure.document_error("u"), "HTTP error: reset by peer");
    }

    #[test]
    fn a_composite_id_names_its_document_by_the_last_segment() {
        assert_eq!(document_key("products/abc"), "abc");
        assert_eq!(document_key("abc"), "abc");
    }
}
