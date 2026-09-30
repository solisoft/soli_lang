//! Cross-process LiveView updates.
//!
//! `send_update` reaches the LiveView whose handler is running, and nothing
//! else. `live_update(component, assigns, opts)` reaches *any* attached
//! instance of a component — from a controller, a job, another worker thread,
//! or another OS process — by publishing to a shared bus and delivering to the
//! instances it can see locally.
//!
//! The bus is a SoliDB collection (`_live_bus`) and its changefeed, the same
//! transport the job engine already uses to learn about work. Every serving
//! process subscribes; a message carries the publisher's identity, and a
//! process ignores its own (it delivered locally before publishing). A
//! message names a component and optionally narrows to a `room`, a `session`,
//! or a nested `child` component inside each instance.
//!
//! **The socket is a hint, not a log.** SoliDB's changefeed drops events for a
//! lagging subscriber and does not cross cluster nodes, so a message can be
//! lost. That is the same contract as a Phoenix PubSub broadcast: fire and
//! forget, for state the next event will restate. Anything that must not be
//! missed belongs in the database, with the view re-reading it.

use std::sync::OnceLock;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value as Json};

use crate::interpreter::builtins::model::db_config;
use crate::serve::tenant::{self, TenantValue};

/// The bus collection. Underscore-prefixed like `_jobs`: engine-owned.
pub const BUS_COLLECTION: &str = "_live_bus";

/// Synthetic event name a worker receives for a delivered update.
pub const UPDATE_EVENT: &str = "live_update";

/// How long a bus row is kept before pruning. Subscribers read the row out of
/// the changefeed event itself, so the row only matters to a debugger.
const PRUNE_AFTER_SECS: i64 = 120;

const BACKOFF_START: Duration = Duration::from_millis(200);
const BACKOFF_MAX: Duration = Duration::from_secs(30);
const TOKEN_REFRESH_MARGIN_SECS: u64 = 300;

/// Per application: each one subscribes to its own database's bus.
static CONNECTED: TenantValue<bool> = TenantValue::new(|| false);

/// True while this application's changefeed subscription is established.
pub fn is_connected() -> bool {
    CONNECTED.read(|connected| *connected)
}

fn set_connected(value: bool) -> bool {
    CONNECTED.write(|connected| std::mem::replace(connected, value))
}

/// Secret carried by every server-originated `live_update` event. A client can
/// send any event name over its socket, including this one; only this process
/// knows the token, so only this process can make a worker apply an update.
pub fn internal_token() -> &'static str {
    static TOKEN: OnceLock<String> = OnceLock::new();
    TOKEN.get_or_init(|| uuid::Uuid::new_v4().to_string())
}

/// Is this worker event a `live_update` this process queued? A client can send
/// an event of that name too; it carries no token, and is an ordinary event
/// for the component's own handler.
pub fn is_server_update(event: &str, params: &Json) -> bool {
    event == UPDATE_EVENT
        && params.get("_bus_token").and_then(Json::as_str) == Some(internal_token())
}

/// Who an update is for.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Target {
    /// The `router_live` component name.
    pub component: String,
    /// Only instances mounted in this room (`data-live-room`).
    pub room: Option<String>,
    /// Only instances belonging to this session.
    pub session: Option<String>,
    /// Apply to this nested `live_component` inside each instance instead of
    /// merging onto the instance's own state.
    pub child: Option<String>,
}

/// What a broadcast did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Delivery {
    /// Instances in this process that were queued for the update.
    pub local: usize,
    /// Whether the message was also published for other processes.
    pub published: bool,
}

/// Deliver `assigns` to every matching instance here, then publish it for the
/// other processes. Local delivery never waits on the database, and a bus
/// failure is reported without undoing it.
pub fn broadcast(target: &Target, assigns: &Json) -> Result<Delivery, String> {
    if !assigns.is_object() {
        return Err("live_update expects an assigns hash".to_string());
    }
    let local = deliver_local(target, assigns);
    let published = match publish(target, assigns) {
        Ok(published) => published,
        Err(e) => {
            eprintln!("[live] could not publish live_update: {e}");
            false
        }
    };
    Ok(Delivery { local, published })
}

/// The `(liveview_id, component)` pairs in this process that `target` names.
pub fn matching_instances(target: &Target) -> Vec<(String, String)> {
    let room_prefix = target.room.as_deref().map(|room| format!("room:{room}:"));
    crate::live::view::live_registry()
        .attached_of_component(&target.component)
        .into_iter()
        .filter(|(id, _)| {
            room_prefix
                .as_ref()
                .is_none_or(|prefix| id.starts_with(prefix))
        })
        .filter(|(_, session)| {
            target
                .session
                .as_ref()
                .is_none_or(|wanted| wanted == session)
        })
        .map(|(id, _)| (id, target.component.clone()))
        .collect()
}

/// Queue `assigns` for each matching instance on this process.
pub fn deliver_local(target: &Target, assigns: &Json) -> usize {
    let instances = matching_instances(target);
    if instances.is_empty() {
        return 0;
    }
    crate::serve::enqueue_live_update(instances, target.child.as_deref(), assigns)
}

fn publish(target: &Target, assigns: &Json) -> Result<bool, String> {
    if !crate::db::is_solidb() {
        return Ok(false);
    }
    use crate::interpreter::builtins::model::crud;

    ensure_collection_once()?;
    let document = json!({
        "origin": crate::jobs::worker_identity(),
        "component": target.component,
        "room": target.room,
        "session": target.session,
        "child": target.child,
        "assigns": assigns,
        "at": crate::jobs::unix_now(),
    });
    crud::exec_insert(BUS_COLLECTION, None, document)?;
    Ok(true)
}

/// Create the bus collection in the current application's database, once per
/// application: in vhost mode each app has a database of its own.
fn ensure_collection_once() -> Result<(), String> {
    static DONE: TenantValue<bool> = TenantValue::new(|| false);
    if DONE.read(|done| *done) {
        return Ok(());
    }
    crate::interpreter::builtins::model::crud::ensure_collection(BUS_COLLECTION)?;
    DONE.write(|done| *done = true);
    Ok(())
}

/// `spawn_blocking` for this task's application. The blocking pool does not
/// carry the task's tenant binding, and the database calls resolve their
/// configuration through it.
async fn blocking_for_tenant<R: Send + 'static>(
    f: impl FnOnce() -> R + Send + 'static,
) -> Result<R, tokio::task::JoinError> {
    let id = tenant::current_id();
    tokio::task::spawn_blocking(move || tenant::scoped(id, f)).await
}

/// Decode one changefeed frame into a `(Target, assigns)` to deliver, or `None`
/// for control frames, our own echoes, deletes and malformed rows.
pub fn parse_frame(text: &str) -> Option<(Target, Json)> {
    let frame: Json = serde_json::from_str(text).ok()?;
    if frame.get("operation")?.as_str()? != "insert" {
        return None;
    }
    let data = frame.get("data")?;
    if data.get("origin")?.as_str()? == crate::jobs::worker_identity() {
        return None;
    }
    let assigns = data.get("assigns")?.clone();
    if !assigns.is_object() {
        return None;
    }
    let text_field = |name: &str| {
        data.get(name)
            .and_then(Json::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    Some((
        Target {
            component: text_field("component")?,
            room: text_field("room"),
            session: text_field("session"),
            child: text_field("child"),
        },
        assigns,
    ))
}

/// Start the subscriber on the server runtime. A no-op without SoliDB.
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
    let mut announced = false;
    // Give the database a moment and make sure the collection exists: the
    // changefeed refuses a subscription to one that does not.
    loop {
        let ready = blocking_for_tenant(ensure_collection_once).await;
        if matches!(ready, Ok(Ok(()))) {
            break;
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(BACKOFF_MAX);
    }
    backoff = BACKOFF_START;

    loop {
        match subscribe_and_pump().await {
            Ok(()) => {
                set_connected(false);
                backoff = BACKOFF_START;
                announced = false;
            }
            Err(e) => {
                let had_socket = set_connected(false);
                if had_socket || !announced {
                    eprintln!("[live] bus unavailable: {e} (cross-process live_update paused)");
                    announced = true;
                }
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(BACKOFF_MAX);
            }
        }
    }
}

async fn subscribe_and_pump() -> Result<(), String> {
    let Some(token) = db_config::get_jwt_token() else {
        return Err("no JWT available for the changefeed".to_string());
    };
    let url = format!("{}?token={}", db_config::get_changefeed_url(), token);
    let (mut socket, _response) = tokio_tungstenite::connect_async(&url)
        .await
        .map_err(|e| format!("connect failed: {e}"))?;

    let subscribe = json!({
        "type": "subscribe",
        "database": db_config::get_database_name(),
        "collection": BUS_COLLECTION,
    });
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            subscribe.to_string(),
        ))
        .await
        .map_err(|e| format!("subscribe failed: {e}"))?;

    let deadline = token_deadline(&token);
    tokio::pin!(deadline);
    let mut prune = tokio::time::interval(Duration::from_secs(60));
    prune.tick().await;

    loop {
        tokio::select! {
            _ = &mut deadline => {
                let _ = socket.close(None).await;
                return Ok(());
            }
            _ = prune.tick() => {
                let _ = blocking_for_tenant(prune_old_rows).await;
            }
            frame = socket.next() => {
                let Some(frame) = frame else {
                    return Err("changefeed closed by server".to_string());
                };
                match frame {
                    Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                        if let Ok(value) = serde_json::from_str::<Json>(&text) {
                            if value.get("type").and_then(Json::as_str) == Some("subscribed") {
                                set_connected(true);
                            }
                            if let Some(error) = value.get("error").and_then(Json::as_str) {
                                return Err(format!("subscription refused: {error}"));
                            }
                        }
                        if let Some((target, assigns)) = parse_frame(&text) {
                            deliver_local(&target, &assigns);
                        }
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

/// Drop rows older than [`PRUNE_AFTER_SECS`]. Best effort.
fn prune_old_rows() {
    let cutoff = crate::jobs::unix_now() - PRUNE_AFTER_SECS;
    let query =
        format!("FOR d IN {BUS_COLLECTION} FILTER d.at < {cutoff} REMOVE d IN {BUS_COLLECTION}");
    let _ = crate::interpreter::builtins::model::crud::exec_query(BUS_COLLECTION, query);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(origin: &str, operation: &str) -> String {
        json!({
            "operation": operation,
            "collection": BUS_COLLECTION,
            "data": {
                "origin": origin,
                "component": "board",
                "room": "lobby",
                "session": "",
                "child": "score",
                "assigns": {"score": 5},
            },
        })
        .to_string()
    }

    #[test]
    fn a_foreign_insert_becomes_a_delivery() {
        let (target, assigns) = parse_frame(&frame("other-host:1", "insert")).unwrap();
        assert_eq!(target.component, "board");
        assert_eq!(target.room.as_deref(), Some("lobby"));
        assert_eq!(target.session, None, "an empty string is no session filter");
        assert_eq!(target.child.as_deref(), Some("score"));
        assert_eq!(assigns["score"], 5);
    }

    #[test]
    fn our_own_echo_is_not_delivered_twice() {
        assert!(parse_frame(&frame(crate::jobs::worker_identity(), "insert")).is_none());
    }

    #[test]
    fn deletes_control_frames_and_garbage_are_ignored() {
        assert!(parse_frame(&frame("other-host:1", "delete")).is_none());
        assert!(parse_frame(r#"{"type":"subscribed"}"#).is_none());
        assert!(parse_frame("not json").is_none());
        let no_component = json!({
            "operation": "insert",
            "data": {"origin": "x:1", "assigns": {"a": 1}},
        });
        assert!(parse_frame(&no_component.to_string()).is_none());
        let scalar_assigns = json!({
            "operation": "insert",
            "data": {"origin": "x:1", "component": "c", "assigns": 3},
        });
        assert!(parse_frame(&scalar_assigns.to_string()).is_none());
    }

    #[test]
    fn broadcast_refuses_non_hash_assigns() {
        let target = Target {
            component: "board".into(),
            ..Target::default()
        };
        assert!(broadcast(&target, &json!(3)).is_err());
    }

    #[test]
    fn the_internal_token_is_stable_and_not_guessable() {
        assert_eq!(internal_token(), internal_token());
        assert!(internal_token().len() >= 32);
    }
}
