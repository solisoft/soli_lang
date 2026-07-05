//! Encrypted client-side cookie session store (Rails-style "cookie store").
//!
//! Unlike the other drivers, nothing is persisted server-side: the whole
//! session payload travels in the `session_id` cookie as an encrypted,
//! authenticated blob. That makes sessions survive server restarts and work
//! across hosts with zero infrastructure — at the cost of a ~4KB size ceiling
//! and no server-side revocation (a stolen cookie stays valid until it
//! expires; `session_destroy` can only overwrite the client's copy).
//!
//! Wire format: `v1.<base64url(nonce[12] ‖ AES-256-GCM(payload))>` where the
//! payload is `{"id": uuid, "iat": unix_secs, "data": {...}}`. The AES key is
//! HKDF-SHA256-derived from `SOLI_SESSION_SECRET` (domain-separated from the
//! model-field key, so rotating one never breaks the other). GCM's auth tag
//! rejects any client-side tampering; a blob that fails to open or whose
//! `iat + ttl` has passed is silently replaced by a fresh empty session,
//! mirroring how the ID-based drivers treat unknown/expired session IDs.
//!
//! Per-request state lives in a thread local (the same worker-thread model as
//! `CURRENT_SESSION_ID`): `get_or_create` installs it from the incoming
//! cookie, the session builtins mutate it, and `outgoing_cookie_value` seals
//! it back into a cookie value when — and only when — something changed. The
//! serve loop clears the thread local at the top of every request so state
//! can never leak across requests on a reused worker thread.

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use sha2::Sha256;
use uuid::Uuid;

use super::crypto::{aes_decrypt_bytes, aes_encrypt_bytes};
use super::session::SessionStore;

/// Version prefix on every sealed value — leaves room to rotate the wire
/// format (or KDF) without breaking existing cookies.
const FORMAT_PREFIX: &str = "v1.";

/// Minimum secret length. A short secret caps the derived key's entropy no
/// matter how strong the KDF is, so refuse outright rather than degrade.
const MIN_SECRET_LEN: usize = 32;

/// Ceiling for the sealed cookie *value*. Browsers enforce ~4096 bytes for
/// the whole `Set-Cookie` line; leaving ~100 bytes for the name and
/// attributes keeps us under it. An oversized session is refused at seal
/// time (the old cookie stays in place) with a loud log line, because a
/// silently-dropped `Set-Cookie` at the browser is far harder to debug.
const MAX_SEALED_LEN: usize = 4000;

/// The decrypted session payload as it travels inside the cookie.
#[derive(Serialize, Deserialize)]
struct SealedPayload {
    id: String,
    iat: u64,
    data: HashMap<String, JsonValue>,
}

/// Per-request session state for the cookie driver.
struct CookieSessionState {
    id: String,
    data: HashMap<String, JsonValue>,
    /// A write happened this request — the outgoing cookie must be re-sealed.
    dirty: bool,
    /// The incoming cookie was absent, invalid, or expired and a fresh
    /// session took its place — emit a cookie even without a write so the
    /// client stops replaying the dead blob.
    replaced: bool,
}

thread_local! {
    static STATE: RefCell<Option<CookieSessionState>> = const { RefCell::new(None) };
}

/// Drop any cookie-session state left over from a previous request on this
/// worker thread. Called at the top of every request (see `serve::mod`).
pub fn clear_request_state() {
    STATE.with(|s| *s.borrow_mut() = None);
}

/// Quick shape check for an incoming cookie value before it reaches the
/// store — the cookie-driver counterpart of SEC-053's UUID check. Bounds the
/// length (memory amplification) and alphabet (log injection / garbage) of
/// attacker-controlled input.
pub fn is_plausible_sealed_value(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_SEALED_LEN
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn fresh_state(replaced: bool) -> CookieSessionState {
    CookieSessionState {
        id: Uuid::new_v4().to_string(),
        data: HashMap::new(),
        dirty: false,
        replaced,
    }
}

pub struct CookieSessionStore {
    key: [u8; 32],
    ttl: u64,
}

impl CookieSessionStore {
    /// Build the store, deriving the AES key from the operator secret.
    /// Rejects short secrets: sessions carry authenticated identity, so a
    /// guessable key equals account takeover for every user.
    pub fn new(secret: &str, ttl: u64) -> Result<Self, String> {
        if secret.len() < MIN_SECRET_LEN {
            return Err(format!(
                "cookie session driver requires SOLI_SESSION_SECRET of at least {} characters (got {})",
                MIN_SECRET_LEN,
                secret.len()
            ));
        }
        let hk = Hkdf::<Sha256>::new(None, secret.as_bytes());
        let mut key = [0u8; 32];
        hk.expand(b"soli.session.cookie.v1", &mut key)
            .map_err(|e| format!("session key derivation failed: {}", e))?;
        Ok(Self { key, ttl })
    }

    fn seal(&self, state: &CookieSessionState) -> Result<String, String> {
        let payload = SealedPayload {
            id: state.id.clone(),
            iat: now_unix_secs(),
            data: state.data.clone(),
        };
        let plaintext =
            serde_json::to_vec(&payload).map_err(|e| format!("session serialize failed: {}", e))?;
        let sealed = format!(
            "{}{}",
            FORMAT_PREFIX,
            URL_SAFE_NO_PAD.encode(aes_encrypt_bytes(&plaintext, &self.key)?)
        );
        if sealed.len() > MAX_SEALED_LEN {
            return Err(format!(
                "session exceeds the ~4KB cookie limit ({} bytes sealed); \
                 the session was NOT saved. Store less in the session or switch \
                 to a server-side driver (disk/solidb/solikv).",
                sealed.len()
            ));
        }
        Ok(sealed)
    }

    fn open(&self, sealed: &str) -> Result<CookieSessionState, String> {
        let encoded = sealed
            .strip_prefix(FORMAT_PREFIX)
            .ok_or_else(|| "unknown session cookie format".to_string())?;
        let raw = URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|e| format!("invalid base64: {}", e))?;
        let plaintext = aes_decrypt_bytes(&raw, &self.key)?;
        let payload: SealedPayload = serde_json::from_slice(&plaintext)
            .map_err(|e| format!("invalid session payload: {}", e))?;
        if now_unix_secs() > payload.iat.saturating_add(self.ttl) {
            return Err("session expired".to_string());
        }
        Ok(CookieSessionState {
            id: payload.id,
            data: payload.data,
            dirty: false,
            replaced: false,
        })
    }

    /// Run `f` over the live state, installing a fresh session first if none
    /// exists (a session builtin can run before the serve loop resolved one —
    /// e.g. in tests or `write_session_fields`).
    fn with_state<R>(&self, f: impl FnOnce(&mut CookieSessionState) -> R) -> R {
        STATE.with(|s| {
            let mut slot = s.borrow_mut();
            let state = slot.get_or_insert_with(|| fresh_state(true));
            f(state)
        })
    }

    /// Every mutation invalidates the static-page response cache for this
    /// request: the outgoing Set-Cookie differs after any write, unlike the
    /// ID drivers where the cookie only changes when the ID does.
    fn mark_mutated(state: &mut CookieSessionState) {
        state.dirty = true;
        crate::template::response_cache::mark_response_dirty();
    }
}

impl SessionStore for CookieSessionStore {
    fn get_or_create(&self, sealed: &str) -> String {
        let state = match self.open(sealed) {
            Ok(state) => state,
            Err(_) => fresh_state(true),
        };
        let id = state.id.clone();
        STATE.with(|s| *s.borrow_mut() = Some(state));
        id
    }

    fn create_session(&self) -> String {
        let state = fresh_state(true);
        let id = state.id.clone();
        STATE.with(|s| *s.borrow_mut() = Some(state));
        id
    }

    fn get(&self, _session_id: &str, key: &str) -> Option<JsonValue> {
        STATE.with(|s| {
            s.borrow()
                .as_ref()
                .and_then(|state| state.data.get(key).cloned())
        })
    }

    fn set(&self, _session_id: &str, key: &str, value: JsonValue) {
        self.with_state(|state| {
            state.data.insert(key.to_string(), value);
            Self::mark_mutated(state);
        });
    }

    fn delete(&self, _session_id: &str, key: &str) -> Option<JsonValue> {
        self.with_state(|state| {
            let removed = state.data.remove(key);
            if removed.is_some() {
                Self::mark_mutated(state);
            }
            removed
        })
    }

    fn destroy(&self, _session_id: &str) {
        // No server-side record to delete; the best we can do is overwrite
        // the client's cookie with a fresh empty session under a new ID.
        STATE.with(|s| {
            let mut state = fresh_state(false);
            Self::mark_mutated(&mut state);
            *s.borrow_mut() = Some(state);
        });
    }

    fn regenerate(&self, _old_id: &str) -> String {
        self.with_state(|state| {
            state.id = Uuid::new_v4().to_string();
            Self::mark_mutated(state);
            state.id.clone()
        })
    }

    fn cleanup(&self) {
        // Nothing stored server-side; expiry is enforced at open time.
    }

    fn driver_name(&self) -> &'static str {
        "cookie"
    }

    fn outgoing_cookie_value(&self) -> Option<String> {
        STATE.with(|s| {
            let slot = s.borrow();
            let state = slot.as_ref()?;
            if !(state.dirty || state.replaced) {
                return None;
            }
            match self.seal(state) {
                Ok(sealed) => Some(sealed),
                Err(e) => {
                    eprintln!(
                        "{} [session] cookie driver: {}",
                        crate::serve::log_timestamp(),
                        e
                    );
                    None
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "0123456789abcdef0123456789abcdef";

    fn store() -> CookieSessionStore {
        CookieSessionStore::new(SECRET, 3600).unwrap()
    }

    #[test]
    fn rejects_short_secret() {
        // No unwrap_err(): the store deliberately doesn't derive Debug so key
        // material can't end up in logs via `{:?}`.
        let err = match CookieSessionStore::new("too-short", 3600) {
            Err(e) => e,
            Ok(_) => panic!("short secret must be rejected"),
        };
        assert!(err.contains("at least 32"), "unexpected error: {}", err);
    }

    #[test]
    fn seal_open_round_trip_preserves_id_and_data() {
        clear_request_state();
        let store = store();
        let id = store.create_session();
        store.set(&id, "user_id", serde_json::json!(42));
        store.set(&id, "theme", serde_json::json!("dark"));

        let sealed = store.outgoing_cookie_value().expect("dirty state seals");
        assert!(sealed.starts_with(FORMAT_PREFIX));
        assert!(is_plausible_sealed_value(&sealed));

        // Simulate the next request: clear thread state, open from cookie.
        clear_request_state();
        let reopened_id = store.get_or_create(&sealed);
        assert_eq!(reopened_id, id, "internal session ID must survive");
        assert_eq!(
            store.get(&reopened_id, "user_id"),
            Some(serde_json::json!(42))
        );
        assert_eq!(
            store.get(&reopened_id, "theme"),
            Some(serde_json::json!("dark"))
        );
        // Nothing written on this "request": no outgoing cookie needed.
        assert!(store.outgoing_cookie_value().is_none());
    }

    #[test]
    fn tampered_cookie_yields_fresh_session() {
        clear_request_state();
        let store = store();
        let id = store.create_session();
        store.set(&id, "user_id", serde_json::json!(1));
        let sealed = store.outgoing_cookie_value().unwrap();

        // Flip a character in the ciphertext body.
        let mut bytes = sealed.into_bytes();
        let mid = bytes.len() / 2;
        bytes[mid] = if bytes[mid] == b'A' { b'B' } else { b'A' };
        let tampered = String::from_utf8(bytes).unwrap();

        clear_request_state();
        let new_id = store.get_or_create(&tampered);
        assert_ne!(new_id, id, "tampered blob must not resolve to the session");
        assert!(store.get(&new_id, "user_id").is_none());
        // The replacement session must be re-emitted to evict the bad blob.
        assert!(store.outgoing_cookie_value().is_some());
    }

    #[test]
    fn wrong_key_yields_fresh_session() {
        clear_request_state();
        let store_a = store();
        let id = store_a.create_session();
        store_a.set(&id, "user_id", serde_json::json!(7));
        let sealed = store_a.outgoing_cookie_value().unwrap();

        clear_request_state();
        let store_b = CookieSessionStore::new("ffffffffffffffffffffffffffffffff", 3600).unwrap();
        let new_id = store_b.get_or_create(&sealed);
        assert_ne!(new_id, id);
        assert!(store_b.get(&new_id, "user_id").is_none());
    }

    #[test]
    fn expired_cookie_yields_fresh_session() {
        clear_request_state();
        let zero_ttl = CookieSessionStore::new(SECRET, 0).unwrap();
        let id = zero_ttl.create_session();
        zero_ttl.set(&id, "user_id", serde_json::json!(9));
        let sealed = zero_ttl.outgoing_cookie_value().unwrap();

        // iat + 0s TTL is already in the past by the next whole second; use
        // open() directly to avoid sleeping in the test.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        assert!(zero_ttl.open(&sealed).is_err(), "0s TTL must expire");
    }

    #[test]
    fn regenerate_rotates_id_and_keeps_data() {
        clear_request_state();
        let store = store();
        let id = store.create_session();
        store.set(&id, "user_id", serde_json::json!(3));
        let new_id = store.regenerate(&id);
        assert_ne!(new_id, id);
        assert_eq!(store.get(&new_id, "user_id"), Some(serde_json::json!(3)));
    }

    #[test]
    fn destroy_drops_data_and_emits_replacement() {
        clear_request_state();
        let store = store();
        let id = store.create_session();
        store.set(&id, "user_id", serde_json::json!(5));
        store.destroy(&id);
        assert!(store.get(&id, "user_id").is_none());
        // An overwriting (empty) cookie must go out.
        let sealed = store.outgoing_cookie_value().unwrap();
        clear_request_state();
        let reopened = store.get_or_create(&sealed);
        assert!(store.get(&reopened, "user_id").is_none());
    }

    #[test]
    fn oversized_session_refuses_to_seal() {
        clear_request_state();
        let store = store();
        let id = store.create_session();
        store.set(&id, "blob", serde_json::json!("x".repeat(MAX_SEALED_LEN)));
        assert!(
            store.outgoing_cookie_value().is_none(),
            "oversized payload must refuse to seal rather than emit a cookie the browser drops"
        );
    }

    #[test]
    fn plausibility_guard_bounds_length_and_alphabet() {
        assert!(is_plausible_sealed_value("v1.abc-DEF_123"));
        assert!(!is_plausible_sealed_value(""));
        assert!(!is_plausible_sealed_value("v1.abc;def"));
        assert!(!is_plausible_sealed_value("v1.abc def"));
        assert!(!is_plausible_sealed_value(&"a".repeat(MAX_SEALED_LEN + 1)));
    }
}
