//! SEC-028: force the `Secure` flag on session cookies.
//!
//! Without this gate, session cookies only get `Secure` when the operator has
//! opted into `enable_trust_proxy()` AND the proxy hop set
//! `X-Forwarded-Proto: https`. A site running on TLS without that header
//! signal — direct `--release` HTTPS bind, or any proxy that doesn't forward
//! the scheme — would emit cookies without `Secure`, letting a plaintext
//! same-domain probe (mixed-content asset, HTTP redirect race, attacker
//! subdomain) leak the session ID.
//!
//! `enable_force_secure_cookies()` / `SOLI_FORCE_SECURE_COOKIES=1` is the
//! operator's explicit "I'm always behind TLS" knob. When enabled, every
//! `Set-Cookie: session_id=...` carries `Secure` regardless of detected
//! scheme.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Once;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

static FORCE_SECURE_COOKIES: AtomicBool = AtomicBool::new(false);
static ENV_INIT: Once = Once::new();

/// Whether session cookies should always carry the `Secure` flag.
pub fn is_force_secure_cookies_enabled() -> bool {
    FORCE_SECURE_COOKIES.load(Ordering::Relaxed)
}

/// Parse a `SOLI_FORCE_SECURE_COOKIES` value. Truthy values (`1`, `true`,
/// `yes`, case-insensitive) flip the gate on. Anything else (including
/// missing or empty) leaves it off.
fn parse_force_secure_env(raw: Option<&str>) -> bool {
    match raw {
        Some(s) => matches!(s.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"),
        None => false,
    }
}

fn init_from_env() {
    ENV_INIT.call_once(|| {
        let raw = std::env::var("SOLI_FORCE_SECURE_COOKIES").ok();
        if parse_force_secure_env(raw.as_deref()) {
            FORCE_SECURE_COOKIES.store(true, Ordering::Relaxed);
        }
    });
}

pub fn register_secure_cookies_builtins(env: &mut Environment) {
    init_from_env();

    env.define(
        "enable_force_secure_cookies".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "enable_force_secure_cookies",
            Some(0),
            |_args| {
                FORCE_SECURE_COOKIES.store(true, Ordering::Relaxed);
                Ok(Value::Bool(true))
            },
        )),
    );

    env.define(
        "disable_force_secure_cookies".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "disable_force_secure_cookies",
            Some(0),
            |_args| {
                FORCE_SECURE_COOKIES.store(false, Ordering::Relaxed);
                Ok(Value::Bool(true))
            },
        )),
    );

    env.define(
        "force_secure_cookies_enabled".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "force_secure_cookies_enabled",
            Some(0),
            |_args| Ok(Value::Bool(is_force_secure_cookies_enabled())),
        )),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bundled into one `#[test]` because the flag is process-global.
    #[test]
    fn force_secure_cookies_toggle_round_trip() {
        FORCE_SECURE_COOKIES.store(false, Ordering::Relaxed);
        assert!(!is_force_secure_cookies_enabled());

        FORCE_SECURE_COOKIES.store(true, Ordering::Relaxed);
        assert!(is_force_secure_cookies_enabled());

        FORCE_SECURE_COOKIES.store(false, Ordering::Relaxed);
        assert!(!is_force_secure_cookies_enabled());
    }

    #[test]
    fn env_parser_recognizes_truthy_and_rejects_other() {
        for truthy in ["1", "true", "True", "TRUE", "yes", "YES", " yes ", "True\n"] {
            assert!(
                parse_force_secure_env(Some(truthy)),
                "expected truthy for {:?}",
                truthy
            );
        }
        for falsy in ["", " ", "0", "false", "no", "off", "maybe"] {
            assert!(
                !parse_force_secure_env(Some(falsy)),
                "expected falsy for {:?}",
                falsy
            );
        }
        assert!(!parse_force_secure_env(None));
    }
}
