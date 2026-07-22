//! Opening the app's window.
//!
//! No embedded webview yet, so the shell is the user's browser. Two steps down
//! from a real app window, in order of preference:
//!
//! 1. A Chromium-family browser in `--app` mode, which gives a chrome-less
//!    window with no tab strip, address bar or bookmarks — most of the
//!    "desktop app" feel for the cost of one flag.
//! 2. Whatever the OS considers the default browser, as an ordinary tab.
//!
//! Launching is best-effort by design: failing to open a window is not a reason
//! to refuse to run, because the URL is printed and the user can paste it.

use std::process::{Command, Stdio};

/// Chromium-family binaries, in the order we would rather have them.
///
/// Shared with the browser test driver, which needs the same list for a
/// different reason — see `platform::browser`.
use crate::platform::browser::CHROMIUM_BINARIES as APP_MODE_BROWSERS;

/// How the window was opened, for an honest log line.
#[derive(Debug, PartialEq, Eq)]
pub enum Opened {
    /// Chrome-less application window.
    AppWindow,
    /// An ordinary browser tab.
    BrowserTab,
    /// Nothing opened; the user needs the printed URL.
    Nothing,
    /// Deliberately not opened — an embedding shell hosts the view itself.
    Suppressed,
}

/// Set by a native wrapper that embeds this server and renders the app in its
/// own web view. Without it the wrapper gets two windows: its own, and the
/// browser this module launches. The launch URL is printed either way, which
/// is how the wrapper learns the port and the one-shot token.
pub const NO_WINDOW_ENV: &str = "SOLI_DESKTOP_NO_WINDOW";

/// Whether an embedding wrapper has asked us not to open anything.
fn window_suppressed() -> bool {
    std::env::var_os(NO_WINDOW_ENV).is_some_and(|v| !v.is_empty() && v != "0")
}

/// Open `url`, preferring a chrome-less window.
pub fn open(url: &str) -> Opened {
    if window_suppressed() {
        return Opened::Suppressed;
    }
    // A dedicated profile directory would isolate cookies from the user's
    // normal browsing, which matters because the session cookie is not
    // port-scoped. Deliberately not doing that yet: it costs a fresh profile
    // (no extensions, no theme) on every launch, and the isolation is only
    // worth it once there is something more sensitive than a local session to
    // protect. Revisit alongside loopback TLS.
    for browser in APP_MODE_BROWSERS {
        if spawn_detached(browser, &[&format!("--app={}", url)]) {
            return Opened::AppWindow;
        }
    }
    if open_with_default_browser(url) {
        return Opened::BrowserTab;
    }
    Opened::Nothing
}

#[cfg(target_os = "linux")]
fn open_with_default_browser(url: &str) -> bool {
    // $BROWSER first — a user who set it means it.
    if let Ok(browser) = std::env::var("BROWSER") {
        if !browser.is_empty() && spawn_detached(&browser, &[url]) {
            return true;
        }
    }
    spawn_detached("xdg-open", &[url]) || spawn_detached("gio", &["open", url])
}

#[cfg(target_os = "macos")]
fn open_with_default_browser(url: &str) -> bool {
    spawn_detached("open", &[url])
}

#[cfg(target_os = "windows")]
fn open_with_default_browser(url: &str) -> bool {
    // `start` is a shell builtin, not an executable, and the empty string is
    // the window title `start` would otherwise take from the URL.
    spawn_detached("cmd", &["/C", "start", "", url])
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn open_with_default_browser(_url: &str) -> bool {
    false
}

/// Spawn without waiting, discarding output.
///
/// Returns whether the process started — not whether it succeeded. A browser
/// that launches and then fails is indistinguishable from here, which is part
/// of why the URL is always printed as well.
fn spawn_detached(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An embedding wrapper must be able to stop this from opening a browser
    /// on top of its own window — and "0"/empty must not count as opting in.
    /// The predicate is tested rather than `open()` itself, which would launch
    /// a real browser on the machine running the suite.
    #[test]
    fn no_window_env_is_read_as_an_explicit_opt_in() {
        let previous = std::env::var_os(NO_WINDOW_ENV);

        std::env::set_var(NO_WINDOW_ENV, "1");
        assert!(window_suppressed());
        std::env::set_var(NO_WINDOW_ENV, "0");
        assert!(!window_suppressed());
        std::env::set_var(NO_WINDOW_ENV, "");
        assert!(!window_suppressed());
        std::env::remove_var(NO_WINDOW_ENV);
        assert!(!window_suppressed());

        if let Some(v) = previous {
            std::env::set_var(NO_WINDOW_ENV, v);
        }
    }

    #[test]
    fn spawning_a_missing_program_reports_failure_rather_than_panicking() {
        assert!(!spawn_detached(
            "soli-definitely-not-a-real-browser-binary",
            &["https://example.invalid"]
        ));
    }

    #[test]
    fn app_mode_candidates_are_listed_for_this_platform() {
        // A platform with no candidates would silently skip straight to a tab,
        // which is a decision worth noticing if it ever changes by accident.
        #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
        assert!(!APP_MODE_BROWSERS.is_empty());
    }
}
