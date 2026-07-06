//! JSONP ("JSON with Padding") helpers shared by the read (`JSON.parse_jsonp`,
//! `HTTP.get_jsonp`) and write (`render_jsonp`) builtins.
//!
//! JSONP bypasses the same-origin policy: a `<script src="…?callback=fn">` loads
//! a response body of the form `fn({…json…});` and the browser executes it,
//! invoking the pre-registered `fn`. Two primitives live here:
//!
//! * [`strip_jsonp_padding`] unwraps `fn(<json>);` back to the raw `<json>` so it
//!   can be handed to the normal JSON parser.
//! * [`is_valid_jsonp_callback`] validates a callback identifier against an
//!   injection-safe whitelist before it is ever reflected into a response body.

/// Unwrap the callback padding from a JSONP response body, returning the inner
/// JSON slice.
///
/// Handles the common shapes — `cb({…})`, `cb([…]);`, a leading `/**/`
/// anti-sniffing guard, and surrounding whitespace — by taking everything
/// between the first `(` and the last `)`. Any `(`/`)` embedded inside JSON
/// string values stays inside that span, so quoted parens are preserved.
///
/// Returns `Err` when the body has no `(`, no `)`, or a `)` before its `(`.
pub fn strip_jsonp_padding(body: &str) -> Result<&str, String> {
    let trimmed = body.trim();
    // Drop the optional `/**/` content-sniffing guard some servers prepend.
    let trimmed = trimmed.strip_prefix("/**/").unwrap_or(trimmed).trim_start();

    let open = trimmed
        .find('(')
        .ok_or_else(|| "not a JSONP response: no '(' found".to_string())?;
    let close = trimmed
        .rfind(')')
        .ok_or_else(|| "not a JSONP response: no ')' found".to_string())?;
    if close <= open {
        return Err("malformed JSONP response: ')' before '('".to_string());
    }
    // `(` and `)` are ASCII, so `open + 1` and `close` are valid char boundaries
    // even when the JSON payload contains multi-byte UTF-8.
    Ok(trimmed[open + 1..close].trim())
}

/// Validate a JSONP callback identifier.
///
/// Accepts a JS-identifier-shaped name — first char `[A-Za-z_$]`, rest
/// `[A-Za-z0-9_$.]`, length `1..=64` — which allows namespaced callbacks like
/// `angular.callbacks._0` while rejecting anything containing `()`, `;`, quotes,
/// `<`, `>`, whitespace, or brackets. Because the name is only ever emitted as
/// `name(<json>);`, this whitelist is sufficient to prevent breaking out of the
/// call context (the core JSONP XSS vector).
pub fn is_valid_jsonp_callback(name: &str) -> bool {
    // All accepted chars are ASCII, so byte length == char count here.
    if name.is_empty() || name.len() > 64 {
        return false;
    }
    let mut chars = name.chars();
    let first = chars.next().unwrap();
    if !(first.is_ascii_alphabetic() || first == '_' || first == '$') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$' || c == '.')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_object_padding() {
        assert_eq!(
            strip_jsonp_padding("cb({\"a\": 1});").unwrap(),
            "{\"a\": 1}"
        );
    }

    #[test]
    fn strips_array_padding_without_semicolon() {
        assert_eq!(strip_jsonp_padding("cb([1, 2, 3])").unwrap(), "[1, 2, 3]");
    }

    #[test]
    fn strips_leading_comment_guard_and_whitespace() {
        assert_eq!(
            strip_jsonp_padding("  /**/ handleData({\"ok\": true});\n").unwrap(),
            "{\"ok\": true}"
        );
    }

    #[test]
    fn preserves_parens_inside_string_values() {
        assert_eq!(
            strip_jsonp_padding("cb({\"x\": \"a)b(c\"})").unwrap(),
            "{\"x\": \"a)b(c\"}"
        );
    }

    #[test]
    fn errors_on_missing_parens() {
        assert!(strip_jsonp_padding("not jsonp at all").is_err());
        assert!(strip_jsonp_padding("cb{\"a\":1}").is_err());
        assert!(strip_jsonp_padding(")cb(").is_err());
    }

    #[test]
    fn accepts_valid_callbacks() {
        for name in [
            "cb",
            "handleData",
            "angular.callbacks._0",
            "jQuery19_1",
            "$",
            "_private",
        ] {
            assert!(is_valid_jsonp_callback(name), "{name} should be valid");
        }
    }

    #[test]
    fn rejects_invalid_callbacks() {
        for name in [
            "", "1abc", "alert(1)", "a;b", "<script>", "a b", "a[0]", "a-b",
        ] {
            assert!(!is_valid_jsonp_callback(name), "{name} should be invalid");
        }
        // Over the 64-char cap.
        assert!(!is_valid_jsonp_callback(&"a".repeat(65)));
    }
}
