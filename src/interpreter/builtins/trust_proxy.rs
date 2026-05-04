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

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

static TRUST_PROXY_ENABLED: AtomicBool = AtomicBool::new(false);

/// Whether the server should honor `X-Forwarded-Proto` / `X-Forwarded-Host`
/// from incoming requests.
pub fn is_trust_proxy_enabled() -> bool {
    TRUST_PROXY_ENABLED.load(Ordering::Relaxed)
}

pub fn register_trust_proxy_builtins(env: &mut Environment) {
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
}
