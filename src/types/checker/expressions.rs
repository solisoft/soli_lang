//! Expression type checking.

use crate::ast::*;
use crate::error::TypeError;
use crate::span::Span;
use crate::types::type_repr::Type;

use super::{TypeChecker, TypeResult};

impl TypeChecker {
    pub(crate) fn check_expr(&mut self, expr: &Expr) -> TypeResult<Type> {
        match &expr.kind {
            ExprKind::IntLiteral(_) => Ok(Type::Int),
            ExprKind::FloatLiteral(_) => Ok(Type::Float),
            ExprKind::StringLiteral(_) => Ok(Type::String),
            ExprKind::BoolLiteral(_) => Ok(Type::Bool),
            ExprKind::Null => Ok(Type::Null),

            ExprKind::Variable(name) => self
                .env
                .get(name)
                .ok_or_else(|| TypeError::UndefinedVariable(name.clone(), expr.span)),

            ExprKind::Grouping(inner) => self.check_expr(inner),

            ExprKind::Binary {
                left,
                operator,
                right,
            } => self.check_binary_expr(expr, left, operator, right),

            ExprKind::Unary { operator, operand } => self.check_unary_expr(expr, operator, operand),

            ExprKind::LogicalAnd { left, right } | ExprKind::LogicalOr { left, right } => {
                self.check_expr(left)?;
                self.check_expr(right)?;
                Ok(Type::Bool)
            }

            ExprKind::NullishCoalescing { left, right } => {
                self.check_expr(left)?;
                let right_type = self.check_expr(right)?;
                // The result type is the right type (since if left is null, we return right)
                Ok(right_type)
            }

            ExprKind::Call { callee, arguments } => self.check_call_expr(expr, callee, arguments),

            ExprKind::Pipeline { left, right } => self.check_pipeline_expr(left, right),

            ExprKind::Member { object, name } => self.check_member_expr(expr, object, name),

            ExprKind::Index { object, index } => self.check_index_expr(expr, object, index),

            ExprKind::This => {
                if let Some(class_type) = self.env.current_class_type() {
                    Ok(Type::Class(class_type.clone()))
                } else {
                    Err(TypeError::ThisOutsideClass(expr.span))
                }
            }

            ExprKind::Super => {
                if let Some(class_type) = self.env.current_class_type() {
                    if let Some(ref superclass) = class_type.superclass {
                        Ok(Type::Class(*superclass.clone()))
                    } else {
                        Err(TypeError::NoSuperclass(class_type.name.clone(), expr.span))
                    }
                } else {
                    Err(TypeError::SuperOutsideClass(expr.span))
                }
            }

            ExprKind::New {
                class_name,
                arguments,
            } => {
                if let Some(class) = self.env.get_class(class_name).cloned() {
                    // Check constructor arguments if available
                    for arg in arguments {
                        self.check_expr(arg)?;
                    }
                    Ok(Type::Class(class))
                } else {
                    Err(TypeError::UndefinedType(class_name.clone(), expr.span))
                }
            }

            ExprKind::Array(elements) => self.check_array_expr(expr, elements),

            ExprKind::Hash(pairs) => self.check_hash_expr(expr, pairs),

            ExprKind::Assign { target, value } => {
                let target_type = self.check_expr(target)?;
                let value_type = self.check_expr(value)?;

                if !value_type.is_assignable_to(&target_type) {
                    return Err(TypeError::mismatch(
                        format!("{}", target_type),
                        format!("{}", value_type),
                        expr.span,
                    ));
                }

                Ok(target_type)
            }

            ExprKind::Lambda {
                params,
                return_type,
                body,
            } => self.check_lambda_expr(body, params, return_type),

            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
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

            ExprKind::InterpolatedString(parts) => {
                for part in parts {
                    if let crate::ast::expr::InterpolatedPart::Expression(expr) = part {
                        self.check_expr(expr)?;
                    }
                }
                Ok(Type::String)
            }
            ExprKind::Match { expression, arms } => {
                let input_type = self.check_expr(expression)?;

                let mut arm_types = Vec::new();
                for arm in arms {
                    let arm_type = self.check_match_arm(&input_type, arm)?;
                    arm_types.push(arm_type);
                }

                self.common_type(&arm_types)
            }
            ExprKind::ListComprehension {
                element: _,
                variable,
                iterable,
                condition,
            } => {
                // Type check the iterable
                let _iter_type = self.check_expr(iterable)?;

                // Define the loop variable in the environment (as Any type)
                self.env.define(variable.clone(), Type::Any);

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
            ExprKind::HashComprehension {
                key: _,
                value: _,
                variable,
                iterable,
                condition,
            } => {
                // Type check the iterable
                let _iter_type = self.check_expr(iterable)?;

                // Define the loop variable in the environment (as Any type)
                self.env.define(variable.clone(), Type::Any);

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
            ExprKind::Await(_) => {
                unimplemented!("Await expressions not yet implemented")
            }
            ExprKind::Spread(inner) => {
                // Spread takes an array and returns its element type (in context)
                // For now, just check the inner expression and return Array type
                let _inner_type = self.check_expr(inner)?;
                // In array context, spread returns the element type of the array
                // We return Any since we can't easily determine the element type
                Ok(Type::Any)
            }
            ExprKind::Throw(_) => {
                unimplemented!("Throw expressions not yet implemented")
            }
        }
    }

    fn check_binary_expr(
        &mut self,
        expr: &Expr,
        left: &Expr,
        operator: &BinaryOp,
        right: &Expr,
    ) -> TypeResult<Type> {
        let left_type = self.check_expr(left)?;
        let right_type = self.check_expr(right)?;

        match operator {
            BinaryOp::Add => {
                if matches!(left_type, Type::String) || matches!(right_type, Type::String) {
                    Ok(Type::String)
                } else if left_type.is_numeric() && right_type.is_numeric() {
                    if matches!(left_type, Type::Float) || matches!(right_type, Type::Float) {
                        Ok(Type::Float)
                    } else {
                        Ok(Type::Int)
                    }
                } else if matches!(left_type, Type::Any | Type::Unknown)
                    || matches!(right_type, Type::Any | Type::Unknown)
                {
                    Ok(Type::Any)
                } else {
                    Err(TypeError::General {
                        message: format!("cannot add {} and {}", left_type, right_type),
                        span: expr.span,
                    })
                }
            }
            BinaryOp::Subtract | BinaryOp::Multiply | BinaryOp::Divide | BinaryOp::Modulo => {
                if left_type.is_numeric() && right_type.is_numeric() {
                    if matches!(left_type, Type::Float) || matches!(right_type, Type::Float) {
                        Ok(Type::Float)
                    } else {
                        Ok(Type::Int)
                    }
                } else if matches!(left_type, Type::Any | Type::Unknown)
                    || matches!(right_type, Type::Any | Type::Unknown)
                {
                    Ok(Type::Any)
                } else {
                    Err(TypeError::General {
                        message: format!(
                            "cannot perform arithmetic on {} and {}",
                            left_type, right_type
                        ),
                        span: expr.span,
                    })
                }
            }
            BinaryOp::Equal | BinaryOp::NotEqual => Ok(Type::Bool),
            BinaryOp::Less | BinaryOp::LessEqual | BinaryOp::Greater | BinaryOp::GreaterEqual => {
                if (left_type.is_numeric() && right_type.is_numeric())
                    || (matches!(left_type, Type::String) && matches!(right_type, Type::String))
                    || matches!(left_type, Type::Any | Type::Unknown)
                    || matches!(right_type, Type::Any | Type::Unknown)
                {
                    Ok(Type::Bool)
                } else {
                    Err(TypeError::General {
                        message: format!("cannot compare {} and {}", left_type, right_type),
                        span: expr.span,
                    })
                }
            }
            BinaryOp::Range => {
                if (matches!(left_type, Type::Int) && matches!(right_type, Type::Int))
                    || matches!(left_type, Type::Any | Type::Unknown)
                    || matches!(right_type, Type::Any | Type::Unknown)
                {
                    Ok(Type::Array(Box::new(Type::Int)))
                } else {
                    Err(TypeError::General {
                        message: format!(
                            "range (..) expects two integers, got {} and {}",
                            left_type, right_type
                        ),
                        span: expr.span,
                    })
                }
            }
        }
    }

    fn check_unary_expr(
        &mut self,
        expr: &Expr,
        operator: &UnaryOp,
        operand: &Expr,
    ) -> TypeResult<Type> {
        let operand_type = self.check_expr(operand)?;
        match operator {
            UnaryOp::Negate => {
                if operand_type.is_numeric() || matches!(operand_type, Type::Any | Type::Unknown) {
                    Ok(operand_type)
                } else {
                    Err(TypeError::General {
                        message: format!("cannot negate {}", operand_type),
                        span: expr.span,
                    })
                }
            }
            UnaryOp::Not => Ok(Type::Bool),
        }
    }

    fn check_call_expr(
        &mut self,
        expr: &Expr,
        callee: &Expr,
        arguments: &[Expr],
    ) -> TypeResult<Type> {
        let callee_type = self.check_expr(callee)?;

        match callee_type {
            Type::Function {
                params,
                return_type,
            } => {
                // Check argument count (allow fewer args for default parameters)
                // Note: We only check upper bound since we can't easily know which params have defaults
                // The runtime will handle default parameter filling
                if arguments.len() > params.len() && !params.iter().any(|p| matches!(p, Type::Any))
                {
                    return Err(TypeError::WrongArity {
                        expected: params.len(),
                        got: arguments.len(),
                        span: expr.span,
                    });
                }

                // Check argument types
                for (i, arg) in arguments.iter().enumerate() {
                    let arg_type = self.check_expr(arg)?;
                    if let Some(param_type) = params.get(i) {
                        // Allow Any param type to accept any argument (for map/filter/each)
                        if matches!(param_type, Type::Any) {
                            continue;
                        }
                        if !arg_type.is_assignable_to(param_type) {
                            return Err(TypeError::mismatch(
                                format!("{}", param_type),
                                format!("{}", arg_type),
                                arg.span,
                            ));
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
                            span: expr.span,
                        });
                    }
                }
                Ok(Type::Class(class))
            }
            Type::Any | Type::Unknown => Ok(Type::Any),
            _ => Err(TypeError::NotCallable(
                format!("{}", callee_type),
                expr.span,
            )),
        }
    }

    fn check_pipeline_expr(&mut self, left: &Expr, right: &Expr) -> TypeResult<Type> {
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
                            let arg_type = self.check_expr(arg)?;
                            if let Some(param_type) = params.get(i + 1) {
                                if !arg_type.is_assignable_to(param_type) {
                                    return Err(TypeError::mismatch(
                                        format!("{}", param_type),
                                        format!("{}", arg_type),
                                        arg.span,
                                    ));
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

    fn check_member_expr(&mut self, expr: &Expr, object: &Expr, name: &str) -> TypeResult<Type> {
        let obj_type = self.check_expr(object)?;

        match obj_type {
            Type::Class(class) => {
                if let Some(field) = class.find_field(name) {
                    return Ok(field.ty.clone());
                }
                if let Some(method) = class.find_method(name) {
                    return Ok(Type::Function {
                        params: method.params.iter().map(|(_, t)| t.clone()).collect(),
                        return_type: Box::new(method.return_type.clone()),
                    });
                }
                Err(TypeError::NoSuchMember {
                    type_name: class.name,
                    member: name.to_string(),
                    span: expr.span,
                })
            }
            Type::Array(inner_type) => {
                // Handle array methods: map, filter, each
                match name {
                    "map" | "filter" | "each" => {
                        // fn(Element) -> Any (for filter/each) or fn(Element) -> T (for map)
                        // For simplicity, we return fn(Any) -> Any
                        Ok(Type::Function {
                            params: vec![Type::Any],
                            return_type: Box::new(Type::Any),
                        })
                    }
                    _ => Err(TypeError::NoSuchMember {
                        type_name: format!("{}[]", inner_type),
                        member: name.to_string(),
                        span: expr.span,
                    }),
                }
            }
            Type::Hash {
                key_type,
                value_type,
            } => {
                // Handle hash methods: map, filter, each
                match name {
                    "map" | "filter" | "each" => {
                        // fn(Any) -> Any (runtime passes [key, value] array for each entry)
                        Ok(Type::Function {
                            params: vec![Type::Any],
                            return_type: Box::new(Type::Any),
                        })
                    }
                    _ => Err(TypeError::NoSuchMember {
                        type_name: format!("Hash({}, {})", key_type, value_type),
                        member: name.to_string(),
                        span: expr.span,
                    }),
                }
            }
            Type::Any | Type::Unknown => Ok(Type::Any),
            _ => Err(TypeError::NoSuchMember {
                type_name: format!("{}", obj_type),
                member: name.to_string(),
                span: expr.span,
            }),
        }
    }

    fn check_index_expr(&mut self, expr: &Expr, object: &Expr, index: &Expr) -> TypeResult<Type> {
        let obj_type = self.check_expr(object)?;
        let idx_type = self.check_expr(index)?;

        match &obj_type {
            Type::Array(inner) => {
                if !matches!(idx_type, Type::Int | Type::Any | Type::Unknown) {
                    return Err(TypeError::mismatch(
                        "Int",
                        format!("{}", idx_type),
                        index.span,
                    ));
                }
                Ok(*inner.clone())
            }
            Type::String => {
                if !matches!(idx_type, Type::Int | Type::Any | Type::Unknown) {
                    return Err(TypeError::mismatch(
                        "Int",
                        format!("{}", idx_type),
                        index.span,
                    ));
                }
                Ok(Type::String)
            }
            Type::Hash {
                key_type,
                value_type,
            } => {
                if !idx_type.is_assignable_to(key_type) {
                    return Err(TypeError::mismatch(
                        format!("{}", key_type),
                        format!("{}", idx_type),
                        index.span,
                    ));
                }
                Ok(*value_type.clone())
            }
            Type::Any | Type::Unknown => Ok(Type::Any),
            _ => Err(TypeError::General {
                message: format!("cannot index {}", obj_type),
                span: expr.span,
            }),
        }
    }

    fn check_array_expr(&mut self, expr: &Expr, elements: &[Expr]) -> TypeResult<Type> {
        if elements.is_empty() {
            return Ok(Type::Array(Box::new(Type::Unknown)));
        }

        let first_type = self.check_expr(&elements[0])?;
        for elem in elements.iter().skip(1) {
            let elem_type = self.check_expr(elem)?;
            if !elem_type.is_assignable_to(&first_type) && !first_type.is_assignable_to(&elem_type)
            {
                return Err(TypeError::General {
                    message: "array elements have inconsistent types".to_string(),
                    span: expr.span,
                });
            }
        }
        Ok(Type::Array(Box::new(first_type)))
    }

    fn check_hash_expr(&mut self, _expr: &Expr, pairs: &[(Expr, Expr)]) -> TypeResult<Type> {
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

    fn check_lambda_expr(
        &mut self,
        body: &[Stmt],
        params: &[Parameter],
        return_type: &Option<TypeAnnotation>,
    ) -> TypeResult<Type> {
        self.env.push_scope();

        let param_types: Vec<Type> = params
            .iter()
            .map(|param| {
                let t = self.resolve_type(&param.type_annotation);
                self.env.define(param.name.clone(), t.clone());
                t
            })
            .collect();

        let ret_type = return_type
            .as_ref()
            .map(|t| self.resolve_type(t))
            .unwrap_or(Type::Any);

        self.env.set_return_type(Some(ret_type.clone()));

        // Check body statements
        // Note: Implicit return logic is handled in parsing (last expr wrapped in Return)
        // or we rely on Return statements in body.
        for stmt in body {
            if let Err(e) = self.check_stmt(stmt) {
                self.errors.push(e);
            }
        }

        self.env.set_return_type(None);
        self.env.pop_scope();

        // Infer return type from body if not explicit?
        // For now, we just validate against explicit return type (or Any).

        Ok(Type::Function {
            params: param_types,
            return_type: Box::new(ret_type),
        })
    }

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
                if !matches!(input_type, Type::Array(_)) {
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
                if !matches!(input_type, Type::Hash { .. }) {
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
