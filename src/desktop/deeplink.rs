//! Pending open path for desktop deep links / custom schemes.
//!
//! A second instance or OS protocol handler launches the artifact with a URL
//! (`myapp://pings/3`) or path (`/pings/3`). Boot stores the mapped path; the
//! launch-token exchange redirects there instead of `/`.

use std::sync::Mutex;

static PENDING_PATH: Mutex<Option<String>> = Mutex::new(None);

/// Remember where to land after the launch-token session is granted.
pub fn set_pending_path(path: impl Into<String>) {
    if let Ok(mut guard) = PENDING_PATH.lock() {
        *guard = Some(normalize_path(&path.into()));
    }
}

/// Take the pending path (if any), clearing it.
pub fn take_pending_path() -> Option<String> {
    PENDING_PATH.lock().ok().and_then(|mut g| g.take())
}

/// Peek without clearing — used when minting the grant redirect.
pub fn peek_pending_path() -> Option<String> {
    PENDING_PATH.lock().ok().and_then(|g| g.clone())
}

/// Map a CLI arg or protocol URL to an in-app path (`/…`).
///
/// Accepts:
/// - `/pings/3`
/// - `myapp://host/pings/3` or `myapp:///pings/3`
/// - `https://example.com/pings/3` (path + query only)
/// - `--open=/pings/3` style is handled by the caller stripping the flag
pub fn path_from_open_arg(arg: &str) -> Option<String> {
    let arg = arg.trim();
    if arg.is_empty() {
        return None;
    }
    if arg.starts_with('/') {
        return Some(normalize_path(arg));
    }
    // scheme://…
    if let Some(rest) = arg.split_once("://").map(|(_, r)| r) {
        // strip host if present: host/path or /path or empty
        let path_and_query = if rest.starts_with('/') {
            rest
        } else if let Some(idx) = rest.find('/') {
            &rest[idx..]
        } else if rest.is_empty() {
            "/"
        } else {
            // scheme://something-with-no-slash → treat as host only
            "/"
        };
        return Some(normalize_path(path_and_query));
    }
    None
}

/// Scan process args for a deep-link target.
///
/// Recognises `--open <path>`, `--open=<path>`, and bare URLs/paths that look
/// like open targets (not flags, not the executable path).
pub fn pending_from_env_and_args() -> Option<String> {
    if let Ok(v) = std::env::var("SOLI_DESKTOP_OPEN") {
        if let Some(p) = path_from_open_arg(&v) {
            return Some(p);
        }
    }

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--open" {
            if let Some(next) = args.get(i + 1) {
                return path_from_open_arg(next);
            }
        } else if let Some(rest) = a.strip_prefix("--open=") {
            return path_from_open_arg(rest);
        } else if !a.starts_with('-') {
            if let Some(p) = path_from_open_arg(a) {
                return Some(p);
            }
        }
        i += 1;
    }
    None
}

fn normalize_path(path: &str) -> String {
    let mut p = path.trim().to_string();
    if p.is_empty() {
        return "/".to_string();
    }
    if !p.starts_with('/') {
        p.insert(0, '/');
    }
    // Block scheme-relative and protocol-relative abuse after mapping.
    //
    // `//` was covered; `/\` was not, and browsers normalise a `Location` of
    // `/\evil.com` to `//evil.com` — the same open redirect through a
    // different spelling. A backslash has no meaning in a URL path here, so
    // refuse the whole shape rather than try to rewrite it.
    if p.starts_with("//") || p.starts_with("/\\") || p.contains('\\') {
        return "/".to_string();
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_scheme_urls() {
        assert_eq!(
            path_from_open_arg("myapp://host/pings/3"),
            Some("/pings/3".into())
        );
        assert_eq!(
            path_from_open_arg("myapp:///threads/1"),
            Some("/threads/1".into())
        );
        assert_eq!(path_from_open_arg("myapp://"), Some("/".into()));
    }

    #[test]
    fn maps_https_and_paths() {
        assert_eq!(
            path_from_open_arg("https://app.example.com/pings/3?x=1"),
            Some("/pings/3?x=1".into())
        );
        assert_eq!(path_from_open_arg("/dashboard"), Some("/dashboard".into()));
    }

    #[test]
    fn pending_roundtrip() {
        set_pending_path("/a");
        assert_eq!(peek_pending_path().as_deref(), Some("/a"));
        assert_eq!(take_pending_path().as_deref(), Some("/a"));
        assert_eq!(take_pending_path(), None);
    }
}

#[cfg(test)]
mod backslash_redirect_tests {
    use super::*;

    /// `//host` was blocked; `/\host` was not, and browsers normalise a
    /// `Location: /\evil.com` to `//evil.com` — the same off-site redirect.
    #[test]
    fn backslash_forms_cannot_become_a_protocol_relative_url() {
        assert_eq!(normalize_path("/\\evil.com"), "/");
        assert_eq!(normalize_path("\\\\evil.com"), "/");
        assert_eq!(normalize_path("/a\\b"), "/");
    }

    #[test]
    fn ordinary_paths_are_unchanged() {
        assert_eq!(normalize_path("/dashboard"), "/dashboard");
        assert_eq!(normalize_path("posts/7"), "/posts/7");
    }

    #[test]
    fn protocol_relative_urls_are_still_blocked() {
        assert_eq!(normalize_path("//evil.com"), "/");
    }
}
