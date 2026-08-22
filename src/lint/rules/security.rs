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

use crate::ast::expr::{Argument, Expr, ExprKind};
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

/// `security/unfiltered-mass-assignment` — `Model.create(params)` (and
/// `update` / `create_many`) in a controller persist every posted key.
/// Scaffolded apps whitelist through `permit` / `_permit_params`; this
/// flags the unfiltered shape.
pub fn check_unfiltered_mass_assignment(
    expr: &Expr,
    file_path: Option<&str>,
    diagnostics: &mut Vec<LintDiagnostic>,
) {
    let Some(file) = file_path else {
        return;
    };
    if !is_controller_or_service_path(file) {
        return;
    }
    let ExprKind::Call { callee, arguments } = &expr.kind else {
        return;
    };
    let ExprKind::Member { name: method, .. } = &callee.kind else {
        return;
    };
    let Some(payload) = mass_assignment_payload(method, arguments) else {
        return;
    };
    if !is_raw_request_payload(payload) {
        return;
    }
    diagnostics.push(LintDiagnostic {
        rule: "security/unfiltered-mass-assignment",
        message: format!(
            "`{method}(...)` is given the raw request params — unlisted keys \
             persist. Whitelist with `permit(params, {{ \"field\": true }})` \
             or `this._permit_params(params)` first"
        ),
        span: payload.span,
        severity: Severity::Warning,
    });
}

fn is_controller_or_service_path(file: &str) -> bool {
    let normalised = file.replace('\\', "/");
    let dirs = ["app/controllers/", "app/services/"];
    dirs.iter()
        .any(|d| normalised.contains(&format!("/{}", d)) || normalised.starts_with(d))
}

fn mass_assignment_payload<'a>(method: &str, arguments: &'a [Argument]) -> Option<&'a Expr> {
    let positional: Vec<&Expr> = arguments
        .iter()
        .filter_map(|a| match a {
            Argument::Positional(e) => Some(e),
            _ => None,
        })
        .collect();
    match method {
        "create" | "create_many" => positional.first().copied(),
        "update" => {
            // Class form `Model.update(id, hash)` — second arg.
            // Instance form `record.update(hash)` — first arg.
            if positional.len() >= 2 {
                Some(positional[1])
            } else {
                positional.first().copied()
            }
        }
        _ => None,
    }
}

/// True when `expr` is the whole (or a subscript of the) request params
/// hash, and has not been wrapped in `permit` / `_permit_params`.
fn is_raw_request_payload(expr: &Expr) -> bool {
    if is_permit_wrap(expr) {
        return false;
    }
    match &expr.kind {
        ExprKind::Variable(name) if name == "params" || name == "json" => true,
        ExprKind::Member { object, name } => match &object.kind {
            ExprKind::Variable(obj) if is_request_object(obj) && is_params_field(name) => true,
            _ => is_raw_request_payload(object),
        },
        ExprKind::Index { object, index } => {
            if let ExprKind::Variable(obj) = &object.kind {
                if is_request_object(obj) {
                    if let ExprKind::StringLiteral(key) = &index.kind {
                        return is_params_field(key);
                    }
                }
            }
            is_raw_request_payload(object)
        }
        _ => false,
    }
}

fn is_request_object(name: &str) -> bool {
    name == "req" || name == "request"
}

fn is_params_field(name: &str) -> bool {
    name == "params" || name == "json" || name == "body"
}

fn is_permit_wrap(expr: &Expr) -> bool {
    let ExprKind::Call { callee, .. } = &expr.kind else {
        return false;
    };
    match &callee.kind {
        ExprKind::Variable(name) if name == "permit" => true,
        ExprKind::Member { name, .. } if name == "permit" || name == "_permit_params" => true,
        _ => false,
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

    fn call_args(callee: Expr, args: Vec<Expr>) -> Expr {
        Expr::new(
            ExprKind::Call {
                callee: Box::new(callee),
                arguments: args.into_iter().map(Argument::Positional).collect(),
            },
            span(),
        )
    }

    fn index(object: Expr, key: &str) -> Expr {
        Expr::new(
            ExprKind::Index {
                object: Box::new(object),
                index: Box::new(Expr::new(ExprKind::StringLiteral(key.to_string()), span())),
            },
            span(),
        )
    }

    #[test]
    fn flags_create_params_in_controller() {
        let mut d = Vec::new();
        let expr = call_args(member(variable("Post"), "create"), vec![variable("params")]);
        check_unfiltered_mass_assignment(
            &expr,
            Some("app/controllers/posts_controller.sl"),
            &mut d,
        );
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].rule, "security/unfiltered-mass-assignment");
        assert!(d[0].message.contains("permit"), "{}", d[0].message);
    }

    #[test]
    fn flags_create_req_json() {
        let mut d = Vec::new();
        let expr = call_args(
            member(variable("Post"), "create"),
            vec![index(variable("req"), "json")],
        );
        check_unfiltered_mass_assignment(
            &expr,
            Some("app/controllers/posts_controller.sl"),
            &mut d,
        );
        assert_eq!(d.len(), 1);
    }

    #[test]
    fn flags_class_update_second_arg() {
        let mut d = Vec::new();
        let expr = call_args(
            member(variable("Post"), "update"),
            vec![variable("id"), variable("params")],
        );
        check_unfiltered_mass_assignment(
            &expr,
            Some("app/controllers/posts_controller.sl"),
            &mut d,
        );
        assert_eq!(d.len(), 1);
    }

    #[test]
    fn flags_instance_update() {
        let mut d = Vec::new();
        let expr = call_args(member(variable("post"), "update"), vec![variable("params")]);
        check_unfiltered_mass_assignment(
            &expr,
            Some("app/controllers/posts_controller.sl"),
            &mut d,
        );
        assert_eq!(d.len(), 1);
    }

    #[test]
    fn does_not_flag_permit_wrap() {
        let mut d = Vec::new();
        let permitted = call_args(variable("permit"), vec![variable("params")]);
        let expr = call_args(member(variable("Post"), "create"), vec![permitted]);
        check_unfiltered_mass_assignment(
            &expr,
            Some("app/controllers/posts_controller.sl"),
            &mut d,
        );
        assert!(d.is_empty());
    }

    #[test]
    fn does_not_flag_permit_params_helper() {
        let mut d = Vec::new();
        let permitted = call_args(
            member(Expr::new(ExprKind::This, span()), "_permit_params"),
            vec![variable("params")],
        );
        let expr = call_args(member(variable("Post"), "create"), vec![permitted]);
        check_unfiltered_mass_assignment(
            &expr,
            Some("app/controllers/posts_controller.sl"),
            &mut d,
        );
        assert!(d.is_empty());
    }

    #[test]
    fn does_not_flag_hash_literal() {
        let mut d = Vec::new();
        let hash = Expr::new(ExprKind::Hash(vec![]), span());
        let expr = call_args(member(variable("Post"), "create"), vec![hash]);
        check_unfiltered_mass_assignment(
            &expr,
            Some("app/controllers/posts_controller.sl"),
            &mut d,
        );
        assert!(d.is_empty());
    }

    #[test]
    fn does_not_flag_create_in_models() {
        let mut d = Vec::new();
        let expr = call_args(member(variable("Post"), "create"), vec![variable("params")]);
        check_unfiltered_mass_assignment(&expr, Some("app/models/post.sl"), &mut d);
        assert!(d.is_empty());
    }
}
