//! Which application serves a request.
//!
//! A process serving one application answers everything with it. A process
//! serving several picks by `Host` header, which is the only thing the client
//! tells us about which site it meant.
//!
//! Generic over the value so the routing and the header parsing can be tested
//! on their own, without standing up a worker pool.

use std::collections::HashMap;

/// Normalise a `Host` header into a key the routing table can be built from.
///
/// Returns `None` for anything that cannot be a host, so a caller distinguishes
/// "no usable host" from "a host nobody claims" — the first is a malformed
/// request, the second is a 421.
///
/// The cases that matter, all of which a naive `split(':').next()` gets wrong:
///
/// * **Case.** `Host` is case-insensitive; `EXAMPLE.com` and `example.com` are
///   the same site.
/// * **Port.** `example.com:8443` is the same site as `example.com`. A site is
///   not registered per port — the proxy in front routinely moves it.
/// * **IPv6.** `[::1]:8080` keeps its brackets and has colons *inside* them, so
///   the port split has to start after the closing bracket or `[::1]` becomes
///   `[`.
/// * **Trailing dot.** `example.com.` is the fully-qualified spelling of
///   `example.com` and some clients send it.
pub(crate) fn normalize_host(raw: &str) -> Option<String> {
    let host = raw.trim();
    if host.is_empty() {
        return None;
    }

    let without_port = if let Some(rest) = host.strip_prefix('[') {
        // IPv6 literal: everything up to and including the closing bracket is
        // the host, and only a colon after it introduces a port.
        let close = rest.find(']')?;
        &host[..close + 2]
    } else {
        match host.split_once(':') {
            Some((name, _port)) => name,
            None => host,
        }
    };

    let trimmed = without_port.trim_end_matches('.');
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_ascii_lowercase())
}

/// Maps a request's `Host` to the application that serves it.
pub(crate) struct Router<T> {
    by_host: HashMap<String, T>,
    /// Answered when the `Host` matches nothing.
    ///
    /// A single-application server is entirely this: it has always answered
    /// every request regardless of the header, and keeping that as a fallback
    /// rather than requiring a registered host is what makes the multi-tenant
    /// work a no-op for `soli serve`.
    fallback: Option<T>,
}

impl<T> Router<T> {
    /// A router for a server with one application, which answers everything.
    pub(crate) fn single(value: T) -> Self {
        Self {
            by_host: HashMap::new(),
            fallback: Some(value),
        }
    }

    /// An empty router, to be filled with [`insert`](Self::insert).
    ///
    /// Unused until a host mounts more than one application; `soli serve` uses
    /// [`single`](Self::single). Kept rather than added later because the
    /// routing rules — case, port, IPv6, trailing dot, a host claimed twice —
    /// are the part worth having under test before anything depends on them.
    #[allow(dead_code)]
    pub(crate) fn new() -> Self {
        Self {
            by_host: HashMap::new(),
            fallback: None,
        }
    }

    /// Claim a host for an application.
    ///
    /// Returns whether the host was free. A host claimed twice is a
    /// configuration error the caller should report rather than resolve
    /// silently: whichever application loses would be unreachable, and which
    /// one that is would depend on mount order.
    #[allow(dead_code)]
    pub(crate) fn insert(&mut self, host: &str, value: T) -> bool {
        match normalize_host(host) {
            Some(key) if !self.by_host.contains_key(&key) => {
                self.by_host.insert(key, value);
                true
            }
            _ => false,
        }
    }

    /// The application serving this request, or `None` if no application claims
    /// the host and there is no fallback — a 421 Misdirected Request.
    pub(crate) fn resolve(&self, host_header: Option<&str>) -> Option<&T> {
        // A single-application server has nothing to look up, and this runs
        // on every request: skip the lowercase allocation and the hash probe
        // that could only ever answer the fallback.
        if self.by_host.is_empty() {
            return self.fallback.as_ref();
        }
        host_header
            .and_then(normalize_host)
            .and_then(|key| self.by_host.get(&key))
            .or(self.fallback.as_ref())
    }

    /// How many applications this router serves, fallback included.
    #[allow(dead_code)]
    pub(crate) fn len(&self) -> usize {
        self.by_host.len() + usize::from(self.fallback.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_is_normalised_case_port_and_trailing_dot() {
        assert_eq!(normalize_host("Example.COM"), Some("example.com".into()));
        assert_eq!(
            normalize_host("example.com:8443"),
            Some("example.com".into())
        );
        assert_eq!(normalize_host("example.com."), Some("example.com".into()));
        assert_eq!(
            normalize_host("  EXAMPLE.com.:443 "),
            Some("example.com".into())
        );
    }

    #[test]
    fn an_ipv6_literal_keeps_its_brackets_and_loses_only_the_port() {
        // The naive `split(':').next()` turns `[::1]:8080` into `[`.
        assert_eq!(normalize_host("[::1]:8080"), Some("[::1]".into()));
        assert_eq!(normalize_host("[::1]"), Some("[::1]".into()));
        assert_eq!(
            normalize_host("[2001:db8::1]:443"),
            Some("[2001:db8::1]".into())
        );
    }

    #[test]
    fn nothing_usable_is_none_rather_than_an_empty_key() {
        assert_eq!(normalize_host(""), None);
        assert_eq!(normalize_host("   "), None);
        assert_eq!(normalize_host(":8080"), None);
        assert_eq!(normalize_host("."), None);
    }

    #[test]
    fn a_single_application_answers_every_host_and_no_host_at_all() {
        let router = Router::single("only");
        assert_eq!(router.resolve(Some("example.com")), Some(&"only"));
        assert_eq!(router.resolve(Some("anything.else")), Some(&"only"));
        assert_eq!(router.resolve(None), Some(&"only"));
    }

    #[test]
    fn each_application_answers_its_own_host() {
        let mut router = Router::new();
        assert!(router.insert("blog.example.com", "blog"));
        assert!(router.insert("Shop.Example.COM:8443", "shop"));

        assert_eq!(router.resolve(Some("blog.example.com")), Some(&"blog"));
        // Registered with a port and mixed case, resolved without either.
        assert_eq!(router.resolve(Some("shop.example.com")), Some(&"shop"));
        assert_eq!(router.resolve(Some("shop.example.com:443")), Some(&"shop"));
        assert_eq!(router.len(), 2);
    }

    #[test]
    fn an_unclaimed_host_is_refused_when_there_is_no_fallback() {
        let mut router = Router::new();
        router.insert("blog.example.com", "blog");
        // Nobody claims it: the caller answers 421 rather than handing the
        // request to whichever application happens to be first.
        assert_eq!(router.resolve(Some("other.example.com")), None);
        assert_eq!(router.resolve(None), None);
    }

    #[test]
    fn a_host_claimed_twice_is_refused_rather_than_silently_reassigned() {
        let mut router = Router::new();
        assert!(router.insert("example.com", "first"));
        assert!(
            !router.insert("EXAMPLE.com.", "second"),
            "the same host in another spelling is still the same host"
        );
        assert_eq!(
            router.resolve(Some("example.com")),
            Some(&"first"),
            "the first claim keeps the host; the second is reported, not applied"
        );
    }
}
