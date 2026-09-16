//! Changefeed subscriber: wakes the job poller when work actually appears.
//!
//! The poller's interval used to be the only way it learned about a job. That
//! put a floor on idle cost — one cron read plus one claim round-trip per
//! `poll_ms`, per process, forever — and several apps sharing a database made
//! that the dominant load on it.
//!
//! This holds a WebSocket subscription to SoliDB's `_jobs` changefeed and calls
//! [`wake::notify`] when something lands, which lets the poller's interval
//! become a backstop measured in tens of seconds instead of the mechanism.
//!
//! **The socket is a hint, never the source of truth.** SoliDB broadcasts over a
//! 100-slot `tokio::sync::broadcast` and a lagging subscriber is skipped
//! silently (`RecvError::Lagged(_) => continue`, with nothing sent to the
//! client), events do not cross cluster nodes, and every emission is a discarded
//! `let _ = send(...)`. So the poller keeps its own timer and its own claim
//! query; this only shortens the sleep.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};

use super::wake;
use crate::interpreter::builtins::model::db_config;

/// First reconnect delay, doubled on each failure.
const BACKOFF_START: Duration = Duration::from_millis(200);
/// Reconnect ceiling. Deliberately as long as `JWT_LOGIN_BACKOFF_SECS`: a
/// reconnect may have to log in again, and SoliDB rate-limits `/auth/login` to
/// 20/min per client IP, so a tighter loop would lock us out of our own retries.
const BACKOFF_MAX: Duration = Duration::from_secs(30);
/// Re-dial this long before the token expires. A socket authenticated at
/// connect time has no refresh path.
const TOKEN_REFRESH_MARGIN_SECS: u64 = 300;

/// Whether a subscription is currently established. The poller reads this to
/// choose between its long backstop and the original `poll_ms`.
static CONNECTED: AtomicBool = AtomicBool::new(false);

/// True when a changefeed subscription is live, so missing an event is unlikely
/// enough to justify a long backstop sleep.
pub fn is_connected() -> bool {
    CONNECTED.load(Ordering::Relaxed)
}

/// Whether this process can subscribe at all.
///
/// Two things rule it out, and both fall back to plain polling rather than
/// failing: a SQL backend has no changefeed, and the endpoint accepts only a
/// JWT in the query string — an app configured with an API key alone cannot
/// open the socket.
pub fn available() -> bool {
    crate::db::is_solidb() && db_config::get_jwt_token().is_some()
}

/// Start the subscriber on the server runtime.
///
/// Not on the poller thread: that thread has no ambient tokio context, and
/// blocking it is how a black-holed webhook once stalled lease renewal and the
/// cron tick at the same time.
///
/// Uses `tenant::spawn` rather than `tokio::spawn` because the credentials,
/// the connection registry and the JWT cache are all per-tenant; a bare spawn
/// would read the wrong application's configuration.
pub fn start(runtime_handle: &tokio::runtime::Handle) {
    if !crate::db::is_solidb() {
        return;
    }
    let _guard = runtime_handle.enter();
    crate::serve::tenant::spawn(async move {
        run().await;
    });
}

async fn run() {
    let mut backoff = BACKOFF_START;
    // Log once per outage rather than once per attempt: a database that is down
    // for an hour should not produce an hour of identical lines.
    let mut announced_failure = false;

    loop {
        match subscribe_and_pump().await {
            Ok(()) => {
                // Planned end: the token is near expiry and we re-dial with a
                // fresh one. No backoff, nothing is wrong.
                CONNECTED.store(false, Ordering::Relaxed);
                backoff = BACKOFF_START;
                announced_failure = false;
            }
            Err(e) => {
                let had_socket = CONNECTED.swap(false, Ordering::Relaxed);
                if had_socket || !announced_failure {
                    eprintln!("[jobs] changefeed unavailable: {e} (falling back to polling)");
                    announced_failure = true;
                }
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(BACKOFF_MAX);
            }
        }
    }
}

/// Connect, subscribe, and relay events until the socket ends or the token is
/// about to expire.
async fn subscribe_and_pump() -> Result<(), String> {
    let Some(token) = db_config::get_jwt_token() else {
        return Err("no JWT available for the changefeed".to_string());
    };
    let database = db_config::get_database_name();
    let url = format!("{}?token={}", db_config::get_changefeed_url(), token);

    let (mut socket, _response) = tokio_tungstenite::connect_async(&url)
        .await
        .map_err(|e| format!("connect failed: {e}"))?;

    for collection in [super::JOBS_COLLECTION, super::CRON_COLLECTION] {
        let subscribe = serde_json::json!({
            "type": "subscribe",
            "database": database,
            "collection": collection,
        });
        socket
            .send(tokio_tungstenite::tungstenite::Message::Text(
                subscribe.to_string(),
            ))
            .await
            .map_err(|e| format!("subscribe to {collection} failed: {e}"))?;
    }

    // Wait for the acknowledgement before declaring the socket healthy. A
    // subscription can be refused — an unknown collection, a permission the
    // token lacks — and the server leaves the connection open when it is, so
    // assuming success here would earn the long backstop with nothing actually
    // listening.
    await_subscription_ack(&mut socket).await?;

    CONNECTED.store(true, Ordering::Relaxed);
    // Sweep once on every (re)connect: anything enqueued while we were away
    // produced an event nobody was listening for.
    wake::notify();

    let deadline = token_deadline(&token);
    tokio::pin!(deadline);

    loop {
        tokio::select! {
            _ = &mut deadline => {
                // Re-dial with a fresh token rather than waiting to be kicked.
                let _ = socket.close(None).await;
                return Ok(());
            }
            frame = socket.next() => {
                let Some(frame) = frame else {
                    return Err("changefeed closed by server".to_string());
                };
                match frame {
                    // tungstenite answers Ping automatically as long as the
                    // stream keeps being polled, which is what this loop does.
                    Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => handle_frame(&text),
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

/// Read frames until `_jobs` is acknowledged, or the server refuses it.
///
/// Only `_jobs` is load-bearing. A refusal on `_cron_jobs` is reported and
/// tolerated: cron still fires on the poller's own deadline.
async fn await_subscription_ack(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> Result<(), String> {
    let deadline = tokio::time::sleep(Duration::from_secs(15));
    tokio::pin!(deadline);

    loop {
        tokio::select! {
            _ = &mut deadline => return Err("changefeed did not acknowledge the subscription".to_string()),
            frame = socket.next() => {
                let Some(frame) = frame else {
                    return Err("changefeed closed before acknowledging".to_string());
                };
                let Ok(tokio_tungstenite::tungstenite::Message::Text(text)) = frame else {
                    continue;
                };
                let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
                    continue;
                };
                if let Some(error) = value.get("error").and_then(|v| v.as_str()) {
                    return Err(format!("subscription refused: {error}"));
                }
                if value.get("type").and_then(|v| v.as_str()) == Some("subscribed") {
                    // The acks arrive in the order we asked; `_jobs` is first,
                    // so the first one is the one that matters.
                    return Ok(());
                }
                // An event can arrive before the ack; it still means work.
                if should_wake(&value) {
                    wake::notify();
                }
            }
        }
    }
}

/// Sleep until the token is close enough to expiry that we should re-dial.
async fn token_deadline(token: &str) {
    let expires_at = db_config::token_expires_at(token);
    let now = super::unix_now().max(0) as u64;
    let remaining = expires_at
        .saturating_sub(now)
        .saturating_sub(TOKEN_REFRESH_MARGIN_SECS);
    // A token with no readable `exp` gets a fixed re-dial rather than never
    // being refreshed.
    let secs = if expires_at == 0 { 3_600 } else { remaining };
    tokio::time::sleep(Duration::from_secs(secs.max(60))).await;
}

/// Decide what one changefeed frame means for the poller.
fn handle_frame(text: &str) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return;
    };

    // Control frames: `{"type":"subscribed"}` and `{"error":...}`. An error is
    // per-subscription and leaves the socket open, so report it and carry on —
    // the backstop still covers that collection.
    if let Some(error) = value.get("error").and_then(|v| v.as_str()) {
        eprintln!("[jobs] changefeed subscription refused: {error}");
        return;
    }
    if should_wake(&value) {
        wake::notify();
    }
}

/// Whether an event is worth waking the poller for.
///
/// Two filters matter. Our own writes come straight back at us — every claim,
/// lease renewal, completion and pruned row — and without filtering them a long
/// job would wake the poller once per lease renewal, forever. And a job
/// enqueued for later should move the sleep deadline rather than trigger a pass
/// that will find nothing due.
fn should_wake(event: &serde_json::Value) -> bool {
    // Control frames (`{"type":"subscribed"}`) carry no operation and mean
    // nothing has changed yet.
    if event.get("operation").is_none() {
        return false;
    }

    let data = event.get("data");

    if let Some(locked_by) = data
        .and_then(|d| d.get("locked_by"))
        .and_then(|v| v.as_str())
    {
        if locked_by == super::worker_identity() {
            return false;
        }
    }

    if let Some(run_at) = data.and_then(|d| d.get("run_at")).and_then(|v| v.as_str()) {
        if let Some(at) = super::parse_iso_secs(run_at) {
            if at > super::unix_now() {
                super::note_future_run_at(at);
                return false;
            }
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn control_frames_are_not_events() {
        assert!(!should_wake(
            &json!({"type": "subscribed", "collection": "_jobs"})
        ));
    }

    #[test]
    fn a_foreign_insert_wakes_the_poller() {
        assert!(should_wake(&json!({
            "operation": "insert",
            "collection": "_jobs",
            "key": "abc",
            "data": {"_key": "abc", "state": "pending"},
        })));
    }

    #[test]
    fn our_own_claim_does_not_wake_us() {
        let event = json!({
            "operation": "update",
            "collection": "_jobs",
            "key": "abc",
            "data": {"_key": "abc", "state": "running", "locked_by": crate::jobs::worker_identity()},
        });
        assert!(
            !should_wake(&event),
            "echo of our own write must not wake the poller — a long job renews \
             its lease every pass and would otherwise wake us each time"
        );
    }

    #[test]
    fn another_process_claiming_still_wakes_us() {
        let event = json!({
            "operation": "update",
            "collection": "_jobs",
            "key": "abc",
            "data": {"_key": "abc", "state": "running", "locked_by": "otherhost:999"},
        });
        assert!(should_wake(&event));
    }

    #[test]
    fn a_future_job_moves_the_deadline_instead_of_waking() {
        let future = crate::jobs::iso_from_unix(crate::jobs::unix_now() + 600);
        let event = json!({
            "operation": "insert",
            "collection": "_jobs",
            "key": "later",
            "data": {"_key": "later", "state": "scheduled", "run_at": future},
        });
        assert!(
            !should_wake(&event),
            "a job due in ten minutes must not trigger a claim pass now"
        );
        assert!(
            crate::jobs::next_future_run_at(crate::jobs::unix_now()).is_some(),
            "it must be remembered as a deadline instead"
        );
    }

    #[test]
    fn a_due_job_wakes_us() {
        let past = crate::jobs::iso_from_unix(crate::jobs::unix_now() - 5);
        let event = json!({
            "operation": "insert",
            "collection": "_jobs",
            "key": "now",
            "data": {"_key": "now", "state": "pending", "run_at": past},
        });
        assert!(should_wake(&event));
    }
}
