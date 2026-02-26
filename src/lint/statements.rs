use crate::ast::stmt::{ClassDecl, Stmt, StmtKind};

use super::rules;
use super::Linter;

impl Linter {
    pub(crate) fn lint_stmt(&mut self, stmt: &Stmt) {
        match &stmt.kind {
            StmtKind::Expression(expr) => self.lint_expr(expr),

            StmtKind::Let {
                name,
                type_annotation: _,
                initializer,
            } => {
                rules::naming::check_variable_name(name, stmt.span, &mut self.diagnostics);
                if let Some(init) = initializer {
                    self.lint_expr(init);
                }
            }

            StmtKind::Const {
                name,
                type_annotation: _,
                initializer,
            } => {
                rules::naming::check_variable_name(name, stmt.span, &mut self.diagnostics);
                self.lint_expr(initializer);
            }

            StmtKind::Block(stmts) => {
                if stmts.is_empty() {
                    rules::style::check_empty_block(stmt.span, &mut self.diagnostics);
                } else {
                    rules::smell::check_unreachable_code(stmts, &mut self.diagnostics);
                    for s in stmts {
                        self.lint_stmt(s);
                    }
                }
            }

            StmtKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.depth += 1;
                rules::smell::check_deep_nesting(self.depth, stmt.span, &mut self.diagnostics);
                self.lint_expr(condition);
                self.lint_stmt(then_branch);
                if let Some(else_b) = else_branch {
                    self.lint_stmt(else_b);
                }
                self.depth -= 1;
            }

            StmtKind::While { condition, body } => {
                self.depth += 1;
                rules::smell::check_deep_nesting(self.depth, stmt.span, &mut self.diagnostics);
                self.lint_expr(condition);
                self.lint_stmt(body);
                self.depth -= 1;
            }

            StmtKind::For {
                variable,
                index_variable,
                iterable,
                body,
            } => {
                rules::naming::check_variable_name(variable, stmt.span, &mut self.diagnostics);
                if let Some(idx_var) = index_variable {
                    rules::naming::check_variable_name(idx_var, stmt.span, &mut self.diagnostics);
                }
                self.depth += 1;
                rules::smell::check_deep_nesting(self.depth, stmt.span, &mut self.diagnostics);
                self.lint_expr(iterable);
                self.lint_stmt(body);
                self.depth -= 1;
            }

            StmtKind::Return(value) => {
                if let Some(val) = value {
                    self.lint_expr(val);
                }
            }

            StmtKind::Throw(expr) => self.lint_expr(expr),

            StmtKind::Try {
                try_block,
                catch_var: _,
                catch_block,
                finally_block,
            } => {
                self.depth += 1;
                rules::smell::check_deep_nesting(self.depth, stmt.span, &mut self.diagnostics);
                self.lint_stmt(try_block);
                if let Some(catch) = catch_block {
                    rules::smell::check_empty_catch(catch, &mut self.diagnostics);
                    self.lint_stmt(catch);
                }
                if let Some(finally) = finally_block {
                    self.lint_stmt(finally);
                }
                self.depth -= 1;
            }

            StmtKind::Function(decl) => {
                rules::naming::check_function_name(&decl.name, decl.span, &mut self.diagnostics);
                for param in &decl.params {
                    rules::naming::check_variable_name(
                        &param.name,
                        param.span,
                        &mut self.diagnostics,
                    );
                }
                self.lint_body(&decl.body);
            }

            StmtKind::Class(decl) => self.lint_class_decl(decl),

            StmtKind::Interface(decl) => {
                rules::naming::check_class_name(
                    "interface",
                    &decl.name,
                    decl.span,
                    &mut self.diagnostics,
                );
            }

            StmtKind::Import(_) => {}

            StmtKind::Export(inner) => self.lint_stmt(inner),
        }
    }

    fn lint_class_decl(&mut self, decl: &ClassDecl) {
        rules::naming::check_class_name("class", &decl.name, decl.span, &mut self.diagnostics);
        rules::smell::check_duplicate_methods(decl, &mut self.diagnostics);

        // Lint field initializers
        for field in &decl.fields {
            if let Some(init) = &field.initializer {
                self.lint_expr(init);
            }
        }

        // Lint methods
        for method in &decl.methods {
            rules::naming::check_function_name(&method.name, method.span, &mut self.diagnostics);
            for param in &method.params {
                rules::naming::check_variable_name(&param.name, param.span, &mut self.diagnostics);
            }
            self.lint_body(&method.body);
        }

        // Lint constructor
        if let Some(ctor) = &decl.constructor {
            for param in &ctor.params {
                rules::naming::check_variable_name(&param.name, param.span, &mut self.diagnostics);
            }
            self.lint_body(&ctor.body);
        }

        // Lint static block
        if let Some(static_stmts) = &decl.static_block {
            for s in static_stmts {
                self.lint_stmt(s);
            }
        }

        // Lint class statements
        for s in &decl.class_statements {
            self.lint_stmt(s);
        }

        // Lint nested classes
        for nested in &decl.nested_classes {
            self.lint_class_decl(nested);
        }
    }
}
