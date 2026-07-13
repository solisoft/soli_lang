//! Pure Origin / Referer / Host authority normalization, shared by the CSRF
//! Origin gate (`check_csrf_origin`) and the WebSocket CSWSH gate
//! (`websocket_origin_allowed`) so both surfaces reject under identical
//! same-origin rules. No I/O or request state — just string canonicalization.

/// Parse an `Origin`/`Referer` value into a normalized `authority`
/// (host[:port], default port dropped), or `None` when it isn't a
/// well-formed http(s) URL.
pub(crate) fn origin_authority(origin: &str) -> Option<String> {
    let origin = origin.trim();
    let (scheme, rest) = origin
        .strip_prefix("http://")
        .map(|rest| ("http", rest))
        .or_else(|| origin.strip_prefix("https://").map(|rest| ("https", rest)))?;
    let authority = rest.split('/').next().unwrap_or("");
    if authority.is_empty() {
        return None;
    }

    Some(normalize_origin_authority(authority, scheme))
}

/// Normalize an authority for a known scheme, dropping the scheme's default
/// port (`:80` for http, `:443` for https).
fn normalize_origin_authority(authority: &str, scheme: &str) -> String {
    let authority = normalize_authority(authority);
    match (scheme, authority.as_str()) {
        ("http", value) if value.ends_with(":80") => value.trim_end_matches(":80").to_string(),
        ("https", value) if value.ends_with(":443") => value.trim_end_matches(":443").to_string(),
        _ => authority,
    }
}

/// Normalize a request authority (`Host`/`X-Forwarded-Host`), dropping both
/// default ports (the scheme isn't known at this point).
pub(crate) fn normalize_request_authority(authority: &str) -> String {
    let authority = normalize_authority(authority);
    if authority.ends_with(":80") {
        return authority.trim_end_matches(":80").to_string();
    }
    if authority.ends_with(":443") {
        return authority.trim_end_matches(":443").to_string();
    }
    authority
}

fn normalize_authority(authority: &str) -> String {
    authority.trim().trim_end_matches('.').to_ascii_lowercase()
}

/// SEC-044: pick the first comma-separated token from a forwarded-header
/// value (`X-Forwarded-Proto`, `X-Forwarded-Host`). Some proxies append
/// instead of overwriting, so a request with `X-Forwarded-Host: real,
/// attacker` would otherwise reach our scheme/host code as the whole
/// concatenated string. The leftmost entry — written by the *outermost*
/// trusted proxy in a chain — is the canonical value once `trust_proxy`
/// is enabled. Empty input or empty first token returns `""`, which lets
/// callers fall back to defaults without an extra branch.
pub(crate) fn first_forwarded_token(value: &str) -> &str {
    value.split(',').next().unwrap_or("").trim()
}
