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
        && !trimmed.starts_with("try")
        && !trimmed.starts_with("import ")
}

/// Prepare REPL source for execution: auto-wrap in `print()` or append `;` as needed.
pub fn prepare_source(code: &str) -> String {
    let trimmed = code.trim();

    let passthrough = trimmed.ends_with('}') || trimmed.ends_with(';') || trimmed.ends_with("end");

    if should_print_result(code) && !passthrough {
        format!("print({});", trimmed)
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
