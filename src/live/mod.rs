//! LiveView - Real-time server-rendered HTML with WebSocket communication.
//!
//! This module provides the foundation for LiveView functionality.

pub mod component;
pub mod diff;
pub mod live_query;
pub mod parser;
pub mod socket;
pub mod view;

pub use diff::compute_patch;
pub use parser::{parse_live_directives, LiveDirective};
pub use socket::{cleanup, handle_event, handle_live_connection};
pub use view::{LiveRegistry, LiveViewId, LiveViewInstance, ServerMessage};

/// LiveView socket path
pub const LIVE_SOCKET_PATH: &str = "/live/socket";

/// Path where the embedded LiveView client script is served.
pub const LIVE_CLIENT_PATH: &str = "/live/client.js";

/// The LiveView client, embedded at compile time so the client protocol
/// (shadow splices + DOM morphing) always matches the server that serves it.
pub const LIVE_CLIENT_JS: &str = include_str!("client.js");

/// Maximum time between heartbeats (seconds)
pub const HEARTBEAT_INTERVAL: u64 = 30;

/// Session timeout for LiveViews (seconds)
pub const LIVE_SESSION_TIMEOUT: u64 = 3600;
