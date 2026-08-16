//! Statement printer.

use crate::ast::expr::{Argument, Expr, ExprKind, InterpolatedPart};

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
    CatchClause, ClassDecl, ConstructorDecl, EnumDecl, FieldDecl, FunctionDecl, ImportDecl,
    ImportSpecifier, InterfaceDecl, MethodDecl, Parameter, Stmt, StmtKind, Visibility,
};

use super::printer::Printer;

impl Printer<'_> {
    pub(super) fn print_stmt(&mut self, stmt: &Stmt) {
        self.flush_comments_before(stmt.span.line_usize());
        // A trailing `# soli-lint-disable-line` would be detached from its
        // target line whenever the formatter alters layout (e.g. breaking a
        // long expression across lines), silently disabling the suppression.
        // Rewrite it as `# soli-lint-disable-next-line` placed just above the
        // statement — same effect, robust against line splits.
        self.rewrite_trailing_lint_disable(stmt.span.line_usize());
        // Reset the postfix-rewrite flag; the if-branch below sets it back
        // to true when we actually rewrite a block-if to postfix form.
        self.last_stmt_rewrote_to_postfix = false;
        // Record this statement's opener line now that its own leading comments
        // are flushed. For a block-bodied statement (`if`/`while`/`for`/`def`/
        // `class`/…) whose body starts with a comment, this keeps
        // `emit_comment`'s blank-line-preservation check measuring the gap from
        // the opener line rather than from the statement *before* the block —
        // otherwise a comment adjacent to the opener spuriously gains a blank
        // line above it. The end line is still recorded after the body prints.
        self.record_emitted_line(stmt.span.line_usize());
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
                if let Some(kw) = detect_postfix_if_kind(self.source, stmt.span.start_usize()) {
                    self.print_postfix_if(condition, then_branch, kw, true);
                } else if let Some(inner) = self.guard_clause_to_rewrite(
                    stmt,
                    condition,
                    then_branch,
                    else_branch.is_some(),
                ) {
                    // Block form `if cond ... end` with a single guard-style
                    // body collapses to idiomatic postfix `expr if cond`.
                    self.print_postfix_if(condition, inner, PostfixIfKind::If, false);
                    self.last_stmt_rewrote_to_postfix = true;
                } else {
                    self.print_if(condition, then_branch, else_branch.as_deref());
                    self.maybe_blank_line_after_guard(then_branch, else_branch.as_deref());
                }
            }
            StmtKind::Unless {
                condition,
                then_branch,
                else_branch,
            } => {
                if let Some(inner) = self.guard_clause_to_rewrite(
                    stmt,
                    condition,
                    then_branch,
                    else_branch.is_some(),
                ) {
                    self.print_postfix_if(condition, inner, PostfixIfKind::Unless, false);
                    self.last_stmt_rewrote_to_postfix = true;
                } else {
                    self.print_unless(condition, then_branch, else_branch.as_deref());
                    self.maybe_blank_line_after_guard(then_branch, else_branch.as_deref());
                }
            }
            StmtKind::While { condition, body } => {
                self.write("while ");
                self.without_do_blocks(|p| p.print_expr(condition));
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
                self.without_do_blocks(|p| p.print_expr(iterable));
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
            StmtKind::Break => {
                self.write("break");
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
            StmtKind::Enum(decl) => self.print_enum_decl(decl),
            StmtKind::Interface(decl) => self.print_interface_decl(decl),
            StmtKind::Import(decl) => self.print_import_decl(decl),
            StmtKind::Export(inner) => {
                self.write("export ");
                self.print_stmt(inner);
            }
        }
        self.flush_trailing_comments_on(stmt.span.line_usize());
        let end_line = super::printer::source_end_line(self.source, stmt.span);
        self.record_emitted_line(end_line);
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
        let start_line = if_stmt.span.line_usize();
        let end_line = super::printer::source_end_line(self.source, if_stmt.span);
        if self.has_comments_in_lines(start_line, end_line + 1) {
            return None;
        }
        // The printer's break heuristics may multi-line a hash/array value
        // (e.g. a 2-pair hash whose value is a `+` concat is forced
        // multi-line). Don't rewrite then — the postfix would degrade to
        // multi-line output. This check is layout-independent so it gives
        // the same answer regardless of whether the source currently has
        // the inner expression collapsed or expanded; using raw source-line
        // counts here would break idempotency (the first pass collapses
        // the inner via hash-inlining, then the second pass would flip
        // to postfix).
        if expr_in_stmt_likely_breaks(inner) {
            return None;
        }
        if expr_likely_breaks(condition) {
            return None;
        }
        // A value printed verbatim across lines strands the postfix keyword
        // after its closing delimiter, producing source that does not parse.
        if stmt_prints_verbatim_newline(inner) || expr_prints_verbatim_newline(condition) {
            return None;
        }
        // Layout-independent width: the inner stmt's span may contain
        // newlines+continuation indent in pass-1 source but be collapsed
        // to one line in pass-2 source. Raw byte width disagrees between
        // passes and breaks idempotency.
        //
        // Cover child spans too: a desugared `unless a || b` is `Not(Or)`,
        // and after the first pass the `!` node's own span is just the
        // bang. Using only `condition.span` then underestimates the line
        // and the second pass rewrites a block the first pass left
        // alone (`google_oauth_service.sl`).
        let cond_w = expr_print_width(condition);
        let inner_w = match &inner.kind {
            StmtKind::Throw(e) => 6 + expr_print_width(e),
            StmtKind::Return(Some(e)) => 7 + expr_print_width(e),
            StmtKind::Return(None) => 6,
            StmtKind::Expression(e) => expr_print_width(e),
            _ => super::expressions::span_inline_width(self.source, stmt_layout_span(inner)),
        };
        let total = self.pending_column() + inner_w + 4 /* " if " */ + cond_w;
        if total > MAX_LINE_LENGTH {
            return None;
        }
        Some(inner)
    }

    /// Emit `expr if cond` / `expr unless cond`. The condition stored on the
    /// AST for `unless` form is `Unary{Not, inner}` (the parser desugars
    /// `expr unless cond` to `if !cond`); we strip that wrapper so the
    /// printed condition is the original `cond`.
    fn print_postfix_if(
        &mut self,
        condition: &Expr,
        then_branch: &Stmt,
        kind: PostfixIfKind,
        strip_desugared_not: bool,
    ) {
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
            StmtKind::Break => self.write("break"),
            // Block-bodied postfix should not exist (parser wraps a single
            // statement in Expression/Return/Throw). Fall back to recursing
            // through `print_stmt` to be safe.
            _ => self.print_stmt(then_branch),
        }
        match kind {
            PostfixIfKind::If => {
                self.write(" if ");
                self.without_do_blocks(|p| p.print_expr(condition));
            }
            PostfixIfKind::Unless => {
                self.write(" unless ");
                // Postfix `expr unless cond` desugars to `If { !cond }`.
                // Block `unless` stores the raw condition — do not strip.
                let cond = if strip_desugared_not {
                    if let ExprKind::Unary {
                        operator: crate::ast::expr::UnaryOp::Not,
                        operand,
                    } = &condition.kind
                    {
                        operand.as_ref()
                    } else {
                        condition
                    }
                } else {
                    condition
                };
                self.without_do_blocks(|p| p.print_expr(cond));
            }
        }
        self.newline();
    }

    fn print_unless(&mut self, condition: &Expr, then_branch: &Stmt, else_branch: Option<&Stmt>) {
        self.write("unless ");
        self.without_do_blocks(|p| p.print_expr(condition));
        self.newline();
        self.print_block_or_stmt(then_branch);
        match else_branch {
            None => {
                self.write("end");
                self.newline();
            }
            Some(else_stmt) => {
                self.write("else");
                self.newline();
                self.print_block_or_stmt(else_stmt);
                self.write("end");
                self.newline();
            }
        }
    }

    fn print_if(&mut self, condition: &Expr, then_branch: &Stmt, else_branch: Option<&Stmt>) {
        // No paren guard here any more. This used to wrap the condition when the
        // body's first statement began with `-`, `+`, `[`, `(` or `.`, because
        // the parser would otherwise continue the condition onto that line and
        // `if x < 0\n  -x\n end` reparsed as `if (x < 0 - x) end`. The parser now
        // ends a condition with its line, so the wrapping is unnecessary — and it
        // was the reason `fmt` was not idempotent: the added parens parsed back as
        // a grouping node, which printed as parens, which got wrapped again.
        self.write("if ");
        self.without_do_blocks(|p| p.print_expr(condition));
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
            // Pass the block's own closing line so a comment sitting after the
            // last statement — or a comment that is the *whole* body, as in
            // `rescue` / `# already exists` / `end` — is emitted inside the
            // block instead of escaping past the `end` on the next flush.
            // Record the block's opening line first. The keyword that opens it
            // (`catch e`, `else`) is written by the caller and never recorded,
            // so without this a body-leading comment measures its gap from the
            // statement *above* the keyword and looks like a new paragraph —
            // gaining a blank line on the second pass, i.e. not idempotent.
            //
            // Clamp to the first statement's line: a block's opening can never
            // come *after* its first statement, but the parser does not give
            // every block a span starting at its opening — an `if` body's span
            // line is its *last* statement, where `for` / `while` use the first.
            // Recording the raw value there set `last_emitted_line` past the
            // body, so `comment_fills_gap` saw a phantom comment above every
            // statement and swallowed the author's blank lines inside `if`
            // bodies (`a()` / blank / `b()` came back with the blank deleted).
            let opening_line = stmts.first().map_or(stmt.span.line_usize(), |first| {
                stmt.span.line_usize().min(first.span.line_usize())
            });
            self.record_emitted_line(opening_line);
            let close_line = super::printer::source_end_line(self.source, stmt.span);
            self.print_block_body_through(stmts, Some(close_line));
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
        // `def` for every *named* function, free-standing or not. `fn` is the
        // lambda keyword; using it for declarations fought the convention the
        // language docs set out (controllers are `def index(req)`) and rewrote
        // hundreds of `def`s in real apps. Interface members are the one
        // exception and the parser insists on `fn` there — see
        // `print_interface_decl`.
        self.write("def ");
        self.write(&decl.name);
        // Free-standing declarations may omit empty parens (Soli convention:
        // "Optional parentheses for no-param functions"). Methods keep
        // their parens to match the project's `def name() ... end` style.
        // Also keep empty parens when the body's first statement starts
        // with `(` — otherwise the parser would consume that `(` as the
        // parameter list (e.g. `def f` followed by `(x ?? "") == "y"`).
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
        // Drop empty parens (`def run()` -> `def run`), matching Soli's
        // optional-parens convention for no-arg definitions. Keep them when
        // the body's first statement starts with `(`, or the parser would
        // consume that `(` as the parameter list.
        if !decl.params.is_empty() || body_starts_with_paren(&decl.body) {
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
        // Mirror `write_param`'s output width — name, `: <Type>` annotation,
        // and ` = <default>` — so long typed signatures actually break instead
        // of overflowing the line-length limit the linter enforces.
        let est: usize = params
            .iter()
            .map(|p| {
                let mut w = p.name.len() + if p.is_block_param { 1 } else { 0 };
                let ty = format_type(&p.type_annotation);
                if !ty.is_empty() && ty != "Any" {
                    w += 2 + ty.len();
                }
                if let Some(def) = &p.default_value {
                    w += 3 + super::expressions::ast_inline_width(self.source, def);
                }
                w + 2 // ", " separator
            })
            .sum::<usize>()
            + 2;
        if params.len() > 1 && self.current_column() + est > MAX_LINE_LENGTH {
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
                p.flush_comments_before(field.span.line_usize());
                p.print_field_decl(field);
            }
            if !decl.fields.is_empty() && (!decl.methods.is_empty() || decl.constructor.is_some()) {
                p.blank_line();
            }
            // Constructor
            if let Some(ctor) = &decl.constructor {
                p.flush_comments_before(ctor.span.line_usize());
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
                p.flush_comments_before(m.span.line_usize());
                p.print_method_decl(m);
            }
            // Nested classes
            if !decl.nested_classes.is_empty() {
                p.blank_line();
                for (i, n) in decl.nested_classes.iter().enumerate() {
                    if i > 0 {
                        p.blank_line();
                    }
                    p.flush_comments_before(n.span.line_usize());
                    p.print_class_decl(n);
                }
            }
        });
        self.write("end");
        self.newline();
    }

    fn print_enum_decl(&mut self, decl: &EnumDecl) {
        self.write("enum ");
        self.write(&decl.name);
        self.newline();
        self.with_indent(|p| {
            let last = decl.variants.len().saturating_sub(1);
            for (i, variant) in decl.variants.iter().enumerate() {
                p.flush_comments_before(variant.span.line_usize());
                p.write(&variant.name);
                if !variant.payload.is_empty() {
                    p.write("(");
                    for (j, field) in variant.payload.iter().enumerate() {
                        if j > 0 {
                            p.write(", ");
                        }
                        p.write(&field.name);
                        if let Some(ty) = &field.type_annotation {
                            p.write(": ");
                            p.write(&ty.to_string());
                        }
                    }
                    p.write(")");
                }
                if i != last {
                    p.write(",");
                }
                p.newline();
            }
            // Methods (the "rich" scope).
            if !decl.methods.is_empty() {
                p.blank_line();
                for (i, m) in decl.methods.iter().enumerate() {
                    if i > 0 {
                        p.blank_line();
                    }
                    p.flush_comments_before(m.span.line_usize());
                    p.print_method_decl(m);
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

/// Printed width of `e` from the AST, not from source spans.
///
/// Spans on a desugared `unless` / a `!` token are often just the operator,
/// so `span_inline_width` underestimates and a later fmt pass rewrites a
/// block that the first pass left alone.
fn expr_print_width(e: &Expr) -> usize {
    use crate::ast::expr::UnaryOp;
    match &e.kind {
        ExprKind::StringLiteral(s) => s.len() + 2,
        ExprKind::IntLiteral(n) => n.to_string().len(),
        ExprKind::FloatLiteral(_) | ExprKind::DecimalLiteral(_) => 8,
        ExprKind::BoolLiteral(true) => 4,
        ExprKind::BoolLiteral(false) => 5,
        ExprKind::Null => 4,
        ExprKind::Variable(name) | ExprKind::Symbol(name) => name.len(),
        ExprKind::Unary {
            operator: UnaryOp::Not,
            operand,
        } => match &operand.kind {
            ExprKind::LogicalOr { .. } | ExprKind::LogicalAnd { .. } | ExprKind::Binary { .. } => {
                3 + expr_print_width(operand)
            }
            ExprKind::Grouping(_) => 1 + expr_print_width(operand),
            _ => 1 + expr_print_width(operand),
        },
        ExprKind::Unary { operand, .. } => 1 + expr_print_width(operand),
        ExprKind::Grouping(inner) => 2 + expr_print_width(inner),
        ExprKind::LogicalOr { left, right } | ExprKind::LogicalAnd { left, right } => {
            expr_print_width(left) + 4 + expr_print_width(right)
        }
        ExprKind::Binary {
            left,
            operator,
            right,
        } => expr_print_width(left) + 1 + operator.to_string().len() + 1 + expr_print_width(right),
        ExprKind::Index { object, index } => expr_print_width(object) + 2 + expr_print_width(index),
        ExprKind::Member { object, name, .. } | ExprKind::SafeMember { object, name, .. } => {
            expr_print_width(object) + 1 + name.len()
        }
        ExprKind::Hash(pairs) => {
            let inner: usize = pairs
                .iter()
                .map(|(k, v)| expr_print_width(k) + 2 + expr_print_width(v))
                .sum();
            let seps = pairs.len().saturating_sub(1) * 2;
            2 + inner + seps
        }
        ExprKind::Call { callee, arguments } => {
            let args: usize = arguments
                .iter()
                .map(|a| match a {
                    Argument::Positional(x) | Argument::Block(x) => expr_print_width(x),
                    Argument::Named(na) => na.name.len() + 2 + expr_print_width(&na.value),
                })
                .sum();
            let seps = arguments.len().saturating_sub(1) * 2;
            expr_print_width(callee) + 2 + args + seps
        }
        _ => super::expressions::span_inline_width("", e.span).max(8),
    }
}

/// Source span covering `e` and every child. The node's own span can be a
/// single operator token (`!` on a desugared `unless`); width checks must
/// count the printed operand too.
fn expr_layout_span(e: &Expr) -> crate::span::Span {
    let mut span = e.span;
    match &e.kind {
        ExprKind::Unary { operand, .. }
        | ExprKind::Grouping(operand)
        | ExprKind::Spread(operand)
        | ExprKind::Throw(operand)
        | ExprKind::Member {
            object: operand, ..
        }
        | ExprKind::SafeMember {
            object: operand, ..
        }
        | ExprKind::QualifiedName {
            qualifier: operand, ..
        } => {
            span = span.merge(&expr_layout_span(operand));
        }
        ExprKind::Binary { left, right, .. }
        | ExprKind::Pipeline { left, right }
        | ExprKind::LogicalAnd { left, right }
        | ExprKind::LogicalOr { left, right }
        | ExprKind::Assign {
            target: left,
            value: right,
        }
        | ExprKind::CompoundAssign {
            target: left,
            value: right,
            ..
        }
        | ExprKind::Rescue {
            expr: left,
            fallback: right,
        }
        | ExprKind::Index {
            object: left,
            index: right,
        } => {
            span = span
                .merge(&expr_layout_span(left))
                .merge(&expr_layout_span(right));
        }
        ExprKind::Call { callee, arguments }
        | ExprKind::New {
            class_expr: callee,
            arguments,
        } => {
            span = span.merge(&expr_layout_span(callee));
            for arg in arguments {
                match arg {
                    Argument::Positional(x) | Argument::Block(x) => {
                        span = span.merge(&expr_layout_span(x));
                    }
                    Argument::Named(na) => {
                        span = span.merge(&expr_layout_span(&na.value));
                    }
                }
            }
        }
        _ => {}
    }
    span
}

fn stmt_layout_span(s: &Stmt) -> crate::span::Span {
    match &s.kind {
        StmtKind::Return(Some(e)) | StmtKind::Throw(e) | StmtKind::Expression(e) => {
            s.span.merge(&expr_layout_span(e))
        }
        _ => s.span,
    }
}

/// Mirror the printer's break heuristics for collection literals. Returns
/// true when `e` (or some sub-expression of `e`) is one the printer will
/// emit across multiple lines, regardless of the source's original layout.
/// Used by `guard_clause_to_rewrite` to avoid producing a multi-line postfix
/// `expr if cond` — which parses fine but breaks `detect_postfix_if_kind`'s
/// single-line lookahead on subsequent fmt passes.
pub(super) fn expr_likely_breaks(e: &Expr) -> bool {
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
            // A one-pair `{k: call(a, b, c)}` is printed across lines once
            // the call is wide. First pass used to keep the `if` block;
            // second pass then collapsed it to postfix (`oauth_tokens`).
            pairs.iter().any(|(k, v)| {
                expr_likely_breaks(k)
                    || expr_likely_breaks(v)
                    || matches!(
                        &v.kind,
                        ExprKind::Call { arguments, .. } if arguments.len() > 1
                    )
            })
        }
        // The Array branch forces multi-line when > 3 elements, or with 3+
        // elements once we're past column 20. Like the Hash case, we can't
        // predict the post-rewrite column from here, so conservatively
        // flag any 3+ element array as likely to break.
        ExprKind::Array(elements) => elements.len() > 2 || elements.iter().any(expr_likely_breaks),
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
        | ExprKind::Member {
            object: operand, ..
        }
        | ExprKind::SafeMember {
            object: operand, ..
        }
        | ExprKind::QualifiedName {
            qualifier: operand, ..
        } => expr_likely_breaks(operand),
        ExprKind::Index { object, index } => {
            expr_likely_breaks(object) || expr_likely_breaks(index)
        }
        ExprKind::Call { callee, arguments }
        | ExprKind::New {
            class_expr: callee,
            arguments,
        } => {
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

/// True when printing `e` necessarily emits an embedded newline, whatever
/// layout the formatter picks, because it carries a construct copied verbatim
/// out of the source: an `@sdbql{ … }` block (`print_expr`'s `SdqlBlock`
/// branch) or a raw string literal `[[ … ]]` / `r"…"` (`raw_string_source`).
///
/// A guard-clause rewrite must refuse these. Postfix `expr if cond` puts the
/// keyword *after* the value, so a multi-line value strands the `if` on the
/// line following the closing delimiter — where it no longer parses:
///
/// ```text
/// rows = @sdbql{
///     FOR d IN docs RETURN d
///   } if keys.length() > 0        # Parser error
/// ```
///
/// Unlike a raw source-line count this is layout-independent, so it answers
/// the same on every pass and keeps `fmt` idempotent: verbatim content is by
/// definition unchanged by formatting.
///
/// `StringLiteral` deliberately over-approximates — whether a literal was
/// written raw needs the source, and a non-raw value holding a real newline
/// (`"a\nb"` escapes to one line) is rare. The only cost of a false positive
/// is keeping the block form, which is always valid.
fn expr_prints_verbatim_newline(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::SdqlBlock { query, .. } => query.contains('\n'),
        ExprKind::StringLiteral(s) => s.contains('\n'),
        ExprKind::Hash(pairs) => pairs
            .iter()
            .any(|(k, v)| expr_prints_verbatim_newline(k) || expr_prints_verbatim_newline(v)),
        ExprKind::Array(elements) => elements.iter().any(expr_prints_verbatim_newline),
        ExprKind::Binary { left, right, .. }
        | ExprKind::Pipeline { left, right }
        | ExprKind::LogicalAnd { left, right }
        | ExprKind::LogicalOr { left, right } => {
            expr_prints_verbatim_newline(left) || expr_prints_verbatim_newline(right)
        }
        ExprKind::Assign { target, value } | ExprKind::CompoundAssign { target, value, .. } => {
            expr_prints_verbatim_newline(target) || expr_prints_verbatim_newline(value)
        }
        ExprKind::Unary { operand, .. }
        | ExprKind::Grouping(operand)
        | ExprKind::Spread(operand)
        | ExprKind::Throw(operand)
        | ExprKind::Member {
            object: operand, ..
        }
        | ExprKind::SafeMember {
            object: operand, ..
        }
        | ExprKind::QualifiedName {
            qualifier: operand, ..
        } => expr_prints_verbatim_newline(operand),
        ExprKind::Index { object, index } => {
            expr_prints_verbatim_newline(object) || expr_prints_verbatim_newline(index)
        }
        ExprKind::Call { callee, arguments }
        | ExprKind::New {
            class_expr: callee,
            arguments,
        } => {
            expr_prints_verbatim_newline(callee)
                || arguments.iter().any(|a| match a {
                    Argument::Positional(x) | Argument::Block(x) => expr_prints_verbatim_newline(x),
                    Argument::Named(na) => expr_prints_verbatim_newline(&na.value),
                })
        }
        ExprKind::Rescue { expr, fallback } => {
            expr_prints_verbatim_newline(expr) || expr_prints_verbatim_newline(fallback)
        }
        ExprKind::InterpolatedString(parts) => parts.iter().any(|p| match p {
            InterpolatedPart::Literal(s) => s.contains('\n'),
            InterpolatedPart::Expression(x) => expr_prints_verbatim_newline(x),
        }),
        _ => false,
    }
}

/// Same predicate, applied to the expression(s) carried by a guard-clause
/// inner statement.
fn stmt_prints_verbatim_newline(s: &Stmt) -> bool {
    match &s.kind {
        StmtKind::Return(Some(e)) | StmtKind::Throw(e) | StmtKind::Expression(e) => {
            expr_prints_verbatim_newline(e)
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
            | ExprKind::QualifiedName {
                qualifier: inner, ..
            } => expr_starts_with_paren(inner),
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
