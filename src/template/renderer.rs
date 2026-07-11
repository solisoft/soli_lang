//! Template renderer that executes template AST with a data context.
//!
//! Uses a single interpreter per render call for optimal performance.
//! Writes directly into a shared output buffer (no intermediate String allocations).

use std::borrow::Cow;
use std::cell::RefCell;
use std::rc::Rc;

use crate::interpreter::executor::Interpreter;
use crate::interpreter::value::{HashKey, HashPairs, Value};
use crate::span::Span;
use crate::template::core_eval;
use crate::template::parser::{Expr, TemplateNode};

/// Type alias for partial renderer callbacks to reduce type complexity.
pub type PartialRenderer<'a> = Option<&'a dyn Fn(&str, &Value) -> Result<String, String>>;

/// Render a template AST with the given data context.
pub fn render_nodes(
    nodes: &[TemplateNode],
    data: &Value,
    partial_renderer: PartialRenderer<'_>,
) -> Result<String, String> {
    render_nodes_with_path(nodes, data, partial_renderer, None)
}

/// Render a template AST with the given data context and template path for error reporting.
pub fn render_nodes_with_path(
    nodes: &[TemplateNode],
    data: &Value,
    partial_renderer: PartialRenderer<'_>,
    template_path: Option<&str>,
) -> Result<String, String> {
    let mut interpreter = core_eval::create_template_interpreter(data);
    let mut output = String::with_capacity(2048);
    render_inner(
        &mut interpreter,
        nodes,
        data,
        partial_renderer,
        template_path,
        &mut output,
    )?;
    Ok(output)
}

/// Render a template AST with an existing interpreter (avoids creating a new one).
/// Used to share a single interpreter across view + layout rendering.
pub fn render_with_interpreter(
    interpreter: &mut Interpreter,
    nodes: &[TemplateNode],
    data: &Value,
    partial_renderer: PartialRenderer<'_>,
    template_path: Option<&str>,
) -> Result<String, String> {
    let mut output = String::with_capacity(4096);
    render_inner(
        interpreter,
        nodes,
        data,
        partial_renderer,
        template_path,
        &mut output,
    )?;
    Ok(output)
}

/// Internal render function that writes directly into the output buffer.
/// Reuses a single interpreter and avoids intermediate String allocations.
fn render_inner(
    interpreter: &mut Interpreter,
    nodes: &[TemplateNode],
    data: &Value,
    partial_renderer: PartialRenderer<'_>,
    template_path: Option<&str>,
    output: &mut String,
) -> Result<(), String> {
    for node in nodes {
        let node_line = match node {
            TemplateNode::Output { line, .. } => Some(*line),
            TemplateNode::If { line, .. } => Some(*line),
            TemplateNode::For { line, .. } => Some(*line),
            TemplateNode::Partial { line, .. } => Some(*line),
            TemplateNode::CodeBlock { line, .. } => Some(*line),
            TemplateNode::CoreCodeBlock { line, .. } => Some(*line),
            TemplateNode::CoreOutput { line, .. } => Some(*line),
            TemplateNode::ContentFor { line, .. } => Some(*line),
            TemplateNode::FormWith { line, .. } => Some(*line),
            TemplateNode::Component { line, .. } => Some(*line),
            _ => None,
        };

        let result: Result<(), String> = (|| {
            match node {
                TemplateNode::Literal(s) => {
                    output.push_str(s);
                }
                TemplateNode::Output {
                    expr,
                    escaped,
                    line: _,
                } => {
                    let value = core_eval::evaluate_with_interpreter(expr, interpreter)?;
                    write_value_to_output(&value, *escaped, output);
                }
                TemplateNode::If {
                    condition,
                    body,
                    else_body,
                    line: _,
                } => {
                    let cond_value = interpreter
                        .evaluate(condition)
                        .map_err(|e| format!("Evaluation error: {}", e))?;
                    // Auto-call methods so `<% if items.empty? %>` works without parens
                    let cond_value = auto_call_if_callable(interpreter, cond_value)?;
                    if is_truthy(&cond_value) {
                        render_inner(
                            interpreter,
                            body,
                            data,
                            partial_renderer,
                            template_path,
                            output,
                        )?;
                    } else if let Some(else_nodes) = else_body {
                        render_inner(
                            interpreter,
                            else_nodes,
                            data,
                            partial_renderer,
                            template_path,
                            output,
                        )?;
                    }
                }
                TemplateNode::For {
                    var,
                    index_var,
                    iterable,
                    body,
                    line: _,
                } => {
                    let iterable_value = interpreter
                        .evaluate(iterable)
                        .map_err(|e| format!("Evaluation error: {}", e))?;
                    match &iterable_value {
                        Value::Array(arr) => {
                            core_eval::push_scope(interpreter);
                            for (i, item) in arr.borrow().iter().enumerate() {
                                core_eval::define_var(interpreter, var, item.clone());
                                if let Some(idx_var) = index_var {
                                    core_eval::define_var(
                                        interpreter,
                                        idx_var,
                                        Value::Int(i as i64),
                                    );
                                }
                                render_inner(
                                    interpreter,
                                    body,
                                    data,
                                    partial_renderer,
                                    template_path,
                                    output,
                                )?;
                            }
                            core_eval::pop_scope(interpreter);
                        }
                        Value::Hash(hash) => {
                            core_eval::push_scope(interpreter);
                            for (k, v) in hash.borrow().iter() {
                                let pair = Value::Array(Rc::new(RefCell::new(vec![
                                    k.to_value(),
                                    v.clone(),
                                ])));
                                core_eval::define_var(interpreter, var, pair);
                                render_inner(
                                    interpreter,
                                    body,
                                    data,
                                    partial_renderer,
                                    template_path,
                                    output,
                                )?;
                            }
                            core_eval::pop_scope(interpreter);
                        }
                        _ => {
                            return Err(format!(
                                "Cannot iterate over {}: expected Array or Hash",
                                iterable_value.type_name()
                            ));
                        }
                    }
                }
                TemplateNode::Yield(name) => {
                    if let Some(n) = name {
                        // Support named yields from content_for (e.g. for component named slots or layouts)
                        if let Some(c) = crate::template::content_store::get(n) {
                            output.push_str(&c);
                            return Ok(());
                        }
                    } else {
                        // Support default slot for components via "content" or yield
                        if let Value::Hash(h) = data {
                            if let Some(c) = h.borrow().get(&HashKey::String("content".into())) {
                                write_value_to_output(c, false, output);
                                return Ok(());
                            }
                        }
                    }
                    return Err("yield encountered outside of layout context".to_string());
                }
                TemplateNode::ContentFor { name, body, .. } => {
                    // Capture into the content_for store, not the page output.
                    // Same recursion as the main body: locals/loop vars stay in
                    // scope and interpolations are escaped once, at capture time.
                    let mut captured = String::with_capacity(256);
                    render_inner(
                        interpreter,
                        body,
                        data,
                        partial_renderer,
                        template_path,
                        &mut captured,
                    )?;
                    crate::template::content_store::append(name, &captured);
                }
                TemplateNode::FormWith {
                    parts,
                    body,
                    line: _,
                } => {
                    // Bind the builder in a child scope, then wrap the body
                    // in its open()/close() output (raw — the builder emits
                    // HTML and escapes field values itself).
                    core_eval::push_scope(interpreter);
                    let builder = interpreter
                        .evaluate(&parts.builder_expr)
                        .map_err(|e| format!("Evaluation error: {}", e))?;
                    core_eval::define_var(interpreter, &parts.var, builder);
                    let open_html = interpreter
                        .evaluate(&parts.open_expr)
                        .map_err(|e| format!("Evaluation error: {}", e))?;
                    write_value_to_output(&open_html, false, output);
                    render_inner(
                        interpreter,
                        body,
                        data,
                        partial_renderer,
                        template_path,
                        output,
                    )?;
                    let close_html = interpreter
                        .evaluate(&parts.close_expr)
                        .map_err(|e| format!("Evaluation error: {}", e))?;
                    write_value_to_output(&close_html, false, output);
                    core_eval::pop_scope(interpreter);
                }
                TemplateNode::Component { parts, body, .. } => {
                    // Evaluate name expr to get the component name (string)
                    let name_val = interpreter
                        .evaluate(&parts.name)
                        .map_err(|e| format!("Evaluation error in component name: {}", e))?;
                    let comp_name = match &name_val {
                        Value::String(s) => s.to_string(),
                        other => {
                            return Err(format!(
                                "component name must evaluate to string, got {}",
                                other.type_name()
                            ))
                        }
                    };

                    // Render the block body into captured default slot content
                    let mut captured = String::new();
                    render_inner(
                        interpreter,
                        body,
                        data,
                        partial_renderer,
                        template_path,
                        &mut captured,
                    )?;

                    // Build props data + content for default slot
                    let mut comp_map: HashPairs = HashPairs::default();
                    if let Some(props_expr) = &parts.props {
                        if let Ok(Value::Hash(h)) = interpreter.evaluate(props_expr) {
                            for (k, v) in h.borrow().iter() {
                                comp_map.insert(k.clone(), v.clone());
                            }
                        }
                    }
                    comp_map.insert(
                        HashKey::String("content".into()),
                        Value::String(captured.into()),
                    );
                    let comp_data = Value::Hash(Rc::new(RefCell::new(comp_map)));

                    // Resolve like the component() helper: a `/`- or `.`-bearing
                    // name is app/views-relative and verbatim; a bare name
                    // resolves under components/. render_partial keeps components/
                    // paths clean (no `_`) and tags them as components in the dev bar.
                    let comp_path = if comp_name.contains('/') || comp_name.contains('.') {
                        comp_name.clone()
                    } else {
                        format!("components/{}", comp_name)
                    };
                    if let Some(r) = partial_renderer {
                        output.push_str(&r(&comp_path, &comp_data)?);
                    } else {
                        return Err(format!(
                            "Component rendering not available for '{}'",
                            comp_name
                        ));
                    }
                }
                TemplateNode::Partial {
                    name,
                    context,
                    line: _,
                } => {
                    if let Some(renderer) = partial_renderer {
                        let partial_data = if let Some(ctx_expr) = context {
                            match interpreter.evaluate(ctx_expr) {
                                Ok(v) => v,
                                Err(e) => {
                                    let msg = e.to_string();
                                    if msg.contains("Undefined variable") {
                                        Value::Null
                                    } else {
                                        return Err(format!("Evaluation error: {}", msg));
                                    }
                                }
                            }
                        } else {
                            data.clone()
                        };
                        output.push_str(&renderer(name, &partial_data)?);
                    } else {
                        return Err(format!("Partial rendering not available for '{}'", name));
                    }
                }
                TemplateNode::CodeBlock { expr, line: _ } => match expr {
                    Expr::Assign(name, value_expr) => {
                        let value = core_eval::evaluate_with_interpreter(value_expr, interpreter)?;
                        core_eval::define_var(interpreter, name, value);
                    }
                    _ => {
                        core_eval::evaluate_with_interpreter(expr, interpreter)?;
                    }
                },
                TemplateNode::CoreCodeBlock { stmts, line: _ } => {
                    for stmt in stmts {
                        interpreter
                            .execute(stmt)
                            .map_err(|e| format!("Evaluation error: {}", e))?;
                    }
                }
                TemplateNode::CoreOutput {
                    expr,
                    escaped,
                    line: _,
                } => {
                    let value = match interpreter.evaluate(expr) {
                        Ok(v) => v,
                        Err(e) => {
                            let msg = e.to_string();
                            if msg.contains("Undefined variable") {
                                Value::Null
                            } else {
                                return Err(format!("Evaluation error: {}", msg));
                            }
                        }
                    };
                    // Auto-call methods/functions with no args (parentheses optional in templates)
                    let value = auto_call_if_callable(interpreter, value)?;
                    write_value_to_output(&value, *escaped, output);
                }
            }
            Ok(())
        })();

        if let Err(e) = result {
            if let Some(path) = template_path {
                if !e.contains(".html.slv")
                    && !e.contains(".slv")
                    && !e.contains(".html.erb")
                    && !e.contains(".erb")
                {
                    if let Some(line) = node_line {
                        return Err(format!("{} at {}:{}", e, path, line));
                    }
                    return Err(format!("{} in {}", e, path));
                }
            }
            return Err(e);
        }
    }

    Ok(())
}

/// Write a Value directly to the output buffer, applying HTML escaping if needed.
/// Avoids intermediate String allocations for Int/Float/Bool (which can't contain HTML chars).
#[inline]
fn write_value_to_output(value: &Value, escaped: bool, output: &mut String) {
    use std::fmt::Write;
    match value {
        Value::String(s) => {
            if escaped {
                output.push_str(&html_escape(s));
            } else {
                output.push_str(s);
            }
        }
        // Int/Float/Bool never contain HTML special chars — skip html_escape entirely
        Value::Int(n) => {
            let _ = write!(output, "{}", n);
        }
        Value::Float(n) => {
            let _ = write!(output, "{}", n);
        }
        Value::Bool(b) => {
            let _ = write!(output, "{}", b);
        }
        Value::Null => {}
        Value::Array(arr) => {
            let arr = arr.borrow();
            for (i, item) in arr.iter().enumerate() {
                if i > 0 {
                    output.push_str(", ");
                }
                write_value_to_output(item, escaped, output);
            }
        }
        Value::Hash(_) => output.push_str("[Hash]"),
        _ => {
            // Any other value type (Decimal, DateTime, Instance, ...) renders via
            // Display. Honor the escape flag here too: an Instance's Display can
            // embed user-controlled field data, so emitting it raw in an escaped
            // (`<%= %>`) context would be an XSS hole. html_escape is a no-op
            // (borrowed, zero-copy) for the common HTML-free cases (numbers/dates).
            if escaped {
                output.push_str(&html_escape(&value.to_string()));
            } else {
                let _ = write!(output, "{}", value);
            }
        }
    }
}

/// Escape HTML special characters.
/// Returns Cow::Borrowed when no escaping is needed (fast path).
pub fn html_escape(s: &str) -> Cow<'_, str> {
    // Fast path: scan bytes to check if escaping is needed at all
    if !s
        .bytes()
        .any(|b| matches!(b, b'&' | b'<' | b'>' | b'"' | b'\''))
    {
        return Cow::Borrowed(s);
    }
    let mut result: Vec<u8> = Vec::with_capacity(s.len() + 8);
    for &b in s.as_bytes() {
        match b {
            b'&' => result.extend_from_slice(b"&amp;"),
            b'<' => result.extend_from_slice(b"&lt;"),
            b'>' => result.extend_from_slice(b"&gt;"),
            b'"' => result.extend_from_slice(b"&quot;"),
            b'\'' => result.extend_from_slice(b"&#x27;"),
            _ => result.push(b),
        }
    }
    // SAFETY: Input was valid UTF-8 and we only replaced ASCII bytes
    // with ASCII sequences, so the result is still valid UTF-8.
    Cow::Owned(unsafe { String::from_utf8_unchecked(result) })
}

/// Escape for use in a JavaScript string literal context.
/// Escapes backslash, quotes, and characters that would break out of the
/// literal — including newlines (line-continuation breakouts), CR, and tab.
/// `< > &` are also escaped so the embedded literal can't terminate the
/// surrounding `<script>` block.
pub fn js_escape(s: &str) -> Cow<'_, str> {
    if !s.bytes().any(|b| {
        matches!(
            b,
            b'\\' | b'"' | b'\'' | b'<' | b'>' | b'&' | b'\n' | b'\r' | b'\t'
        )
    }) {
        return Cow::Borrowed(s);
    }
    let mut result: Vec<u8> = Vec::with_capacity(s.len() + 8);
    for &b in s.as_bytes() {
        match b {
            b'\\' => result.extend_from_slice(b"\\\\"),
            b'"' => result.extend_from_slice(b"\\\""),
            b'\'' => result.extend_from_slice(b"\\'"),
            b'\n' => result.extend_from_slice(b"\\n"),
            b'\r' => result.extend_from_slice(b"\\r"),
            b'\t' => result.extend_from_slice(b"\\t"),
            b'<' => result.extend_from_slice(b"&lt;"),
            b'>' => result.extend_from_slice(b"&gt;"),
            b'&' => result.extend_from_slice(b"&amp;"),
            _ => result.push(b),
        }
    }
    Cow::Owned(unsafe { String::from_utf8_unchecked(result) })
}

/// Escape for use in an HTML attribute value context.
/// Escapes quotes and angle brackets.
pub fn attr_escape(s: &str) -> Cow<'_, str> {
    if !s
        .bytes()
        .any(|b| matches!(b, b'"' | b'\'' | b'<' | b'>' | b'&'))
    {
        return Cow::Borrowed(s);
    }
    let mut result: Vec<u8> = Vec::with_capacity(s.len() + 8);
    for &b in s.as_bytes() {
        match b {
            b'"' => result.extend_from_slice(b"&quot;"),
            b'\'' => result.extend_from_slice(b"&#x27;"),
            b'<' => result.extend_from_slice(b"&lt;"),
            b'>' => result.extend_from_slice(b"&gt;"),
            b'&' => result.extend_from_slice(b"&amp;"),
            _ => result.push(b),
        }
    }
    Cow::Owned(unsafe { String::from_utf8_unchecked(result) })
}

/// Escape for use in a URL query parameter context.
/// Percent-encodes characters that are not safe in query values.
pub fn url_escape(s: &str) -> Cow<'_, str> {
    let mut result = String::new();
    for c in s.chars() {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => result.push(c),
            _ => {
                let mut buf = [0u8; 4];
                let encoded = c.encode_utf8(&mut buf);
                for byte in encoded.bytes() {
                    result.push_str(&format!("%{:02X}", byte));
                }
            }
        }
    }
    Cow::Owned(result)
}

/// Auto-call callable values (Function, NativeFunction, Method) with no arguments.
/// This allows templates to omit parentheses for no-arg method calls: `<%= now.to_iso %>`.
#[inline]
fn auto_call_if_callable(interpreter: &mut Interpreter, value: Value) -> Result<Value, String> {
    match &value {
        Value::Function(_) | Value::NativeFunction(_) | Value::Method(_) => interpreter
            .call_value(value, vec![], Span::default())
            .map_err(|e| format!("Evaluation error: {}", e)),
        _ => Ok(value),
    }
}

/// Check if a value is truthy
#[inline]
fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Bool(b) => *b,
        Value::Null => false,
        Value::Int(0) => false,
        Value::String(s) if s.is_empty() => false,
        Value::Array(arr) if arr.borrow().is_empty() => false,
        Value::Hash(hash) if hash.borrow().is_empty() => false,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::value::{HashKey, HashPairs};
    use crate::template::parser::parse_template;

    fn make_hash(pairs: Vec<(&str, Value)>) -> Value {
        let hash: HashPairs = pairs
            .into_iter()
            .map(|(k, v)| (HashKey::String(k.to_string().into()), v))
            .collect();
        Value::Hash(Rc::new(RefCell::new(hash)))
    }

    #[test]
    fn test_render_literal() {
        let nodes = vec![TemplateNode::Literal("Hello World".to_string())];
        let data = make_hash(vec![]);
        let result = render_nodes(&nodes, &data, None).unwrap();
        assert_eq!(result, "Hello World");
    }

    #[test]
    fn test_render_output_escaped() {
        let nodes = vec![TemplateNode::Output {
            expr: Expr::Var("name".to_string()),
            escaped: true,
            line: 1,
        }];
        let data = make_hash(vec![("name", Value::String("<script>".into()))]);
        let result = render_nodes(&nodes, &data, None).unwrap();
        assert_eq!(result, "&lt;script&gt;");
    }

    #[test]
    fn test_render_output_raw() {
        let nodes = vec![TemplateNode::Output {
            expr: Expr::Var("html".to_string()),
            escaped: false,
            line: 1,
        }];
        let data = make_hash(vec![("html", Value::String("<b>bold</b>".into()))]);
        let result = render_nodes(&nodes, &data, None).unwrap();
        assert_eq!(result, "<b>bold</b>");
    }

    #[test]
    fn test_render_at_prefixed_var_falls_back_to_local() {
        // `@title` in a view has no class context — it should resolve to the
        // bare `title` local rather than erroring with "'this' outside of class".
        // Mirrors the lenient-vars mode the real Renderer enters during render.
        let _guard = crate::interpreter::executor::enter_template_lenient_vars();

        let nodes = parse_template("<%= @title %>|<%= title %>").unwrap();
        let data = make_hash(vec![("title", Value::String("Welcome".into()))]);
        let result = render_nodes(&nodes, &data, None).unwrap();
        assert_eq!(result, "Welcome|Welcome");

        // Member access and method calls through an `@ivar` work too.
        let user = make_hash(vec![("name", Value::String("Alice".into()))]);
        let nodes = parse_template("<%= @user.name %>|<%= @title.upcase %>").unwrap();
        let data = make_hash(vec![("user", user), ("title", Value::String("hi".into()))]);
        let result = render_nodes(&nodes, &data, None).unwrap();
        assert_eq!(result, "Alice|HI");

        // A missing `@ivar` yields Null (empty output), like any absent local.
        let nodes = parse_template("[<%= @missing %>]").unwrap();
        let result = render_nodes(&nodes, &make_hash(vec![]), None).unwrap();
        assert_eq!(result, "[]");
    }

    #[test]
    fn test_render_if_true() {
        let nodes = parse_template("<% if show %>visible<% end %>").unwrap();
        let data = make_hash(vec![("show", Value::Bool(true))]);
        let result = render_nodes(&nodes, &data, None).unwrap();
        assert_eq!(result, "visible");
    }

    #[test]
    fn test_render_if_false() {
        let nodes = parse_template("<% if show %>visible<% else %>hidden<% end %>").unwrap();
        let data = make_hash(vec![("show", Value::Bool(false))]);
        let result = render_nodes(&nodes, &data, None).unwrap();
        assert_eq!(result, "hidden");
    }

    #[test]
    fn test_render_for_loop() {
        let nodes = parse_template("<% for item in items %><%= item %><% end %>").unwrap();
        let items = Value::Array(Rc::new(RefCell::new(vec![
            Value::String("a".into()),
            Value::String("b".into()),
            Value::String("c".into()),
        ])));
        let data = make_hash(vec![("items", items)]);
        let result = render_nodes(&nodes, &data, None).unwrap();
        assert_eq!(result, "abc");
    }

    #[test]
    fn test_hash_access() {
        let nodes = parse_template("<%= user[\"name\"] %>").unwrap();
        let user = make_hash(vec![("name", Value::String("Alice".into()))]);
        let data = make_hash(vec![("user", user)]);
        let result = render_nodes(&nodes, &data, None).unwrap();
        assert_eq!(result, "Alice");
    }

    #[test]
    fn test_html_escape() {
        assert_eq!(
            html_escape("<script>alert('xss')</script>"),
            "&lt;script&gt;alert(&#x27;xss&#x27;)&lt;/script&gt;"
        );
    }

    #[test]
    fn test_html_escape_preserves_utf8() {
        assert_eq!(html_escape("café"), "café");
        assert_eq!(html_escape("日本語"), "日本語");
        assert_eq!(html_escape("🚀 rocket"), "🚀 rocket");
        assert_eq!(html_escape("<é> & \"ñ\""), "&lt;é&gt; &amp; &quot;ñ&quot;");
    }

    #[test]
    fn test_html_escape_borrows_when_clean() {
        let plain = "no special chars 日本語";
        match html_escape(plain) {
            Cow::Borrowed(s) => assert_eq!(s, plain),
            Cow::Owned(_) => panic!("expected borrowed Cow for clean input"),
        }
    }

    #[test]
    fn test_code_block_assignment() {
        let nodes =
            parse_template("<% let colors = [\"#D3CCFF\", \"#FF8C8C\"] %><%= colors %>").unwrap();
        let data = make_hash(vec![]);
        let result = render_nodes(&nodes, &data, None).unwrap();
        assert_eq!(result, "#D3CCFF, #FF8C8C");
    }

    #[test]
    fn test_code_block_for_loop() {
        let nodes = parse_template("<% let colors = [\"#D3CCFF\", \"#FF8C8C\"] %><% for color in colors %><%= color %><% end %>").unwrap();
        let data = make_hash(vec![]);
        let result = render_nodes(&nodes, &data, None).unwrap();
        assert_eq!(result, "#D3CCFF#FF8C8C");
    }

    #[test]
    fn each_do_block_renders_like_for() {
        // `<% xs.each do |x| %>` is tokenizer sugar for `<% for x in xs %>`.
        let items = || {
            Value::Array(Rc::new(RefCell::new(vec![
                Value::String("a".into()),
                Value::String("b".into()),
            ])))
        };
        let nodes = parse_template("<% items.each do |item| %>[<%= item %>]<% end %>").unwrap();
        let result = render_nodes(&nodes, &make_hash(vec![("items", items())]), None).unwrap();
        assert_eq!(result, "[a][b]");

        // Two-param form carries the index, mirroring `for item, i in items`.
        let nodes =
            parse_template("<% items.each do |item, i| %><%= i %>:<%= item %>;<% end %>").unwrap();
        let result = render_nodes(&nodes, &make_hash(vec![("items", items())]), None).unwrap();
        assert_eq!(result, "0:a;1:b;");
    }

    #[test]
    fn test_render_for_loop_with_index() {
        let nodes = parse_template("<% for item, i in items %><%= i %><% end %>").unwrap();
        let items = Value::Array(Rc::new(RefCell::new(vec![
            Value::String("a".into()),
            Value::String("b".into()),
            Value::String("c".into()),
        ])));
        let data = make_hash(vec![("items", items)]);
        let result = render_nodes(&nodes, &data, None).unwrap();
        assert_eq!(result, "012");
    }

    #[test]
    fn test_content_for_captures_to_store_not_output() {
        use crate::template::content_store;
        let _frame = content_store::ensure_frame();

        let nodes = parse_template(
            "before<% content_for \"head\" do %><script src=\"/a.js\"></script><% end %>after",
        )
        .unwrap();
        let result = render_nodes(&nodes, &make_hash(vec![]), None).unwrap();
        assert_eq!(result, "beforeafter");
        assert_eq!(
            content_store::get("head").as_deref(),
            Some("<script src=\"/a.js\"></script>")
        );
    }

    #[test]
    fn test_content_for_appends_on_multiple_calls() {
        use crate::template::content_store;
        let _frame = content_store::ensure_frame();

        let nodes = parse_template(
            "<% content_for \"head\" do %>one<% end %><% content_for \"head\" do %>two<% end %>",
        )
        .unwrap();
        render_nodes(&nodes, &make_hash(vec![]), None).unwrap();
        assert_eq!(content_store::get("head").as_deref(), Some("onetwo"));
    }

    #[test]
    fn test_content_for_interpolation_escaped_once() {
        use crate::template::content_store;
        let _frame = content_store::ensure_frame();

        // `<%= %>` inside a capture escapes at capture time; the literal
        // markup around it stays raw.
        let nodes =
            parse_template("<% content_for \"head\" do %><b><%= label %></b><% end %>").unwrap();
        let data = make_hash(vec![("label", Value::String("<xss>".into()))]);
        render_nodes(&nodes, &data, None).unwrap();
        assert_eq!(
            content_store::get("head").as_deref(),
            Some("<b>&lt;xss&gt;</b>")
        );
    }

    #[test]
    fn test_content_for_sees_view_locals_and_loop_vars() {
        use crate::template::content_store;
        let _frame = content_store::ensure_frame();

        let nodes = parse_template(
            "<% for item in items %><% content_for \"list\" do %>[<%= item %>]<% end %><% end %>",
        )
        .unwrap();
        let items = Value::Array(Rc::new(RefCell::new(vec![
            Value::String("a".into()),
            Value::String("b".into()),
        ])));
        let data = make_hash(vec![("items", items)]);
        render_nodes(&nodes, &data, None).unwrap();
        assert_eq!(content_store::get("list").as_deref(), Some("[a][b]"));
    }

    #[test]
    fn test_yield_in_view_still_errors() {
        for src in ["<%= yield %>", "<%= yield \"head\" %>"] {
            let nodes = parse_template(src).unwrap();
            let err = render_nodes(&nodes, &make_hash(vec![]), None).unwrap_err();
            assert!(
                err.contains("outside of layout context"),
                "source {}: got {}",
                src,
                err
            );
        }
    }

    // --- form builder (form_with / csrf_field / button_to) ---------------

    fn render_form(src: &str, data: Vec<(&str, Value)>) -> String {
        let nodes = parse_template(src).unwrap();
        render_nodes(&nodes, &make_hash(data), None).unwrap()
    }

    fn record_with_errors(pairs: Vec<(&str, Value)>, errors: Vec<(&str, &str)>) -> Value {
        let errs: Vec<Value> = errors
            .into_iter()
            .map(|(field, message)| {
                make_hash(vec![
                    ("field", Value::String(field.into())),
                    ("message", Value::String(message.into())),
                ])
            })
            .collect();
        let mut all = pairs;
        all.push(("_errors", Value::Array(Rc::new(RefCell::new(errs)))));
        make_hash(all)
    }

    #[test]
    fn form_with_get_form_has_no_csrf_or_method_override() {
        let html = render_form(
            "<% f = form_with(null, {\"url\": \"/search\", \"method\": \"get\"}) %><%- f.open() %><%- f.close() %>",
            vec![],
        );
        assert_eq!(html, "<form action=\"/search\" method=\"GET\"></form>");
    }

    #[test]
    fn form_with_post_embeds_csrf_token() {
        let html = render_form(
            "<% f = form_with(null, {\"url\": \"/posts\"}) %><%- f.open() %>",
            vec![],
        );
        assert!(html.starts_with("<form action=\"/posts\" method=\"POST\">"));
        assert!(
            html.contains("name=\"_csrf_token\" value=\""),
            "missing csrf field: {}",
            html
        );
    }

    #[test]
    fn form_with_patch_emits_method_override_field() {
        let html = render_form(
            "<% f = form_with(null, {\"url\": \"/posts/7\", \"method\": \"patch\"}) %><%- f.open() %>",
            vec![],
        );
        assert!(
            html.contains("<input type=\"hidden\" name=\"_method\" value=\"PATCH\">"),
            "missing _method override: {}",
            html
        );
    }

    #[test]
    fn form_with_extra_options_become_form_attributes() {
        let html = render_form(
            "<% f = form_with(null, {\"url\": \"/x\", \"multipart\": true, \"class\": \"stack\"}) %><%- f.open() %>",
            vec![],
        );
        assert!(html.contains("enctype=\"multipart/form-data\""), "{}", html);
        assert!(html.contains("class=\"stack\""), "{}", html);
    }

    #[test]
    fn text_field_prefills_value_and_escapes_it() {
        let record = make_hash(vec![("title", Value::String("a \"b\" <c>".into()))]);
        let html = render_form(
            "<% f = form_with(post, {\"url\": \"/posts\"}) %><%- f.text_field(\"title\") %>",
            vec![("post", record)],
        );
        assert!(
            html.contains(
                "type=\"text\" id=\"title\" name=\"title\" value=\"a &quot;b&quot; &lt;c&gt;\""
            ),
            "unexpected input html: {}",
            html
        );
    }

    #[test]
    fn password_field_never_prefills() {
        let record = make_hash(vec![("password", Value::String("secret".into()))]);
        let html = render_form(
            "<% f = form_with(user, {\"url\": \"/login\"}) %><%- f.password_field(\"password\") %>",
            vec![("user", record)],
        );
        assert!(!html.contains("secret"), "password leaked: {}", html);
        assert!(html.contains("type=\"password\""), "{}", html);
    }

    #[test]
    fn field_with_errors_gets_error_class_and_aria() {
        let record = record_with_errors(vec![], vec![("title", "cant be blank")]);
        let html = render_form(
            "<% f = form_with(post, {\"url\": \"/posts\"}) %><%- f.text_field(\"title\", {\"class\": \"input\"}) %><%- f.errors_for(\"title\") %>",
            vec![("post", record)],
        );
        assert!(html.contains("class=\"input field-error\""), "{}", html);
        assert!(html.contains("aria-invalid=\"true\""), "{}", html);
        assert!(
            html.contains("<span class=\"field-error-message\">cant be blank</span>"),
            "missing inline error: {}",
            html
        );
    }

    #[test]
    fn error_summary_lists_all_messages() {
        let record = record_with_errors(
            vec![],
            vec![("title", "can be blank"), ("body", "too short")],
        );
        let html = render_form(
            "<% f = form_with(post, {\"url\": \"/posts\"}) %><%- f.error_summary() %>",
            vec![("post", record)],
        );
        assert!(
            html.starts_with("<div class=\"form-errors\"><ul>"),
            "{}",
            html
        );
        assert!(html.contains("<li>can be blank</li>"), "{}", html);
        assert!(html.contains("<li>too short</li>"), "{}", html);
    }

    #[test]
    fn error_summary_empty_for_clean_record() {
        let record = make_hash(vec![("title", Value::String("ok".into()))]);
        let html = render_form(
            "<% f = form_with(post, {\"url\": \"/posts\"}) %>[<%- f.error_summary() %>]",
            vec![("post", record)],
        );
        assert_eq!(html, "[]");
    }

    #[test]
    fn select_marks_current_value_selected_and_supports_pairs() {
        // `[ [` with a space — a leading `[[` would lex as a Lua-style raw
        // string, not a nested array literal.
        let record = make_hash(vec![("status", Value::String("late".into()))]);
        let html = render_form(
            "<% f = form_with(rec, {\"url\": \"/x\"}) %><% choices = [ [\"On time\", \"up\"], [\"Late\", \"late\"] ] %><%- f.select(\"status\", choices) %>",
            vec![("rec", record)],
        );
        assert!(
            html.contains("<option value=\"up\">On time</option>"),
            "{}",
            html
        );
        assert!(
            html.contains("<option value=\"late\" selected>Late</option>"),
            "{}",
            html
        );
    }

    #[test]
    fn check_box_checked_from_bool_value() {
        let record = make_hash(vec![("published", Value::Bool(true))]);
        let html = render_form(
            "<% f = form_with(rec, {\"url\": \"/x\"}) %><%- f.check_box(\"published\") %>",
            vec![("rec", record)],
        );
        assert!(
            html.contains("name=\"published\" value=\"true\" checked"),
            "{}",
            html
        );
    }

    #[test]
    fn label_humanizes_field_name() {
        let html = render_form(
            "<% f = form_with(null, {\"url\": \"/x\"}) %><%- f.label(\"first_name\") %>",
            vec![],
        );
        assert_eq!(html, "<label for=\"first_name\">First name</label>");
    }

    #[test]
    fn button_to_delete_emits_override_csrf_and_confirm() {
        let html = render_form(
            "<%- button_to(\"Delete\", \"/posts/7\", {\"method\": \"delete\", \"confirm\": \"Sure?\"}) %>",
            vec![],
        );
        assert!(
            html.starts_with("<form action=\"/posts/7\" method=\"POST\">"),
            "{}",
            html
        );
        assert!(
            html.contains("<input type=\"hidden\" name=\"_method\" value=\"DELETE\">"),
            "{}",
            html
        );
        assert!(html.contains("name=\"_csrf_token\""), "{}", html);
        assert!(
            html.ends_with(
                "<button type=\"submit\" onclick=\"return confirm('Sure?')\">Delete</button></form>"
            ),
            "{}",
            html
        );
    }

    #[test]
    fn form_with_block_wraps_body_in_open_close() {
        let record = make_hash(vec![("title", Value::String("Hi".into()))]);
        let html = render_form(
            "<% form_with(post, {\"url\": \"/posts\"}) do %><%- f.text_field(\"title\") %><% end %>",
            vec![("post", record)],
        );
        assert!(
            html.starts_with("<form action=\"/posts\" method=\"POST\">"),
            "{}",
            html
        );
        assert!(html.contains("name=\"_csrf_token\""), "{}", html);
        assert!(html.contains("value=\"Hi\""), "{}", html);
        assert!(html.ends_with("</form>"), "{}", html);
    }

    #[test]
    fn form_with_block_supports_named_var_and_output_tags() {
        // Rails-style: the opener reads naturally as an output tag; `-%>`
        // swallows the newline after each tag.
        let html = render_form(
            "<%- form_with(null, {\"url\": \"/search\", \"method\": \"get\"}) do |form| -%>\n<%- form.text_field(\"q\") -%>\n<%- end -%>\n",
            vec![],
        );
        assert_eq!(
            html,
            "<form action=\"/search\" method=\"GET\"><input type=\"text\" id=\"q\" name=\"q\"></form>"
        );
    }

    #[test]
    fn form_with_block_nests_inside_for() {
        let items = Value::Array(Rc::new(RefCell::new(vec![
            Value::String("a".into()),
            Value::String("b".into()),
        ])));
        let html = render_form(
            "<% for item in items %><% form_with(null, {\"url\": \"/x\", \"method\": \"get\"}) do %><%= item %><% end %><% end %>",
            vec![("items", items)],
        );
        assert_eq!(
            html,
            "<form action=\"/x\" method=\"GET\">a</form><form action=\"/x\" method=\"GET\">b</form>"
        );
    }

    #[test]
    fn form_with_block_missing_end_errors() {
        let err = parse_template("<% form_with(null) do %>never closed").unwrap_err();
        assert!(err.contains("Unclosed form_with block"), "got: {}", err);
    }

    #[test]
    fn trim_marker_swallows_following_newline() {
        let html = render_form(
            "<%= name -%>\nrest",
            vec![("name", Value::String("A".into()))],
        );
        assert_eq!(html, "Arest");
        // Without the marker the newline stays.
        let html = render_form(
            "<%= name %>\nrest",
            vec![("name", Value::String("A".into()))],
        );
        assert_eq!(html, "A\nrest");
    }

    #[test]
    fn fields_for_prefixes_names_and_prefills_nested_values() {
        let record = make_hash(vec![(
            "author",
            make_hash(vec![("name", Value::String("Ada".into()))]),
        )]);
        let html = render_form(
            "<% f = form_with(post, {\"url\": \"/x\"}) %><% af = f.fields_for(\"author\") %><%- af.label(\"name\") %><%- af.text_field(\"name\") %>",
            vec![("post", record)],
        );
        assert!(
            html.contains("<label for=\"author_name\">Name</label>"),
            "{}",
            html
        );
        assert!(
            html.contains(
                "<input type=\"text\" id=\"author_name\" name=\"author[name]\" value=\"Ada\">"
            ),
            "{}",
            html
        );
    }

    #[test]
    fn fields_for_block_binds_sub_builder_without_wrapping() {
        let record = make_hash(vec![(
            "author",
            make_hash(vec![("name", Value::String("Ada".into()))]),
        )]);
        let html = render_form(
            "<%- form_with(post, {\"url\": \"/x\"}) do |f| -%>\n<%- f.fields_for(\"author\") do |af| -%>\n<%- af.text_field(\"name\") -%>\n<%- end -%>\n<%- end -%>",
            vec![("post", record)],
        );
        assert!(
            html.contains("name=\"author[name]\" value=\"Ada\""),
            "{}",
            html
        );
        // Exactly one form tag pair — the fields_for block adds no wrapper.
        assert_eq!(html.matches("<form").count(), 1, "{}", html);
        assert_eq!(html.matches("</form>").count(), 1, "{}", html);
    }

    #[test]
    fn fields_for_with_index_and_deep_nesting() {
        let html = render_form(
            "<% f = form_with(null, {\"url\": \"/x\"}) %><% item = f.fields_for(\"items\", 0) %><%- item.text_field(\"sku\") %>",
            vec![],
        );
        assert!(
            html.contains("id=\"items_0_sku\" name=\"items[0][sku]\""),
            "{}",
            html
        );
    }

    #[test]
    fn select_multiple_appends_brackets_and_name_override_wins() {
        let html = render_form(
            "<% f = form_with(null, {\"url\": \"/x\"}) %><% choices = [\"a\", \"b\"] %><%- f.select(\"tags\", choices, {\"multiple\": true}) %><%- f.text_field(\"extra\", {\"name\": \"custom[key]\"}) %>",
            vec![],
        );
        assert!(
            html.contains("<select id=\"tags\" name=\"tags[]\" multiple>"),
            "{}",
            html
        );
        assert!(html.contains("name=\"custom[key]\""), "{}", html);
    }

    #[test]
    fn csrf_meta_tag_and_field_share_the_session_token() {
        let html = render_form("<%- csrf_meta_tag() %>|<%- csrf_field() %>", vec![]);
        let token_of = |marker: &str| {
            let start = html.find(marker).unwrap() + marker.len();
            html[start..start + 32].to_string()
        };
        let meta_token = token_of("content=\"");
        let field_token = token_of("value=\"");
        assert_eq!(meta_token, field_token);
        assert_eq!(meta_token.len(), 32);
    }

    #[test]
    fn component_block_renders_with_props_and_content() {
        // Mock component renderer that reads BOTH the prop (title) and the
        // default slot (content) straight from the data hash, so the assertion
        // proves props actually flow through (not a hardcoded value).
        let component_renderer: PartialRenderer = Some(&|name: &str, data: &Value| {
            if name != "components/card" {
                return Err(format!("unknown component: {}", name));
            }
            let Value::Hash(h) = data else {
                return Ok("<div class=\"card\"></div>".to_string());
            };
            let borrowed = h.borrow();
            let field = |key: &str| match borrowed.get(&HashKey::String(key.into())) {
                Some(Value::String(s)) => s.to_string(),
                _ => String::new(),
            };
            Ok(format!(
                "<div class=\"card\"><h3>{}</h3>{}</div>",
                field("title"),
                field("content")
            ))
        });

        // Named-arg form: `title: "Stats"` must reach the component as a prop
        // (regression guard for the previously-dropped Named argument).
        let nodes =
            parse_template("<%- component \"card\", title: \"Stats\" do %>body here<% end %>")
                .unwrap();
        let result = render_nodes(&nodes, &make_hash(vec![]), component_renderer).unwrap();
        assert_eq!(result, "<div class=\"card\"><h3>Stats</h3>body here</div>");

        // Paren form with an explicit positional hash must render identically.
        let nodes2 = parse_template(
            "<%- component(\"card\", { \"title\": \"Stats\" }) do %>body here<% end %>",
        )
        .unwrap();
        let result2 = render_nodes(&nodes2, &make_hash(vec![]), component_renderer).unwrap();
        assert_eq!(result2, "<div class=\"card\"><h3>Stats</h3>body here</div>");
    }
}
