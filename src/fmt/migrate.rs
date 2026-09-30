//! `soli fmt --migrate-raw-strings`: rewrite the removed `[[ … ]]` raw strings
//! as `"""…"""`.
//!
//! `[[` opened a Lua-style raw string unless a digit, `-` or `[` followed it;
//! it is a pair of brackets now. Applied to old source, the new reading either
//! fails to parse (SQL, HTML, prose) or — worse — silently turns a JSON body
//! (`[[{"a": 1}]]`) into an array. This pass applies the *old* rule once, to
//! code written under it: outside strings, comments, `@sdbql{…}` blocks and
//! command substitutions, each `[[`-string becomes a `"""…"""` holding the
//! same bytes. It must not run on code written since: `[["a", 1]]` is a
//! nested array now, and this rule would read it as a string.

/// The rewritten source, how many literals were converted, and the 1-based
/// lines of `[[`-strings that could not be (their content holds `"""`, which
/// would close the new literal early). Those are left as they are.
pub fn migrate_legacy_raw_strings(source: &str) -> (String, usize, Vec<usize>) {
    let bytes = source.as_bytes();
    let mut out = String::with_capacity(source.len());
    let mut converted = 0usize;
    let mut skipped = Vec::new();
    let mut line = 1usize;
    let mut i = 0usize;
    // Start of the not-yet-copied stretch of `source`.
    let mut copied = 0usize;

    // Advance past bytes[i..end], counting newlines.
    let skip_to = |i: &mut usize, end: usize, line: &mut usize| {
        *line += bytes[*i..end].iter().filter(|&&b| b == b'\n').count();
        *i = end;
    };

    while i < bytes.len() {
        let b = bytes[i];
        let rest = &bytes[i..];

        // `"""…"""`: to the closing run of three or more quotes.
        if rest.starts_with(b"\"\"\"") {
            let body = i + 3;
            let close = find(bytes, body, b"\"\"\"").unwrap_or(bytes.len());
            let mut end = close;
            while end < bytes.len() && bytes[end] == b'"' {
                end += 1;
            }
            skip_to(&mut i, end, &mut line);
            continue;
        }
        // `"…"` / `'…'`, with escapes (`r"…"` is covered: the `r` is plain
        // code and its body holds no backslash escape to mis-skip).
        if b == b'"' || b == b'\'' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] != b {
                j += if bytes[j] == b'\\' { 2 } else { 1 };
            }
            skip_to(&mut i, (j + 1).min(bytes.len()), &mut line);
            continue;
        }
        // `@sdbql{…}` / `@sdql{…}`: to the matching brace.
        if b == b'@' && (starts_at(bytes, i + 1, b"sdbql{") || starts_at(bytes, i + 1, b"sdql{")) {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] != b'{' {
                j += 1;
            }
            let mut depth = 0u32;
            while j < bytes.len() {
                match bytes[j] {
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            j += 1;
                            break;
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            skip_to(&mut i, j, &mut line);
            continue;
        }
        // Command substitution.
        if b == b'`' {
            let close = find(bytes, i + 1, b"`").map_or(bytes.len(), |c| c + 1);
            skip_to(&mut i, close, &mut line);
            continue;
        }
        // `#` and `//` line comments.
        if b == b'#' || rest.starts_with(b"//") {
            let end = find(bytes, i, b"\n").unwrap_or(bytes.len());
            skip_to(&mut i, end, &mut line);
            continue;
        }
        // The old rule: `[[` not followed by a digit, `-` or `[`.
        if rest.starts_with(b"[[")
            && !matches!(bytes.get(i + 2), Some(c) if c.is_ascii_digit() || *c == b'-' || *c == b'[')
        {
            let body = i + 2;
            let Some(close) = find(bytes, body, b"]]") else {
                break; // unterminated: leave the rest untouched
            };
            let content = &source[body..close];
            if content.contains("\"\"\"") {
                skipped.push(line);
            } else {
                out.push_str(&source[copied..i]);
                out.push_str("\"\"\"");
                out.push_str(content);
                out.push_str("\"\"\"");
                copied = close + 2;
                converted += 1;
            }
            skip_to(&mut i, close + 2, &mut line);
            continue;
        }
        if b == b'\n' {
            line += 1;
        }
        i += 1;
    }
    out.push_str(&source[copied..]);
    (out, converted, skipped)
}

fn find(bytes: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    bytes
        .get(from..)?
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

fn starts_at(bytes: &[u8], at: usize, prefix: &[u8]) -> bool {
    bytes.get(at..).is_some_and(|r| r.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::migrate_legacy_raw_strings;

    fn migrate(src: &str) -> String {
        migrate_legacy_raw_strings(src).0
    }

    #[test]
    fn rewrites_old_raw_strings() {
        assert_eq!(
            migrate("q = db.query([[\n  FOR p IN posts RETURN p\n]])\n"),
            "q = db.query(\"\"\"\n  FOR p IN posts RETURN p\n\"\"\")\n"
        );
        // JSON, quotes at either end, and an empty one.
        assert_eq!(migrate(r#"t = [[{"a": "b"}]]"#), r#"t = """{"a": "b"}""""#);
        assert_eq!(migrate(r#"s = [["hi"]]"#), r#"s = """"hi"""""#);
        assert_eq!(migrate("e = [[]]"), "e = \"\"\"\"\"\"");
    }

    #[test]
    fn leaves_what_the_old_rule_did_not_read_as_a_string() {
        for src in [
            "m = [[1, 2], [3]]",
            "m = [[-1]]",
            "m = [[[0]]]",
            "s = \"[[not code]]\"",
            "# [[ a comment ]]",
            "q = @sdbql{ FOR x IN [[1]] RETURN x }",
            "r = \"\"\"keep [[this]]\"\"\"",
        ] {
            assert_eq!(migrate(src), src);
        }
    }

    #[test]
    fn content_holding_triple_quotes_is_reported_not_rewritten() {
        let src = "a = 1\nb = [[say \"\"\"hi\"\"\"]]\n";
        let (out, converted, skipped) = migrate_legacy_raw_strings(src);
        assert_eq!(out, src);
        assert_eq!(converted, 0);
        assert_eq!(skipped, vec![2]);
    }

    #[test]
    fn the_rewrite_lexes_to_the_same_value() {
        let content = "{\"x\": \"]\", \"y\": \"say \\\"hi\\\"\"}";
        let migrated = migrate(&format!("[[{content}]]"));
        let tokens = crate::lexer::Scanner::new(&migrated)
            .scan_tokens()
            .expect("migrated source lexes");
        assert!(matches!(
            &tokens[0].kind,
            crate::lexer::TokenKind::StringLiteral(v) if v == content
        ));
    }
}
