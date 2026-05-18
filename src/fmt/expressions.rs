//! Expression printer.

use crate::ast::expr::{
    Argument, BinaryOp, CompoundOp, Expr, ExprKind, InterpolatedPart, MatchArm, MatchPattern,
    UnaryOp,
};

use super::printer::{Printer, MAX_LINE_LENGTH};

/// Return the source text for a span (used for width estimation).
fn span_source(source: &str, span: crate::span::Span) -> &str {
    let start = span.start.min(source.len());
    let end = span.end.min(source.len());
    &source[start..end]
}

/// Layout-independent width estimate for `span`. Approximates the width of
/// `span`'s source content re-flowed to a single line. Newlines and the
/// indent that follows each one are stripped, then a single ` ` is inserted
/// in their place when the surrounding tokens are ones that would have a
/// space between them inline (matching the printer's actual emission rules:
/// no space after `(`/`[`/`{`, no space before `)`/`]`/`}`, no space around
/// `.`). Without this normalization, break-decision heuristics see different
/// widths for the same AST node depending on whether the source currently
/// has it on one line or wrapped — and the formatter flips between layouts
/// on successive passes.
pub(super) fn span_inline_width(source: &str, span: crate::span::Span) -> usize {
    let s = span_source(source, span);
    let mut count = 0usize;
    let mut at_continuation = false;
    // `pending_space` holds whether we'd emit a space if the next non-ws
    // char isn't a "no-space-before" token. This lets us drop spaces that
    // sit immediately before a closer (`)`, `]`, `}`, `.`) — the printer
    // doesn't emit those either.
    let mut pending_space = false;
    let mut last_non_ws: Option<char> = None;
    for c in s.chars() {
        if c == '\n' {
            at_continuation = true;
        } else if at_continuation && (c == ' ' || c == '\t') {
            // Skip indent run on continuation lines.
        } else if c == ' ' || c == '\t' {
            // Inline whitespace inside a line: collapse runs to at most a
            // single space, and only emit it if surrounded by tokens the
            // printer would also separate (i.e. not after an opener and
            // not before a closer — checked when we see the next non-ws).
            let suppress = matches!(last_non_ws, Some('(' | '[' | '{' | '.'));
            if !suppress {
                pending_space = true;
            }
        } else {
            if at_continuation {
                if let Some(prev) = last_non_ws {
                    let needs_space = !matches!(prev, '(' | '[' | '{' | '.')
                        && !matches!(c, ')' | ']' | '}' | '.');
                    if needs_space {
                        pending_space = true;
                    }
                }
            }
            if pending_space {
                let suppress_before = matches!(c, ')' | ']' | '}' | '.');
                if !suppress_before {
                    count += 1;
                }
                pending_space = false;
            }
            at_continuation = false;
            count += 1;
            last_non_ws = Some(c);
        }
    }
    count
}

/// Check whether a logical operator (&&/||) chain would exceed MAX_LINE_LENGTH.
fn should_logical_break(p: &Printer, left: &Expr, right: &Expr, op: &str) -> bool {
    let _col = p.current_column();
    let left_src = span_inline_width(p.source, left.span).min(120);
    let right_src = span_inline_width(p.source, right.span).min(80);
    // Add 4 safety margin to account for underestimates (e.g., quoted strings
    // re-printed with different escaping than the source span suggests).
    p.current_column() + left_src + op.len() + right_src + 8 > MAX_LINE_LENGTH
}

impl Printer<'_> {
    pub(super) fn print_expr(&mut self, expr: &Expr) {
        match &expr.kind {
            ExprKind::IntLiteral(n) => self.write(&n.to_string()),
            ExprKind::FloatLiteral(n) => {
                let s = format!("{}", n);
                self.write(&s);
                if !s.contains('.') && !s.contains('e') && !s.contains('E') {
                    self.write(".0");
                }
            }
            ExprKind::DecimalLiteral(s) => {
                self.write(s);
                if !s.ends_with('D') && !s.ends_with('d') {
                    self.write("D");
                }
            }
            ExprKind::StringLiteral(s) => {
                self.write("\"");
                for c in s.chars() {
                    match c {
                        '\\' => self.write("\\\\"),
                        '"' => self.write("\\\""),
                        '\n' => self.write("\\n"),
                        '\r' => self.write("\\r"),
                        '\t' => self.write("\\t"),
                        c => {
                            let mut buf = [0u8; 4];
                            self.write(c.encode_utf8(&mut buf));
                        }
                    }
                }
                self.write("\"");
            }
            ExprKind::InterpolatedString(parts) => {
                self.write("\"");
                for part in parts {
                    match part {
                        InterpolatedPart::Literal(s) => {
                            for c in s.chars() {
                                match c {
                                    '\\' => self.write("\\\\"),
                                    '"' => self.write("\\\""),
                                    '\n' => self.write("\\n"),
                                    '\r' => self.write("\\r"),
                                    '\t' => self.write("\\t"),
                                    c => {
                                        let mut buf = [0u8; 4];
                                        self.write(c.encode_utf8(&mut buf));
                                    }
                                }
                            }
                        }
                        InterpolatedPart::Expression(e) => {
                            self.write("#{");
                            self.print_expr(e);
                            self.write("}");
                        }
                    }
                }
                self.write("\"");
            }
            ExprKind::CommandSubstitution(s) => {
                self.write("`");
                self.write(s);
                self.write("`");
            }
            // Not yet specially formatted — copy the original source bytes
            // verbatim so semantics are preserved. The lint/runtime layers
            // round-trip these correctly.
            ExprKind::SdqlBlock { .. } => {
                self.write_source_span(expr.span.start, expr.span.end);
            }
            ExprKind::BoolLiteral(b) => self.write(if *b { "true" } else { "false" }),
            ExprKind::Symbol(name) => {
                self.write(":");
                self.write(name);
            }
            ExprKind::Null => {
                // Preserve the user's choice of `nil` vs `null` (Soli accepts
                // both as synonyms). Falls back to `null` for synthesized
                // Null AST nodes that aren't tied to source text.
                if source_starts_with_nil(self.source, expr.span.start) {
                    self.write("nil");
                } else {
                    self.write("null");
                }
            }
            ExprKind::Variable(name) => self.write(name),
            ExprKind::This => self.write("this"),
            ExprKind::Super => self.write("super"),
            ExprKind::Binary {
                left,
                operator,
                right,
            } => {
                self.print_binary_op(left, *operator, right);
            }
            ExprKind::Unary { operator, operand } => {
                let op = match operator {
                    UnaryOp::Negate => "-",
                    UnaryOp::Not => "!",
                };
                self.write(op);
                self.print_expr(operand);
            }
            ExprKind::Grouping(inner) => {
                self.write("(");
                self.print_expr(inner);
                self.write(")");
            }
            ExprKind::Call { callee, arguments } => {
                self.print_expr(callee);
                // Preserve () for zero-arg calls so the linter can distinguish
                // function calls (.all(), .keys()) from variable reads (.all).
                // But don't ADD parens to bare DSL forms like `soft_delete`
                // that were written without them in the source.
                if arguments.is_empty() && !source_has_parens_after(self.source, callee.span.end) {
                    // bare call, no source parens — skip "()"
                } else {
                    self.print_arg_list(arguments);
                }
            }
            ExprKind::Pipeline { left, right } => {
                self.print_expr(left);
                self.write(" |> ");
                self.print_expr(right);
            }
            ExprKind::Member { object, name } => {
                // `@title` desugars to `Member { This, "title" }` in the
                // AST — same node as `this.title`. Preserve whichever form
                // the user wrote by peeking at the source at the object's
                // start byte.
                if matches!(object.kind, ExprKind::This)
                    && source_starts_with_at(self.source, object.span.start)
                {
                    self.write("@");
                    self.write(name);
                } else {
                    self.print_expr(object);
                    self.write(".");
                    self.write(name);
                }
            }
            ExprKind::SafeMember { object, name } => {
                self.print_expr(object);
                self.write("&.");
                self.write(name);
            }
            ExprKind::QualifiedName { qualifier, name } => {
                self.print_expr(qualifier);
                self.write("::");
                self.write(name);
            }
            ExprKind::Index { object, index } => {
                self.print_expr(object);
                self.write("[");
                self.print_expr(index);
                self.write("]");
            }
            ExprKind::New {
                class_expr,
                arguments,
            } => {
                self.write("new ");
                self.print_expr(class_expr);
                self.print_arg_list(arguments);
            }
            ExprKind::Array(elements) => {
                // Estimate inline width and break long arrays across lines.
                let est: usize = elements.iter().map(|e| {
                    span_inline_width(self.source, e.span).min(40)
                }).sum::<usize>()
                    + 2 // "[]"
                    + (elements.len().saturating_sub(1)) * 2; // ", "
                                                              // Break long arrays across lines if:
                                                              // - More than 3 elements at any column, or
                                                              // - 3+ elements when already past 20 chars
                if (elements.len() > 3 || (elements.len() > 2 && self.current_column() > 20))
                    && (self.current_column() + est > MAX_LINE_LENGTH || self.current_column() > 20)
                {
                    self.write("[");
                    self.newline();
                    self.with_indent(|p| {
                        for (i, e) in elements.iter().enumerate() {
                            if i > 0 {
                                p.write(",");
                                p.newline();
                            }
                            p.print_expr(e);
                        }
                    });
                    self.newline();
                    self.write("]");
                } else {
                    self.write("[");
                    // Avoid `[[` as the first two characters — Soli's lexer
                    // treats `[[X...` (where X is not a digit/minus/[) as a
                    // Lua-style multiline string. Adding a space before the
                    // nested `[` disambiguates.
                    let first_is_array = elements
                        .first()
                        .map(|e| matches!(e.kind, ExprKind::Array(_)))
                        .unwrap_or(false);
                    if first_is_array {
                        self.write(" ");
                    }
                    for (i, e) in elements.iter().enumerate() {
                        if i > 0 {
                            self.write(", ");
                        }
                        self.print_expr(e);
                    }
                    self.write("]");
                }
            }
            ExprKind::Hash(pairs) => {
                if pairs.is_empty() {
                    self.write("{}");
                } else if pairs.len() > 2
                    || (pairs.len() > 1 && pairs.iter().any(|(_, v)| {
                        // Break 2-pair hashes if a value is a long expression
                        // (concat, function call, or column past 30)
                        self.current_column() > 30
                            || matches!(&v.kind,
                                ExprKind::Binary { operator, .. } if binary_op_str(*operator) == "+"
                            )
                    }))
                {
                    // Multi-line for hashes with more than 2 entries, or
                    // 2 entries when already past 30 chars.
                    self.write("{");
                    self.newline();
                    self.with_indent(|p| {
                        for (i, (k, v)) in pairs.iter().enumerate() {
                            if i > 0 {
                                p.write(",");
                                p.newline();
                            }
                            p.print_expr(k);
                            p.write(": ");
                            p.print_expr(v);
                        }
                    });
                    self.newline();
                    self.write("}");
                } else {
                    self.write("{");
                    for (i, (k, v)) in pairs.iter().enumerate() {
                        if i > 0 {
                            self.write(", ");
                        }
                        self.print_expr(k);
                        self.write(": ");
                        self.print_expr(v);
                    }
                    self.write("}");
                }
            }
            ExprKind::Block(stmts) => {
                self.write("{");
                self.newline();
                self.print_block_body(stmts);
                self.write("}");
            }
            ExprKind::Assign { target, value } => {
                self.print_expr(target);
                self.write(" = ");
                self.print_expr(value);
            }
            ExprKind::CompoundAssign {
                target,
                operator,
                value,
            } => {
                self.print_expr(target);
                self.write(" ");
                self.write(&compound_op_str(*operator));
                self.write(" ");
                self.print_expr(value);
            }
            ExprKind::PostfixIncrement(inner) => {
                self.print_expr(inner);
                self.write("++");
            }
            ExprKind::PostfixDecrement(inner) => {
                self.print_expr(inner);
                self.write("--");
            }
            ExprKind::LogicalAnd { left, right } => {
                if should_logical_break(self, left, right, " && ") {
                    self.print_expr(left);
                    self.newline();
                    self.write("&& ");
                    self.print_expr(right);
                } else {
                    self.print_expr(left);
                    self.write(" && ");
                    self.print_expr(right);
                }
            }
            ExprKind::LogicalOr { left, right } => {
                if should_logical_break(self, left, right, " || ") {
                    self.print_expr(left);
                    self.newline();
                    self.write("|| ");
                    self.print_expr(right);
                } else {
                    self.print_expr(left);
                    self.write(" || ");
                    self.print_expr(right);
                }
            }
            ExprKind::NullishCoalescing { left, right } => {
                self.print_expr(left);
                self.write(" ?? ");
                self.print_expr(right);
            }
            ExprKind::Lambda {
                params,
                return_type,
                body,
            } => {
                // Prefer `fn(params) { body }` for non-block-param lambdas.
                self.write("fn");
                self.print_param_list(params);
                if let Some(ret) = return_type {
                    self.write(" -> ");
                    self.write(&ret.to_string());
                }
                self.write(" {");
                if body.len() == 1 {
                    // Inline single-expression bodies: `fn(x) { x * 2 }`. We
                    // skip the inline form when the inner expression is
                    // itself a Lambda — at statement position inside the
                    // outer body, an implicit `fn(...)` would be parsed as a
                    // function declaration (which needs a name) and fail.
                    // Fall through to the multi-line form, which keeps the
                    // `return` keyword and prints via `print_block_body`.
                    // Also skip inline when it would exceed MAX_LINE_LENGTH.
                    let inner_is_lambda = match &body[0].kind {
                        crate::ast::stmt::StmtKind::Expression(e)
                        | crate::ast::stmt::StmtKind::Return(Some(e)) => {
                            matches!(e.kind, ExprKind::Lambda { .. })
                        }
                        _ => false,
                    };
                    if !inner_is_lambda {
                        // Width estimate must be source-layout independent (use
                        // `span_inline_width`, not raw `.len()` on the source
                        // span — the latter includes newlines+continuation
                        // indent and so disagrees with the second-pass span,
                        // breaking idempotency). The `+9` slack covers the
                        // emitted `" { " + " }"` envelope (5 chars) plus enough
                        // headroom to ensure any inner Call body the lambda
                        // promises to inline can ALSO fit inline at its own
                        // print_arg_list check — otherwise the call breaks
                        // mid-args and we get an ugly `fn() { f(\n  a,\n  b\n) }`
                        // hybrid that the next pass keeps reformatting.
                        if let crate::ast::stmt::StmtKind::Expression(e) = &body[0].kind {
                            let body_w = span_inline_width(self.source, e.span);
                            let est = self.current_column() + 9 + body_w;
                            if est <= MAX_LINE_LENGTH {
                                self.write(" ");
                                self.print_expr(e);
                                self.write(" }");
                                return;
                            }
                        }
                        if let crate::ast::stmt::StmtKind::Return(Some(e)) = &body[0].kind {
                            // `return` is stripped on inline emission, so the
                            // envelope size matches the Expression case.
                            let body_w = span_inline_width(self.source, e.span);
                            let est = self.current_column() + 9 + body_w;
                            if est <= MAX_LINE_LENGTH {
                                self.write(" ");
                                self.print_expr(e);
                                self.write(" }");
                                return;
                            }
                        }
                    }
                }
                self.newline();
                let close_line = super::printer::source_end_line(self.source, expr.span);
                self.print_block_body_through(body, Some(close_line));
                self.write("}");
            }
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                // Ternary `cond ? then : else` — the only surface syntax
                // that produces ExprKind::If (see parser/expressions.rs).
                self.print_expr(condition);
                self.write(" ? ");
                self.print_expr(then_branch);
                self.write(" : ");
                if let Some(eb) = else_branch {
                    self.print_expr(eb);
                } else {
                    self.write("null");
                }
            }
            ExprKind::Match { expression, arms } => {
                self.write("match ");
                self.print_expr(expression);
                self.write(" {");
                self.newline();
                self.with_indent(|p| {
                    for arm in arms {
                        p.print_match_arm(arm);
                    }
                });
                self.write("}");
            }
            ExprKind::ListComprehension { .. } | ExprKind::HashComprehension { .. } => {
                self.write_source_span(expr.span.start, expr.span.end);
            }
            ExprKind::Await(inner) => {
                self.write("await ");
                self.print_expr(inner);
            }
            ExprKind::Spread(inner) => {
                self.write("...");
                self.print_expr(inner);
            }
            ExprKind::Throw(inner) => {
                self.write("throw ");
                self.print_expr(inner);
            }
            ExprKind::Rescue { expr, fallback } => {
                // Estimate full inline width and break before `rescue` when
                // it would push the line past MAX_LINE_LENGTH. Use
                // `span_inline_width` so the same expression yields the same
                // width whether the source currently has it on one line or
                // wrapped across several — required for fmt idempotency.
                let expr_w = span_inline_width(self.source, expr.span);
                let fb_w = span_inline_width(self.source, fallback.span);
                let total = self.current_column() + expr_w + 8 /* " rescue " */ + fb_w;
                if total >= MAX_LINE_LENGTH {
                    self.print_expr(expr);
                    self.newline();
                    self.with_indent(|p| {
                        p.write("rescue ");
                        p.print_expr(fallback);
                    });
                } else {
                    self.print_expr(expr);
                    self.write(" rescue ");
                    self.print_expr(fallback);
                }
            }
        }
    }

    fn print_arg_list(&mut self, args: &[Argument]) {
        let arg_count = args.len();
        // If the estimated inline width exceeds MAX_LINE_LENGTH, break
        // arguments across multiple lines so the formatter doesn't produce
        // lines the linter will flag as style/line-length violations.
        let multi_line = (|| {
            if arg_count <= 1 {
                return false; // Single-arg calls break via their internal formatting
            }
            // For 2-arg calls: break if the total would exceed the limit,
            // or if any argument is a hash with more than 1 pair.
            if arg_count == 2 {
                let has_multi_hash = args.iter().any(|a| {
                    if let Argument::Positional(e) = a {
                        if let ExprKind::Hash(pairs) = &e.kind {
                            return pairs.len() > 1;
                        }
                    }
                    false
                });
                if has_multi_hash {
                    return true;
                }
            }
            // Per-arg width capped at 60 to keep multi-line aggregates
            // (lambdas with statement-blocks, big arrays/hashes) from
            // dominating the inline estimate — their flattened width
            // misrepresents the actual emitted width (which will be
            // multi-line anyway, so contributes only the opening token to
            // the current line). The previous `+8` safety on top of the
            // cap was over-pessimistic and caused lines that genuinely
            // fit in 120 chars to be wrapped, which the next fmt pass
            // would then re-collapse — fmt oscillating between forms.
            let args_w: usize = args.iter().map(|a| {
                let span = match a {
                    Argument::Positional(e) => e.span,
                    Argument::Named(na) => na.value.span,
                    Argument::Block(e) => e.span,
                };
                span_inline_width(self.source, span).min(60)
            }).sum::<usize>()
                + 2 // "()"
                + (arg_count.saturating_sub(1)) * 2; // ", "
            // +2 slack absorbs minor byte/char drift (multi-byte chars
            // push emission past the char-count estimate by a couple bytes)
            // and any trailing `;`/` }` the caller appends.
            self.current_column() + args_w + 2 > MAX_LINE_LENGTH
        })();

        if multi_line {
            self.write("(");
            self.newline();
            self.with_indent(|p| {
                for (i, a) in args.iter().enumerate() {
                    if i > 0 {
                        p.write(",");
                        p.newline();
                    }
                    p.print_arg(a);
                }
            });
            self.newline();
            self.write(")");
        } else {
            self.write("(");
            for (i, a) in args.iter().enumerate() {
                if i > 0 {
                    self.write(", ");
                }
                self.print_arg(a);
            }
            self.write(")");
        }
    }

    fn print_arg(&mut self, a: &Argument) {
        match a {
            Argument::Positional(e) => self.print_expr(e),
            Argument::Named(na) => {
                self.write(&na.name);
                self.write(": ");
                self.print_expr(&na.value);
            }
            Argument::Block(e) => {
                self.write("&");
                self.print_block_arg_expr(e);
            }
        }
    }

    fn print_block_arg_expr(&mut self, e: &Expr) {
        match &e.kind {
            ExprKind::Lambda { params, body, .. } => {
                self.write("{");
                if !params.is_empty() {
                    self.write(" |");
                    for (i, p) in params.iter().enumerate() {
                        if i > 0 {
                            self.write(", ");
                        }
                        if p.is_block_param {
                            self.write("&");
                        }
                        self.write(&p.name);
                    }
                    self.write("|");
                }
                if body.is_empty() {
                    self.write(" }");
                } else {
                    self.newline();
                    self.print_block_body(body);
                    self.write("}");
                }
            }
            _ => self.print_expr(e),
        }
    }

    /// Print a binary operation. For string concatenation (`+`), estimate the
    /// total width before printing and break the chain into continuation lines
    /// when it would exceed MAX_LINE_LENGTH.
    fn print_binary_op(&mut self, left: &Expr, operator: BinaryOp, right: &Expr) {
        let op_str = binary_op_str(operator);

        // Stylistic rewrite: collapse `x == null` / `x == nil` to `x.nil?`,
        // and `x != null` / `x != nil` to `x.present?`. Handles either
        // ordering of operands. Note: `.present?` differs from `!= null`
        // for empty strings/arrays (it returns false); this is intentional
        // per project style.
        if op_str == "==" || op_str == "!=" {
            let null_method = if op_str == "==" { ".nil?" } else { ".present?" };
            if matches!(right.kind, ExprKind::Null) {
                self.print_expr(left);
                self.write(null_method);
                return;
            }
            if matches!(left.kind, ExprKind::Null) {
                self.print_expr(right);
                self.write(null_method);
                return;
            }
        }

        // For + concatenation: estimate total width and break across lines
        // if it would exceed the limit. Use `span_inline_width` so the same
        // expression produces the same width regardless of whether the
        // source currently has it inline or wrapped — required for fmt
        // idempotency.
        if op_str == "+" || op_str == "||" || op_str == "&&" {
            let left_src = span_inline_width(self.source, left.span);
            let right_src = span_inline_width(self.source, right.span);
            let total = self.current_column() + left_src + 3 + right_src.min(80);

            if total + 12 > MAX_LINE_LENGTH {
                // Recursive multi-line: print left inline (which may itself
                // trigger this check at inner levels), then break right.
                self.print_expr(left);
                self.newline();
                self.write("+ ");
                self.print_expr(right);
                return;
            }
        }

        self.print_expr(left);
        self.write(" ");
        self.write(&op_str);
        self.write(" ");
        self.print_expr(right);
    }

    fn print_match_arm(&mut self, arm: &MatchArm) {
        self.print_match_pattern(&arm.pattern);
        if let Some(g) = &arm.guard {
            self.write(" if ");
            self.print_expr(g);
        }
        self.write(" => ");
        self.print_expr(&arm.body);
        self.write(",");
        self.newline();
    }

    fn print_match_pattern(&mut self, p: &MatchPattern) {
        match p {
            MatchPattern::Wildcard => self.write("_"),
            MatchPattern::Variable(name) => self.write(name),
            MatchPattern::Typed { name, type_name } => {
                self.write(name);
                self.write(": ");
                self.write(type_name);
            }
            MatchPattern::Literal(kind) => {
                // Re-wrap into a temporary Expr to reuse the literal printer.
                let tmp_expr = Expr {
                    kind: kind.clone(),
                    span: crate::span::Span::new(0, 0, 0, 0),
                };
                self.print_expr(&tmp_expr);
            }
            MatchPattern::Array { elements, rest } => {
                self.write("[");
                for (i, el) in elements.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.print_match_pattern(el);
                }
                if let Some(rest_name) = rest {
                    if !elements.is_empty() {
                        self.write(", ");
                    }
                    self.write("...");
                    self.write(rest_name);
                }
                self.write("]");
            }
            MatchPattern::Hash { fields, rest } => {
                self.write("{");
                for (i, (name, pat)) in fields.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.write(name);
                    self.write(": ");
                    self.print_match_pattern(pat);
                }
                if let Some(rest_name) = rest {
                    if !fields.is_empty() {
                        self.write(", ");
                    }
                    self.write("...");
                    self.write(rest_name);
                }
                self.write("}");
            }
            MatchPattern::Destructuring { type_name, fields } => {
                self.write(type_name);
                self.write(" { ");
                for (i, (name, pat)) in fields.iter().enumerate() {
                    if i > 0 {
                        self.write(", ");
                    }
                    self.write(name);
                    self.write(": ");
                    self.print_match_pattern(pat);
                }
                self.write(" }");
            }
            MatchPattern::And(pats) => {
                for (i, pat) in pats.iter().enumerate() {
                    if i > 0 {
                        self.write(" & ");
                    }
                    self.print_match_pattern(pat);
                }
            }
            MatchPattern::Or(pats) => {
                for (i, pat) in pats.iter().enumerate() {
                    if i > 0 {
                        self.write(" | ");
                    }
                    self.print_match_pattern(pat);
                }
            }
        }
    }
}

fn binary_op_str(op: BinaryOp) -> String {
    op.to_string()
}

fn compound_op_str(op: CompoundOp) -> String {
    op.to_string()
}

/// Inspect source bytes starting at `at` (the position right after a call's
/// callee). Returns true if the first non-whitespace byte is `(` — i.e., the
/// source spelled the call with parens. Used to preserve paren-less DSL forms
/// like `class TestSoft < Model\n  soft_delete\nend` that the parser rejects
/// when surrounded by parens.
fn source_has_parens_after(source: &str, at: usize) -> bool {
    let bytes = source.as_bytes();
    let mut i = at.min(bytes.len());
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    bytes.get(i) == Some(&b'(')
}

/// True when the source byte at `at` is `@` — used to distinguish the
/// `@name` instance-var sigil from `this.name` (they share an AST node).
fn source_starts_with_at(source: &str, at: usize) -> bool {
    source.as_bytes().get(at) == Some(&b'@')
}

/// True when the source bytes starting at `at` spell `nil` (Soli accepts
/// `nil` and `null` interchangeably). Lets the formatter preserve the
/// user's choice rather than normalizing all null literals to `null`.
fn source_starts_with_nil(source: &str, at: usize) -> bool {
    let bytes = source.as_bytes();
    if bytes.get(at..at + 3) == Some(b"nil") {
        // Word boundary: next byte (if any) must not be an identifier char.
        match bytes.get(at + 3).copied() {
            Some(c) => !(c.is_ascii_alphanumeric() || c == b'_'),
            None => true,
        }
    } else {
        false
    }
}
