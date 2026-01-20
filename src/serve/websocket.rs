//! WebSocket support for the Solilang MVC framework.
//!
//! This module provides WebSocket handling including:
//! - Connection management with unique IDs
//! - Channel/room support for targeted broadcasting
//! - Single handler pattern for all WebSocket events (connect, message, disconnect)

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use tokio::sync::Mutex as AsyncMutex;
use tungstenite::Message;
use uuid::Uuid;

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
}

impl WebSocketRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            connections: Arc::new(AsyncMutex::new(HashMap::new())),
            channels: Arc::new(AsyncMutex::new(HashMap::new())),
        }
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
        let mut pairs: Vec<(Value, Value)> = vec![
            (
                Value::String("type".to_string()),
                Value::String(self.event_type.clone()),
            ),
            (
                Value::String("connection_id".to_string()),
                Value::String(self.connection_id.clone()),
            ),
        ];

        if let Some(ref msg) = self.message {
            pairs.push((
                Value::String("message".to_string()),
                Value::String(msg.clone()),
            ));
        }

        if let Some(ref channel) = self.channel {
            pairs.push((
                Value::String("channel".to_string()),
                Value::String(channel.clone()),
            ));
        }

        Value::Hash(Rc::new(RefCell::new(pairs)))
    }
}

use crate::interpreter::value::Value;

/// Actions that a WebSocket handler can return.
#[derive(Clone)]
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
        }
    }

    /// Parse actions from a handler return value.
    pub fn from_value(value: &Value) -> Self {
        let mut action = Self::new();

        if let Value::Hash(hash) = value {
            for (k, v) in hash.borrow().iter() {
                if let Value::String(key) = k {
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

// Global WebSocket registry for use from tokio threads
lazy_static::lazy_static! {
    pub static ref GLOBAL_WS_REGISTRY: std::sync::Arc<WebSocketRegistry> = std::sync::Arc::new(WebSocketRegistry::new());
}

/// Get the global WebSocket registry.
pub fn get_ws_registry() -> std::sync::Arc<WebSocketRegistry> {
    GLOBAL_WS_REGISTRY.clone()
}
