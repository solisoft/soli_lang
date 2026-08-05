//! Native-driver transport for the model layer, behind `SOLI_DB_DRIVER=1`.
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
//! `SOLI_DB_DRIVER=1` routes document CRUD **and** multi-row queries. Anything
//! not covered falls back to HTTP by returning `None` from `try_*`, so enabling
//! the flag can only change the transport, never the semantics.
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
use solidb_client::SoliDBClient;

use crate::interpreter::builtins::http_class::block_on_db;
use crate::interpreter::builtins::model::db_config::{get_database_name, DB_CONFIG};

/// Connections per worker. Matches the reqwest pool the HTTP path holds, so the
/// two transports place the same load on the server and the comparison is about
/// protocol rather than concurrency.
const POOL_SIZE: usize = 5;

thread_local! {
    /// `None` until the first use; `Some(Err)` once a connection attempt has
    /// failed, so a broken driver setup degrades to HTTP instead of retrying a
    /// dead endpoint on every query.
    static CLIENT: RefCell<Option<Result<SoliDBClient, String>>> = const { RefCell::new(None) };
}

/// Is the native driver transport switched on?
pub fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("SOLI_DB_DRIVER").as_deref() == Ok("1"))
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
    *ON.get_or_init(|| std::env::var("SOLI_DB_DRIVER_QUERY").as_deref() != Ok("0"))
}

/// Opt out of SoliDB's read-result memoization on the driver path too.
/// Mirrors the HTTP `payload["cache"] = false` in `crud.rs`.
fn no_query_cache() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("SOLI_DB_NO_QUERY_CACHE").as_deref() == Ok("1"))
}

/// `host:port` for the driver, taken from the same `SOLIDB_HOST` the HTTP path
/// uses. The driver speaks raw TCP, so the scheme is dropped — and a TLS host is
/// refused rather than silently downgraded to plaintext.
fn driver_addr() -> Result<String, String> {
    if DB_CONFIG.scheme.starts_with("https") {
        return Err("SOLI_DB_DRIVER does not support TLS hosts; refusing to downgrade".into());
    }
    Ok(DB_CONFIG.host.clone())
}

/// Authenticate every pooled socket, matching the credentials the HTTP model
/// path would use.
///
/// Preference order:
/// 1. `SOLIDB_API_KEY` — the common production path
/// 2. `SOLIDB_USERNAME` / `SOLIDB_PASSWORD` when either is set
/// 3. `admin` / `admin` for the local-dev default (same as SoliDB's bootstrap)
async fn authenticate(client: &mut SoliDBClient, database: &str) -> Result<(), String> {
    if let Ok(api_key) = std::env::var("SOLIDB_API_KEY") {
        if !api_key.is_empty() {
            return client
                .auth_with_api_key(database, &api_key)
                .await
                .map_err(|e| format!("driver auth (api key) failed: {e}"));
        }
    }

    let username = std::env::var("SOLIDB_USERNAME").unwrap_or_else(|_| "admin".into());
    let password = std::env::var("SOLIDB_PASSWORD").unwrap_or_else(|_| "admin".into());
    client
        .auth(database, &username, &password)
        .await
        .map_err(|e| format!("driver auth failed: {e}"))
}

/// Run `f` against this worker's driver client, connecting on first use.
///
/// Returns `None` when the driver is off or unusable, which is the caller's
/// signal to take the HTTP path.
fn with_client<T>(
    f: impl FnOnce(&mut SoliDBClient) -> Result<T, String>,
) -> Option<Result<T, String>> {
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
                    .map_err(|e| format!("driver connect failed: {e}"))?;
                let db = get_database_name();
                authenticate(&mut client, &db).await?;
                Ok::<_, String>(client)
            });
            if let Err(e) = &connected {
                eprintln!("[solidb_driver] {e}; falling back to HTTP");
            }
            *slot = Some(connected);
        }
        match slot.as_mut() {
            Some(Ok(client)) => Some(f(client)),
            // Connection failed earlier: stay on HTTP for this worker's lifetime.
            _ => None,
        }
    })
}

/// Insert one document. `None` means "not handled — use HTTP".
pub fn try_insert(
    collection: &str,
    document: &Value,
    key: Option<&str>,
) -> Option<Result<Value, String>> {
    let (coll, doc, key) = (
        collection.to_string(),
        document.clone(),
        key.map(str::to_string),
    );
    let db = get_database_name();
    with_client(move |client| {
        block_on_db(async move {
            client
                .insert(&db, &coll, key.as_deref(), doc)
                .await
                .map_err(|e| format!("driver insert failed: {e}"))
        })
    })
}

pub fn try_update(collection: &str, key: &str, document: &Value) -> Option<Result<Value, String>> {
    let (coll, k, doc) = (collection.to_string(), key.to_string(), document.clone());
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
                .map_err(|e| format!("driver update failed: {e}"))
        })
    })
}

pub fn try_delete(collection: &str, key: &str) -> Option<Result<Value, String>> {
    let (coll, k) = (collection.to_string(), key.to_string());
    let db = get_database_name();
    with_client(move |client| {
        block_on_db(async move {
            client
                .delete(&db, &coll, &k)
                .await
                .map(|()| Value::Null)
                .map_err(|e| format!("driver delete failed: {e}"))
        })
    })
}

pub fn try_get(collection: &str, key: &str) -> Option<Result<Value, String>> {
    let (coll, k) = (collection.to_string(), key.to_string());
    let db = get_database_name();
    with_client(move |client| {
        block_on_db(async move {
            client
                .get(&db, &coll, &k)
                .await
                .map_err(|e| format!("driver get failed: {e}"))
        })
    })
}

/// Run an SDBQL query and return its rows.
///
/// The driver hands back the whole result set in one response, so unlike the
/// HTTP cursor path there is no `has_more` batch to drain — which is also why
/// the 1,000-row cursor truncation that bit the HTTP path cannot happen here.
pub fn try_query(
    sdbql: &str,
    bind_vars: Option<std::collections::HashMap<String, Value>>,
) -> Option<Result<Vec<Value>, String>> {
    // Opt-in separately: measured slower than the HTTP cursor. See query_enabled().
    if !query_enabled() {
        return None;
    }
    let q = sdbql.to_string();
    let db = get_database_name();
    // Honour SOLI_DB_NO_QUERY_CACHE on the driver path the same way the HTTP
    // cursor payload does — otherwise the diagnostic only uncached one arm.
    let cache = !no_query_cache();
    with_client(move |client| {
        block_on_db(async move {
            client
                .query_with_cache(&db, &q, bind_vars, cache)
                .await
                .map_err(|e| format!("driver query failed: {e}"))
        })
    })
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
