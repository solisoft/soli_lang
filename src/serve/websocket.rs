//! WebSocket support for the Solilang MVC framework.
//!
//! This module provides WebSocket handling including:
//! - Connection management with unique IDs
//! - Channel/room support for targeted broadcasting
//! - Single handler pattern for all WebSocket events (connect, message, disconnect)

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex as AsyncMutex;
use tungstenite::Message;
use uuid::Uuid;

use crate::interpreter::value::{HashKey, Value};

type ConnectionPresenceMap = HashMap<Uuid, Vec<(String, String)>>;

/// Metadata for a single presence connection.
/// Each user can have multiple connections (tabs/devices), each with its own meta.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PresenceMeta {
    /// The connection ID this meta belongs to
    pub connection_id: Uuid,
    /// Unique reference for diff tracking (Phoenix-style)
    pub phx_ref: String,
    /// Presence state: "online", "away", "typing", or custom
    pub state: String,
    /// Unix timestamp when this connection joined
    pub online_at: u64,
    /// Additional user-provided fields (name, avatar, etc.)
    #[serde(flatten)]
    pub extra: HashMap<String, String>,
}

/// A user's presence in a channel, grouped by user_id.
/// Contains all metas (one per connection/device).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserPresence {
    /// The user identifier (provided by the application)
    pub user_id: String,
    /// One meta entry per connection
    pub metas: Vec<PresenceMeta>,
}

/// Payload format for presence in messages (keyed by user_id).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserPresencePayload {
    /// The metas for this user
    pub metas: Vec<PresenceMeta>,
}

/// Presence diff message sent to clients when presence changes.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PresenceDiff {
    /// Users who joined (keyed by user_id)
    pub joins: HashMap<String, UserPresencePayload>,
    /// Users who left (keyed by user_id)
    pub leaves: HashMap<String, UserPresencePayload>,
}

/// Full presence state message sent to new joiners.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PresenceState {
    /// All users in the channel (keyed by user_id)
    #[serde(flatten)]
    pub presences: HashMap<String, UserPresencePayload>,
}

/// A WebSocket connection with its sender and metadata.
#[derive(Clone)]
pub struct WebSocketConnection {
    /// Unique identifier for this connection
    pub id: Uuid,
    /// Channel sender for sending messages to this client
    pub sender: Arc<tokio::sync::mpsc::Sender<Result<Message, tungstenite::Error>>>,
    /// Channels this connection is subscribed to
    pub channels: Vec<String>,
    /// User-defined metadata for this connection
    pub metadata: HashMap<String, String>,
}

impl WebSocketConnection {
    /// Create a new WebSocket connection.
    pub fn new(
        sender: Arc<tokio::sync::mpsc::Sender<Result<Message, tungstenite::Error>>>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            sender,
            channels: Vec::new(),
            metadata: HashMap::new(),
        }
    }
}

/// Registry for all active WebSocket connections.
#[derive(Clone)]
pub struct WebSocketRegistry {
    /// All connections indexed by their ID
    connections: Arc<AsyncMutex<HashMap<Uuid, WebSocketConnection>>>,
    /// Channels mapping channel name to connection IDs
    channels: Arc<AsyncMutex<HashMap<String, HashSet<Uuid>>>>,
    /// Presence tracking: channel -> user_id -> UserPresence
    room_presence: Arc<AsyncMutex<HashMap<String, HashMap<String, UserPresence>>>>,
    /// Connection to presence mapping for cleanup: connection_id -> Vec<(channel, user_id)>
    connection_presence: Arc<AsyncMutex<ConnectionPresenceMap>>,
    /// Counter for generating unique phx_ref values
    ref_counter: Arc<AtomicU64>,
}

impl Default for WebSocketRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl WebSocketRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            connections: Arc::new(AsyncMutex::new(HashMap::new())),
            channels: Arc::new(AsyncMutex::new(HashMap::new())),
            room_presence: Arc::new(AsyncMutex::new(HashMap::new())),
            connection_presence: Arc::new(AsyncMutex::new(HashMap::new())),
            ref_counter: Arc::new(AtomicU64::new(1)),
        }
    }

    /// Generate a unique phx_ref for presence tracking.
    fn next_ref(&self) -> String {
        self.ref_counter.fetch_add(1, Ordering::SeqCst).to_string()
    }

    /// Register a new connection.
    pub async fn register(&self, connection: WebSocketConnection) {
        let mut connections = self.connections.lock().await;
        connections.insert(connection.id, connection);
    }

    /// Unregister a connection and clean up channel subscriptions.
    pub async fn unregister(&self, id: &Uuid) {
        let mut connections = self.connections.lock().await;
        if let Some(conn) = connections.remove(id) {
            // Remove from all channels
            let mut channels = self.channels.lock().await;
            for channel in &conn.channels {
                if let Some(set) = channels.get_mut(channel) {
                    set.remove(id);
                    if set.is_empty() {
                        channels.remove(channel);
                    }
                }
            }
        }
    }

    /// Get a connection by ID.
    pub async fn get(&self, id: &Uuid) -> Option<WebSocketConnection> {
        let connections = self.connections.lock().await;
        connections.get(id).cloned()
    }

    /// Get all connection IDs.
    pub async fn get_all_ids(&self) -> Vec<Uuid> {
        let connections = self.connections.lock().await;
        connections.keys().cloned().collect()
    }

    /// Get all connections in a channel.
    pub async fn get_channel_ids(&self, channel: &str) -> Vec<Uuid> {
        let channels = self.channels.lock().await;
        if let Some(set) = channels.get(channel) {
            set.iter().cloned().collect()
        } else {
            Vec::new()
        }
    }

    /// Join a connection to a channel.
    pub async fn join_channel(&self, connection_id: &Uuid, channel: &str) {
        let mut connections = self.connections.lock().await;
        if let Some(conn) = connections.get_mut(connection_id) {
            if !conn.channels.contains(&channel.to_string()) {
                conn.channels.push(channel.to_string());
            }
        }

        let mut channels = self.channels.lock().await;
        channels
            .entry(channel.to_string())
            .or_insert_with(HashSet::new)
            .insert(*connection_id);
    }

    /// Remove a connection from a channel.
    pub async fn leave_channel(&self, connection_id: &Uuid, channel: &str) {
        let mut connections = self.connections.lock().await;
        if let Some(conn) = connections.get_mut(connection_id) {
            conn.channels.retain(|c| c != channel);
        }

        let mut channels = self.channels.lock().await;
        if let Some(set) = channels.get_mut(channel) {
            set.remove(connection_id);
            if set.is_empty() {
                channels.remove(channel);
            }
        }
    }

    /// Send a message to a specific connection.
    pub async fn send_to(&self, id: &Uuid, message: &str) -> Result<(), tungstenite::Error> {
        let connections = self.connections.lock().await;
        if let Some(conn) = connections.get(id) {
            let msg = Message::text(message);
            if let Err(_e) = conn.sender.send(Ok(msg)).await {
                return Err(tungstenite::Error::ConnectionClosed);
            }
        }
        Ok(())
    }

    /// Broadcast a message to all connections.
    pub async fn broadcast_all(&self, message: &str) {
        let connections = self.connections.lock().await;
        let msg = Message::text(message);
        for conn in connections.values() {
            let _ = conn.sender.send(Ok(msg.clone())).await;
        }
    }

    /// Broadcast a message to all connections in a channel.
    pub async fn broadcast_to_channel(&self, channel: &str, message: &str) {
        let channel_ids = self.get_channel_ids(channel).await;
        let connections = self.connections.lock().await;
        let msg = Message::text(message);
        for id in channel_ids {
            if let Some(conn) = connections.get(&id) {
                let _ = conn.sender.send(Ok(msg.clone())).await;
            }
        }
    }

    /// Close a specific connection.
    pub async fn close(&self, id: &Uuid, reason: &str) {
        let connections = self.connections.lock().await;
        if let Some(conn) = connections.get(id) {
            let close_frame = tungstenite::protocol::CloseFrame {
                code: tungstenite::protocol::frame::coding::CloseCode::Normal,
                reason: std::borrow::Cow::Owned(reason.to_string()),
            };
            let _ = conn
                .sender
                .send(Ok(Message::Close(Some(close_frame))))
                .await;
        }
    }

    /// Set metadata for a connection.
    pub async fn set_metadata(&self, id: &Uuid, key: &str, value: &str) {
        let mut connections = self.connections.lock().await;
        if let Some(conn) = connections.get_mut(id) {
            conn.metadata.insert(key.to_string(), value.to_string());
        }
    }

    /// Get the number of active connections.
    pub async fn connection_count(&self) -> usize {
        self.connections.lock().await.len()
    }

    // ========== Presence Methods ==========

    /// Track a connection's presence in a channel.
    /// Returns (is_new_user, PresenceMeta) - is_new_user is true if this is the first connection for this user.
    pub async fn track(
        &self,
        connection_id: &Uuid,
        channel: &str,
        user_id: &str,
        meta: HashMap<String, String>,
    ) -> (bool, PresenceMeta) {
        let phx_ref = self.next_ref();
        let online_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let presence_meta = PresenceMeta {
            connection_id: *connection_id,
            phx_ref,
            state: meta
                .get("state")
                .cloned()
                .unwrap_or_else(|| "online".to_string()),
            online_at,
            extra: meta,
        };

        let mut room_presence = self.room_presence.lock().await;
        let channel_presence = room_presence.entry(channel.to_string()).or_default();

        let is_new_user = !channel_presence.contains_key(user_id);

        let user_presence = channel_presence
            .entry(user_id.to_string())
            .or_insert_with(|| UserPresence {
                user_id: user_id.to_string(),
                metas: Vec::new(),
            });

        user_presence.metas.push(presence_meta.clone());

        // Track connection -> (channel, user_id) mapping for cleanup
        let mut conn_presence = self.connection_presence.lock().await;
        conn_presence
            .entry(*connection_id)
            .or_default()
            .push((channel.to_string(), user_id.to_string()));

        (is_new_user, presence_meta)
    }

    /// Stop tracking presence for a connection in a channel.
    /// Returns Option<(was_last_connection, PresenceMeta)> if the connection was being tracked.
    pub async fn untrack(
        &self,
        connection_id: &Uuid,
        channel: &str,
    ) -> Option<(bool, PresenceMeta)> {
        let mut room_presence = self.room_presence.lock().await;

        if let Some(channel_presence) = room_presence.get_mut(channel) {
            // Find which user this connection belongs to
            let mut found_user_id: Option<String> = None;
            let mut found_meta: Option<PresenceMeta> = None;

            for (user_id, user_presence) in channel_presence.iter_mut() {
                if let Some(pos) = user_presence
                    .metas
                    .iter()
                    .position(|m| m.connection_id == *connection_id)
                {
                    found_meta = Some(user_presence.metas.remove(pos));
                    found_user_id = Some(user_id.clone());
                    break;
                }
            }

            if let (Some(user_id), Some(meta)) = (found_user_id, found_meta) {
                let was_last = channel_presence
                    .get(&user_id)
                    .map(|up| up.metas.is_empty())
                    .unwrap_or(true);

                // Remove user entry if no more connections
                if was_last {
                    channel_presence.remove(&user_id);
                }

                // Clean up empty channel
                if channel_presence.is_empty() {
                    room_presence.remove(channel);
                }

                // Remove from connection_presence mapping
                let mut conn_presence = self.connection_presence.lock().await;
                if let Some(entries) = conn_presence.get_mut(connection_id) {
                    entries.retain(|(c, _)| c != channel);
                    if entries.is_empty() {
                        conn_presence.remove(connection_id);
                    }
                }

                return Some((was_last, meta));
            }
        }

        None
    }

    /// Untrack all presences for a connection (called on disconnect).
    /// Returns Vec<(channel, user_id, was_last, PresenceMeta)> for all tracked presences.
    pub async fn untrack_all(
        &self,
        connection_id: &Uuid,
    ) -> Vec<(String, String, bool, PresenceMeta)> {
        let mut results = Vec::new();

        // Get the list of (channel, user_id) pairs for this connection
        let channels_to_untrack: Vec<(String, String)> = {
            let conn_presence = self.connection_presence.lock().await;
            conn_presence
                .get(connection_id)
                .cloned()
                .unwrap_or_default()
        };

        // Untrack each one
        for (channel, user_id) in channels_to_untrack {
            let mut room_presence = self.room_presence.lock().await;

            if let Some(channel_presence) = room_presence.get_mut(&channel) {
                if let Some(user_presence) = channel_presence.get_mut(&user_id) {
                    if let Some(pos) = user_presence
                        .metas
                        .iter()
                        .position(|m| m.connection_id == *connection_id)
                    {
                        let meta = user_presence.metas.remove(pos);
                        let was_last = user_presence.metas.is_empty();

                        if was_last {
                            channel_presence.remove(&user_id);
                        }

                        if channel_presence.is_empty() {
                            room_presence.remove(&channel);
                        }

                        results.push((channel, user_id, was_last, meta));
                    }
                }
            }
        }

        // Clean up connection_presence entry
        let mut conn_presence = self.connection_presence.lock().await;
        conn_presence.remove(connection_id);

        results
    }

    /// Update presence state for a connection in a channel.
    /// Returns the updated PresenceMeta if found.
    pub async fn set_presence(
        &self,
        connection_id: &Uuid,
        channel: &str,
        state: &str,
    ) -> Option<PresenceMeta> {
        let mut room_presence = self.room_presence.lock().await;

        if let Some(channel_presence) = room_presence.get_mut(channel) {
            for user_presence in channel_presence.values_mut() {
                if let Some(meta) = user_presence
                    .metas
                    .iter_mut()
                    .find(|m| m.connection_id == *connection_id)
                {
                    meta.state = state.to_string();
                    return Some(meta.clone());
                }
            }
        }

        None
    }

    /// Get all presences in a channel (grouped by user).
    pub async fn list_presence(&self, channel: &str) -> Vec<UserPresence> {
        let room_presence = self.room_presence.lock().await;
        room_presence
            .get(channel)
            .map(|cp| cp.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Get presence for a specific user in a channel.
    pub async fn get_user_presence(&self, channel: &str, user_id: &str) -> Option<UserPresence> {
        let room_presence = self.room_presence.lock().await;
        room_presence
            .get(channel)
            .and_then(|cp| cp.get(user_id).cloned())
    }

    /// Get unique user count in a channel (not connection count).
    pub async fn presence_count(&self, channel: &str) -> usize {
        let room_presence = self.room_presence.lock().await;
        room_presence.get(channel).map(|cp| cp.len()).unwrap_or(0)
    }

    /// Broadcast a message to all connections in a channel except one.
    pub async fn broadcast_to_channel_except(&self, channel: &str, message: &str, except: &Uuid) {
        let channel_ids = self.get_channel_ids(channel).await;
        let connections = self.connections.lock().await;
        let msg = Message::text(message);
        for id in channel_ids {
            if id != *except {
                if let Some(conn) = connections.get(&id) {
                    let _ = conn.sender.send(Ok(msg.clone())).await;
                }
            }
        }
    }

    /// Build a presence_state message for a channel.
    pub fn build_presence_state(presences: &[UserPresence]) -> String {
        let mut state: HashMap<String, UserPresencePayload> = HashMap::new();
        for presence in presences {
            state.insert(
                presence.user_id.clone(),
                UserPresencePayload {
                    metas: presence.metas.clone(),
                },
            );
        }
        serde_json::json!({
            "event": "presence_state",
            "payload": state
        })
        .to_string()
    }

    /// Build a presence_diff message.
    pub fn build_presence_diff(diff: &PresenceDiff) -> String {
        serde_json::json!({
            "event": "presence_diff",
            "payload": diff
        })
        .to_string()
    }
}

/// WebSocket event types for the single handler pattern.
#[derive(Clone)]
pub enum WebSocketEventType {
    Connect,
    Message(String),
    Disconnect,
}

/// A WebSocket event passed to the handler.
#[derive(Clone)]
pub struct WebSocketEvent {
    /// Type of event: "connect", "message", or "disconnect"
    pub event_type: String,
    /// Connection ID that triggered this event
    pub connection_id: String,
    /// Message payload (only for "message" events)
    pub message: Option<String>,
    /// Channel name (for join/leave events)
    pub channel: Option<String>,
}

impl WebSocketEvent {
    /// Create a connect event.
    pub fn connect(id: &Uuid) -> Self {
        Self {
            event_type: "connect".to_string(),
            connection_id: id.to_string(),
            message: None,
            channel: None,
        }
    }

    /// Create a message event.
    pub fn message(id: &Uuid, msg: &str) -> Self {
        Self {
            event_type: "message".to_string(),
            connection_id: id.to_string(),
            message: Some(msg.to_string()),
            channel: None,
        }
    }

    /// Create a disconnect event.
    pub fn disconnect(id: &Uuid) -> Self {
        Self {
            event_type: "disconnect".to_string(),
            connection_id: id.to_string(),
            message: None,
            channel: None,
        }
    }

    /// Convert to a Value for the Soli interpreter.
    pub fn to_value(&self) -> Value {
        let mut result: IndexMap<HashKey, Value> = IndexMap::new();
        result.insert(
            HashKey::String("type".to_string()),
            Value::String(self.event_type.clone()),
        );
        result.insert(
            HashKey::String("connection_id".to_string()),
            Value::String(self.connection_id.clone()),
        );

        if let Some(ref msg) = self.message {
            result.insert(
                HashKey::String("message".to_string()),
                Value::String(msg.clone()),
            );
        }

        if let Some(ref channel) = self.channel {
            result.insert(
                HashKey::String("channel".to_string()),
                Value::String(channel.clone()),
            );
        }

        Value::Hash(Rc::new(RefCell::new(result)))
    }
}

/// Actions that a WebSocket handler can return.
#[derive(Clone, Debug)]
pub struct WebSocketHandlerAction {
    /// Join a channel
    pub join: Option<String>,
    /// Leave a channel
    pub leave: Option<String>,
    /// Send a message to this client
    pub send: Option<String>,
    /// Broadcast to all clients
    pub broadcast: Option<String>,
    /// Broadcast to a channel
    pub broadcast_room: Option<String>,
    /// Close the connection
    pub close: Option<String>,
    /// Track presence: {channel, user_id, ...extra meta}
    pub track: Option<HashMap<String, String>>,
    /// Untrack presence from a channel
    pub untrack: Option<String>,
    /// Update presence state: {channel, state}
    pub set_presence: Option<HashMap<String, String>>,
}

impl Default for WebSocketHandlerAction {
    fn default() -> Self {
        Self::new()
    }
}

impl WebSocketHandlerAction {
    /// Create a new empty action.
    pub fn new() -> Self {
        Self {
            join: None,
            leave: None,
            send: None,
            broadcast: None,
            broadcast_room: None,
            close: None,
            track: None,
            untrack: None,
            set_presence: None,
        }
    }

    /// Parse actions from a handler return value.
    pub fn from_value(value: &Value) -> Self {
        let mut action = Self::new();

        if let Value::Hash(hash) = value {
            for (k, v) in hash.borrow().iter() {
                if let HashKey::String(key) = k {
                    match key.as_str() {
                        "join" => {
                            if let Value::String(s) = v {
                                action.join = Some(s.clone());
                            }
                        }
                        "leave" => {
                            if let Value::String(s) = v {
                                action.leave = Some(s.clone());
                            }
                        }
                        "send" => {
                            if let Value::String(s) = v {
                                action.send = Some(s.clone());
                            }
                        }
                        "broadcast" => {
                            if let Value::String(s) = v {
                                action.broadcast = Some(s.clone());
                            }
                        }
                        "broadcast_room" => {
                            if let Value::String(s) = v {
                                action.broadcast_room = Some(s.clone());
                            }
                        }
                        "close" => {
                            if let Value::String(s) = v {
                                action.close = Some(s.clone());
                            }
                        }
                        "track" => {
                            // Parse track: { channel: "...", user_id: "...", ...extra }
                            if let Value::Hash(track_hash) = v {
                                let mut track_map = HashMap::new();
                                for (tk, tv) in track_hash.borrow().iter() {
                                    if let (HashKey::String(track_key), Value::String(track_val)) =
                                        (tk, tv)
                                    {
                                        track_map.insert(track_key.clone(), track_val.clone());
                                    }
                                }
                                if !track_map.is_empty() {
                                    action.track = Some(track_map);
                                }
                            }
                        }
                        "untrack" => {
                            // Parse untrack: "channel_name"
                            if let Value::String(s) = v {
                                action.untrack = Some(s.clone());
                            }
                        }
                        "set_presence" => {
                            // Parse set_presence: { channel: "...", state: "..." }
                            if let Value::Hash(presence_hash) = v {
                                let mut presence_map = HashMap::new();
                                for (pk, pv) in presence_hash.borrow().iter() {
                                    if let (HashKey::String(pkey), Value::String(pval)) = (pk, pv) {
                                        presence_map.insert(pkey.clone(), pval.clone());
                                    }
                                }
                                if !presence_map.is_empty() {
                                    action.set_presence = Some(presence_map);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        action
    }
}

/// A WebSocket route with its handler reference.
#[derive(Clone)]
pub struct WebSocketRoute {
    pub path_pattern: String,
    /// Controller#action string for handler lookup in CONTROLLERS registry
    pub handler_name: String,
}

// Use lazy_static with Mutex for thread-safe access from both main thread and tokio threads
lazy_static::lazy_static! {
    pub static ref WEBSOCKET_ROUTES: std::sync::Mutex<Vec<WebSocketRoute>> = std::sync::Mutex::new(Vec::new());
}

/// Register a WebSocket route.
/// Only stores path and handler_name (both thread-safe strings).
/// The actual handler Value is looked up from CONTROLLERS registry when events are processed.
pub fn register_websocket_route(path: &str, handler_name: &str) {
    let mut routes = WEBSOCKET_ROUTES.lock().unwrap();
    routes.push(WebSocketRoute {
        path_pattern: path.to_string(),
        handler_name: handler_name.to_string(),
    });
}

/// Get all WebSocket routes.
pub fn get_websocket_routes() -> Vec<WebSocketRoute> {
    WEBSOCKET_ROUTES.lock().unwrap().clone()
}

/// Match a path against WebSocket routes.
pub fn match_websocket_route(path: &str) -> Option<WebSocketRoute> {
    let routes = WEBSOCKET_ROUTES.lock().unwrap();
    for route in routes.iter() {
        if route.path_pattern == path {
            return Some(route.clone());
        }
    }
    None
}

/// Clear all WebSocket routes.
pub fn clear_websocket_routes() {
    let mut routes = WEBSOCKET_ROUTES.lock().unwrap();
    routes.clear();
}

/// Take all WebSocket routes (consumes and returns them).
pub fn take_websocket_routes() -> Vec<WebSocketRoute> {
    let mut routes = WEBSOCKET_ROUTES.lock().unwrap();
    std::mem::take(&mut *routes)
}

/// Restore WebSocket routes from a previous state.
pub fn restore_websocket_routes(routes: Vec<WebSocketRoute>) {
    let mut ws_routes = WEBSOCKET_ROUTES.lock().unwrap();
    *ws_routes = routes;
}

// Global WebSocket registry for use from tokio threads
lazy_static::lazy_static! {
    pub static ref GLOBAL_WS_REGISTRY: std::sync::Arc<WebSocketRegistry> = std::sync::Arc::new(WebSocketRegistry::new());
}

/// Get the global WebSocket registry.
pub fn get_ws_registry() -> std::sync::Arc<WebSocketRegistry> {
    GLOBAL_WS_REGISTRY.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_presence_track_new_user() {
        let registry = WebSocketRegistry::new();
        let conn_id = Uuid::new_v4();
        let mut meta = HashMap::new();
        meta.insert("name".to_string(), "Alice".to_string());

        let (is_new, presence_meta) = registry
            .track(&conn_id, "room:lobby", "user_123", meta)
            .await;

        assert!(is_new, "First connection should be a new user");
        assert_eq!(presence_meta.state, "online");
        assert_eq!(presence_meta.extra.get("name"), Some(&"Alice".to_string()));

        // Verify presence is tracked
        let presences = registry.list_presence("room:lobby").await;
        assert_eq!(presences.len(), 1);
        assert_eq!(presences[0].user_id, "user_123");
        assert_eq!(presences[0].metas.len(), 1);
    }

    #[tokio::test]
    async fn test_presence_track_same_user_multiple_connections() {
        let registry = WebSocketRegistry::new();
        let conn_id1 = Uuid::new_v4();
        let conn_id2 = Uuid::new_v4();
        let meta = HashMap::new();

        // First connection
        let (is_new1, _) = registry
            .track(&conn_id1, "room:lobby", "user_123", meta.clone())
            .await;
        assert!(is_new1, "First connection should be new");

        // Second connection for same user (e.g., another tab)
        let (is_new2, _) = registry
            .track(&conn_id2, "room:lobby", "user_123", meta)
            .await;
        assert!(
            !is_new2,
            "Second connection for same user should NOT be new"
        );

        // Verify only one user but two metas
        let presences = registry.list_presence("room:lobby").await;
        assert_eq!(presences.len(), 1);
        assert_eq!(presences[0].metas.len(), 2);
    }

    #[tokio::test]
    async fn test_presence_untrack_not_last_connection() {
        let registry = WebSocketRegistry::new();
        let conn_id1 = Uuid::new_v4();
        let conn_id2 = Uuid::new_v4();
        let meta = HashMap::new();

        // Track two connections for same user
        registry
            .track(&conn_id1, "room:lobby", "user_123", meta.clone())
            .await;
        registry
            .track(&conn_id2, "room:lobby", "user_123", meta)
            .await;

        // Untrack first connection
        let result = registry.untrack(&conn_id1, "room:lobby").await;
        assert!(result.is_some());
        let (was_last, _) = result.unwrap();
        assert!(!was_last, "Should NOT be last connection");

        // User should still be present with one meta
        let presences = registry.list_presence("room:lobby").await;
        assert_eq!(presences.len(), 1);
        assert_eq!(presences[0].metas.len(), 1);
    }

    #[tokio::test]
    async fn test_presence_untrack_last_connection() {
        let registry = WebSocketRegistry::new();
        let conn_id = Uuid::new_v4();
        let meta = HashMap::new();

        registry
            .track(&conn_id, "room:lobby", "user_123", meta)
            .await;

        let result = registry.untrack(&conn_id, "room:lobby").await;
        assert!(result.is_some());
        let (was_last, _) = result.unwrap();
        assert!(was_last, "Should be last connection");

        // User should be gone
        let presences = registry.list_presence("room:lobby").await;
        assert_eq!(presences.len(), 0);
    }

    #[tokio::test]
    async fn test_presence_untrack_all() {
        let registry = WebSocketRegistry::new();
        let conn_id = Uuid::new_v4();
        let meta = HashMap::new();

        // Track in multiple channels
        registry
            .track(&conn_id, "room:lobby", "user_123", meta.clone())
            .await;
        registry
            .track(&conn_id, "room:general", "user_123", meta)
            .await;

        // Untrack all (simulating disconnect)
        let results = registry.untrack_all(&conn_id).await;
        assert_eq!(results.len(), 2);

        // All channels should be empty
        assert_eq!(registry.list_presence("room:lobby").await.len(), 0);
        assert_eq!(registry.list_presence("room:general").await.len(), 0);
    }

    #[tokio::test]
    async fn test_presence_set_state() {
        let registry = WebSocketRegistry::new();
        let conn_id = Uuid::new_v4();
        let meta = HashMap::new();

        registry
            .track(&conn_id, "room:lobby", "user_123", meta)
            .await;

        // Update state to typing
        let result = registry
            .set_presence(&conn_id, "room:lobby", "typing")
            .await;
        assert!(result.is_some());
        assert_eq!(result.unwrap().state, "typing");

        // Verify state was updated
        let presences = registry.list_presence("room:lobby").await;
        assert_eq!(presences[0].metas[0].state, "typing");
    }

    #[tokio::test]
    async fn test_presence_count() {
        let registry = WebSocketRegistry::new();
        let meta = HashMap::new();

        // Add 3 users, one with 2 connections
        registry
            .track(&Uuid::new_v4(), "room:lobby", "user_1", meta.clone())
            .await;
        registry
            .track(&Uuid::new_v4(), "room:lobby", "user_2", meta.clone())
            .await;
        registry
            .track(&Uuid::new_v4(), "room:lobby", "user_2", meta.clone())
            .await; // Same user, different connection
        registry
            .track(&Uuid::new_v4(), "room:lobby", "user_3", meta)
            .await;

        // Should count unique users, not connections
        let count = registry.presence_count("room:lobby").await;
        assert_eq!(count, 3);
    }

    #[tokio::test]
    async fn test_get_user_presence() {
        let registry = WebSocketRegistry::new();
        let conn_id = Uuid::new_v4();
        let mut meta = HashMap::new();
        meta.insert("name".to_string(), "Alice".to_string());

        registry
            .track(&conn_id, "room:lobby", "user_123", meta)
            .await;

        // Get existing user
        let presence = registry.get_user_presence("room:lobby", "user_123").await;
        assert!(presence.is_some());
        assert_eq!(presence.unwrap().user_id, "user_123");

        // Get non-existing user
        let no_presence = registry.get_user_presence("room:lobby", "user_999").await;
        assert!(no_presence.is_none());
    }

    #[tokio::test]
    async fn test_broadcast_to_channel_except() {
        let registry = WebSocketRegistry::new();

        // Create mock connections (we can't actually test message delivery without real WebSockets,
        // but we can verify the method runs without error)
        let conn_id1 = Uuid::new_v4();
        let conn_id2 = Uuid::new_v4();

        registry.join_channel(&conn_id1, "room:lobby").await;
        registry.join_channel(&conn_id2, "room:lobby").await;

        // This should not panic
        registry
            .broadcast_to_channel_except("room:lobby", "test message", &conn_id1)
            .await;
    }

    #[tokio::test]
    async fn test_build_presence_state() {
        let presences = vec![UserPresence {
            user_id: "user_123".to_string(),
            metas: vec![PresenceMeta {
                connection_id: Uuid::new_v4(),
                phx_ref: "1".to_string(),
                state: "online".to_string(),
                online_at: 1234567890,
                extra: HashMap::new(),
            }],
        }];

        let json = WebSocketRegistry::build_presence_state(&presences);
        assert!(json.contains("presence_state"));
        assert!(json.contains("user_123"));
    }

    #[tokio::test]
    async fn test_build_presence_diff() {
        let mut joins = HashMap::new();
        joins.insert(
            "user_123".to_string(),
            UserPresencePayload {
                metas: vec![PresenceMeta {
                    connection_id: Uuid::new_v4(),
                    phx_ref: "1".to_string(),
                    state: "online".to_string(),
                    online_at: 1234567890,
                    extra: HashMap::new(),
                }],
            },
        );

        let diff = PresenceDiff {
            joins,
            leaves: HashMap::new(),
        };

        let json = WebSocketRegistry::build_presence_diff(&diff);
        assert!(json.contains("presence_diff"));
        assert!(json.contains("joins"));
        assert!(json.contains("user_123"));
    }

    // ========== Room/Channel Tests ==========

    #[tokio::test]
    async fn test_join_channel() {
        let registry = WebSocketRegistry::new();
        let conn_id = Uuid::new_v4();

        // Join a channel
        registry.join_channel(&conn_id, "room:lobby").await;

        // Verify connection is in channel
        let channel_ids = registry.get_channel_ids("room:lobby").await;
        assert_eq!(channel_ids.len(), 1);
        assert!(channel_ids.contains(&conn_id));
    }

    #[tokio::test]
    async fn test_join_multiple_channels() {
        let registry = WebSocketRegistry::new();
        let conn_id = Uuid::new_v4();

        // Join multiple channels
        registry.join_channel(&conn_id, "room:lobby").await;
        registry.join_channel(&conn_id, "room:general").await;
        registry.join_channel(&conn_id, "room:private").await;

        // Verify connection is in all channels
        assert_eq!(registry.get_channel_ids("room:lobby").await.len(), 1);
        assert_eq!(registry.get_channel_ids("room:general").await.len(), 1);
        assert_eq!(registry.get_channel_ids("room:private").await.len(), 1);
    }

    #[tokio::test]
    async fn test_join_channel_idempotent() {
        let registry = WebSocketRegistry::new();
        let conn_id = Uuid::new_v4();

        // Join same channel twice
        registry.join_channel(&conn_id, "room:lobby").await;
        registry.join_channel(&conn_id, "room:lobby").await;

        // Should only be in channel once
        let channel_ids = registry.get_channel_ids("room:lobby").await;
        assert_eq!(channel_ids.len(), 1);
    }

    #[tokio::test]
    async fn test_leave_channel() {
        let registry = WebSocketRegistry::new();
        let conn_id = Uuid::new_v4();

        // Join then leave
        registry.join_channel(&conn_id, "room:lobby").await;
        registry.leave_channel(&conn_id, "room:lobby").await;

        // Should be empty
        let channel_ids = registry.get_channel_ids("room:lobby").await;
        assert_eq!(channel_ids.len(), 0);
    }

    #[tokio::test]
    async fn test_leave_channel_preserves_other_channels() {
        let registry = WebSocketRegistry::new();
        let conn_id = Uuid::new_v4();

        // Join multiple channels
        registry.join_channel(&conn_id, "room:lobby").await;
        registry.join_channel(&conn_id, "room:general").await;

        // Leave one
        registry.leave_channel(&conn_id, "room:lobby").await;

        // Should still be in other channel
        assert_eq!(registry.get_channel_ids("room:lobby").await.len(), 0);
        assert_eq!(registry.get_channel_ids("room:general").await.len(), 1);
    }

    #[tokio::test]
    async fn test_multiple_connections_in_channel() {
        let registry = WebSocketRegistry::new();
        let conn_id1 = Uuid::new_v4();
        let conn_id2 = Uuid::new_v4();
        let conn_id3 = Uuid::new_v4();

        // Three connections join same channel
        registry.join_channel(&conn_id1, "room:lobby").await;
        registry.join_channel(&conn_id2, "room:lobby").await;
        registry.join_channel(&conn_id3, "room:lobby").await;

        let channel_ids = registry.get_channel_ids("room:lobby").await;
        assert_eq!(channel_ids.len(), 3);
        assert!(channel_ids.contains(&conn_id1));
        assert!(channel_ids.contains(&conn_id2));
        assert!(channel_ids.contains(&conn_id3));
    }

    #[tokio::test]
    async fn test_leave_channel_one_of_many() {
        let registry = WebSocketRegistry::new();
        let conn_id1 = Uuid::new_v4();
        let conn_id2 = Uuid::new_v4();

        // Two connections join
        registry.join_channel(&conn_id1, "room:lobby").await;
        registry.join_channel(&conn_id2, "room:lobby").await;

        // One leaves
        registry.leave_channel(&conn_id1, "room:lobby").await;

        // Other should still be there
        let channel_ids = registry.get_channel_ids("room:lobby").await;
        assert_eq!(channel_ids.len(), 1);
        assert!(channel_ids.contains(&conn_id2));
        assert!(!channel_ids.contains(&conn_id1));
    }

    #[tokio::test]
    async fn test_get_channel_ids_empty_channel() {
        let registry = WebSocketRegistry::new();

        // Non-existent channel should return empty vec
        let channel_ids = registry.get_channel_ids("room:nonexistent").await;
        assert_eq!(channel_ids.len(), 0);
    }

    #[tokio::test]
    async fn test_channel_cleanup_on_last_leave() {
        let registry = WebSocketRegistry::new();
        let conn_id = Uuid::new_v4();

        // Join and leave
        registry.join_channel(&conn_id, "room:lobby").await;
        registry.leave_channel(&conn_id, "room:lobby").await;

        // Channel should be cleaned up (empty)
        let channel_ids = registry.get_channel_ids("room:lobby").await;
        assert!(channel_ids.is_empty());
    }

    #[tokio::test]
    async fn test_broadcast_to_channel_membership() {
        let registry = WebSocketRegistry::new();
        let conn_id1 = Uuid::new_v4();
        let conn_id2 = Uuid::new_v4();
        let conn_id3 = Uuid::new_v4();

        // conn1 and conn2 in lobby, conn3 in general
        registry.join_channel(&conn_id1, "room:lobby").await;
        registry.join_channel(&conn_id2, "room:lobby").await;
        registry.join_channel(&conn_id3, "room:general").await;

        // Verify channel membership
        let lobby_ids = registry.get_channel_ids("room:lobby").await;
        let general_ids = registry.get_channel_ids("room:general").await;

        assert_eq!(lobby_ids.len(), 2);
        assert!(lobby_ids.contains(&conn_id1));
        assert!(lobby_ids.contains(&conn_id2));
        assert!(!lobby_ids.contains(&conn_id3));

        assert_eq!(general_ids.len(), 1);
        assert!(general_ids.contains(&conn_id3));
    }

    #[tokio::test]
    async fn test_broadcast_to_channel_except_membership() {
        let registry = WebSocketRegistry::new();
        let conn_id1 = Uuid::new_v4();
        let conn_id2 = Uuid::new_v4();
        let conn_id3 = Uuid::new_v4();

        // All join lobby
        registry.join_channel(&conn_id1, "room:lobby").await;
        registry.join_channel(&conn_id2, "room:lobby").await;
        registry.join_channel(&conn_id3, "room:lobby").await;

        // Verify all are members before broadcast
        let channel_ids = registry.get_channel_ids("room:lobby").await;
        assert_eq!(channel_ids.len(), 3);

        // broadcast_to_channel_except should work without panic
        // (actual message delivery requires real WebSocket connections)
        registry
            .broadcast_to_channel_except("room:lobby", r#"{"msg":"hello"}"#, &conn_id1)
            .await;
    }

    #[tokio::test]
    async fn test_connection_count() {
        let registry = WebSocketRegistry::new();

        // Initially empty
        assert_eq!(registry.connection_count().await, 0);

        // Note: We can't fully test connection registration without real WebSocket senders,
        // but we can verify the method works
    }

    #[tokio::test]
    async fn test_handler_action_from_value_join() {
        let mut hash: IndexMap<HashKey, Value> = IndexMap::new();
        hash.insert(
            HashKey::String("join".to_string()),
            Value::String("room:lobby".to_string()),
        );
        let value = Value::Hash(Rc::new(RefCell::new(hash)));

        let action = WebSocketHandlerAction::from_value(&value);
        assert_eq!(action.join, Some("room:lobby".to_string()));
        assert!(action.leave.is_none());
    }

    #[tokio::test]
    async fn test_handler_action_from_value_leave() {
        let mut hash: IndexMap<HashKey, Value> = IndexMap::new();
        hash.insert(
            HashKey::String("leave".to_string()),
            Value::String("room:lobby".to_string()),
        );
        let value = Value::Hash(Rc::new(RefCell::new(hash)));

        let action = WebSocketHandlerAction::from_value(&value);
        assert_eq!(action.leave, Some("room:lobby".to_string()));
    }

    #[tokio::test]
    async fn test_handler_action_from_value_broadcast_room() {
        let mut hash: IndexMap<HashKey, Value> = IndexMap::new();
        hash.insert(
            HashKey::String("broadcast_room".to_string()),
            Value::String(r#"{"msg":"hello"}"#.to_string()),
        );
        let value = Value::Hash(Rc::new(RefCell::new(hash)));

        let action = WebSocketHandlerAction::from_value(&value);
        assert_eq!(
            action.broadcast_room,
            Some(r#"{"msg":"hello"}"#.to_string())
        );
    }

    #[tokio::test]
    async fn test_handler_action_from_value_track() {
        let mut track_hash: IndexMap<HashKey, Value> = IndexMap::new();
        track_hash.insert(
            HashKey::String("channel".to_string()),
            Value::String("room:lobby".to_string()),
        );
        track_hash.insert(
            HashKey::String("user_id".to_string()),
            Value::String("user_123".to_string()),
        );
        track_hash.insert(
            HashKey::String("name".to_string()),
            Value::String("Alice".to_string()),
        );

        let mut hash: IndexMap<HashKey, Value> = IndexMap::new();
        hash.insert(
            HashKey::String("track".to_string()),
            Value::Hash(Rc::new(RefCell::new(track_hash))),
        );
        let value = Value::Hash(Rc::new(RefCell::new(hash)));

        let action = WebSocketHandlerAction::from_value(&value);
        assert!(action.track.is_some());
        let track = action.track.unwrap();
        assert_eq!(track.get("channel"), Some(&"room:lobby".to_string()));
        assert_eq!(track.get("user_id"), Some(&"user_123".to_string()));
        assert_eq!(track.get("name"), Some(&"Alice".to_string()));
    }

    #[tokio::test]
    async fn test_handler_action_from_value_multiple_actions() {
        let mut hash: IndexMap<HashKey, Value> = IndexMap::new();
        hash.insert(
            HashKey::String("join".to_string()),
            Value::String("room:lobby".to_string()),
        );
        hash.insert(
            HashKey::String("send".to_string()),
            Value::String("Welcome!".to_string()),
        );
        let value = Value::Hash(Rc::new(RefCell::new(hash)));

        let action = WebSocketHandlerAction::from_value(&value);
        assert_eq!(action.join, Some("room:lobby".to_string()));
        assert_eq!(action.send, Some("Welcome!".to_string()));
    }
}
