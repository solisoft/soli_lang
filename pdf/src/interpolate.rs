//! `${path}` interpolation and `#PAGE#` / `#TOTAL_PAGE#` substitution.

use crate::data::Resolver;
use crate::error::RenderWarning;

/// Replace every `${path}` token in `input` with its resolved value. Unresolved
/// paths are replaced with an empty string and recorded as a warning. A literal
/// `${` is written `$${` (double the `$`) — the escape passes the `${` through
/// verbatim instead of interpolating, so a template can show its own syntax
/// (e.g. a code sample, or a `$` that legitimately precedes a `{`).
pub fn interpolate(input: &str, resolver: &Resolver, warnings: &mut Vec<RenderWarning>) -> String {
    if !input.contains("${") {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Escape: `$${` renders a literal `${` (checked before the token start
        // so a doubled `$` always wins over interpolation).
        if bytes[i] == b'$' && i + 2 < bytes.len() && bytes[i + 1] == b'$' && bytes[i + 2] == b'{' {
            out.push_str("${");
            i += 3;
            continue;
        }
        if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            if let Some(end) = find_close(input, i + 2) {
                let path = input[i + 2..end].trim();
                match resolver.lookup(path) {
                    Some(v) => out.push_str(&v),
                    None => warnings.push(RenderWarning::MissingPath(path.to_string())),
                }
                i = end + 1;
                continue;
            }
        }
        // Not a token start (or unterminated): copy the char.
        let ch = input[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn find_close(s: &str, from: usize) -> Option<usize> {
    s[from..].find('}').map(|rel| from + rel)
}

/// Whether a string still contains page tokens (so footer alignment must be
/// recomputed in pass 2).
pub fn has_page_tokens(s: &str) -> bool {
    s.contains("#PAGE#")
        || s.contains("#PAGES#")
        || s.contains("#TOTAL_PAGE#")
        || s.contains("#PAGE_OF:")
}

/// Substitute `#PAGE#` and `#TOTAL_PAGE#` (alias `#PAGES#`). Order matters:
/// replace the longer tokens first so `#PAGE#` doesn't clobber the `#...#`
/// inside them.
pub fn substitute_page_tokens(s: &str, page: usize, total: usize) -> String {
    s.replace("#TOTAL_PAGE#", &total.to_string())
        .replace("#PAGES#", &total.to_string())
        .replace("#PAGE#", &page.to_string())
}

/// Substitute every `#PAGE_OF:anchor#` token with the 1-based page number of
/// the named `anchor` (the jump targets set by paragraph `options.anchor`) —
/// what turns a `linkTo` table of contents into a real one with page numbers.
/// An unknown anchor renders empty and records a warning.
pub fn substitute_anchor_tokens(
    s: &str,
    anchors: &std::collections::HashMap<String, (usize, f32)>,
    warnings: &mut Vec<RenderWarning>,
) -> String {
    const OPEN: &str = "#PAGE_OF:";
    if !s.contains(OPEN) {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find(OPEN) {
        out.push_str(&rest[..start]);
        let after = &rest[start + OPEN.len()..];
        match after.find('#') {
            Some(end) => {
                let name = after[..end].trim();
                match anchors.get(name) {
                    Some((page_idx, _)) => out.push_str(&(page_idx + 1).to_string()),
                    None => warnings.push(RenderWarning::MissingPath(format!("PAGE_OF:{name}"))),
                }
                rest = &after[end + 1..];
            }
            None => {
                // Unterminated token: keep it literal.
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::DataDocument;

    #[test]
    fn basic_interpolation() {
        let d = DataDocument::parse(br#"{"data":{"a":{"b":"X"},"n":7}}"#).unwrap();
        let r = d.resolver();
        let mut w = Vec::new();
        assert_eq!(interpolate("v=${a.b}/${n}", &r, &mut w), "v=X/7");
        assert!(w.is_empty());
    }

    #[test]
    fn missing_path_warns_and_blanks() {
        let d = DataDocument::parse(br#"{"data":{}}"#).unwrap();
        let r = d.resolver();
        let mut w = Vec::new();
        assert_eq!(interpolate("[${gone}]", &r, &mut w), "[]");
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn double_dollar_escapes_to_literal() {
        let d = DataDocument::parse(br#"{"data":{"name":"Ada"}}"#).unwrap();
        let r = d.resolver();
        let mut w = Vec::new();
        // `$${name}` passes through as a literal `${name}` — no lookup, no warning.
        assert_eq!(
            interpolate(r#"text: "$${name}""#, &r, &mut w),
            r#"text: "${name}""#
        );
        assert!(w.is_empty());
        // A real token beside an escaped one still resolves.
        assert_eq!(interpolate("$${a} and ${name}", &r, &mut w), "${a} and Ada");
        assert!(w.is_empty());
    }

    #[test]
    fn unicode_passthrough() {
        let d = DataDocument::parse(br#"{"data":{}}"#).unwrap();
        let r = d.resolver();
        let mut w = Vec::new();
        assert_eq!(
            interpolate("Invoice こんにちは", &r, &mut w),
            "Invoice こんにちは"
        );
    }

    #[test]
    fn page_tokens() {
        assert_eq!(
            substitute_page_tokens("Page #PAGE# of #TOTAL_PAGE#", 2, 5),
            "Page 2 of 5"
        );
        // #PAGES# is an alias of #TOTAL_PAGE#.
        assert_eq!(
            substitute_page_tokens("Page #PAGE# of #PAGES#", 2, 5),
            "Page 2 of 5"
        );
        assert!(has_page_tokens("x #PAGE#"));
        assert!(has_page_tokens("x #PAGES#"));
        assert!(!has_page_tokens("plain"));
    }
}
