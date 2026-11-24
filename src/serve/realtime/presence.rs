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

/// A presence message as it travels: the op, and where it stands among
/// those of its node.
///
/// A node's ops are published in order, but a `sync` is a snapshot: taken a
/// moment before a join and published a moment after it, it reached the
/// others last and took the join away again until the next beat (and gave a
/// user who had just left ten more seconds). The number says which came
/// first. A `sync` takes its own *before* reading the state, so whatever it
/// misses carries a higher one; the other ops take theirs once the change
/// is made. A receiver then drops a `sync` older than an op it has applied.
/// `0`: a node from before the numbering, taken as it always was.
#[derive(Serialize, Deserialize)]
struct Stamped {
    #[serde(default)]
    seq: u64,
    #[serde(flatten)]
    op: PresenceOp,
}

static NEXT_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn next_seq() -> u64 {
    NEXT_SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
}

fn publish_op(id: TenantId, op: &PresenceOp) {
    publish_stamped(id, next_seq(), op);
}

fn publish_stamped(id: TenantId, seq: u64, op: &PresenceOp) {
    if super::backend() == super::Backend::Local {
        return;
    }
    let stamped = Stamped {
        seq,
        op: op.clone(),
    };
    if let Ok(message) = serde_json::to_string(&stamped) {
        super::publish_for(id, Target::Presence, &message);
    }
}

/// The highest number among the ops applied from each node, per
/// application. Node names carry a per-process nonce, so a restarted node is
/// a new one; the map is emptied when it grows past any real cluster.
static LAST_OP: crate::serve::tenant::TenantValue<HashMap<String, u64>> =
    crate::serve::tenant::TenantValue::new(HashMap::new);

/// Whether an op numbered `seq` from `node` is applied, given the highest
/// op number applied from it so far; records it when it is an op.
fn admit(last_ops: &mut HashMap<String, u64>, node: &str, seq: u64, is_sync: bool) -> bool {
    if seq == 0 {
        return true;
    }
    let last = last_ops.get(node).copied().unwrap_or(0);
    if is_sync {
        return seq > last;
    }
    if seq > last {
        if last_ops.len() > 4096 {
            last_ops.clear();
        }
        last_ops.insert(node.to_string(), seq);
    }
    true
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

/// Send each diff to the local sockets of its channel — without waiting
/// for any of them. These run on the two tasks presence lives on (the
/// consumer of what other nodes say, the heartbeat): a client that kept its
/// connection open and stopped reading would hold them, and with them every
/// other channel's presence and this node's own beats.
pub async fn send_diffs(registry: &WebSocketRegistry, diffs: Vec<ChannelDiff>) {
    for (channel, diff) in diffs {
        let message = WebSocketRegistry::build_presence_diff(&diff);
        registry.offer_to_channel(&channel, &message).await;
    }
}

/// Publish this process's full presence and connection count. Returns
/// whether there was anything to state.
async fn publish_sync(registry: &WebSocketRegistry) -> bool {
    // Numbered before the state is read: see `Stamped`.
    let seq = next_seq();
    let snapshot = registry.local_presence_snapshot().await;
    let connections = registry.connection_count().await;
    let held = !snapshot.is_empty() || connections > 0;
    publish_stamped(
        tenant::current_id(),
        seq,
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
    let Ok(Stamped { seq, op }) = serde_json::from_str::<Stamped>(message) else {
        return false;
    };
    let Some(queue) = inbound() else {
        return false;
    };
    queue
        .send((tenant::current_id(), node.to_string(), seq, op))
        .is_ok()
}

type Inbound = (TenantId, String, u64, PresenceOp);

/// The ops received, applied one after the other in the order the bus
/// delivered them. Each used to get a task of its own, and two from one
/// node could be applied in either order.
fn inbound() -> Option<&'static tokio::sync::mpsc::UnboundedSender<Inbound>> {
    static QUEUE: std::sync::OnceLock<Option<tokio::sync::mpsc::UnboundedSender<Inbound>>> =
        std::sync::OnceLock::new();
    QUEUE
        .get_or_init(|| {
            let handle = crate::serve::websocket::try_runtime_handle()?;
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Inbound>();
            handle.spawn(async move {
                use futures_util::FutureExt;
                while let Some((id, node, seq, op)) = rx.recv().await {
                    // One consumer for the whole process: an op that panics
                    // must cost that op, not presence until the restart.
                    let taken = std::panic::AssertUnwindSafe(tenant::task_scope(
                        id,
                        take_in(node, seq, op),
                    ));
                    if taken.catch_unwind().await.is_err() {
                        eprintln!("[realtime] a presence op could not be applied (panic)");
                    }
                }
            });
            Some(tx)
        })
        .as_ref()
}

/// Apply one received op, under the application that received it.
async fn take_in(node: String, seq: u64, op: PresenceOp) {
    let registry = crate::serve::websocket::get_ws_registry();
    let hello = op == PresenceOp::Hello;
    let is_sync = matches!(op, PresenceOp::Sync { .. });
    if LAST_OP.write(|last_ops| admit(last_ops, &node, seq, is_sync)) {
        let diffs = apply(&registry, &node, op).await;
        send_diffs(&registry, diffs).await;
    } else if let PresenceOp::Sync { connections, .. } = op {
        // Its presence is older than what was heard since, but the node is
        // alive and this is its connection count: both are still taken.
        registry.note_remote_connections(&node, connections).await;
    }
    if hello && holds_anything(&registry).await {
        publish_sync(&registry).await;
    }
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

    #[test]
    fn a_sync_older_than_an_op_already_applied_is_dropped() {
        let mut last = HashMap::new();
        // A join numbered 5, then the snapshot that was taken before it.
        assert!(admit(&mut last, "b", 5, false));
        assert!(
            !admit(&mut last, "b", 3, true),
            "the stale sync was applied"
        );
        // The next beat's snapshot is newer than everything, and is taken.
        assert!(admit(&mut last, "b", 9, true));
        // An op that arrives late is still applied (it is idempotent), and
        // does not move the mark back.
        assert!(admit(&mut last, "b", 4, false));
        assert!(!admit(&mut last, "b", 4, true));
        // Another node has its own numbers.
        assert!(admit(&mut last, "c", 1, true));
        // A node from before the numbering: everything, as before.
        assert!(admit(&mut last, "old", 0, true));
        assert!(admit(&mut last, "old", 0, false));
    }

    #[test]
    fn a_stamped_op_reads_with_or_without_its_number() {
        let with: Stamped = serde_json::from_str(r#"{"seq":7,"op":"hello"}"#).unwrap();
        assert_eq!((with.seq, with.op), (7, PresenceOp::Hello));
        // What a node from before the numbering sends.
        let without: Stamped = serde_json::from_str(r#"{"op":"hello"}"#).unwrap();
        assert_eq!((without.seq, without.op), (0, PresenceOp::Hello));
        // And what such a node reads of ours: the op, the number ignored.
        let sent = serde_json::to_string(&Stamped {
            seq: 7,
            op: PresenceOp::Hello,
        })
        .unwrap();
        assert_eq!(
            serde_json::from_str::<PresenceOp>(&sent).unwrap(),
            PresenceOp::Hello
        );
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
