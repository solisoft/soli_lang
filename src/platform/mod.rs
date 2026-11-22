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
/// May a credential be sent to `url`? Over https, or over http to this
/// machine. Anywhere else over plain http the key travels in clear — a
/// deploy key, a publish token, the key of an encrypted bundle — and whoever
/// is on the path has it. `SOLI_ALLOW_INSECURE_ENDPOINTS=1` says the network
/// in between is trusted (a private segment, a tunnel).
pub fn credential_may_travel_to(url: &str, what: &str) -> Result<(), String> {
    let lower = url.trim().to_ascii_lowercase();
    if lower.starts_with("https://") {
        return Ok(());
    }
    if let Some(rest) = lower.strip_prefix("http://") {
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        let host = match authority.strip_prefix('[') {
            Some(v6) => v6.split(']').next().unwrap_or(""),
            None => authority.split(':').next().unwrap_or(""),
        };
        if matches!(host, "localhost" | "127.0.0.1" | "::1") {
            return Ok(());
        }
    }
    if env::var("SOLI_ALLOW_INSECURE_ENDPOINTS").as_deref() == Ok("1") {
        return Ok(());
    }
    Err(format!(
        "{what}: refusing to send a credential to {url} over plain http. Use https://, or set \
         SOLI_ALLOW_INSECURE_ENDPOINTS=1 if the network in between is trusted."
    ))
}

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

/// Hand the memory this thread has freed back to the OS now.
///
/// mimalloc returns freed pages lazily, when the thread that freed them next
/// allocates. A thread that goes quiet after a burst keeps them: the boot
/// thread, parked for the life of the server once it has dropped the boot
/// interpreter, and a worker waiting for its first request after loading the
/// app. That was some 11 MiB of a two-worker server's 44, held for nothing —
/// `MIMALLOC_PURGE_DELAY=0` released it too, at the cost of a syscall on every
/// later free.
// Linked into every native build, not only the `soli` binary that makes it
// the global allocator: `mi_collect` below is resolved from it.
#[cfg(not(target_arch = "wasm32"))]
extern crate mimalloc as _;

#[cfg(not(target_arch = "wasm32"))]
pub fn release_freed_memory() {
    // The C function is in mimalloc whatever the Rust crate's features; its
    // `extended` bindings would declare it, at the cost of a dependency.
    extern "C" {
        fn mi_collect(force: bool);
    }
    // SAFETY: `mi_collect` takes no pointers. It frees this thread's unused
    // pages (and, forced, abandoned segments); with another global
    // allocator, as in tests, mimalloc's heaps are empty and it does nothing.
    unsafe { mi_collect(true) }
}

/// Nothing to hand back on the edge build: no mimalloc, no threads.
#[cfg(target_arch = "wasm32")]
pub fn release_freed_memory() {}

#[cfg(test)]
mod credential_transport_tests {
    use super::credential_may_travel_to;

    #[test]
    fn a_credential_goes_over_https_or_to_this_machine() {
        if super::env::var("SOLI_ALLOW_INSECURE_ENDPOINTS").as_deref() == Ok("1") {
            return;
        }
        for ok in [
            "https://proxy.example.com",
            "http://localhost:9000",
            "http://127.0.0.1:9000/x",
            "http://[::1]:9000",
        ] {
            assert!(credential_may_travel_to(ok, "t").is_ok(), "{ok}");
        }
        for refused in [
            "http://proxy.example.com",
            "http://10.0.0.5:9000",
            "http://localhost.evil.example",
            "ftp://x",
        ] {
            assert!(credential_may_travel_to(refused, "t").is_err(), "{refused}");
        }
    }
}
