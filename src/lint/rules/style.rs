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
}
