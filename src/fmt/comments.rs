//! Source-comment extractor — the lexer drops comments before the parser sees
//! them, so the formatter needs its own scanner to find them and emit them at
//! the right places in the output.
//!
//! Recognizes `#` line comments, `//` line comments, and `/* … */` block
//! comments. Skips comment-like bytes that appear inside string literals
//! (`"…"`, `'…'`, `@"…"`, `[[…]]`, ``` `…` ```) and inside `@sdbql{…}` blocks.

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

        // String literal — skip its body so `#`/`//` inside don't register.
        if b == b'"' || b == b'\'' {
            let quote = b;
            i += 1;
            col += 1;
            while i < bytes.len() && bytes[i] != quote {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    // Skip an escape sequence.
                    if bytes[i + 1] == b'\n' {
                        line += 1;
                        col = 1;
                        i += 2;
                        continue;
                    }
                    i += 2;
                    col += 2;
                    continue;
                }
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

        // `@"…"` raw string or `@sdbql{…}` / `@sdql{…}` query block.
        if b == b'@' && i + 1 < bytes.len() {
            let next = bytes[i + 1];
            if next == b'"' {
                // Raw string until next `"`.
                i += 2;
                col += 2;
                while i < bytes.len() && bytes[i] != b'"' {
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
            if starts_with(bytes, i + 1, b"sdbql{") || starts_with(bytes, i + 1, b"sdql{") {
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
        }

        // `[[…]]` multi-line string.
        if b == b'[' && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            i += 2;
            col += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b']' && bytes[i + 1] == b']') {
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
        let src = "let s = [[\n# inside\n]]\n# real\n";
        let cs = extract_comments(src);
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].text, "# real");
    }
}
