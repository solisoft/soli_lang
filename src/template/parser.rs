//! ERB-style template parser for Soli.
//!
//! Parses templates with syntax like:
//! - `<%= expr %>` - HTML-escaped output
//! - `<%- expr %>` - Raw/unescaped output
//! - `<%== expr %>` - HTML-unescaped output (shorthand for `<%= html_unescape(expr) %>`)
//! - `<% code %>` - Control flow (if, for, end, else, elsif)
//! - `<%= yield %>` - Layout content insertion point

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

/// A node in the template AST.
#[derive(Debug, Clone, PartialEq)]
pub enum TemplateNode {
    /// Raw HTML/text content
    Literal(String),
    /// Output expression: `<%= expr %>` (escaped), `<%- expr %>` (raw), or `<%== expr %>` (unescape)
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
    /// Layout content insertion point
    Yield,
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
    Literal(String, usize),        // content, line
    OutputEscaped(String, usize),  // <%= ... %>, line
    OutputRaw(String, usize),      // <%- ... %>, line
    OutputUnescape(String, usize), // <%== ... %>, line (html_unescape)
    Code(String, usize),           // <% ... %>, line
}

/// Parse an ERB-style template into an AST.
pub fn parse_template(source: &str) -> Result<Vec<TemplateNode>, String> {
    let tokens = tokenize(source)?;
    parse_tokens(&tokens)
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

            // Check for output tag types: <%== (unescape), <%= (escaped), or <%- (raw)
            let is_output = chars.peek() == Some(&'=');
            let is_raw = chars.peek() == Some(&'-');
            let mut is_unescape = false;

            if is_output {
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

            let tag_content = tag_content.trim().to_string();

            if is_raw {
                tokens.push(Token::OutputRaw(tag_content, tag_line));
            } else if is_unescape {
                tokens.push(Token::OutputUnescape(tag_content, tag_line));
            } else if is_output {
                tokens.push(Token::OutputEscaped(tag_content, tag_line));
            } else {
                tokens.push(Token::Code(tag_content, tag_line));
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
            parse_for_statement(&code[4..])?
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

/// Parse a for statement like "item in items" or "(item, index in items)"
/// Supports: "x in items" or "x, i in items" where i is the index
fn parse_for_statement(
    s: &str,
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

        Ok((var, index_var, parse_core_expr(iterable_str, 0)?))
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

/// Convert a non-Code token (Literal, OutputEscaped, OutputRaw, OutputUnescape)
/// into the corresponding TemplateNode. Used to avoid duplicating output handling
/// across parse_tokens, parse_if_block, and parse_for_block.
fn parse_output_token(token: &Token) -> Result<TemplateNode, String> {
    match token {
        Token::Literal(s, _line) => Ok(TemplateNode::Literal(s.clone())),
        Token::OutputEscaped(expr, line) => {
            if expr == "yield" {
                Ok(TemplateNode::Yield)
            } else if expr.starts_with("render ") || expr.starts_with("render(") {
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
            if expr == "yield" {
                Ok(TemplateNode::Yield)
            } else {
                let core_expr = parse_core_expr(expr, *line)?;
                Ok(TemplateNode::CoreOutput {
                    expr: core_expr,
                    escaped: false,
                    line: *line,
                })
            }
        }
        Token::OutputUnescape(expr, line) => {
            let inner_expr = parse_core_expr(expr, *line)?;
            let call_expr = crate::ast::expr::Expr::new(
                crate::ast::expr::ExprKind::Call {
                    callee: Box::new(crate::ast::expr::Expr::new(
                        crate::ast::expr::ExprKind::Variable("html_unescape".to_string()),
                        crate::span::Span::default(),
                    )),
                    arguments: vec![crate::ast::expr::Argument::Positional(inner_expr)],
                },
                crate::span::Span::default(),
            );
            Ok(TemplateNode::CoreOutput {
                expr: call_expr,
                escaped: false,
                line: *line,
            })
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
            '+' if depth == 0 && i > 0 => {
                // Make sure it's not part of a number like 1e+10
                if prev_char != 'e' && prev_char != 'E' {
                    last_found = Some((i, BinaryOp::Add));
                }
            }
            '-' if depth == 0 && i > 0 => {
                // Make sure it's not a unary minus or part of a number
                if prev_char != 'e'
                    && prev_char != 'E'
                    && prev_char != '('
                    && prev_char != '['
                    && prev_char != ','
                {
                    last_found = Some((i, BinaryOp::Subtract));
                }
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
        | "sample" | "shuffle" | "take" | "drop" | "zip" => {
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

    #[test]
    fn test_parse_unescape_output() {
        let nodes = parse_template("<%== encoded %>").unwrap();
        assert_eq!(nodes.len(), 1);
        match &nodes[0] {
            TemplateNode::CoreOutput { expr, escaped, .. } => {
                // Should be a call to html_unescape wrapping the expression
                assert!(matches!(
                    &expr.kind,
                    crate::ast::expr::ExprKind::Call { .. }
                ));
                assert!(!escaped); // Should not escape the unescaped output
            }
            _ => panic!("Expected CoreOutput node"),
        }
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

    #[test]
    fn test_parse_unescape_in_if() {
        // Test <%== inside if block
        let nodes = parse_template("<% if show %><%== encoded %><% end %>").unwrap();
        assert_eq!(nodes.len(), 1);
        match &nodes[0] {
            TemplateNode::If { body, .. } => {
                assert_eq!(body.len(), 1);
                match &body[0] {
                    TemplateNode::CoreOutput { expr, escaped, .. } => {
                        assert!(matches!(
                            &expr.kind,
                            crate::ast::expr::ExprKind::Call { .. }
                        ));
                        assert!(!escaped);
                    }
                    _ => panic!("Expected CoreOutput node"),
                }
            }
            _ => panic!("Expected If node"),
        }
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
        assert_eq!(nodes, vec![TemplateNode::Yield]);
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
}
