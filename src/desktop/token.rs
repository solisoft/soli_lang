//! Gating a desktop app's local HTTP port.
//!
//! Binding loopback keeps the network out, but it does not keep the *machine*
//! out: every process running as the user can reach `127.0.0.1:<port>`, and the
//! port is discoverable. Without a gate, any local program — or any page in a
//! browser that guesses the port — could drive the app's API.
//!
//! The shell is launched with a one-shot token in the URL. Presenting it once
//! exchanges it for a session cookie; everything else is refused.
//!
//! Two deliberate bounds on the token's exposure, because a URL is not a
//! secret-friendly place: it lands in shell history, in `/proc/<pid>/cmdline`,
//! and in the browser's history and referrer. So the token is **single-use**
//! and **short-lived** — by the time it has leaked anywhere durable, it is
//! already spent.
//!
//! What this does not solve: cookies are not port-scoped, so another server on
//! `127.0.0.1` in the same browser profile can read or overwrite the session
//! cookie. Closing that needs loopback HTTPS with a per-launch certificate,
//! which means touching the user's trust store. What the gate *does* close is
//! the realistic threat — a non-browser local process driving the API.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Path that trades the launch token for a session cookie.
pub const EXCHANGE_PATH: &str = "/__desktop__/session";
/// Session cookie name.
pub const SESSION_COOKIE: &str = "soli_desktop";
/// How long the launch token stays usable.
const TOKEN_TTL: Duration = Duration::from_secs(60);

struct GateState {
    /// The one-shot launch token. `None` once spent or expired.
    launch_token: Option<String>,
    /// When the launch token stops being accepted.
    expires_at: Instant,
    /// Session value handed out in exchange for the token.
    session: String,
}

static GATE: OnceLock<Mutex<GateState>> = OnceLock::new();
/// Whether gating applies at all. Only a desktop boot arms it, so ordinary
/// `soli serve` is completely unaffected.
static ARMED: AtomicBool = AtomicBool::new(false);

/// What the gate decided about a request.
#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    /// Not a desktop app, or the caller already holds a valid session.
    Allow,
    /// A valid launch token: set this cookie and redirect to `/`.
    GrantSession(String),
    /// Refuse.
    Deny,
}

/// Arm the gate and return the URL to open.
///
/// Called once, by desktop boot, before serving starts.
pub fn arm(port: u16) -> String {
    let launch_token = random_hex(32);
    let session = random_hex(32);
    let url = format!(
        "http://127.0.0.1:{}{}?t={}",
        port, EXCHANGE_PATH, launch_token
    );

    let state = GATE.get_or_init(|| {
        Mutex::new(GateState {
            launch_token: None,
            expires_at: Instant::now(),
            session: String::new(),
        })
    });
    if let Ok(mut gate) = state.lock() {
        gate.launch_token = Some(launch_token);
        gate.expires_at = Instant::now() + TOKEN_TTL;
        gate.session = session;
    }
    ARMED.store(true, Ordering::SeqCst);
    url
}

/// Whether the gate is active.
#[inline]
pub fn is_armed() -> bool {
    ARMED.load(Ordering::SeqCst)
}

/// Decide whether a request may proceed.
///
/// `query` is the raw query string (without `?`), `cookie_header` the raw
/// `Cookie` header if present.
pub fn evaluate(path: &str, query: Option<&str>, cookie_header: Option<&str>) -> Decision {
    if !is_armed() {
        return Decision::Allow;
    }
    let Some(state) = GATE.get() else {
        return Decision::Allow;
    };
    let Ok(mut gate) = state.lock() else {
        // A poisoned lock must not become an open door.
        return Decision::Deny;
    };

    // An established session is the common case.
    if let Some(cookies) = cookie_header {
        if cookie_value(cookies, SESSION_COOKIE)
            .is_some_and(|value| constant_time_eq(value.as_bytes(), gate.session.as_bytes()))
        {
            return Decision::Allow;
        }
    }

    if path == EXCHANGE_PATH {
        let supplied = query.and_then(|q| query_value(q, "t"));
        let Some(supplied) = supplied else {
            return Decision::Deny;
        };
        if Instant::now() > gate.expires_at {
            gate.launch_token = None;
            return Decision::Deny;
        }
        // Take the token regardless of whether it matches: a wrong guess burns
        // the launch, which is the correct outcome for a single-use secret and
        // removes any brute-force margin.
        let Some(expected) = gate.launch_token.take() else {
            return Decision::Deny;
        };
        if constant_time_eq(supplied.as_bytes(), expected.as_bytes()) {
            return Decision::GrantSession(gate.session.clone());
        }
        return Decision::Deny;
    }

    Decision::Deny
}

/// `Set-Cookie` value for a granted session.
///
/// `HttpOnly` keeps it away from page scripts, `SameSite=Strict` stops another
/// origin driving the API through the user's browser, and `Path=/` scopes it to
/// the whole app. Deliberately no `Secure`: the app is served over plain
/// loopback HTTP, and `Secure` would stop the cookie being stored at all.
pub fn session_cookie_header(session: &str) -> String {
    format!(
        "{}={}; HttpOnly; SameSite=Strict; Path=/",
        SESSION_COOKIE, session
    )
}

fn random_hex(bytes: usize) -> String {
    use rand::RngCore;
    let mut buf = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut buf);
    buf.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Compare without leaking where the first difference is.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn cookie_value(header: &str, name: &str) -> Option<String> {
    header.split(';').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key.trim() == name).then(|| value.trim().to_string())
    })
}

fn query_value(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == name).then(|| value.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The gate is process-global, so these run under one lock and reset it
    /// between cases rather than fighting over shared state.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn reset() {
        ARMED.store(false, Ordering::SeqCst);
        if let Some(state) = GATE.get() {
            if let Ok(mut gate) = state.lock() {
                gate.launch_token = None;
                gate.session = String::new();
            }
        }
    }

    fn token_from(url: &str) -> String {
        url.split("?t=")
            .nth(1)
            .expect("url carries a token")
            .to_string()
    }

    #[test]
    fn unarmed_allows_everything() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        assert_eq!(evaluate("/anything", None, None), Decision::Allow);
    }

    #[test]
    fn armed_denies_without_a_token_or_session() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        arm(1234);
        assert_eq!(evaluate("/", None, None), Decision::Deny);
        assert_eq!(evaluate("/api/data", None, None), Decision::Deny);
        reset();
    }

    #[test]
    fn a_valid_token_grants_a_session_that_then_works() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        let url = arm(1234);
        let token = token_from(&url);

        let session = match evaluate(EXCHANGE_PATH, Some(&format!("t={}", token)), None) {
            Decision::GrantSession(s) => s,
            other => panic!("expected a session, got {:?}", other),
        };

        let cookie = format!("{}={}", SESSION_COOKIE, session);
        assert_eq!(evaluate("/", None, Some(&cookie)), Decision::Allow);
        assert_eq!(evaluate("/api/data", None, Some(&cookie)), Decision::Allow);
        reset();
    }

    #[test]
    fn the_token_is_single_use() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        let url = arm(1234);
        let token = token_from(&url);
        let query = format!("t={}", token);

        assert!(matches!(
            evaluate(EXCHANGE_PATH, Some(&query), None),
            Decision::GrantSession(_)
        ));
        // Replaying it — from history, from /proc, from a referrer — must fail.
        assert_eq!(evaluate(EXCHANGE_PATH, Some(&query), None), Decision::Deny);
        reset();
    }

    #[test]
    fn a_wrong_guess_burns_the_launch() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        let url = arm(1234);
        let real = token_from(&url);

        assert_eq!(
            evaluate(EXCHANGE_PATH, Some("t=deadbeef"), None),
            Decision::Deny
        );
        // Spending the token on a wrong guess is intentional: it leaves no
        // room to keep guessing.
        assert_eq!(
            evaluate(EXCHANGE_PATH, Some(&format!("t={}", real)), None),
            Decision::Deny
        );
        reset();
    }

    #[test]
    fn a_wrong_session_cookie_is_refused() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        arm(1234);
        let forged = format!("{}={}", SESSION_COOKIE, "0".repeat(64));
        assert_eq!(evaluate("/", None, Some(&forged)), Decision::Deny);
        reset();
    }

    #[test]
    fn expiry_refuses_a_late_token() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        let url = arm(1234);
        let token = token_from(&url);

        if let Some(state) = GATE.get() {
            let mut gate = state.lock().unwrap_or_else(|e| e.into_inner());
            gate.expires_at = Instant::now() - Duration::from_secs(1);
        }
        assert_eq!(
            evaluate(EXCHANGE_PATH, Some(&format!("t={}", token)), None),
            Decision::Deny
        );
        reset();
    }

    #[test]
    fn cookie_parsing_handles_neighbours_and_spacing() {
        assert_eq!(
            cookie_value("other=1; soli_desktop=abc; third=2", SESSION_COOKIE),
            Some("abc".to_string())
        );
        assert_eq!(cookie_value("nothing=here", SESSION_COOKIE), None);
        // A cookie whose name merely ends with ours must not match.
        assert_eq!(cookie_value("xsoli_desktop=abc", SESSION_COOKIE), None);
    }

    #[test]
    fn constant_time_eq_matches_normal_equality() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }
}
