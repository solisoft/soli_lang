//! LiveView - Real-time server-rendered HTML with WebSocket communication.
//!
//! This module provides the foundation for LiveView functionality.

pub mod bus;
pub mod changefeed;
pub mod component;
pub mod diff;
pub mod live_query;
pub mod nested;
pub mod parser;
pub mod socket;
pub mod update;
pub mod upload;
pub mod view;

pub use diff::compute_patch;
pub use parser::{parse_live_directives, LiveDirective};
pub use socket::{
    cleanup, handle_event, handle_live_connection, liveview_instance_id, sanitize_mount_id,
};
pub use view::{LiveRegistry, LiveViewId, LiveViewInstance, ServerMessage};

/// Path where the embedded LiveView client script is served.
pub const LIVE_CLIENT_PATH: &str = "/live/client.js";

/// The LiveView client, embedded at compile time so the client protocol
/// (shadow splices + DOM morphing) always matches the server that serves it.
pub const LIVE_CLIENT_JS: &str = include_str!("client.js");

/// The client's ETag: a hash of its text, not the Soli version. A client
/// changed without a version bump (a build from source, a patched release)
/// gets a new tag, so a browser revalidating with `no-cache` picks it up.
pub fn live_client_etag() -> &'static str {
    static ETAG: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ETAG.get_or_init(|| {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        LIVE_CLIENT_JS.hash(&mut hasher);
        format!("\"soli-live-{:016x}\"", hasher.finish())
    })
}

/// Session timeout for LiveViews (seconds)
pub const LIVE_SESSION_TIMEOUT: u64 = 3600;
