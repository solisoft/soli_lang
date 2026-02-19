use crate::lint::{LintDiagnostic, Severity};
use crate::span::Span;

fn has_uppercase(name: &str) -> bool {
    name.chars().any(|c| c.is_uppercase())
}

pub fn check_variable_name(name: &str, span: Span, diagnostics: &mut Vec<LintDiagnostic>) {
    if has_uppercase(name) {
        diagnostics.push(LintDiagnostic {
            rule: "naming/snake-case",
            message: format!("variable '{}' should use snake_case", name),
            span,
            severity: Severity::Warning,
        });
    }
}

pub fn check_function_name(name: &str, span: Span, diagnostics: &mut Vec<LintDiagnostic>) {
    if has_uppercase(name) {
        diagnostics.push(LintDiagnostic {
            rule: "naming/snake-case",
            message: format!("function '{}' should use snake_case", name),
            span,
            severity: Severity::Warning,
        });
    }
}

pub fn check_class_name(kind: &str, name: &str, span: Span, diagnostics: &mut Vec<LintDiagnostic>) {
    let starts_upper = name.starts_with(|c: char| c.is_uppercase());
    if !starts_upper || name.contains('_') {
        diagnostics.push(LintDiagnostic {
            rule: "naming/pascal-case",
            message: format!("{} '{}' should use PascalCase", kind, name),
            span,
            severity: Severity::Warning,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span() -> Span {
        Span::new(0, 0, 1, 1)
    }

    #[test]
    fn test_snake_case_valid() {
        let mut d = Vec::new();
        check_variable_name("foo_bar", span(), &mut d);
        check_variable_name("x", span(), &mut d);
        check_variable_name("_unused", span(), &mut d);
        check_variable_name("my_long_name", span(), &mut d);
        assert!(d.is_empty());
    }

    #[test]
    fn test_snake_case_invalid() {
        let mut d = Vec::new();
        check_variable_name("myVar", span(), &mut d);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].rule, "naming/snake-case");
        assert!(d[0].message.contains("myVar"));
    }

    #[test]
    fn test_function_name_valid() {
        let mut d = Vec::new();
        check_function_name("do_thing", span(), &mut d);
        check_function_name("run", span(), &mut d);
        assert!(d.is_empty());
    }

    #[test]
    fn test_function_name_invalid() {
        let mut d = Vec::new();
        check_function_name("doThing", span(), &mut d);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].rule, "naming/snake-case");
        assert!(d[0].message.contains("doThing"));
    }

    #[test]
    fn test_class_name_valid() {
        let mut d = Vec::new();
        check_class_name("class", "MyClass", span(), &mut d);
        check_class_name("interface", "Printable", span(), &mut d);
        check_class_name("class", "A", span(), &mut d);
        assert!(d.is_empty());
    }

    #[test]
    fn test_class_name_invalid_lowercase() {
        let mut d = Vec::new();
        check_class_name("class", "myClass", span(), &mut d);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].rule, "naming/pascal-case");
    }

    #[test]
    fn test_class_name_invalid_underscore() {
        let mut d = Vec::new();
        check_class_name("class", "My_Class", span(), &mut d);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].rule, "naming/pascal-case");
    }
}
