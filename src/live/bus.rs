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

use serde_json::{json, Value as Json};

use super::changefeed;
use crate::serve::tenant::TenantValue;

/// The bus collection. Underscore-prefixed like `_jobs`: engine-owned.
pub const BUS_COLLECTION: &str = "_live_bus";

/// Synthetic event name a worker receives for a delivered update.
pub const UPDATE_EVENT: &str = "live_update";

/// How long a bus row is kept before pruning. Subscribers read the row out of
/// the changefeed event itself, so the row only matters to a debugger.
const PRUNE_AFTER_SECS: i64 = 120;

/// Per application: each one subscribes to its own database's bus.
static CONNECTED: TenantValue<bool> = TenantValue::new(|| false);

/// True while this application's changefeed subscription is established.
pub fn is_connected() -> bool {
    CONNECTED.read(|connected| *connected)
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

    let document = json!({
        "origin": crate::jobs::worker_identity(),
        "component": target.component,
        "room": target.room,
        "session": target.session,
        "child": target.child,
        "assigns": assigns,
        "at": crate::jobs::unix_now(),
    });
    // Into the base database, where every subscriber listens, even from a
    // request that switched to a tenant's own database.
    changefeed::base_database(|| {
        ensure_collection_once()?;
        crud::exec_insert(BUS_COLLECTION, None, document).map(|_| true)
    })
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

/// Decode one changefeed frame into a `(Target, assigns)` to deliver, or `None`
/// for control frames, our own echoes, deletes and malformed rows.
pub fn parse_frame(text: &str) -> Option<(Target, Json)> {
    let document = changefeed::inserted_document(text)?;
    let data = &document;
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
        changefeed::run(changefeed::Feed {
            collection: BUS_COLLECTION,
            log_prefix: "[live]",
            paused: "cross-process live_update paused",
            connected: &CONNECTED,
            ensure: ensure_collection_once,
            on_frame: |text| {
                if let Some((target, assigns)) = parse_frame(text) {
                    deliver_local(&target, &assigns);
                }
            },
            prune_after_secs: PRUNE_AFTER_SECS,
        })
        .await;
    });
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
