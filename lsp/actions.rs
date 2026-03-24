//! Code actions provider for LSP.

use lsp_types::{
    CodeAction, CodeActionKind, Diagnostic, DiagnosticSeverity, Position, Range, TextEdit,
    WorkspaceEdit,
};

pub fn get_code_actions(source: &str, range: Range) -> Vec<CodeAction> {
    let mut actions = Vec::new();

    if let Ok(diagnostics) = crate::lint(source) {
        for diag in diagnostics {
            let diag_range = Range {
                start: Position {
                    line: (diag.span.line.saturating_sub(1) as u32).max(range.start.line),
                    character: (diag.span.column.saturating_sub(1) as u32)
                        .max(range.start.character),
                },
                end: Position {
                    line: (diag.span.line.saturating_sub(1) as u32).min(range.end.line),
                    character: ((diag.span.column + diag.message.len()) as u32)
                        .min(range.end.character),
                },
            };

            if diag_range.start.line >= range.start.line && diag_range.end.line <= range.end.line {
                if let Some(fix) = get_fix_for_rule(&diag.rule, &diag.message, diag_range) {
                    actions.push(fix);
                }

                actions.push(CodeAction {
                    title: format!("Ignore {}", diag.rule),
                    kind: Some(CodeActionKind::QUICKFIX),
                    diagnostics: Some(vec![Diagnostic {
                        range: diag_range,
                        severity: Some(DiagnosticSeverity::WARNING),
                        message: diag.message.clone(),
                        code: Some(lsp_types::NumberOrString::String(diag.rule.to_string())),
                        source: Some("soli".to_string()),
                        ..Default::default()
                    }]),
                    command: None,
                    ..Default::default()
                });
            }
        }
    }

    actions
}

fn get_fix_for_rule(rule: &str, message: &str, range: Range) -> Option<CodeAction> {
    match rule {
        "snake_case_variable" => {
            let suggested = message
                .split("should be snake_case, e.g., '")
                .nth(1)?
                .split('\'')
                .next()?;

            Some(CodeAction {
                title: format!("Rename to {}", suggested),
                kind: Some(CodeActionKind::QUICKFIX),
                edit: Some(WorkspaceEdit {
                    changes: Some(std::collections::HashMap::from([(
                        lsp_types::Url::parse("file:///").unwrap(),
                        vec![TextEdit {
                            range,
                            new_text: suggested.to_string(),
                        }],
                    )])),
                    ..Default::default()
                }),
                ..Default::default()
            })
        }
        "snake_case_function" => {
            let suggested = message
                .split("should be snake_case, e.g., '")
                .nth(1)?
                .split('\'')
                .next()?;

            Some(CodeAction {
                title: format!("Rename to {}", suggested),
                kind: Some(CodeActionKind::QUICKFIX),
                edit: Some(WorkspaceEdit {
                    changes: Some(std::collections::HashMap::from([(
                        lsp_types::Url::parse("file:///").unwrap(),
                        vec![TextEdit {
                            range,
                            new_text: suggested.to_string(),
                        }],
                    )])),
                    ..Default::default()
                }),
                ..Default::default()
            })
        }
        "pascal_case_class" => {
            let suggested = message
                .split("should be PascalCase, e.g., '")
                .nth(1)?
                .split('\'')
                .next()?;

            Some(CodeAction {
                title: format!("Rename to {}", suggested),
                kind: Some(CodeActionKind::QUICKFIX),
                edit: Some(WorkspaceEdit {
                    changes: Some(std::collections::HashMap::from([(
                        lsp_types::Url::parse("file:///").unwrap(),
                        vec![TextEdit {
                            range,
                            new_text: suggested.to_string(),
                        }],
                    )])),
                    ..Default::default()
                }),
                ..Default::default()
            })
        }
        "empty_block" => Some(CodeAction {
            title: "Remove empty block".to_string(),
            kind: Some(CodeActionKind::QUICKFIX),
            edit: Some(WorkspaceEdit {
                changes: Some(std::collections::HashMap::from([(
                    lsp_types::Url::parse("file:///").unwrap(),
                    vec![TextEdit {
                        range,
                        new_text: String::new(),
                    }],
                )])),
                ..Default::default()
            }),
            ..Default::default()
        }),
        _ => None,
    }
}
