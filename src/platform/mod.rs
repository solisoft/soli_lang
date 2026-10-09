//! Platform-specific primitives, isolated behind cross-platform APIs.
//!
//! Everything here exists because the operation genuinely differs per OS —
//! advisory file locking, process liveness, private directory creation. Keeping
//! the `cfg` branches in one place stops them from spreading through call
//! sites, and gives the Windows port a single set of holes to fill rather than
//! an audit of every module.

pub mod browser;
pub mod dirs;
pub mod env;
pub mod fs;
pub mod job;
#[cfg(target_arch = "wasm32")]
pub mod jspi;
pub mod lock;
pub mod process;

/// Seconds since the Unix epoch by the wall clock (0 for a clock set before
/// it). Not the frozen test clock: token expiries and cookie lifetimes are
/// checked against real time.
pub fn unix_now_secs() -> u64 {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The error a builtin returns on the edge (wasm32) build when what it needs —
/// a socket, a subprocess, a thread, the disk — does not exist there.
pub fn unsupported_on_edge(what: &str) -> String {
    format!("{what} is not available on the edge (Cloudflare Workers) build")
}
