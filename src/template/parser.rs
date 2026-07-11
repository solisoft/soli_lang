//! ERB-style template parser for Soli.
//!
//! Parses templates with syntax like:
//! - `<%= expr %>` - HTML-escaped output
//! - `<%- expr %>` - Raw/unescaped output
//! - `<% code %>` - Control flow (if, for, end, else, elsif)
//! - `<%= yield %>` - Layout content insertion point
//!
//! `<%== expr %>` was removed in SEC-023 — it decoded HTML entities and
//! emitted the result raw, which silently re-created `<script>` from
//! `&lt;script&gt;` whenever a value had been round-tripped through
//! escape-encoded storage. Templates that reach for it are rejected at
//! parse time with a migration hint.

/// Pre-compiled expression for fast evaluation.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// String literal: "hello"
    StringLit(String),
    /// Integer literal: 42
    IntLit(i64),
    /// Float literal: 3.14
    FloatLit(f64),
    /// Boolean literal: true/false
    BoolLit(bool),
    /// Null literal
    Null,
    /// Array literal: [1, 2, 3]
    ArrayLit(Vec<Expr>),
    /// Simple variable lookup: name
    Var(String),
    /// Field access: expr.field
    Field(Box<Expr>, String),
    /// Index access: expr[key]
    Index(Box<Expr>, Box<Expr>),
    /// Binary operation: expr op expr (for +, -, *, /)
    Binary(Box<Expr>, BinaryOp, Box<Expr>),
    /// Comparison: expr op expr
    Compare(Box<Expr>, CompareOp, Box<Expr>),
    /// Logical AND: expr && expr
    And(Box<Expr>, Box<Expr>),
    /// Logical OR: expr || expr
    Or(Box<Expr>, Box<Expr>),
    /// Logical NOT: !expr
    Not(Box<Expr>),
    /// Method call: expr.length (for built-in methods without args)
    Method(Box<Expr>, String),
    /// Method call with arguments: expr.join(",")
    MethodCall {
        base: Box<Expr>,
        method: String,
        args: Vec<Expr>,
    },
    /// Function call: name(arg1, arg2, ...)
    Call(String, Vec<Expr>),
    /// Assignment: name = value or let name = value
    Assign(String, Box<Expr>),
    /// Range: start..end
    Range(Box<Expr>, Box<Expr>),
}

impl Expr {
    pub fn to_source(&self) -> String {
        match self {
            Expr::StringLit(s) => format!("\"{}\"", s),
            Expr::IntLit(n) => n.to_string(),
            Expr::FloatLit(n) => n.to_string(),
            Expr::BoolLit(b) => b.to_string(),
            Expr::Null => "null".to_string(),
            Expr::ArrayLit(elements) => {
                let parts: Vec<String> = elements.iter().map(|e| e.to_source()).collect();
                format!("[{}]", parts.join(", "))
            }
            Expr::Var(name) => name.clone(),
            Expr::Field(base, field) => format!("{}.{}", base.to_source(), field),
            Expr::Index(base, key) => format!("{}[{}]", base.to_source(), key.to_source()),
            Expr::Binary(left, op, right) => {
                let op_str = match op {
                    BinaryOp::Add => "+",
                    BinaryOp::Subtract => "-",
                    BinaryOp::Multiply => "*",
                    BinaryOp::Divide => "/",
                    BinaryOp::Modulo => "%",
                };
                format!("({} {} {})", left.to_source(), op_str, right.to_source())
            }
            Expr::Compare(left, op, right) => {
                let op_str = match op {
                    CompareOp::Eq => "==",
                    CompareOp::Ne => "!=",
                    CompareOp::Lt => "<",
                    CompareOp::Le => "<=",
                    CompareOp::Gt => ">",
                    CompareOp::Ge => ">=",
                };
                format!("({} {} {})", left.to_source(), op_str, right.to_source())
            }
            Expr::And(left, right) => format!("({} && {})", left.to_source(), right.to_source()),
            Expr::Or(left, right) => format!("({} || {})", left.to_source(), right.to_source()),
            Expr::Not(inner) => format!("!{}", inner.to_source()),
            Expr::Method(base, method) => format!("{}.{}", base.to_source(), method),
            Expr::MethodCall { base, method, args } => {
                let arg_sources: Vec<String> = args.iter().map(|e| e.to_source()).collect();
                format!(
                    "{}.{}({})",
                    base.to_source(),
                    method,
                    arg_sources.join(", ")
                )
            }
            Expr::Call(name, args) => {
                let arg_sources: Vec<String> = args.iter().map(|e| e.to_source()).collect();
                format!("{}({})", name, arg_sources.join(", "))
            }
            Expr::Assign(name, value) => format!("{} = {}", name, value.to_source()),
            Expr::Range(start, end) => format!("{}..{}", start.to_source(), end.to_source()),
        }
    }
}

/// Binary operators for arithmetic and string operations
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinaryOp {
    Add,      // +
    Subtract, // -
    Multiply, // *
    Divide,   // /
    Modulo,   // %
}

/// Comparison operators
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CompareOp {
    Eq, // ==
    Ne, // !=
    Lt, // <
    Le, // <=
    Gt, // >
    Ge, // >=
}

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
}

/// A node in the template AST.
#[derive(Debug, Clone, PartialEq)]
pub enum TemplateNode {
    /// Raw HTML/text content
    Literal(String),
    /// Output expression: `<%= expr %>` (escaped) or `<%- expr %>` (raw).
    /// `<%== expr %>` was removed in SEC-023.
    Output {
        expr: Expr,
        escaped: bool,
        line: usize,
    },
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
    /// Code block to execute (for variable assignments, etc.)
    CodeBlock { expr: Expr, line: usize },
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

/// Detect `component "name", props do` or `component("name", props) do` openers.
/// Returns the head (the call part before "do").
fn component_block_parts(code: &str) -> Option<&str> {
    let code = code.trim();
    if let Some(rest) = code.strip_suffix("do") {
        let head = rest.trim_end();
        if head.starts_with("component") {
            return Some(head);
        }
    }
    None
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
            {
                // `form_with(...) do |f|` and `component ... do` block openers and their `end` read
                // naturally as output tags (`<%- %>` / `<%= %>`, Rails-style).
                // Normalize them to Code tokens so the block dispatcher sees
                // them regardless of tag style.
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
        "Unclosed if block at line {} - missing 'end'",
        if_line
    ))
}

/// Parse a for block starting at the given position.
/// Returns the ForNode and the number of tokens consumed.
fn parse_for_block(tokens: &[Token], for_line: usize) -> Result<(TemplateNode, usize), String> {
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
    } else if let Some(rest) = strip_directive_keyword(expr, "content_for") {
        (rest, "content_for")
    } else {
        return None;
    };
    Some(parse_directive_name(args, directive, line).map(|name| TemplateNode::Yield(Some(name))))
}

/// Parse a `<% form_with(record) do |f| %> ... <% end %>` block starting at
/// the given position. Returns the FormWith node and the tokens consumed.
fn parse_form_with_block(
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
    let head = match &tokens[0] {
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

/// Compile an expression string into a pre-compiled Expr AST.
pub fn compile_expr(expr: &str) -> Expr {
    let expr = expr.trim();

    // Check for string literals
    if (expr.starts_with('"') && expr.ends_with('"'))
        || (expr.starts_with('\'') && expr.ends_with('\''))
    {
        return Expr::StringLit(expr[1..expr.len() - 1].to_string());
    }

    // Check for integer literals
    if let Ok(n) = expr.parse::<i64>() {
        return Expr::IntLit(n);
    }

    // Check for float literals
    if let Ok(n) = expr.parse::<f64>() {
        return Expr::FloatLit(n);
    }

    // Check for boolean literals
    if expr == "true" {
        return Expr::BoolLit(true);
    }
    if expr == "false" {
        return Expr::BoolLit(false);
    }
    if expr == "null" {
        return Expr::Null;
    }

    // Check for array literals: [1, 2, 3]
    if expr.starts_with('[') && expr.ends_with(']') {
        let inner = &expr[1..expr.len() - 1];
        if inner.trim().is_empty() {
            return Expr::ArrayLit(Vec::new());
        }
        let elements = parse_function_args(inner);
        return Expr::ArrayLit(elements);
    }

    // Check for assignment: name = value or let name = value
    if let Some(pos) = find_binary_op(expr, " = ") {
        // Make sure it's not a comparison (==, !=, <=, >=)
        let op_char = expr.chars().nth(pos - 1);
        if op_char != Some('=')
            && op_char != Some('!')
            && op_char != Some('<')
            && op_char != Some('>')
        {
            let name = expr[..pos].trim();
            // Check for valid variable name
            if name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                let value_expr = compile_expr(expr[pos + 3..].trim());
                return Expr::Assign(name.to_string(), Box::new(value_expr));
            }
        }
    }

    // Check for logical operators (lower precedence than comparison)
    // Process these first because they have lower precedence
    if let Some(pos) = find_logical_op(expr, " && ") {
        let left = compile_expr(&expr[..pos]);
        let right = compile_expr(&expr[pos + 4..]);
        return Expr::And(Box::new(left), Box::new(right));
    }
    if let Some(pos) = find_logical_op(expr, " || ") {
        let left = compile_expr(&expr[..pos]);
        let right = compile_expr(&expr[pos + 4..]);
        return Expr::Or(Box::new(left), Box::new(right));
    }

    // Check for range operator: start..end
    if let Some(pos) = find_binary_op(expr, "..") {
        let left = compile_expr(&expr[..pos]);
        let right = compile_expr(&expr[pos + 2..]);
        return Expr::Range(Box::new(left), Box::new(right));
    }

    // Check for comparison operators
    for (op_str, op) in [
        ("==", CompareOp::Eq),
        ("!=", CompareOp::Ne),
        (">=", CompareOp::Ge),
        ("<=", CompareOp::Le),
        (">", CompareOp::Gt),
        ("<", CompareOp::Lt),
    ] {
        if let Some(pos) = find_binary_op(expr, op_str) {
            let left = compile_expr(&expr[..pos]);
            let right = compile_expr(&expr[pos + op_str.len()..]);
            return Expr::Compare(Box::new(left), op, Box::new(right));
        }
    }

    // Check for additive operators (+ -)
    // Scan right-to-left for left associativity
    if let Some((pos, op)) = find_additive_op(expr) {
        let left = compile_expr(&expr[..pos]);
        let right = compile_expr(&expr[pos + 1..]);
        return Expr::Binary(Box::new(left), op, Box::new(right));
    }

    // Check for multiplicative operators (* / %)
    if let Some((pos, op)) = find_multiplicative_op(expr) {
        let left = compile_expr(&expr[..pos]);
        let right = compile_expr(&expr[pos + 1..]);
        return Expr::Binary(Box::new(left), op, Box::new(right));
    }

    // Check for negation
    if let Some(rest) = expr.strip_prefix('!') {
        let inner = compile_expr(rest);
        return Expr::Not(Box::new(inner));
    }

    // Check for function calls like "name(arg1, arg2)"
    if let Some(paren_pos) = expr.find('(') {
        let name = &expr[..paren_pos];
        // Check if this looks like a function name (alphanumeric with underscores)
        if name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            let args_str = &expr[paren_pos..];
            if let Some(close_paren) = find_matching_bracket_compile(args_str) {
                let args_content = &args_str[1..close_paren];
                let args = if args_content.trim().is_empty() {
                    Vec::new()
                } else {
                    parse_function_args(args_content)
                };
                return Expr::Call(name.to_string(), args);
            }
        }
    }

    // Parse variable access with optional chained lookups
    compile_variable_access(expr)
}

/// Find a logical operator position, respecting bracket/quote nesting
fn find_logical_op(expr: &str, op: &str) -> Option<usize> {
    let mut depth = 0;
    let mut in_string = false;
    let mut string_char = ' ';
    let mut prev_char = ' ';

    for (i, c) in expr.char_indices() {
        if in_string {
            if c == string_char && prev_char != '\\' {
                in_string = false;
            }
            prev_char = c;
            continue;
        }

        match c {
            '"' | '\'' => {
                in_string = true;
                string_char = c;
            }
            '[' | '(' => depth += 1,
            ']' | ')' => depth -= 1,
            _ => {
                if depth == 0 && expr[i..].starts_with(op) {
                    return Some(i);
                }
            }
        }
        prev_char = c;
    }
    None
}

/// Find a binary operator position, respecting bracket/quote nesting
fn find_binary_op(expr: &str, op: &str) -> Option<usize> {
    let mut depth = 0;
    let mut in_string = false;
    let mut string_char = ' ';
    let mut prev_char = ' ';

    for (i, c) in expr.char_indices() {
        if in_string {
            if c == string_char && prev_char != '\\' {
                in_string = false;
            }
            prev_char = c;
            continue;
        }

        match c {
            '"' | '\'' => {
                in_string = true;
                string_char = c;
            }
            '[' | '(' => depth += 1,
            ']' | ')' => depth -= 1,
            _ => {
                if depth == 0 && expr[i..].starts_with(op) {
                    return Some(i);
                }
            }
        }
        prev_char = c;
    }
    None
}

/// Find additive operator (+ or -), scanning right-to-left for left associativity
/// Returns position and operator type
fn find_additive_op(expr: &str) -> Option<(usize, BinaryOp)> {
    let mut depth = 0;
    let mut in_string = false;
    let mut string_char = ' ';
    let mut prev_char = ' ';
    let mut last_found: Option<(usize, BinaryOp)> = None;

    for (i, c) in expr.char_indices() {
        if in_string {
            if c == string_char && prev_char != '\\' {
                in_string = false;
            }
            prev_char = c;
            continue;
        }

        match c {
            '"' | '\'' => {
                in_string = true;
                string_char = c;
            }
            '[' | '(' => depth += 1,
            ']' | ')' => depth -= 1,
            // `+` but not part of a number like `1e+10`
            '+' if depth == 0 && i > 0 && prev_char != 'e' && prev_char != 'E' => {
                last_found = Some((i, BinaryOp::Add));
            }
            // `-` but not unary minus and not part of a number
            '-' if depth == 0
                && i > 0
                && prev_char != 'e'
                && prev_char != 'E'
                && prev_char != '('
                && prev_char != '['
                && prev_char != ',' =>
            {
                last_found = Some((i, BinaryOp::Subtract));
            }
            _ => {}
        }
        prev_char = c;
    }
    last_found
}

/// Find multiplicative operator (* / %), scanning right-to-left for left associativity
fn find_multiplicative_op(expr: &str) -> Option<(usize, BinaryOp)> {
    let mut depth = 0;
    let mut in_string = false;
    let mut string_char = ' ';
    let mut prev_char = ' ';
    let mut last_found: Option<(usize, BinaryOp)> = None;

    for (i, c) in expr.char_indices() {
        if in_string {
            if c == string_char && prev_char != '\\' {
                in_string = false;
            }
            prev_char = c;
            continue;
        }

        match c {
            '"' | '\'' => {
                in_string = true;
                string_char = c;
            }
            '[' | '(' => depth += 1,
            ']' | ')' => depth -= 1,
            '*' if depth == 0 => {
                last_found = Some((i, BinaryOp::Multiply));
            }
            '/' if depth == 0 => {
                last_found = Some((i, BinaryOp::Divide));
            }
            '%' if depth == 0 => {
                last_found = Some((i, BinaryOp::Modulo));
            }
            _ => {}
        }
        prev_char = c;
    }
    last_found
}

/// Compile variable access like `user`, `user["name"]`, or `user.name`
fn compile_variable_access(expr: &str) -> Expr {
    let expr = expr.trim();

    // Handle bracket notation first
    if let Some(bracket_pos) = find_first_bracket(expr) {
        let base = &expr[..bracket_pos];
        let rest = &expr[bracket_pos..];

        // Find closing bracket
        if let Some(close_pos) = find_matching_bracket_compile(rest) {
            let key_expr = &rest[1..close_pos];
            let after_bracket = &rest[close_pos + 1..];

            // Compile the base
            let base_expr = if base.is_empty() {
                // Direct bracket on a previous expression - shouldn't happen at top level
                return Expr::Var(expr.to_string());
            } else {
                compile_variable_access(base)
            };

            // Compile the key
            let key = compile_expr(key_expr);

            // Build the index expression
            let indexed = Expr::Index(Box::new(base_expr), Box::new(key));

            // Handle any further access
            if after_bracket.is_empty() {
                return indexed;
            } else if let Some(rest) = after_bracket.strip_prefix('.') {
                return compile_chained_access(indexed, rest);
            } else if after_bracket.starts_with('[') {
                return compile_further_brackets(indexed, after_bracket);
            }
        }
    }

    // Handle dot notation
    if let Some(dot_pos) = expr.find('.') {
        let base = &expr[..dot_pos];
        let field = &expr[dot_pos + 1..];

        let base_expr = Expr::Var(base.to_string());
        return compile_chained_access(base_expr, field);
    }

    // Simple variable
    Expr::Var(expr.to_string())
}

/// Find the first bracket that's not inside quotes
fn find_first_bracket(expr: &str) -> Option<usize> {
    let mut in_string = false;
    let mut string_char = ' ';

    // Use char_indices() to get byte positions for safe UTF-8 slicing
    for (i, c) in expr.char_indices() {
        if in_string {
            if c == string_char {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' | '\'' => {
                in_string = true;
                string_char = c;
            }
            '[' => return Some(i),
            _ => {}
        }
    }
    None
}

/// Find matching closing bracket for compile-time parsing
fn find_matching_bracket_compile(s: &str) -> Option<usize> {
    let mut depth = 0;
    let mut in_string = false;
    let mut string_char = ' ';

    // Use char_indices() to get byte positions for safe UTF-8 slicing
    for (i, c) in s.char_indices() {
        if in_string {
            if c == string_char {
                in_string = false;
            }
            continue;
        }

        match c {
            '"' | '\'' => {
                in_string = true;
                string_char = c;
            }
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Compile chained field/method access after a dot
fn compile_chained_access(base: Expr, field: &str) -> Expr {
    // Check if it's a method call with arguments: method_name(args)
    // This needs to be checked BEFORE bracket access because method calls use ()
    if let Some(paren_pos) = field.find('(') {
        let method_name = &field[..paren_pos];
        let rest = &field[paren_pos..];

        if let Some(close_pos) = find_matching_bracket_compile(rest) {
            let args_content = &rest[1..close_pos];
            let after = &rest[close_pos + 1..];

            // Check if it's a known method that needs function argument or can work directly
            let known_methods = [
                "length",
                "len",
                "size",
                "first",
                "last",
                "reverse",
                "join",
                "empty",
                "is_empty",
                "sum",
                "min",
                "max",
                "map",
                "filter",
                "each",
                "reduce",
                "find",
                "any?",
                "all?",
                "include?",
                "sort",
                "sort_by",
                "uniq",
                "compact",
                "flatten",
                "sample",
                "shuffle",
                "take",
                "drop",
                "zip",
                // String methods
                "uppercase",
                "upcase",
                "lowercase",
                "downcase",
                "trim",
                "capitalize",
                "replace",
                "split",
                "includes",
                "contains",
                "starts_with",
                "ends_with",
            ];

            if known_methods.contains(&method_name) {
                let args = if args_content.trim().is_empty() {
                    vec![]
                } else {
                    parse_function_args(args_content)
                };

                let method_expr = Expr::MethodCall {
                    base: Box::new(base.clone()),
                    method: method_name.to_string(),
                    args,
                };

                // Handle rest of chain
                if after.is_empty() {
                    return method_expr;
                } else if let Some(rest_field) = after.strip_prefix('.') {
                    return compile_chained_access(method_expr, rest_field);
                }
            }
        }
    }

    // Check for method-like properties (no arguments)
    let (current_field, rest) = if let Some(dot_pos) = field.find('.') {
        (&field[..dot_pos], Some(&field[dot_pos + 1..]))
    } else if let Some(bracket_pos) = find_first_bracket(field) {
        (&field[..bracket_pos], Some(&field[bracket_pos..]))
    } else {
        (field, None)
    };

    // Handle special methods - these become Expr::Method for the renderer
    let current = match current_field {
        "length" | "len" | "size" | "first" | "last" | "reverse" | "join" | "empty"
        | "is_empty" | "sum" | "min" | "max" | "map" | "filter" | "each" | "reduce" | "find"
        | "any?" | "all?" | "include?" | "sort" | "sort_by" | "uniq" | "compact" | "flatten"
        | "sample" | "shuffle" | "take" | "drop" | "slice" | "zip" => {
            Expr::Method(Box::new(base), current_field.to_string())
        }
        _ => Expr::Field(Box::new(base), current_field.to_string()),
    };

    // Handle rest of the chain
    match rest {
        Some(r) if r.starts_with('[') => compile_further_brackets(current, r),
        Some(r) => compile_chained_access(current, r),
        None => current,
    }
}

/// Compile further bracket access
fn compile_further_brackets(base: Expr, brackets: &str) -> Expr {
    if !brackets.starts_with('[') {
        return base;
    }

    if let Some(close_pos) = find_matching_bracket_compile(brackets) {
        let key_expr = &brackets[1..close_pos];
        let after = &brackets[close_pos + 1..];

        let key = compile_expr(key_expr);
        let indexed = Expr::Index(Box::new(base), Box::new(key));

        if after.is_empty() {
            indexed
        } else if let Some(rest) = after.strip_prefix('.') {
            compile_chained_access(indexed, rest)
        } else if after.starts_with('[') {
            compile_further_brackets(indexed, after)
        } else {
            indexed
        }
    } else {
        base
    }
}

/// Parse function arguments separated by commas
fn parse_function_args(args_str: &str) -> Vec<Expr> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut depth = 0;
    let mut in_string = false;
    let mut string_char = ' ';

    for c in args_str.chars() {
        if in_string {
            if c == string_char && !current.ends_with('\\') {
                in_string = false;
            }
            current.push(c);
        } else {
            match c {
                '"' | '\'' => {
                    in_string = true;
                    string_char = c;
                    current.push(c);
                }
                '(' | '[' => {
                    depth += 1;
                    current.push(c);
                }
                ')' | ']' => {
                    depth -= 1;
                    current.push(c);
                }
                ',' if depth == 0 => {
                    if !current.trim().is_empty() {
                        args.push(compile_expr(current.trim()));
                    }
                    current.clear();
                }
                _ => {
                    current.push(c);
                }
            }
        }
    }

    // Don't forget the last argument
    if !current.trim().is_empty() {
        args.push(compile_expr(current.trim()));
    }

    args
}

#[cfg(test)]
mod tests {
    use super::*;

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
