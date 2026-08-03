//! Locating a Chromium-family browser on this machine.
//!
//! Two callers want a browser and want different things from it: the desktop
//! shell wants *something* to open a window with and treats failure as cosmetic,
//! while the browser test driver needs a real path to a real binary it can drive
//! over the DevTools protocol. The candidate list is the same either way, so it
//! lives here rather than in one of them.

use std::path::{Path, PathBuf};

/// Chromium-family binaries, in the order we would rather have them.
///
/// Order is preference, not availability: Chrome before Chromium because it is
/// the one most likely to match what CI runs, and Edge/Brave last because they
/// are Chromium underneath but lag it.
#[cfg(target_os = "linux")]
pub const CHROMIUM_BINARIES: &[&str] = &[
    "google-chrome",
    "google-chrome-stable",
    "chromium",
    "chromium-browser",
    "microsoft-edge",
    "brave-browser",
];

#[cfg(target_os = "macos")]
pub const CHROMIUM_BINARIES: &[&str] = &[
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
];

#[cfg(target_os = "windows")]
pub const CHROMIUM_BINARIES: &[&str] = &["chrome.exe", "msedge.exe", "chrome", "msedge"];

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub const CHROMIUM_BINARIES: &[&str] = &[];

/// Environment override, checked before the candidate list.
pub const CHROME_PATH_ENV: &str = "SOLI_CHROME_PATH";

/// Find a browser we can drive over the DevTools protocol.
///
/// `$SOLI_CHROME_PATH` wins outright — someone who set it has a reason, and
/// silently preferring a different browser would be worse than failing. After
/// that it is the candidate list, resolved against `$PATH`.
///
/// Returns a path that existed at the time of the call. That is not a promise it
/// still exists when spawned, which is why the caller still has to handle a
/// spawn failure.
pub fn find_chrome() -> Option<PathBuf> {
    find_chromes().into_iter().next()
}

/// Every browser on this machine, most preferred first.
///
/// Being installed is not the same as being launchable: a snap-packaged Chromium
/// on a host whose user session has no D-Bus refuses to start at all, and a
/// caller that only ever saw the first candidate would fail the whole run with a
/// working browser sitting next in the list. So the driver gets the list and
/// tries them in turn.
///
/// The override still resolves to at most one entry — falling through from a
/// pinned browser to a different one would defeat the point of pinning it.
pub fn find_chromes() -> Vec<PathBuf> {
    if let Ok(explicit) = std::env::var(CHROME_PATH_ENV) {
        let explicit = explicit.trim();
        if !explicit.is_empty() {
            let path = PathBuf::from(explicit);
            // An override that doesn't resolve is a configuration error worth
            // surfacing, not a reason to quietly fall through to a different
            // browser than the one that was asked for.
            return if path.is_file() {
                vec![path]
            } else {
                Vec::new()
            };
        }
    }

    walk_candidates(CHROMIUM_BINARIES, resolve)
}

/// The candidate walk, with resolution injected so it can be tested without
/// installing browsers.
fn walk_candidates(candidates: &[&str], resolve: impl Fn(&str) -> Option<PathBuf>) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    let mut seen: Vec<PathBuf> = Vec::new();
    for candidate in candidates {
        let Some(path) = resolve(candidate) else {
            continue;
        };
        // `google-chrome` and `google-chrome-stable` are usually the same file
        // through a symlink; trying it twice would only double the wait on a
        // machine where it does not work.
        let identity = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if seen.contains(&identity) {
            continue;
        }
        seen.push(identity);
        found.push(path);
    }
    found
}

/// Whether `$SOLI_CHROME_PATH` is set to something non-empty.
///
/// Lets a caller tell "no browser installed" apart from "the override points at
/// nothing", which are different problems with different fixes.
pub fn chrome_path_override() -> Option<String> {
    std::env::var(CHROME_PATH_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Resolve one candidate to an executable path.
///
/// An absolute path (the macOS entries) is used as-is; a bare name is looked up
/// in `$PATH`. There is no `which` crate in the tree and this is the only place
/// that needs one.
fn resolve(candidate: &str) -> Option<PathBuf> {
    let as_path = Path::new(candidate);
    if as_path.is_absolute() {
        return as_path.is_file().then(|| as_path.to_path_buf());
    }

    let path_var = std::env::var_os("PATH")?;
    std::env::split_paths(&path_var).find_map(|dir| {
        let full = dir.join(candidate);
        full.is_file().then_some(full)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_are_listed_for_this_platform() {
        // A platform with no candidates can never find a browser, which is a
        // decision worth noticing if it ever changes by accident.
        #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
        assert!(!CHROMIUM_BINARIES.is_empty());
    }

    #[test]
    fn an_absolute_candidate_that_does_not_exist_resolves_to_nothing() {
        assert_eq!(resolve("/soli/definitely/not/a/browser"), None);
    }

    #[test]
    fn a_bare_name_that_is_not_on_path_resolves_to_nothing() {
        assert_eq!(resolve("soli-definitely-not-a-real-browser-binary"), None);
    }

    #[test]
    fn every_installed_browser_is_offered_in_preference_order() {
        // The driver needs the whole list: the first browser can be installed and
        // still refuse to run (a snap launcher in a session without D-Bus), and
        // the run should continue with the next one.
        let found = walk_candidates(&["first", "missing", "second"], |name| match name {
            "first" => Some(PathBuf::from("/bin/first")),
            "second" => Some(PathBuf::from("/bin/second")),
            _ => None,
        });
        assert_eq!(
            found,
            vec![PathBuf::from("/bin/first"), PathBuf::from("/bin/second")]
        );
    }

    #[test]
    fn one_browser_under_two_names_is_offered_once() {
        // `google-chrome` and `google-chrome-stable` are the same binary on most
        // Debian machines.
        let found = walk_candidates(&["chrome", "chrome-stable"], |_| {
            Some(PathBuf::from("/bin/sh"))
        });
        assert_eq!(found, vec![PathBuf::from("/bin/sh")]);
    }

    #[test]
    fn a_bare_name_on_path_resolves_to_an_existing_file() {
        // `sh` stands in for a browser: the point is the $PATH walk, not the
        // binary. Every supported platform's test runner has a shell.
        #[cfg(unix)]
        {
            let found = resolve("sh").expect("sh must be on PATH");
            assert!(found.is_file());
            assert!(found.is_absolute());
        }
    }
}
