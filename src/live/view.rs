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
    Error {
        message: String,
    },
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
    pub sender: Arc<async_channel::Sender<Result<Message, tungstenite::Error>>>,
    pub channels: HashSet<String>,
    pub created_at: Instant,
    pub last_active: Instant,
    /// Current tick interval in milliseconds, if a periodic tick is scheduled.
    /// `None` means no tick task is running.
    pub tick_interval_ms: Option<u64>,
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
            sender,
            channels: HashSet::new(),
            created_at: now,
            last_active: now,
            tick_interval_ms: None,
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

        if let Err(_e) = self.sender.try_send(Ok(msg)) {
            return Err(tungstenite::Error::ConnectionClosed);
        }
        Ok(())
    }
}

/// Registry for all active LiveView instances.
pub struct LiveRegistry {
    views: Arc<std::sync::Mutex<HashMap<LiveViewId, LiveViewInstance>>>,
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
            timeout: Duration::from_secs(3600),
        }
    }

    pub fn register(&self, instance: LiveViewInstance) {
        let mut views = self.views.lock().unwrap();
        views.insert(instance.id.clone(), instance);
    }

    pub fn unregister(&self, id: &str) {
        let mut views = self.views.lock().unwrap();
        views.remove(id);
        // Drop any live-query subscriptions this LiveView held, so a write to
        // the collection can't keep waking a disconnected view.
        crate::live::live_query::unsubscribe_all(id);
    }

    pub fn get(&self, id: &str) -> Option<LiveViewInstance> {
        let views = self.views.lock().unwrap();
        views.get(id).cloned()
    }

    pub fn cleanup(&self) {
        let mut views = self.views.lock().unwrap();
        let expired: Vec<LiveViewId> = views
            .iter()
            .filter(|(_, v)| v.is_expired(self.timeout))
            .map(|(k, _)| k.clone())
            .collect();

        for id in expired {
            views.remove(&id);
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
}

/// Global LiveView registry.
pub static LIVE_REGISTRY: std::sync::LazyLock<LiveRegistry> =
    std::sync::LazyLock::new(LiveRegistry::new);

#[cfg(test)]
mod tests {
    use super::*;

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
}
