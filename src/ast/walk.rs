//! Read-only traversal of a whole program: every statement and every
//! expression, including class and method bodies, lambdas, default values and
//! interpolations.
//!
//! For passes that only need to *find* something (a name, a construct) and do
//! not care about scope; anything scope-aware walks the tree itself.

use super::expr::{Argument, Expr, ExprKind, InterpolatedPart};
use super::stmt::{ClassDecl, MethodDecl, Parameter, Program, Stmt, StmtKind};

/// Call `on_stmt` for every statement and `on_expr` for every expression of
/// `program`, parents before children.
pub fn walk_program(
    program: &Program,
    on_stmt: &mut dyn FnMut(&Stmt),
    on_expr: &mut dyn FnMut(&Expr),
) {
    let mut walker = Walker { on_stmt, on_expr };
    walker.stmts(&program.statements);
}

struct Walker<'a> {
    on_stmt: &'a mut dyn FnMut(&Stmt),
    on_expr: &'a mut dyn FnMut(&Expr),
}

impl Walker<'_> {
    fn stmts(&mut self, stmts: &[Stmt]) {
        for stmt in stmts {
            self.stmt(stmt);
        }
    }

    fn params(&mut self, params: &[Parameter]) {
        for param in params {
            if let Some(default) = &param.default_value {
                self.expr(default);
            }
        }
    }

    fn method(&mut self, method: &MethodDecl) {
        self.params(&method.params);
        self.stmts(&method.body);
    }

    fn class(&mut self, class: &ClassDecl) {
        for field in &class.fields {
            if let Some(init) = &field.initializer {
                self.expr(init);
            }
        }
        for method in class.methods.iter().chain(&class.concern_class_methods) {
            self.method(method);
        }
        if let Some(ctor) = &class.constructor {
            self.params(&ctor.params);
            self.stmts(&ctor.body);
        }
        if let Some(block) = &class.static_block {
            self.stmts(block);
        }
        self.stmts(&class.class_statements);
        for hook in class.included_hooks.iter().chain(&class.extended_hooks) {
            self.stmts(hook);
        }
        for nested in &class.nested_classes {
            self.class(nested);
        }
    }

    fn stmt(&mut self, stmt: &Stmt) {
        (self.on_stmt)(stmt);
        match &stmt.kind {
            StmtKind::Expression(e) | StmtKind::Throw(e) => self.expr(e),
            StmtKind::Let { initializer, .. } => {
                if let Some(e) = initializer {
                    self.expr(e);
                }
            }
            StmtKind::Const { initializer, .. } => self.expr(initializer),
            StmtKind::Block(stmts) => self.stmts(stmts),
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
                self.expr(condition);
                self.stmt(then_branch);
                if let Some(other) = else_branch {
                    self.stmt(other);
                }
            }
            StmtKind::While { condition, body } => {
                self.expr(condition);
                self.stmt(body);
            }
            StmtKind::For { iterable, body, .. } => {
                self.expr(iterable);
                self.stmt(body);
            }
            StmtKind::Return(value) => {
                if let Some(e) = value {
                    self.expr(e);
                }
            }
            StmtKind::Try {
                try_block,
                catch_clauses,
                finally_block,
            } => {
                self.stmt(try_block);
                for clause in catch_clauses {
                    self.stmt(&clause.body);
                }
                if let Some(finally) = finally_block {
                    self.stmt(finally);
                }
            }
            StmtKind::Function(decl) => {
                self.params(&decl.params);
                self.stmts(&decl.body);
            }
            StmtKind::Class(decl) => self.class(decl),
            StmtKind::Enum(decl) => {
                for method in &decl.methods {
                    self.method(method);
                }
            }
            StmtKind::Export(inner) => self.stmt(inner),
            StmtKind::Break | StmtKind::Interface(_) | StmtKind::Import(_) => {}
        }
    }

    fn args(&mut self, arguments: &[Argument]) {
        for arg in arguments {
            match arg {
                Argument::Positional(e) | Argument::Block(e) => self.expr(e),
                Argument::Named(named) => self.expr(&named.value),
            }
        }
    }

    fn expr(&mut self, expr: &Expr) {
        (self.on_expr)(expr);
        match &expr.kind {
            ExprKind::IntLiteral(_)
            | ExprKind::FloatLiteral(_)
            | ExprKind::DecimalLiteral(_)
            | ExprKind::StringLiteral(_)
            | ExprKind::CommandSubstitution(_)
            | ExprKind::SdqlBlock { .. }
            | ExprKind::BoolLiteral(_)
            | ExprKind::Symbol(_)
            | ExprKind::Null
            | ExprKind::Variable(_)
            | ExprKind::This
            | ExprKind::Super => {}
            ExprKind::InterpolatedString(parts) => {
                for part in parts {
                    if let InterpolatedPart::Expression(e) = part {
                        self.expr(e);
                    }
                }
            }
            ExprKind::Binary { left, right, .. }
            | ExprKind::Pipeline { left, right }
            | ExprKind::LogicalAnd { left, right }
            | ExprKind::LogicalOr { left, right }
            | ExprKind::NullishCoalescing { left, right } => {
                self.expr(left);
                self.expr(right);
            }
            ExprKind::Unary { operand: inner, .. }
            | ExprKind::Grouping(inner)
            | ExprKind::Member { object: inner, .. }
            | ExprKind::SafeMember { object: inner, .. }
            | ExprKind::QualifiedName {
                qualifier: inner, ..
            }
            | ExprKind::PostfixIncrement(inner)
            | ExprKind::PostfixDecrement(inner)
            | ExprKind::Spread(inner)
            | ExprKind::Throw(inner) => self.expr(inner),
            ExprKind::Call { callee, arguments } => {
                self.expr(callee);
                self.args(arguments);
            }
            ExprKind::New {
                class_expr,
                arguments,
            } => {
                self.expr(class_expr);
                self.args(arguments);
            }
            ExprKind::Index { object, index } => {
                self.expr(object);
                self.expr(index);
            }
            ExprKind::Array(items) => {
                for item in items {
                    self.expr(item);
                }
            }
            ExprKind::Hash(pairs) => {
                for (k, v) in pairs {
                    self.expr(k);
                    self.expr(v);
                }
            }
            ExprKind::Block(stmts) => self.stmts(stmts),
            ExprKind::Assign { target, value } | ExprKind::CompoundAssign { target, value, .. } => {
                self.expr(target);
                self.expr(value);
            }
            ExprKind::Lambda { params, body, .. } => {
                self.params(params);
                self.stmts(body);
            }
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.expr(condition);
                self.expr(then_branch);
                if let Some(other) = else_branch {
                    self.expr(other);
                }
            }
            ExprKind::Match { expression, arms } => {
                self.expr(expression);
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        self.expr(guard);
                    }
                    self.expr(&arm.body);
                }
            }
            ExprKind::ListComprehension {
                element,
                iterable,
                condition,
                ..
            } => {
                self.expr(element);
                self.expr(iterable);
                if let Some(c) = condition {
                    self.expr(c);
                }
            }
            ExprKind::HashComprehension {
                key,
                value,
                iterable,
                condition,
                ..
            } => {
                self.expr(key);
                self.expr(value);
                self.expr(iterable);
                if let Some(c) = condition {
                    self.expr(c);
                }
            }
            ExprKind::Rescue { expr, fallback } => {
                self.expr(expr);
                self.expr(fallback);
            }
        }
    }
}
