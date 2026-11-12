//! WebSocket presence across instances.
//!
//! Each node stays the authority for its own connections' metas. With a
//! realtime bus configured, it also tells the other nodes what changes:
//!
//! * `track`, `untrack`, `set_presence` and a disconnect publish a `join`,
//!   `leave` or `update` for the meta concerned;
//! * every [`HEARTBEAT_EVERY`], a node that holds presence or connections (or
//!   held some at the previous beat) publishes a `sync` with its full state
//!   and its connection count, which the others take in place of what they
//!   knew of it — so a lost message heals at the next beat;
//! * a node not heard from for [`NODE_TTL`] is forgotten, and leaves are sent
//!   for its users;
//! * a node whose subscription comes up publishes a `hello`, and the others
//!   answer with a `sync` at once rather than at their next beat.
//!
//! Joins and leaves are decided cluster-wide: a `presence_diff` join goes out
//! for a user's first meta anywhere, a leave for the last. `ws_list_presence`,
//! `ws_presence_count` and `ws_get_presence` merge every node's metas.
//! Eventually consistent — the contract of Phoenix.Presence.

use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::Target;
use crate::serve::tenant::{self, TenantId};
use crate::serve::websocket::{ChannelDiff, PresenceMeta, PresenceSnapshot, WebSocketRegistry};

/// How often a node holding presence restates it.
pub const HEARTBEAT_EVERY: Duration = Duration::from_secs(10);

/// How long a silent node's presence is kept.
pub const NODE_TTL: Duration = Duration::from_secs(30);

/// A meta on the wire. `PresenceMeta` flattens its extra fields next to its
/// own, which reads back ambiguously when the extras repeat a field name
/// (`state` usually does); here they stay apart.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WireMeta {
    pub connection_id: Uuid,
    pub phx_ref: String,
    pub state: String,
    pub online_at: u64,
    #[serde(default)]
    pub extra: HashMap<String, String>,
}

impl From<&PresenceMeta> for WireMeta {
    fn from(meta: &PresenceMeta) -> Self {
        WireMeta {
            connection_id: meta.connection_id,
            phx_ref: meta.phx_ref.clone(),
            state: meta.state.clone(),
            online_at: meta.online_at,
            extra: meta.extra.clone(),
        }
    }
}

impl From<WireMeta> for PresenceMeta {
    fn from(meta: WireMeta) -> Self {
        PresenceMeta {
            connection_id: meta.connection_id,
            phx_ref: meta.phx_ref,
            state: meta.state,
            online_at: meta.online_at,
            extra: meta.extra,
        }
    }
}

type WireSnapshot = HashMap<String, HashMap<String, Vec<WireMeta>>>;

/// One presence message, carried as the envelope's `message`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum PresenceOp {
    Join {
        channel: String,
        user_id: String,
        meta: WireMeta,
    },
    Update {
        channel: String,
        user_id: String,
        meta: WireMeta,
    },
    Leave {
        channel: String,
        user_id: String,
        phx_ref: String,
    },
    Sync {
        state: WireSnapshot,
        /// The node's WebSocket connections, for `ws_count({"cluster": true})`.
        #[serde(default)]
        connections: usize,
    },
    Hello,
}

fn to_wire(snapshot: &PresenceSnapshot) -> WireSnapshot {
    snapshot
        .iter()
        .map(|(channel, users)| {
            let users = users
                .iter()
                .map(|(user, metas)| (user.clone(), metas.iter().map(WireMeta::from).collect()))
                .collect();
            (channel.clone(), users)
        })
        .collect()
}

fn from_wire(snapshot: WireSnapshot) -> PresenceSnapshot {
    snapshot
        .into_iter()
        .map(|(channel, users)| {
            let users = users
                .into_iter()
                .map(|(user, metas)| (user, metas.into_iter().map(PresenceMeta::from).collect()))
                .collect();
            (channel, users)
        })
        .collect()
}

fn publish_op(id: TenantId, op: &PresenceOp) {
    if super::backend() == super::Backend::Local {
        return;
    }
    if let Ok(message) = serde_json::to_string(op) {
        super::publish_for(id, Target::Presence, &message);
    }
}

/// Tell the other nodes this process tracked `meta`.
pub fn announce_join(id: TenantId, channel: &str, user_id: &str, meta: &PresenceMeta) {
    publish_op(
        id,
        &PresenceOp::Join {
            channel: channel.to_string(),
            user_id: user_id.to_string(),
            meta: meta.into(),
        },
    );
}

/// Tell the other nodes a meta of this process changed.
pub fn announce_update(id: TenantId, channel: &str, user_id: &str, meta: &PresenceMeta) {
    publish_op(
        id,
        &PresenceOp::Update {
            channel: channel.to_string(),
            user_id: user_id.to_string(),
            meta: meta.into(),
        },
    );
}

/// Tell the other nodes this process untracked a meta.
pub fn announce_leave(id: TenantId, channel: &str, user_id: &str, phx_ref: &str) {
    publish_op(
        id,
        &PresenceOp::Leave {
            channel: channel.to_string(),
            user_id: user_id.to_string(),
            phx_ref: phx_ref.to_string(),
        },
    );
}

/// Apply one op from `node` to `registry` and return the diffs for the
/// local sockets. `Hello` changes nothing here; the caller answers it.
pub async fn apply(registry: &WebSocketRegistry, node: &str, op: PresenceOp) -> Vec<ChannelDiff> {
    match op {
        PresenceOp::Join {
            channel,
            user_id,
            meta,
        } => registry
            .remote_join(node, &channel, &user_id, meta.into())
            .await
            .into_iter()
            .collect(),
        PresenceOp::Update {
            channel,
            user_id,
            meta,
        } => registry
            .remote_update(node, &channel, &user_id, meta.into())
            .await
            .into_iter()
            .collect(),
        PresenceOp::Leave {
            channel,
            user_id,
            phx_ref,
        } => registry
            .remote_leave(node, &channel, &user_id, &phx_ref)
            .await
            .into_iter()
            .collect(),
        PresenceOp::Sync { state, connections } => {
            let diffs = registry
                .replace_remote_node(node, Some(from_wire(state)))
                .await;
            registry.note_remote_connections(node, connections).await;
            diffs
        }
        PresenceOp::Hello => Vec::new(),
    }
}

/// Send each diff to the local sockets of its channel.
pub async fn send_diffs(registry: &WebSocketRegistry, diffs: Vec<ChannelDiff>) {
    for (channel, diff) in diffs {
        let message = WebSocketRegistry::build_presence_diff(&diff);
        registry.broadcast_to_channel(&channel, &message).await;
    }
}

/// Publish this process's full presence and connection count. Returns
/// whether there was anything to state.
async fn publish_sync(registry: &WebSocketRegistry) -> bool {
    let snapshot = registry.local_presence_snapshot().await;
    let connections = registry.connection_count().await;
    let held = !snapshot.is_empty() || connections > 0;
    publish_op(
        tenant::current_id(),
        &PresenceOp::Sync {
            state: to_wire(&snapshot),
            connections,
        },
    );
    held
}

/// Whether this process has presence or connections to tell the others of.
async fn holds_anything(registry: &WebSocketRegistry) -> bool {
    registry.connection_count().await > 0 || !registry.local_presence_snapshot().await.is_empty()
}

/// The inbound path for a `presence` envelope from `node`, under the
/// application that received it. Runs on the server runtime; without one (a
/// script, `soli worker`) there are no sockets to tell, and nothing to do.
pub fn receive(node: &str, message: &str) -> bool {
    let Ok(op) = serde_json::from_str::<PresenceOp>(message) else {
        return false;
    };
    let Some(handle) = crate::serve::websocket::try_runtime_handle() else {
        return false;
    };
    let registry = crate::serve::websocket::get_ws_registry();
    let node = node.to_string();
    let _guard = handle.enter();
    tenant::spawn(async move {
        let hello = op == PresenceOp::Hello;
        let diffs = apply(&registry, &node, op).await;
        send_diffs(&registry, diffs).await;
        if hello && holds_anything(&registry).await {
            publish_sync(&registry).await;
        }
    });
    true
}

/// The heartbeat beside the bus subscriber: a `hello` each time the
/// subscription comes up, a `sync` every beat while this node holds presence
/// (and once more after it stops), and the eviction of silent nodes.
pub fn start(runtime_handle: &tokio::runtime::Handle) {
    let _guard = runtime_handle.enter();
    tenant::spawn(async move {
        let mut beat = tokio::time::interval(HEARTBEAT_EVERY);
        let mut check = tokio::time::interval(Duration::from_secs(1));
        let mut was_connected = false;
        let mut held = false;
        loop {
            tokio::select! {
                _ = check.tick() => {
                    let connected = super::CONNECTED.read(|connected| *connected);
                    if connected && !was_connected {
                        publish_op(tenant::current_id(), &PresenceOp::Hello);
                    }
                    was_connected = connected;
                }
                _ = beat.tick() => {
                    let registry = crate::serve::websocket::get_ws_registry();
                    let holds_now = holds_anything(&registry).await;
                    if holds_now || held {
                        held = publish_sync(&registry).await;
                    }
                    let diffs = registry
                        .evict_silent_nodes(web_time::Instant::now(), NODE_TTL)
                        .await;
                    send_diffs(&registry, diffs).await;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(connection: u128, phx_ref: &str, state: &str) -> PresenceMeta {
        let mut extra = HashMap::new();
        extra.insert("state".to_string(), state.to_string());
        extra.insert("name".to_string(), "Ann".to_string());
        PresenceMeta {
            connection_id: Uuid::from_u128(connection),
            phx_ref: phx_ref.to_string(),
            state: state.to_string(),
            online_at: 1,
            extra,
        }
    }

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
    }

    #[test]
    fn an_op_round_trips_even_when_the_extras_repeat_a_field() {
        let op = PresenceOp::Join {
            channel: "room".into(),
            user_id: "u1".into(),
            meta: (&meta(1, "a.1", "online")).into(),
        };
        let text = serde_json::to_string(&op).unwrap();
        assert!(text.contains(r#""op":"join""#), "{text}");
        assert_eq!(serde_json::from_str::<PresenceOp>(&text).unwrap(), op);
        let hello = serde_json::to_string(&PresenceOp::Hello).unwrap();
        assert_eq!(hello, r#"{"op":"hello"}"#);
    }

    #[test]
    fn a_remote_join_is_a_diff_only_for_a_user_new_to_the_cluster() {
        rt().block_on(async {
            let registry = WebSocketRegistry::new();
            let local = Uuid::from_u128(9);
            registry.track(&local, "room", "u1", HashMap::new()).await;

            // u1 is already here: no diff, but the meta is merged.
            let diffs = apply(
                &registry,
                "node-b",
                PresenceOp::Join {
                    channel: "room".into(),
                    user_id: "u1".into(),
                    meta: (&meta(1, "b.1", "online")).into(),
                },
            )
            .await;
            assert!(diffs.is_empty());
            let listed = registry.list_presence("room").await;
            assert_eq!(listed.len(), 1);
            assert_eq!(listed[0].metas.len(), 2);

            // u2 is new everywhere: a join diff.
            let diffs = apply(
                &registry,
                "node-b",
                PresenceOp::Join {
                    channel: "room".into(),
                    user_id: "u2".into(),
                    meta: (&meta(2, "b.2", "online")).into(),
                },
            )
            .await;
            assert_eq!(diffs.len(), 1);
            assert!(diffs[0].1.joins.contains_key("u2"));
            assert_eq!(registry.presence_count("room").await, 2);
            assert_eq!(
                registry
                    .get_user_presence("room", "u2")
                    .await
                    .unwrap()
                    .metas[0]
                    .phx_ref,
                "b.2"
            );
        });
    }

    #[test]
    fn a_leave_is_sent_for_the_last_meta_anywhere() {
        rt().block_on(async {
            let registry = WebSocketRegistry::new();
            let join = |phx: &str, conn| PresenceOp::Join {
                channel: "room".into(),
                user_id: "u1".into(),
                meta: (&meta(conn, phx, "online")).into(),
            };
            apply(&registry, "node-b", join("b.1", 1)).await;
            apply(&registry, "node-c", join("c.1", 2)).await;
            let leave = |phx: &str| PresenceOp::Leave {
                channel: "room".into(),
                user_id: "u1".into(),
                phx_ref: phx.into(),
            };
            assert!(apply(&registry, "node-b", leave("b.1")).await.is_empty());
            let diffs = apply(&registry, "node-c", leave("c.1")).await;
            assert_eq!(diffs.len(), 1);
            assert!(diffs[0].1.leaves.contains_key("u1"));
            assert_eq!(registry.presence_count("room").await, 0);
            // A leave for a meta nobody knows is nothing.
            assert!(apply(&registry, "node-c", leave("c.9")).await.is_empty());
        });
    }

    #[test]
    fn a_local_track_is_not_new_when_another_node_has_the_user() {
        rt().block_on(async {
            let registry = WebSocketRegistry::new();
            apply(
                &registry,
                "node-b",
                PresenceOp::Join {
                    channel: "room".into(),
                    user_id: "u1".into(),
                    meta: (&meta(1, "b.1", "online")).into(),
                },
            )
            .await;
            let conn = Uuid::from_u128(5);
            let (is_new, tracked) = registry.track(&conn, "room", "u1", HashMap::new()).await;
            assert!(!is_new, "u1 is already on node-b");
            assert!(tracked
                .phx_ref
                .starts_with(crate::serve::websocket::node_tag()));
            let (was_last, _) = registry.untrack(&conn, "room").await.unwrap();
            assert!(!was_last, "u1 is still on node-b");
            let gone = registry.untrack_all(&conn).await;
            assert!(gone.is_empty());
        });
    }

    #[test]
    fn a_sync_replaces_the_node_and_diffs_both_ways() {
        rt().block_on(async {
            let registry = WebSocketRegistry::new();
            let mut first: WireSnapshot = HashMap::new();
            first
                .entry("room".into())
                .or_default()
                .insert("u1".into(), vec![(&meta(1, "b.1", "online")).into()]);
            let diffs = apply(
                &registry,
                "node-b",
                PresenceOp::Sync {
                    state: first,
                    connections: 3,
                },
            )
            .await;
            assert_eq!(diffs.len(), 1);
            assert!(diffs[0].1.joins.contains_key("u1"));

            // Next beat: u1 left (missed message), u2 arrived.
            let mut second: WireSnapshot = HashMap::new();
            second
                .entry("room".into())
                .or_default()
                .insert("u2".into(), vec![(&meta(2, "b.2", "online")).into()]);
            let diffs = apply(
                &registry,
                "node-b",
                PresenceOp::Sync {
                    state: second,
                    connections: 2,
                },
            )
            .await;
            assert_eq!(diffs.len(), 1);
            assert!(diffs[0].1.joins.contains_key("u2"));
            assert!(diffs[0].1.leaves.contains_key("u1"));
            let users: Vec<_> = registry
                .list_presence("room")
                .await
                .into_iter()
                .map(|p| p.user_id)
                .collect();
            assert_eq!(users, ["u2"]);
            assert_eq!(
                registry.cluster_connection_count().await,
                2,
                "no local connection, node-b's two"
            );
        });
    }

    #[test]
    fn a_silent_node_is_evicted_with_leaves() {
        rt().block_on(async {
            let registry = WebSocketRegistry::new();
            apply(
                &registry,
                "node-b",
                PresenceOp::Join {
                    channel: "room".into(),
                    user_id: "u1".into(),
                    meta: (&meta(1, "b.1", "online")).into(),
                },
            )
            .await;
            let now = web_time::Instant::now();
            assert!(registry.evict_silent_nodes(now, NODE_TTL).await.is_empty());
            let diffs = registry
                .evict_silent_nodes(now + NODE_TTL + Duration::from_secs(1), NODE_TTL)
                .await;
            assert_eq!(diffs.len(), 1);
            assert!(diffs[0].1.leaves.contains_key("u1"));
            assert_eq!(registry.remote_nodes().await, 0);
            assert_eq!(registry.presence_count("room").await, 0);
        });
    }

    #[test]
    fn a_remote_update_replaces_the_meta() {
        rt().block_on(async {
            let registry = WebSocketRegistry::new();
            let op = |state: &str| PresenceOp::Update {
                channel: "room".into(),
                user_id: "u1".into(),
                meta: (&meta(1, "b.1", state)).into(),
            };
            apply(&registry, "node-b", op("online")).await;
            let diffs = apply(&registry, "node-b", op("typing")).await;
            assert_eq!(diffs.len(), 1);
            let listed = registry.list_presence("room").await;
            assert_eq!(listed[0].metas.len(), 1);
            assert_eq!(listed[0].metas[0].state, "typing");
        });
    }
}
