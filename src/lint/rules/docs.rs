use crate::interpreter::builtins::controller::api_docs::extract_action_docs_with_issues;
use crate::lint::{LintDiagnostic, Severity};
use crate::span::Span;

/// `docs/openapi`: a problem in an action's OpenAPI doc comments — an unknown
/// `@tag`, a `@body`/`@response` that starts like JSON but does not parse, a
/// `@response` without a status code, a `@param` without a name. The spec
/// generator tolerates all of them (the bad part is dropped or used as text),
/// which is exactly why a typo would otherwise go unnoticed.
pub fn check_openapi_docs(
    source: &str,
    file_path: Option<&str>,
    diagnostics: &mut Vec<LintDiagnostic>,
) {
    let Some(file) = file_path else {
        return;
    };
    if !super::style::is_controller_path(file) {
        return;
    }
    for issue in extract_action_docs_with_issues(source).1 {
        diagnostics.push(LintDiagnostic {
            rule: "docs/openapi",
            message: issue.message,
            span: Span::new(0, 0, issue.line, 1),
            severity: Severity::Warning,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "  # @returns 200\n  # @body {\"a\": }\n  def create\n  end\n";

    #[test]
    fn flags_bad_doc_tags_in_a_controller() {
        let mut d = Vec::new();
        check_openapi_docs(SRC, Some("app/controllers/posts_controller.sl"), &mut d);
        assert_eq!(d.len(), 2, "{d:?}");
        assert!(d.iter().all(|x| x.rule == "docs/openapi"));
        assert_eq!(d[0].span.line, 1);
        assert_eq!(d[1].span.line, 2);
    }

    #[test]
    fn ignores_files_outside_controllers() {
        let mut d = Vec::new();
        check_openapi_docs(SRC, Some("app/models/post.sl"), &mut d);
        assert!(d.is_empty());
    }
}
