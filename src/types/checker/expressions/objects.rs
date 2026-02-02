//! Array/Hash/Class and other object type checking.

use crate::ast::expr::Argument;
use crate::ast::*;
use crate::error::TypeError;
use crate::span::Span;
use crate::types::type_repr::Type;

use super::{TypeChecker, TypeResult};

impl TypeChecker {
    /// Check array expression.
    pub(crate) fn check_array_expr(&mut self, _span: Span, elements: &[Expr]) -> TypeResult<Type> {
        if elements.is_empty() {
            return Ok(Type::Array(Box::new(Type::Unknown)));
        }

        let mut result_type = self.check_expr(&elements[0])?;
        for elem in elements.iter().skip(1) {
            let elem_type = self.check_expr(elem)?;
            // Widen to Any if types are inconsistent instead of erroring
            if !elem_type.is_assignable_to(&result_type)
                && !result_type.is_assignable_to(&elem_type)
            {
                result_type = Type::Any;
            }
        }
        Ok(Type::Array(Box::new(result_type)))
    }

    /// Check hash expression.
    pub(crate) fn check_hash_expr(
        &mut self,
        _span: Span,
        pairs: &[(Expr, Expr)],
    ) -> TypeResult<Type> {
        if pairs.is_empty() {
            return Ok(Type::Hash {
                key_type: Box::new(Type::Any),
                value_type: Box::new(Type::Any),
            });
        }

        let (first_key, first_val) = &pairs[0];
        let mut key_type = self.check_expr(first_key)?;
        let mut value_type = self.check_expr(first_val)?;

        // Check that key is a valid hashable type
        if !matches!(
            key_type,
            Type::Int | Type::Float | Type::String | Type::Bool | Type::Any | Type::Unknown
        ) {
            return Err(TypeError::General {
                message: format!("{} cannot be used as a hash key", key_type),
                span: first_key.span,
            });
        }

        // Check remaining pairs - widen to Any if types don't match (Ruby-like behavior)
        for (key_expr, val_expr) in pairs.iter().skip(1) {
            let k_type = self.check_expr(key_expr)?;
            let v_type = self.check_expr(val_expr)?;

            // If key types don't match, widen to Any
            if !k_type.is_assignable_to(&key_type) && !key_type.is_assignable_to(&k_type) {
                key_type = Type::Any;
            }
            // If value types don't match, widen to Any
            if !v_type.is_assignable_to(&value_type) && !value_type.is_assignable_to(&v_type) {
                value_type = Type::Any;
            }
        }

        Ok(Type::Hash {
            key_type: Box::new(key_type),
            value_type: Box::new(value_type),
        })
    }

    /// Check new expression (constructor call).
    pub(crate) fn check_new_expr(
        &mut self,
        _span: Span,
        _class_expr: &Expr,
        arguments: &[Argument],
    ) -> TypeResult<Type> {
        // For now, just check arguments and return an error type
        // Full type checking for qualified names would require runtime evaluation
        for arg in arguments {
            match arg {
                Argument::Positional(expr) => {
                    self.check_expr(expr)?;
                }
                Argument::Named(named) => {
                    self.check_expr(&named.value)?;
                }
            }
        }
        // Return an error type that will be resolved at runtime
        // This is a simplified approach for nested classes
        Ok(Type::Unknown)
    }

    /// Check block expression.
    pub(crate) fn check_block_expr(&mut self, statements: &[Stmt]) -> TypeResult<Type> {
        for stmt in statements {
            self.check_stmt(stmt)?;
        }
        Ok(Type::Null)
    }

    /// Check assignment expression.
    pub(crate) fn check_assign_expr(
        &mut self,
        span: Span,
        target: &Expr,
        value: &Expr,
    ) -> TypeResult<Type> {
        let target_type = self.check_expr(target)?;
        let value_type = self.check_expr(value)?;

        if !value_type.is_assignable_to(&target_type) {
            return Err(TypeError::mismatch(
                format!("{}", target_type),
                format!("{}", value_type),
                span,
            ));
        }

        Ok(target_type)
    }

    /// Check if expression.
    pub(crate) fn check_if_expr(
        &mut self,
        condition: &Expr,
        then_branch: &Expr,
        else_branch: Option<&Expr>,
    ) -> TypeResult<Type> {
        let cond_type = self.check_expr(condition)?;
        if !matches!(cond_type, Type::Bool) {
            return Err(TypeError::mismatch(
                "Bool".to_string(),
                format!("{}", cond_type),
                condition.span,
            ));
        }

        let then_type = self.check_expr(then_branch)?;

        if let Some(else_branch) = else_branch {
            let else_type = self.check_expr(else_branch)?;
            Ok(self.widen_types(&then_type, &else_type))
        } else {
            Ok(then_type)
        }
    }

    /// Check match expression.
    pub(crate) fn check_match_expr(
        &mut self,
        expression: &Expr,
        arms: &[MatchArm],
    ) -> TypeResult<Type> {
        let input_type = self.check_expr(expression)?;

        let mut arm_types = Vec::new();
        for arm in arms {
            let arm_type = self.check_match_arm(&input_type, arm)?;
            arm_types.push(arm_type);
        }

        self.common_type(&arm_types)
    }

    /// Check match arm.
    fn check_match_arm(&mut self, input_type: &Type, arm: &MatchArm) -> TypeResult<Type> {
        self.check_match_pattern(input_type, &arm.pattern)?;

        if let Some(guard) = &arm.guard {
            let guard_type = self.check_expr(guard)?;
            if !matches!(guard_type, Type::Bool) {
                return Err(TypeError::mismatch(
                    "Bool".to_string(),
                    format!("{}", guard_type),
                    guard.span,
                ));
            }
        }

        self.check_expr(&arm.body)
    }

    /// Check match pattern.
    fn check_match_pattern(&mut self, input_type: &Type, pattern: &MatchPattern) -> TypeResult<()> {
        match pattern {
            MatchPattern::Wildcard => Ok(()),

            MatchPattern::Variable(name) => {
                self.env.define(name.clone(), input_type.clone());
                Ok(())
            }

            MatchPattern::Typed { name, type_name } => {
                let expected_type = match type_name.as_str() {
                    "Int" => Type::Int,
                    "Float" => Type::Float,
                    "Bool" => Type::Bool,
                    "String" => Type::String,
                    "Void" => Type::Void,
                    _ => {
                        if let Some(class) = self.env.get_class(type_name).cloned() {
                            Type::Class(class)
                        } else {
                            return Err(TypeError::UndefinedType(
                                type_name.clone(),
                                Span::default(),
                            ));
                        }
                    }
                };

                if !input_type.is_assignable_to(&expected_type) {
                    return Err(TypeError::mismatch(
                        type_name.clone(),
                        format!("{}", input_type),
                        Span::default(),
                    ));
                }

                self.env.define(name.clone(), input_type.clone());
                Ok(())
            }

            MatchPattern::Literal(literal) => {
                let literal_type = match literal {
                    ExprKind::IntLiteral(_) => Type::Int,
                    ExprKind::FloatLiteral(_) => Type::Float,
                    ExprKind::StringLiteral(_) => Type::String,
                    ExprKind::BoolLiteral(_) => Type::Bool,
                    ExprKind::Null => Type::Null,
                    _ => Type::Any,
                };

                if !literal_type.is_assignable_to(input_type)
                    && !input_type.is_assignable_to(&literal_type)
                {
                    return Err(TypeError::mismatch(
                        format!("{}", input_type),
                        format!("{}", literal_type),
                        Span::default(),
                    ));
                }
                Ok(())
            }

            MatchPattern::Array { elements, rest: _ } => {
                // Allow Type::Any to match array patterns (e.g., untyped function parameters)
                if !matches!(input_type, Type::Array(_) | Type::Any) {
                    return Err(TypeError::mismatch(
                        "Array".to_string(),
                        format!("{}", input_type),
                        Span::default(),
                    ));
                }

                for element_pattern in elements {
                    self.check_match_pattern(input_type, element_pattern)?;
                }
                Ok(())
            }

            MatchPattern::Hash { fields, rest: _ } => {
                // Allow Type::Any to match hash patterns (e.g., untyped function parameters)
                if !matches!(input_type, Type::Hash { .. } | Type::Any) {
                    return Err(TypeError::mismatch(
                        "Hash".to_string(),
                        format!("{}", input_type),
                        Span::default(),
                    ));
                }

                for (_, field_pattern) in fields {
                    self.check_match_pattern(input_type, field_pattern)?;
                }
                Ok(())
            }

            MatchPattern::Destructuring { type_name, fields } => {
                if let Some(class) = self.env.get_class(type_name).cloned() {
                    if !input_type.is_assignable_to(&Type::Class(class.clone())) {
                        return Err(TypeError::mismatch(
                            type_name.clone(),
                            format!("{}", input_type),
                            Span::default(),
                        ));
                    }

                    for (_, field_pattern) in fields {
                        self.check_match_pattern(input_type, field_pattern)?;
                    }
                    Ok(())
                } else {
                    Err(TypeError::UndefinedType(type_name.clone(), Span::default()))
                }
            }

            MatchPattern::And(patterns) => {
                for pattern in patterns {
                    self.check_match_pattern(input_type, pattern)?;
                }
                Ok(())
            }

            MatchPattern::Or(patterns) => {
                for pattern in patterns {
                    self.check_match_pattern(input_type, pattern)?;
                }
                Ok(())
            }
        }
    }

    /// Check list comprehension expression.
    pub(crate) fn check_list_comprehension(
        &mut self,
        _element: &Expr,
        variable: &str,
        iterable: &Expr,
        condition: Option<&Expr>,
    ) -> TypeResult<Type> {
        // Type check the iterable
        let _iter_type = self.check_expr(iterable)?;

        // Define the loop variable in the environment (as Any type)
        self.env.define(variable.to_string(), Type::Any);

        // Type check the condition if present
        if let Some(cond) = condition {
            let cond_type = self.check_expr(cond)?;
            if !matches!(cond_type, Type::Bool) {
                return Err(TypeError::mismatch(
                    "Bool".to_string(),
                    format!("{}", cond_type),
                    cond.span,
                ));
            }
        }

        // Return Array of the element type
        Ok(Type::Array(Box::new(Type::Any)))
    }

    /// Check hash comprehension expression.
    pub(crate) fn check_hash_comprehension(
        &mut self,
        _key: &Expr,
        _value: &Expr,
        variable: &str,
        iterable: &Expr,
        condition: Option<&Expr>,
    ) -> TypeResult<Type> {
        // Type check the iterable
        let _iter_type = self.check_expr(iterable)?;

        // Define the loop variable in the environment (as Any type)
        self.env.define(variable.to_string(), Type::Any);

        // Type check the condition if present
        if let Some(cond) = condition {
            let cond_type = self.check_expr(cond)?;
            if !matches!(cond_type, Type::Bool) {
                return Err(TypeError::mismatch(
                    "Bool".to_string(),
                    format!("{}", cond_type),
                    cond.span,
                ));
            }
        }

        // Return Hash with Any key and value types
        Ok(Type::Hash {
            key_type: Box::new(Type::Any),
            value_type: Box::new(Type::Any),
        })
    }

    /// Check spread expression.
    pub(crate) fn check_spread_expr(&mut self, inner: &Expr) -> TypeResult<Type> {
        // Spread takes an array and returns its element type (in context)
        // For now, just check the inner expression and return Array type
        let _inner_type = self.check_expr(inner)?;
        // In array context, spread returns the element type of the array
        // We return Any since we can't easily determine the element type
        Ok(Type::Any)
    }
}
