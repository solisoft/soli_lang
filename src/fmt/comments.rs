//! Source-comment extractor — the lexer drops comments before the parser sees
//! them, so the formatter needs its own scanner to find them and emit them at
//! the right places in the output.
//!
//! Recognizes `#` line comments, `//` line comments, and `/* … */` block
//! comments. Skips comment-like bytes that appear inside string literals
//! (`"…"`, `'…'`, `r"…"`, `"""…"""`, ``` `…` ```) and inside `@sdbql{…}`
//! blocks.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommentKind {
    /// `# …` or `// …`
    Line,
    /// `/* … */`
    Block,
}

#[derive(Debug, Clone)]
pub struct Comment {
    /// 1-indexed line where the comment starts.
    pub line: usize,
    /// 1-indexed column where the comment starts.
    pub column: usize,
    /// The raw comment text, including the leading marker (`#`, `//`, `/*`).
    pub text: String,
    pub kind: CommentKind,
}

/// Extract every comment from the source, in source order.
pub fn extract_comments(source: &str) -> Vec<Comment> {
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    let mut line = 1usize;
    let mut col = 1usize;

    while i < bytes.len() {
        let b = bytes[i];

        // Track line / column for whatever we *don't* skip below.
        if b == b'\n' {
            line += 1;
            col = 1;
            i += 1;
            continue;
        }

        // `"""…"""` raw multi-line string: its body may hold lone quotes, so
        // it is skipped to the closing run of quotes, not to the next `"`.
        if bytes[i..].starts_with(b"\"\"\"") {
            i += 3;
            col += 3;
            while i < bytes.len() && !bytes[i..].starts_with(b"\"\"\"") {
                if bytes[i] == b'\n' {
                    line += 1;
                    col = 1;
                } else {
                    col += 1;
                }
                i += 1;
            }
            while i < bytes.len() && bytes[i] == b'"' {
                i += 1;
                col += 1;
            }
            continue;
        }

        // String literal — skip its body so `#`/`//` inside don't register.
        // The end is found by the lexer's own rule (`quoted_string_end`): a
        // string nested in a `#{…}` is part of the outer literal, so
        // `"#{doc.fetch(".//TEXT")} tail"` holds no comment.
        if b == b'"' || b == b'\'' || raw_string_at(bytes, i) {
            let end = if b == b'r' {
                raw_string_end(bytes, i + 1)
            } else {
                quoted_string_end(bytes, i)
            };
            for &c in &bytes[i..end] {
                if c == b'\n' {
                    line += 1;
                    col = 1;
                } else {
                    col += 1;
                }
            }
            i = end;
            continue;
        }

        // `@sdbql{…}` / `@sdql{…}` query block. (`@"…"` is not a form of its
        // own: the lexer reads `@` then an ordinary string, and so does this.)
        if b == b'@'
            && (starts_with(bytes, i + 1, b"sdbql{") || starts_with(bytes, i + 1, b"sdql{"))
        {
            // Skip until the matching closing `}` (1-depth, since we're
            // not interpolating).
            while i < bytes.len() && bytes[i] != b'{' {
                i += 1;
                col += 1;
            }
            if i < bytes.len() {
                i += 1;
                col += 1;
            }
            let mut depth = 1u32;
            while i < bytes.len() && depth > 0 {
                let c = bytes[i];
                if c == b'{' {
                    depth += 1;
                } else if c == b'}' {
                    depth -= 1;
                } else if c == b'\n' {
                    line += 1;
                    col = 0;
                }
                i += 1;
                col += 1;
            }
            continue;
        }

        // Command substitution: `…`.
        if b == b'`' {
            i += 1;
            col += 1;
            while i < bytes.len() && bytes[i] != b'`' {
                if bytes[i] == b'\n' {
                    line += 1;
                    col = 1;
                } else {
                    col += 1;
                }
                i += 1;
            }
            if i < bytes.len() {
                i += 1;
                col += 1;
            }
            continue;
        }

        // `#` line comment.
        if b == b'#' {
            let start_line = line;
            let start_col = col;
            let start = i;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            let text = String::from_utf8_lossy(&bytes[start..i]).into_owned();
            out.push(Comment {
                line: start_line,
                column: start_col,
                text,
                kind: CommentKind::Line,
            });
            // The newline itself will be handled by the outer loop.
            continue;
        }

        // `//` line comment.
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            let start_line = line;
            let start_col = col;
            let start = i;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            let text = String::from_utf8_lossy(&bytes[start..i]).into_owned();
            out.push(Comment {
                line: start_line,
                column: start_col,
                text,
                kind: CommentKind::Line,
            });
            continue;
        }

        // `/* … */` block comment.
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            let start_line = line;
            let start_col = col;
            let start = i;
            i += 2;
            col += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                if bytes[i] == b'\n' {
                    line += 1;
                    col = 1;
                } else {
                    col += 1;
                }
                i += 1;
            }
            if i + 1 < bytes.len() {
                i += 2;
                col += 2;
            }
            let text = String::from_utf8_lossy(&bytes[start..i]).into_owned();
            out.push(Comment {
                line: start_line,
                column: start_col,
                text,
                kind: CommentKind::Block,
            });
            continue;
        }

        col += 1;
        i += 1;
    }

    out
}

/// Index just past the `"…"` / `'…'` literal whose opening quote is at
/// `open` — the lexer's rule (`Scanner::scan_string`), so the two agree on
/// where a string ends. A backslash escapes the next byte. `#{` opens an
/// interpolation, after which `{`/`}` count depth; while the depth is above
/// zero a quote of either kind is part of the expression (a nested string's
/// delimiter), not the end of the literal. At depth zero the opening quote
/// closes it. Unterminated: the end of the source.
pub(crate) fn quoted_string_end(bytes: &[u8], open: usize) -> usize {
    let quote = bytes[open];
    // Braces open in the code of the current `#{…}`; 0 in the string's text.
    let mut depth = 0u32;
    let mut i = open + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => {
                i += 2;
                continue;
            }
            // A string in the code of an interpolation is a string of its own,
            // with its own `#{…}` — skipped whole, as the lexer reads it.
            // Counting its characters here ended the outer literal at the
            // inner one's quote (`"#{"}"} tail"` read ` tail` as code).
            b'"' | b'\'' if depth > 0 => {
                i = quoted_string_end(bytes, i);
                continue;
            }
            b'#' if depth == 0 && bytes.get(i + 1) == Some(&b'{') => {
                depth = 1;
                i += 2;
                continue;
            }
            b'{' if depth > 0 => depth += 1,
            b'}' if depth > 0 => depth -= 1,
            c if c == quote && depth == 0 => return i + 1,
            _ => {}
        }
        i += 1;
    }
    bytes.len()
}

/// Whether `r"…"` / `r'…'` raw string starts at `at`: an `r` followed by a
/// quote that does not end an identifier (`bar"x"` is not one).
pub(crate) fn raw_string_at(bytes: &[u8], at: usize) -> bool {
    bytes[at] == b'r'
        && matches!(bytes.get(at + 1), Some(b'"' | b'\''))
        && !(at > 0 && {
            let prev = bytes[at - 1];
            prev.is_ascii_alphanumeric() || prev == b'_' || prev == b'@' || prev >= 0x80
        })
}

/// Index just past the raw string whose opening quote is at `open`: the next
/// matching quote, no escapes (`r"C:\"` ends at its second quote).
pub(crate) fn raw_string_end(bytes: &[u8], open: usize) -> usize {
    let quote = bytes[open];
    bytes[open + 1..]
        .iter()
        .position(|&c| c == quote)
        .map_or(bytes.len(), |p| open + 1 + p + 1)
}

fn starts_with(bytes: &[u8], at: usize, needle: &[u8]) -> bool {
    bytes.get(at..at + needle.len()) == Some(needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_hash_comments() {
        let src = "let x = 1 # trailing\n# leading\nlet y = 2\n";
        let cs = extract_comments(src);
        assert_eq!(cs.len(), 2);
        assert_eq!(cs[0].line, 1);
        assert_eq!(cs[0].text, "# trailing");
        assert_eq!(cs[1].line, 2);
        assert_eq!(cs[1].text, "# leading");
    }

    #[test]
    fn extracts_slash_comments() {
        let src = "// hello\nlet x = 1\n";
        let cs = extract_comments(src);
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].text, "// hello");
        assert_eq!(cs[0].kind, CommentKind::Line);
    }

    #[test]
    fn extracts_block_comment() {
        let src = "/* a\nb */\nlet x = 1\n";
        let cs = extract_comments(src);
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].kind, CommentKind::Block);
        assert!(cs[0].text.starts_with("/*"));
    }

    #[test]
    fn ignores_hash_inside_string() {
        let src = "let x = \"# not a comment\"\n# real comment\n";
        let cs = extract_comments(src);
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].line, 2);
    }

    #[test]
    fn ignores_slashes_inside_string() {
        let src = "let url = \"http://example.com\"\n// real\n";
        let cs = extract_comments(src);
        assert_eq!(cs.len(), 1);
        assert!(cs[0].text.starts_with("//"));
    }

    #[test]
    fn ignores_comments_inside_sdbql_block() {
        let src = "let q = @sdbql{ FOR u IN users # not a real comment\n RETURN u }\n# real\n";
        let cs = extract_comments(src);
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].text, "# real");
    }

    #[test]
    fn ignores_comment_in_multiline_string() {
        let src = "let s = \"\"\"\n\"quoted\" # inside\n\"\"\"\n# real\n";
        let cs = extract_comments(src);
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].text, "# real");
    }

    #[test]
    fn a_string_nested_in_an_interpolation_is_part_of_the_literal() {
        let src = "m = \"#{doc.fetch(\".//TEXT\") ?? \"x\"} (code 1)\" # real\n";
        let cs = extract_comments(src);
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].text, "# real");
        assert_eq!(cs[0].column, src.rfind("# real").unwrap() + 1);

        let src = "s = '#{h['//']} // x' // real\n";
        let cs = extract_comments(src);
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].text, "// real");
    }

    #[test]
    fn raw_strings_end_at_their_next_quote() {
        let cs = extract_comments("p = r\"C:\\\" # real\nq = r'//'\n");
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].text, "# real");
        // `bar"x"` is an identifier then a string, not a raw string.
        assert!(!raw_string_at(b"bar\"x\"", 2));
    }
}
