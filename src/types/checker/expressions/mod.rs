//! Expression type checking modules.

mod access;
mod calls;
mod control;
mod literals;
mod objects;
mod operators;
mod variables;

use crate::ast::*;
use crate::types::type_repr::Type;

use super::{TypeChecker, TypeResult};

impl TypeChecker {
    /// Main expression type checker - dispatches to specialized checkers.
    pub(crate) fn check_expr(&mut self, expr: &Expr) -> TypeResult<Type> {
        match &expr.kind {
            // Literals
            ExprKind::IntLiteral(_)
            | ExprKind::FloatLiteral(_)
            | ExprKind::DecimalLiteral(_)
            | ExprKind::StringLiteral(_)
            | ExprKind::BoolLiteral(_)
            | ExprKind::Null => self.check_literal(&expr.kind),

            // Variables and references
            ExprKind::Variable(name) => self.check_variable(name, expr.span),
            ExprKind::QualifiedName { .. } => self.check_qualified_name(expr.span),
            ExprKind::Grouping(inner) => self.check_grouping(inner),

            // Operators
            ExprKind::Binary {
                left,
                operator,
                right,
            } => self.check_binary_expr(expr.span, left, operator, right),
            ExprKind::Unary { operator, operand } => {
                self.check_unary_expr(expr.span, operator, operand)
            }
            ExprKind::LogicalAnd { left, right } | ExprKind::LogicalOr { left, right } => {
                self.check_logical(left, right)
            }
            ExprKind::NullishCoalescing { left, right } => {
                self.check_nullish_coalescing(left, right)
            }

            // Calls
            ExprKind::Call { callee, arguments } => {
                self.check_call_expr(expr.span, callee, arguments)
            }
            ExprKind::Pipeline { left, right } => self.check_pipeline_expr(left, right),

            // Access
            ExprKind::Member { object, name } => self.check_member_expr(expr.span, object, name),
            ExprKind::Index { object, index } => self.check_index_expr(expr.span, object, index),

            // Objects and collections
            ExprKind::New {
                class_expr,
                arguments,
            } => self.check_new_expr(expr.span, class_expr, arguments),
            ExprKind::Array(elements) => self.check_array_expr(expr.span, elements),
            ExprKind::Hash(pairs) => self.check_hash_expr(expr.span, pairs),
            ExprKind::Block(statements) => self.check_block_expr(statements),
            ExprKind::Assign { target, value } => self.check_assign_expr(expr.span, target, value),
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => self.check_if_expr(condition, then_branch, else_branch.as_deref()),
            ExprKind::Match { expression, arms } => self.check_match_expr(expression, arms),
            ExprKind::ListComprehension {
                element,
                variable,
                iterable,
                condition,
            } => self.check_list_comprehension(element, variable, iterable, condition.as_deref()),
            ExprKind::HashComprehension {
                key,
                value,
                variable,
                iterable,
                condition,
            } => {
                self.check_hash_comprehension(key, value, variable, iterable, condition.as_deref())
            }
            ExprKind::Spread(inner) => self.check_spread_expr(inner),

            // Control flow
            ExprKind::This => self.check_this_expr(expr.span),
            ExprKind::Super => self.check_super_expr(expr.span),
            ExprKind::Lambda {
                params,
                return_type,
                body,
            } => self.check_lambda_expr(body, params, return_type),
            ExprKind::Await(inner) => self.check_await_expr(inner),
            ExprKind::Throw(inner) => self.check_throw_expr(inner),

            // String interpolation
            ExprKind::InterpolatedString(parts) => self.check_interpolated_string(parts),
        }
    }

    /// Compute common type from a list of types.
    fn common_type(&self, types: &[Type]) -> TypeResult<Type> {
        if types.is_empty() {
            return Ok(Type::Any);
        }

        let mut result = types[0].clone();
        for t in &types[1..] {
            result = self.widen_types(&result, t);
        }

        Ok(result)
    }

    /// Widen two types to a common supertype.
    #[allow(clippy::only_used_in_recursion)]
    fn widen_types(&self, a: &Type, b: &Type) -> Type {
        if a == b {
            return a.clone();
        }

        match (a, b) {
            (Type::Any, _) | (_, Type::Any) => Type::Any,
            (Type::Int, Type::Float) | (Type::Float, Type::Int) => Type::Float,
            (Type::Array(a_elem), Type::Array(b_elem)) => {
                Type::Array(Box::new(self.widen_types(a_elem, b_elem)))
            }
            (
                Type::Hash {
                    key_type: a_key,
                    value_type: a_val,
                },
                Type::Hash {
                    key_type: b_key,
                    value_type: b_val,
                },
            ) => Type::Hash {
                key_type: Box::new(self.widen_types(a_key, b_key)),
                value_type: Box::new(self.widen_types(a_val, b_val)),
            },
            _ => Type::Any,
        }
    }
}
