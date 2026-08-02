//! Talking to `soli-proxy`'s admin API.
//!
//! Four calls, and the ordering between them is the deployment.
//!
//! Kept apart from the SSH side because they fail differently and the operator
//! needs to know which happened: a build that never reached the host is a retry,
//! a deploy the proxy refused is a look at the app's logs, and a health check
//! that never went green is a rollback.

use std::time::{Duration, Instant};

/// Where the proxy admin API is, and the key for it.
#[derive(Debug, Clone)]
pub struct Admin {
    /// `http://127.0.0.1:9090`, usually reached through the SSH tunnel the
    /// deploy already has.
    pub base_url: String,
    pub api_key: String,
}

#[derive(Debug)]
pub enum AdminError {
    /// The call never completed.
    Unreachable(String),
    /// The proxy answered, and said no.
    Refused { status: u16, body: String },
}

impl std::fmt::Display for AdminError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AdminError::Unreachable(why) => write!(f, "cannot reach the proxy admin API: {why}"),
            AdminError::Refused { status, body } => {
                write!(f, "the proxy refused ({status}): {body}")
            }
        }
    }
}

impl Admin {
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
        }
    }

    fn call(&self, method: &str, path: &str, body: Option<&str>) -> Result<String, AdminError> {
        let url = format!("{}{path}", self.base_url);
        let mut request = ureq::request(method, &url)
            .set("X-Api-Key", &self.api_key)
            .timeout(Duration::from_secs(120));
        if body.is_some() {
            request = request.set("Content-Type", "application/json");
        }

        let result = match body {
            Some(payload) => request.send_string(payload),
            None => request.call(),
        };
        match result {
            Ok(response) => Ok(response.into_string().unwrap_or_default()),
            Err(ureq::Error::Status(status, response)) => Err(AdminError::Refused {
                status,
                body: response.into_string().unwrap_or_default(),
            }),
            Err(e) => Err(AdminError::Unreachable(e.to_string())),
        }
    }

    /// Asks the proxy to start the app from whatever `sites/<app>` now points at.
    ///
    /// Blue/green happens on the proxy's side: it starts the new slot, gates on
    /// the health check, and only then flips. That is why the symlink is moved
    /// *before* this call and the alias only *after* it.
    pub fn deploy(&self, app: &str) -> Result<String, AdminError> {
        self.call("POST", &format!("/api/v1/apps/{app}/deploy"), None)
    }

    /// Points a domain at an app.
    ///
    /// Idempotent by design on the proxy's side — repointing an existing alias
    /// is the same call — which is what makes this the rollback primitive
    /// rather than a one-way door.
    pub fn set_alias(&self, app: &str, domain: &str) -> Result<String, AdminError> {
        let body = format!(r#"{{"domain":{}}}"#, json_string(domain));
        self.call("POST", &format!("/api/v1/apps/{app}/aliases"), Some(&body))
    }

    /// Stops an app cleanly.
    ///
    /// Not called by a deploy — a deploy replaces, it does not stop. This is
    /// for teardown (`soli env down`, removing an app) and it lives here so the
    /// mandatory-ordering note below sits with the call it constrains.
    #[allow(dead_code)]
    ///
    /// **Mandatory before removing an app's directory.** The proxy's discovery
    /// only does `apps.retain(...)`, so a directory removed underneath it leaves
    /// an orphan process still holding its ports — and the next deploy then
    /// fails to bind for a reason that points nowhere near the cause.
    pub fn stop(&self, app: &str) -> Result<String, AdminError> {
        self.call("POST", &format!("/api/v1/apps/{app}/stop"), None)
    }

    /// What the proxy believes it is running. For `one import`-style diffing
    /// and for a `soli cloud status` that has somewhere to compare against.
    #[allow(dead_code)]
    pub fn apps(&self) -> Result<String, AdminError> {
        self.call("GET", "/api/v1/apps", None)
    }
}

/// Waits for a URL to answer 200.
///
/// The gate between "the proxy accepted the deploy" and "the alias moves". A
/// deploy that is accepted and never becomes healthy is the case this exists
/// for: without the gate the alias would move first and real traffic would
/// arrive at a release that is still starting, or never will.
///
/// Fails fast on connection refused *after* a grace period, and never before:
/// a refused connection in the first seconds is a process that has not bound
/// yet, which is normal; the same refusal thirty seconds in is a dead app.
pub fn wait_healthy(url: &str, timeout: Duration) -> Result<Duration, String> {
    let started = Instant::now();
    let mut last = String::from("never answered");
    while started.elapsed() < timeout {
        match ureq::get(url).timeout(Duration::from_secs(5)).call() {
            Ok(response) if response.status() == 200 => return Ok(started.elapsed()),
            Ok(response) => last = format!("status {}", response.status()),
            Err(ureq::Error::Status(status, _)) => last = format!("status {status}"),
            Err(e) => last = e.to_string(),
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err(format!(
        "{url} did not answer 200 within {}s — last: {last}",
        timeout.as_secs()
    ))
}

/// Minimal JSON string escaping.
///
/// Written out rather than pulled in: the only value that ever goes through it
/// is a domain name, and a domain containing a quote is either an attack or a
/// typo — both of which should produce a rejected request rather than a
/// malformed body the proxy has to guess at.
fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trailing_slash_on_the_base_url_does_not_double_up() {
        // `http://host//api/v1/apps` mostly works, and then does not, on
        // whichever proxy normalises paths differently.
        for base in ["http://h:9090", "http://h:9090/", "http://h:9090///"] {
            assert_eq!(Admin::new(base, "k").base_url, "http://h:9090");
        }
    }

    #[test]
    fn a_domain_is_escaped_rather_than_interpolated() {
        // The one value that reaches a hand-built JSON body. A quote in it is
        // an attack or a typo, and either way the proxy should reject a
        // well-formed request rather than parse a broken one.
        assert_eq!(json_string("x.soli.app"), r#""x.soli.app""#);
        assert_eq!(json_string(r#"a"b"#), r#""a\"b""#);
        assert_eq!(json_string("a\\b"), r#""a\\b""#);
        assert_eq!(json_string("a\nb"), r#""a\nb""#);
        // Escaped, not dropped: a control character silently removed
        // would change the value the proxy stores from the one sent.
        assert_eq!(json_string("a\u{1}b"), "\"a\\u0001b\"");
    }

    #[test]
    fn unreachable_and_refused_are_different_errors() {
        // They call for different actions: a retry versus reading the app's
        // logs. One error type for both sends an operator to the wrong place.
        let unreachable = AdminError::Unreachable("connection refused".into());
        let refused = AdminError::Refused {
            status: 404,
            body: "no such app".into(),
        };
        assert!(unreachable.to_string().contains("cannot reach"));
        assert!(refused.to_string().contains("404"));
        assert!(refused.to_string().contains("no such app"));
    }

    #[test]
    fn health_reports_what_it_last_saw() {
        // "did not become healthy" is useless; "last: status 502" says the app
        // is up and broken, and "connection refused" says it never started.
        // A port nothing listens on, with a deadline short enough to be a test.
        let err = wait_healthy("http://127.0.0.1:1/up", Duration::from_millis(600)).unwrap_err();
        assert!(err.contains("did not answer 200"), "got {err}");
        assert!(err.contains("last:"), "got {err}");
    }
}
