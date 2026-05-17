//! Folding ranges provider for LSP.

use lsp_types::{FoldingRange, FoldingRangeKind};

pub fn get_folding_ranges(source: &str) -> Vec<FoldingRange> {
    let mut ranges = Vec::new();
    let lines: Vec<&str> = source.lines().collect();

    let mut brace_stack: Vec<(usize, char)> = Vec::new();

    for (i, line) in lines.iter().enumerate() {
        for (j, c) in line.char_indices() {
            match c {
                '{' | '(' | '[' => {
                    brace_stack.push((i, c));
                }
                '}' => {
                    if let Some((start, '{')) = brace_stack.pop() {
                        if start != i {
                            ranges.push(FoldingRange {
                                start_line: start as u32,
                                start_character: None,
                                end_line: i as u32,
                                end_character: Some(j as u32),
                                kind: Some(FoldingRangeKind::Region),
                                collapsed_text: None,
                            });
                        }
                    }
                }
                ')' => {
                    if let Some((start, '(')) = brace_stack.pop() {
                        if start != i {
                            ranges.push(FoldingRange {
                                start_line: start as u32,
                                start_character: None,
                                end_line: i as u32,
                                end_character: Some(j as u32),
                                kind: Some(FoldingRangeKind::Region),
                                collapsed_text: None,
                            });
                        }
                    }
                }
                ']' => {
                    if let Some((start, '[')) = brace_stack.pop() {
                        if start != i {
                            ranges.push(FoldingRange {
                                start_line: start as u32,
                                start_character: None,
                                end_line: i as u32,
                                end_character: Some(j as u32),
                                kind: Some(FoldingRangeKind::Region),
                                collapsed_text: None,
                            });
                        }
                    }
                }
                _ => {}
            }
        }

        let trimmed = line.trim();
        if trimmed.starts_with("class ")
            || trimmed.starts_with("fn ")
            || trimmed.starts_with("def ")
        {
            if let Some(start_brace) = line.find('{') {
                let mut depth = 0;
                let mut found = false;
                for k in i + 1..lines.len() {
                    for c in lines[k].chars() {
                        if c == '{' {
                            depth += 1;
                        } else if c == '}' {
                            if depth == 0 {
                                ranges.push(FoldingRange {
                                    start_line: i as u32,
                                    start_character: Some(start_brace as u32 + 1),
                                    end_line: k as u32,
                                    end_character: Some(lines[k].find('}').unwrap_or(0) as u32),
                                    kind: Some(FoldingRangeKind::Region),
                                    collapsed_text: None,
                                });
                                found = true;
                                break;
                            }
                            depth -= 1;
                        }
                    }
                    if found {
                        break;
                    }
                }
            }
        }
    }

    ranges
}
