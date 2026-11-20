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
                self.check_body(&decl.body);
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

            self.check_body(&method.body);

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
            for method_name in names {
                let sig = &iface.methods[method_name];
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
    let loose = |t: &Type| matches!(t, Type::Any | Type::Unknown);
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
}
