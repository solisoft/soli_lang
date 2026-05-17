//! Document formatting provider for LSP.

use lsp_types::{Position, Range, TextEdit};

pub fn format_document(source: &str) -> Vec<TextEdit> {
    let mut edits = Vec::new();

    let lines: Vec<&str> = source.lines().collect();
    let mut indent_level: usize = 0;
    let indent_str = "    ";

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();

        if trimmed.is_empty() {
            continue;
        }

        let current_indent = line.len() - line.trim_start().len();
        let expected_indent = indent_level * indent_str.len();

        if trimmed.starts_with("}") || trimmed.starts_with(")") || trimmed.starts_with("]") {
            indent_level = indent_level.saturating_sub(1);
        }

        let new_expected_indent = indent_level * indent_str.len();

        if current_indent != new_expected_indent {
            let range = Range {
                start: Position {
                    line: i as u32,
                    character: 0,
                },
                end: Position {
                    line: i as u32,
                    character: current_indent as u32,
                },
            };
            edits.push(TextEdit {
                range,
                new_text: indent_str.repeat(indent_level),
            });
        }

        if trimmed.starts_with("{") || trimmed.starts_with("(") || trimmed.starts_with("[") {
            indent_level += 1;
        }
    }

    edits
}

pub fn format_range(source: &str, range: Range) -> Vec<TextEdit> {
    let lines: Vec<&str> = source.lines().collect();
    let start_line = range.start.line as usize;
    let end_line = range.end.line as usize;

    if start_line >= lines.len() || end_line >= lines.len() {
        return Vec::new();
    }

    let mut edits = Vec::new();
    let mut indent_level: usize = 0;

    for i in start_line..=end_line {
        let line = lines[i];
        let trimmed = line.trim_start();

        if trimmed.is_empty() {
            continue;
        }

        let current_indent = line.len() - line.trim_start().len();
        let expected_indent = indent_level * 4;

        if trimmed.starts_with("}") || trimmed.starts_with(")") || trimmed.starts_with("]") {
            indent_level = indent_level.saturating_sub(1);
        }

        let new_expected_indent = indent_level * 4;

        if current_indent != new_expected_indent {
            let range = Range {
                start: Position {
                    line: i as u32,
                    character: 0,
                },
                end: Position {
                    line: i as u32,
                    character: current_indent as u32,
                },
            };
            edits.push(TextEdit {
                range,
                new_text: "    ".repeat(indent_level),
            });
        }

        if trimmed.ends_with("{") || trimmed.ends_with("(") || trimmed.ends_with("[") {
            indent_level += 1;
        }
    }

    edits
}
