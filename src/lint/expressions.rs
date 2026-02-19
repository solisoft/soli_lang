use crate::ast::expr::{Argument, Expr, ExprKind, InterpolatedPart};

use super::rules;
use super::Linter;

impl Linter {
    pub(crate) fn lint_expr(&mut self, expr: &Expr) {
        match &expr.kind {
            ExprKind::IntLiteral(_)
            | ExprKind::FloatLiteral(_)
            | ExprKind::DecimalLiteral(_)
            | ExprKind::StringLiteral(_)
            | ExprKind::BoolLiteral(_)
            | ExprKind::Null
            | ExprKind::This
            | ExprKind::Super
            | ExprKind::Variable(_)
            | ExprKind::CommandSubstitution(_)
            | ExprKind::SdqlBlock { .. } => {}

            ExprKind::InterpolatedString(parts) => {
                for part in parts {
                    if let InterpolatedPart::Expression(inner) = part {
                        self.lint_expr(inner);
                    }
                }
            }

            ExprKind::Binary { left, right, .. } => {
                self.lint_expr(left);
                self.lint_expr(right);
            }

            ExprKind::Unary { operand, .. } => self.lint_expr(operand),

            ExprKind::Grouping(inner) => self.lint_expr(inner),

            ExprKind::Call { callee, arguments } => {
                self.lint_expr(callee);
                self.lint_arguments(arguments);
            }

            ExprKind::Pipeline { left, right } => {
                self.lint_expr(left);
                self.lint_expr(right);
            }

            ExprKind::Member { object, .. }
            | ExprKind::SafeMember { object, .. }
            | ExprKind::QualifiedName {
                qualifier: object, ..
            } => self.lint_expr(object),

            ExprKind::Index { object, index } => {
                self.lint_expr(object);
                self.lint_expr(index);
            }

            ExprKind::New {
                class_expr,
                arguments,
            } => {
                self.lint_expr(class_expr);
                self.lint_arguments(arguments);
            }

            ExprKind::Array(elements) => {
                for elem in elements {
                    self.lint_expr(elem);
                }
            }

            ExprKind::Hash(pairs) => {
                for (key, value) in pairs {
                    self.lint_expr(key);
                    self.lint_expr(value);
                }
            }

            ExprKind::Block(stmts) => {
                if stmts.is_empty() {
                    rules::style::check_empty_block(expr.span, &mut self.diagnostics);
                } else {
                    rules::smell::check_unreachable_code(stmts, &mut self.diagnostics);
                    for s in stmts {
                        self.lint_stmt(s);
                    }
                }
            }

            ExprKind::Assign { target, value } => {
                self.lint_expr(target);
                self.lint_expr(value);
            }

            ExprKind::LogicalAnd { left, right }
            | ExprKind::LogicalOr { left, right }
            | ExprKind::NullishCoalescing { left, right } => {
                self.lint_expr(left);
                self.lint_expr(right);
            }

            ExprKind::Lambda {
                params,
                return_type: _,
                body,
            } => {
                self.depth += 1;
                rules::smell::check_deep_nesting(self.depth, expr.span, &mut self.diagnostics);
                for param in params {
                    rules::naming::check_variable_name(
                        &param.name,
                        param.span,
                        &mut self.diagnostics,
                    );
                }
                self.lint_body(body);
                self.depth -= 1;
            }

            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.depth += 1;
                rules::smell::check_deep_nesting(self.depth, expr.span, &mut self.diagnostics);
                self.lint_expr(condition);
                self.lint_expr(then_branch);
                if let Some(else_b) = else_branch {
                    self.lint_expr(else_b);
                }
                self.depth -= 1;
            }

            ExprKind::Match { expression, arms } => {
                self.depth += 1;
                rules::smell::check_deep_nesting(self.depth, expr.span, &mut self.diagnostics);
                self.lint_expr(expression);
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        self.lint_expr(guard);
                    }
                    self.lint_expr(&arm.body);
                }
                self.depth -= 1;
            }

            ExprKind::ListComprehension {
                element,
                variable,
                iterable,
                condition,
            } => {
                rules::naming::check_variable_name(variable, expr.span, &mut self.diagnostics);
                self.lint_expr(element);
                self.lint_expr(iterable);
                if let Some(cond) = condition {
                    self.lint_expr(cond);
                }
            }

            ExprKind::HashComprehension {
                key,
                value,
                variable,
                iterable,
                condition,
            } => {
                rules::naming::check_variable_name(variable, expr.span, &mut self.diagnostics);
                self.lint_expr(key);
                self.lint_expr(value);
                self.lint_expr(iterable);
                if let Some(cond) = condition {
                    self.lint_expr(cond);
                }
            }

            ExprKind::Await(inner) | ExprKind::Spread(inner) | ExprKind::Throw(inner) => {
                self.lint_expr(inner);
            }
        }
    }

    fn lint_arguments(&mut self, arguments: &[Argument]) {
        for arg in arguments {
            match arg {
                Argument::Positional(expr) => self.lint_expr(expr),
                Argument::Named(named) => self.lint_expr(&named.value),
            }
        }
    }
}
