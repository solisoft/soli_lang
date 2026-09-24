//! The gate in front of the built-in operator pages: `/__soli/jobs` and
//! `/__soli/errors`.
//!
//! Open in `--dev` to a loopback peer on a local host name. Everyone else —
//! a LAN peer in `--dev`, and every request in production — needs credentials:
//! HTTP Basic from `SOLI_<PAGE>_USER` + `SOLI_<PAGE>_PASSWORD`, or a `Bearer`
//! token from `SOLI_<PAGE>_TOKEN`. The shared `SOLI_ADMIN_USER` /
//! `SOLI_ADMIN_PASSWORD` / `SOLI_ADMIN_TOKEN` unlock every page, so an operator
//! configures one set instead of one per page. With nothing configured the page
//! answers 404, so production does not advertise that it exists.

use base64::Engine;
use hyper::{header::HeaderMap, Response, StatusCode};

use crate::interpreter::builtins::crypto::do_secure_compare;

use super::{full, Bytes, ResponseBody};

#[derive(Debug, PartialEq, Eq)]
pub(super) enum DashAuth {
    Allow,
    NeedAuth,
    Hidden,
}

/// What one page accepts: every Basic pair and every token that is configured.
#[derive(Default)]
pub(super) struct Credentials {
    pub basic: Vec<(String, String)>,
    pub tokens: Vec<String>,
}

fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|s| !s.is_empty())
}

/// `SOLI_<prefix>_*` plus the shared `SOLI_ADMIN_*`.
fn configured(prefix: &str) -> Credentials {
    let mut creds = Credentials::default();
    for scope in [prefix, "ADMIN"] {
        if let (Some(user), Some(password)) = (
            env_nonempty(&format!("SOLI_{scope}_USER")),
            env_nonempty(&format!("SOLI_{scope}_PASSWORD")),
        ) {
            creds.basic.push((user, password));
        }
        if let Some(token) = env_nonempty(&format!("SOLI_{scope}_TOKEN")) {
            creds.tokens.push(token);
        }
    }
    creds
}

fn parse_basic(headers: &HeaderMap) -> Option<(String, String)> {
    let raw = headers.get(hyper::header::AUTHORIZATION)?.to_str().ok()?;
    let b64 = raw.strip_prefix("Basic ")?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .ok()?;
    let decoded = String::from_utf8(bytes).ok()?;
    let (user, password) = decoded.split_once(':')?;
    Some((user.to_string(), password.to_string()))
}

fn parse_bearer(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(hyper::header::AUTHORIZATION)?.to_str().ok()?;
    raw.strip_prefix("Bearer ").map(|s| s.trim().to_string())
}

/// Decide a request to the page whose variables are `SOLI_<prefix>_*`.
pub(super) fn authorize(
    headers: &HeaderMap,
    dev_mode: bool,
    peer_ip: std::net::IpAddr,
    prefix: &str,
) -> DashAuth {
    // `--dev` opens the page without credentials only to the machine it runs
    // on, under a name that cannot be DNS-rebound — the same rule as the rest
    // of the dev bank. `--dev` binds 0.0.0.0, so "open in dev" used to mean
    // anyone on the LAN could read job payloads and retry or cancel them.
    // Anyone else falls through to the credential check below, exactly as in
    // production.
    let dev_local = dev_mode
        && super::dev_routes::is_trusted_dev_peer(peer_ip)
        && super::dev_routes::is_local_dev_host_header(headers);
    authorize_with(headers, dev_local, &configured(prefix))
}

pub(super) fn authorize_with(
    headers: &HeaderMap,
    dev_local: bool,
    creds: &Credentials,
) -> DashAuth {
    if dev_local {
        return DashAuth::Allow;
    }
    if creds.basic.is_empty() && creds.tokens.is_empty() {
        return DashAuth::Hidden;
    }
    if let Some((got_user, got_pass)) = parse_basic(headers) {
        // Every pair is compared, so the time taken does not say which one
        // matched.
        let mut ok = false;
        for (want_user, want_pass) in &creds.basic {
            ok |= do_secure_compare(want_user, &got_user) & do_secure_compare(want_pass, &got_pass);
        }
        if ok {
            return DashAuth::Allow;
        }
    }
    if let Some(got) = parse_bearer(headers) {
        let mut ok = false;
        for want in &creds.tokens {
            ok |= do_secure_compare(want, &got);
        }
        if ok {
            return DashAuth::Allow;
        }
    }
    DashAuth::NeedAuth
}

pub(super) fn unauthorized(realm: &str) -> Response<ResponseBody> {
    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header("WWW-Authenticate", format!("Basic realm=\"{realm}\""))
        .header("Content-Type", "text/plain; charset=utf-8")
        .body(full(Bytes::from("Unauthorized")))
        .unwrap()
}

pub(super) fn hidden_not_found() -> Response<ResponseBody> {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header("Content-Type", "text/plain; charset=utf-8")
        .body(full(Bytes::from("Not Found")))
        .unwrap()
}

/// Turn a decision into the response for a refused request, or `None` to go on.
pub(super) fn refusal(decision: DashAuth, realm: &str) -> Option<Response<ResponseBody>> {
    match decision {
        DashAuth::Allow => None,
        DashAuth::NeedAuth => Some(unauthorized(realm)),
        DashAuth::Hidden => Some(hidden_not_found()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers_with(value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(hyper::header::AUTHORIZATION, value.parse().expect("header"));
        h
    }

    fn basic(user: &str, password: &str) -> Credentials {
        Credentials {
            basic: vec![(user.into(), password.into())],
            tokens: vec![],
        }
    }

    fn token(value: &str) -> Credentials {
        Credentials {
            basic: vec![],
            tokens: vec![value.into()],
        }
    }

    #[test]
    fn prod_hidden_without_credentials() {
        let none = Credentials::default();
        assert_eq!(
            authorize_with(&HeaderMap::new(), false, &none),
            DashAuth::Hidden
        );
        assert_eq!(
            authorize_with(&HeaderMap::new(), true, &none),
            DashAuth::Allow
        );
    }

    /// `--dev` binds 0.0.0.0: a LAN peer (or a DNS-rebound page) must not get
    /// the credential-free page.
    #[test]
    fn dev_mode_opens_the_page_only_to_a_local_request() {
        let loopback: std::net::IpAddr = "127.0.0.1".parse().unwrap();
        let lan: std::net::IpAddr = "192.168.1.30".parse().unwrap();
        let mut local = HeaderMap::new();
        local.insert(hyper::header::HOST, "localhost:5011".parse().unwrap());
        let mut rebound = HeaderMap::new();
        rebound.insert(hyper::header::HOST, "rebind.attacker.test".parse().unwrap());

        let dev_local = |headers: &HeaderMap, ip| {
            super::super::dev_routes::is_trusted_dev_peer(ip)
                && super::super::dev_routes::is_local_dev_host_header(headers)
        };
        assert!(dev_local(&local, loopback));
        assert!(!dev_local(&local, lan));
        assert!(!dev_local(&rebound, loopback));

        // Not local and nothing configured: hidden, as in production.
        assert_eq!(
            authorize_with(&local, dev_local(&local, lan), &Credentials::default()),
            DashAuth::Hidden
        );
    }

    #[test]
    fn prod_basic_auth() {
        let creds = basic("ops", "s3cret");
        let ok = base64::engine::general_purpose::STANDARD.encode("ops:s3cret");
        let bad = base64::engine::general_purpose::STANDARD.encode("ops:wrong");
        assert_eq!(
            authorize_with(&headers_with(&format!("Basic {ok}")), false, &creds),
            DashAuth::Allow
        );
        assert_eq!(
            authorize_with(&headers_with(&format!("Basic {bad}")), false, &creds),
            DashAuth::NeedAuth
        );
        assert_eq!(
            authorize_with(&HeaderMap::new(), false, &creds),
            DashAuth::NeedAuth
        );
    }

    #[test]
    fn prod_bearer_token() {
        let creds = token("tok-xyz");
        assert_eq!(
            authorize_with(&headers_with("Bearer tok-xyz"), false, &creds),
            DashAuth::Allow
        );
        assert_eq!(
            authorize_with(&headers_with("Bearer nope"), false, &creds),
            DashAuth::NeedAuth
        );
    }

    /// A page-specific pair and the shared admin pair both open the page.
    #[test]
    fn any_configured_pair_opens_the_page() {
        let creds = Credentials {
            basic: vec![
                ("jobs".into(), "one".into()),
                ("admin".into(), "two".into()),
            ],
            tokens: vec![],
        };
        for pair in ["jobs:one", "admin:two"] {
            let encoded = base64::engine::general_purpose::STANDARD.encode(pair);
            assert_eq!(
                authorize_with(&headers_with(&format!("Basic {encoded}")), false, &creds),
                DashAuth::Allow,
                "{pair}"
            );
        }
        let crossed = base64::engine::general_purpose::STANDARD.encode("jobs:two");
        assert_eq!(
            authorize_with(&headers_with(&format!("Basic {crossed}")), false, &creds),
            DashAuth::NeedAuth
        );
    }

    #[test]
    fn configured_reads_page_and_admin_variables() {
        // Unique names: tests run in parallel and share the environment.
        unsafe {
            std::env::set_var("SOLI_AUTHTESTPAGE_TOKEN", "page-token");
            std::env::set_var("SOLI_AUTHTESTPAGE_USER", "u");
        }
        let creds = configured("AUTHTESTPAGE");
        assert!(creds.tokens.contains(&"page-token".to_string()));
        // A user without a password is not a pair.
        assert!(!creds.basic.iter().any(|(u, _)| u == "u"));
        unsafe {
            std::env::remove_var("SOLI_AUTHTESTPAGE_TOKEN");
            std::env::remove_var("SOLI_AUTHTESTPAGE_USER");
        }
    }
}
