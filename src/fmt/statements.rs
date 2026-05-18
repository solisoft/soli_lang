//! Statement printer.

use crate::ast::expr::{Argument, Expr, ExprKind};

use super::printer::MAX_LINE_LENGTH;

#[derive(Clone, Copy)]
pub(super) enum PostfixIfKind {
    If,
    Unless,
}

/// Decide whether the source bytes at `start` introduce a *block* `if` or a
/// *postfix* `if`/`unless`. We look at the first non-whitespace token: if it
/// is the keyword `if` we're in block form; otherwise the statement begins
/// with an expression and a postfix keyword appears later — find it.
pub(super) fn detect_postfix_if_kind(source: &str, start: usize) -> Option<PostfixIfKind> {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut i = start.min(len);
    while i < len && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    if starts_with_keyword(bytes, i, b"if") {
        return None; // block form
    }
    // Postfix form. Walk forward looking for ` if ` / ` unless ` at the top
    // level (depth 0) of brackets/parens/braces — the keyword that turns the
    // expression into the conditional is unbracketed.
    let mut depth: i32 = 0;
    let mut j = i;
    let mut quote: Option<u8> = None;
    while j < len {
        let c = bytes[j];
        if let Some(q) = quote {
            if c == b'\\' && j + 1 < len {
                j += 2;
                continue;
            }
            if c == q {
                quote = None;
            }
            j += 1;
            continue;
        }
        match c {
            b'"' | b'\'' | b'`' => quote = Some(c),
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b'\n' => break,
            b'i' | b'u' if depth == 0 => {
                if starts_with_keyword(bytes, j, b"if") {
                    // Make sure the preceding char is whitespace — avoid
                    // matching identifiers like `i` or `notify`.
                    if j > start && bytes[j - 1].is_ascii_whitespace() {
                        return Some(PostfixIfKind::If);
                    }
                }
                if starts_with_keyword(bytes, j, b"unless")
                    && j > start
                    && bytes[j - 1].is_ascii_whitespace()
                {
                    return Some(PostfixIfKind::Unless);
                }
            }
            _ => {}
        }
        j += 1;
    }
    None
}

fn starts_with_keyword(bytes: &[u8], at: usize, kw: &[u8]) -> bool {
    if at + kw.len() > bytes.len() {
        return false;
    }
    if &bytes[at..at + kw.len()] != kw {
        return false;
    }
    // Word boundary: next byte (if any) must NOT be an identifier char.
    let next = bytes.get(at + kw.len()).copied();
    match next {
        Some(c) => !(c.is_ascii_alphanumeric() || c == b'_'),
        None => true,
    }
}

use crate::ast::stmt::{
    CatchClause, ClassDecl, ConstructorDecl, FieldDecl, FunctionDecl, ImportDecl, ImportSpecifier,
    InterfaceDecl, MethodDecl, Parameter, Stmt, StmtKind, Visibility,
};

use super::printer::Printer;

impl Printer<'_> {
    pub(super) fn print_stmt(&mut self, stmt: &Stmt) {
        self.flush_comments_before(stmt.span.line);
        // A trailing `# soli-lint-disable-line` would be detached from its
        // target line whenever the formatter alters layout (e.g. breaking a
        // long expression across lines), silently disabling the suppression.
        // Rewrite it as `# soli-lint-disable-next-line` placed just above the
        // statement — same effect, robust against line splits.
        self.rewrite_trailing_lint_disable(stmt.span.line);
        // Reset the postfix-rewrite flag; the if-branch below sets it back
        // to true when we actually rewrite a block-if to postfix form.
        self.last_stmt_rewrote_to_postfix = false;
        match &stmt.kind {
            StmtKind::Expression(expr) => {
                // At statement position, `fn` is a function declaration and
                // requires a name — so a bare `fn(...) { ... }` lambda
                // expression-statement is illegal. Wrap it in parens to keep
                // it an expression. (Common case: implicit-return of an inner
                // lambda from the last statement of an outer lambda body.)
                let wrap = matches!(expr.kind, ExprKind::Lambda { .. });
                if wrap {
                    self.write("(");
                }
                self.print_expr(expr);
                if wrap {
                    self.write(")");
                }
                self.newline();
            }
            StmtKind::Let {
                name,
                type_annotation,
                initializer,
            } => {
                self.write("let ");
                self.write(name);
                if let Some(ty) = type_annotation {
                    self.write(": ");
                    self.write(&format_type(ty));
                }
                if let Some(init) = initializer {
                    self.write(" = ");
                    self.print_expr(init);
                }
                self.newline();
            }
            StmtKind::Const {
                name,
                type_annotation,
                initializer,
            } => {
                self.write("const ");
                self.write(name);
                if let Some(ty) = type_annotation {
                    self.write(": ");
                    self.write(&format_type(ty));
                }
                self.write(" = ");
                self.print_expr(initializer);
                self.newline();
            }
            StmtKind::Block(stmts) => {
                // Bare blocks `{ ... }` introduce a nested scope (e.g. for
                // variable shadowing). Preserve the braces or the nested
                // scope collapses into the enclosing one.
                self.write("{");
                self.newline();
                self.print_block_body(stmts);
                self.write("}");
                self.newline();
            }
            StmtKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                // Postfix `expr if cond` / `expr unless cond` lowers to the
                // same StmtKind::If as block `if cond ... end`. Inspect the
                // source bytes at the statement's start to recover the form.
                if let Some(kw) = detect_postfix_if_kind(self.source, stmt.span.start) {
                    self.print_postfix_if(condition, then_branch, kw);
                } else if let Some(inner) = self.guard_clause_to_rewrite(
                    stmt, condition, then_branch, else_branch.is_some(),
                ) {
                    // Block form `if cond ... end` with a single guard-style
                    // body collapses to idiomatic postfix `expr if cond`.
                    self.print_postfix_if(condition, inner, PostfixIfKind::If);
                    self.last_stmt_rewrote_to_postfix = true;
                } else {
                    self.print_if(condition, then_branch, else_branch.as_deref());
                    self.maybe_blank_line_after_guard(then_branch, else_branch.as_deref());
                }
            }
            StmtKind::While { condition, body } => {
                self.write("while ");
                self.print_expr(condition);
                self.newline();
                self.print_block_or_stmt(body);
                self.write("end");
                self.newline();
            }
            StmtKind::For {
                variable,
                index_variable,
                iterable,
                body,
            } => {
                self.write("for ");
                self.write(variable);
                if let Some(idx) = index_variable {
                    self.write(", ");
                    self.write(idx);
                }
                self.write(" in ");
                self.print_expr(iterable);
                self.newline();
                self.print_block_or_stmt(body);
                self.write("end");
                self.newline();
            }
            StmtKind::Return(expr) => {
                self.write("return");
                if let Some(e) = expr {
                    self.write(" ");
                    self.print_expr(e);
                }
                self.newline();
            }
            StmtKind::Throw(expr) => {
                self.write("throw ");
                self.print_expr(expr);
                self.newline();
            }
            StmtKind::Try {
                try_block,
                catch_clauses,
                finally_block,
            } => {
                self.write("try");
                self.newline();
                self.print_block_or_stmt(try_block);
                for clause in catch_clauses {
                    self.print_catch_clause(clause);
                }
                if let Some(fb) = finally_block {
                    self.write("finally");
                    self.newline();
                    self.print_block_or_stmt(fb);
                }
                self.write("end");
                self.newline();
            }
            StmtKind::Function(decl) => self.print_function_decl(decl, false),
            StmtKind::Class(decl) => self.print_class_decl(decl),
            StmtKind::Interface(decl) => self.print_interface_decl(decl),
            StmtKind::Import(decl) => self.print_import_decl(decl),
            StmtKind::Export(inner) => {
                self.write("export ");
                self.print_stmt(inner);
            }
        }
        self.flush_trailing_comments_on(stmt.span.line);
        self.record_emitted_line(stmt.span.line);
    }

    /// Decide whether a block-form `if cond ... end` should be rewritten as
    /// the idiomatic postfix `expr if cond`. Returns the inner statement to
    /// emit on the postfix line, or `None` to keep the block form.
    ///
    /// Conditions for rewrite:
    /// - No `else` / `elsif` branch (postfix has no else form)
    /// - Body is a single Return / Throw / Expression statement
    /// - No comments anywhere inside the block (they'd be detached)
    /// - The resulting postfix line fits within `MAX_LINE_LENGTH`
    fn guard_clause_to_rewrite<'s>(
        &self,
        if_stmt: &Stmt,
        condition: &Expr,
        then_branch: &'s Stmt,
        else_present: bool,
    ) -> Option<&'s Stmt> {
        if else_present {
            return None;
        }
        let inner = match &then_branch.kind {
            StmtKind::Block(stmts) if stmts.len() == 1 => &stmts[0],
            _ => return None,
        };
        if !matches!(
            inner.kind,
            StmtKind::Return(_) | StmtKind::Throw(_) | StmtKind::Expression(_)
        ) {
            return None;
        }
        let start_line = if_stmt.span.line;
        let end_line = super::printer::source_end_line(self.source, if_stmt.span);
        if self.has_comments_in_lines(start_line, end_line + 1) {
            return None;
        }
        // The inner stmt must fit on a single source line. Multi-line
        // postfix (e.g. `return {\n  ...\n} if cond`) parses fine, but
        // `detect_postfix_if_kind` only scans within the first line and
        // would misclassify the output as block form on the next pass —
        // flipping back and forth and breaking idempotency.
        let inner_end_line = super::printer::source_end_line(self.source, inner.span);
        if inner_end_line != inner.span.line {
            return None;
        }
        let cond_end_line = super::printer::source_end_line(self.source, condition.span);
        if cond_end_line != condition.span.line {
            return None;
        }
        // Even when the source is single-line, the printer's own break
        // heuristics may multi-line a hash/array value (e.g. a 2-pair
        // hash whose value is a `+` concat is forced multi-line). Don't
        // rewrite then — the postfix would degrade to multi-line output.
        if expr_in_stmt_likely_breaks(inner) {
            return None;
        }
        if expr_likely_breaks(condition) {
            return None;
        }
        let cond_w = condition.span.end.saturating_sub(condition.span.start);
        let inner_w = inner.span.end.saturating_sub(inner.span.start);
        let total = self.current_column() + inner_w + 4 /* " if " */ + cond_w;
        if total > MAX_LINE_LENGTH {
            return None;
        }
        Some(inner)
    }

    /// Emit `expr if cond` / `expr unless cond`. The condition stored on the
    /// AST for `unless` form is `Unary{Not, inner}` (the parser desugars
    /// `expr unless cond` to `if !cond`); we strip that wrapper so the
    /// printed condition is the original `cond`.
    fn print_postfix_if(&mut self, condition: &Expr, then_branch: &Stmt, kind: PostfixIfKind) {
        // The then_branch is `Stmt::Expression(expr)` — print the inner expr
        // directly (no trailing newline yet), then the keyword and condition.
        match &then_branch.kind {
            StmtKind::Expression(e) => self.print_expr(e),
            StmtKind::Return(opt) => {
                self.write("return");
                if let Some(e) = opt {
                    self.write(" ");
                    self.print_expr(e);
                }
            }
            StmtKind::Throw(e) => {
                self.write("throw ");
                self.print_expr(e);
            }
            // Block-bodied postfix should not exist (parser wraps a single
            // statement in Expression/Return/Throw). Fall back to recursing
            // through `print_stmt` to be safe.
            _ => self.print_stmt(then_branch),
        }
        match kind {
            PostfixIfKind::If => {
                self.write(" if ");
                self.print_expr(condition);
            }
            PostfixIfKind::Unless => {
                self.write(" unless ");
                // The parser wraps the cond in `!cond` — strip it.
                if let ExprKind::Unary {
                    operator: crate::ast::expr::UnaryOp::Not,
                    operand,
                } = &condition.kind
                {
                    self.print_expr(operand);
                } else {
                    self.print_expr(condition);
                }
            }
        }
        self.newline();
    }

    fn print_if(&mut self, condition: &Expr, then_branch: &Stmt, else_branch: Option<&Stmt>) {
        self.write("if ");
        self.print_expr(condition);
        self.newline();
        self.print_block_or_stmt(then_branch);
        match else_branch {
            None => {
                self.write("end");
                self.newline();
            }
            Some(else_stmt) => {
                // `elsif` chain: an `else { if ... }` collapses to `elsif ...`.
                if let StmtKind::If {
                    condition: c2,
                    then_branch: t2,
                    else_branch: e2,
                } = &else_stmt.kind
                {
                    self.write("elsif ");
                    self.print_expr(c2);
                    self.newline();
                    self.print_block_or_stmt(t2);
                    self.print_if_tail(e2.as_deref());
                } else {
                    self.write("else");
                    self.newline();
                    self.print_block_or_stmt(else_stmt);
                    self.write("end");
                    self.newline();
                }
            }
        }
    }

    fn print_if_tail(&mut self, else_branch: Option<&Stmt>) {
        match else_branch {
            None => {
                self.write("end");
                self.newline();
            }
            Some(else_stmt) => {
                if let StmtKind::If {
                    condition: c2,
                    then_branch: t2,
                    else_branch: e2,
                } = &else_stmt.kind
                {
                    self.write("elsif ");
                    self.print_expr(c2);
                    self.newline();
                    self.print_block_or_stmt(t2);
                    self.print_if_tail(e2.as_deref());
                } else {
                    self.write("else");
                    self.newline();
                    self.print_block_or_stmt(else_stmt);
                    self.write("end");
                    self.newline();
                }
            }
        }
    }

    /// If `then_branch` is a guard clause (one statement that's a `return`,
    /// `throw`, or unconditional flow exit) and there's no `else`, emit a
    /// blank line after the `end` to separate the guard from the rest of
    /// the method body — Ruby/Rails style.
    fn maybe_blank_line_after_guard(&mut self, then_branch: &Stmt, else_branch: Option<&Stmt>) {
        if else_branch.is_some() {
            return;
        }
        if !is_guard_body(then_branch) {
            return;
        }
        self.blank_line();
    }

    fn print_block_or_stmt(&mut self, stmt: &Stmt) {
        if let StmtKind::Block(stmts) = &stmt.kind {
            self.print_block_body(stmts);
        } else {
            // Single statement: still indent it as a block body.
            self.with_indent(|p| {
                p.print_stmt(stmt);
                if !p.is_at_line_start() {
                    p.newline();
                }
            });
        }
    }

    fn print_catch_clause(&mut self, clause: &CatchClause) {
        self.write("catch");
        if let Some(ty) = &clause.type_name {
            self.write(" ");
            self.write(ty);
        }
        if let Some(v) = &clause.var_name {
            self.write(" ");
            self.write(v);
        }
        self.newline();
        self.print_block_or_stmt(&clause.body);
    }

    pub(super) fn print_function_decl(&mut self, decl: &FunctionDecl, is_method: bool) {
        let keyword = if is_method { "def" } else { "fn" };
        self.write(keyword);
        self.write(" ");
        self.write(&decl.name);
        // Free-standing `fn` may omit empty parens (Soli convention:
        // "Optional parentheses for no-param functions"). Methods keep
        // their parens to match the project's `def name() ... end` style.
        // Also keep empty parens when the body's first statement starts
        // with `(` — otherwise the parser would consume that `(` as the
        // parameter list (e.g. `fn f` followed by `(x ?? "") == "y"`).
        if !decl.params.is_empty() || is_method || body_starts_with_paren(&decl.body) {
            self.print_param_list(&decl.params);
        }
        if let Some(ret) = &decl.return_type {
            self.write(" -> ");
            self.write(&format_type(ret));
        }
        self.newline();
        self.print_block_body(&decl.body);
        self.write("end");
        self.newline();
    }

    fn print_method_decl(&mut self, decl: &MethodDecl) {
        if decl.is_static {
            self.write("static def ");
        } else {
            self.write("def ");
        }
        self.write(&decl.name);
        // Methods always keep parens, even when empty, to match the
        // task-orchestrator-style `static def run_state_root()` convention.
        self.print_param_list(&decl.params);
        if let Some(ret) = &decl.return_type {
            self.write(" -> ");
            self.write(&format_type(ret));
        }
        self.newline();
        self.print_block_body(&decl.body);
        self.write("end");
        self.newline();
    }

    fn print_constructor_decl(&mut self, decl: &ConstructorDecl) {
        self.write("new");
        self.print_param_list(&decl.params);
        self.newline();
        self.print_block_body(&decl.body);
        self.write("end");
        self.newline();
    }

    pub(super) fn print_param_list(&mut self, params: &[Parameter]) {
        // Estimate inline width and break to multi-line if it would exceed.
        let est: usize = params.iter().map(|p| p.name.len() + 2).sum::<usize>() + 2;
        if params.len() > 3 && self.current_column() + est > MAX_LINE_LENGTH {
            self.write("(");
            self.newline();
            self.with_indent(|p| {
                for (i, param) in params.iter().enumerate() {
                    if i > 0 {
                        p.write(",");
                        p.newline();
                    }
                    p.write_param(param);
                }
            });
            self.newline();
            self.write(")");
        } else {
            self.write("(");
            for (i, p) in params.iter().enumerate() {
                if i > 0 {
                    self.write(", ");
                }
                self.write_param(p);
            }
            self.write(")");
        }
    }

    fn write_param(&mut self, p: &Parameter) {
        if p.is_block_param {
            self.write("&");
        }
        self.write(&p.name);
        let ty_str = format_type(&p.type_annotation);
        if !ty_str.is_empty() && ty_str != "Any" {
            self.write(": ");
            self.write(&ty_str);
        }
        if let Some(def) = &p.default_value {
            self.write(" = ");
            self.print_expr(def);
        }
    }

    fn print_class_decl(&mut self, decl: &ClassDecl) {
        self.write("class ");
        self.write(&decl.name);
        if let Some(sup) = &decl.superclass {
            self.write(" < ");
            self.write(sup);
        }
        if !decl.interfaces.is_empty() {
            self.write(" implements ");
            for (i, iface) in decl.interfaces.iter().enumerate() {
                if i > 0 {
                    self.write(", ");
                }
                self.write(iface);
            }
        }
        self.newline();
        self.with_indent(|p| {
            // Fields
            for field in &decl.fields {
                p.flush_comments_before(field.span.line);
                p.print_field_decl(field);
            }
            if !decl.fields.is_empty() && (!decl.methods.is_empty() || decl.constructor.is_some()) {
                p.blank_line();
            }
            // Constructor
            if let Some(ctor) = &decl.constructor {
                p.flush_comments_before(ctor.span.line);
                p.print_constructor_decl(ctor);
                if !decl.methods.is_empty() {
                    p.blank_line();
                }
            }
            // Class-level statements (DSL: validates, before_save, etc.)
            for s in &decl.class_statements {
                p.print_stmt(s);
            }
            if !decl.class_statements.is_empty() && !decl.methods.is_empty() {
                p.blank_line();
            }
            // Static block — parser requires `static { ... }` (braces, no
            // `end` form). See parser/declarations.rs::parse_static_block.
            if let Some(static_stmts) = &decl.static_block {
                p.write("static {");
                p.newline();
                p.with_indent(|pp| {
                    for s in static_stmts {
                        pp.print_stmt(s);
                    }
                });
                p.write("}");
                p.newline();
                if !decl.methods.is_empty() {
                    p.blank_line();
                }
            }
            // Methods
            for (i, m) in decl.methods.iter().enumerate() {
                if i > 0 {
                    p.blank_line();
                }
                // Flush comments that sit ABOVE this method declaration
                // so they're emitted as leading comments to the method,
                // not picked up later by `print_stmt` and dropped inside
                // the body before the first statement.
                p.flush_comments_before(m.span.line);
                p.print_method_decl(m);
            }
            // Nested classes
            if !decl.nested_classes.is_empty() {
                p.blank_line();
                for (i, n) in decl.nested_classes.iter().enumerate() {
                    if i > 0 {
                        p.blank_line();
                    }
                    p.flush_comments_before(n.span.line);
                    p.print_class_decl(n);
                }
            }
        });
        self.write("end");
        self.newline();
    }

    fn print_field_decl(&mut self, field: &FieldDecl) {
        match field.visibility {
            Visibility::Public => {}
            Visibility::Private => self.write("private "),
            Visibility::Protected => self.write("protected "),
        }
        if field.is_static {
            self.write("static ");
        }
        if field.is_const {
            self.write("const ");
        }
        self.write(&field.name);
        // Regular (non-const) fields require a `: Type` annotation —
        // the parser rejects bare `name` (see parser/declarations.rs::
        // parse_field). Const fields may omit the type. Always emit the
        // annotation if present, even when it's `Any`, so the output
        // re-parses.
        if let Some(ty) = &field.type_annotation {
            let ty_str = format_type(ty);
            if !ty_str.is_empty() {
                self.write(": ");
                self.write(&ty_str);
            }
        } else if !field.is_const {
            // AST has no annotation but parser requires one — emit `Any`
            // as the safest default so the output still parses.
            self.write(": Any");
        }
        if let Some(init) = &field.initializer {
            self.write(" = ");
            self.print_expr(init);
        }
        self.newline();
    }

    fn print_interface_decl(&mut self, decl: &InterfaceDecl) {
        // Soli's parser only accepts `interface X { fn m() ... }` — braces are
        // required, and methods use `fn`, not `def` (see parser/declarations
        // .rs::interface_declaration / parse_interface_method).
        self.write("interface ");
        self.write(&decl.name);
        self.write(" {");
        self.newline();
        self.with_indent(|p| {
            for m in &decl.methods {
                p.write("fn ");
                p.write(&m.name);
                p.print_param_list(&m.params);
                if let Some(ret) = &m.return_type {
                    p.write(" -> ");
                    p.write(&format_type(ret));
                }
                p.newline();
            }
        });
        self.write("}");
        self.newline();
    }

    fn print_import_decl(&mut self, decl: &ImportDecl) {
        self.write("import ");
        match &decl.specifier {
            ImportSpecifier::All => {}
            ImportSpecifier::Named(items) => {
                self.write("{ ");
                for (i, it) in items.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.write(&it.name);
                    if let Some(alias) = &it.alias {
                        self.write(" as ");
                        self.write(alias);
                    }
                }
                self.write(" } from ");
            }
            ImportSpecifier::Namespace(name) => {
                self.write("* as ");
                self.write(name);
                self.write(" from ");
            }
        }
        self.write("\"");
        self.write(&decl.path);
        self.write("\"");
        self.newline();
    }
}

/// Heuristic: a body is a "guard clause" if it's a single `return` or `throw`,
/// or a Block containing exactly one such statement.
fn is_guard_body(stmt: &Stmt) -> bool {
    match &stmt.kind {
        StmtKind::Return(_) | StmtKind::Throw(_) => true,
        StmtKind::Block(stmts) => {
            stmts.len() == 1 && matches!(stmts[0].kind, StmtKind::Return(_) | StmtKind::Throw(_))
        }
        StmtKind::Expression(e) => matches!(e.kind, ExprKind::Throw(_)),
        _ => false,
    }
}

fn format_type(ty: &crate::ast::types::TypeAnnotation) -> String {
    ty.to_string()
}

/// Mirror the printer's break heuristics for collection literals. Returns
/// true when `e` (or some sub-expression of `e`) is one the printer will
/// emit across multiple lines, regardless of the source's original layout.
/// Used by `guard_clause_to_rewrite` to avoid producing a multi-line postfix
/// `expr if cond` — which parses fine but breaks `detect_postfix_if_kind`'s
/// single-line lookahead on subsequent fmt passes.
fn expr_likely_breaks(e: &Expr) -> bool {
    match &e.kind {
        // Matches `print_expr`'s Hash branch: a Hash with 2+ pairs may be
        // emitted multi-line — either unconditionally (> 2 pairs) or once
        // the formatter is past column 30 (the project's tightened
        // threshold). Since we can't predict the post-rewrite column here,
        // be conservative and flag any 2+ pair hash as likely to break.
        ExprKind::Hash(pairs) => {
            if pairs.len() > 1 {
                return true;
            }
            pairs.iter().any(|(k, v)| expr_likely_breaks(k) || expr_likely_breaks(v))
        }
        // The Array branch forces multi-line when > 3 elements, or with 3+
        // elements once we're past column 20. Like the Hash case, we can't
        // predict the post-rewrite column from here, so conservatively
        // flag any 3+ element array as likely to break.
        ExprKind::Array(elements) => {
            elements.len() > 2 || elements.iter().any(expr_likely_breaks)
        }
        ExprKind::Block(_) | ExprKind::Lambda { .. } => true,
        ExprKind::Binary { left, right, .. } | ExprKind::Pipeline { left, right } => {
            expr_likely_breaks(left) || expr_likely_breaks(right)
        }
        ExprKind::LogicalAnd { left, right } | ExprKind::LogicalOr { left, right } => {
            expr_likely_breaks(left) || expr_likely_breaks(right)
        }
        ExprKind::Assign { target, value } | ExprKind::CompoundAssign { target, value, .. } => {
            expr_likely_breaks(target) || expr_likely_breaks(value)
        }
        ExprKind::Unary { operand, .. }
        | ExprKind::Grouping(operand)
        | ExprKind::Spread(operand)
        | ExprKind::Throw(operand)
        | ExprKind::Member { object: operand, .. }
        | ExprKind::SafeMember { object: operand, .. }
        | ExprKind::QualifiedName { qualifier: operand, .. } => expr_likely_breaks(operand),
        ExprKind::Index { object, index } => {
            expr_likely_breaks(object) || expr_likely_breaks(index)
        }
        ExprKind::Call { callee, arguments } | ExprKind::New { class_expr: callee, arguments } => {
            expr_likely_breaks(callee)
                || arguments.iter().any(|a| match a {
                    Argument::Positional(x) => expr_likely_breaks(x),
                    Argument::Named(na) => expr_likely_breaks(&na.value),
                    Argument::Block(x) => expr_likely_breaks(x),
                })
        }
        ExprKind::Rescue { expr, fallback } => {
            expr_likely_breaks(expr) || expr_likely_breaks(fallback)
        }
        _ => false,
    }
}

/// Same predicate, applied to the expression(s) carried by a guard-clause
/// inner statement (`return`, `throw`, or a bare expression).
fn expr_in_stmt_likely_breaks(s: &Stmt) -> bool {
    match &s.kind {
        StmtKind::Return(Some(e)) | StmtKind::Throw(e) | StmtKind::Expression(e) => {
            expr_likely_breaks(e)
        }
        StmtKind::Return(None) => false,
        _ => true,
    }
}

/// True when the body's first statement, once printed, starts with `(`.
/// Used to decide whether `fn name` may safely drop its empty parens —
/// when the body opens with a parenthesized expression, the parser would
/// otherwise consume it as the parameter list.
fn body_starts_with_paren(body: &[Stmt]) -> bool {
    fn expr_starts_with_paren(e: &Expr) -> bool {
        match &e.kind {
            ExprKind::Grouping(_) => true,
            ExprKind::Binary { left, .. } | ExprKind::Pipeline { left, .. } => {
                expr_starts_with_paren(left)
            }
            ExprKind::Call { callee: inner, .. }
            | ExprKind::Member { object: inner, .. }
            | ExprKind::SafeMember { object: inner, .. }
            | ExprKind::Index { object: inner, .. }
            | ExprKind::QualifiedName { qualifier: inner, .. } => expr_starts_with_paren(inner),
            ExprKind::Assign { target, .. } | ExprKind::CompoundAssign { target, .. } => {
                expr_starts_with_paren(target)
            }
            ExprKind::Rescue { expr, .. } => expr_starts_with_paren(expr),
            _ => false,
        }
    }
    match body.first().map(|s| &s.kind) {
        Some(StmtKind::Expression(e)) => expr_starts_with_paren(e),
        _ => false,
    }
}
