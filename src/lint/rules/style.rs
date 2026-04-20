use crate::lint::{LintDiagnostic, Severity};
use crate::span::Span;

pub fn check_empty_block(span: Span, diagnostics: &mut Vec<LintDiagnostic>) {
    diagnostics.push(LintDiagnostic {
        rule: "style/empty-block",
        message: "empty block".to_string(),
        span,
        severity: Severity::Warning,
    });
}

pub fn check_line_lengths(source: &str, diagnostics: &mut Vec<LintDiagnostic>) {
    for (i, line) in source.lines().enumerate() {
        if line.len() > 120 {
            diagnostics.push(LintDiagnostic {
                rule: "style/line-length",
                message: format!("line exceeds 120 characters ({} chars)", line.len()),
                span: Span::new(0, 0, i + 1, 1),
                severity: Severity::Warning,
            });
        }
    }
}

/// Warn when a controller file imports a model from `app/models/`. Models
/// are auto-loaded by `soli serve` and the REPL (see `load_models()` in
/// `src/serve/app_loader.rs`), so the import is redundant.
pub fn check_redundant_model_import(
    import_path: &str,
    file_path: Option<&str>,
    span: Span,
    diagnostics: &mut Vec<LintDiagnostic>,
) {
    let Some(file) = file_path else {
        return;
    };
    if !is_controller_path(file) {
        return;
    }
    if !is_model_import(import_path) {
        return;
    }
    diagnostics.push(LintDiagnostic {
        rule: "style/redundant-model-import",
        message: "redundant import: models in app/models/ are auto-loaded by soli serve and the REPL — remove this line"
            .to_string(),
        span,
        severity: Severity::Warning,
    });
}

fn is_controller_path(file: &str) -> bool {
    let normalised = file.replace('\\', "/");
    normalised.contains("/app/controllers/") || normalised.starts_with("app/controllers/")
}

fn is_model_import(path: &str) -> bool {
    let trimmed = path.trim();
    if !trimmed.ends_with(".sl") {
        return false;
    }
    let after_prefix = if let Some(rest) = trimmed.strip_prefix("../") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix('/') {
        rest
    } else {
        return false;
    };
    let Some(rest) = after_prefix.strip_prefix("models/") else {
        return false;
    };
    !rest.is_empty() && !rest.contains('/')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_block() {
        let mut d = Vec::new();
        check_empty_block(Span::new(0, 0, 1, 1), &mut d);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].rule, "style/empty-block");
    }

    #[test]
    fn test_line_length_under_limit() {
        let mut d = Vec::new();
        check_line_lengths("short line", &mut d);
        assert!(d.is_empty());
    }

    #[test]
    fn test_line_length_exactly_120() {
        let mut d = Vec::new();
        let exact_line = "x".repeat(120);
        check_line_lengths(&exact_line, &mut d);
        assert!(d.is_empty());
    }

    #[test]
    fn test_line_length_over_limit() {
        let mut d = Vec::new();
        let long_line = "x".repeat(121);
        check_line_lengths(&long_line, &mut d);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].rule, "style/line-length");
        assert_eq!(d[0].span.line, 1);
    }

    #[test]
    fn test_line_length_multiple_lines() {
        let mut d = Vec::new();
        let source = format!("short\n{}\nok\n{}", "a".repeat(130), "b".repeat(200));
        check_line_lengths(&source, &mut d);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].span.line, 2);
        assert_eq!(d[1].span.line, 4);
    }

    #[test]
    fn test_redundant_model_import_in_controller() {
        let mut d = Vec::new();
        check_redundant_model_import(
            "../models/contact_model.sl",
            Some("app/controllers/contacts_controller.sl"),
            Span::new(0, 0, 1, 1),
            &mut d,
        );
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].rule, "style/redundant-model-import");
    }

    #[test]
    fn test_redundant_model_import_absolute_nested_path() {
        let mut d = Vec::new();
        check_redundant_model_import(
            "../models/user_model.sl",
            Some("/home/alice/app/controllers/admin/users_controller.sl"),
            Span::new(0, 0, 1, 1),
            &mut d,
        );
        assert_eq!(d.len(), 1);
    }

    #[test]
    fn test_redundant_model_import_ignored_outside_controllers() {
        let mut d = Vec::new();
        check_redundant_model_import(
            "../models/contact_model.sl",
            Some("app/models/contact_model.sl"),
            Span::new(0, 0, 1, 1),
            &mut d,
        );
        assert!(d.is_empty());
    }

    #[test]
    fn test_non_model_import_ignored_in_controller() {
        let mut d = Vec::new();
        check_redundant_model_import(
            "../lib/util.sl",
            Some("app/controllers/home_controller.sl"),
            Span::new(0, 0, 1, 1),
            &mut d,
        );
        assert!(d.is_empty());
    }

    #[test]
    fn test_redundant_model_import_no_file_path_is_silent() {
        let mut d = Vec::new();
        check_redundant_model_import(
            "../models/contact_model.sl",
            None,
            Span::new(0, 0, 1, 1),
            &mut d,
        );
        assert!(d.is_empty());
    }

    #[test]
    fn test_nested_model_path_not_matched() {
        let mut d = Vec::new();
        check_redundant_model_import(
            "../models/subdir/foo.sl",
            Some("app/controllers/x_controller.sl"),
            Span::new(0, 0, 1, 1),
            &mut d,
        );
        assert!(d.is_empty());
    }
}
