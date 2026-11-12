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
            Type::Int
                | Type::Float
                | Type::String
                | Type::Symbol
                | Type::Bool
                | Type::Any
                | Type::Unknown
        ) {
            return Err(TypeError::General {
                message: format!("Cannot use {} as hash key", key_type),
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
        class_expr: &Expr,
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
                Argument::Block(expr) => {
                    self.check_expr(expr)?;
                }
            }
        }
        // A class this pass knows builds an instance of it; anything else
        // (a nested class, a qualified name) is left to the runtime.
        Ok(self
            .constructed_instance(class_expr)
            .unwrap_or(Type::Unknown))
    }

    /// The instance `new C(…)` / `C.new(…)` builds, `class_expr` being `C`:
    /// a class this pass declared (in this file or, in a project, in
    /// another), not shadowed by a local of that name, and without a
    /// `static def new` of its own that may return anything.
    pub(crate) fn constructed_instance(&self, class_expr: &Expr) -> Option<Type> {
        let ExprKind::Variable(name) = &class_expr.kind else {
            return None;
        };
        let class = self.env.get_class(name)?;
        match self.env.get(name) {
            None => {}
            Some(Type::Class(bound)) if bound.name == class.name => {}
            Some(_) => return None,
        }
        if class.methods.get("new").is_some_and(|m| m.is_static) {
            return None;
        }
        Some(Type::Class(class.clone()))
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
        let value_type = self.check_expr(value)?;

        // Auto-define: if target is an undefined variable, define it with the RHS type
        if let ExprKind::Variable(name) = &target.kind {
            // A name the checker knows only as a function the runtime does not
            // install — a stale model of an old global (`push`), or a sibling
            // file's `def` — is a fresh local here, not a rebinding the value
            // must fit: `push = 0.0` in a method was refused as
            // "expected (Any, Any) -> Void, found Float". A `def` of this
            // file is installed, though, and a bare assignment rebinds it.
            let shadows_a_non_builtin = !self.declared_functions.contains(name)
                && !self.env.is_variable(name)
                && self.env.get_class(name).is_none()
                && !crate::types::runtime_globals::is_builtin_name(name);
            if self.env.get(name).is_none() || shadows_a_non_builtin {
                // Same reading as `let x = nil` (see `statements.rs`): a bare
                // `x = nil` says nothing is known yet. Defining it as `Null`
                // refused the next `x = records` as "expected Null, found …"
                // — and the bare form is the one the conventions recommend.
                let defined_type = match value_type {
                    Type::Null => Type::Any,
                    ref other => other.clone(),
                };
                self.env.define(name.clone(), defined_type);
                self.note_constructed_class(name, Some(value));
                return Ok(value_type);
            }
            self.note_constructed_class(name, Some(value));
        }

        // Writing a key into a hash. The value type of a hash literal is a
        // snapshot of what that literal happened to hold, not a declaration:
        // `let e = {"timeout": 10}` followed by `e["Authorization"] = "Bearer …"`
        // is ordinary Soli — a hash is a heterogeneous bag at run time — and
        // reading the snapshot as a constraint rejected it as
        // "expected Int, found String".
        if let ExprKind::Index { object, .. } = &target.kind {
            if matches!(self.check_expr(object)?, Type::Hash { .. }) {
                return Ok(value_type);
            }
        }

        // Writing `@errors` sets a field. When the class declares no field of
        // that name but has a method of it (`static def errors(…)`), the member
        // lookup answers the method's type, and the write was refused as
        // "expected (Hash, Bool) -> Any[], found Any[]".
        if let ExprKind::Member { object, name } = &target.kind {
            let receiver = self.check_expr(object)?;
            self.check_xml_property_write(&receiver, name, &value_type, span)?;
            if let Type::Class(class) = receiver {
                if let Some(class_def) = self.env.get_class(&class.name) {
                    if class_def.find_field_with_owner(name).is_none()
                        && class_def.find_method_with_owner(name).is_some()
                    {
                        return Ok(value_type);
                    }
                }
            }
        }

        // The slot as declared, `?` included: `x = nil` into an `x: Int?`
        // parameter is allowed, and reading the target as a value would have
        // stripped the `?`.
        let target_type = match &target.kind {
            ExprKind::Variable(name) => match self.env.get(name) {
                Some(ty) => ty,
                None => self.check_expr(target)?,
            },
            _ => self.check_expr_slot(target)?,
        };

        if !value_type.is_assignable_to(&target_type) {
            // A local nobody annotated holds whatever was last put in it, so a
            // reassignment that changes its type widens it instead of being
            // refused: `total = 0` then `total = total * 1.5` is ordinary
            // Soli. An annotated binding (`let x: Int`, a typed parameter)
            // still refuses, and so does a builtin (the message below).
            if let ExprKind::Variable(name) = &target.kind {
                if !self.rebinds_a_function(name, &target_type) && self.env.is_widenable(name) {
                    let widened = match (&target_type, &value_type) {
                        (Type::Int, Type::Float) | (Type::Float, Type::Int) => Type::Float,
                        _ => Type::Any,
                    };
                    self.env.widen(name, widened);
                    return Ok(value_type);
                }
            }
            // `page_url = "x"` inside a method: the name is a builtin, so the
            // assignment reads as rebinding it, and the bare mismatch
            // ("expected (Any, Any) -> String, found String") named neither
            // the builtin nor the way out.
            if let ExprKind::Variable(name) = &target.kind {
                if matches!(target_type, Type::Function { .. })
                    && !crate::types::runtime_globals::is_builtin_name(name)
                    && self.declared_functions.contains(name)
                {
                    return Err(TypeError::General {
                        message: format!(
                            "`{name} = …` assigns a {value_type} to the function {name}() this \
                             file defines, which a bare assignment rebinds rather than shadows; \
                             rename the local (or declare it with `let {name} = …`)"
                        ),
                        span,
                    });
                }
                if matches!(target_type, Type::Function { .. })
                    && crate::types::runtime_globals::is_builtin_name(name)
                {
                    return Err(TypeError::General {
                        message: format!(
                            "`{name} = …` assigns a {value_type} to the builtin {name}(), \
                             which a bare assignment rebinds rather than shadows; rename the \
                             local (or declare it with `let {name} = …`)"
                        ),
                        span,
                    });
                }
            }
            return Err(TypeError::mismatch(
                format!("{}", target_type),
                format!("{}", value_type),
                span,
            ));
        }

        Ok(target_type)
    }

    /// Whether `name = …` would rebind a function the runtime installs — a
    /// builtin or a `def` of this file — rather than set a local: such a
    /// reassignment is reported, never widened.
    fn rebinds_a_function(&self, name: &str, target_type: &Type) -> bool {
        matches!(target_type, Type::Function { .. })
            && (crate::types::runtime_globals::is_builtin_name(name)
                || self.declared_functions.contains(name))
    }

    /// Check if expression.
    pub(crate) fn check_if_expr(
        &mut self,
        condition: &Expr,
        then_branch: &Expr,
        else_branch: Option<&Expr>,
    ) -> TypeResult<Type> {
        // Any value is a condition, as for an `if` *statement* (see
        // `statements.rs`): Soli's truthiness takes any value, so demanding
        // `Bool` refused working code.
        self.check_expr(condition)?;

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

        self.check_match_exhaustiveness(&input_type, arms, expression.span);

        self.common_type(&arm_types)
    }

    /// Best-effort exhaustiveness check: when the scrutinee's static type is a
    /// known enum and the arms cover neither all variants nor a `_`/binding
    /// catch-all, record a non-blocking [`TypeError::ExhaustivenessWarning`].
    /// Only fires when the enum type is statically inferable (dynamic Soli code
    /// where the type is unknown is left alone).
    fn check_match_exhaustiveness(&mut self, input_type: &Type, arms: &[MatchArm], span: Span) {
        let enum_name = match input_type {
            Type::Class(class) => class.name.clone(),
            _ => return,
        };
        let all_variants = match self.env.get_enum(&enum_name) {
            Some(enum_type) => enum_type.variants.clone(),
            None => return,
        };

        // An unguarded `_` or bare-variable arm covers everything. A guarded
        // arm guarantees nothing, so it doesn't count toward coverage.
        let has_catch_all = arms.iter().any(|arm| {
            arm.guard.is_none()
                && matches!(
                    arm.pattern,
                    MatchPattern::Wildcard | MatchPattern::Variable(_)
                )
        });
        if has_catch_all {
            return;
        }

        let mut covered = std::collections::HashSet::new();
        for arm in arms {
            if arm.guard.is_some() {
                continue;
            }
            if let MatchPattern::EnumVariant {
                enum_name: arm_enum,
                variant_name,
                ..
            } = &arm.pattern
            {
                if *arm_enum == enum_name {
                    covered.insert(variant_name.clone());
                }
            }
        }

        let missing: Vec<String> = all_variants
            .into_iter()
            .filter(|variant| !covered.contains(variant))
            .collect();
        if !missing.is_empty() {
            self.warnings.push(TypeError::ExhaustivenessWarning {
                enum_name,
                missing: missing.join(", "),
                span,
            });
        }
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
                    ExprKind::DecimalLiteral(_) => Type::Decimal(0),
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

            MatchPattern::Array { elements, rest } => {
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
                // `...rest` is the leftover slice: an array of the same kind.
                // Left undefined, any use of it was an "Undefined variable".
                if let Some(rest_name) = rest {
                    self.env.define(rest_name.clone(), input_type.clone());
                }
                Ok(())
            }

            MatchPattern::Hash { fields, rest } => {
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
                // `...rest` is the hash minus the named keys.
                if let Some(rest_name) = rest {
                    self.env.define(rest_name.clone(), input_type.clone());
                }
                Ok(())
            }

            MatchPattern::Destructuring { type_name, .. } => {
                if let Some(class) = self.env.get_class(type_name).cloned() {
                    if !input_type.is_assignable_to(&Type::Class(class.clone())) {
                        return Err(TypeError::mismatch(
                            type_name.clone(),
                            format!("{}", input_type),
                            Span::default(),
                        ));
                    }
                    Ok(())
                } else {
                    Err(TypeError::UndefinedType(type_name.clone(), Span::default()))
                }
            }

            MatchPattern::EnumVariant { bindings, .. } => {
                // Best-effort: bind each payload sub-pattern as Any. Full
                // enum-aware checking + exhaustiveness lands with enum type
                // registration (see `declare_enum`).
                for binding in bindings {
                    self.check_match_pattern(&Type::Any, binding)?;
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
