//! Covers the bound-port hook on `serve::serve_folder_with_options_and_hooks`.
//!
//! Lives in its own test binary on purpose: serving installs process-global
//! state (VFS, file jails, app root) and never returns, so sharing a binary
//! with other tests would leak that state into them.

use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// The hook must fire with the port the server actually bound — including when
/// the caller asked for an ephemeral one and therefore cannot know it up front.
///
/// This is the only moment an embedding caller can learn the port: serving
/// blocks forever joining its workers, so there is no "after" to inspect.
#[test]
fn bound_port_hook_receives_a_real_serving_port() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/_e2e_app");
    assert!(fixture.exists(), "missing fixture: {:?}", fixture);

    // Loopback-only so the test never advertises itself on the LAN, and so the
    // boot path skips the UDP probe it otherwise makes to print a LAN URL.
    std::env::set_var("SOLI_HOST", "127.0.0.1");

    let (tx, rx) = mpsc::channel::<u16>();
    thread::spawn(move || {
        // Port 0: the kernel assigns, so the hook is the only way to find out.
        let _ = solilang::serve::serve_folder_with_options_and_hooks(
            &fixture,
            0,
            false,
            1,
            Some(Box::new(move |port| {
                let _ = tx.send(port);
            })),
        );
    });

    let port = rx
        .recv_timeout(Duration::from_secs(30))
        .expect("bound-port hook never fired");
    assert_ne!(port, 0, "hook received 0 rather than the assigned port");

    // The hook fires once the listener is accepting, but workers finish booting
    // a moment later — poll rather than asserting on a single request.
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut reachable = false;
    while Instant::now() < deadline {
        if ureq::get(&format!("http://127.0.0.1:{}/ping", port))
            .timeout(Duration::from_millis(500))
            .call()
            .is_ok()
        {
            reachable = true;
            break;
        }
        thread::sleep(Duration::from_millis(200));
    }
    assert!(
        reachable,
        "hook reported port {} but nothing is serving there",
        port
    );

    // The serving thread is intentionally left running; it never returns, and
    // the test binary exiting tears it down.
}
