//! Shared REPL utilities used by both TUI and simple REPLs.

/// Check if a single input line should trigger multiline mode.
pub fn detect_multiline_needed(line: &str) -> bool {
    let trimmed = line.trim();
    // Check if the line opens a block via keyword or trailing brace
    let opens_block = trimmed.ends_with('{')
        || (trimmed.starts_with("class ") && !trimmed.ends_with('}'))
        || is_keyword_block_opener(trimmed);

    if !opens_block {
        return false;
    }

    // If the line also closes the block (e.g. `if x then print("hi") end`),
    // count_block_balance will return 0 — no multiline needed.
    count_block_balance(trimmed) > 0
}

/// Calculate the indentation level for the next REPL line based on the current line.
pub fn calculate_indent(line: &str) -> usize {
    let trimmed = line.trim_start();
    let leading_spaces = line.len() - trimmed.len();
    if trimmed == "end" {
        return leading_spaces.saturating_sub(4);
    }
    let extra_indent = if trimmed.ends_with('{')
        || trimmed.ends_with("then")
        || trimmed.ends_with("do")
        || trimmed.ends_with("catch")
        || trimmed.ends_with("finally")
        || trimmed.ends_with("try")
    {
        4
    } else if trimmed.ends_with("else") || trimmed.ends_with("elsif") {
        if trimmed.starts_with("els") {
            4
        } else {
            0
        }
    } else if is_keyword_block_opener(trimmed) && !trimmed.contains('{') {
        4
    } else {
        0
    };
    leading_spaces + extra_indent
}

/// Check if a trimmed line starts with a keyword that opens a block.
pub fn is_keyword_block_opener(trimmed: &str) -> bool {
    trimmed.starts_with("def ")
        || trimmed.starts_with("fn ")
        || trimmed.starts_with("if ")
        || trimmed.starts_with("unless ")
        || trimmed.starts_with("while ")
        || trimmed.starts_with("for ")
        || trimmed.starts_with("class ")
        || trimmed.starts_with("match ")
        || trimmed == "do"
        || trimmed.starts_with("do ")
        || trimmed.starts_with("try")
}

/// Count the block balance of a line: +1 for openers, -1 for closers.
/// Handles braces (string-aware) and keyword blocks (`def`/`end`).
pub fn count_block_balance(s: &str) -> i32 {
    let mut balance = 0;
    let mut in_string = false;
    let mut escaped = false;
    let mut has_braces = false;

    for c in s.chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '"' {
            in_string = true;
            escaped = false;
        } else if c == '{' {
            balance += 1;
            has_braces = true;
        } else if c == '}' {
            balance -= 1;
            has_braces = true;
        }
    }

    // Track keyword-based blocks when no braces on this line
    if !has_braces {
        let trimmed = s.trim();
        if trimmed == "end" {
            balance -= 1;
        } else if is_keyword_block_opener(trimmed) {
            balance += 1;
        }
    }

    balance
}

/// Check whether the REPL should auto-wrap source in `print()`.
pub fn should_print_result(source: &str) -> bool {
    let trimmed = source.trim_end_matches(';').trim();

    !trimmed.starts_with("let ")
        && !trimmed.starts_with("const ")
        && !trimmed.starts_with("fn ")
        && !trimmed.starts_with("def ")
        && !trimmed.starts_with("class ")
        && !trimmed.starts_with("interface ")
        && !trimmed.starts_with("if ")
        && !trimmed.starts_with("while ")
        && !trimmed.starts_with("for ")
        && !trimmed.starts_with("return ")
        && !trimmed.starts_with("print(")
        && !trimmed.starts_with("println(")
        && !trimmed.starts_with(".")
        && !trimmed.starts_with('#')
        && !trimmed.starts_with("//")
        && !trimmed.starts_with("try")
        && !trimmed.starts_with("import ")
}

/// Strip trailing comment from a line (respects strings).
fn strip_trailing_comment(s: &str) -> &str {
    let mut in_double = false;
    let mut in_single = false;
    let mut prev = '\0';
    for (i, c) in s.char_indices() {
        match c {
            '"' if !in_single && prev != '\\' => in_double = !in_double,
            '\'' if !in_double && prev != '\\' => in_single = !in_single,
            '#' | '/' if !in_double && !in_single => {
                if c == '#' {
                    return s[..i].trim_end();
                }
                // Check for //
                if c == '/' && s[i + 1..].starts_with('/') {
                    return s[..i].trim_end();
                }
            }
            _ => {}
        }
        prev = c;
    }
    s
}

/// Prepare REPL source for execution: auto-wrap in `print()` or append `;` as needed.
pub fn prepare_source(code: &str) -> String {
    let trimmed = code.trim();

    // Comment-only lines: pass through as-is (wrapping in print() would eat the closing paren)
    if trimmed.starts_with('#') || trimmed.starts_with("//") {
        return code.to_string();
    }

    let passthrough = trimmed.ends_with('}') || trimmed.ends_with(';') || trimmed.ends_with("end");

    if should_print_result(code) && !passthrough {
        let expr = strip_trailing_comment(trimmed);
        if expr.is_empty() {
            return format!("{};", trimmed);
        }
        format!("println(({}).inspect);", expr)
    } else if !passthrough
        && !trimmed.starts_with("let ")
        && !trimmed.starts_with("fn ")
        && !trimmed.starts_with("def ")
        && !trimmed.starts_with("class ")
        && !trimmed.starts_with("const ")
    {
        format!("{};", trimmed)
    } else {
        code.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_multiline_for() {
        assert!(detect_multiline_needed("for i in range(1, 10)"));
        assert!(detect_multiline_needed("for (i in range(1, 10))"));
        assert!(detect_multiline_needed("for i in 0..10"));
        assert!(detect_multiline_needed("if true"));
        assert!(detect_multiline_needed("while x > 0"));
        assert!(detect_multiline_needed("fn foo()"));
    }

    #[test]
    fn test_no_multiline_for_simple() {
        assert!(!detect_multiline_needed("let x = 1"));
        assert!(!detect_multiline_needed("println(1)"));
        assert!(!detect_multiline_needed("x += 1"));
        assert!(!detect_multiline_needed("x++"));
    }

    #[test]
    fn test_count_block_balance() {
        assert_eq!(count_block_balance("{"), 1);
        assert_eq!(count_block_balance("}"), -1);
        assert_eq!(count_block_balance("end"), -1);
        assert_eq!(count_block_balance("for i in range(1, 10)"), 1);
        assert_eq!(count_block_balance("println(i)"), 0);
    }
}
