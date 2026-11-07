//! `/__soli/test/session`: how `soli test` writes into a test server's session.
//!
//! A spec runs in the `soli test` process; the application it calls runs in a
//! `soli serve` child. `with_session`, `as_user` and `sign_in` used to write
//! into the session store of the *runner* — which, under the default in-memory
//! driver, is a different store from the one the server reads. The cookie they
//! set named a session the server had never heard of, so the next request ran
//! as a guest. (The cookie driver worked by accident: its payload travels in
//! the cookie itself.)
//!
//! The write now happens where the session lives. The runner POSTs the fields
//! here, with the jar's cookie, and installs the cookie value the server
//! answers. The same call hands back the session's CSRF token, which is what
//! lets the spec client post a form under `SOLI_CSRF_TOKENS=require` the way a
//! browser that rendered the form would.
//!
//! Only a test server answers: the process must have been started by
//! `soli test` (`SOLI_INTERNAL_TEST_RUNNER=<uuid-v4>`, checked once at boot by
//! `main.rs`), the request must come from loopback, and it must present that
//! same per-run token as `X-Soli-Test-Token`. Anything else is the reserved
//! namespace's 404. The path lives under `/__soli/`, which application routing
//! never sees and which neither CSRF barrier applies to.

use std::net::SocketAddr;

use bytes::Bytes;
use http_body_util::{BodyExt, Limited};
use hyper::body::Incoming;
use hyper::{Request, Response, StatusCode};
use serde_json::Value as JsonValue;

use super::{full, ResponseBody};
use crate::interpreter::builtins::session::{self, CSRF_SESSION_KEY};

pub(crate) const PATH: &str = "/__soli/test/session";
pub(crate) const TOKEN_HEADER: &str = "x-soli-test-token";
const MAX_BODY: usize = 256 * 1024;

/// What the runner asked for, parsed from the JSON body.
struct SessionWrite {
    /// The raw `session_id` cookie value the runner's jar holds, if any.
    cookie: Option<String>,
    /// Fields to store, in order.
    fields: Vec<(String, JsonValue)>,
    /// Also answer the session's CSRF token, creating it when missing.
    csrf: bool,
}

/// The session write, or the request back when the path is not ours.
#[allow(clippy::result_large_err)]
pub(super) async fn dispatch(
    req: Request<Incoming>,
    method: &str,
    path: &str,
    peer_addr: SocketAddr,
) -> Result<Response<ResponseBody>, Request<Incoming>> {
    if path != PATH {
        return Err(req);
    }
    let provided = req
        .headers()
        .get(TOKEN_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let expected = std::env::var("SOLI_INTERNAL_TEST_RUNNER").ok();
    if method != "POST"
        || !authorized(
            crate::interpreter::builtins::test_server::is_test_runner_process(),
            peer_addr,
            expected.as_deref(),
            &provided,
        )
    {
        return Ok(text(StatusCode::NOT_FOUND, "Not Found"));
    }
    let cookie_header = req
        .headers()
        .get(hyper::header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let body = match Limited::new(req.into_body(), MAX_BODY).collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(_) => return Ok(text(StatusCode::PAYLOAD_TOO_LARGE, "body too large")),
    };
    let write = match parse(&body, cookie_header.as_deref()) {
        Ok(write) => write,
        Err(message) => return Ok(text(StatusCode::BAD_REQUEST, &message)),
    };
    // The store may be network-backed (SoliDB, SoliKV) and its calls block;
    // the cookie driver keeps per-request state in a thread-local. A thread
    // of its own gives both what a worker thread gives them.
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(apply(write));
    });
    match rx.await {
        Ok(reply) => Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .header("Cache-Control", "no-store")
            .body(full(Bytes::from(reply.to_string())))
            .unwrap_or_else(|_| text(StatusCode::INTERNAL_SERVER_ERROR, "response"))),
        Err(_) => Ok(text(
            StatusCode::INTERNAL_SERVER_ERROR,
            "session write did not complete",
        )),
    }
}

/// Test runner process, loopback peer, and the per-run token presented —
/// compared in constant time.
fn authorized(
    test_runner: bool,
    peer_addr: SocketAddr,
    expected: Option<&str>,
    provided: &str,
) -> bool {
    let Some(expected) = expected.map(str::trim).filter(|t| !t.is_empty()) else {
        return false;
    };
    test_runner
        && peer_addr.ip().is_loopback()
        && !provided.is_empty()
        && crate::interpreter::builtins::crypto::do_secure_compare(expected, provided.trim())
}

fn parse(body: &[u8], cookie_header: Option<&str>) -> Result<SessionWrite, String> {
    let input: JsonValue = if body.is_empty() {
        JsonValue::Object(Default::default())
    } else {
        serde_json::from_slice(body).map_err(|e| format!("expected a JSON body: {e}"))?
    };
    let fields = match input.get("set") {
        None | Some(JsonValue::Null) => Vec::new(),
        Some(JsonValue::Object(map)) => map.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        Some(_) => return Err("`set` must be an object".to_string()),
    };
    Ok(SessionWrite {
        cookie: session::extract_session_id_from_cookie(cookie_header).filter(|c| !c.is_empty()),
        fields,
        csrf: input
            .get("csrf")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false),
    })
}

/// Open (or create) the session the cookie names, store the fields, and
/// answer the cookie value that now names it plus, on request, its CSRF token.
fn apply(write: SessionWrite) -> JsonValue {
    crate::interpreter::builtins::session_cookie::clear_request_state();
    let store = session::get_current_store();
    let cookie_driver = store.driver_name() == "cookie";
    let session_id = match write.cookie.as_deref() {
        Some(raw) if cookie_driver => store.get_or_create(raw),
        Some(raw) if session::is_valid_session_id(raw) && store.exists(raw) => raw.to_string(),
        // An id-based cookie the store does not know: start a fresh session
        // rather than resurrect an unknown id (SEC-053's posture).
        _ => store.create_session(),
    };
    for (key, value) in write.fields {
        store.set(&session_id, &key, value);
    }
    let csrf_token = if write.csrf {
        let existing = store
            .get(&session_id, CSRF_SESSION_KEY)
            .and_then(|v| v.as_str().map(str::to_string));
        Some(existing.unwrap_or_else(|| {
            let token = uuid::Uuid::new_v4().simple().to_string();
            store.set(
                &session_id,
                CSRF_SESSION_KEY,
                JsonValue::String(token.clone()),
            );
            token
        }))
    } else {
        None
    };
    let cookie = if cookie_driver {
        // Unchanged session (a token read that found one): the jar's blob is
        // still the right one.
        store
            .outgoing_cookie_value()
            .or(write.cookie)
            .unwrap_or_default()
    } else {
        session_id
    };
    crate::interpreter::builtins::session_cookie::clear_request_state();
    serde_json::json!({ "cookie": cookie, "csrf_token": csrf_token })
}

fn text(status: StatusCode, body: &str) -> Response<ResponseBody> {
    let mut response = Response::new(full(Bytes::from(body.to_string())));
    *response.status_mut() = status;
    response.headers_mut().insert(
        hyper::header::CONTENT_TYPE,
        hyper::header::HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loopback() -> SocketAddr {
        "127.0.0.1:5000".parse().unwrap()
    }

    #[test]
    fn refused_outside_a_test_runner_process() {
        assert!(!authorized(false, loopback(), Some("tok"), "tok"));
    }

    #[test]
    fn refused_from_a_remote_peer() {
        let remote: SocketAddr = "203.0.113.9:5000".parse().unwrap();
        assert!(!authorized(true, remote, Some("tok"), "tok"));
    }

    #[test]
    fn refused_without_or_with_the_wrong_token() {
        assert!(!authorized(true, loopback(), None, "tok"));
        assert!(!authorized(true, loopback(), Some(""), ""));
        assert!(!authorized(true, loopback(), Some("tok"), ""));
        assert!(!authorized(true, loopback(), Some("tok"), "nope"));
    }

    #[test]
    fn accepted_with_runner_loopback_and_token() {
        assert!(authorized(true, loopback(), Some("tok"), "tok"));
    }

    #[test]
    fn parse_reads_fields_cookie_and_csrf_flag() {
        let body = br#"{"set": {"user_id": "abc", "role": "admin"}, "csrf": true}"#;
        let write = parse(body, Some("theme=dark; session_id=xyz")).unwrap();
        assert_eq!(write.cookie.as_deref(), Some("xyz"));
        assert!(write.csrf);
        assert_eq!(write.fields.len(), 2);
    }

    #[test]
    fn parse_rejects_a_non_object_set() {
        assert!(parse(br#"{"set": [1]}"#, None).is_err());
    }

    #[test]
    fn apply_writes_into_the_store_and_answers_the_session_cookie() {
        let _guard = session::SESSION_STORE_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let reply = apply(SessionWrite {
            cookie: None,
            fields: vec![("user_id".to_string(), serde_json::json!("k-1"))],
            csrf: true,
        });
        let cookie = reply["cookie"].as_str().unwrap().to_string();
        let token = reply["csrf_token"].as_str().unwrap().to_string();
        let store = session::get_current_store();
        assert_eq!(
            store.get(&cookie, "user_id"),
            Some(serde_json::json!("k-1"))
        );
        assert_eq!(
            store.get(&cookie, CSRF_SESSION_KEY),
            Some(serde_json::json!(token))
        );

        // A second call naming that session reuses it and its token.
        let again = apply(SessionWrite {
            cookie: Some(cookie.clone()),
            fields: vec![("role".to_string(), serde_json::json!("admin"))],
            csrf: true,
        });
        assert_eq!(again["cookie"].as_str(), Some(cookie.as_str()));
        assert_eq!(again["csrf_token"].as_str(), Some(token.as_str()));
        assert_eq!(store.get(&cookie, "role"), Some(serde_json::json!("admin")));
    }
}
