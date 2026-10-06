//! ERB-style template parser for Soli.
//!
//! Parses templates with syntax like:
//! - `<%= expr %>` - HTML-escaped output
//! - `<%- expr %>` - Raw/unescaped output
//! - `<% code %>` - Control flow (if, unless, for, end, else, elsif)
//! - `<%= yield %>` - Layout content insertion point
//!
//! `<%== expr %>` was removed in SEC-023 — it decoded HTML entities and
//! emitted the result raw, which silently re-created `<script>` from
//! `&lt;script&gt;` whenever a value had been round-tripped through
//! escape-encoded storage. Templates that reach for it are rejected at
//! parse time with a migration hint.

/// The pre-parsed pieces of a `form_with ... do |f|` block: the
/// `form_with(...)` builder call, the block variable, and the synthesized
/// `<var>.open()` / `<var>.close()` calls the renderer wraps the body in.
#[derive(Debug, Clone, PartialEq)]
pub struct FormWithParts {
    pub builder_expr: crate::ast::expr::Expr,
    pub var: String,
    pub open_expr: crate::ast::expr::Expr,
    pub close_expr: crate::ast::expr::Expr,
}

/// Parts for a component block.
#[derive(Debug, Clone, PartialEq)]
pub struct ComponentParts {
    pub name: crate::ast::expr::Expr,
    pub props: Option<crate::ast::expr::Expr>,
    /// Block variable in `component "x" do |c|` (the slot-builder). `None` for a
    /// bare `do`. `c.slot("name") do … end` calls in the body are desugared to
    /// `ContentFor` at parse time, so this var is a parse-time marker only.
    pub var: Option<String>,
}

/// A node in the template AST.
#[derive(Debug, Clone, PartialEq)]
pub enum TemplateNode {
    /// Raw HTML/text content
    Literal(String),
    /// If conditional block
    If {
        condition: crate::ast::expr::Expr,
        body: Vec<TemplateNode>,
        else_body: Option<Vec<TemplateNode>>,
        line: usize,
    },
    /// For loop block
    For {
        var: String,
        index_var: Option<String>,
        iterable: crate::ast::expr::Expr,
        body: Vec<TemplateNode>,
        line: usize,
    },
    /// Layout content insertion point. `None` is the plain `<%= yield %>`
    /// (the whole rendered view); `Some(name)` is `<%= yield "name" %>` /
    /// `<%= content_for "name" %>`, spliced from the content_for store.
    Yield(Option<String>),
    /// Named content capture: `<% content_for "name" do %> ... <% end %>`.
    /// The body renders into the content_for store instead of the output.
    ContentFor {
        name: String,
        body: Vec<TemplateNode>,
        line: usize,
    },
    /// Form-builder block: `<% form_with(record) do |f| %> ... <% end %>`.
    /// Sugar for binding the builder and wrapping the body in `f.open()` /
    /// `f.close()`. The exprs live behind a Box so the enum stays small.
    FormWith {
        parts: Box<FormWithParts>,
        body: Vec<TemplateNode>,
        line: usize,
    },
    /// Component block: `<%- component "card", title: "x" do %> ... <%- end %>`.
    /// Captures body as default slot content.
    Component {
        parts: Box<ComponentParts>,
        body: Vec<TemplateNode>,
        line: usize,
    },
    /// Render a partial template
    Partial {
        name: String,
        context: Option<crate::ast::expr::Expr>,
        line: usize,
    },
    /// Code block parsed by the core language parser (full language support)
    CoreCodeBlock {
        stmts: Vec<crate::ast::stmt::Stmt>,
        line: usize,
    },
    /// Output expression parsed by the core language parser (full language support)
    CoreOutput {
        expr: crate::ast::expr::Expr,
        escaped: bool,
        line: usize,
    },
}

/// Token types during lexing
#[derive(Debug, Clone, PartialEq)]
enum Token {
    Literal(String, usize),       // content, line
    OutputEscaped(String, usize), // <%= ... %>, line
    OutputRaw(String, usize),     // <%- ... %>, line
    /// `<%== ... %>` — removed in SEC-023. The lexer still recognizes the
    /// syntax so the parser can produce a clean migration error, instead
    /// of dropping into `<%=` and treating the trailing `=` as an operator.
    OutputUnescape(String, usize),
    Code(String, usize), // <% ... %>, line
}

/// Parse an ERB-style template into an AST.
pub fn parse_template(source: &str) -> Result<Vec<TemplateNode>, String> {
    let tokens = tokenize(source)?;
    parse_tokens(&tokens)
}

/// Maximum nesting depth for template block constructs (`<% if %>`, `<% for %>`,
/// `content_for`, `form_with`/component blocks). Recursive block parsing on an
/// adversarial template (`<% if %><% if %>...`) would otherwise overflow the
/// stack, which aborts the process without unwinding — beyond the reach of the
/// server's `catch_unwind` fault isolation. Kept at 64 so even debug builds
/// (with their much larger frames) stay safely under 2 MB stacks.
const MAX_TEMPLATE_BLOCK_DEPTH: usize = 64;

thread_local! {
    static BLOCK_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Enter a nested template-block level; errors past [`MAX_TEMPLATE_BLOCK_DEPTH`].
fn enter_template_block(line: usize) -> Result<(), String> {
    let next = BLOCK_DEPTH.with(|d| {
        d.set(d.get() + 1);
        d.get()
    });
    if next > MAX_TEMPLATE_BLOCK_DEPTH {
        exit_template_block();
        return Err(format!(
            "template blocks nested too deeply (max depth {MAX_TEMPLATE_BLOCK_DEPTH}) at line {line}"
        ));
    }
    Ok(())
}

fn exit_template_block() {
    BLOCK_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
}

/// Rewrite a Ruby-style block-iteration opener — `xs.each do |x|` or
/// `xs.each do |x, i|` — into the engine's `for` form (`for x in xs` /
/// `for x, i in xs`). Returns `None` when the code isn't that shape (a
/// complete inline statement ends with `end`, not `|param|`, so only
/// multi-tag block openers match).
fn rewrite_each_opener(code: &str) -> Option<String> {
    let rest = code.trim().strip_suffix('|')?;
    let bar = rest.rfind('|')?;
    let params = rest[bar + 1..].trim();
    let head = rest[..bar].trim_end().strip_suffix("do")?.trim_end();
    let iterable = head.strip_suffix(".each")?.trim_end();
    if iterable.is_empty() || params.is_empty() {
        return None;
    }
    let idents: Vec<&str> = params.split(',').map(str::trim).collect();
    let valid_ident = |s: &str| {
        !s.is_empty()
            && !s.starts_with(|c: char| c.is_ascii_digit())
            && s.chars().all(|c| c.is_alphanumeric() || c == '_')
    };
    if idents.len() > 2 || !idents.iter().all(|p| valid_ident(p)) {
        return None;
    }
    Some(format!("for {} in {}", idents.join(", "), iterable))
}

/// Split a builder block opener — `form_with(...) do [|var|]` or
/// `<recv>.fields_for(...) do [|var|]` — into (builder-call source, block
/// variable, wraps_output). `wraps_output` is true for `form_with` (the
/// body is wrapped in `open()`/`close()`); a `fields_for` block only binds
/// the sub-builder. `None` when the code isn't a block opener (e.g. a plain
/// `<% f = form_with(post) %>` assignment).
fn form_with_block_parts(code: &str) -> Option<(&str, String, bool)> {
    let code = code.trim();
    let (head, var) = if let Some(rest) = code.strip_suffix('|') {
        let bar = rest.rfind('|')?;
        let var = rest[bar + 1..].trim();
        let head = rest[..bar].trim_end().strip_suffix("do")?.trim_end();
        let valid_ident = !var.is_empty()
            && !var.starts_with(|c: char| c.is_ascii_digit())
            && var.chars().all(|c| c.is_alphanumeric() || c == '_');
        if !valid_ident {
            return None;
        }
        (head, var.to_string())
    } else {
        (code.strip_suffix("do")?.trim_end(), "f".to_string())
    };
    if head.starts_with("form_with") {
        Some((head, var, true))
    } else if head.contains(".fields_for(") && head.ends_with(')') {
        Some((head, var, false))
    } else {
        None
    }
}

/// Detect `component "name", props do [|c|]` or `component("name", props) do [|c|]`
/// openers. Returns `(head, block-var)` where `head` is the call part before `do`
/// and block-var is the optional `|c|` slot-builder binding (mirrors form_with).
fn component_block_parts(code: &str) -> Option<(&str, Option<String>)> {
    let code = code.trim();
    let (before_var, var) = if let Some(rest) = code.strip_suffix('|') {
        let bar = rest.rfind('|')?;
        let name = rest[bar + 1..].trim();
        let valid = !name.is_empty()
            && !name.starts_with(|c: char| c.is_ascii_digit())
            && name.chars().all(|c| c.is_alphanumeric() || c == '_');
        if !valid {
            return None;
        }
        (rest[..bar].trim_end(), Some(name.to_string()))
    } else {
        (code, None)
    };
    let head = before_var.strip_suffix("do")?.trim_end();
    if head.starts_with("component") {
        Some((head, var))
    } else {
        None
    }
}

/// Detect a slot-builder opener `<var>.slot("name") do` inside a component block
/// whose block variable is `var`. Returns the raw `slot(...)` args (e.g. `"name"`).
fn slot_block_open<'a>(code: &'a str, var: &str) -> Option<&'a str> {
    let inner = code.trim().strip_suffix("do")?.trim_end();
    let prefix = format!("{}.slot(", var);
    inner
        .strip_prefix(prefix.as_str())?
        .strip_suffix(')')
        .map(str::trim)
}

/// Whether `code` is a slot-builder opener `<ident>.slot(...) do` for *some*
/// identifier. Used by the tokenizer to fold such openers to `Code` tokens
/// regardless of tag style; the component body parser then validates the
/// receiver against the actual block variable via [`slot_block_open`].
fn is_slot_block_opener(code: &str) -> bool {
    let Some(inner) = code.trim().strip_suffix("do") else {
        return false;
    };
    let inner = inner.trim_end();
    let Some(dot) = inner.find(".slot(") else {
        return false;
    };
    let ident = &inner[..dot];
    !ident.is_empty()
        && !ident.starts_with(|c: char| c.is_ascii_digit())
        && ident.chars().all(|c| c.is_alphanumeric() || c == '_')
        && inner.ends_with(')')
}

/// Tokenize the template source into a sequence of tokens.
fn tokenize(source: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let mut chars = source.chars().peekable();
    let mut current_literal = String::new();
    let mut current_line: usize = 1;
    let mut literal_start_line: usize = 1;

    while let Some(c) = chars.next() {
        if c == '<' && chars.peek() == Some(&'%') {
            // Start of a tag
            chars.next(); // consume '%'
            let tag_line = current_line;

            // Save any accumulated literal
            if !current_literal.is_empty() {
                tokens.push(Token::Literal(
                    std::mem::take(&mut current_literal),
                    literal_start_line,
                ));
            }

            // Check for comment tag <%# ... %> — consumed silently, emits nothing
            let is_comment = chars.peek() == Some(&'#');

            // Check for output tag types: <%== (unescape), <%= (escaped), or <%- (raw)
            let is_output = !is_comment && chars.peek() == Some(&'=');
            let is_raw = !is_comment && chars.peek() == Some(&'-');
            let mut is_unescape = false;

            if is_comment {
                chars.next(); // consume '#'
            } else if is_output {
                chars.next(); // consume first '='
                              // Check for second '=' (<%==)
                if chars.peek() == Some(&'=') {
                    chars.next(); // consume second '='
                    is_unescape = true;
                }
            } else if is_raw {
                chars.next(); // consume '-'
            }

            // Read until closing %>
            let mut tag_content = String::new();
            loop {
                match chars.next() {
                    Some('%') if chars.peek() == Some(&'>') => {
                        chars.next(); // consume '>'
                        break;
                    }
                    Some('\n') => {
                        current_line += 1;
                        tag_content.push('\n');
                    }
                    Some(ch) => tag_content.push(ch),
                    None => return Err(format!("Unclosed template tag at line {}", tag_line)),
                }
            }

            // ERB-style `-%>` trim marker: strip it here and swallow the
            // newline right after the tag (below), so block tags don't
            // leave blank lines in the rendered output.
            let mut tag_content = tag_content.trim().to_string();
            let trim_following_newline = tag_content.ends_with('-');
            if trim_following_newline {
                tag_content.pop();
                tag_content.truncate(tag_content.trim_end().len());
            }

            if is_comment {
                // discard — <%# comment %> is silently dropped; content is never rendered or executed
            } else if tag_content == "end"
                || form_with_block_parts(&tag_content).is_some()
                || component_block_parts(&tag_content).is_some()
                || is_slot_block_opener(&tag_content)
            {
                // `form_with(...) do |f|`, `component ... do [|c|]`, `c.slot(...) do`
                // block openers and their `end` read naturally as output tags
                // (`<%- %>` / `<%= %>`, Rails-style). Normalize them to Code tokens
                // so the block dispatcher sees them regardless of tag style.
                tokens.push(Token::Code(tag_content, tag_line));
            } else if is_raw {
                tokens.push(Token::OutputRaw(tag_content, tag_line));
            } else if is_unescape {
                tokens.push(Token::OutputUnescape(tag_content, tag_line));
            } else if is_output {
                tokens.push(Token::OutputEscaped(tag_content, tag_line));
            } else {
                // Ruby-style iteration: `<% xs.each do |x| %>` (and the
                // `|x, i|` form) is sugar for `<% for x in xs %>` —
                // normalize here so every block-dispatch site (top
                // level, if/for bodies) resolves it via the same For
                // node machinery.
                let tag_content = rewrite_each_opener(&tag_content).unwrap_or(tag_content);
                tokens.push(Token::Code(tag_content, tag_line));
            }

            if trim_following_newline {
                if chars.peek() == Some(&'\r') {
                    chars.next();
                }
                if chars.peek() == Some(&'\n') {
                    chars.next();
                    current_line += 1;
                }
            }

            // Reset literal start line for next literal
            literal_start_line = current_line;
        } else {
            if current_literal.is_empty() {
                literal_start_line = current_line;
            }
            if c == '\n' {
                current_line += 1;
            }
            current_literal.push(c);
        }
    }

    // Don't forget trailing literal
    if !current_literal.is_empty() {
        tokens.push(Token::Literal(current_literal, literal_start_line));
    }

    Ok(tokens)
}

/// Extract a line-preserving Soli source string from a `.slv` template: only
/// the code inside `<% %>` / `<%= %>` / `<%- %>` regions is kept, with the
/// literal HTML dropped. The linter feeds this to the normal Soli lexer/parser
/// so it can run rules on the embedded code without choking on markup —
/// apostrophes in HTML text would otherwise be read as string delimiters and
/// `<` as an operator, both of which abort the lex/parse before any rule runs.
///
/// Newlines are preserved so diagnostics map back to the original template
/// lines. When two tags share a source line, the second is pushed onto the
/// next line rather than separated by `;` — Soli rejects `;` immediately after
/// `if`/`for`, so a newline is the only separator that keeps the synthesized
/// source parseable. This can nudge such a region's reported line down by one,
/// an acceptable trade. Bodies that contain only HTML (`<% if x %>…markup…<% end %>`)
/// extract to an empty block; the caller drops `style/empty-block` for
/// templates so that isn't reported as a false positive.
pub fn extract_lintable_code(source: &str) -> Result<String, String> {
    let tokens = tokenize(source)?;
    let mut out = String::new();
    let mut cur_line: usize = 1;
    let mut line_has_code = false;

    for token in &tokens {
        let (snippet, line): (std::borrow::Cow<'_, str>, usize) = match token {
            Token::Literal(..) => continue,
            Token::Code(code, line) => {
                let code = code.trim();
                if content_for_block_open(code).is_some() {
                    // A capture-open tag isn't Soli, but its `end` is still
                    // consumed by the core parser — synthesize `if true` so
                    // the block stays balanced in the extracted source.
                    (std::borrow::Cow::Borrowed("if true"), *line)
                } else if is_content_for_code(code) {
                    // Malformed capture (missing `do`); the template parser
                    // reports the friendly error, nothing to lint here.
                    continue;
                } else if let Some((_, var, _)) = form_with_block_parts(code) {
                    // A form_with/fields_for block opener: balance its `end`
                    // and bind the block param so `f.text_field(...)` in the
                    // body doesn't trip undefined-local.
                    (std::borrow::Cow::Owned(format!("for {} in []", var)), *line)
                } else if component_block_parts(code).is_some() {
                    // Component block opener: the head (component call) is Soli,
                    // the body will be handled as captured content. Treat head as-is.
                    (std::borrow::Cow::Borrowed(code), *line)
                } else {
                    (std::borrow::Cow::Borrowed(code), *line)
                }
            }
            Token::OutputEscaped(expr, line)
            | Token::OutputRaw(expr, line)
            | Token::OutputUnescape(expr, line) => {
                let expr = expr.trim();
                // `yield` / `yield "name"` / the `content_for "name"`
                // read-form are layout directives, not lintable Soli.
                // (`content_for?(...)` is a real call and stays lintable.)
                if parse_yield_directive(expr, *line).is_some() {
                    continue;
                }
                (std::borrow::Cow::Borrowed(expr), *line)
            }
        };
        if snippet.is_empty() {
            continue;
        }

        while cur_line < line {
            out.push('\n');
            cur_line += 1;
            line_has_code = false;
        }
        if line_has_code {
            out.push('\n');
            cur_line += 1;
        }
        out.push_str(&snippet);
        cur_line += snippet.matches('\n').count();
        line_has_code = true;
    }

    Ok(out)
}

/// Escaped-output expressions whose entire value is a redundant escape-helper
/// call. `<%= %>` already HTML-escapes its output, so `<%= h(x) %>`,
/// `<%= html_escape(x) %>` and `<%= attr(x) %>` escape twice — the page shows
/// literal `&#x27;` / `&amp;` wherever the value contains a special character.
/// Returns `(template_line, helper_name)` pairs for `lint_file` to report.
pub fn redundant_escape_helpers(source: &str) -> Result<Vec<(usize, &'static str)>, String> {
    // `html_escape` before `h`: both start with `h`, and the paren check after
    // the prefix keeps `h` from matching `html_escape(...)` — this order just
    // reports the accurate helper name on the first try.
    const HELPERS: [&str; 3] = ["html_escape", "h", "attr"];

    let tokens = tokenize(source)?;
    let mut found = Vec::new();
    for token in &tokens {
        let Token::OutputEscaped(expr, line) = token else {
            continue;
        };
        let expr = expr.trim();
        for helper in HELPERS {
            let Some(rest) = expr.strip_prefix(helper) else {
                continue;
            };
            // Only a call whose parenthesized arguments span the rest of the
            // expression: `<%= h(x) %>` is flagged, `<%= h(x) + y %>` and
            // `<%= hello(x) %>` are not.
            if rest.starts_with('(') && paren_group_spans_whole(rest) {
                found.push((*line, helper));
                break;
            }
        }
    }
    Ok(found)
}

/// True when the `(` opening `text` finds its matching `)` exactly at the last
/// character — i.e. one parenthesized group is the entire text (`(a(b))` yes,
/// `(a) + (b)` no). String literals are skipped so parens inside them don't
/// count; an unterminated literal or unbalanced parens conservatively return
/// false.
fn paren_group_spans_whole(text: &str) -> bool {
    let mut depth: usize = 0;
    let mut in_string: Option<char> = None;
    let mut escaped = false;
    for (i, c) in text.char_indices() {
        if let Some(quote) = in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == quote {
                in_string = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => in_string = Some(c),
            '(' => depth += 1,
            ')' => {
                if depth == 0 {
                    return false;
                }
                depth -= 1;
                if depth == 0 {
                    return i == text.len() - 1;
                }
            }
            _ => {}
        }
    }
    false
}

/// Parse tokens into an AST.
fn parse_tokens(tokens: &[Token]) -> Result<Vec<TemplateNode>, String> {
    let mut nodes = Vec::new();
    let mut i = 0;

    while i < tokens.len() {
        match &tokens[i] {
            Token::Code(code, line) => {
                let code = code.trim();

                if let Some(rest) = code.strip_prefix("if ") {
                    // Parse if block
                    let condition = parse_core_expr(rest.trim(), *line)?;
                    let (if_node, consumed) = parse_if_block(&tokens[i..], condition, *line)?;
                    nodes.push(if_node);
                    i += consumed;
                } else if let Some(rest) = code.strip_prefix("unless ") {
                    // Parse unless block
                    let (unless_node, consumed) = parse_unless_block(&tokens[i..], rest, *line)?;
                    nodes.push(unless_node);
                    i += consumed;
                } else if code.starts_with("for ") {
                    // Parse for loop
                    let (for_node, consumed) = parse_for_block(&tokens[i..], *line)?;
                    nodes.push(for_node);
                    i += consumed;
                } else if is_content_for_code(code) {
                    // Parse content_for capture block
                    let (cf_node, consumed) = parse_content_for_block(&tokens[i..], *line)?;
                    nodes.push(cf_node);
                    i += consumed;
                } else if form_with_block_parts(code).is_some() {
                    // Parse form_with builder block
                    let (fw_node, consumed) = parse_form_with_block(&tokens[i..], *line)?;
                    nodes.push(fw_node);
                    i += consumed;
                } else if component_block_parts(code).is_some() {
                    // Parse component block with body as slot
                    let (comp_node, consumed) = parse_component_block(&tokens[i..], *line)?;
                    nodes.push(comp_node);
                    i += consumed;
                } else if code == "end" || code.starts_with("else") || code.starts_with("elsif ") {
                    // These should be handled by their parent block parsers
                    return Err(format!(
                        "Unexpected '{}' outside of block at line {}",
                        code, line
                    ));
                } else {
                    // Parse through the core language parser for full language support
                    let stmts = parse_core_code(code, *line)?;
                    nodes.push(TemplateNode::CoreCodeBlock { stmts, line: *line });
                    i += 1;
                }
            }
            token => {
                nodes.push(parse_output_token(token)?);
                i += 1;
            }
        }
    }

    Ok(nodes)
}

/// Parse an if block starting at the given position.
/// Returns the IfNode and the number of tokens consumed.
fn parse_if_block(
    tokens: &[Token],
    condition: crate::ast::expr::Expr,
    if_line: usize,
) -> Result<(TemplateNode, usize), String> {
    parse_conditional_block(tokens, condition, if_line, "if")
}

/// `<% unless cond %> … <% else %> … <% end %>`.
///
/// The same block as `if` on the inverted condition, so the renderer sees one
/// node kind and nothing downstream has to learn a second one. `elsif` is
/// refused, as it is after an `unless` in the language itself: "unless A, else
/// if B" reads as a puzzle rather than a guard.
///
/// Without this, `<% unless … %>` fell through to the core parser as a
/// statement complete in its own tag — and since a block-form `unless` there
/// does not insist on its `end`, it parsed as an *empty* one, silently. The
/// template's `<% end %>` then arrived with no block open, and the error named
/// that line rather than this one.
fn parse_unless_block(
    tokens: &[Token],
    rest: &str,
    unless_line: usize,
) -> Result<(TemplateNode, usize), String> {
    let condition = parse_core_expr(rest.trim(), unless_line)?;
    let span = condition.span;
    let negated = crate::ast::expr::Expr::new(
        crate::ast::expr::ExprKind::Unary {
            operator: crate::ast::expr::UnaryOp::Not,
            operand: Box::new(condition),
        },
        span,
    );
    parse_conditional_block(tokens, negated, unless_line, "unless")
}

/// The body of an `if` or an `unless`, up to its `end`. `keyword` is the one
/// that opened it: it names the block in a diagnostic, and says whether
/// `elsif` may close a branch of it.
fn parse_conditional_block(
    tokens: &[Token],
    condition: crate::ast::expr::Expr,
    if_line: usize,
    keyword: &str,
) -> Result<(TemplateNode, usize), String> {
    enter_template_block(if_line)?;
    let result = parse_if_block_inner(tokens, condition, if_line, keyword);
    exit_template_block();
    result
}

fn parse_if_block_inner(
    tokens: &[Token],
    condition: crate::ast::expr::Expr,
    if_line: usize,
    keyword: &str,
) -> Result<(TemplateNode, usize), String> {
    let mut body = Vec::new();
    let mut else_body = None;
    let mut i = 1; // Skip the initial `if` token
    let mut in_else = false;

    while i < tokens.len() {
        match &tokens[i] {
            Token::Code(code, line) => {
                let code = code.trim();

                if code == "end" {
                    return Ok((
                        TemplateNode::If {
                            condition,
                            body,
                            else_body,
                            line: if_line,
                        },
                        i + 1,
                    ));
                } else if code == "else" {
                    in_else = true;
                    else_body = Some(Vec::new());
                    i += 1;
                } else if let Some(rest) = code.strip_prefix("elsif ") {
                    if keyword != "if" {
                        return Err(format!(
                            "'elsif' cannot follow 'unless' at line {} - use 'if' with the \
                             condition written the other way round",
                            line
                        ));
                    }
                    // Handle elsif as nested if in else
                    let elsif_condition = parse_core_expr(rest.trim(), *line)?;
                    let (elsif_node, consumed) =
                        parse_if_block(&tokens[i..], elsif_condition, *line)?;
                    else_body = Some(vec![elsif_node]);
                    // The elsif consumed tokens up to 'end', so we're done
                    return Ok((
                        TemplateNode::If {
                            condition,
                            body,
                            else_body,
                            line: if_line,
                        },
                        i + consumed,
                    ));
                } else if let Some(rest) = code.strip_prefix("if ") {
                    // Nested if
                    let nested_condition = parse_core_expr(rest.trim(), *line)?;
                    let (nested_if, consumed) =
                        parse_if_block(&tokens[i..], nested_condition, *line)?;
                    if in_else {
                        else_body.as_mut().unwrap().push(nested_if);
                    } else {
                        body.push(nested_if);
                    }
                    i += consumed;
                } else if let Some(rest) = code.strip_prefix("unless ") {
                    // Nested unless. `in_else` and `else_body.is_some()` are
                    // set together above, so the filter is what the `unwrap`
                    // on the arms around this one asserts — without the
                    // panic, which `scripts/lint_unwraps.sh` counts.
                    let (nested_unless, consumed) = parse_unless_block(&tokens[i..], rest, *line)?;
                    match else_body.as_mut().filter(|_| in_else) {
                        Some(branch) => branch.push(nested_unless),
                        None => body.push(nested_unless),
                    }
                    i += consumed;
                } else if code.starts_with("for ") {
                    // Nested for
                    let (nested_for, consumed) = parse_for_block(&tokens[i..], *line)?;
                    if in_else {
                        else_body.as_mut().unwrap().push(nested_for);
                    } else {
                        body.push(nested_for);
                    }
                    i += consumed;
                } else if is_content_for_code(code) {
                    // Nested content_for
                    let (nested_cf, consumed) = parse_content_for_block(&tokens[i..], *line)?;
                    if in_else {
                        else_body.as_mut().unwrap().push(nested_cf);
                    } else {
                        body.push(nested_cf);
                    }
                    i += consumed;
                } else if form_with_block_parts(code).is_some() {
                    // Nested form_with block
                    let (nested_fw, consumed) = parse_form_with_block(&tokens[i..], *line)?;
                    if in_else {
                        else_body.as_mut().unwrap().push(nested_fw);
                    } else {
                        body.push(nested_fw);
                    }
                    i += consumed;
                } else if component_block_parts(code).is_some() {
                    // Nested component block
                    let (nested_comp, consumed) = parse_component_block(&tokens[i..], *line)?;
                    if in_else {
                        else_body.as_mut().unwrap().push(nested_comp);
                    } else {
                        body.push(nested_comp);
                    }
                    i += consumed;
                } else {
                    // Other code block - parse through core parser
                    let stmts = parse_core_code(code, *line)?;
                    let node = TemplateNode::CoreCodeBlock { stmts, line: *line };
                    if in_else {
                        else_body.as_mut().unwrap().push(node);
                    } else {
                        body.push(node);
                    }
                    i += 1;
                }
            }
            token => {
                let node = parse_output_token(token)?;
                if in_else {
                    else_body.as_mut().unwrap().push(node);
                } else {
                    body.push(node);
                }
                i += 1;
            }
        }
    }

    Err(format!(
        "Unclosed {} block at line {} - missing 'end'",
        keyword, if_line
    ))
}

/// Parse a for block starting at the given position.
/// Returns the ForNode and the number of tokens consumed.
fn parse_for_block(tokens: &[Token], for_line: usize) -> Result<(TemplateNode, usize), String> {
    enter_template_block(for_line)?;
    let result = parse_for_block_inner(tokens, for_line);
    exit_template_block();
    result
}

fn parse_for_block_inner(
    tokens: &[Token],
    for_line: usize,
) -> Result<(TemplateNode, usize), String> {
    // First token should be the `for` code
    let (var, index_var, iterable) = match &tokens[0] {
        Token::Code(code, _line) => {
            let code = code.trim();
            if !code.starts_with("for ") {
                return Err(format!("Expected 'for' statement at line {}", for_line));
            }
            parse_for_statement(&code[4..], for_line)?
        }
        _ => return Err(format!("Expected 'for' statement at line {}", for_line)),
    };

    let mut body = Vec::new();
    let mut i = 1; // Skip the initial `for` token

    while i < tokens.len() {
        match &tokens[i] {
            Token::Code(code, line) => {
                let code = code.trim();

                if code == "end" {
                    return Ok((
                        TemplateNode::For {
                            var,
                            index_var,
                            iterable,
                            body,
                            line: for_line,
                        },
                        i + 1,
                    ));
                } else if let Some(rest) = code.strip_prefix("if ") {
                    // Nested if
                    let condition = parse_core_expr(rest.trim(), *line)?;
                    let (nested_if, consumed) = parse_if_block(&tokens[i..], condition, *line)?;
                    body.push(nested_if);
                    i += consumed;
                } else if let Some(rest) = code.strip_prefix("unless ") {
                    // Nested unless
                    let (nested_unless, consumed) = parse_unless_block(&tokens[i..], rest, *line)?;
                    body.push(nested_unless);
                    i += consumed;
                } else if code.starts_with("for ") {
                    // Nested for
                    let (nested_for, consumed) = parse_for_block(&tokens[i..], *line)?;
                    body.push(nested_for);
                    i += consumed;
                } else if is_content_for_code(code) {
                    // Nested content_for
                    let (nested_cf, consumed) = parse_content_for_block(&tokens[i..], *line)?;
                    body.push(nested_cf);
                    i += consumed;
                } else if form_with_block_parts(code).is_some() {
                    // Nested form_with block
                    let (nested_fw, consumed) = parse_form_with_block(&tokens[i..], *line)?;
                    body.push(nested_fw);
                    i += consumed;
                } else if component_block_parts(code).is_some() {
                    // Nested component block
                    let (nested_comp, consumed) = parse_component_block(&tokens[i..], *line)?;
                    body.push(nested_comp);
                    i += consumed;
                } else {
                    // Other code block - parse through core parser
                    let stmts = parse_core_code(code, *line)?;
                    body.push(TemplateNode::CoreCodeBlock { stmts, line: *line });
                    i += 1;
                }
            }
            token => {
                body.push(parse_output_token(token)?);
                i += 1;
            }
        }
    }

    Err(format!(
        "Unclosed for block at line {} - missing 'end'",
        for_line
    ))
}

/// Reject a loop variable name that collides with a Soli reserved keyword.
///
/// Without this check the cryptic core-parser error fires only when the
/// loop body references the variable (e.g. `<%= fn %>` lexes `fn` as the
/// `Fn` token and the parser then complains about an unexpected EOF
/// "expected identifier"). Catching the keyword here keeps the diagnostic
/// pointed at the offending `<% for ... %>` tag.
fn ensure_loop_var_not_keyword(name: &str, role: &str, line: usize) -> Result<(), String> {
    if crate::lexer::TokenKind::keyword(name).is_some() {
        Err(format!(
            "Template for-loop {} '{}' at line {} is a reserved keyword. \
             Rename it (e.g. '{}_') so it doesn't collide with Soli syntax.",
            role, name, line, name
        ))
    } else {
        Ok(())
    }
}

/// Parse a for statement like "item in items" or "(item, index in items)"
/// Supports: "x in items" or "x, i in items" where i is the index
fn parse_for_statement(
    s: &str,
    line: usize,
) -> Result<(String, Option<String>, crate::ast::expr::Expr), String> {
    let s = s.trim();

    // Only strip outer parens if the whole expression is wrapped: "(item in items)"
    // Don't strip if it's something like "item in range(1, 5)"
    let s = if s.starts_with('(') && s.ends_with(')') {
        // Check if these are matching outer parens by verifying paren balance
        let inner = &s[1..s.len() - 1];
        let mut depth = 0;
        let mut is_outer_parens = true;
        for c in inner.chars() {
            match c {
                '(' => depth += 1,
                ')' => {
                    if depth == 0 {
                        // Found unmatched ), so the outer parens aren't wrapping the whole thing
                        is_outer_parens = false;
                        break;
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }
        if is_outer_parens && depth == 0 {
            inner.trim()
        } else {
            s
        }
    } else {
        s
    };

    // Look for " in " as the separator
    if let Some(pos) = s.find(" in ") {
        let var_part = s[..pos].trim().to_string();
        let iterable_str = s[pos + 4..].trim();

        if var_part.is_empty() {
            return Err("Missing loop variable in for statement".to_string());
        }
        if iterable_str.is_empty() {
            return Err("Missing iterable in for statement".to_string());
        }

        // Check for index variable: "x, i in items"
        let (var, index_var) = if let Some(comma_pos) = var_part.rfind(',') {
            let var = var_part[..comma_pos].trim().to_string();
            let index_var = var_part[comma_pos + 1..].trim().to_string();
            if var.is_empty() {
                return Err("Missing loop variable in for statement".to_string());
            }
            if index_var.is_empty() {
                return Err("Missing index variable in for statement".to_string());
            }
            (var, Some(index_var))
        } else {
            (var_part, None)
        };

        ensure_loop_var_not_keyword(&var, "variable", line)?;
        if let Some(ref idx) = index_var {
            ensure_loop_var_not_keyword(idx, "index variable", line)?;
        }

        Ok((var, index_var, parse_core_expr(iterable_str, line)?))
    } else {
        Err(format!(
            "Invalid for statement: expected 'var in iterable', got '{}'",
            s
        ))
    }
}

/// Parse a partial render call like "render 'users/_card'" or "render('users/_card', user)"
fn parse_partial_call(expr: &str, line: usize) -> Result<TemplateNode, String> {
    let expr = expr.trim();

    // Handle both "render 'name'" and "render('name', context)" forms
    let args = if let Some(inner) = expr.strip_prefix("render(") {
        // Function call form: render('name', context)
        inner.trim_end_matches(')').trim()
    } else if let Some(rest) = expr.strip_prefix("render ") {
        // Space form: render 'name' or render 'name', context
        rest.trim()
    } else {
        return Err(format!("Invalid render call at line {}: {}", line, expr));
    };

    // Split by comma to get name and optional context
    let parts: Vec<&str> = args.splitn(2, ',').collect();

    let name = parts[0]
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_string();

    let context = if parts.len() > 1 {
        Some(parse_core_expr(parts[1].trim(), line)?)
    } else {
        None
    };

    Ok(TemplateNode::Partial {
        name,
        context,
        line,
    })
}

/// Strip a directive keyword (`yield` / `content_for`) from the start of an
/// expression, requiring a word boundary: the next char must be a space or
/// `(`. This keeps `content_for?("head")` (the predicate builtin) and
/// identifiers like `yield_count` out of the directive path.
fn strip_directive_keyword<'a>(expr: &'a str, keyword: &str) -> Option<&'a str> {
    let rest = expr.strip_prefix(keyword)?;
    match rest.chars().next() {
        Some(c) if c == '(' || c.is_whitespace() => Some(rest),
        _ => None,
    }
}

/// Parse the name argument of a `yield` / `content_for` directive. Only a
/// string literal is accepted — `"head"`, `'head'`, or the parenthesized
/// forms — so the layout/store lookup key is known at parse time.
fn parse_directive_name(args: &str, directive: &str, line: usize) -> Result<String, String> {
    let mut args = args.trim();
    if let Some(inner) = args.strip_prefix('(') {
        if let Some(inner) = inner.strip_suffix(')') {
            args = inner.trim();
        }
    }
    let bad = || {
        Err(format!(
            "{} name must be a string literal (e.g. {} \"head\") at line {}",
            directive, directive, line
        ))
    };
    let mut chars = args.chars();
    let quote = match chars.next() {
        Some(q @ ('"' | '\'')) => q,
        _ => return bad(),
    };
    let Some(name) = args
        .strip_prefix(quote)
        .and_then(|rest| rest.strip_suffix(quote))
    else {
        return bad();
    };
    if name.is_empty() || name.contains(quote) {
        return bad();
    }
    Ok(name.to_string())
}

/// Recognize a `yield` / named-`yield` / `content_for`-read output directive.
/// Returns `None` when the expression isn't a directive at all (falls through
/// to the core parser), `Some(Err)` when it is one but the name is malformed.
fn parse_yield_directive(expr: &str, line: usize) -> Option<Result<TemplateNode, String>> {
    let expr = expr.trim();
    if expr == "yield" {
        return Some(Ok(TemplateNode::Yield(None)));
    }
    let (args, directive) = if let Some(rest) = strip_directive_keyword(expr, "yield") {
        (rest, "yield")
    } else {
        let rest = strip_directive_keyword(expr, "content_for")?;
        (rest, "content_for")
    };
    Some(parse_directive_name(args, directive, line).map(|name| TemplateNode::Yield(Some(name))))
}

/// Parse a `<% form_with(record) do |f| %> ... <% end %>` block starting at
/// the given position. Returns the FormWith node and the tokens consumed.
fn parse_form_with_block(
    tokens: &[Token],
    open_line: usize,
) -> Result<(TemplateNode, usize), String> {
    enter_template_block(open_line)?;
    let result = parse_form_with_block_inner(tokens, open_line);
    exit_template_block();
    result
}

fn parse_form_with_block_inner(
    tokens: &[Token],
    open_line: usize,
) -> Result<(TemplateNode, usize), String> {
    let (head, var, wraps_output) = match &tokens[0] {
        Token::Code(code, _) => form_with_block_parts(code)
            .ok_or_else(|| format!("Expected form_with block at line {}", open_line))?,
        _ => return Err(format!("Expected form_with block at line {}", open_line)),
    };
    ensure_loop_var_not_keyword(&var, "form_with block parameter", open_line)?;
    let builder_expr = parse_core_expr(head, open_line)?;
    // fields_for blocks bind the sub-builder without wrapping the body in
    // any output — their open/close render as empty strings.
    let (open_expr, close_expr) = if wraps_output {
        (
            parse_core_expr(&format!("{}.open()", var), open_line)?,
            parse_core_expr(&format!("{}.close()", var), open_line)?,
        )
    } else {
        (
            parse_core_expr("\"\"", open_line)?,
            parse_core_expr("\"\"", open_line)?,
        )
    };

    let mut body = Vec::new();
    let mut i = 1; // Skip the opener token

    while i < tokens.len() {
        match &tokens[i] {
            Token::Code(code, line) => {
                let code = code.trim();

                if code == "end" {
                    return Ok((
                        TemplateNode::FormWith {
                            parts: Box::new(FormWithParts {
                                builder_expr,
                                var,
                                open_expr,
                                close_expr,
                            }),
                            body,
                            line: open_line,
                        },
                        i + 1,
                    ));
                } else if let Some(rest) = code.strip_prefix("if ") {
                    let condition = parse_core_expr(rest.trim(), *line)?;
                    let (nested_if, consumed) = parse_if_block(&tokens[i..], condition, *line)?;
                    body.push(nested_if);
                    i += consumed;
                } else if let Some(rest) = code.strip_prefix("unless ") {
                    let (nested_unless, consumed) = parse_unless_block(&tokens[i..], rest, *line)?;
                    body.push(nested_unless);
                    i += consumed;
                } else if code.starts_with("for ") {
                    let (nested_for, consumed) = parse_for_block(&tokens[i..], *line)?;
                    body.push(nested_for);
                    i += consumed;
                } else if is_content_for_code(code) {
                    let (nested_cf, consumed) = parse_content_for_block(&tokens[i..], *line)?;
                    body.push(nested_cf);
                    i += consumed;
                } else if form_with_block_parts(code).is_some() {
                    let (nested_fw, consumed) = parse_form_with_block(&tokens[i..], *line)?;
                    body.push(nested_fw);
                    i += consumed;
                } else {
                    let stmts = parse_core_code(code, *line)?;
                    body.push(TemplateNode::CoreCodeBlock { stmts, line: *line });
                    i += 1;
                }
            }
            token => {
                body.push(parse_output_token(token)?);
                i += 1;
            }
        }
    }

    Err(format!(
        "Unclosed form_with block at line {} - missing 'end'",
        open_line
    ))
}

/// Parse a `<% component "name", props do %> ... <% end %>` block starting at
/// the given position. The body is captured as the default slot content.
fn parse_component_block(
    tokens: &[Token],
    open_line: usize,
) -> Result<(TemplateNode, usize), String> {
    enter_template_block(open_line)?;
    let result = parse_component_block_inner(tokens, open_line);
    exit_template_block();
    result
}

fn parse_component_block_inner(
    tokens: &[Token],
    open_line: usize,
) -> Result<(TemplateNode, usize), String> {
    let (head, block_var) = match &tokens[0] {
        Token::Code(code, _) => component_block_parts(code)
            .ok_or_else(|| format!("Expected component block at line {}", open_line))?,
        _ => return Err(format!("Expected component block at line {}", open_line)),
    };

    // Parse the head as a call expr to extract name and props.
    // head e.g. `component "card", { title: x }` or `component("card", props)`
    let call_expr = parse_core_expr(head, open_line)?;
    let (name_expr, props_expr) = match &call_expr.kind {
        crate::ast::expr::ExprKind::Call {
            callee: _,
            arguments,
        } => {
            // callee should resolve to "component" at runtime, we take first arg as name
            if arguments.is_empty() {
                return Err(format!(
                    "component block requires a name at line {}",
                    open_line
                ));
            }
            let name_e = match &arguments[0] {
                crate::ast::expr::Argument::Positional(e) => e.clone(),
                _ => {
                    return Err(format!(
                        "component name must be positional at line {}",
                        open_line
                    ))
                }
            };
            // Props can arrive two ways:
            //   component "card", { "title": x } do    -> a positional hash literal
            //   component "card", title: x, size: y do -> named args
            // The named form parses as `Argument::Named`; fold those into a
            // synthesized hash literal so both spellings reach the renderer as a
            // `Value::Hash`. A leading positional hash wins if present.
            let props_e = if arguments.len() > 1 {
                match &arguments[1] {
                    crate::ast::expr::Argument::Positional(e) => Some(e.clone()),
                    crate::ast::expr::Argument::Named(_) => {
                        let pairs: Vec<(crate::ast::expr::Expr, crate::ast::expr::Expr)> =
                            arguments[1..]
                                .iter()
                                .filter_map(|arg| match arg {
                                    crate::ast::expr::Argument::Named(named) => Some((
                                        crate::ast::expr::Expr::new(
                                            crate::ast::expr::ExprKind::StringLiteral(
                                                named.name.clone(),
                                            ),
                                            named.span,
                                        ),
                                        named.value.clone(),
                                    )),
                                    _ => None,
                                })
                                .collect();
                        Some(crate::ast::expr::Expr::new(
                            crate::ast::expr::ExprKind::Hash(pairs),
                            call_expr.span,
                        ))
                    }
                    crate::ast::expr::Argument::Block(_) => None,
                }
            } else {
                None
            };
            (name_e, props_e)
        }
        _ => {
            // bare component "name" ? treat as name
            (call_expr, None)
        }
    };

    let mut body = Vec::new();
    let mut i = 1; // Skip the opener token

    while i < tokens.len() {
        match &tokens[i] {
            Token::Code(code, line) => {
                let code = code.trim();

                if code == "end" {
                    return Ok((
                        TemplateNode::Component {
                            parts: Box::new(ComponentParts {
                                name: name_expr,
                                props: props_expr,
                                var: block_var,
                            }),
                            body,
                            line: open_line,
                        },
                        i + 1,
                    ));
                } else if let Some(rest) = code.strip_prefix("if ") {
                    let condition = parse_core_expr(rest.trim(), *line)?;
                    let (nested_if, consumed) = parse_if_block(&tokens[i..], condition, *line)?;
                    body.push(nested_if);
                    i += consumed;
                } else if let Some(rest) = code.strip_prefix("unless ") {
                    let (nested_unless, consumed) = parse_unless_block(&tokens[i..], rest, *line)?;
                    body.push(nested_unless);
                    i += consumed;
                } else if code.starts_with("for ") {
                    let (nested_for, consumed) = parse_for_block(&tokens[i..], *line)?;
                    body.push(nested_for);
                    i += consumed;
                } else if is_content_for_code(code) {
                    let (nested_cf, consumed) = parse_content_for_block(&tokens[i..], *line)?;
                    body.push(nested_cf);
                    i += consumed;
                } else if form_with_block_parts(code).is_some() {
                    let (nested_fw, consumed) = parse_form_with_block(&tokens[i..], *line)?;
                    body.push(nested_fw);
                    i += consumed;
                } else if component_block_parts(code).is_some() {
                    let (nested_comp, consumed) = parse_component_block(&tokens[i..], *line)?;
                    body.push(nested_comp);
                    i += consumed;
                } else if let Some(slot_args) =
                    block_var.as_deref().and_then(|v| slot_block_open(code, v))
                {
                    // `c.slot("name") do … end` -> ContentFor capture (named slot).
                    let slot_name = parse_directive_name(slot_args, "slot", *line)?;
                    let (nested_slot, consumed) = parse_slot_block(&tokens[i..], slot_name, *line)?;
                    body.push(nested_slot);
                    i += consumed;
                } else {
                    let stmts = parse_core_code(code, *line)?;
                    body.push(TemplateNode::CoreCodeBlock { stmts, line: *line });
                    i += 1;
                }
            }
            token => {
                body.push(parse_output_token(token)?);
                i += 1;
            }
        }
    }

    Err(format!(
        "Unclosed component block at line {} - missing 'end'",
        open_line
    ))
}

/// Parse a `<% c.slot("name") do %> ... <% end %>` slot block (the slot-builder
/// form inside a component). Desugars to a `ContentFor` node so it shares the
/// content_for store and the `yield "name"` machinery. Mirrors
/// `parse_content_for_block`.
fn parse_slot_block(
    tokens: &[Token],
    slot_name: String,
    slot_line: usize,
) -> Result<(TemplateNode, usize), String> {
    let mut body = Vec::new();
    let mut i = 1; // Skip the `c.slot(...) do` opener token

    while i < tokens.len() {
        match &tokens[i] {
            Token::Code(code, line) => {
                let code = code.trim();

                if code == "end" {
                    return Ok((
                        TemplateNode::ContentFor {
                            name: slot_name,
                            body,
                            line: slot_line,
                        },
                        i + 1,
                    ));
                } else if let Some(rest) = code.strip_prefix("if ") {
                    let condition = parse_core_expr(rest.trim(), *line)?;
                    let (nested_if, consumed) = parse_if_block(&tokens[i..], condition, *line)?;
                    body.push(nested_if);
                    i += consumed;
                } else if let Some(rest) = code.strip_prefix("unless ") {
                    let (nested_unless, consumed) = parse_unless_block(&tokens[i..], rest, *line)?;
                    body.push(nested_unless);
                    i += consumed;
                } else if code.starts_with("for ") {
                    let (nested_for, consumed) = parse_for_block(&tokens[i..], *line)?;
                    body.push(nested_for);
                    i += consumed;
                } else if is_content_for_code(code) {
                    let (nested_cf, consumed) = parse_content_for_block(&tokens[i..], *line)?;
                    body.push(nested_cf);
                    i += consumed;
                } else if form_with_block_parts(code).is_some() {
                    let (nested_fw, consumed) = parse_form_with_block(&tokens[i..], *line)?;
                    body.push(nested_fw);
                    i += consumed;
                } else {
                    let stmts = parse_core_code(code, *line)?;
                    body.push(TemplateNode::CoreCodeBlock { stmts, line: *line });
                    i += 1;
                }
            }
            token => {
                body.push(parse_output_token(token)?);
                i += 1;
            }
        }
    }

    Err(format!(
        "Unclosed slot block at line {} - missing 'end'",
        slot_line
    ))
}

/// Whether a code-tag starts a `content_for` statement (well-formed or not).
/// Used by the block parsers to dispatch; `parse_content_for_block` reports
/// the friendly error when the trailing `do` is missing.
fn is_content_for_code(code: &str) -> bool {
    strip_directive_keyword(code, "content_for").is_some()
}

/// Extract the name-args portion of a `content_for ... do` open tag,
/// or `None` when the trailing `do` is missing.
fn content_for_block_open(code: &str) -> Option<&str> {
    let rest = strip_directive_keyword(code, "content_for")?;
    let inner = rest.trim_end().strip_suffix("do")?;
    // `do` must be its own word, not the tail of the name args.
    if !inner.is_empty() && !inner.ends_with(char::is_whitespace) {
        return None;
    }
    Some(inner.trim())
}

/// Parse a `<% content_for "name" do %> ... <% end %>` block starting at the
/// given position. Returns the ContentFor node and the tokens consumed.
/// Mirrors `parse_for_block`.
fn parse_content_for_block(
    tokens: &[Token],
    cf_line: usize,
) -> Result<(TemplateNode, usize), String> {
    enter_template_block(cf_line)?;
    let result = parse_content_for_block_inner(tokens, cf_line);
    exit_template_block();
    result
}

fn parse_content_for_block_inner(
    tokens: &[Token],
    cf_line: usize,
) -> Result<(TemplateNode, usize), String> {
    let name = match &tokens[0] {
        Token::Code(code, _line) => match content_for_block_open(code.trim()) {
            Some(args) => parse_directive_name(args, "content_for", cf_line)?,
            None => {
                return Err(format!(
                    "content_for requires a block: <% content_for \"name\" do %> ... <% end %> at line {}",
                    cf_line
                ))
            }
        },
        _ => {
            return Err(format!(
                "Expected 'content_for' statement at line {}",
                cf_line
            ))
        }
    };

    let mut body = Vec::new();
    let mut i = 1; // Skip the initial `content_for` token

    while i < tokens.len() {
        match &tokens[i] {
            Token::Code(code, line) => {
                let code = code.trim();

                if code == "end" {
                    return Ok((
                        TemplateNode::ContentFor {
                            name,
                            body,
                            line: cf_line,
                        },
                        i + 1,
                    ));
                } else if let Some(rest) = code.strip_prefix("if ") {
                    let condition = parse_core_expr(rest.trim(), *line)?;
                    let (nested_if, consumed) = parse_if_block(&tokens[i..], condition, *line)?;
                    body.push(nested_if);
                    i += consumed;
                } else if let Some(rest) = code.strip_prefix("unless ") {
                    let (nested_unless, consumed) = parse_unless_block(&tokens[i..], rest, *line)?;
                    body.push(nested_unless);
                    i += consumed;
                } else if code.starts_with("for ") {
                    let (nested_for, consumed) = parse_for_block(&tokens[i..], *line)?;
                    body.push(nested_for);
                    i += consumed;
                } else if is_content_for_code(code) {
                    let (nested_cf, consumed) = parse_content_for_block(&tokens[i..], *line)?;
                    body.push(nested_cf);
                    i += consumed;
                } else if form_with_block_parts(code).is_some() {
                    let (nested_fw, consumed) = parse_form_with_block(&tokens[i..], *line)?;
                    body.push(nested_fw);
                    i += consumed;
                } else {
                    let stmts = parse_core_code(code, *line)?;
                    body.push(TemplateNode::CoreCodeBlock { stmts, line: *line });
                    i += 1;
                }
            }
            token => {
                body.push(parse_output_token(token)?);
                i += 1;
            }
        }
    }

    Err(format!(
        "Unclosed content_for block at line {} - missing 'end'",
        cf_line
    ))
}

/// Convert a non-Code token (Literal, OutputEscaped, OutputRaw, OutputUnescape)
/// into the corresponding TemplateNode. Used to avoid duplicating output handling
/// across parse_tokens, parse_if_block, and parse_for_block.
fn parse_output_token(token: &Token) -> Result<TemplateNode, String> {
    match token {
        Token::Literal(s, _line) => Ok(TemplateNode::Literal(s.clone())),
        Token::OutputEscaped(expr, line) => {
            if let Some(directive) = parse_yield_directive(expr, *line) {
                directive
            } else if expr.starts_with("render ") && !expr.starts_with("render (") {
                // Rails-style DSL form: `render "foo"` / `render "foo", ctx`.
                // Paren-form `render(...)` is a regular function call —
                // `render` is a real builtin — and is handled by the core
                // parser below, which lexes hash keys like `"class"` correctly.
                parse_partial_call(expr, *line)
            } else {
                let core_expr = parse_core_expr(expr, *line)?;
                Ok(TemplateNode::CoreOutput {
                    expr: core_expr,
                    escaped: true,
                    line: *line,
                })
            }
        }
        Token::OutputRaw(expr, line) => {
            if let Some(directive) = parse_yield_directive(expr, *line) {
                directive
            } else {
                let core_expr = parse_core_expr(expr, *line)?;
                Ok(TemplateNode::CoreOutput {
                    expr: core_expr,
                    escaped: false,
                    line: *line,
                })
            }
        }
        Token::OutputUnescape(_expr, line) => {
            // SEC-023: `<%==` previously rewrote to `html_unescape(expr)` and
            // emitted with `escaped: false`. The combination "decode HTML
            // entities, then emit raw" is a silent XSS footgun — applied to
            // any value that round-tripped through the database or JSON, it
            // turns `&lt;script&gt;` back into `<script>`. The syntax has
            // been removed; `<%= html_unescape(expr) %>` (entity decode +
            // safe escape) and `<%- expr %>` (raw output) cover the two
            // legitimate use cases visibly.
            Err(format!(
                "<%== %> at line {} has been removed (SEC-023). Use `<%= html_unescape(expr) %>` for entity-decoded but escaped output, or `<%- expr %>` for raw HTML.",
                line
            ))
        }
        Token::Code(_, _) => unreachable!("Code tokens handled separately"),
    }
}

/// Parse a code block through the core language parser for full language support.
/// This handles `let` declarations, function calls, assignments, and all other statements.
fn parse_core_code(code: &str, line: usize) -> Result<Vec<crate::ast::stmt::Stmt>, String> {
    let tokens = crate::lexer::Scanner::new(code)
        .scan_tokens()
        .map_err(|e| format!("Syntax error at line {}: {}", line, e))?;
    let program = crate::parser::Parser::new(tokens)
        .parse()
        .map_err(|e| format!("Parse error at line {}: {}", line, e))?;
    Ok(program.statements)
}

/// Parse an expression through the core language parser.
/// Used for `<%= %>` output expressions to support the full language.
fn parse_core_expr(code: &str, line: usize) -> Result<crate::ast::expr::Expr, String> {
    let stmts = parse_core_code(code, line)?;
    match stmts.into_iter().next() {
        Some(stmt) => match stmt.kind {
            crate::ast::stmt::StmtKind::Expression(expr) => Ok(expr),
            _ => Err(format!("Expected expression at line {}", line)),
        },
        None => Err(format!("Empty expression at line {}", line)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A template with absurdly nested `<% if %>` blocks must produce a clean
    /// parse error, never a native stack overflow (which aborts without
    /// unwinding — beyond the reach of the server's fault isolation).
    #[test]
    fn deeply_nested_if_blocks_error_instead_of_overflowing() {
        let source = format!(
            "{}{}",
            "<% if true %>".repeat(100_000),
            "<% end %>".repeat(100_000)
        );
        let err = parse_template(&source).expect_err("must refuse absurd nesting");
        assert!(err.contains("too deeply"), "unexpected error: {err}");
    }

    #[test]
    fn moderately_nested_blocks_still_parse() {
        let source = format!(
            "a {}b{} c",
            "<% if true %>".repeat(40),
            "<% end %>".repeat(40)
        );
        parse_template(&source).expect("40-deep block nesting parses");
    }

    #[test]
    fn test_tokenize_simple() {
        let tokens = tokenize("Hello <%= name %>!").unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::Literal("Hello ".to_string(), 1),
                Token::OutputEscaped("name".to_string(), 1),
                Token::Literal("!".to_string(), 1),
            ]
        );
    }

    #[test]
    fn test_tokenize_raw_output() {
        let tokens = tokenize("<%- raw_html %>").unwrap();
        assert_eq!(tokens, vec![Token::OutputRaw("raw_html".to_string(), 1)]);
    }

    #[test]
    fn test_tokenize_unescape_output() {
        let tokens = tokenize("<%== encoded %>").unwrap();
        assert_eq!(
            tokens,
            vec![Token::OutputUnescape("encoded".to_string(), 1)]
        );
    }

    /// SEC-023: `<%== expr %>` is rejected at parse time with a migration
    /// hint pointing at the safer alternatives.
    #[test]
    fn test_parse_unescape_output_is_rejected() {
        let err = parse_template("<%== encoded %>").unwrap_err();
        assert!(
            err.contains("SEC-023") && err.contains("html_unescape"),
            "expected SEC-023 migration error, got: {}",
            err
        );
    }

    #[test]
    fn test_tokenize_code_block() {
        let tokens = tokenize("<% if true %>yes<% end %>").unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::Code("if true".to_string(), 1),
                Token::Literal("yes".to_string(), 1),
                Token::Code("end".to_string(), 1),
            ]
        );
    }

    #[test]
    fn test_parse_literal() {
        let nodes = parse_template("Hello World").unwrap();
        assert_eq!(
            nodes,
            vec![TemplateNode::Literal("Hello World".to_string())]
        );
    }

    #[test]
    fn test_parse_output() {
        let nodes = parse_template("<%= name %>").unwrap();
        assert_eq!(nodes.len(), 1);
        match &nodes[0] {
            TemplateNode::CoreOutput {
                expr,
                escaped,
                line,
            } => {
                assert!(
                    matches!(&expr.kind, crate::ast::expr::ExprKind::Variable(n) if n == "name")
                );
                assert!(escaped);
                assert_eq!(*line, 1);
            }
            _ => panic!("Expected CoreOutput node"),
        }
    }

    #[test]
    fn test_parse_if() {
        let nodes = parse_template("<% if show %>visible<% end %>").unwrap();
        assert_eq!(nodes.len(), 1);
        match &nodes[0] {
            TemplateNode::If {
                condition,
                body,
                else_body,
                line,
            } => {
                assert!(
                    matches!(&condition.kind, crate::ast::expr::ExprKind::Variable(n) if n == "show")
                );
                assert_eq!(body.len(), 1);
                assert!(matches!(&body[0], TemplateNode::Literal(s) if s == "visible"));
                assert!(else_body.is_none());
                assert_eq!(*line, 1);
            }
            _ => panic!("Expected If node"),
        }
    }

    #[test]
    fn test_parse_if_else() {
        let nodes = parse_template("<% if show %>yes<% else %>no<% end %>").unwrap();
        assert_eq!(nodes.len(), 1);
        match &nodes[0] {
            TemplateNode::If {
                body, else_body, ..
            } => {
                assert_eq!(body.len(), 1);
                assert!(matches!(&body[0], TemplateNode::Literal(s) if s == "yes"));
                let else_nodes = else_body.as_ref().unwrap();
                assert_eq!(else_nodes.len(), 1);
                assert!(matches!(&else_nodes[0], TemplateNode::Literal(s) if s == "no"));
            }
            _ => panic!("Expected If node"),
        }
    }

    /// The template, or a panic naming it. Unwrapping would do, but every
    /// unwrap and expect call under `src/template` is counted by
    /// `scripts/lint_unwraps.sh` — a ratchet that does not read `#[cfg(test)]`
    /// — so the tests below say it this way instead.
    fn parsed(src: &str) -> Vec<TemplateNode> {
        match parse_template(src) {
            Ok(nodes) => nodes,
            Err(e) => panic!("{src:?} did not parse: {e}"),
        }
    }

    /// The reason the template was refused, or a panic because it was not.
    fn refused(src: &str) -> String {
        match parse_template(src) {
            Ok(_) => panic!("{src:?} parsed, and should not have"),
            Err(e) => e,
        }
    }

    /// The `else` branch of an `If` node, or a panic.
    fn else_of(node: &TemplateNode) -> &[TemplateNode] {
        match node {
            TemplateNode::If {
                else_body: Some(nodes),
                ..
            } => nodes,
            other => panic!("expected an If with an else branch, got {other:?}"),
        }
    }

    /// `<% unless c %>` is an `If` on the negated condition: one node kind
    /// for the renderer, and `else` works the way it does after an `if`.
    #[test]
    fn test_parse_unless() {
        let nodes = parsed("<% unless hidden %>visible<% end %>");
        assert_eq!(nodes.len(), 1);
        match &nodes[0] {
            TemplateNode::If {
                condition,
                body,
                else_body,
                line,
            } => {
                match &condition.kind {
                    crate::ast::expr::ExprKind::Unary { operator, operand } => {
                        assert_eq!(*operator, crate::ast::expr::UnaryOp::Not);
                        assert!(
                            matches!(&operand.kind, crate::ast::expr::ExprKind::Variable(n) if n == "hidden")
                        );
                    }
                    other => panic!("expected the condition negated, got {other:?}"),
                }
                assert_eq!(body.len(), 1);
                assert!(matches!(&body[0], TemplateNode::Literal(s) if s == "visible"));
                assert!(else_body.is_none());
                assert_eq!(*line, 1);
            }
            _ => panic!("Expected If node"),
        }
    }

    #[test]
    fn test_parse_unless_else() {
        let nodes = parsed("<% unless hidden %>yes<% else %>no<% end %>");
        assert_eq!(nodes.len(), 1);
        match &nodes[0] {
            TemplateNode::If { body, .. } => {
                assert!(matches!(&body[0], TemplateNode::Literal(s) if s == "yes"));
                assert!(matches!(&else_of(&nodes[0])[0], TemplateNode::Literal(s) if s == "no"));
            }
            _ => panic!("Expected If node"),
        }
    }

    /// The whole point of the change: the tag used to fall through to the
    /// core parser as a complete, empty-bodied `unless`, and the error named
    /// the orphaned `<% end %>` two lines below instead.
    #[test]
    fn test_unless_body_is_not_swallowed() {
        let nodes = parsed("a\n<% unless hidden %>kept<% end %>\nb");
        let kept = nodes.iter().any(|n| {
            matches!(n, TemplateNode::If { body, .. }
                if matches!(body.first(), Some(TemplateNode::Literal(s)) if s == "kept"))
        });
        assert!(kept, "the unless body must be in the tree: {nodes:?}");
    }

    /// `unless` nests inside every block that already took an `if`.
    #[test]
    fn test_unless_nests_in_for_and_if() {
        let in_for = parsed("<% for x in xs %><% unless x %>y<% end %><% end %>");
        match &in_for[0] {
            TemplateNode::For { body, .. } => {
                assert!(matches!(&body[0], TemplateNode::If { .. }))
            }
            _ => panic!("Expected For node"),
        }
        let in_if = parsed("<% if a %><% unless b %>y<% end %><% end %>");
        match &in_if[0] {
            TemplateNode::If { body, .. } => {
                assert!(matches!(&body[0], TemplateNode::If { .. }))
            }
            _ => panic!("Expected If node"),
        }
        let in_else = parsed("<% if a %>x<% else %><% unless b %>y<% end %><% end %>");
        assert!(matches!(&else_of(&in_else[0])[0], TemplateNode::If { .. }));
    }

    /// As in the language itself: "unless A, else if B" is a puzzle, not a
    /// guard. The diagnostic says what to write instead.
    #[test]
    fn test_elsif_after_unless_is_refused() {
        let err = refused("<% unless a %>x<% elsif b %>y<% end %>");
        assert!(err.contains("'elsif' cannot follow 'unless'"), "got: {err}");
        // After an `if` it is still fine.
        assert!(parse_template("<% if a %>x<% elsif b %>y<% end %>").is_ok());
    }

    #[test]
    fn test_unclosed_unless_names_the_block_that_opened() {
        let err = refused("<% unless a %>x");
        assert!(err.contains("Unclosed unless block"), "got: {err}");
        let err = refused("<% if a %>x");
        assert!(err.contains("Unclosed if block"), "got: {err}");
    }

    /// A postfix `unless` in a code tag is a statement the core parser owns;
    /// it must not be mistaken for a block opener.
    #[test]
    fn test_postfix_unless_is_left_to_the_core_parser() {
        let nodes = parsed("<% x = 1 unless a %>");
        assert!(matches!(&nodes[0], TemplateNode::CoreCodeBlock { .. }));
    }

    /// SEC-023: `<%==` must also be rejected when wrapped in control flow.
    #[test]
    fn test_parse_unescape_in_if_is_rejected() {
        let err = parse_template("<% if show %><%== encoded %><% end %>").unwrap_err();
        assert!(
            err.contains("SEC-023"),
            "expected SEC-023 migration error, got: {}",
            err
        );
    }

    #[test]
    fn test_parse_for() {
        let nodes = parse_template("<% for item in items %><%= item %><% end %>").unwrap();
        assert_eq!(nodes.len(), 1);
        match &nodes[0] {
            TemplateNode::For {
                var,
                index_var,
                iterable,
                body,
                ..
            } => {
                assert_eq!(var, "item");
                assert!(index_var.is_none());
                assert!(
                    matches!(&iterable.kind, crate::ast::expr::ExprKind::Variable(n) if n == "items")
                );
                assert_eq!(body.len(), 1);
                assert!(matches!(
                    &body[0],
                    TemplateNode::CoreOutput { escaped: true, .. }
                ));
            }
            _ => panic!("Expected For node"),
        }
    }

    #[test]
    fn test_parse_for_with_index() {
        // Test parsing "for x, i in items"
        let nodes =
            parse_template("<% for item, i in items %><%= i %>: <%= item %><% end %>").unwrap();
        assert_eq!(nodes.len(), 1);
        match &nodes[0] {
            TemplateNode::For {
                var,
                index_var,
                iterable,
                body,
                ..
            } => {
                assert_eq!(var, "item");
                assert_eq!(index_var, &Some("i".to_string()));
                assert!(
                    matches!(&iterable.kind, crate::ast::expr::ExprKind::Variable(n) if n == "items")
                );
                assert_eq!(body.len(), 3);
            }
            _ => panic!("Expected For node"),
        }
    }

    #[test]
    fn test_parse_yield() {
        let nodes = parse_template("<%= yield %>").unwrap();
        assert_eq!(nodes, vec![TemplateNode::Yield(None)]);
    }

    #[test]
    fn test_parse_yield_named() {
        for src in [
            "<%= yield \"head\" %>",
            "<%= yield 'head' %>",
            "<%= yield(\"head\") %>",
            "<%- yield \"head\" %>",
        ] {
            let nodes = parse_template(src).unwrap();
            assert_eq!(
                nodes,
                vec![TemplateNode::Yield(Some("head".to_string()))],
                "source: {}",
                src
            );
        }
    }

    #[test]
    fn test_parse_content_for_read_form() {
        for src in [
            "<%= content_for \"head\" %>",
            "<%= content_for(\"head\") %>",
        ] {
            let nodes = parse_template(src).unwrap();
            assert_eq!(
                nodes,
                vec![TemplateNode::Yield(Some("head".to_string()))],
                "source: {}",
                src
            );
        }
    }

    #[test]
    fn test_parse_yield_non_literal_name_rejected() {
        let err = parse_template("<%= yield section %>").unwrap_err();
        assert!(
            err.contains("string literal") && err.contains("line 1"),
            "expected string-literal diagnostic, got: {}",
            err
        );
    }

    #[test]
    fn test_parse_content_for_block() {
        let nodes =
            parse_template("<% content_for \"head\" do %><script></script><% end %>").unwrap();
        assert_eq!(
            nodes,
            vec![TemplateNode::ContentFor {
                name: "head".to_string(),
                body: vec![TemplateNode::Literal("<script></script>".to_string())],
                line: 1,
            }]
        );
        // Paren form parses too.
        let nodes = parse_template("<% content_for(\"head\") do %>x<% end %>").unwrap();
        assert!(matches!(
            &nodes[0],
            TemplateNode::ContentFor { name, .. } if name == "head"
        ));
    }

    #[test]
    fn test_parse_content_for_nested_in_if_and_for() {
        let nodes = parse_template("<% if show %><% content_for \"head\" do %>a<% end %><% end %>")
            .unwrap();
        match &nodes[0] {
            TemplateNode::If { body, .. } => {
                assert!(matches!(&body[0], TemplateNode::ContentFor { name, .. } if name == "head"))
            }
            other => panic!("Expected If node, got {:?}", other),
        }

        let nodes = parse_template(
            "<% for item in items %><% content_for \"list\" do %><%= item %><% end %><% end %>",
        )
        .unwrap();
        match &nodes[0] {
            TemplateNode::For { body, .. } => {
                assert!(matches!(&body[0], TemplateNode::ContentFor { name, .. } if name == "list"))
            }
            other => panic!("Expected For node, got {:?}", other),
        }

        // And control flow nests inside a capture block.
        let nodes = parse_template(
            "<% content_for \"head\" do %><% if debug %><script></script><% end %><% end %>",
        )
        .unwrap();
        match &nodes[0] {
            TemplateNode::ContentFor { body, .. } => {
                assert!(matches!(&body[0], TemplateNode::If { .. }))
            }
            other => panic!("Expected ContentFor node, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_content_for_unclosed() {
        let err = parse_template("<% content_for \"head\" do %>never closed").unwrap_err();
        assert!(
            err.contains("Unclosed content_for") && err.contains("line 1"),
            "expected unclosed diagnostic, got: {}",
            err
        );
    }

    #[test]
    fn test_parse_content_for_non_literal_name_rejected() {
        let err = parse_template("<% content_for section do %>x<% end %>").unwrap_err();
        assert!(
            err.contains("string literal"),
            "expected string-literal diagnostic, got: {}",
            err
        );
    }

    #[test]
    fn test_parse_content_for_without_do_rejected() {
        let err = parse_template("<% content_for \"head\" %>x<% end %>").unwrap_err();
        assert!(
            err.contains("content_for requires a block"),
            "expected missing-do diagnostic, got: {}",
            err
        );
    }

    #[test]
    fn test_content_for_predicate_not_swallowed() {
        // `content_for?(...)` is the predicate builtin, not a directive —
        // it must reach the core parser as a normal escaped output call.
        let nodes = parse_template("<%= content_for?(\"head\") %>").unwrap();
        assert!(
            matches!(&nodes[0], TemplateNode::CoreOutput { escaped: true, .. }),
            "expected CoreOutput, got {:?}",
            nodes[0]
        );
    }

    #[test]
    fn test_extract_lintable_code_skips_content_for_directives() {
        let src = "<% content_for \"head\" do %>\n<script></script>\n<% end %>\n<%= yield \"head\" %>\n<%= content_for(\"other\") %>";
        let extracted = extract_lintable_code(src).unwrap();
        // The capture-open becomes `if true` so its `end` stays balanced,
        // and the yield/read-form directives are dropped entirely.
        assert!(extracted.contains("if true"));
        assert!(extracted.contains("end"));
        assert!(!extracted.contains("content_for"));
        assert!(!extracted.contains("yield"));
        // The synthesized source must be parseable Soli.
        let tokens = crate::lexer::Scanner::new(&extracted)
            .scan_tokens()
            .unwrap();
        crate::parser::Parser::new(tokens).parse().unwrap();
    }

    #[test]
    fn test_extract_lintable_code_handles_slot_builder() {
        // The `|c|` block var binds and `c.slot(...) do … end` is a real
        // method-call-with-block — the extracted source must parse so `soli lint`
        // runs cleanly instead of choking on the slot syntax.
        let src = "<%- component \"card\" do |c| %>\n<%- c.slot(\"header\") do %>\n<h1>Hi</h1>\n<% end %>\nBody\n<%- end %>";
        let extracted = extract_lintable_code(src).unwrap();
        assert!(extracted.contains("c.slot"));
        let tokens = crate::lexer::Scanner::new(&extracted)
            .scan_tokens()
            .unwrap();
        crate::parser::Parser::new(tokens).parse().unwrap();
    }

    #[test]
    fn test_parse_partial() {
        let nodes = parse_template("<%= render 'users/_card' %>").unwrap();
        assert_eq!(
            nodes,
            vec![TemplateNode::Partial {
                name: "users/_card".to_string(),
                context: None,
                line: 1,
            }]
        );
    }

    /// BUG-001: a reserved keyword used as the loop variable in
    /// `<% for KW in items %>` previously produced a cryptic
    /// "Unexpected token 'EOF', expected identifier at 1:3" coming from
    /// the core parser when the body referenced the variable. The
    /// template parser now rejects it up-front with a message that
    /// names the offending keyword and the template line.
    #[test]
    fn test_for_loop_keyword_var_rejected() {
        let err = parse_template("\n<% for fn in items %><%= fn %><% end %>").unwrap_err();
        assert!(
            err.contains("'fn'") && err.contains("reserved keyword"),
            "expected keyword diagnostic, got: {}",
            err
        );
        // The error should include the template line of the `for`,
        // not a synthetic span from the core parser.
        assert!(
            err.contains("line 2"),
            "expected template line in error, got: {}",
            err
        );
    }

    /// Same protection for the index variable: `<% for x, KW in items %>`.
    #[test]
    fn test_for_loop_keyword_index_var_rejected() {
        let err = parse_template("<% for x, class in items %><% end %>").unwrap_err();
        assert!(
            err.contains("'class'") && err.contains("reserved keyword"),
            "expected keyword diagnostic for index var, got: {}",
            err
        );
    }

    /// Sanity: legitimate non-keyword names still parse.
    #[test]
    fn test_for_loop_non_keyword_var_accepted() {
        assert!(parse_template("<% for func in items %><% end %>").is_ok());
        assert!(parse_template("<% for item, idx in items %><% end %>").is_ok());
    }

    #[test]
    fn test_parse_function_call() {
        let nodes = parse_template("<%= public_path(\"css/application.css\") %>").unwrap();
        assert_eq!(nodes.len(), 1);
        match &nodes[0] {
            TemplateNode::CoreOutput { expr, escaped, .. } => {
                assert!(matches!(
                    &expr.kind,
                    crate::ast::expr::ExprKind::Call { .. }
                ));
                assert!(escaped);
            }
            _ => panic!("Expected CoreOutput node"),
        }
    }

    #[test]
    fn test_tokenize_comment_only() {
        let tokens = tokenize("<%# single line comment %>").unwrap();
        assert_eq!(tokens, vec![]);
    }

    #[test]
    fn test_tokenize_comment_multiline() {
        let tokens = tokenize("<%# do\n    nothing\n    here %>").unwrap();
        assert_eq!(tokens, vec![]);
    }

    #[test]
    fn test_tokenize_comment_inline() {
        // <%# ... %> should be dropped; surrounding text becomes separate literals
        let tokens = tokenize("before<%# this is a comment %>after").unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::Literal("before".to_string(), 1),
                Token::Literal("after".to_string(), 1),
            ]
        );
    }

    #[test]
    fn test_parse_comment_produces_no_nodes() {
        let nodes = parse_template("<%# single line comment %>").unwrap();
        assert_eq!(nodes, vec![]);
    }

    #[test]
    fn test_parse_comment_multiline_produces_no_nodes() {
        let nodes = parse_template("<%# do\n    nothing\n    here %>").unwrap();
        assert_eq!(nodes, vec![]);
    }

    #[test]
    fn test_parse_comment_between_literals() {
        let nodes = parse_template("Hello<%# remove me %>World").unwrap();
        assert_eq!(
            nodes,
            vec![
                TemplateNode::Literal("Hello".to_string()),
                TemplateNode::Literal("World".to_string()),
            ]
        );
    }

    #[test]
    fn test_parse_comment_not_executed() {
        // Content inside <%# %> must not be parsed or executed as Soli code
        let nodes = parse_template("ok<%# raise(\"boom\") %>end").unwrap();
        assert!(nodes.iter().all(|n| matches!(n, TemplateNode::Literal(_))));
    }

    #[test]
    fn test_parse_component_block() {
        // Named-arg form: `title: "Hi"` must be folded into props (regression:
        // the Named arg used to be silently dropped).
        let nodes =
            parse_template("<%- component \"card\", title: \"Hi\" do %>body<% end %>").unwrap();
        let TemplateNode::Component { parts, .. } = &nodes[0] else {
            panic!("expected a Component node, got {:?}", nodes[0]);
        };
        let props = parts
            .props
            .as_ref()
            .expect("named args should become props");
        match &props.kind {
            crate::ast::expr::ExprKind::Hash(pairs) => {
                assert_eq!(pairs.len(), 1);
                assert!(matches!(
                    &pairs[0].0.kind,
                    crate::ast::expr::ExprKind::StringLiteral(k) if k == "title"
                ));
                assert!(matches!(
                    &pairs[0].1.kind,
                    crate::ast::expr::ExprKind::StringLiteral(v) if v == "Hi"
                ));
            }
            other => panic!("expected props to be a hash literal, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_component_block_positional_hash() {
        // Paren form: an explicit positional hash literal is used directly as
        // props. (The paren-less brace form isn't supported — `{` is not a
        // command-arg starter — so an explicit hash must use parentheses.)
        let nodes =
            parse_template("<%- component(\"card\", { \"title\": \"Hi\" }) do %>b<% end %>")
                .unwrap();
        let TemplateNode::Component { parts, .. } = &nodes[0] else {
            panic!("expected a Component node");
        };
        let props = parts
            .props
            .as_ref()
            .expect("positional hash should be props");
        assert!(matches!(&props.kind, crate::ast::expr::ExprKind::Hash(_)));
    }

    #[test]
    fn test_parse_component_block_multiple_named_args() {
        // Several named args fold into a multi-pair hash in order.
        let nodes =
            parse_template("<%- component \"card\", title: \"Hi\", size: \"lg\" do %>b<% end %>")
                .unwrap();
        let TemplateNode::Component { parts, .. } = &nodes[0] else {
            panic!("expected a Component node");
        };
        let props = parts
            .props
            .as_ref()
            .expect("named args should become props");
        match &props.kind {
            crate::ast::expr::ExprKind::Hash(pairs) => assert_eq!(pairs.len(), 2),
            other => panic!("expected a hash literal, got {:?}", other),
        }
    }
}
