//! Function/method call type checking.

use crate::ast::expr::Argument;
use crate::ast::*;
use crate::error::TypeError;
use crate::span::Span;
use crate::types::type_repr::Type;

use super::{TypeChecker, TypeResult};

impl TypeChecker {
    /// Check call expression.
    pub(crate) fn check_call_expr(
        &mut self,
        span: Span,
        callee: &Expr,
        arguments: &[Argument],
    ) -> TypeResult<Type> {
        let callee_type = self.check_expr(callee)?;

        match callee_type {
            Type::Function {
                params,
                return_type,
            } => {
                let total_args = arguments.len();

                // Check argument count (allow fewer args for default parameters)
                // Note: We only check upper bound since we can't easily know which params have defaults
                // The runtime will handle default parameter filling
                if total_args > params.len() && !params.iter().any(|p| matches!(p, Type::Any)) {
                    return Err(TypeError::WrongArity {
                        expected: params.len(),
                        got: total_args,
                        span,
                    });
                }

                // Get parameter names for named argument validation
                // For now, we use index-based matching since we can't resolve callee name
                let _param_count = params.len();
                let param_names: Vec<String> = params
                    .iter()
                    .enumerate()
                    .map(|(i, _)| format!("param_{}", i))
                    .collect();

                // Check argument types
                for (i, arg) in arguments.iter().enumerate() {
                    match arg {
                        Argument::Positional(expr) => {
                            let arg_type = self.check_expr(expr)?;
                            if let Some(param_type) = params.get(i) {
                                // Allow Any param type to accept any argument (for map/filter/each)
                                if matches!(param_type, Type::Any) {
                                    continue;
                                }
                                if !arg_type.is_assignable_to(param_type) {
                                    return Err(TypeError::mismatch(
                                        format!("{}", param_type),
                                        format!("{}", arg_type),
                                        expr.span,
                                    ));
                                }
                            }
                        }
                        Argument::Named(named) => {
                            // For named arguments, find the corresponding parameter
                            if let Some(param_idx) =
                                param_names.iter().position(|n| n == &named.name)
                            {
                                if let Some(param_type) = params.get(param_idx) {
                                    let arg_type = self.check_expr(&named.value)?;
                                    if matches!(param_type, Type::Any) {
                                        continue;
                                    }
                                    if !arg_type.is_assignable_to(param_type) {
                                        return Err(TypeError::mismatch(
                                            format!("{}", param_type),
                                            format!("{}", arg_type),
                                            named.span,
                                        ));
                                    }
                                }
                            }
                            // Unknown named argument - runtime will catch this
                        }
                    }
                }

                Ok(*return_type)
            }
            Type::Class(class) => {
                // Constructor call
                if let Some(ref ctor) = self
                    .env
                    .get_class(&class.name)
                    .and_then(|c| c.methods.get("new").cloned())
                {
                    if ctor.params.len() != arguments.len() {
                        return Err(TypeError::WrongArity {
                            expected: ctor.params.len(),
                            got: arguments.len(),
                            span,
                        });
                    }
                }
                Ok(Type::Class(class))
            }
            Type::Any | Type::Unknown => Ok(Type::Any),
            _ => Err(TypeError::NotCallable(format!("{}", callee_type), span)),
        }
    }

    /// Check pipeline expression.
    pub(crate) fn check_pipeline_expr(&mut self, left: &Expr, right: &Expr) -> TypeResult<Type> {
        let left_type = self.check_expr(left)?;

        // Right side can be a call or a function value
        match &right.kind {
            ExprKind::Call { callee, arguments } => {
                let callee_type = self.check_expr(callee)?;

                match callee_type {
                    Type::Function {
                        params,
                        return_type,
                    } => {
                        // First param should match left_type
                        if let Some(first_param) = params.first() {
                            if !left_type.is_assignable_to(first_param) {
                                return Err(TypeError::mismatch(
                                    format!("{}", first_param),
                                    format!("{}", left_type),
                                    left.span,
                                ));
                            }
                        }

                        // Check remaining arguments
                        for (i, arg) in arguments.iter().enumerate() {
                            match arg {
                                Argument::Positional(expr) => {
                                    let arg_type = self.check_expr(expr)?;
                                    if let Some(param_type) = params.get(i + 1) {
                                        if !arg_type.is_assignable_to(param_type) {
                                            return Err(TypeError::mismatch(
                                                format!("{}", param_type),
                                                format!("{}", arg_type),
                                                expr.span,
                                            ));
                                        }
                                    }
                                }
                                Argument::Named(_) => {
                                    // Named arguments in pipeline - skip type checking
                                    // Runtime will validate these
                                }
                            }
                        }

                        Ok(*return_type)
                    }
                    Type::Any | Type::Unknown => Ok(Type::Any),
                    _ => Err(TypeError::NotCallable(
                        format!("{}", callee_type),
                        right.span,
                    )),
                }
            }
            _ => {
                // Try evaluating right as a function value
                let right_type = self.check_expr(right)?;
                match right_type {
                    Type::Function {
                        params,
                        return_type,
                    } => {
                        // Must have at least one parameter
                        if params.is_empty() {
                            return Err(TypeError::General {
                                message:
                                    "pipeline right-hand function must take at least one argument"
                                        .to_string(),
                                span: right.span,
                            });
                        }

                        // First param should match left_type
                        if !left_type.is_assignable_to(&params[0]) {
                            return Err(TypeError::mismatch(
                                format!("{}", params[0]),
                                format!("{}", left_type),
                                left.span,
                            ));
                        }

                        // Remaining params must be optional or have default values?
                        // For now, we only support 1-arg functions if they are values.
                        if params.len() > 1 {
                            return Err(TypeError::General {
                                message: "pipeline right-hand function value must take exactly one argument"
                                    .to_string(),
                                span: right.span,
                            });
                        }

                        Ok(*return_type)
                    }
                    Type::Any | Type::Unknown => Ok(Type::Any),
                    _ => Err(TypeError::General {
                        message:
                            "right side of pipeline must be a function call or a function value"
                                .to_string(),
                        span: right.span,
                    }),
                }
            }
        }
    }
}
