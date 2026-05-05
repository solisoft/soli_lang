//! HTML string manipulation functions.
//!
//! Provides shared implementations for HTML escaping, unescaping, sanitization, and stripping.

use std::collections::HashSet;
use std::sync::OnceLock;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

/// Register all HTML built-in functions.
pub fn register_html_builtins(env: &mut Environment) {
    // html_escape(string) - Escape HTML special characters
    env.define(
        "html_escape".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "html_escape",
            Some(1),
            |args| match &args[0] {
                Value::String(s) => Ok(Value::String(html_escape(s))),
                other => Err(format!(
                    "html_escape expects string, got {}",
                    other.type_name()
                )),
            },
        )),
    );

    // html_unescape(string) - Unescape HTML entities
    env.define(
        "html_unescape".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "html_unescape",
            Some(1),
            |args| match &args[0] {
                Value::String(s) => Ok(Value::String(html_unescape(s))),
                other => Err(format!(
                    "html_unescape expects string, got {}",
                    other.type_name()
                )),
            },
        )),
    );

    // sanitize_html(string) - Remove dangerous HTML tags and attributes
    env.define(
        "sanitize_html".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "sanitize_html",
            Some(1),
            |args| match &args[0] {
                Value::String(s) => Ok(Value::String(sanitize_html(s))),
                other => Err(format!(
                    "sanitize_html expects string, got {}",
                    other.type_name()
                )),
            },
        )),
    );

    // strip_html(string) -> string - removes all HTML tags
    env.define(
        "strip_html".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "strip_html",
            Some(1),
            |args| match &args[0] {
                Value::String(s) => Ok(Value::String(strip_html(s))),
                other => Err(format!(
                    "strip_html expects string, got {}",
                    other.type_name()
                )),
            },
        )),
    );
}

/// Escape HTML special characters.
pub fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Unescape HTML entities back to characters.
pub fn html_unescape(s: &str) -> String {
    let mut result = s.to_string();
    let replacements = [
        ("&amp;", "&"),
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&quot;", "\""),
        ("&#39;", "'"),
        ("&#x27;", "'"),
        ("&apos;", "'"),
        ("&nbsp;", " "),
    ];
    for (from, to) in replacements {
        result = result.replace(from, to);
    }
    result
}

/// Strip all HTML tags from a string.
pub fn strip_html(s: &str) -> String {
    let mut result = String::new();
    let mut in_tag = false;

    for c in s.chars() {
        if c == '<' {
            in_tag = true;
        } else if c == '>' {
            in_tag = false;
        } else if !in_tag {
            result.push(c);
        }
    }
    result
}

/// SEC-024: HTML sanitizer backed by `ammonia` (which uses `html5ever`'s
/// real HTML5 parser). The previous implementation was a substring
/// scanner that could be bypassed with entity-encoded attributes
/// (`<a href="java&#115;cript:...">`), unquoted attributes, comments,
/// CDATA, attribute newlines, and SVG sub-trees. A real parser is the
/// only durable defense — string scanning will keep producing bypasses.
fn sanitize_builder() -> &'static ammonia::Builder<'static> {
    static SANITIZER: OnceLock<ammonia::Builder<'static>> = OnceLock::new();
    SANITIZER.get_or_init(|| {
        let mut b = ammonia::Builder::default();
        // Allowlist: keep parity with the prior implementation's tag set.
        b.tags(HashSet::from([
            "p",
            "br",
            "b",
            "i",
            "u",
            "em",
            "strong",
            "a",
            "ul",
            "ol",
            "li",
            "blockquote",
            "code",
            "pre",
            "h1",
            "h2",
            "h3",
            "h4",
            "h5",
            "h6",
            "span",
            "div",
        ]));
        // Generic attributes mirror the prior `safe_attrs` list. Note
        // `style` is allowed for visual parity but ammonia strips
        // dangerous CSS values (expression(), url(javascript:), etc.).
        b.generic_attributes(HashSet::from([
            "href", "title", "alt", "class", "id", "style",
        ]));
        // Restrict `href` to network-safe schemes — refuses `javascript:`,
        // `data:`, `vbscript:`, etc. ammonia's own URL parser does the
        // entity-decoding so `java&#115;cript:` is also caught.
        b.url_schemes(HashSet::from(["http", "https", "mailto"]));
        b
    })
}

/// Sanitize HTML by keeping only safe tags and removing dangerous attributes.
pub fn sanitize_html(s: &str) -> String {
    sanitize_builder().clean(s).to_string()
}

/// Extract a substring from start to end index.
pub fn substring(s: &str, start: usize, end: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    let end = end.min(chars.len());
    let start = start.min(end);
    chars[start..end].iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SEC-024: bypass cases that defeated the prior substring scanner
    /// must all produce inert output now that ammonia is doing the work.
    /// Each case asserts the output cannot trigger script execution
    /// — the exact normalized form is ammonia's call.
    #[test]
    fn sanitize_html_blocks_known_bypasses() {
        let cases: &[(&str, &str)] = &[
            // Entity-encoded `javascript:` in an attribute.
            (
                r#"<a href="java&#115;cript:alert(1)">x</a>"#,
                "entity-encoded javascript",
            ),
            // Unquoted `javascript:` attribute.
            (
                r#"<a href=javascript:alert(1)>x</a>"#,
                "unquoted javascript",
            ),
            // Tab/newline-separated attribute.
            (
                "<a\thref=\"javascript:alert(1)\">x</a>",
                "tab-separated attribute",
            ),
            (
                "<a\nhref=\"javascript:alert(1)\">x</a>",
                "newline-separated attribute",
            ),
            // Mixed-case scheme.
            (
                r#"<a href="JaVaScRiPt:alert(1)">x</a>"#,
                "mixed-case javascript",
            ),
            // SVG sub-tree with onload.
            (
                "<svg><g onload=\"alert(1)\">x</g></svg>",
                "svg onload sub-tree",
            ),
            // foreignObject with embedded script.
            (
                "<svg><foreignObject><script>alert(1)</script></foreignObject></svg>",
                "svg foreignObject script",
            ),
            // iframe srcdoc.
            (
                "<iframe srcdoc=\"<script>alert(1)</script>\">x</iframe>",
                "iframe srcdoc",
            ),
            // HTML comment hiding script.
            (
                "<!-- <script>alert(1)</script> -->",
                "comment-wrapped script",
            ),
            // CDATA wrapper.
            (
                "<![CDATA[<script>alert(1)</script>]]>",
                "cdata-wrapped script",
            ),
            // event handler with leading space.
            (
                "<p onclick =\"alert(1)\">x</p>",
                "event handler with whitespace",
            ),
        ];
        for (input, label) in cases {
            let output = sanitize_html(input);
            let lower = output.to_lowercase();
            assert!(
                !lower.contains("javascript:"),
                "[{}] expected `javascript:` to be stripped, got: {}",
                label,
                output
            );
            assert!(
                !lower.contains("<script"),
                "[{}] expected `<script` to be stripped, got: {}",
                label,
                output
            );
            assert!(
                !lower.contains("onload"),
                "[{}] expected `onload` to be stripped, got: {}",
                label,
                output
            );
            assert!(
                !lower.contains("onclick"),
                "[{}] expected `onclick` to be stripped, got: {}",
                label,
                output
            );
            assert!(
                !lower.contains("onerror"),
                "[{}] expected `onerror` to be stripped, got: {}",
                label,
                output
            );
            assert!(
                !lower.contains("srcdoc"),
                "[{}] expected `srcdoc` to be stripped, got: {}",
                label,
                output
            );
        }
    }

    /// SEC-024 negative side: legitimate safe HTML must still survive.
    #[test]
    fn sanitize_html_keeps_safe_content() {
        // Plain paragraph text.
        let out = sanitize_html("<p>hello <b>world</b></p>");
        assert!(out.contains("hello"));
        assert!(out.contains("<b>world</b>") || out.contains("<b>world"));
        assert!(out.contains("<p>"));

        // Safe link.
        let out = sanitize_html(r#"<a href="https://example.com">click</a>"#);
        assert!(out.contains("https://example.com"));
        assert!(out.contains("click"));

        // mailto: scheme allowed.
        let out = sanitize_html(r#"<a href="mailto:x@y.z">contact</a>"#);
        assert!(out.contains("mailto:x@y.z"));
    }

    /// SEC-024: dangerous tags (script, iframe, object, embed, style)
    /// must be removed even if the rest of the document is safe.
    #[test]
    fn sanitize_html_removes_dangerous_tags() {
        for input in [
            "<script>alert(1)</script>",
            "<style>body{}</style>",
            "<object data='evil'></object>",
            "<embed src='evil'>",
            "<iframe src='evil'></iframe>",
            "<form action='evil'><input></form>",
            "<meta http-equiv='refresh' content='0;url=evil'>",
        ] {
            let out = sanitize_html(input);
            let lower = out.to_lowercase();
            assert!(
                !lower.contains("<script")
                    && !lower.contains("<style")
                    && !lower.contains("<object")
                    && !lower.contains("<embed")
                    && !lower.contains("<iframe")
                    && !lower.contains("<form")
                    && !lower.contains("<input")
                    && !lower.contains("<meta"),
                "input `{}` left a dangerous tag in output: {}",
                input,
                out
            );
        }
    }
}
