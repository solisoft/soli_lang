//! Pre-pass that discovers function-scoped locals introduced by bare assignment.
//!
//! Soli's `let` is optional: `name = value` creates the binding when `name`
//! isn't already in scope. The tree-walking interpreter implements this at
//! runtime (assign up the scope chain, else define in the current scope). The
//! bytecode compiler resolves variables statically, so before compiling a
//! function body it must know which bare-assigned names become *new locals* of
//! that function versus assignments to an existing global.
//!
//! This pass collects the names that are introduced by a plain `=` assignment
//! somewhere in the body and are not otherwise declared (`let`/`const`, loop
//! variables, catch bindings). The compiler then declares those as locals at
//! function entry — except names that are parameters, captured from an
//! enclosing scope (upvalues), or already known globals.
//!
//! Scope model: this is *function*-scoped (it ignores `{ }` block boundaries
//! within the body), which is a safe superset of the tree-walker's
//! block-scoped behavior — any program that runs cleanly on the interpreter
//! produces identical results, the VM only additionally tolerates reads that
//! the interpreter would reject as "undefined" (a developer would never ship
//! such code, since it errors in `--dev`). Nested function/lambda/class bodies
//! are *not* scanned: they are separate scopes with their own hoisting.

use std::collections::HashSet;

use crate::ast::expr::{Argument, Expr, ExprKind, InterpolatedPart};
use crate::ast::stmt::{Stmt, StmtKind};

/// Collect names that should be declared as function-scoped locals because a
/// bare `name = value` assignment introduces them in `body`. Names that are
/// `let`/`const`-declared, loop variables, or catch bindings are excluded —
/// those manage their own bindings.
pub(super) fn collect_hoisted_locals(body: &[Stmt]) -> Vec<String> {
    let mut scan = Scan::default();
    for stmt in body {
        scan.stmt(stmt);
    }
    scan.assigned
        .into_iter()
        .filter(|name| !scan.declared.contains(name))
        .collect()
}

#[derive(Default)]
struct Scan {
    /// Plain-assignment targets, in first-seen order, deduped via `seen`.
    assigned: Vec<String>,
    seen: HashSet<String>,
    /// Names with their own binding mechanism (let/const/loop var/catch var).
    declared: HashSet<String>,
}

impl Scan {
    fn record_assigned(&mut self, name: &str) {
        if self.seen.insert(name.to_string()) {
            self.assigned.push(name.to_string());
        }
    }

    fn stmt(&mut self, stmt: &Stmt) {
        match &stmt.kind {
            StmtKind::Expression(e) => self.expr(e),
            StmtKind::Let {
                name, initializer, ..
            } => {
                self.declared.insert(name.clone());
                if let Some(init) = initializer {
                    self.expr(init);
                }
            }
            StmtKind::Break => {}
            StmtKind::Const {
                name, initializer, ..
            } => {
                self.declared.insert(name.clone());
                self.expr(initializer);
            }
            StmtKind::Block(stmts) => {
                // Same function scope — block boundaries don't matter here.
                for s in stmts {
                    self.stmt(s);
                }
            }
            StmtKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.expr(condition);
                self.stmt(then_branch);
                if let Some(eb) = else_branch {
                    self.stmt(eb);
                }
            }
            StmtKind::While { condition, body } => {
                self.expr(condition);
                self.stmt(body);
            }
            StmtKind::For {
                variable,
                index_variable,
                iterable,
                body,
            } => {
                self.declared.insert(variable.clone());
                if let Some(iv) = index_variable {
                    self.declared.insert(iv.clone());
                }
                self.expr(iterable);
                self.stmt(body);
            }
            StmtKind::Return(Some(e)) => self.expr(e),
            StmtKind::Return(None) => {}
            StmtKind::Throw(e) => self.expr(e),
            StmtKind::Try {
                try_block,
                catch_clauses,
                finally_block,
            } => {
                self.stmt(try_block);
                for clause in catch_clauses {
                    if let Some(var) = &clause.var_name {
                        self.declared.insert(var.clone());
                    }
                    self.stmt(&clause.body);
                }
                if let Some(fb) = finally_block {
                    self.stmt(fb);
                }
            }
            StmtKind::Export(inner) => self.stmt(inner),
            // Separate scopes / no in-scope expressions to scan.
            StmtKind::Function(_)
            | StmtKind::Class(_)
            | StmtKind::Enum(_)
            | StmtKind::Interface(_)
            | StmtKind::Import(_) => {}
        }
    }

    fn expr(&mut self, expr: &Expr) {
        match &expr.kind {
            ExprKind::Assign { target, value } => {
                if let ExprKind::Variable(name) = &target.kind {
                    self.record_assigned(name);
                } else {
                    // obj.field = / arr[i] = : the target has sub-expressions
                    // to scan, but introduces no new local binding.
                    self.expr(target);
                }
                self.expr(value);
            }
            ExprKind::CompoundAssign { target, value, .. } => {
                // Compound assignment requires an existing binding (it reads
                // first), so it never introduces a new local. Scan operands.
                self.expr(target);
                self.expr(value);
            }
            ExprKind::PostfixIncrement(e) | ExprKind::PostfixDecrement(e) => self.expr(e),
            ExprKind::Binary { left, right, .. }
            | ExprKind::Pipeline { left, right }
            | ExprKind::LogicalAnd { left, right }
            | ExprKind::LogicalOr { left, right }
            | ExprKind::NullishCoalescing { left, right } => {
                self.expr(left);
                self.expr(right);
            }
            ExprKind::Unary { operand, .. } => self.expr(operand),
            ExprKind::Grouping(e) | ExprKind::Spread(e) | ExprKind::Throw(e) => self.expr(e),
            ExprKind::Call { callee, arguments } => {
                self.expr(callee);
                self.arguments(arguments);
            }
            ExprKind::New {
                class_expr,
                arguments,
            } => {
                self.expr(class_expr);
                self.arguments(arguments);
            }
            ExprKind::Member { object, .. } | ExprKind::SafeMember { object, .. } => {
                self.expr(object)
            }
            ExprKind::QualifiedName { qualifier, .. } => self.expr(qualifier),
            ExprKind::Index { object, index } => {
                self.expr(object);
                self.expr(index);
            }
            ExprKind::Array(elems) => {
                for e in elems {
                    self.expr(e);
                }
            }
            ExprKind::Hash(pairs) => {
                for (k, v) in pairs {
                    self.expr(k);
                    self.expr(v);
                }
            }
            ExprKind::Block(stmts) => {
                for s in stmts {
                    self.stmt(s);
                }
            }
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.expr(condition);
                self.expr(then_branch);
                if let Some(eb) = else_branch {
                    self.expr(eb);
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
                variable,
                iterable,
                condition,
            } => {
                self.declared.insert(variable.clone());
                self.expr(element);
                self.expr(iterable);
                if let Some(c) = condition {
                    self.expr(c);
                }
            }
            ExprKind::HashComprehension {
                key,
                value,
                variable,
                iterable,
                condition,
            } => {
                self.declared.insert(variable.clone());
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
            ExprKind::InterpolatedString(parts) => {
                for part in parts {
                    if let InterpolatedPart::Expression(e) = part {
                        self.expr(e);
                    }
                }
            }
            // Separate scope — not scanned (lambdas hoist their own locals).
            ExprKind::Lambda { .. } => {}
            // Leaves / nothing to scan.
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
        }
    }

    fn arguments(&mut self, arguments: &[Argument]) {
        for arg in arguments {
            match arg {
                Argument::Positional(e) | Argument::Block(e) => self.expr(e),
                Argument::Named(named) => self.expr(&named.value),
            }
        }
    }
}
