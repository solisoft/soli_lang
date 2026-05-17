//! Expression printer.

use crate::ast::expr::{
    Argument, BinaryOp, CompoundOp, Expr, ExprKind, InterpolatedPart, MatchArm, MatchPattern,
    UnaryOp,
};

use super::printer::Printer;

impl Printer<'_> {
    pub(super) fn print_expr(&mut self, expr: &Expr) {
        match &expr.kind {
            ExprKind::IntLiteral(n) => self.write(&n.to_string()),
            ExprKind::FloatLiteral(n) => {
                let s = format!("{}", n);
                self.write(&s);
                if !s.contains('.') && !s.contains('e') && !s.contains('E') {
                    self.write(".0");
                }
            }
            ExprKind::DecimalLiteral(s) => {
                self.write(s);
                if !s.ends_with('D') && !s.ends_with('d') {
                    self.write("D");
                }
            }
            ExprKind::StringLiteral(s) => {
                self.write("\"");
                for c in s.chars() {
                    match c {
                        '\\' => self.write("\\\\"),
                        '"' => self.write("\\\""),
                        '\n' => self.write("\\n"),
                        '\r' => self.write("\\r"),
                        '\t' => self.write("\\t"),
                        c => {
                            let mut buf = [0u8; 4];
                            self.write(c.encode_utf8(&mut buf));
                        }
                    }
                }
                self.write("\"");
            }
            ExprKind::InterpolatedString(parts) => {
                self.write("\"");
                for part in parts {
                    match part {
                        InterpolatedPart::Literal(s) => {
                            for c in s.chars() {
                                match c {
                                    '\\' => self.write("\\\\"),
                                    '"' => self.write("\\\""),
                                    '\n' => self.write("\\n"),
                                    '\r' => self.write("\\r"),
                                    '\t' => self.write("\\t"),
                                    c => {
                                        let mut buf = [0u8; 4];
                                        self.write(c.encode_utf8(&mut buf));
                                    }
                                }
                            }
                        }
                        InterpolatedPart::Expression(e) => {
                            self.write("#{");
                            self.print_expr(e);
                            self.write("}");
                        }
                    }
                }
                self.write("\"");
            }
            ExprKind::CommandSubstitution(s) => {
                self.write("`");
                self.write(s);
                self.write("`");
            }
            // Not yet specially formatted — copy the original source bytes
            // verbatim so semantics are preserved. The lint/runtime layers
            // round-trip these correctly.
            ExprKind::SdqlBlock { .. } => {
                self.write_source_span(expr.span.start, expr.span.end);
            }
            ExprKind::BoolLiteral(b) => self.write(if *b { "true" } else { "false" }),
            ExprKind::Symbol(name) => {
                self.write(":");
                self.write(name);
            }
            ExprKind::Null => self.write("null"),
            ExprKind::Variable(name) => self.write(name),
            ExprKind::This => self.write("this"),
            ExprKind::Super => self.write("super"),
            ExprKind::Binary {
                left,
                operator,
                right,
            } => {
                self.print_expr(left);
                self.write(" ");
                self.write(&binary_op_str(*operator));
                self.write(" ");
                self.print_expr(right);
            }
            ExprKind::Unary { operator, operand } => {
                let op = match operator {
                    UnaryOp::Negate => "-",
                    UnaryOp::Not => "!",
                };
                self.write(op);
                self.print_expr(operand);
            }
            ExprKind::Grouping(inner) => {
                self.write("(");
                self.print_expr(inner);
                self.write(")");
            }
            ExprKind::Call { callee, arguments } => {
                self.print_expr(callee);
                // Class-body bare DSL macros (`soft_delete`) and command-style
                // calls (`puts "hi"`) are parsed into a Call AST without parens
                // in the source. The parser rejects parens around those forms,
                // so we must preserve "no parens" when the source had none.
                if arguments.is_empty() && !source_has_parens_after(self.source, callee.span.end) {
                    // bare call with no args and no source parens — skip "()"
                } else {
                    self.print_arg_list(arguments);
                }
            }
            ExprKind::Pipeline { left, right } => {
                self.print_expr(left);
                self.write(" |> ");
                self.print_expr(right);
            }
            ExprKind::Member { object, name } => {
                self.print_expr(object);
                self.write(".");
                self.write(name);
            }
            ExprKind::SafeMember { object, name } => {
                self.print_expr(object);
                self.write("&.");
                self.write(name);
            }
            ExprKind::QualifiedName { qualifier, name } => {
                self.print_expr(qualifier);
                self.write("::");
                self.write(name);
            }
            ExprKind::Index { object, index } => {
                self.print_expr(object);
                self.write("[");
                self.print_expr(index);
                self.write("]");
            }
            ExprKind::New {
                class_expr,
                arguments,
            } => {
                self.write("new ");
                self.print_expr(class_expr);
                self.print_arg_list(arguments);
            }
            ExprKind::Array(elements) => {
                self.write("[");
                // Avoid `[[` as the first two characters — Soli's lexer
                // treats `[[X...` (where X is not a digit/minus/[) as a
                // Lua-style multiline string. Adding a space before the
                // nested `[` disambiguates.
                let first_is_array = elements
                    .first()
                    .map(|e| matches!(e.kind, ExprKind::Array(_)))
                    .unwrap_or(false);
                if first_is_array {
                    self.write(" ");
                }
                for (i, e) in elements.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.print_expr(e);
                }
                self.write("]");
            }
            ExprKind::Hash(pairs) => {
                if pairs.is_empty() {
                    self.write("{}");
                } else {
                    self.write("{");
                    for (i, (k, v)) in pairs.iter().enumerate() {
                        if i > 0 {
                            self.write(", ");
                        }
                        self.print_expr(k);
                        self.write(": ");
                        self.print_expr(v);
                    }
                    self.write("}");
                }
            }
            ExprKind::Block(stmts) => {
                self.write("{");
                self.newline();
                self.print_block_body(stmts);
                self.write("}");
            }
            ExprKind::Assign { target, value } => {
                self.print_expr(target);
                self.write(" = ");
                self.print_expr(value);
            }
            ExprKind::CompoundAssign {
                target,
                operator,
                value,
            } => {
                self.print_expr(target);
                self.write(" ");
                self.write(&compound_op_str(*operator));
                self.write(" ");
                self.print_expr(value);
            }
            ExprKind::PostfixIncrement(inner) => {
                self.print_expr(inner);
                self.write("++");
            }
            ExprKind::PostfixDecrement(inner) => {
                self.print_expr(inner);
                self.write("--");
            }
            ExprKind::LogicalAnd { left, right } => {
                self.print_expr(left);
                self.write(" && ");
                self.print_expr(right);
            }
            ExprKind::LogicalOr { left, right } => {
                self.print_expr(left);
                self.write(" || ");
                self.print_expr(right);
            }
            ExprKind::NullishCoalescing { left, right } => {
                self.print_expr(left);
                self.write(" ?? ");
                self.print_expr(right);
            }
            ExprKind::Lambda {
                params,
                return_type,
                body,
            } => {
                // Prefer `fn(params) { body }` for non-block-param lambdas.
                self.write("fn");
                self.print_param_list(params);
                if let Some(ret) = return_type {
                    self.write(" -> ");
                    self.write(&ret.to_string());
                }
                self.write(" {");
                if body.len() == 1 {
                    // Inline single-expression bodies: `fn(x) { x * 2 }`. We
                    // skip the inline form when the inner expression is
                    // itself a Lambda — at statement position inside the
                    // outer body, an implicit `fn(...)` would be parsed as a
                    // function declaration (which needs a name) and fail.
                    // Fall through to the multi-line form, which keeps the
                    // `return` keyword and prints via `print_block_body`.
                    let inner_is_lambda = match &body[0].kind {
                        crate::ast::stmt::StmtKind::Expression(e)
                        | crate::ast::stmt::StmtKind::Return(Some(e)) => {
                            matches!(e.kind, ExprKind::Lambda { .. })
                        }
                        _ => false,
                    };
                    if !inner_is_lambda {
                        if let crate::ast::stmt::StmtKind::Expression(e) = &body[0].kind {
                            self.write(" ");
                            self.print_expr(e);
                            self.write(" }");
                            return;
                        }
                        if let crate::ast::stmt::StmtKind::Return(Some(e)) = &body[0].kind {
                            self.write(" ");
                            self.print_expr(e);
                            self.write(" }");
                            return;
                        }
                    }
                }
                self.newline();
                let close_line = super::printer::source_end_line(self.source, expr.span);
                self.print_block_body_through(body, Some(close_line));
                self.write("}");
            }
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                // Ternary `cond ? then : else` — the only surface syntax
                // that produces ExprKind::If (see parser/expressions.rs).
                self.print_expr(condition);
                self.write(" ? ");
                self.print_expr(then_branch);
                self.write(" : ");
                if let Some(eb) = else_branch {
                    self.print_expr(eb);
                } else {
                    self.write("null");
                }
            }
            ExprKind::Match { expression, arms } => {
                self.write("match ");
                self.print_expr(expression);
                self.write(" {");
                self.newline();
                self.with_indent(|p| {
                    for arm in arms {
                        p.print_match_arm(arm);
                    }
                });
                self.write("}");
            }
            ExprKind::ListComprehension { .. } | ExprKind::HashComprehension { .. } => {
                self.write_source_span(expr.span.start, expr.span.end);
            }
            ExprKind::Await(inner) => {
                self.write("await ");
                self.print_expr(inner);
            }
            ExprKind::Spread(inner) => {
                self.write("...");
                self.print_expr(inner);
            }
            ExprKind::Throw(inner) => {
                self.write("throw ");
                self.print_expr(inner);
            }
            ExprKind::Rescue { expr, fallback } => {
                self.print_expr(expr);
                self.write(" rescue ");
                self.print_expr(fallback);
            }
        }
    }

    fn print_arg_list(&mut self, args: &[Argument]) {
        self.write("(");
        for (i, a) in args.iter().enumerate() {
            if i > 0 {
                self.write(", ");
            }
            match a {
                Argument::Positional(e) => self.print_expr(e),
                Argument::Named(na) => {
                    self.write(&na.name);
                    self.write(": ");
                    self.print_expr(&na.value);
                }
                Argument::Block(e) => {
                    // Soli's `&` block-arg only accepts `&{ body }`,
                    // `&(params) body`, `&:method`, or `&identifier` — NOT
                    // `&fn(...)`. Re-emit a Lambda block as `&{ |params| body }`
                    // (the trailing-brace-block form), and fall back to a
                    // bare `&expr` for variable references or other shapes.
                    self.write("&");
                    match &e.kind {
                        ExprKind::Lambda { params, body, .. } => {
                            self.write("{");
                            if !params.is_empty() {
                                self.write(" |");
                                for (i, p) in params.iter().enumerate() {
                                    if i > 0 {
                                        self.write(", ");
                                    }
                                    if p.is_block_param {
                                        self.write("&");
                                    }
                                    self.write(&p.name);
                                }
                                self.write("|");
                            }
                            if body.is_empty() {
                                self.write(" }");
                            } else if body.len() == 1 {
                                // Single-statement body — keep it on one line
                                // when it's an expression: `&{ |x| x * 2 }`.
                                self.write(" ");
                                if let crate::ast::stmt::StmtKind::Expression(expr) = &body[0].kind
                                {
                                    self.print_expr(expr);
                                } else {
                                    self.print_stmt(&body[0]);
                                }
                                self.write(" }");
                            } else {
                                self.newline();
                                self.print_block_body(body);
                                self.write("}");
                            }
                        }
                        _ => self.print_expr(e),
                    }
                }
            }
        }
        self.write(")");
    }

    fn print_match_arm(&mut self, arm: &MatchArm) {
        self.print_match_pattern(&arm.pattern);
        if let Some(g) = &arm.guard {
            self.write(" if ");
            self.print_expr(g);
        }
        self.write(" => ");
        self.print_expr(&arm.body);
        self.write(",");
        self.newline();
    }

    fn print_match_pattern(&mut self, p: &MatchPattern) {
        match p {
            MatchPattern::Wildcard => self.write("_"),
            MatchPattern::Variable(name) => self.write(name),
            MatchPattern::Typed { name, type_name } => {
                self.write(name);
                self.write(": ");
                self.write(type_name);
            }
            MatchPattern::Literal(kind) => {
                // Re-wrap into a temporary Expr to reuse the literal printer.
                let tmp_expr = Expr {
                    kind: kind.clone(),
                    span: crate::span::Span::new(0, 0, 0, 0),
                };
                self.print_expr(&tmp_expr);
            }
            MatchPattern::Array { elements, rest } => {
                self.write("[");
                for (i, el) in elements.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.print_match_pattern(el);
                }
                if let Some(rest_name) = rest {
                    if !elements.is_empty() {
                        self.write(", ");
                    }
                    self.write("...");
                    self.write(rest_name);
                }
                self.write("]");
            }
            MatchPattern::Hash { fields, rest } => {
                self.write("{");
                for (i, (name, pat)) in fields.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.write(name);
                    self.write(": ");
                    self.print_match_pattern(pat);
                }
                if let Some(rest_name) = rest {
                    if !fields.is_empty() {
                        self.write(", ");
                    }
                    self.write("...");
                    self.write(rest_name);
                }
                self.write("}");
            }
            MatchPattern::Destructuring { type_name, fields } => {
                self.write(type_name);
                self.write(" { ");
                for (i, (name, pat)) in fields.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.write(name);
                    self.write(": ");
                    self.print_match_pattern(pat);
                }
                self.write(" }");
            }
            MatchPattern::And(pats) => {
                for (i, pat) in pats.iter().enumerate() {
                    if i > 0 {
                        self.write(" & ");
                    }
                    self.print_match_pattern(pat);
                }
            }
            MatchPattern::Or(pats) => {
                for (i, pat) in pats.iter().enumerate() {
                    if i > 0 {
                        self.write(" | ");
                    }
                    self.print_match_pattern(pat);
                }
            }
        }
    }
}

fn binary_op_str(op: BinaryOp) -> String {
    op.to_string()
}

fn compound_op_str(op: CompoundOp) -> String {
    op.to_string()
}

/// Inspect source bytes starting at `at` (the position right after a call's
/// callee). Returns true if the first non-whitespace byte is `(` — i.e., the
/// source spelled the call with parens. Used to preserve paren-less DSL forms
/// like `class TestSoft < Model\n  soft_delete\nend` that the parser rejects
/// when surrounded by parens.
fn source_has_parens_after(source: &str, at: usize) -> bool {
    let bytes = source.as_bytes();
    let mut i = at.min(bytes.len());
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    bytes.get(i) == Some(&b'(')
}
