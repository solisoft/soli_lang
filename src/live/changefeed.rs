//! A long-lived subscription to one engine-owned SoliDB collection's
//! changefeed, shared by the cross-process buses: `_live_bus` (`live_update`)
//! and `_rt_bus` (`broadcast`, `ws_broadcast*`, `sse_broadcast`).
//!
//! It makes sure the collection exists, subscribes, hands every text frame to
//! the bus's own decoder, prunes old rows once a minute, re-dials before the
//! JWT expires and reconnects with backoff when the socket drops. A failure is
//! logged once per outage, and never reaches a caller.
//!
//! **Pinned to the application's base database.** A request that called
//! `switch_tenant` runs its queries against the tenant's database, and so would
//! a publish made from it — into a database no subscriber watches. Every
//! database call here, and every publish through [`base_database`], clears the
//! per-request db tenancy first.

use std::time::Duration;

use serde_json::{json, Value as Json};

use crate::interpreter::builtins::model::db_config;
use crate::serve::tenant::{self, TenantValue};

const BACKOFF_START: Duration = Duration::from_millis(200);
const BACKOFF_MAX: Duration = Duration::from_secs(30);
const TOKEN_REFRESH_MARGIN_SECS: u64 = 300;
const PRUNE_EVERY: Duration = Duration::from_secs(60);

/// One bus's collection and what to do with what arrives on it.
pub struct Feed {
    /// The engine-owned collection (`_live_bus`, `_rt_bus`).
    pub collection: &'static str,
    /// Prefix of the log lines (`[live]`, `[realtime]`).
    pub log_prefix: &'static str,
    /// What stops working while the feed is down, for the log line.
    pub paused: &'static str,
    /// Set while the subscription is established, per application.
    pub connected: &'static TenantValue<bool>,
    /// Create the collection if needed (called under the base database).
    pub ensure: fn() -> Result<(), String>,
    /// Decode and deliver one text frame.
    pub on_frame: fn(&str),
    /// Rows older than this many seconds are deleted once a minute.
    pub prune_after_secs: i64,
}

/// Run `f` against the application's base database, whatever db tenancy the
/// calling thread is under.
pub fn base_database<R>(f: impl FnOnce() -> R) -> R {
    crate::db::tenancy::scoped(None, || db_config::with_base_database(f))
}

/// `spawn_blocking` for this task's application, against its base database.
/// The blocking pool does not carry the task's tenant binding, and the
/// database calls resolve their configuration through it.
pub async fn blocking_for_tenant<R: Send + 'static>(
    f: impl FnOnce() -> R + Send + 'static,
) -> Result<R, tokio::task::JoinError> {
    let id = tenant::current_id();
    tokio::task::spawn_blocking(move || tenant::scoped(id, || base_database(f))).await
}

/// Keep `feed` subscribed for the life of the process. Run it inside a
/// tenant-scoped task (`tenant::spawn`).
pub async fn run(feed: Feed) {
    let mut backoff = BACKOFF_START;
    let mut announced = false;
    // Give the database a moment and make sure the collection exists: the
    // changefeed refuses a subscription to one that does not.
    loop {
        let ready = blocking_for_tenant(feed.ensure).await;
        if matches!(ready, Ok(Ok(()))) {
            break;
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(BACKOFF_MAX);
    }
    backoff = BACKOFF_START;

    loop {
        match subscribe_and_pump(&feed).await {
            Ok(()) => {
                set_connected(&feed, false);
                backoff = BACKOFF_START;
                announced = false;
            }
            Err(e) => {
                let had_socket = set_connected(&feed, false);
                if had_socket || !announced {
                    eprintln!("{} bus unavailable: {e} ({})", feed.log_prefix, feed.paused);
                    announced = true;
                }
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(BACKOFF_MAX);
            }
        }
    }
}

fn set_connected(feed: &Feed, value: bool) -> bool {
    feed.connected
        .write(|connected| std::mem::replace(connected, value))
}

#[cfg(target_arch = "wasm32")]
async fn subscribe_and_pump(_feed: &Feed) -> Result<(), String> {
    // The subscriber only runs beside the native server; the edge build has
    // no long-lived socket to hold.
    Err(crate::platform::unsupported_on_edge(
        "a SoliDB changefeed subscription",
    ))
}

#[cfg(not(target_arch = "wasm32"))]
async fn subscribe_and_pump(feed: &Feed) -> Result<(), String> {
    use futures_util::{SinkExt, StreamExt};

    let (token, url, database) = base_database(|| {
        (
            db_config::get_jwt_token(),
            db_config::get_changefeed_url(),
            db_config::get_database_name(),
        )
    });
    let Some(token) = token else {
        return Err("no JWT available for the changefeed".to_string());
    };
    let url = format!("{url}?token={token}");
    let (mut socket, _response) = tokio_tungstenite::connect_async(&url)
        .await
        .map_err(|e| format!("connect failed: {e}"))?;

    let subscribe = json!({
        "type": "subscribe",
        "database": database,
        "collection": feed.collection,
    });
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            subscribe.to_string(),
        ))
        .await
        .map_err(|e| format!("subscribe failed: {e}"))?;

    let deadline = token_deadline(&token);
    tokio::pin!(deadline);
    let mut prune = tokio::time::interval(PRUNE_EVERY);
    prune.tick().await;

    loop {
        tokio::select! {
            _ = &mut deadline => {
                let _ = socket.close(None).await;
                return Ok(());
            }
            _ = prune.tick() => {
                let collection = feed.collection;
                let after = feed.prune_after_secs;
                let _ = blocking_for_tenant(move || prune_rows(collection, after)).await;
            }
            frame = socket.next() => {
                let Some(frame) = frame else {
                    return Err("changefeed closed by server".to_string());
                };
                match frame {
                    Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                        if let Ok(value) = serde_json::from_str::<Json>(&text) {
                            if value.get("type").and_then(Json::as_str) == Some("subscribed") {
                                set_connected(feed, true);
                            }
                            if let Some(error) = value.get("error").and_then(Json::as_str) {
                                return Err(format!("subscription refused: {error}"));
                            }
                        }
                        (feed.on_frame)(&text);
                    }
                    Ok(tokio_tungstenite::tungstenite::Message::Close(_)) => {
                        return Err("changefeed closed by server".to_string());
                    }
                    Ok(_) => {}
                    Err(e) => return Err(format!("changefeed read failed: {e}")),
                }
            }
        }
    }
}

async fn token_deadline(token: &str) {
    let expires_at = db_config::token_expires_at(token);
    let now = crate::jobs::unix_now().max(0) as u64;
    let remaining = expires_at
        .saturating_sub(now)
        .saturating_sub(TOKEN_REFRESH_MARGIN_SECS);
    let secs = if expires_at == 0 { 3_600 } else { remaining };
    tokio::time::sleep(Duration::from_secs(secs.max(60))).await;
}

/// The query that drops `collection`'s rows written more than `after_secs`
/// seconds before `now`.
pub fn prune_query(collection: &str, after_secs: i64, now: i64) -> String {
    let cutoff = now - after_secs;
    format!("FOR d IN {collection} FILTER d.at < {cutoff} REMOVE d IN {collection}")
}

/// Drop rows older than `after_secs`. Best effort.
pub fn prune_rows(collection: &'static str, after_secs: i64) {
    let query = prune_query(collection, after_secs, crate::jobs::unix_now());
    let _ = crate::interpreter::builtins::model::crud::exec_query(collection, query);
}

/// The `data` of a changefeed `insert` frame, or `None` for control frames,
/// deletes, updates and anything that does not parse.
pub fn inserted_document(text: &str) -> Option<Json> {
    let mut frame: Json = serde_json::from_str(text).ok()?;
    if frame.get("operation")?.as_str()? != "insert" {
        return None;
    }
    Some(frame.get_mut("data")?.take())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_inserts_carry_a_document() {
        let insert = json!({"operation": "insert", "data": {"a": 1}}).to_string();
        assert_eq!(inserted_document(&insert), Some(json!({"a": 1})));
        let delete = json!({"operation": "delete", "data": {"a": 1}}).to_string();
        assert_eq!(inserted_document(&delete), None);
        assert_eq!(inserted_document(r#"{"type":"subscribed"}"#), None);
        assert_eq!(inserted_document("not json"), None);
    }

    #[test]
    fn the_prune_query_cuts_at_now_minus_the_window() {
        assert_eq!(
            prune_query("_rt_bus", 120, 1_000),
            "FOR d IN _rt_bus FILTER d.at < 880 REMOVE d IN _rt_bus"
        );
    }

    #[test]
    fn base_database_pins_the_database_name_and_restores_the_override() {
        let expected = db_config::get_database_name();
        assert_eq!(base_database(db_config::get_database_name), expected);

        db_config::set_database_override("rt_spec_worker_db".to_string());
        assert_eq!(
            base_database(db_config::get_database_name),
            "rt_spec_worker_db",
            "a thread's own override (the parallel test runner's) is the base"
        );
        assert_eq!(db_config::get_database_name(), "rt_spec_worker_db");
    }

    #[test]
    fn base_database_clears_db_tenancy_and_restores_it() {
        let tenant = crate::db::tenancy::Tenant::new("acme").unwrap();
        crate::db::tenancy::scoped(Some(tenant), || {
            assert_eq!(base_database(crate::db::tenancy::current_key), None);
            assert_eq!(
                crate::db::tenancy::current_key().as_deref(),
                Some("acme"),
                "the request's tenant is back afterwards"
            );
        });
    }
}
