//! Statement type checking.

use crate::ast::*;
use crate::error::TypeError;
use crate::types::type_repr::Type;

use super::{TypeChecker, TypeResult};

impl TypeChecker {
    pub(crate) fn check_stmt(&mut self, stmt: &Stmt) -> TypeResult<()> {
        match &stmt.kind {
            StmtKind::Expression(expr) => {
                self.check_expr(expr)?;
                Ok(())
            }

            StmtKind::Break => Ok(()),

            StmtKind::Let {
                name,
                type_annotation,
                initializer,
            } => {
                let declared_type = type_annotation.as_ref().map(|t| self.resolve_type(t));
                let init_type = if let Some(init) = initializer {
                    Some(if declared_type.is_some() {
                        self.check_stored(init)?
                    } else {
                        self.check_expr(init)?
                    })
                } else {
                    None
                };

                let var_type = match (declared_type, init_type) {
                    (Some(decl), Some(init)) => {
                        if !init.is_assignable_to(&decl) {
                            return Err(TypeError::mismatch(
                                format!("{}", decl),
                                format!("{}", init),
                                stmt.span,
                            ));
                        }
                        decl
                    }
                    (Some(decl), None) => decl,
                    // `let retenue = nil` states that nothing is known yet,
                    // not that the variable is forever null: the next lines
                    // assign a record to it and read a field back. Typing it
                    // `Null` turned that into "cannot index Null" on code that
                    // runs. An explicit annotation still says what it says.
                    (None, Some(Type::Null)) => Type::Any,
                    (None, Some(init)) => init,
                    (None, None) => Type::Unknown,
                };

                if type_annotation.is_some() {
                    self.env.define_annotated(name.clone(), var_type);
                } else {
                    self.env.define(name.clone(), var_type);
                }
                if type_annotation.is_none() {
                    self.note_constructed_class(name, initializer.as_ref());
                }
                Ok(())
            }

            StmtKind::Const {
                name,
                type_annotation,
                initializer,
            } => {
                let declared_type = type_annotation.as_ref().map(|t| self.resolve_type(t));
                let init_type = self.check_expr(initializer)?;

                let const_type = match declared_type {
                    Some(decl) => {
                        if !init_type.is_assignable_to(&decl) {
                            return Err(TypeError::mismatch(
                                format!("{}", decl),
                                format!("{}", init_type),
                                stmt.span,
                            ));
                        }
                        decl
                    }
                    None => init_type,
                };

                self.env.define(name.clone(), const_type);
                Ok(())
            }

            StmtKind::Block(statements) => {
                self.env.push_scope();
                self.check_body(statements);
                self.env.pop_scope();
                Ok(())
            }

            StmtKind::If {
                condition,
                then_branch,
                else_branch,
            }
            | StmtKind::Unless {
                condition,
                then_branch,
                else_branch,
            } => {
                // Any value is a condition: Soli tests truthiness (`false`,
                // `nil`, `0`, `""`, `[]` and `{}` are falsy), so `if xs.length`
                // and `if hash` run, and refusing them refused working code.
                self.check_expr(condition)?;
                self.check_stmt(then_branch)?;
                if let Some(else_br) = else_branch {
                    self.check_stmt(else_br)?;
                }
                Ok(())
            }

            StmtKind::While { condition, body } => {
                // A truthiness test, like `if`'s.
                self.check_expr(condition)?;
                self.check_stmt(body)?;
                Ok(())
            }

            StmtKind::For {
                variable,
                index_variable,
                iterable,
                body,
            } => {
                let iter_type = self.check_expr(iterable)?;
                let elem_type = match iter_type {
                    Type::Array(inner) => *inner,
                    // Both engines iterate a string by character, each one a
                    // one-character String.
                    Type::String => Type::String,
                    // A hash yields its keys (#639).
                    Type::Hash { key_type, .. } => *key_type,
                    // Unknown is the checker not knowing — a bare `xs: Array`
                    // annotation, a class it cannot see — not a value known not
                    // to iterate. `if` and `while` already let it through;
                    // refusing it here rejected `for x in xs` while
                    // `xs.each do |x| … end` passed.
                    Type::Any | Type::Unknown => Type::Any,
                    _ => {
                        return Err(TypeError::General {
                            message: format!("cannot iterate over {}", iter_type),
                            span: iterable.span,
                        });
                    }
                };

                self.env.push_scope();
                self.env.define(variable.clone(), elem_type);
                if let Some(idx_var) = index_variable {
                    self.env.define(idx_var.clone(), Type::Int);
                }
                let result = self.check_stmt(body);
                self.env.pop_scope();
                result
            }

            StmtKind::Return(value) => {
                let return_type = if let Some(expr) = value {
                    self.check_stored(expr)?
                } else {
                    Type::Void
                };

                if let Some(expected) = self.env.return_type() {
                    if !return_type.is_assignable_to(expected) {
                        return Err(TypeError::mismatch(
                            format!("{}", expected),
                            format!("{}", return_type),
                            stmt.span,
                        ));
                    }
                }
                Ok(())
            }

            StmtKind::Function(decl) => {
                // Resolve parameter types and return type
                let param_types: Vec<Type> = decl
                    .params
                    .iter()
                    .map(|p| self.resolve_type(&p.type_annotation))
                    .collect();
                let return_type = decl
                    .return_type
                    .as_ref()
                    .map(|t| self.resolve_type(t))
                    .unwrap_or(Type::Any);

                // Register function in the OUTER scope so callers (and recursion) can see it
                let func_type = Type::Function {
                    params: param_types.clone(),
                    return_type: Box::new(return_type.clone()),
                };
                self.env.define(decl.name.clone(), func_type);

                // Now push inner scope for the body
                self.env.push_scope();

                // Define parameters
                for (param, ty) in decl.params.iter().zip(param_types.iter()) {
                    self.env.define_annotated(param.name.clone(), ty.clone());
                }

                let outer_return = self.env.return_type().cloned();
                self.env.set_return_type(Some(return_type));
                self.check_function_body(&decl.body);
                self.env.set_return_type(outer_return);
                self.env.pop_scope();
                Ok(())
            }

            StmtKind::Class(decl) => self.check_class_stmt(decl),

            StmtKind::Enum(decl) => self.check_enum_stmt(decl),

            StmtKind::Interface(_) => {
                // Already handled in first pass
                Ok(())
            }

            StmtKind::Import(_) => {
                // Import type checking is handled during module resolution
                Ok(())
            }

            StmtKind::Export(inner) => {
                // Check the inner statement
                self.check_stmt(inner)
            }

            StmtKind::Throw(value) => {
                // throw expressions can throw any type
                self.check_expr(value)?;
                Ok(())
            }

            StmtKind::Try {
                try_block,
                catch_clauses,
                finally_block,
            } => {
                self.check_stmt(try_block)?;

                for clause in catch_clauses {
                    if let Some(ref var_name) = clause.var_name {
                        self.env.push_scope();
                        self.env.define(var_name.clone(), Type::Any);
                        let result = self.check_stmt(&clause.body);
                        self.env.pop_scope();
                        result?;
                    } else {
                        self.check_stmt(&clause.body)?;
                    }
                }

                if let Some(finally_blk) = finally_block {
                    self.check_stmt(finally_blk)?;
                }

                Ok(())
            }
        }
    }

    /// Check a block body, one statement at a time: an error is recorded and
    /// the next statement is still checked, so one mistake early in a `def`
    /// or a `describe` no longer hides every other one in it.
    ///
    /// A statement that fails while binding a new name (`x = broken(…)`)
    /// still binds it, as `Unknown`, so the lines that read it are checked
    /// without a cascade of "Undefined variable 'x'".
    /// A `def` or method body: checked as any body, and its last line — the
    /// value it returns when no `return` runs — held to the declared return
    /// type, as an explicit `return` is. Only a plain expression statement
    /// is judged (not an `if`, a `throw` or a guard clause), and only against
    /// a declared type other than `Any` or `Void`.
    pub(crate) fn check_function_body(&mut self, statements: &[Stmt]) {
        let Some((last, rest)) = statements.split_last() else {
            return;
        };
        self.check_body(rest);
        let StmtKind::Expression(expr) = &last.kind else {
            self.check_body(std::slice::from_ref(last));
            return;
        };
        let value = match self.check_expr(expr) {
            Ok(ty) => ty,
            Err(error) => {
                self.errors.push(error);
                self.bind_after_error(last);
                return;
            }
        };
        let Some(expected) = self.env.return_type().cloned() else {
            return;
        };
        if matches!(expected, Type::Any | Type::Unknown | Type::Void) {
            return;
        }
        if !value.is_assignable_to(&expected) {
            self.errors.push(TypeError::mismatch(
                format!("{}", expected),
                format!("{}", value),
                last.span,
            ));
        }
    }

    pub(crate) fn check_body(&mut self, statements: &[Stmt]) {
        for stmt in statements {
            if let Err(error) = self.check_stmt(stmt) {
                self.errors.push(error);
                self.bind_after_error(stmt);
            }
        }
    }

    fn bind_after_error(&mut self, stmt: &Stmt) {
        match &stmt.kind {
            StmtKind::Let { name, .. } | StmtKind::Const { name, .. } => {
                self.env.define(name.clone(), Type::Unknown);
            }
            StmtKind::Expression(expr) => {
                if let ExprKind::Assign { target, .. } = &expr.kind {
                    if let ExprKind::Variable(name) = &target.kind {
                        if self.env.get(name).is_none() {
                            self.env.define(name.clone(), Type::Unknown);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn check_enum_stmt(&mut self, _decl: &EnumDecl) -> TypeResult<()> {
        // Enum type registration, member-access checking, and match
        // exhaustiveness are handled separately (see `declare_enum`). The user
        // methods are checked via the lowered class in that pass. Nothing to do
        // here for now beyond accepting the declaration.
        Ok(())
    }

    fn check_class_stmt(&mut self, decl: &ClassDecl) -> TypeResult<()> {
        self.env.set_current_class(Some(decl.name.clone()));

        // Check methods
        for method in &decl.methods {
            self.env.push_scope();

            // Define 'this'
            if let Some(class_type) = self.env.get_class(&decl.name) {
                self.env
                    .define("this".to_string(), Type::Class(class_type.clone()));
            }

            // Define parameters
            for param in &method.params {
                let ty = self.resolve_type(&param.type_annotation);
                self.env.define_annotated(param.name.clone(), ty);
            }

            // Set return type (Any if not annotated)
            let return_type = method
                .return_type
                .as_ref()
                .map(|t| self.resolve_type(t))
                .unwrap_or(Type::Any);
            self.env.set_return_type(Some(return_type));

            self.check_function_body(&method.body);

            self.env.set_return_type(None);
            self.env.pop_scope();
        }

        // Check constructor
        if let Some(ref ctor) = decl.constructor {
            self.env.push_scope();

            if let Some(class_type) = self.env.get_class(&decl.name) {
                self.env
                    .define("this".to_string(), Type::Class(class_type.clone()));
            }

            for param in &ctor.params {
                let ty = self.resolve_type(&param.type_annotation);
                self.env.define_annotated(param.name.clone(), ty);
            }

            self.env.set_return_type(Some(Type::Void));

            self.check_body(&ctor.body);

            self.env.set_return_type(None);
            self.env.pop_scope();
        }

        // Verify interface implementation
        self.check_interface_implementation(decl);
        self.check_abstract_methods_defined(decl);
        self.check_overrides(decl);

        self.env.set_current_class(None);
        Ok(())
    }

    /// A concrete class must define every `abstract def` it inherits: one
    /// still reached on its abstract declarer would raise when called.
    fn check_abstract_methods_defined(&mut self, decl: &ClassDecl) {
        if decl.is_abstract || decl.is_module {
            return;
        }
        let Some(class) = self.env.get_class(&decl.name).cloned() else {
            return;
        };
        let mut ancestor = class.superclass.as_deref();
        let mut missing: Vec<(String, String)> = Vec::new();
        while let Some(parent) = ancestor {
            let declared = self
                .abstract_methods
                .get(&parent.name)
                .cloned()
                .unwrap_or_default();
            for name in &declared {
                let owner = class.find_method_with_owner(name).map(|(_, owner)| owner);
                if owner == Some(parent.name.as_str())
                    && !missing.iter().any(|(method, _)| method == name)
                {
                    missing.push((name.clone(), parent.name.clone()));
                }
            }
            ancestor = parent.superclass.as_deref();
        }
        for (method, owner) in missing {
            self.errors.push(TypeError::General {
                message: format!(
                    "class '{}' does not define abstract method '{}' of '{}': define it, or declare `abstract class {}`",
                    decl.name, method, owner, decl.name
                ),
                span: decl.span,
            });
        }
    }

    /// Liskov: a method that overrides one of a superclass declared in source
    /// must be callable as the one it replaces — the same arguments accepted
    /// (a parameter may be wider), a return that fits (it may be narrower).
    /// Unannotated types match anything, as for interfaces. Private parents,
    /// statics, and builtin or framework parents (`Model#save`) are not
    /// judged.
    fn check_overrides(&mut self, decl: &ClassDecl) {
        if decl.is_module || decl.superclass.is_none() {
            return;
        }
        let Some(class) = self.env.get_class(&decl.name).cloned() else {
            return;
        };
        let Some(parent) = class.superclass.as_deref() else {
            return;
        };
        let mut problems = Vec::new();
        for method in decl.methods.iter().filter(|m| !m.is_static) {
            let Some((overridden, owner)) = parent.find_method_with_owner(&method.name) else {
                continue;
            };
            if overridden.is_static || overridden.is_private {
                continue;
            }
            let Some(parent_required) = self
                .source_methods
                .get(owner)
                .and_then(|methods| methods.get(&method.name))
                .copied()
            else {
                continue;
            };
            let Some(own) = class.methods.get(&method.name) else {
                continue;
            };
            let required = method
                .params
                .iter()
                .filter(|p| !p.is_block_param && p.default_value.is_none())
                .count();
            let sig = crate::types::type_repr::MethodSignature {
                name: method.name.clone(),
                params: overridden.params.iter().map(|(_, t)| t.clone()).collect(),
                return_type: overridden.return_type.clone(),
            };
            let against = format!("{}#{}", owner, method.name);
            if let Some(problem) = override_mismatch(
                &own.params,
                required,
                &own.return_type,
                &sig,
                parent_required,
                &against,
            ) {
                problems.push(format!(
                    "method '{}' of class '{}' does not match {} it overrides: {}",
                    method.name, decl.name, against, problem
                ));
            }
        }
        for message in problems {
            self.errors.push(TypeError::General {
                message,
                span: decl.span,
            });
        }
    }

    fn check_interface_implementation(&mut self, decl: &ClassDecl) {
        for iface_name in &decl.interfaces {
            let Some(iface) = self.env.get_interface(iface_name).cloned() else {
                continue;
            };
            let Some(class) = self.env.get_class(&decl.name).cloned() else {
                continue;
            };
            let mut names: Vec<&String> = iface.methods.keys().collect();
            names.sort();
            // `implements Repo<String>`: `T` in the signatures reads `String`
            // (an unbound parameter reads `Any`).
            let mut applied = iface.clone();
            if let Some(generic) = decl
                .interface_type_args
                .iter()
                .find(|g| &g.name == iface_name)
            {
                applied.type_args = generic.args.iter().map(|a| self.resolve_type(a)).collect();
            }
            let bindings = applied.bindings();
            for method_name in names {
                let raw = &iface.methods[method_name];
                let sig = &crate::types::type_repr::MethodSignature {
                    name: raw.name.clone(),
                    params: raw
                        .params
                        .iter()
                        .map(|t| crate::types::generics::substitute(t, &bindings))
                        .collect(),
                    return_type: crate::types::generics::substitute(&raw.return_type, &bindings),
                };
                let Some(method) = class.find_method(method_name) else {
                    self.errors.push(TypeError::General {
                        message: format!(
                            "class '{}' does not implement method '{}' from interface '{}'",
                            decl.name, method_name, iface_name
                        ),
                        span: decl.span,
                    });
                    continue;
                };
                // Parameters with a default, when the method is declared in
                // this class (an inherited one counts every parameter).
                let required = decl
                    .methods
                    .iter()
                    .find(|m| &m.name == method_name && !m.is_static)
                    .map(|m| {
                        m.params
                            .iter()
                            .filter(|p| !p.is_block_param && p.default_value.is_none())
                            .count()
                    })
                    .unwrap_or(method.params.len());
                if let Some(problem) =
                    signature_mismatch(&method.params, required, &method.return_type, sig)
                {
                    self.errors.push(TypeError::General {
                        message: format!(
                            "method '{}' of class '{}' does not match interface '{}': {}",
                            method_name, decl.name, iface_name, problem
                        ),
                        span: decl.span,
                    });
                }
            }
        }
    }
}

/// Why an override is not callable as the method it replaces, or `None`.
/// Like [`signature_mismatch`], except that the parent's own optional
/// parameters need not be taken: an override must accept every call the
/// parent's callers make with its required arguments, and as many of the
/// rest as it declares parameters for.
fn override_mismatch(
    params: &[(String, Type)],
    required: usize,
    return_type: &Type,
    parent: &crate::types::type_repr::MethodSignature,
    parent_required: usize,
    against: &str,
) -> Option<String> {
    let count = |n: usize| {
        if n == 1 {
            "1 parameter".to_string()
        } else {
            format!("{n} parameters")
        }
    };
    if params.len() < parent_required || required > parent_required {
        return Some(format!(
            "it requires {} and takes {}, {} requires {}",
            required,
            count(params.len()),
            against,
            parent_required
        ));
    }
    let shared = params.len().min(parent.params.len());
    let narrowed = crate::types::type_repr::MethodSignature {
        name: parent.name.clone(),
        params: parent.params[..shared].to_vec(),
        return_type: parent.return_type.clone(),
    };
    signature_mismatch(
        &params[..shared],
        required.min(shared),
        return_type,
        &narrowed,
    )
    .map(|problem| problem.replace("in the interface", &format!("in {against}")))
}

/// Why a method does not satisfy an interface signature, or `None` when it
/// does. An unannotated type (`Any`) on either side matches anything: an
/// implementation need not repeat the interface's annotations. Where both
/// sides are annotated, a parameter may be wider than the interface's (it
/// accepts every argument the interface's callers pass) and the return
/// narrower (callers get what the interface promised). The method may take
/// extra parameters only if they have defaults.
fn signature_mismatch(
    params: &[(String, Type)],
    required: usize,
    return_type: &Type,
    sig: &crate::types::type_repr::MethodSignature,
) -> Option<String> {
    let loose = |t: &Type| matches!(t, Type::Any | Type::Unknown | Type::Param(_));
    let count = |n: usize| {
        if n == 1 {
            "1 parameter".to_string()
        } else {
            format!("{n} parameters")
        }
    };
    if params.len() < sig.params.len() || required > sig.params.len() {
        let here = if required == params.len() {
            count(params.len())
        } else {
            format!("{} to {} parameters", required, params.len())
        };
        return Some(format!(
            "it takes {here}, the interface {}",
            count(sig.params.len())
        ));
    }
    for (i, (wanted, (name, actual))) in sig.params.iter().zip(params).enumerate() {
        if loose(wanted) || loose(actual) || wanted.is_assignable_to(actual) {
            continue;
        }
        return Some(format!(
            "parameter {} ('{}') is {} here, {} in the interface",
            i + 1,
            name,
            actual,
            wanted
        ));
    }
    let wanted = &sig.return_type;
    if !(loose(wanted) || loose(return_type) || return_type.is_assignable_to(wanted)) {
        return Some(format!(
            "it returns {} here, {} in the interface",
            return_type, wanted
        ));
    }
    None
}

#[cfg(test)]
mod interface_signature_tests {
    use crate::types::TypeChecker;

    fn check(source: &str) -> Result<(), String> {
        let tokens = crate::lexer::Scanner::new(source)
            .scan_tokens()
            .map_err(|e| e.to_string())?;
        let program = crate::parser::Parser::new(tokens)
            .parse()
            .map_err(|e| e.to_string())?;
        TypeChecker::new().check(&program).map_err(|errs| {
            errs.iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("; ")
        })
    }

    const IFACE: &str = "interface Repository {\n  fn find(id: Int) -> String\n}\n";

    fn class(body: &str) -> String {
        format!("{IFACE}class Repo implements Repository\n{body}\nend\n")
    }

    #[test]
    fn an_implementation_need_not_repeat_the_annotations() {
        check(&class("  def find(id)\n    \"x\"\n  end")).unwrap();
        check(&class("  def find(id: Int)\n    \"x\"\n  end")).unwrap();
        check(&class("  def find(id) -> String\n    \"x\"\n  end")).unwrap();
    }

    #[test]
    fn an_extra_parameter_with_a_default_is_fine_without_one_it_is_not() {
        check(&class("  def find(id, scope = nil)\n    \"x\"\n  end")).unwrap();
        let err = check(&class("  def find(id, scope)\n    \"x\"\n  end")).unwrap_err();
        assert!(
            err.contains("it takes 2 parameters, the interface 1 parameter"),
            "{err}"
        );
        let err = check(&class("  def find\n    \"x\"\n  end")).unwrap_err();
        assert!(err.contains("it takes 0 parameters"), "{err}");
    }

    #[test]
    fn annotated_on_both_sides_and_different_names_the_difference() {
        let err = check(&class("  def find(id: String) -> String\n    id\n  end")).unwrap_err();
        assert!(
            err.contains("does not match interface 'Repository': parameter 1 ('id') is String here, Int in the interface"),
            "{err}"
        );
        let err = check(&class("  def find(id: Int) -> Int\n    id\n  end")).unwrap_err();
        assert!(
            err.contains("it returns Int here, String in the interface"),
            "{err}"
        );
    }

    #[test]
    fn a_wider_parameter_and_a_narrower_return_are_compatible() {
        let source = "interface Scale {\n  fn by(factor: Int) -> Float\n}\n\
            class Doubler implements Scale\n  def by(factor: Float) -> Float\n    factor * 2.0\n  end\nend\n";
        check(source).unwrap();
    }

    const SHAPES: &str = "abstract class Shape\n  abstract def area -> Float\n\n  def describe -> String\n    \"area #{@area}\"\n  end\nend\n";

    #[test]
    fn a_concrete_subclass_must_define_the_abstract_methods() {
        let err = check(&format!("{SHAPES}class Blob < Shape\nend\n")).unwrap_err();
        assert!(
            err.contains("class 'Blob' does not define abstract method 'area' of 'Shape'"),
            "{err}"
        );
        check(&format!(
            "{SHAPES}class Square < Shape\n  def area -> Float\n    4.0\n  end\nend\n"
        ))
        .unwrap();
    }

    #[test]
    fn an_abstract_subclass_may_leave_them_to_its_own_subclasses() {
        let source = format!(
            "{SHAPES}abstract class Polygon < Shape\nend\n\
             class Square < Polygon\n  def area -> Float\n    4.0\n  end\nend\n"
        );
        check(&source).unwrap();
        let err = check(&format!(
            "{SHAPES}abstract class Polygon < Shape\nend\nclass Line < Polygon\nend\n"
        ))
        .unwrap_err();
        assert!(
            err.contains("class 'Line' does not define abstract method 'area'"),
            "{err}"
        );
    }

    #[test]
    fn an_interface_takes_the_methods_of_those_it_extends() {
        let base = "interface Named\n  def name -> String\nend\ninterface Person extends Named\n  def age -> Int\nend\n";
        let err = check(&format!(
            "{base}class Ada implements Person\n  def age -> Int\n    36\n  end\nend\n"
        ))
        .unwrap_err();
        assert!(
            err.contains("does not implement method 'name' from interface 'Person'"),
            "{err}"
        );
        // A Person goes where a Named is expected, and so does its class.
        check(&format!(
            "{base}class Ada implements Person\n  def age -> Int\n    36\n  end\n  def name -> String\n    \"Ada\"\n  end\nend\n\
             def greet(who: Named) -> String\n  who.name\nend\n\
             def hello(who: Person) -> String\n  greet(who) + who.name\nend\n\
             print(greet(Ada.new))\n"
        ))
        .unwrap();
    }

    #[test]
    fn a_generic_interface_binds_its_parameter() {
        let base = "interface Repo<T>\n  def find(id: Int) -> T\nend\n";
        check(&format!(
            "{base}class Names implements Repo<String>\n  def find(id: Int) -> String\n    \"x\"\n  end\nend\n"
        ))
        .unwrap();
        let err = check(&format!(
            "{base}class Names implements Repo<String>\n  def find(id: Int) -> Int\n    1\n  end\nend\n"
        ))
        .unwrap_err();
        assert!(
            err.contains("it returns Int here, String in the interface"),
            "{err}"
        );
        // Through a typed parameter, `T` reads as the argument.
        let err = check(&format!(
            "{base}def bad(repo: Repo<String>)\n  let n: Int = repo.find(1)\n  n\nend\n"
        ))
        .unwrap_err();
        assert!(err.to_lowercase().contains("string"), "{err}");
        check(&format!(
            "{base}def ok(repo: Repo<String>)\n  let s: String = repo.find(1)\n  s\nend\n"
        ))
        .unwrap();
    }

    #[test]
    fn an_override_must_be_callable_as_the_method_it_replaces() {
        let base = "class A\n  def area(x: Int, round = false) -> Int\n    x\n  end\nend\n";
        let err = check(&format!(
            "{base}class B < A\n  def area(x: String) -> String\n    x\n  end\nend\n"
        ))
        .unwrap_err();
        assert!(
            err.contains("method 'area' of class 'B' does not match A#area it overrides: parameter 1 ('x') is String here, Int in A#area"),
            "{err}"
        );
        let err = check(&format!(
            "{base}class B < A\n  def area(x: Int, y: Int) -> Int\n    x\n  end\nend\n"
        ))
        .unwrap_err();
        assert!(
            err.contains("it requires 2 and takes 2 parameters, A#area requires 1"),
            "{err}"
        );
        // Unannotated, wider, narrower, dropping the parent's optional one,
        // or adding an optional one: all fine.
        for body in [
            "  def area(x)\n    x\n  end",
            "  def area(x: Float) -> Int\n    1\n  end",
            "  def area(x: Int) -> Int\n    x\n  end",
            "  def area(x: Int, round = true, scale = 1) -> Int\n    x\n  end",
        ] {
            check(&format!("{base}class B < A\n{body}\nend\n")).unwrap();
        }
    }

    #[test]
    fn the_last_line_is_held_to_the_declared_return_type() {
        let err = check("def label -> Int\n  \"text\"\nend\n").unwrap_err();
        assert!(err.contains("expected Int, found String"), "{err}");
        let err = check("class A\n  def total -> Int\n    \"text\"\n  end\nend\n").unwrap_err();
        assert!(err.contains("expected Int, found String"), "{err}");
        // Not judged: no annotation, Void, a final `if`, a final throw, a
        // nullable return of nil, a widening Int -> Float.
        for source in [
            "def a\n  \"text\"\nend\n",
            "def b -> Void\n  print(\"x\")\nend\n",
            "def c(x) -> Int\n  if x\n    return 1\n  end\n  return 2\nend\n",
            "def d -> Int\n  throw \"no\"\nend\n",
            "def e -> Int?\n  nil\nend\n",
            "def f -> Float\n  1\nend\n",
            "def g -> Int\n  1 + 2\nend\n",
        ] {
            check(source).unwrap_or_else(|e| panic!("{source:?}: {e}"));
        }
    }
}
