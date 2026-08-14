//! LiveView registry and instance management.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_channel;
use serde::Serialize;
use tungstenite::Message;

/// Type alias for LiveView ID
pub type LiveViewId = String;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum ServerMessage {
    Render {
        html: String,
        liveview_id: String,
    },
    Patch {
        liveview_id: String,
        diff: String,
    },
    /// Targeted collection updates (append/prepend/insert/remove/reset) applied
    /// directly to a container by id — no full-list re-render or diff. The
    /// container's items stay out of the diff shadow so patches don't fight the
    /// streamed DOM (Phoenix LiveView streams / Turbo Streams model).
    Stream {
        liveview_id: String,
        ops: Vec<StreamOp>,
    },
    Redirect {
        url: String,
    },
    /// Swap this page-root LiveView to a different component socket
    /// (`/live/socket/<name>`) without a full page load.
    Live {
        url: String,
    },
    /// Update the address bar without leaving the socket (`history.pushState`
    /// or `replaceState`). Distinct from [`Redirect`], which does a full load.
    Url {
        url: String,
        #[serde(default)]
        replace: bool,
    },
    /// Safe client-side commands (no eval): add/remove/toggle class, set/remove
    /// attributes, focus, dispatch a DOM event, navigate, or push a history
    /// entry. The payload is the handler's `js` array, forwarded as-is.
    Js {
        cmds: serde_json::Value,
    },
    Error {
        message: String,
    },
    /// Reply to the client's keepalive. Named for the wire so one type owns the
    /// shape — the server used to hand-write this JSON, and the two spellings
    /// drifted (`HeartbeatAck` vs `heartbeat_ack`).
    #[serde(rename = "heartbeat_ack")]
    HeartbeatAck,
}

/// One stream mutation targeting a container (`container`) and, for inserts, a
/// keyed child (`id`). `html` is the rendered item markup for add ops.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum StreamOp {
    /// Append `html` as the container's last child (or move it there if `id` exists).
    Append {
        container: String,
        id: String,
        html: String,
    },
    /// Prepend `html` as the container's first child.
    Prepend {
        container: String,
        id: String,
        html: String,
    },
    /// Insert `html` before the child with id `before` (append if `before` is absent/missing).
    Insert {
        container: String,
        id: String,
        html: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        before: Option<String>,
    },
    /// Remove the element with id `id`.
    Remove { id: String },
    /// Clear all children of the container.
    Reset { container: String },
}

/// A single LiveView instance.
#[derive(Clone)]
pub struct LiveViewInstance {
    pub id: LiveViewId,
    pub component: String,
    pub template_path: PathBuf,
    pub state: serde_json::Value,
    pub session_id: String,
    pub last_html: String,
    pub senders: Vec<Arc<async_channel::Sender<Result<Message, tungstenite::Error>>>>,
    pub channels: HashSet<String>,
    pub created_at: Instant,
    pub last_active: Instant,
    /// Current tick interval in milliseconds, if a periodic tick is scheduled.
    /// `None` means no tick task is running.
    pub tick_interval_ms: Option<u64>,
    /// When the socket closed, if it has. A detached instance is kept only long
    /// enough for a refresh or a network blip to reclaim its state
    /// (`DETACHED_GRACE`), then reaped — otherwise every connection ever made
    /// keeps its state and full `last_html` for the process lifetime.
    pub detached_at: Option<Instant>,
}

impl LiveViewInstance {
    pub fn new(
        component: String,
        template_path: PathBuf,
        state: serde_json::Value,
        session_id: String,
        sender: Arc<async_channel::Sender<Result<Message, tungstenite::Error>>>,
    ) -> Self {
        let id = format!("{}:{}", session_id, component);
        let now = Instant::now();

        Self {
            id,
            component,
            template_path,
            state,
            session_id,
            last_html: String::new(),
            senders: vec![sender],
            channels: HashSet::new(),
            created_at: now,
            last_active: now,
            tick_interval_ms: None,
            detached_at: None,
        }
    }

    pub fn touch(&mut self) {
        self.last_active = Instant::now();
    }

    pub fn is_expired(&self, timeout: Duration) -> bool {
        self.last_active.elapsed() > timeout
    }

    #[allow(clippy::result_large_err)]
    pub fn send(&self, message: ServerMessage) -> Result<(), tungstenite::Error> {
        let json =
            serde_json::to_string(&message).map_err(|_| tungstenite::Error::ConnectionClosed)?;
        let msg = Message::text(json);

        let mut delivered = false;
        for sender in &self.senders {
            if sender.try_send(Ok(msg.clone())).is_ok() {
                delivered = true;
            }
        }
        if delivered {
            Ok(())
        } else {
            Err(tungstenite::Error::ConnectionClosed)
        }
    }
}

/// How long a closed socket's instance is kept so a refresh or a network blip
/// can reclaim its state.
pub const DETACHED_GRACE: Duration = Duration::from_secs(120);

/// Registry for all active LiveView instances.
pub struct LiveRegistry {
    views: Arc<std::sync::Mutex<HashMap<LiveViewId, LiveViewInstance>>>,
    /// One mutex per instance, held for a whole frame (handler → render → diff →
    /// send). It serializes the frames of *one* LiveView without serializing
    /// unrelated connections the way holding `views` across a render would.
    frame_locks: Arc<std::sync::Mutex<HashMap<LiveViewId, Arc<std::sync::Mutex<()>>>>>,
    timeout: Duration,
}

impl Default for LiveRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl LiveRegistry {
    pub fn new() -> Self {
        Self {
            views: Arc::new(std::sync::Mutex::new(HashMap::new())),
            frame_locks: Arc::new(std::sync::Mutex::new(HashMap::new())),
            timeout: Duration::from_secs(crate::live::LIVE_SESSION_TIMEOUT),
        }
    }

    pub fn register(&self, instance: LiveViewInstance) {
        let mut views = self.views.lock().unwrap();
        views.insert(instance.id.clone(), instance);
    }

    /// Mark an instance's socket as closed, keeping its state for a reconnect.
    ///
    /// Its live-query subscriptions go away immediately — waking a socket-less
    /// view costs a handler run, a render and a diff for markup nobody receives.
    /// A reconnect re-subscribes when the handler re-runs its live query.
    pub fn detach(&self, id: &str) {
        let existed = self
            .with_instance(id, |inst| {
                inst.detached_at = Some(Instant::now());
            })
            .is_some();
        if existed {
            crate::live::live_query::unsubscribe_all(id);
        }
    }

    pub fn unregister(&self, id: &str) {
        let mut views = self.views.lock().unwrap();
        views.remove(id);
        self.frame_locks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
        // Drop any live-query subscriptions this LiveView held, so a write to
        // the collection can't keep waking a disconnected view.
        crate::live::live_query::unsubscribe_all(id);
    }

    pub fn get(&self, id: &str) -> Option<LiveViewInstance> {
        let views = self.views.lock().unwrap();
        views.get(id).cloned()
    }

    /// Drop instances whose socket has been closed longer than
    /// [`DETACHED_GRACE`], plus any that idled past the session timeout.
    pub fn cleanup(&self) {
        let mut views = self.views.lock().unwrap();
        let expired: Vec<LiveViewId> = views
            .iter()
            .filter(|(_, v)| {
                v.is_expired(self.timeout)
                    || v.detached_at
                        .is_some_and(|at| at.elapsed() > DETACHED_GRACE)
            })
            .map(|(k, _)| k.clone())
            .collect();

        let mut frame_locks = self.frame_locks.lock().unwrap_or_else(|e| e.into_inner());
        for id in expired {
            views.remove(&id);
            frame_locks.remove(&id);
            crate::live::live_query::unsubscribe_all(&id);
        }
    }

    #[allow(clippy::result_large_err)]
    pub fn send(&self, id: &str, message: ServerMessage) -> Result<(), tungstenite::Error> {
        let views = self.views.lock().unwrap();
        if let Some(view) = views.get(id) {
            view.send(message)
        } else {
            Err(tungstenite::Error::ConnectionClosed)
        }
    }

    pub fn update(&self, instance: LiveViewInstance) {
        let mut views = self.views.lock().unwrap();
        views.insert(instance.id.clone(), instance);
    }

    /// Mutate an instance in place, under the registry lock.
    ///
    /// Prefer this over `get` + `update`: that pair is a read-modify-write on a
    /// deep clone, so a tick and a client event racing on two workers resolve
    /// last-writer-wins — one mutation is lost, and a stale `last_html` becomes
    /// the next diff base. Holding the lock across the render-and-diff keeps the
    /// server's diff base equal to what the client actually received.
    ///
    /// The closure must not call back into the interpreter or into
    /// `live_query`: it runs with the registry `Mutex` held.
    pub fn with_instance<T>(
        &self,
        id: &str,
        f: impl FnOnce(&mut LiveViewInstance) -> T,
    ) -> Option<T> {
        let mut views = self.views.lock().unwrap_or_else(|e| e.into_inner());
        views.get_mut(id).map(f)
    }

    /// Attach another socket to an existing instance (second browser tab).
    pub fn add_sender(
        &self,
        id: &str,
        sender: Arc<async_channel::Sender<Result<Message, tungstenite::Error>>>,
    ) -> bool {
        self.with_instance(id, |inst| {
            inst.senders.retain(|s| !s.is_closed());
            if !inst.senders.iter().any(|s| Arc::ptr_eq(s, &sender)) {
                inst.senders.push(sender);
            }
            inst.detached_at = None;
            inst.touch();
        })
        .is_some()
    }

    /// Drop one socket. Returns true when no sockets remain (caller should detach).
    pub fn drop_sender(
        &self,
        id: &str,
        sender: &Arc<async_channel::Sender<Result<Message, tungstenite::Error>>>,
    ) -> bool {
        self.with_instance(id, |inst| {
            inst.senders
                .retain(|s| !Arc::ptr_eq(s, sender) && !s.is_closed());
            inst.senders.is_empty()
        })
        .unwrap_or(true)
    }

    /// Write a frame's mutable fields back onto the registered instance.
    ///
    /// Returns `false` when the instance is gone — the socket closed while the
    /// handler ran. `update()` would re-insert it, resurrecting a view whose
    /// sender is dead and whose live queries keep waking a worker forever.
    pub fn commit(&self, frame: &LiveViewInstance) -> bool {
        self.with_instance(&frame.id, |inst| {
            inst.state = frame.state.clone();
            inst.last_html = frame.last_html.clone();
            inst.tick_interval_ms = frame.tick_interval_ms;
            inst.channels = frame.channels.clone();
            inst.last_active = frame.last_active;
        })
        .is_some()
    }

    /// The frame lock for `id`, created on first use.
    ///
    /// Hold it for the whole frame — handler call, render, diff and send. Two
    /// frames of one LiveView (a tick and a client event, on different workers)
    /// would otherwise both read the same state, both render from it, and the
    /// slower one would overwrite the faster one's state and regress
    /// `last_html`, leaving the client's shadow diffed against markup it never
    /// received.
    pub fn frame_lock(&self, id: &str) -> Arc<std::sync::Mutex<()>> {
        let mut locks = self.frame_locks.lock().unwrap_or_else(|e| e.into_inner());
        locks.entry(id.to_string()).or_default().clone()
    }
}

/// Global LiveView registry.
pub static LIVE_REGISTRY: std::sync::LazyLock<LiveRegistry> =
    std::sync::LazyLock::new(LiveRegistry::new);

#[cfg(test)]
mod tests {
    use super::*;

    fn instance(id: &str) -> LiveViewInstance {
        let (tx, rx) = async_channel::bounded(8);
        // Keep the receiver alive for the test's lifetime: a dropped receiver
        // makes every `send` look like a closed connection.
        std::mem::forget(rx);
        let mut inst = LiveViewInstance::new(
            "counter".to_string(),
            PathBuf::from("app/views/live/counter.slv"),
            serde_json::json!({ "count": 0 }),
            "sess-test".to_string(),
            Arc::new(tx),
        );
        inst.id = id.to_string();
        inst
    }

    /// Read-modify-write through `get`/`update` loses one of two concurrent
    /// changes; `with_instance` mutates in place under the lock.
    #[test]
    fn concurrent_updates_do_not_lose_state() {
        const THREADS: i64 = 8;
        const PER_THREAD: i64 = 50;
        let registry = LiveRegistry::new();
        registry.register(instance("race:counter"));

        std::thread::scope(|scope| {
            for _ in 0..THREADS {
                scope.spawn(|| {
                    for _ in 0..PER_THREAD {
                        registry.with_instance("race:counter", |inst| {
                            let n = inst.state["count"].as_i64().unwrap_or(0);
                            inst.state["count"] = serde_json::json!(n + 1);
                        });
                    }
                });
            }
        });

        let count = registry
            .get("race:counter")
            .expect("still registered")
            .state["count"]
            .as_i64()
            .unwrap();
        assert_eq!(
            count,
            THREADS * PER_THREAD,
            "every mutation must survive; a get/update round trip loses them"
        );
    }

    /// A patch's diff base must be the HTML the client last received. Holding
    /// the lock across render-and-diff is what guarantees it.
    #[test]
    fn with_instance_sees_the_previous_html() {
        let registry = LiveRegistry::new();
        registry.register(instance("html:counter"));
        registry.with_instance("html:counter", |inst| {
            inst.last_html = "<p>1</p>".to_string();
        });
        let seen = registry
            .with_instance("html:counter", |inst| {
                let previous = inst.last_html.clone();
                inst.last_html = "<p>2</p>".to_string();
                previous
            })
            .expect("registered");
        assert_eq!(seen, "<p>1</p>");
        assert_eq!(registry.get("html:counter").unwrap().last_html, "<p>2</p>");
        // A missing instance yields None rather than panicking.
        assert!(registry.with_instance("nope", |_| ()).is_none());
    }

    /// The client matches on `heartbeat_ack`; the enum must serialize to that.
    #[test]
    fn heartbeat_ack_serializes_to_the_client_wire_shape() {
        let json = serde_json::to_string(&ServerMessage::HeartbeatAck).unwrap();
        assert_eq!(json, r#"{"type":"heartbeat_ack"}"#);
        assert!(
            crate::live::LIVE_CLIENT_JS.contains("case 'heartbeat_ack':"),
            "the embedded client must handle the shape the server sends"
        );
    }

    /// A closed socket must stop being woken by live queries, but keep its
    /// state long enough for a refresh to reclaim it.
    #[test]
    fn detach_keeps_state_but_drops_subscriptions() {
        let registry = LiveRegistry::new();
        registry.register(instance("detach:counter"));
        let guard = crate::live::live_query::set_current(
            "detach:counter".to_string(),
            "counter".to_string(),
        );
        crate::live::live_query::subscribe("detach_test_collection", None);
        drop(guard);

        registry.detach("detach:counter");
        assert!(
            registry.get("detach:counter").is_some(),
            "state must survive for a reconnect"
        );
        assert!(
            crate::live::live_query::subscribers_to_wake("detach_test_collection", None).is_empty(),
            "a socket-less view must not be woken by writes"
        );
    }

    /// The reaper is what keeps detached instances from accumulating for the
    /// process lifetime.
    #[test]
    fn cleanup_reaps_instances_detached_past_the_grace() {
        let registry = LiveRegistry::new();
        registry.register(instance("reap:counter"));
        registry.register(instance("keep:counter"));
        registry.with_instance("reap:counter", |inst| {
            inst.detached_at = Some(Instant::now() - DETACHED_GRACE - Duration::from_secs(1));
        });
        registry.detach("keep:counter"); // detached just now — inside the grace

        registry.cleanup();
        assert!(registry.get("reap:counter").is_none(), "past grace: reaped");
        assert!(registry.get("keep:counter").is_some(), "in grace: kept");
    }

    /// A frame that finishes after its socket closed must not re-create the
    /// instance: that is how "unregister on close" turns back into a leak.
    #[test]
    fn commit_does_not_resurrect_a_closed_instance() {
        let registry = LiveRegistry::new();
        registry.register(instance("gone:counter"));
        let frame = registry.get("gone:counter").expect("registered");
        registry.unregister("gone:counter");
        assert!(!registry.commit(&frame), "commit must report the miss");
        assert!(registry.get("gone:counter").is_none());
    }

    /// Unregistering must also drop the live-query subscriptions, or a write
    /// keeps waking a view whose socket is gone.
    #[test]
    fn unregister_drops_live_query_subscriptions() {
        let registry = LiveRegistry::new();
        registry.register(instance("lqdrop:counter"));
        let guard = crate::live::live_query::set_current(
            "lqdrop:counter".to_string(),
            "counter".to_string(),
        );
        crate::live::live_query::subscribe("view_test_collection", None);
        drop(guard);

        assert_eq!(
            crate::live::live_query::subscribers_to_wake("view_test_collection", None).len(),
            1,
            "the subscription exists before unregistering"
        );

        registry.unregister("lqdrop:counter");
        assert!(registry.get("lqdrop:counter").is_none());
        assert!(
            crate::live::live_query::subscribers_to_wake("view_test_collection", None).is_empty(),
            "a write must not keep waking a view whose socket is gone"
        );
    }

    #[test]
    fn stream_message_serializes_to_client_wire_shape() {
        // The client matches `msg.type` (lowercased) and each op's `op` tag —
        // lock that contract so a rename can't silently break the JS.
        let msg = ServerMessage::Stream {
            liveview_id: "sess:board".to_string(),
            ops: vec![
                StreamOp::Append {
                    container: "posts".to_string(),
                    id: "post-7".to_string(),
                    html: "<li>hi</li>".to_string(),
                },
                StreamOp::Remove {
                    id: "post-1".to_string(),
                },
            ],
        };
        let json: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&msg).unwrap()).unwrap();
        assert_eq!(json["type"], "Stream");
        assert_eq!(json["liveview_id"], "sess:board");
        assert_eq!(json["ops"][0]["op"], "append");
        assert_eq!(json["ops"][0]["container"], "posts");
        assert_eq!(json["ops"][0]["id"], "post-7");
        assert_eq!(json["ops"][0]["html"], "<li>hi</li>");
        assert_eq!(json["ops"][1]["op"], "remove");
        assert_eq!(json["ops"][1]["id"], "post-1");
        // Remove carries no container/html.
        assert!(json["ops"][1].get("container").is_none());
    }

    #[test]
    fn js_message_serializes_as_js_type() {
        let msg = ServerMessage::Js {
            cmds: serde_json::json!([{ "op": "focus", "to": "#q" }]),
        };
        let json: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&msg).unwrap()).unwrap();
        assert_eq!(json["type"], "Js");
        assert_eq!(json["cmds"][0]["op"], "focus");
    }

    #[test]
    fn url_message_serializes_as_url_type() {
        let msg = ServerMessage::Url {
            url: "/items?q=1".to_string(),
            replace: true,
        };
        let json: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&msg).unwrap()).unwrap();
        assert_eq!(json["type"], "Url");
        assert_eq!(json["url"], "/items?q=1");
        assert_eq!(json["replace"], true);
    }

    #[test]
    fn live_message_serializes_as_live_type() {
        let msg = ServerMessage::Live {
            url: "/live/socket/about".to_string(),
        };
        let json: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&msg).unwrap()).unwrap();
        assert_eq!(json["type"], "Live");
        assert_eq!(json["url"], "/live/socket/about");
    }
}
