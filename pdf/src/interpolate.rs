//! `${path}` interpolation and `#PAGE#` / `#TOTAL_PAGE#` substitution.

use crate::data::Resolver;
use crate::error::RenderWarning;

/// Replace every `${path}` token in `input` with its resolved value. Unresolved
/// paths are replaced with an empty string and recorded as a warning.
pub fn interpolate(input: &str, resolver: &Resolver, warnings: &mut Vec<RenderWarning>) -> String {
    if !input.contains("${") {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
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
    s.contains("#PAGE#") || s.contains("#TOTAL_PAGE#")
}

/// Substitute `#PAGE#` and `#TOTAL_PAGE#`. Order matters: replace the longer
/// token first so `#PAGE#` doesn't clobber the `#...#` inside `#TOTAL_PAGE#`.
pub fn substitute_page_tokens(s: &str, page: usize, total: usize) -> String {
    s.replace("#TOTAL_PAGE#", &total.to_string())
        .replace("#PAGE#", &page.to_string())
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
        assert!(has_page_tokens("x #PAGE#"));
        assert!(!has_page_tokens("plain"));
    }
}
