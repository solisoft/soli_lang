//! What counts as a secret, in one place.
//!
//! Three separate rules for "this key holds a secret" grew up independently:
//! the request-param list in `serve::error_pages`, the header list beside it,
//! and `interpreter::value::is_sensitive_field_name` for model serialisation.
//! The first two guard the request snapshot in an error log; the third guards
//! model output. Nothing guarded the `env:` line of that same error log, which
//! is where a handler's **local variables** are written by value — so a local
//! called `api_key` was redacted as a request param and printed in full three
//! lines below.
//!
//! This module owns the param-style rule so the snapshot and the environment
//! dump cannot disagree. The header list stays where it is: header names are
//! matched exactly, not by substring, and that is a different rule rather than
//! a drifted copy of this one.

/// Substrings that signal a secret-bearing key. Matched case-insensitively
/// anywhere in the key, so `csrf_token`, `access_token` and `user_password` are
/// all caught along with the bare names.
///
/// Substring matching over-redacts a little — `author` contains `auth`,
/// `secretary` contains `secret` — and that is the intended direction. A
/// redacted local costs a debugging session some context; a logged credential
/// costs a rotation, and logs get shipped, retained and shared.
pub(crate) const SECRET_KEY_SUBSTRINGS: &[&str] = &[
    "password",
    "passwd",
    "secret",
    "token",
    "api_key",
    "apikey",
    "private_key",
    "privatekey",
    "authorization",
    "auth",
    "session_id",
    "sessionid",
    "csrf",
];

/// The marker written in place of a redacted value. One spelling, so a log
/// reader (or a grep for leaks) only has to know about one.
pub(crate) const REDACTED: &str = "[REDACTED]";

/// Does this key look like it holds a secret?
pub(crate) fn looks_sensitive(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    SECRET_KEY_SUBSTRINGS.iter().any(|sub| lower.contains(sub))
}

#[cfg(test)]
mod tests {
    use super::looks_sensitive;

    #[test]
    fn catches_the_names_that_carry_credentials() {
        for key in [
            "password",
            "user_password",
            "passwd",
            "api_key",
            "apiKey",
            "private_key",
            "access_token",
            "csrf_token",
            "session_id",
            "Authorization",
            "session_secret",
            "SECRET",
        ] {
            assert!(looks_sensitive(key), "{key} must be redacted");
        }
    }

    #[test]
    fn leaves_ordinary_names_alone() {
        for key in ["user_name", "email", "id", "count", "created_at", "title"] {
            assert!(!looks_sensitive(key), "{key} must not be redacted");
        }
    }

    /// Documented over-redaction: substring matching catches these, and that
    /// is the deliberate direction. Pinned so the trade stays a decision
    /// rather than a surprise.
    #[test]
    fn over_redacts_a_few_innocent_names_on_purpose() {
        for key in ["author", "secretary", "authenticated_at"] {
            assert!(
                looks_sensitive(key),
                "{key} is expected to be caught by substring matching"
            );
        }
    }
}
