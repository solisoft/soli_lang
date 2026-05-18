//! Low-level output primitives for the formatter.
//!
//! The Printer keeps a String buffer plus an indent counter (in *levels*, not
//! spaces — we multiply by 2 on emit). It tracks whether we're at the start of
//! a line so callers can emit `write("foo")` without worrying about the
//! leading indent — `write` inserts it automatically the first time per line.
//!
//! Column tracking enables line-length-aware formatting: the printer tracks
//! the current column position so expression printers can break long lines.

use crate::ast::stmt::{Program, Stmt};

use super::comments::{Comment, CommentKind};

const INDENT_WIDTH: usize = 2;

/// Maximum line length the formatter will produce. Lines exceeding this
/// trigger `style/line-length` in `soli lint`, so the formatter must stay
/// under it. Based on the lint rule default of 120.
pub const MAX_LINE_LENGTH: usize = 120;

pub struct Printer<'a> {
    pub(super) source: &'a str,
    out: String,
    indent: usize,
    at_line_start: bool,
    /// Current column position (0-indexed). Reset to `indent * INDENT_WIDTH`
    /// after `newline()`, updated on every `write()`.
    column: usize,
    /// Next comment to emit (index into `comments`).
    comment_cursor: usize,
    comments: Vec<Comment>,
    /// Source line of the last node we emitted — used to decide whether to
    /// flush a blank line between top-level statements.
    pub(super) last_emitted_line: usize,
}

impl<'a> Printer<'a> {
    pub fn new(source: &'a str, comments: Vec<Comment>) -> Self {
        Self {
            source,
            out: String::with_capacity(source.len()),
            indent: 0,
            at_line_start: true,
            column: 0,
            comment_cursor: 0,
            comments,
            last_emitted_line: 0,
        }
    }

    pub fn current_column(&self) -> usize {
        self.column
    }

    pub fn finish(mut self) -> String {
        // Emit any comments that came after the last statement.
        let total_lines = self.source.lines().count().max(1);
        self.flush_comments_before(total_lines + 1);
        // Ensure trailing newline.
        if !self.out.ends_with('\n') {
            self.out.push('\n');
        }
        self.out
    }

    // ---------- low-level emit ----------

    pub(super) fn write(&mut self, s: &str) {
        if self.at_line_start {
            let indent_spaces = self.indent * INDENT_WIDTH;
            for _ in 0..indent_spaces {
                self.out.push(' ');
            }
            self.at_line_start = false;
            self.column = indent_spaces;
        }
        self.out.push_str(s);
        // Update column: find the last newline in s, count from there
        if let Some(pos) = s.rfind('\n') {
            self.column = s.len() - pos - 1;
        } else {
            self.column += s.len();
        }
    }

    /// Estimate the display width of a string as it would appear on the
    /// current line. Useful for checking whether appending `s` would exceed
    /// MAX_LINE_LENGTH.
    pub(super) fn would_exceed_max_width(&self, s: &str) -> bool {
        let new_col = if let Some(pos) = s.rfind('\n') {
            s.len() - pos - 1
        } else {
            self.column + s.len()
        };
        new_col > MAX_LINE_LENGTH
    }

    pub(super) fn newline(&mut self) {
        // Strip any trailing spaces from the line we just finished.
        while self.out.ends_with(' ') {
            self.out.pop();
        }
        self.out.push('\n');
        self.at_line_start = true;
        self.column = self.indent * INDENT_WIDTH;
    }

    pub(super) fn blank_line(&mut self) {
        if !self.at_line_start {
            self.newline();
        }
        // Avoid emitting more than one blank in a row.
        if self.out.ends_with("\n\n") || self.out.is_empty() {
            return;
        }
        self.out.push('\n');
    }

    pub(super) fn indent(&mut self) {
        self.indent += 1;
    }

    pub(super) fn dedent(&mut self) {
        if self.indent > 0 {
            self.indent -= 1;
        }
    }

    pub(super) fn with_indent(&mut self, f: impl FnOnce(&mut Self)) {
        self.indent();
        f(self);
        self.dedent();
    }

    pub(super) fn is_at_line_start(&self) -> bool {
        self.at_line_start
    }

    /// If the output buffer ends with a single `\n`, remove it and return true.
    /// Used to "back up" so a caller can append something (like a `;`) just
    /// before the newline that `print_stmt` already emitted.
    pub(super) fn pop_trailing_newline(&mut self) -> bool {
        if self.out.ends_with('\n') {
            self.out.pop();
            self.at_line_start = false;
            true
        } else {
            false
        }
    }

    /// Copy raw source bytes for a span. Fallback for AST nodes the printer
    /// doesn't model yet — semantics preserved at the cost of formatting.
    pub(super) fn write_source_span(&mut self, start: usize, end: usize) {
        let slice = &self.source[start.min(self.source.len())..end.min(self.source.len())];
        self.write(slice);
    }

    // ---------- comment interleaving ----------

    /// Emit any comments whose source line is strictly before `line`. Inserts
    /// blank lines between the previous emission and the comment block to
    /// preserve the user's intentional spacing.
    pub(super) fn flush_comments_before(&mut self, line: usize) {
        while self.comment_cursor < self.comments.len()
            && self.comments[self.comment_cursor].line < line
        {
            let c = self.comments[self.comment_cursor].clone();
            self.emit_comment(&c);
            self.comment_cursor += 1;
        }
    }

    /// Emit comments on the same source line as `line` as trailing comments
    /// on the current output line (joined with a single space).
    pub(super) fn flush_trailing_comments_on(&mut self, line: usize) {
        while self.comment_cursor < self.comments.len()
            && self.comments[self.comment_cursor].line == line
        {
            let c = self.comments[self.comment_cursor].clone();
            // Only treat as trailing if we haven't just started a line.
            if !self.at_line_start {
                self.write("  ");
                self.write(&c.text);
                self.comment_cursor += 1;
            } else {
                self.emit_comment(&c);
                self.comment_cursor += 1;
            }
        }
    }

    fn emit_comment(&mut self, c: &Comment) {
        if !self.at_line_start {
            self.newline();
        }
        // Preserve user's intentional blank line *before* the comment.
        if c.line > self.last_emitted_line + 1 && self.last_emitted_line != 0 {
            self.blank_line();
        }
        match c.kind {
            CommentKind::Line => {
                // Normalize `//` line comments to `#` per project style.
                let normalized = if let Some(rest) = c.text.strip_prefix("//") {
                    format!("#{}", rest)
                } else {
                    c.text.clone()
                };
                self.write(&normalized);
                self.newline();
            }
            CommentKind::Block => {
                // Block comments preserve formatting verbatim (they may span
                // multiple lines with their own internal indent).
                self.write(&c.text);
                self.newline();
            }
        }
        self.last_emitted_line = c.line;
    }

    // ---------- top-level dispatch ----------

    pub fn print_program(&mut self, program: &Program) {
        // Track the previous statement's SOURCE end-line (not the emitted
        // line) so multi-line expansion of compact one-liners doesn't
        // spuriously insert blank lines on second-pass formatting.
        let mut prev_source_end: usize = 0;
        for (idx, stmt) in program.statements.iter().enumerate() {
            self.flush_comments_before(stmt.span.line);
            if idx > 0 && stmt.span.line > prev_source_end + 1 {
                self.blank_line();
            }
            self.print_stmt(stmt);
            // Disambiguate against a following `[`/`(`/`.`-led line.
            if let Some(next) = program.statements.get(idx + 1) {
                if needs_disambiguating_semicolon(self.source, stmt, next) {
                    let had_newline = self.pop_trailing_newline();
                    self.write(";");
                    if had_newline {
                        self.newline();
                    }
                }
            }
            if !self.at_line_start {
                self.newline();
            }
            prev_source_end = source_end_line(self.source, stmt.span);
        }
    }

    pub(super) fn record_emitted_line(&mut self, line: usize) {
        if line > self.last_emitted_line {
            self.last_emitted_line = line;
        }
    }

    /// Print a vec of statements as an indented block body (used by fn/class/
    /// if/while/for bodies). Caller emits the surrounding keyword/`end`.
    pub(super) fn print_block_body(&mut self, stmts: &[Stmt]) {
        self.print_block_body_through(stmts, None)
    }

    /// Variant of [`print_block_body`] that also flushes any pending comments
    /// whose source line falls *inside* the enclosing block but *after* the
    /// last statement, before returning. Pass the source line of the closing
    /// delimiter (`}` or `end`) so trailing in-body comments don't escape
    /// the block and re-attach to outer code on the next fmt pass.
    pub(super) fn print_block_body_through(&mut self, stmts: &[Stmt], close_line: Option<usize>) {
        self.with_indent(|p| {
            // Same source-end-line tracking as in `print_program` — see
            // there for why this matters for idempotency.
            let mut prev_source_end: usize = 0;
            for (idx, stmt) in stmts.iter().enumerate() {
                p.flush_comments_before(stmt.span.line);
                if idx > 0 && stmt.span.line > prev_source_end + 1 {
                    p.blank_line();
                }
                p.print_stmt(stmt);
                // Disambiguate against a following continuation-token line.
                if let Some(next) = stmts.get(idx + 1) {
                    if needs_disambiguating_semicolon(p.source, stmt, next) {
                        let had_newline = p.pop_trailing_newline();
                        p.write(";");
                        if had_newline {
                            p.newline();
                        }
                    }
                }
                if !p.at_line_start {
                    p.newline();
                }
                prev_source_end = source_end_line(p.source, stmt.span);
            }
            if let Some(line) = close_line {
                p.flush_comments_before(line);
            }
        });
    }
}

/// Return the source line number that contains the last byte of `span`.
pub(super) fn source_end_line(source: &str, span: crate::span::Span) -> usize {
    let end = span.end.min(source.len());
    let start = span.start.min(end);
    span.line + source[start..end].matches('\n').count()
}

/// Soli's parser is greedy across newlines for `[`, `(`, `.` — a line that
/// begins with one of those continues the previous expression. After an
/// expression-ending statement we must emit a `;` if the next source line
/// starts that way; otherwise `let x = 0\n[1, 2, 3]` reparses as
/// `let x = 0[1, 2, 3]`.
pub(super) fn needs_disambiguating_semicolon(source: &str, current: &Stmt, next: &Stmt) -> bool {
    if !ends_in_expression(current) {
        return false;
    }
    starts_with_continuation_char(source, next.span.start)
}

fn ends_in_expression(stmt: &Stmt) -> bool {
    use crate::ast::stmt::StmtKind;
    match &stmt.kind {
        StmtKind::Expression(_) => true,
        StmtKind::Let { initializer, .. } => initializer.is_some(),
        StmtKind::Const { .. } => true,
        StmtKind::Return(opt) => opt.is_some(),
        StmtKind::Throw(_) => true,
        // Postfix `if`/`unless` lower to StmtKind::If but the printed form
        // ends with an expression too. Block-form `if/while/for/try/fn/class`
        // end with `end`, safe.
        StmtKind::If {
            else_branch: None,
            then_branch,
            ..
        } => matches!(
            &then_branch.kind,
            StmtKind::Expression(_) | StmtKind::Return(_) | StmtKind::Throw(_)
        ),
        _ => false,
    }
}

fn starts_with_continuation_char(source: &str, start: usize) -> bool {
    let bytes = source.as_bytes();
    let mut i = start.min(bytes.len());
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    matches!(bytes.get(i), Some(b'[') | Some(b'(') | Some(b'.'))
}
