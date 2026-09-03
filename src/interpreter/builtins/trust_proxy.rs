//! Trust-proxy gate for `X-Forwarded-*` headers.
//!
//! Without this gate, any client that can reach the server directly (i.e. when
//! the app is *not* behind a proxy that strips inbound `X-Forwarded-*`
//! headers) can spoof the values used for the session-cookie `Secure` flag
//! and the host portion of `*_url` helpers — opening attacks like cookie
//! downgrade and URL phishing. We default to **off**: apps must explicitly
//! call `enable_trust_proxy()` after confirming their deployment terminates
//! and rewrites these headers at a trusted proxy hop.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Once;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

pub(crate) static TRUST_PROXY_ENABLED: AtomicBool = AtomicBool::new(false);
static ENV_INIT: Once = Once::new();

thread_local! {
    /// Peer address of the request this worker thread is handling, set by the
    /// serve layer before dispatch. Only consulted when `SOLI_TRUSTED_PROXIES`
    /// is configured.
    #[allow(clippy::missing_const_for_thread_local)]
    static CURRENT_PEER_IP: std::cell::RefCell<Option<std::net::IpAddr>> =
        const { std::cell::RefCell::new(None) };
}

/// Record the TCP peer of the current request (called by the server; `None`
/// clears it between requests).
pub fn set_current_peer_ip(ip: Option<std::net::IpAddr>) {
    CURRENT_PEER_IP.with(|c| *c.borrow_mut() = ip);
}

/// Parsed `SOLI_TRUSTED_PROXIES` entries: an IP or a CIDR block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TrustedProxyEntry {
    network: std::net::IpAddr,
    prefix_len: u8,
}

impl TrustedProxyEntry {
    fn contains(&self, addr: std::net::IpAddr) -> bool {
        match (self.network, addr) {
            (std::net::IpAddr::V4(net), std::net::IpAddr::V4(ip)) => {
                prefix_matches(&net.octets(), &ip.octets(), self.prefix_len)
            }
            (std::net::IpAddr::V6(net), std::net::IpAddr::V6(ip)) => {
                prefix_matches(&net.octets(), &ip.octets(), self.prefix_len)
            }
            // An IPv4-mapped peer (::ffff:a.b.c.d) must still match an IPv4
            // rule, or a dual-stack listener silently trusts nobody.
            (std::net::IpAddr::V4(_), std::net::IpAddr::V6(ip)) => match ip.to_ipv4_mapped() {
                Some(v4) => self.contains(std::net::IpAddr::V4(v4)),
                None => false,
            },
            (std::net::IpAddr::V6(_), std::net::IpAddr::V4(_)) => false,
        }
    }
}

/// Compare the first `prefix_len` bits of two addresses.
fn prefix_matches(network: &[u8], addr: &[u8], prefix_len: u8) -> bool {
    let full_bytes = (prefix_len / 8) as usize;
    let remaining_bits = prefix_len % 8;
    if network[..full_bytes] != addr[..full_bytes] {
        return false;
    }
    if remaining_bits == 0 {
        return true;
    }
    let mask = 0xffu8 << (8 - remaining_bits);
    network[full_bytes] & mask == addr[full_bytes] & mask
}

/// Parse one `SOLI_TRUSTED_PROXIES` entry: `10.0.0.0/8`, `192.168.1.7`, `::1`.
fn parse_trusted_proxy_entry(raw: &str) -> Option<TrustedProxyEntry> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let (addr_part, prefix_part) = match raw.split_once('/') {
        Some((a, p)) => (a, Some(p)),
        None => (raw, None),
    };
    let network: std::net::IpAddr = addr_part.parse().ok()?;
    let max_prefix = if network.is_ipv4() { 32 } else { 128 };
    let prefix_len = match prefix_part {
        Some(p) => {
            let n: u8 = p.trim().parse().ok()?;
            if n > max_prefix {
                return None;
            }
            n
        }
        None => max_prefix,
    };
    Some(TrustedProxyEntry {
        network,
        prefix_len,
    })
}

static TRUSTED_PROXIES: std::sync::OnceLock<Vec<TrustedProxyEntry>> = std::sync::OnceLock::new();

fn trusted_proxies() -> &'static Vec<TrustedProxyEntry> {
    TRUSTED_PROXIES.get_or_init(|| {
        std::env::var("SOLI_TRUSTED_PROXIES")
            .ok()
            .map(|raw| {
                raw.split(',')
                    .filter_map(parse_trusted_proxy_entry)
                    .collect()
            })
            .unwrap_or_default()
    })
}

/// Whether the server should honor `X-Forwarded-Proto` / `X-Forwarded-Host`
/// from incoming requests.
pub fn is_trust_proxy_enabled() -> bool {
    // Seeded here, not only from `register_trust_proxy_builtins`.
    //
    // The CSRF and WebSocket origin checks run in the HTTP server path, which
    // may ask this question before any interpreter `Environment` has been
    // constructed in that thread — so seeding only at builtin-registration
    // time made `SOLI_TRUST_PROXY=1` silently ineffective for exactly the two
    // checks it exists to configure. Behind a reverse proxy that meant every
    // form post was rejected with "Origin <public host> does not match request
    // authority <backend>", with no way to fix it from the environment.
    //
    // `Once` makes this free after the first call, and `enable_trust_proxy()`
    // / `disable_trust_proxy()` still override at runtime.
    init_from_env();

    if !TRUST_PROXY_ENABLED.load(Ordering::Relaxed) {
        return false;
    }

    // `SOLI_TRUSTED_PROXIES` narrows the gate to named hops. Trusting
    // `X-Forwarded-*` is only sound when the request actually arrived from the
    // proxy that rewrites them; with the list set, a client that reaches the
    // app directly is not trusted even though the flag is on. Unset (the
    // default) keeps the previous all-or-nothing behaviour.
    let trusted = trusted_proxies();
    if trusted.is_empty() {
        return true;
    }
    CURRENT_PEER_IP.with(|c| match *c.borrow() {
        // No peer recorded: not an HTTP request path (a job, a script, a test),
        // where there are no inbound forwarded headers to distrust.
        None => true,
        Some(peer) => trusted.iter().any(|entry| entry.contains(peer)),
    })
}

/// Parse a `SOLI_TRUST_PROXY` value. Truthy values (`1`, `true`, `yes`,
/// case-insensitive) flip the gate on. Anything else (including missing or
/// empty) leaves it off. Factored out so tests can exercise the parser
/// without racing on `std::env::var` or the `Once`-protected init.
fn parse_trust_proxy_env(raw: Option<&str>) -> bool {
    match raw {
        Some(s) => matches!(s.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"),
        None => false,
    }
}

/// Read `SOLI_TRUST_PROXY` once and seed the flag from it. `enable_trust_proxy()`
/// / `disable_trust_proxy()` still override at runtime — env just sets the
/// startup default so deployments can flip the flag without editing app code.
fn init_from_env() {
    ENV_INIT.call_once(|| {
        let raw = std::env::var("SOLI_TRUST_PROXY").ok();
        if parse_trust_proxy_env(raw.as_deref()) {
            TRUST_PROXY_ENABLED.store(true, Ordering::Relaxed);
        }
    });
}

pub fn register_trust_proxy_builtins(env: &mut Environment) {
    init_from_env();

    env.define(
        "enable_trust_proxy".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "enable_trust_proxy",
            Some(0),
            |_args| {
                TRUST_PROXY_ENABLED.store(true, Ordering::Relaxed);
                Ok(Value::Bool(true))
            },
        )),
    );

    env.define(
        "disable_trust_proxy".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "disable_trust_proxy",
            Some(0),
            |_args| {
                TRUST_PROXY_ENABLED.store(false, Ordering::Relaxed);
                Ok(Value::Bool(true))
            },
        )),
    );

    env.define(
        "trust_proxy_enabled".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "trust_proxy_enabled",
            Some(0),
            |_args| Ok(Value::Bool(is_trust_proxy_enabled())),
        )),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bundled into one `#[test]` because the flag is process-global —
    /// running the cases as separate tests would race them under cargo's
    /// default parallel runner and produce nondeterministic results.
    #[test]
    fn trust_proxy_toggle_round_trip() {
        // Default state is off, regardless of test ordering: explicitly
        // disable first so we don't depend on which test ran before us.
        TRUST_PROXY_ENABLED.store(false, Ordering::Relaxed);
        assert!(!is_trust_proxy_enabled());

        TRUST_PROXY_ENABLED.store(true, Ordering::Relaxed);
        assert!(is_trust_proxy_enabled());

        TRUST_PROXY_ENABLED.store(false, Ordering::Relaxed);
        assert!(!is_trust_proxy_enabled());
    }

    #[test]
    fn env_parser_recognizes_truthy_and_rejects_other() {
        for truthy in ["1", "true", "True", "TRUE", "yes", "YES", " yes ", "True\n"] {
            assert!(
                parse_trust_proxy_env(Some(truthy)),
                "expected truthy for {:?}",
                truthy
            );
        }
        for falsy in ["", " ", "0", "false", "no", "off", "maybe", "1; rm -rf /"] {
            assert!(
                !parse_trust_proxy_env(Some(falsy)),
                "expected falsy for {:?}",
                falsy
            );
        }
        assert!(!parse_trust_proxy_env(None));
    }
}

#[cfg(test)]
mod trusted_proxy_tests {
    use super::*;
    use std::net::IpAddr;

    fn entry(raw: &str) -> TrustedProxyEntry {
        parse_trusted_proxy_entry(raw).unwrap_or_else(|| panic!("{raw} should parse"))
    }

    #[test]
    fn cidr_membership_is_bitwise_not_textual() {
        let net = entry("10.0.0.0/8");
        assert!(net.contains("10.1.2.3".parse::<IpAddr>().unwrap()));
        assert!(net.contains("10.255.255.255".parse::<IpAddr>().unwrap()));
        assert!(!net.contains("11.0.0.1".parse::<IpAddr>().unwrap()));
        // A prefix that does not land on a byte boundary must still be exact.
        let net = entry("192.168.4.0/22");
        assert!(net.contains("192.168.7.9".parse::<IpAddr>().unwrap()));
        assert!(!net.contains("192.168.8.1".parse::<IpAddr>().unwrap()));
    }

    #[test]
    fn a_bare_address_matches_only_itself() {
        let net = entry("203.0.113.7");
        assert!(net.contains("203.0.113.7".parse::<IpAddr>().unwrap()));
        assert!(!net.contains("203.0.113.8".parse::<IpAddr>().unwrap()));
    }

    #[test]
    fn ipv4_mapped_peers_match_ipv4_rules() {
        // A dual-stack listener reports 127.0.0.1 as ::ffff:127.0.0.1; without
        // this, a loopback proxy rule would silently match nothing.
        let net = entry("127.0.0.0/8");
        assert!(net.contains("::ffff:127.0.0.1".parse::<IpAddr>().unwrap()));
    }

    #[test]
    fn ipv6_rules_do_not_match_ipv4_peers() {
        let net = entry("fd00::/8");
        assert!(net.contains("fd12::1".parse::<IpAddr>().unwrap()));
        assert!(!net.contains("10.0.0.1".parse::<IpAddr>().unwrap()));
    }

    #[test]
    fn malformed_entries_are_dropped_not_treated_as_wildcards() {
        assert!(parse_trusted_proxy_entry("").is_none());
        assert!(parse_trusted_proxy_entry("not-an-ip").is_none());
        assert!(parse_trusted_proxy_entry("10.0.0.0/33").is_none());
        assert!(parse_trusted_proxy_entry("::1/129").is_none());
    }
}
