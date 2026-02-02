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
    Render { html: String, liveview_id: String },
    Patch { liveview_id: String, diff: String },
    Redirect { url: String },
    Error { message: String },
    HeartbeatAck,
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
