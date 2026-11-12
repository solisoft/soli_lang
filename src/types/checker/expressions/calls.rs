//! Function/method call type checking.

use crate::ast::expr::Argument;
use crate::ast::*;
use crate::error::TypeError;
use crate::span::Span;
use crate::types::generics;
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
        // `Inner.new(...)` naming a class this scope cannot see — a nested
        // class from inside its outer class — is left to the runtime, as
        // `new Inner(...)` is (see `check_new_expr`).
        if let ExprKind::Member { object, name } = &callee.kind {
            if let ExprKind::Variable(class_name) = &object.kind {
                if name == "new"
                    && class_name.starts_with(|c: char| c.is_ascii_uppercase())
                    && self.env.get(class_name).is_none()
                    && self.env.get_class(class_name).is_none()
                {
                    return self.check_new_expr(span, object, arguments);
                }
            }
        }
        // `C.new(…)` on a class this pass knows: an instance of `C`, so its
        // members are checked (#693). The arguments are checked as
        // expressions; the constructor's shape (positional, or one hash of
        // attributes) is left to the runtime.
        if let ExprKind::Member { object, name } = &callee.kind {
            if name == "new" {
                if let Some(instance) = self.constructed_instance(object) {
                    let positional = self.check_constructor_arguments(arguments)?;
                    return Ok(match instance {
                        Type::Class(class) => {
                            generics::instance_from_constructor(&class, &positional)
                        }
                        other => other,
                    });
                }
            }
        }
        // The receiver of an XML query (`doc.find("…")`), whose answer the
        // expression decides — see `xml::xml_query_type`.
        let mut query_receiver = None;
        let callee_type = match &callee.kind {
            ExprKind::Variable(name) => {
                self.view_callee = true;
                let callee_type = self.check_variable(name, callee.span);
                self.view_callee = false;
                callee_type?
            }
            ExprKind::Member { object, name } | ExprKind::SafeMember { object, name }
                if !arguments.is_empty() && super::xml::is_query_method(name) =>
            {
                let (member, receiver) =
                    self.check_member_callee_and_receiver(callee.span, object, name)?;
                query_receiver = Some((receiver, name.as_str()));
                member
            }
            // `"ff".to_i(16)`: the arguments are checked against the method's
            // signature, which a bare `"12".to_i` would have collapsed.
            ExprKind::Member { object, name } | ExprKind::SafeMember { object, name }
                if !arguments.is_empty() =>
            {
                self.check_member_callee_with_args(callee.span, object, name)?
            }
            _ => self.check_expr(callee)?,
        };

        // A `def` of this file or of the project: its parameters, defaults
        // included, are known — an untyped parameter used to switch the
        // count off altogether.
        if let ExprKind::Variable(name) = &callee.kind {
            if let Some((min, max, declared, at)) = self.function_arities.get(name) {
                if *declared == callee_type {
                    let given = arguments
                        .iter()
                        .filter(|a| !matches!(a, Argument::Block(_)))
                        .count();
                    if given < *min || given > *max {
                        return Err(TypeError::WrongArity {
                            expected: if given < *min { *min } else { *max },
                            got: given,
                            span,
                            declared: *at,
                        });
                    }
                }
            }
        }

        match callee_type {
            Type::Function {
                params,
                return_type,
            } => {
                // A generic signature (`def first<T>(xs: T[]) -> T`): bind its
                // type parameters from the positional arguments, then check
                // and type the call against the bound signature.
                let mut checked: Vec<Option<Type>> = Vec::new();
                let (params, return_type) = if params.iter().any(generics::mentions_param)
                    || generics::mentions_param(&return_type)
                {
                    let mut bindings = std::collections::HashMap::new();
                    for (i, arg) in arguments.iter().enumerate() {
                        let arg_type = match arg {
                            Argument::Positional(expr) => Some(self.check_stored(expr)?),
                            _ => None,
                        };
                        if let (Some(arg_type), Some(param)) = (&arg_type, params.get(i)) {
                            generics::unify(param, arg_type, &mut bindings);
                        }
                        checked.push(arg_type);
                    }
                    (
                        params
                            .iter()
                            .map(|p| generics::substitute(p, &bindings))
                            .collect::<Vec<_>>(),
                        Box::new(generics::substitute(&return_type, &bindings)),
                    )
                } else {
                    (params, return_type)
                };
                let total_args = arguments.len();

                // Check argument count (allow fewer args for default parameters)
                // Note: We only check upper bound since we can't easily know which params have defaults
                // The runtime will handle default parameter filling
                if total_args > params.len() && !params.iter().any(|p| matches!(p, Type::Any)) {
                    return Err(TypeError::WrongArity {
                        expected: params.len(),
                        got: total_args,
                        span,
                        declared: None,
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
                            let arg_type = match checked.get_mut(i).and_then(Option::take) {
                                Some(arg_type) => arg_type,
                                None => self.check_stored(expr)?,
                            };
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
                        Argument::Block(expr) => {
                            // Block arguments - type check the block expression
                            self.check_expr(expr)?;
                            // For now, skip block param type checking
                        }
                    }
                }

                if let Some((receiver, name)) = &query_receiver {
                    if let Some(answer) = self.xml_query_type(receiver, name, arguments)? {
                        return Ok(answer);
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
                            declared: None,
                        });
                    }
                }
                Ok(Type::Class(class))
            }
            // Nothing is known of the callee, but its arguments are still
            // expressions of this file: a block passed to a method of an
            // untyped value (`xs.each do |x| … end` with `xs` a parameter) was
            // never checked at all.
            Type::Any | Type::Unknown => {
                for arg in arguments {
                    match arg {
                        Argument::Positional(expr) | Argument::Block(expr) => {
                            self.check_expr(expr)?;
                        }
                        Argument::Named(named) => {
                            self.check_expr(&named.value)?;
                        }
                    }
                }
                Ok(Type::Any)
            }
            // `s.length()` — empty parens on a member whose type already
            // collapsed to the method's return type (bare zero-arg methods
            // auto-invoke at runtime, see collapse_zero_arg_method). The
            // runtime treats the call form like the bare form; mirror it.
            // The same for a bare method name inside the class (`_label()`),
            // which `check_variable` resolved as `@_label`.
            _ if arguments.is_empty()
                && (matches!(
                    callee.kind,
                    ExprKind::Member { .. } | ExprKind::SafeMember { .. }
                ) || self.is_implicit_self_call(callee)) =>
            {
                Ok(callee_type)
            }
            _ => match &callee.kind {
                // `options = "…"` then `options("/x", …)`: a local named after
                // a builtin, called as the builtin.
                ExprKind::Variable(name)
                    if crate::types::runtime_globals::is_builtin_name(name) =>
                {
                    Err(TypeError::General {
                        message: format!(
                            "`{name}` is a local {callee_type} here, shadowing the builtin \
                             {name}(); rename it"
                        ),
                        span,
                    })
                }
                _ => Err(TypeError::NotCallable(format!("{}", callee_type), span)),
            },
        }
    }

    /// A bare name that is no variable or function in scope, inside an
    /// instance method: `check_variable` resolved it as a method of `this`.
    fn is_implicit_self_call(&self, callee: &Expr) -> bool {
        match &callee.kind {
            ExprKind::Variable(name) => {
                self.env.get(name).is_none() && self.env.get("this").is_some()
            }
            _ => false,
        }
    }

    /// Check pipeline expression.
    pub(crate) fn check_pipeline_expr(&mut self, left: &Expr, right: &Expr) -> TypeResult<Type> {
        let left_type = self.check_expr(left)?;

        // Right side can be a call or a function value
        match &right.kind {
            ExprKind::Call { callee, arguments } => {
                // Both engines evaluate the callee before passing the piped
                // value, so `x |> s.split()` reads `s.split` bare: a method the
                // runtime calls bare is collapsed here too.
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
                                Argument::Block(expr) => {
                                    // Block arguments in pipeline - skip type checking
                                    self.check_expr(expr)?;
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
