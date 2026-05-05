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

/// Whether the server should honor `X-Forwarded-Proto` / `X-Forwarded-Host`
/// from incoming requests.
pub fn is_trust_proxy_enabled() -> bool {
    TRUST_PROXY_ENABLED.load(Ordering::Relaxed)
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
