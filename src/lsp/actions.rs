//! Code actions for LSP: quick fixes for the lint findings on the requested lines.
use super::util::{identifier_occurrences, range_of};
use std::collections::HashMap;
use tower_lsp::lsp_types::{
    CodeAction, CodeActionKind, Diagnostic, DiagnosticSeverity, NumberOrString, Position, Range,
    TextEdit, Url, WorkspaceEdit,
};

/// For each lint finding on the lines of `range`: a rename for a naming rule
/// (every occurrence of the name), and an "ignore" that writes the suppression
/// comment above the line. Both edit `uri`.
///
/// They used to be no-ops: the ignore action carried no edit at all, and the
/// renames matched rule names the linter never emits (`snake_case_function`
/// for `naming/snake-case`), so they never appeared.
pub fn get_code_actions(source: &str, range: Range, uri: &Url) -> Vec<CodeAction> {
    let Ok(findings) = crate::lint(source) else {
        return Vec::new();
    };
    let mut actions = Vec::new();
    for finding in findings {
        let line = finding.span.line_usize().saturating_sub(1) as u32;
        if line < range.start.line || line > range.end.line {
            continue;
        }
        let column = finding.span.column_usize().saturating_sub(1) as u32;
        let diagnostic = Diagnostic {
            range: Range::new(Position::new(line, column), Position::new(line, column + 1)),
            severity: Some(DiagnosticSeverity::WARNING),
            code: Some(NumberOrString::String(finding.rule.to_string())),
            source: Some("soli".to_string()),
            message: finding.message.clone(),
            ..Default::default()
        };
        if let Some(rename) = naming_fix(source, finding.rule, &finding.message, uri, &diagnostic) {
            actions.push(rename);
        }
        actions.push(ignore_fix(source, line, finding.rule, uri, &diagnostic));
    }
    actions
}

/// `naming/snake-case` / `naming/pascal-case`: rename the name the message
/// quotes ("function 'myFunction' should use snake_case") everywhere in the file.
fn naming_fix(
    source: &str,
    rule: &str,
    message: &str,
    uri: &Url,
    diagnostic: &Diagnostic,
) -> Option<CodeAction> {
    let name = message.split('\'').nth(1)?;
    let suggested = match rule {
        "naming/snake-case" => crate::scaffold::utils::to_snake_case(name),
        "naming/pascal-case" => crate::scaffold::utils::to_pascal_case(name),
        _ => return None,
    };
    if suggested.is_empty() || suggested == name {
        return None;
    }
    let edits: Vec<TextEdit> = identifier_occurrences(source, name)
        .into_iter()
        .map(|(start, end)| TextEdit {
            range: range_of(source, start, end),
            new_text: suggested.clone(),
        })
        .collect();
    if edits.is_empty() {
        return None;
    }
    Some(CodeAction {
        title: format!("Rename {name} to {suggested}"),
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: Some(vec![diagnostic.clone()]),
        edit: Some(WorkspaceEdit {
            changes: Some(HashMap::from([(uri.clone(), edits)])),
            ..Default::default()
        }),
        is_preferred: Some(true),
        ..Default::default()
    })
}

/// Insert `# soli-lint-disable-next-line <rule>` above the line, at its indentation.
fn ignore_fix(
    source: &str,
    line: u32,
    rule: &str,
    uri: &Url,
    diagnostic: &Diagnostic,
) -> CodeAction {
    let line_text = source.lines().nth(line as usize).unwrap_or("");
    let indent: String = line_text
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect();
    CodeAction {
        title: format!("Ignore {rule} on this line"),
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: Some(vec![diagnostic.clone()]),
        edit: Some(WorkspaceEdit {
            changes: Some(HashMap::from([(
                uri.clone(),
                vec![TextEdit {
                    range: Range::new(Position::new(line, 0), Position::new(line, 0)),
                    new_text: format!("{indent}# soli-lint-disable-next-line {rule}\n"),
                }],
            )])),
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn whole(source: &str) -> Range {
        Range::new(
            Position::new(0, 0),
            Position::new(source.lines().count() as u32, 0),
        )
    }

    #[test]
    fn a_camel_case_function_gets_a_rename_of_every_occurrence_and_an_ignore() {
        let uri = Url::parse("file:///app/x.sl").unwrap();
        let source = "def myFunction(x)\n  x\nend\n\nprint(myFunction(1))\n";
        let actions = get_code_actions(source, whole(source), &uri);
        let rename = actions
            .iter()
            .find(|a| a.title.starts_with("Rename"))
            .unwrap();
        assert_eq!(rename.title, "Rename myFunction to my_function");
        let edits = &rename.edit.as_ref().unwrap().changes.as_ref().unwrap()[&uri];
        assert_eq!(edits.len(), 2);
        let ignore = actions
            .iter()
            .find(|a| a.title.starts_with("Ignore"))
            .unwrap();
        let insert = &ignore.edit.as_ref().unwrap().changes.as_ref().unwrap()[&uri][0];
        assert_eq!(
            insert.new_text,
            "# soli-lint-disable-next-line naming/snake-case\n"
        );
    }

    #[test]
    fn the_inserted_comment_silences_the_finding() {
        let source =
            "# soli-lint-disable-next-line naming/snake-case\ndef myFunction(x)\n  x\nend\n";
        assert!(crate::lint(source).unwrap().is_empty());
    }
}
