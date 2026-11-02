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
//!
//! Declaring at entry costs each local a `Null` there, and every assignment
//! then stores into the slot — the first one too, `SetLocal` plus the
//! statement's `Pop` — where `let x = v` leaves `v` on the stack as the slot.
//! So [`in_place_declarations`] picks out the locals that can be declared
//! where they are first assigned instead: see there for when that is
//! invisible.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;

use crate::ast::expr::{Argument, Expr, ExprKind, InterpolatedPart, MatchPattern};
use crate::ast::stmt::{ImportSpecifier, Stmt, StmtKind};
use crate::ast::walk;
use crate::error::CompileError;

use super::compiler::{CompileResult, Compiler};

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
            }
            | StmtKind::Unless {
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

/// A top-level `name = value` statement planned to declare `name` where it
/// stands rather than assign a slot hoisted to function entry
/// ([`Compiler::hoist_locals`]). The entry script plans its top-level
/// variables the same way, `let` and `const` included
/// ([`Compiler::compile_entry_script`]).
#[derive(Debug, Clone)]
pub struct InPlaceDecl {
    /// The statement, matched by address: only the statement planned from
    /// the body being compiled can match, never a nested one and never a copy
    /// (a `finally` block is compiled from a clone at each exit).
    pub stmt: *const Stmt,
    pub name: String,
    /// `name` was resolved as a global before its statement was compiled: a
    /// mention [`in_place_declarations`] did not see.
    pub resolved_early: bool,
}

/// The top-level statements of a function body that can declare their local
/// in place: `(index, name)` for each statement `name = value` in `body`
/// whose `name` is one of `candidates` and is *mentioned* by no statement
/// before it and not by its own `value`.
///
/// Such a statement is compiled as `let name = value` is: the value left on
/// the stack is the new local's slot. That is invisible only if nothing can
/// reach the slot before the statement runs, which is what the rule ensures.
/// A top-level statement runs once per call, after every statement before
/// it, and nothing at top level jumps backwards, so if none of them and not
/// `value` names the variable, nothing reads, writes or captures it earlier.
/// Every later mention is compiled after the declaration and resolves to it,
/// as it resolved to the hoisted slot, and the two hold the same value from
/// that statement on.
///
/// "Mentions" is deliberately coarse: any occurrence of the identifier, read
/// or written, declared or bound (a parameter, a loop or catch variable, a
/// match binding, a nested `def`), including inside lambdas and nested
/// `def`s, which could capture the slot. It also covers `#{name}` in an
/// SDBQL block and, in a view, `@name`. A class or enum declared in the body
/// counts as mentioning every name. A name first mentioned anywhere else
/// stays hoisted: one first assigned in a branch or a loop keeps its
/// function-wide slot.
pub(super) fn in_place_declarations<'a>(
    body: &'a [Stmt],
    candidates: &[String],
    template_mode: bool,
) -> Vec<(usize, &'a str)> {
    // Candidates whose first mention is still ahead.
    let mut open: HashSet<&str> = candidates.iter().map(String::as_str).collect();
    let mut found = Vec::new();
    for (index, stmt) in body.iter().enumerate() {
        if open.is_empty() {
            break;
        }
        let declares = assignment_statement(stmt);
        let node = match declares {
            Some((_, value)) => Node::Expr(value),
            None => Node::Stmt(stmt),
        };
        // A class or an enum: every name counts as mentioned from here on.
        let Some(mentioned) = mentions(&open, node, template_mode) else {
            break;
        };
        if let Some((name, _)) = declares {
            if open.remove(name) && !mentioned.contains(name) {
                found.push((index, name));
            }
        }
        for name in mentioned {
            open.remove(name);
        }
    }
    found
}

/// `name = value` standing as a statement of its own: the shape that can
/// declare `name`.
pub(super) fn assignment_statement(stmt: &Stmt) -> Option<(&str, &Expr)> {
    let StmtKind::Expression(expr) = &stmt.kind else {
        return None;
    };
    let ExprKind::Assign { target, value } = &expr.kind else {
        return None;
    };
    match &target.kind {
        ExprKind::Variable(name) => Some((name.as_str(), value.as_ref())),
        _ => None,
    }
}

/// Where [`mentions`] looks: a whole statement, or the value of a
/// `name = value` one.
enum Node<'n> {
    Stmt(&'n Stmt),
    Expr(&'n Expr),
}

/// The names in `open` that `node` mentions anywhere, scopes ignored — or
/// `None` when it declares a class or an enum, which bind and resolve names
/// (their own, a superclass, includes) in ways this does not follow.
fn mentions<'c>(
    open: &HashSet<&'c str>,
    node: Node<'_>,
    template_mode: bool,
) -> Option<HashSet<&'c str>> {
    let found = RefCell::new(HashSet::new());
    let everything = Cell::new(false);
    let note = |name: &str| {
        if let Some(&candidate) = open.get(name) {
            found.borrow_mut().insert(candidate);
        }
    };
    let mut on_stmt = |stmt: &Stmt| match &stmt.kind {
        StmtKind::Let { name, .. } | StmtKind::Const { name, .. } => note(name),
        StmtKind::For {
            variable,
            index_variable,
            ..
        } => {
            note(variable);
            if let Some(index) = index_variable {
                note(index);
            }
        }
        StmtKind::Try { catch_clauses, .. } => {
            for clause in catch_clauses {
                if let Some(var) = &clause.var_name {
                    note(var);
                }
            }
        }
        StmtKind::Function(decl) => {
            note(&decl.name);
            for param in &decl.params {
                note(&param.name);
            }
        }
        StmtKind::Class(_) | StmtKind::Enum(_) => everything.set(true),
        StmtKind::Import(decl) => match &decl.specifier {
            ImportSpecifier::All => {}
            ImportSpecifier::Named(items) => {
                for item in items {
                    note(&item.name);
                    if let Some(alias) = &item.alias {
                        note(alias);
                    }
                }
            }
            ImportSpecifier::Namespace(name) => note(name),
        },
        _ => {}
    };
    let mut on_expr = |expr: &Expr| match &expr.kind {
        ExprKind::Variable(name) => note(name),
        // A view reads, calls and assigns `@name` as its local `name`
        // (`template_ivar`).
        ExprKind::Member { object, name }
            if template_mode && matches!(object.kind, ExprKind::This) =>
        {
            note(name)
        }
        // `#{name}` is read as the variable `name`, from the interpolation's
        // text (`compile_sdql_block`): that name counts, and every word in it.
        ExprKind::SdqlBlock { interpolations, .. } => {
            for interpolation in interpolations {
                let text = interpolation.expr.as_str();
                note(text.trim_start_matches('{').trim_end_matches('}').trim());
                for word in text.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
                    note(word);
                }
            }
        }
        ExprKind::Lambda { params, .. } => {
            for param in params {
                note(&param.name);
            }
        }
        ExprKind::ListComprehension { variable, .. }
        | ExprKind::HashComprehension { variable, .. } => note(variable),
        ExprKind::Match { arms, .. } => {
            for arm in arms {
                pattern_names(&arm.pattern, &note);
            }
        }
        _ => {}
    };
    match node {
        Node::Stmt(stmt) => walk::walk_stmt(stmt, &mut on_stmt, &mut on_expr),
        Node::Expr(expr) => walk::walk_expr(expr, &mut on_stmt, &mut on_expr),
    }
    if everything.get() {
        None
    } else {
        Some(found.into_inner())
    }
}

/// Every name a match pattern binds or names, for [`mentions`] (and the
/// entry script's plan, `compiler_script_locals`).
pub(super) fn pattern_names(pattern: &MatchPattern, note: &dyn Fn(&str)) {
    match pattern {
        MatchPattern::Wildcard => {}
        MatchPattern::Variable(name) => note(name),
        MatchPattern::Typed { name, type_name }
        | MatchPattern::Destructuring { name, type_name } => {
            note(name);
            note(type_name);
        }
        MatchPattern::Literal(ExprKind::Variable(name)) => note(name),
        MatchPattern::Literal(_) => {}
        MatchPattern::Array { elements, rest } => {
            for element in elements {
                pattern_names(element, note);
            }
            if let Some(rest) = rest {
                note(rest);
            }
        }
        MatchPattern::Hash { fields, rest } => {
            for (key, value) in fields {
                note(key);
                pattern_names(value, note);
            }
            if let Some(rest) = rest {
                note(rest);
            }
        }
        MatchPattern::EnumVariant {
            enum_name,
            bindings,
            ..
        } => {
            note(enum_name);
            for binding in bindings {
                pattern_names(binding, note);
            }
        }
    }
}

/// The refusal of a planned declaration whose name was reached before its
/// statement.
fn mentioned_early(decl: &InPlaceDecl, stmt: &Stmt) -> CompileError {
    CompileError::new(
        format!(
            "`{}` was mentioned before the statement that declares it",
            decl.name
        ),
        stmt.span,
    )
}

impl Compiler {
    /// `stmt`'s name and value, when [`Compiler::hoist_locals`] planned it to
    /// declare its local in place. Taken once.
    ///
    /// Refuses the function — it then runs on the interpreter — when the plan
    /// missed a mention: the name was resolved as a global before this
    /// statement (that read would have answered from the hoisted slot), or a
    /// local of that name already exists (the hoisted assignment would have
    /// stored into it, not declared a new one).
    pub(crate) fn take_in_place_decl<'s>(
        &mut self,
        stmt: &'s Stmt,
    ) -> CompileResult<Option<(&'s str, &'s Expr)>> {
        let Some(decl) = self.take_planned(stmt) else {
            return Ok(None);
        };
        match assignment_statement(stmt) {
            Some(found) if !decl.resolved_early && self.resolve_local(found.0).is_none() => {
                Ok(Some(found))
            }
            _ => Err(mentioned_early(&decl, stmt)),
        }
    }

    /// [`Self::take_in_place_decl`] for a `let` or `const` statement:
    /// whether it is planned to declare its local in place, which only the
    /// entry script's top-level variables are
    /// ([`Compiler::compile_entry_script`]). Refused the same way.
    pub(crate) fn take_in_place_binding(&mut self, stmt: &Stmt) -> CompileResult<bool> {
        let Some(decl) = self.take_planned(stmt) else {
            return Ok(false);
        };
        if !decl.resolved_early && self.resolve_local(&decl.name).is_none() {
            Ok(true)
        } else {
            Err(mentioned_early(&decl, stmt))
        }
    }

    /// The plan for `stmt`, matched by address, removed from the pending
    /// ones.
    fn take_planned(&mut self, stmt: &Stmt) -> Option<InPlaceDecl> {
        if self.in_place_decls.is_empty() {
            return None;
        }
        let at = self
            .in_place_decls
            .iter()
            .position(|decl| std::ptr::eq(decl.stmt, stmt))?;
        Some(self.in_place_decls.swap_remove(at))
    }

    /// Whether this function, or one enclosing it, plans to declare `name` in
    /// place at a statement not compiled yet.
    pub(crate) fn in_place_pending(&self, name: &str) -> bool {
        let mut compiler = Some(self);
        while let Some(current) = compiler {
            if current.in_place_decls.iter().any(|decl| decl.name == name) {
                return true;
            }
            compiler = current.enclosing.as_deref();
        }
        false
    }

    /// `name` was just resolved as a global. A pending in-place declaration of
    /// it, here or in an enclosing function, is then marked, so its statement
    /// is refused when it is reached (`take_in_place_decl`).
    pub(crate) fn note_global_resolution(&mut self, name: &str) {
        let mut compiler = Some(self);
        while let Some(current) = compiler {
            for decl in &mut current.in_place_decls {
                if decl.name == name {
                    decl.resolved_early = true;
                }
            }
            compiler = current.enclosing.as_deref_mut();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::stmt::Program;
    use crate::interpreter::value::Value;
    use crate::lexer::Scanner;
    use crate::parser::Parser;
    use crate::vm::chunk::{Constant, FunctionProto};
    use crate::vm::compiler::{FunctionType, VariableAccess};
    use crate::vm::opcode::Op;
    use std::sync::Arc;

    fn parse(src: &str) -> Program {
        let tokens = Scanner::new(src).scan_tokens().expect("lex");
        Parser::new(tokens).parse().expect("parse")
    }

    /// The program `def f(p) { body }`.
    fn function(body: &str) -> Program {
        parse(&format!("def f(p) {{\n{body}\n}}\n"))
    }

    fn body_of(program: &Program) -> &[Stmt] {
        match &program.statements[0].kind {
            StmtKind::Function(decl) => &decl.body,
            other => panic!("expected a def, got {other:?}"),
        }
    }

    /// The bare locals of `def f(p) { body }` that are declared in place.
    fn in_place(body: &str, template_mode: bool) -> Vec<String> {
        let program = function(body);
        let body = body_of(&program);
        let candidates = collect_hoisted_locals(body);
        in_place_declarations(body, &candidates, template_mode)
            .into_iter()
            .map(|(_, name)| name.to_string())
            .collect()
    }

    /// The first function proto named `name` under `proto`, depth first.
    fn proto_named(proto: &FunctionProto, name: &str) -> Option<Arc<FunctionProto>> {
        proto
            .chunk
            .constants
            .iter()
            .find_map(|constant| match constant {
                Constant::Function(inner) if inner.name == name => Some(inner.clone()),
                Constant::Function(inner) => proto_named(inner, name),
                _ => None,
            })
    }

    fn compiled_fn(body: &str) -> Arc<FunctionProto> {
        let module = Compiler::compile(&function(body)).expect("compile");
        proto_named(&module.main, "f").expect("the proto of f")
    }

    /// `f`'s instructions up to its first `Return`: the straight-line path
    /// a call of the test bodies runs.
    fn ops_to_return(body: &str) -> Vec<Op> {
        let code = compiled_fn(body).chunk.code.clone();
        let end = code.iter().position(|op| matches!(op, Op::Return));
        code[..=end.expect("a Return")].to_vec()
    }

    /// Call `f(7)` on the VM and show what it returns.
    fn run_fn(body: &str) -> String {
        let program = parse(&format!("def f(p) {{\n{body}\n}}\nresult = f(7)\n"));
        run_program_result(&program)
    }

    /// Run `program` on a VM seeded with the builtins and show its `result`.
    fn run_program_result(program: &Program) -> String {
        let module = Compiler::compile(program).expect("compile");
        let interpreter = crate::interpreter::Interpreter::new();
        let mut vm = crate::vm::Vm::new();
        for (name, value) in interpreter.environment.borrow().get_all_bindings() {
            vm.globals.insert(name, value);
        }
        if let Err(err) = vm.execute(&module.main) {
            panic!("vm: {err}");
        }
        let result: &Value = vm.globals.get("result").expect("a result global");
        result.to_string()
    }

    // ---------- which statements declare in place ----------

    #[test]
    fn straight_line_first_assignments_declare_in_place() {
        assert_eq!(
            in_place("a = 1\nb = a + p\nc = [a, b]\nc", false),
            ["a", "b", "c"]
        );
    }

    #[test]
    fn a_later_reassignment_does_not_matter() {
        assert_eq!(
            in_place("a = 1\nif p { a = 2 }\nwhile a < 9 { a = a + 1 }\na", false),
            ["a"]
        );
    }

    #[test]
    fn a_read_before_the_first_assignment_keeps_the_name_hoisted() {
        assert_eq!(in_place("seen = x\nx = 1\nx", false), ["seen"]);
    }

    #[test]
    fn the_assignments_own_value_counts_as_a_mention() {
        assert_eq!(in_place("x = [x]\ny = 2\n[x, y]", false), ["y"]);
    }

    #[test]
    fn a_closure_or_nested_def_made_earlier_could_capture_the_slot() {
        assert_eq!(
            in_place(
                "get = fn() { x }\nx = 5\ndef helper() { y }\ny = 1\n[get(), helper()]",
                false
            ),
            ["get"]
        );
    }

    #[test]
    fn a_first_assignment_in_a_branch_or_a_loop_stays_hoisted() {
        assert_eq!(
            in_place(
                "if p { a = 1 }\na = 2\nwhile p { b = 1 }\nfor i in [1] { c = i }\nd = 3\n[a, d]",
                false
            ),
            ["d"]
        );
    }

    #[test]
    fn a_tail_assignment_declares_in_place() {
        assert_eq!(in_place("a = p * 2", false), ["a"]);
    }

    #[test]
    fn an_sdbql_interpolation_mentions_its_variable() {
        assert_eq!(
            in_place(
                "rows = @sdbql{ FOR u IN users FILTER u.age > #{min_age} RETURN u }\nmin_age = 1\nrows",
                false
            ),
            ["rows"]
        );
    }

    #[test]
    fn in_a_view_an_ivar_mentions_the_local_of_that_name() {
        let body = "shown = @title\ntitle = 1\n[shown, title]";
        assert_eq!(in_place(body, true), ["shown"]);
        assert_eq!(in_place(body, false), ["shown", "title"]);
    }

    #[test]
    fn match_bindings_and_lambda_parameters_count_as_mentions() {
        assert_eq!(
            in_place(
                "r = match p { x => x }\nx = 1\nsq = fn(y) { y * y }\ny = 2\n[r, x, sq(y)]",
                false
            ),
            ["r", "sq"]
        );
    }

    #[test]
    fn a_class_declared_in_the_body_ends_the_plan() {
        assert_eq!(
            in_place("a = 1\nclass Inner\nend\nb = 2\n[a, b]", false),
            ["a"]
        );
    }

    #[test]
    fn only_candidates_are_planned() {
        let program = function("a = 1\nb = 2\n[a, b]");
        let body = body_of(&program);
        let planned = in_place_declarations(body, &["b".to_string()], false);
        assert_eq!(planned, [(1, "b")]);
    }

    // ---------- what is emitted ----------

    /// Straight-line bare locals compile to exactly the bytecode of the same
    /// body written with `let`: no `Null` at entry, no `SetLocal`, no `Pop`.
    #[test]
    fn straight_line_bare_locals_compile_like_let() {
        let bare = compiled_fn("a = p + 1\nb = a * 2\nc = [a, b]\nc")
            .chunk
            .code
            .clone();
        let with_let = compiled_fn("let a = p + 1\nlet b = a * 2\nlet c = [a, b]\nc")
            .chunk
            .code
            .clone();
        assert_eq!(bare, with_let);
        let path = ops_to_return("a = p + 1\nb = a * 2\nc = [a, b]\nc");
        assert!(
            !path.iter().any(|op| matches!(
                op,
                Op::Null | Op::PushNulls(_) | Op::SetLocal(_) | Op::SetLocalPop(_) | Op::Pop
            )),
            "got: {path:?}"
        );
    }

    #[test]
    fn branch_assigned_locals_share_one_push_nulls() {
        let path = ops_to_return("if p > 0 {\n  a = 1\n  b = 2\n  c = 3\n}\n0");
        assert_eq!(path[0], Op::PushNulls(3), "got: {path:?}");
        assert!(
            !path.iter().any(|op| matches!(op, Op::Null)),
            "got: {path:?}"
        );
    }

    #[test]
    fn a_single_hoisted_local_is_one_plain_null() {
        let path = ops_to_return("if p > 0 {\n  a = 1\n}\n0");
        assert_eq!(path[0], Op::Null, "got: {path:?}");
        assert!(
            !path.iter().any(|op| matches!(op, Op::PushNulls(_))),
            "got: {path:?}"
        );
    }

    #[test]
    fn methods_constructors_and_blocks_declare_in_place_too() {
        let program = parse(
            "class Invoice\n  total: Int\n  new(amount)\n    tax = amount / 10\n    @total = amount + tax\n  end\n  def summary(rate)\n    base = @total\n    extra = base * rate\n    [base, extra]\n  end\nend\ndoubled = [1, 2, 3].map do |x|\n  twice = x * 2\n  twice + 1\nend\n",
        );
        let module = Compiler::compile(&program).expect("compile");
        for name in ["init", "summary", "<lambda>"] {
            let proto = proto_named(&module.main, name).expect(name);
            let code = &proto.chunk.code;
            let end = code
                .iter()
                .position(|op| matches!(op, Op::Return))
                .expect("a Return");
            assert!(
                !code[..end].iter().any(|op| matches!(
                    op,
                    Op::Null | Op::PushNulls(_) | Op::SetLocal(_) | Op::SetLocalPop(_)
                )),
                "{name}: {:?}",
                code
            );
        }
    }

    // ---------- what it computes ----------

    #[test]
    fn a_read_before_the_first_assignment_sees_nil() {
        assert_eq!(run_fn("seen = x\nx = 5\n[seen, x]"), "[null, 5]");
    }

    #[test]
    fn a_closure_made_before_the_first_assignment_sees_the_later_value() {
        assert_eq!(run_fn("get = fn() { x }\nx = p\nget()"), "7");
    }

    #[test]
    fn the_assignments_own_value_reads_nil() {
        assert_eq!(run_fn("x = [x]\nx"), "[null]");
    }

    #[test]
    fn a_tail_assignment_returns_its_value() {
        assert_eq!(run_fn("a = p * 2\nb = a * 3"), "42");
    }

    /// In-place, hoisted and `let` locals side by side, a `for` loop and a
    /// branch over them, and a closure that captures an in-place local and
    /// sees it change afterwards.
    #[test]
    fn mixed_locals_keep_their_values() {
        assert_eq!(
            run_fn(
                "total = 0\nitems = [1, 2, 3]\nlet factor = p\nfor item in items {\n  total = total + item * factor\n}\nif total > 0 {\n  label = \"positive\"\n} else {\n  label = \"zero\"\n}\nadd = fn(n) { total + n }\ntotal = total + 1\n[total, label, add(100)]"
            ),
            "[43, positive, 143]"
        );
    }

    #[test]
    fn methods_constructors_and_blocks_compute_the_same() {
        let program = parse(
            "class Invoice\n  total: Int\n  new(amount)\n    tax = amount / 10\n    @total = amount + tax\n  end\n  def summary(rate)\n    base = @total\n    extra = base * rate\n    [base, extra]\n  end\nend\ninv = new Invoice(100)\ndoubled = [1, 2, 3].map do |x|\n  twice = x * 2\n  twice + 1\nend\nresult = [inv.summary(2), doubled]\n",
        );
        assert_eq!(run_program_result(&program), "[[110, 220], [3, 5, 7]]");
    }

    // ---------- the plan cannot be broken silently ----------

    /// A mention the scan missed shows up as a global resolution of a name
    /// still waiting for its declaration: the function is refused, so it
    /// runs on the interpreter instead of reading the global.
    #[test]
    fn a_pending_name_resolved_as_a_global_refuses_the_function() {
        let program = function("x = 1\nx");
        let body = body_of(&program);
        let mut compiler = Compiler::new(FunctionType::Function, "f".to_string());
        compiler.begin_scope();
        compiler.hoist_locals(body, 1);
        assert_eq!(compiler.in_place_decls.len(), 1);
        assert!(matches!(
            compiler.resolve_variable("x"),
            VariableAccess::Global(_)
        ));
        let err = compiler
            .take_in_place_decl(&body[0])
            .expect_err("the declaration must be refused");
        assert!(err.to_string().contains("`x`"), "{err}");
    }

    /// The planned statement compiled the ordinary way would store to a
    /// global: refused too.
    #[test]
    fn a_global_store_to_a_pending_name_is_refused() {
        let program = function("x = 1\nx");
        let body = body_of(&program);
        let mut compiler = Compiler::new(FunctionType::Function, "f".to_string());
        compiler.begin_scope();
        compiler.hoist_locals(body, 1);
        let StmtKind::Expression(assign) = &body[0].kind else {
            panic!("expected an assignment statement");
        };
        assert!(compiler.compile_expr(assign).is_err());
    }
}
