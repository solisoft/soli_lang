//! Security-focused lint rules.
//!
//! `smell/dangerous-server-builtin` (SEC-085) flags calls to powerful but
//! injection-prone builtins from request-handling code (`app/controllers/`,
//! `app/middleware/`, `app/views/`). The lint doesn't trace data flow — it
//! catches the broad-stroke pattern of "controller code reaches for
//! `db_query_raw`, `Trusted.*`, `System.shell`, or backtick command
//! substitution" and suggests the safe alternative for each. Models,
//! migrations, tests, and helpers are out of scope: those layers
//! legitimately use these APIs against operator-supplied data.

use crate::ast::expr::{Expr, ExprKind};
use crate::lint::{LintDiagnostic, Severity};
use crate::span::Span;

/// Returns true when `file_path` lives in a request-handling MVC dir.
/// Mirrors the path-aware pattern in `style/redundant-model-import`.
fn is_request_handling_path(file: &str) -> bool {
    let normalised = file.replace('\\', "/");
    let dirs = ["app/controllers/", "app/middleware/", "app/views/"];
    dirs.iter()
        .any(|d| normalised.contains(&format!("/{}", d)) || normalised.starts_with(d))
}

/// Inspect a single expression for a dangerous-builtin call. Recurses into
/// nested expressions (call arguments etc.) via the parent linter — this
/// helper only looks at the immediate node so the existing
/// `lint_expr` recursion still handles children.
pub fn check_dangerous_server_builtin(
    expr: &Expr,
    file_path: Option<&str>,
    diagnostics: &mut Vec<LintDiagnostic>,
) {
    let Some(file) = file_path else {
        return;
    };
    if !is_request_handling_path(file) {
        return;
    }

    if let Some((rule_message, span)) = classify(expr) {
        diagnostics.push(LintDiagnostic {
            rule: "smell/dangerous-server-builtin",
            message: rule_message.to_string(),
            span,
            severity: Severity::Warning,
        });
    }
}

/// Match the expression against the known-dangerous shapes. Returns the
/// human-readable message (with the safe alternative spelled out) plus
/// the span to attach the diagnostic to.
fn classify(expr: &Expr) -> Option<(&'static str, Span)> {
    match &expr.kind {
        // Backtick command substitution: `ls -la` etc. Always shells out.
        ExprKind::CommandSubstitution(_) => Some((
            "command substitution (\"`...`\") shells out — request-controlled \
             input becomes shell injection. Prefer System.run([\"prog\", \"arg1\", ...]) \
             with an argv array, which never invokes a shell.",
            expr.span,
        )),
        ExprKind::Call { callee, .. } => match &callee.kind {
            // Bare-name builtins.
            ExprKind::Variable(name) if name == "db_query_raw" => Some((
                "db_query_raw splices its argument straight into a query — \
                 request-derived input becomes SQL/AQL injection. Prefer \
                 the parameterised `@sdbql{ ... #{value} ... }` block or \
                 `Model.where(\"x = #{v}\", { \"v\": v })` so values are \
                 bound, not interpolated.",
                expr.span,
            )),
            // Class.method calls — `Trusted.*` and `System.shell*`.
            ExprKind::Member { object, name } => match (&object.kind, name.as_str()) {
                (ExprKind::Variable(class), _) if class == "Trusted" => Some((
                    "Trusted.* bypasses the app-root filesystem jail — \
                     request-controlled paths become arbitrary file read/write. \
                     Prefer the jailed `File.*` API (File.read, File.write, \
                     File.exists), which keeps every operation under the app \
                     root.",
                    expr.span,
                )),
                (ExprKind::Variable(class), method)
                    if class == "System" && (method == "shell" || method == "shell_sync") =>
                {
                    Some((
                        "System.shell / System.shell_sync execute through `sh -c` — \
                         request-controlled input becomes shell injection. Prefer \
                         System.run / System.run_sync with an argv array \
                         ([\"prog\", \"arg1\", ...]), which never invokes a shell.",
                        expr.span,
                    ))
                }
                _ => None,
            },
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::expr::{Argument, Expr, ExprKind};

    fn span() -> Span {
        Span::new(0, 0, 1, 1)
    }

    fn variable(name: &str) -> Expr {
        Expr::new(ExprKind::Variable(name.to_string()), span())
    }

    fn call(callee: Expr) -> Expr {
        Expr::new(
            ExprKind::Call {
                callee: Box::new(callee),
                arguments: Vec::<Argument>::new(),
            },
            span(),
        )
    }

    fn member(object: Expr, name: &str) -> Expr {
        Expr::new(
            ExprKind::Member {
                object: Box::new(object),
                name: name.to_string(),
            },
            span(),
        )
    }

    #[test]
    fn flags_db_query_raw_in_controller() {
        let mut d = Vec::new();
        check_dangerous_server_builtin(
            &call(variable("db_query_raw")),
            Some("app/controllers/posts_controller.sl"),
            &mut d,
        );
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].rule, "smell/dangerous-server-builtin");
        assert!(d[0].message.contains("db_query_raw"), "{}", d[0].message);
        assert!(d[0].message.contains("@sdbql"), "{}", d[0].message);
    }

    #[test]
    fn flags_trusted_call_in_middleware() {
        let mut d = Vec::new();
        check_dangerous_server_builtin(
            &call(member(variable("Trusted"), "read")),
            Some("/home/x/app/middleware/audit.sl"),
            &mut d,
        );
        assert_eq!(d.len(), 1);
        assert!(d[0].message.contains("Trusted.*"), "{}", d[0].message);
        assert!(d[0].message.contains("File"), "{}", d[0].message);
    }

    #[test]
    fn flags_system_shell_in_view() {
        let mut d = Vec::new();
        check_dangerous_server_builtin(
            &call(member(variable("System"), "shell")),
            Some("app/views/admin/index.html.slv"),
            &mut d,
        );
        assert_eq!(d.len(), 1);
        assert!(d[0].message.contains("System.shell"), "{}", d[0].message);
        assert!(d[0].message.contains("argv"), "{}", d[0].message);
    }

    #[test]
    fn flags_system_shell_sync_in_controller() {
        let mut d = Vec::new();
        check_dangerous_server_builtin(
            &call(member(variable("System"), "shell_sync")),
            Some("app/controllers/admin_controller.sl"),
            &mut d,
        );
        assert_eq!(d.len(), 1);
        assert!(d[0].message.contains("shell_sync"), "{}", d[0].message);
    }

    #[test]
    fn flags_command_substitution_in_controller() {
        let mut d = Vec::new();
        let backtick = Expr::new(ExprKind::CommandSubstitution("ls -la".to_string()), span());
        check_dangerous_server_builtin(
            &backtick,
            Some("app/controllers/util_controller.sl"),
            &mut d,
        );
        assert_eq!(d.len(), 1);
        assert!(d[0].message.contains("argv"), "{}", d[0].message);
    }

    #[test]
    fn does_not_flag_in_models() {
        // Models legitimately use these APIs against operator-controlled
        // data; the lint stays out of `app/models/`.
        let mut d = Vec::new();
        check_dangerous_server_builtin(
            &call(variable("db_query_raw")),
            Some("app/models/post.sl"),
            &mut d,
        );
        assert!(d.is_empty());
    }

    #[test]
    fn does_not_flag_in_tests() {
        // Test fixtures often need raw SQL / shell — the lint shouldn't
        // nag tests/.
        let mut d = Vec::new();
        check_dangerous_server_builtin(
            &call(member(variable("System"), "shell")),
            Some("tests/integration_spec.sl"),
            &mut d,
        );
        assert!(d.is_empty());
    }

    #[test]
    fn does_not_flag_in_migrations() {
        let mut d = Vec::new();
        check_dangerous_server_builtin(
            &call(variable("db_query_raw")),
            Some("db/migrations/001_initial.sl"),
            &mut d,
        );
        assert!(d.is_empty());
    }

    #[test]
    fn does_not_flag_safe_calls_in_controller() {
        // Routine controller code: a regular method call shouldn't trigger
        // the rule.
        let mut d = Vec::new();
        check_dangerous_server_builtin(
            &call(member(variable("Post"), "find")),
            Some("app/controllers/posts_controller.sl"),
            &mut d,
        );
        assert!(d.is_empty());
    }

    #[test]
    fn does_not_flag_when_file_path_unknown() {
        // Linting source from stdin (no file path) shouldn't fire — we
        // can't tell which directory it would land in.
        let mut d = Vec::new();
        check_dangerous_server_builtin(&call(variable("db_query_raw")), None, &mut d);
        assert!(d.is_empty());
    }
}
