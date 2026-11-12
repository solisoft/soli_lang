//! Realtime fan-out across processes: `broadcast`, `Model.broadcast`,
//! `ws_broadcast`, `ws_broadcast_room`, the handler `broadcast*` actions and
//! `sse_broadcast`.
//!
//! Every one of them delivers to this process's WebSockets and SSE streams
//! first, exactly as before. Then, when a bus is configured, it hands an
//! [`Envelope`] to a publisher thread and returns: the caller never waits on
//! the bus. Every other `soli serve` process subscribed to the bus decodes the
//! envelope and delivers it to *its* sockets and streams — locally only, never
//! publishing again, and never re-delivering its own envelopes.
//!
//! The bus is opt-in, chosen by `SOLI_REALTIME_BUS`:
//!
//! * `local` (the default) publishes nothing: one process, today's behavior.
//! * `solidb` writes each envelope to the engine-owned `_rt_bus` collection
//!   and reads the others' off its changefeed — the transport `live_update`
//!   already uses, pinned to the application's base database.
//! * `solikv` (also `redis`, `valkey`) publishes on a RESP channel of the
//!   SoliKV the app names with `SOLIKV_RESP_HOST`, and holds one `SUBSCRIBE`
//!   connection per process.
//!
//! **Fire and forget.** The changefeed drops events for a lagging subscriber
//! and does not cross SoliDB cluster nodes, so a message can be lost — the
//! same contract as a Phoenix PubSub broadcast. Return values stay local: the
//! count `broadcast` returns is this process's SSE subscribers.

pub mod presence;
pub mod solidb;
pub mod solikv;

use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::serve::tenant::{self, TenantId, TenantValue};

/// Version of the envelope format; a receiver ignores any other.
pub const ENVELOPE_VERSION: u32 = 1;

/// The largest message a bus carries. A larger one is still delivered
/// locally, but not published, and a receiver ignores one it is sent.
pub const MAX_MESSAGE_BYTES: usize = 256 * 1024;

/// Envelopes waiting for the publisher thread. A full queue drops the newest
/// (counted in `dropped`) instead of blocking the caller.
const QUEUE_CAPACITY: usize = 4096;

/// Which bus carries broadcasts to the other processes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// No bus: everything stays in this process.
    Local,
    /// The `_rt_bus` SoliDB collection and its changefeed.
    Solidb,
    /// `PUBLISH` / `SUBSCRIBE` on SoliKV, Redis or Valkey.
    Solikv,
}

impl Backend {
    pub fn name(self) -> &'static str {
        match self {
            Backend::Local => "local",
            Backend::Solidb => "solidb",
            Backend::Solikv => "solikv",
        }
    }

    /// Parse a `SOLI_REALTIME_BUS` value. Unset or empty is `local`.
    pub fn parse(value: Option<&str>) -> Result<Backend, String> {
        match value.map(str::trim).unwrap_or("") {
            "" | "local" => Ok(Backend::Local),
            "solidb" => Ok(Backend::Solidb),
            "solikv" | "redis" | "valkey" => Ok(Backend::Solikv),
            other => Err(format!(
                "SOLI_REALTIME_BUS={other} is not a realtime bus (local, solidb, solikv)"
            )),
        }
    }
}

/// The configured backend, read once per process from `SOLI_REALTIME_BUS`. An
/// unknown value is reported once and treated as `local`.
pub fn backend() -> Backend {
    static BACKEND: OnceLock<Backend> = OnceLock::new();
    *BACKEND.get_or_init(|| {
        let value = crate::platform::env::var("SOLI_REALTIME_BUS").ok();
        Backend::parse(value.as_deref()).unwrap_or_else(|e| {
            eprintln!("[realtime] {e}; broadcasts stay in this process");
            Backend::Local
        })
    })
}

/// Who a message is for. Serialized flat into the envelope under `kind`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Target {
    /// `broadcast` / `Model.broadcast`: the WebSocket channel *and* the SSE
    /// topic of that name.
    Broadcast { channel: String },
    /// `ws_broadcast_room`, the `broadcast_room` / `broadcast_channel` actions.
    WsChannel { channel: String },
    /// `ws_broadcast`, the `broadcast` action: every WebSocket.
    WsAll,
    /// `sse_broadcast`: one SSE topic, with an optional event name.
    Sse {
        channel: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        event: Option<String>,
    },
    /// WebSocket presence: the message is a [`presence::PresenceOp`].
    Presence,
    /// `ws_send` to a connection held by another process; the message is
    /// the payload. Only the process holding `conn` delivers it.
    WsSend { conn: String },
    /// `ws_close` likewise; the message is the reason.
    WsClose { conn: String },
}

/// This process on the bus: `HOSTNAME:pid` plus a random suffix. The suffix
/// matters: two containers without `HOSTNAME` both run as pid 1, and would
/// otherwise take each other's envelopes for their own echo and drop them.
pub fn origin() -> &'static str {
    static ORIGIN: OnceLock<String> = OnceLock::new();
    ORIGIN.get_or_init(|| {
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        format!("{}/{}", crate::jobs::worker_identity(), &nonce[..12])
    })
}

/// What travels on the bus.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    pub v: u32,
    /// The publishing process ([`origin`]); a process drops its own.
    pub origin: String,
    #[serde(flatten)]
    pub target: Target,
    /// The wire payload, already serialized.
    pub message: String,
    /// Unix seconds, for pruning.
    pub at: i64,
}

impl Envelope {
    /// An envelope from this process.
    pub fn new(target: Target, message: String) -> Self {
        Envelope {
            v: ENVELOPE_VERSION,
            origin: origin().to_string(),
            target,
            message,
            at: crate::jobs::unix_now(),
        }
    }

    /// Decode a stored envelope, or `None` for anything this process must not
    /// deliver: another version, its own echo, an oversized message, a
    /// document that is not an envelope.
    pub fn decode(document: serde_json::Value) -> Option<Envelope> {
        let envelope: Envelope = serde_json::from_value(document).ok()?;
        if envelope.v != ENVELOPE_VERSION
            || envelope.origin == origin()
            || envelope.message.len() > MAX_MESSAGE_BYTES
        {
            return None;
        }
        Some(envelope)
    }
}

/// Deliver to this process's WebSockets and SSE streams. Returns the SSE
/// subscribers reached; WebSocket delivery is asynchronous, and skipped when
/// no server runs in this process.
pub fn deliver_local(target: &Target, message: &str) -> usize {
    use crate::interpreter::builtins::streaming::broadcast_sse;
    match target {
        Target::Broadcast { channel } => {
            ws_to_channel(channel, message);
            broadcast_sse(channel, message, None)
        }
        Target::WsChannel { channel } => {
            ws_to_channel(channel, message);
            0
        }
        Target::WsAll => {
            if let Some(handle) = crate::serve::websocket::try_runtime_handle() {
                let registry = crate::serve::websocket::get_ws_registry();
                let message = message.to_string();
                handle.spawn(async move { registry.broadcast_all(&message).await });
            }
            0
        }
        Target::Sse { channel, event } => broadcast_sse(channel, message, event.as_deref()),
        // Presence is applied where the registry is, by `receive`; locally
        // the track that produced it already happened.
        Target::Presence => 0,
        Target::WsSend { conn } | Target::WsClose { conn } => {
            let Ok(id) = conn.parse::<uuid::Uuid>() else {
                return 0;
            };
            if let Some(handle) = crate::serve::websocket::try_runtime_handle() {
                let registry = crate::serve::websocket::get_ws_registry();
                let message = message.to_string();
                let close = matches!(target, Target::WsClose { .. });
                // An id this process does not hold is somebody else's.
                handle.spawn(async move {
                    if close {
                        registry.close(&id, &message).await;
                    } else {
                        let _ = registry.send_to(&id, &message).await;
                    }
                });
            }
            0
        }
    }
}

fn ws_to_channel(channel: &str, message: &str) {
    if let Some(handle) = crate::serve::websocket::try_runtime_handle() {
        let registry = crate::serve::websocket::get_ws_registry();
        let channel = channel.to_string();
        let message = message.to_string();
        handle.spawn(async move { registry.broadcast_to_channel(&channel, &message).await });
    }
}

/// Deliver here, then publish for the other processes unless `local_only`.
/// Returns what [`deliver_local`] returns.
pub fn fanout(target: Target, message: &str, local_only: bool) -> usize {
    let delivered = deliver_local(&target, message);
    if !local_only {
        publish(target, message);
    }
    delivered
}

/// Hand `message` to the bus for the other processes, as the application this
/// thread serves. Never blocks; `false` when nothing was queued (no bus, too
/// large, queue full).
pub fn publish(target: Target, message: &str) -> bool {
    publish_for(tenant::current_id(), target, message)
}

/// [`publish`] for an explicit application, from a task that does not carry
/// its tenant (a plain `runtime_handle.spawn`).
pub fn publish_for(id: TenantId, target: Target, message: &str) -> bool {
    if backend() == Backend::Local {
        return false;
    }
    if message.len() > MAX_MESSAGE_BYTES {
        tenant::scoped(id, || {
            note_dropped(&format!(
                "not published: a {} byte message is over the {MAX_MESSAGE_BYTES} byte bus limit",
                message.len()
            ))
        });
        return false;
    }
    let Some(queue) = publisher() else {
        return false;
    };
    match queue.try_send((id, Envelope::new(target, message.to_string()))) {
        Ok(()) => true,
        Err(TrySendError::Full(_)) => {
            tenant::scoped(id, || note_dropped("not published: the bus queue is full"));
            false
        }
        Err(TrySendError::Disconnected(_)) => false,
    }
}

/// The publisher thread's queue, started on first use.
fn publisher() -> Option<&'static SyncSender<(TenantId, Envelope)>> {
    static QUEUE: OnceLock<Option<SyncSender<(TenantId, Envelope)>>> = OnceLock::new();
    QUEUE
        .get_or_init(|| match backend() {
            Backend::Solikv => spawn_publisher(solikv::publish),
            _ => spawn_publisher(solidb::publish),
        })
        .as_ref()
}

/// Start a thread that hands each queued envelope to `sink` in order, under
/// the application it was published for.
fn spawn_publisher(
    sink: impl Fn(&Envelope) -> Result<(), String> + Send + 'static,
) -> Option<SyncSender<(TenantId, Envelope)>> {
    let (tx, rx) = sync_channel::<(TenantId, Envelope)>(QUEUE_CAPACITY);
    let spawned = std::thread::Builder::new()
        .name("soli-realtime-publish".to_string())
        .spawn(move || pump(rx, sink));
    match spawned {
        Ok(_) => Some(tx),
        Err(e) => {
            eprintln!("[realtime] could not start the bus publisher: {e}");
            None
        }
    }
}

fn pump(rx: Receiver<(TenantId, Envelope)>, sink: impl Fn(&Envelope) -> Result<(), String>) {
    for (id, envelope) in rx {
        tenant::scoped(id, || match sink(&envelope) {
            Ok(()) => {
                STATUS.write(|status| {
                    status.published += 1;
                    status.publish_error_logged = false;
                });
            }
            Err(e) => note_dropped(&format!("could not publish to the bus: {e}")),
        });
    }
}

/// Count a message that was not published, logging the first in a row.
fn note_dropped(why: &str) {
    let first = STATUS.write(|status| {
        status.dropped += 1;
        !std::mem::replace(&mut status.publish_error_logged, true)
    });
    if first {
        eprintln!("[realtime] {why}");
    }
}

/// Decode one stored envelope and deliver it here. The inbound path: it
/// never publishes. `false` when the document was not for this process.
pub fn receive(document: serde_json::Value) -> bool {
    let Some(envelope) = Envelope::decode(document) else {
        return false;
    };
    if envelope.target == Target::Presence {
        // A malformed op, or no server to hold presence: not received.
        if !presence::receive(&envelope.origin, &envelope.message) {
            return false;
        }
    } else {
        deliver_local(&envelope.target, &envelope.message);
    }
    STATUS.write(|status| {
        status.received += 1;
        status.last_rx_at = Some(crate::jobs::unix_now());
    });
    true
}

/// Counters for `realtime_bus()`, per application.
#[derive(Clone, Debug, Default)]
pub struct Status {
    pub published: u64,
    pub received: u64,
    pub dropped: u64,
    pub last_rx_at: Option<i64>,
    publish_error_logged: bool,
}

static STATUS: TenantValue<Status> = TenantValue::new(Status::default);

/// Set while this application's bus subscription is established.
pub(crate) static CONNECTED: TenantValue<bool> = TenantValue::new(|| false);

/// What `realtime_bus()` reports.
pub fn status() -> (Backend, bool, Status) {
    (
        backend(),
        CONNECTED.read(|connected| *connected),
        STATUS.read(Status::clone),
    )
}

/// Start the bus subscriber beside `soli serve`, if a bus is configured. The
/// standalone job worker does not call this: it only publishes.
pub fn start(runtime_handle: &tokio::runtime::Handle) {
    match backend() {
        Backend::Local => {}
        Backend::Solidb => {
            if !crate::db::is_solidb() {
                eprintln!(
                    "[realtime] SOLI_REALTIME_BUS=solidb needs a SoliDB database; \
                     broadcasts stay in this process"
                );
                return;
            }
            solidb::start(runtime_handle);
            presence::start(runtime_handle);
        }
        Backend::Solikv => {
            solikv::start(runtime_handle);
            presence::start(runtime_handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn foreign(target: Target, message: &str) -> serde_json::Value {
        let mut envelope = Envelope::new(target, message.to_string());
        envelope.origin = "other-host:1".to_string();
        serde_json::to_value(envelope).unwrap()
    }

    #[test]
    fn the_origin_is_the_worker_identity_plus_a_per_process_nonce() {
        let prefix = format!("{}/", crate::jobs::worker_identity());
        assert!(origin().starts_with(&prefix), "{}", origin());
        assert_eq!(origin().len(), prefix.len() + 12);
        assert_eq!(origin(), origin(), "stable for the life of the process");
    }

    #[test]
    fn the_backend_is_local_unless_one_is_named() {
        assert_eq!(Backend::parse(None), Ok(Backend::Local));
        assert_eq!(Backend::parse(Some("")), Ok(Backend::Local));
        assert_eq!(Backend::parse(Some("local")), Ok(Backend::Local));
        assert_eq!(Backend::parse(Some(" solidb ")), Ok(Backend::Solidb));
        assert_eq!(Backend::parse(Some("solikv")), Ok(Backend::Solikv));
        assert_eq!(Backend::parse(Some("redis")), Ok(Backend::Solikv));
        assert!(Backend::parse(Some("kafka")).is_err());
    }

    #[test]
    fn an_envelope_round_trips_with_its_kind_flattened() {
        let envelope = Envelope::new(
            Target::Sse {
                channel: "news".into(),
                event: Some("story".into()),
            },
            "hello".into(),
        );
        let stored = serde_json::to_value(&envelope).unwrap();
        assert_eq!(stored["kind"], "sse");
        assert_eq!(stored["channel"], "news");
        assert_eq!(stored["event"], "story");
        assert_eq!(stored["v"], ENVELOPE_VERSION);
        let back: Envelope = serde_json::from_value(stored).unwrap();
        assert_eq!(back, envelope);

        let all = serde_json::to_value(Envelope::new(Target::WsAll, "x".into())).unwrap();
        assert_eq!(all["kind"], "ws_all");
        assert!(all.get("channel").is_none());
    }

    #[test]
    fn a_stored_row_with_engine_fields_still_decodes() {
        let mut row = foreign(
            Target::Broadcast {
                channel: "posts".into(),
            },
            "m",
        );
        row["_key"] = json!("123");
        row["_rev"] = json!("abc");
        let envelope = Envelope::decode(row).unwrap();
        assert_eq!(
            envelope.target,
            Target::Broadcast {
                channel: "posts".into()
            }
        );
    }

    #[test]
    fn our_own_echo_other_versions_garbage_and_oversized_are_dropped() {
        let own = serde_json::to_value(Envelope::new(Target::WsAll, "x".into())).unwrap();
        assert!(Envelope::decode(own).is_none());

        let mut old = foreign(Target::WsAll, "x");
        old["v"] = json!(99);
        assert!(Envelope::decode(old).is_none());

        assert!(Envelope::decode(json!({"origin": "x:1", "message": "m"})).is_none());
        assert!(Envelope::decode(
            json!({"kind": "nope", "v": 1, "origin": "x:1", "message": "m", "at": 0})
        )
        .is_none());

        let big = "a".repeat(MAX_MESSAGE_BYTES + 1);
        assert!(Envelope::decode(foreign(Target::WsAll, &big)).is_none());
    }

    #[test]
    fn a_foreign_sse_envelope_reaches_local_subscribers_without_republishing() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        crate::interpreter::builtins::streaming::register_subscriber("rt_spec_topic", tx);
        let before = STATUS.read(Status::clone);

        assert!(receive(foreign(
            Target::Sse {
                channel: "rt_spec_topic".into(),
                event: Some("tick".into()),
            },
            "payload",
        )));

        let frame = String::from_utf8(rx.try_recv().unwrap()).unwrap();
        assert!(frame.contains("event: tick"), "{frame}");
        assert!(frame.contains("data: payload"), "{frame}");
        let after = STATUS.read(Status::clone);
        // Other tests receive on the same application concurrently.
        assert!(after.received > before.received);
        assert_eq!(after.published, 0, "inbound never publishes");
        assert!(after.last_rx_at.is_some());
    }

    #[test]
    fn a_broadcast_envelope_reaches_the_sse_topic_of_the_same_name() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        crate::interpreter::builtins::streaming::register_subscriber("rt_spec_channel", tx);
        receive(foreign(
            Target::Broadcast {
                channel: "rt_spec_channel".into(),
            },
            "{\"id\":1}",
        ));
        let frame = String::from_utf8(rx.try_recv().unwrap()).unwrap();
        assert!(frame.contains("data: {\"id\":1}"), "{frame}");
    }

    #[test]
    fn targeted_and_presence_envelopes_round_trip() {
        for target in [
            Target::WsSend {
                conn: uuid::Uuid::nil().to_string(),
            },
            Target::WsClose {
                conn: uuid::Uuid::nil().to_string(),
            },
            Target::Presence,
        ] {
            let stored = serde_json::to_value(Envelope::new(target.clone(), "m".into())).unwrap();
            let back: Envelope = serde_json::from_value(stored).unwrap();
            assert_eq!(back.target, target);
        }
        let stored = serde_json::to_value(Envelope::new(
            Target::WsSend { conn: "x".into() },
            "m".into(),
        ))
        .unwrap();
        assert_eq!(stored["kind"], "ws_send");
        // No server here: a send to an unknown or malformed id is nothing.
        assert_eq!(
            deliver_local(
                &Target::WsSend {
                    conn: "nope".into()
                },
                "m"
            ),
            0
        );
        assert!(!receive(foreign(Target::Presence, "not an op")));
    }

    #[test]
    fn with_no_bus_nothing_is_queued() {
        // The test process sets no SOLI_REALTIME_BUS.
        assert_eq!(backend(), Backend::Local);
        assert!(!publish(Target::WsAll, "x"));
    }

    #[test]
    fn the_publisher_runs_off_the_caller_thread_in_order() {
        let (seen_tx, seen_rx) = std::sync::mpsc::channel();
        let (gate_tx, gate_rx) = std::sync::mpsc::channel::<()>();
        let gate_rx = std::sync::Mutex::new(gate_rx);
        let queue = spawn_publisher(move |envelope| {
            // Hold the publisher until the caller has returned.
            let _ = gate_rx.lock().unwrap().recv();
            seen_tx
                .send((std::thread::current().id(), envelope.message.clone()))
                .unwrap();
            Ok(())
        })
        .unwrap();
        let id = tenant::current_id();
        for n in 0..3 {
            queue
                .try_send((id, Envelope::new(Target::WsAll, n.to_string())))
                .unwrap();
        }
        // Enqueueing returned while the sink was still blocked.
        for _ in 0..3 {
            gate_tx.send(()).unwrap();
        }
        let seen: Vec<_> = (0..3)
            .map(|_| {
                seen_rx
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .unwrap()
            })
            .collect();
        assert!(seen
            .iter()
            .all(|(thread, _)| *thread != std::thread::current().id()));
        let order: Vec<_> = seen.into_iter().map(|(_, message)| message).collect();
        assert_eq!(order, ["0", "1", "2"]);
    }
}
