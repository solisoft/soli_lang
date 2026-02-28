//! Template renderer that executes template AST with a data context.
//!
//! Uses a single interpreter per render call for optimal performance.
//! Writes directly into a shared output buffer (no intermediate String allocations).

use std::borrow::Cow;
use std::cell::RefCell;
use std::rc::Rc;

use crate::interpreter::executor::Interpreter;
use crate::interpreter::value::Value;
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
                    let cond_value =
                        core_eval::evaluate_with_interpreter(condition, interpreter)?;
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
                    let iterable_value =
                        core_eval::evaluate_with_interpreter(iterable, interpreter)?;
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
                TemplateNode::Yield => {
                    return Err("yield encountered outside of layout context".to_string());
                }
                TemplateNode::Partial {
                    name,
                    context,
                    line: _,
                } => {
                    if let Some(renderer) = partial_renderer {
                        let partial_data = if let Some(ctx_expr) = context {
                            core_eval::evaluate_with_interpreter(ctx_expr, interpreter)?
                        } else {
                            data.clone()
                        };
                        output.push_str(&renderer(name, &partial_data)?);
                    } else {
                        return Err(format!("Partial rendering not available for '{}'", name));
                    }
                }
                TemplateNode::CodeBlock { expr, line: _ } => {
                    match expr {
                        Expr::Assign(name, value_expr) => {
                            let value = core_eval::evaluate_with_interpreter(
                                value_expr,
                                interpreter,
                            )?;
                            core_eval::define_var(interpreter, name, value);
                        }
                        _ => {
                            core_eval::evaluate_with_interpreter(expr, interpreter)?;
                        }
                    }
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
        Value::Int(n) => { let _ = write!(output, "{}", n); }
        Value::Float(n) => { let _ = write!(output, "{}", n); }
        Value::Bool(b) => { let _ = write!(output, "{}", b); }
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
        _ => { let _ = write!(output, "{}", value); }
    }
}

/// Escape HTML special characters.
/// Returns Cow::Borrowed when no escaping is needed (fast path).
pub fn html_escape(s: &str) -> Cow<'_, str> {
    // Fast path: scan bytes to check if escaping is needed at all
    if !s.bytes().any(|b| matches!(b, b'&' | b'<' | b'>' | b'"' | b'\'')) {
        return Cow::Borrowed(s);
    }
    let mut result = String::with_capacity(s.len() + 8);
    for b in s.bytes() {
        match b {
            b'&' => result.push_str("&amp;"),
            b'<' => result.push_str("&lt;"),
            b'>' => result.push_str("&gt;"),
            b'"' => result.push_str("&quot;"),
            b'\'' => result.push_str("&#x27;"),
            _ => result.push(b as char),
        }
    }
    Cow::Owned(result)
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
    use crate::interpreter::value::HashKey;
    use crate::template::parser::compile_expr;
    use indexmap::IndexMap;

    fn make_hash(pairs: Vec<(&str, Value)>) -> Value {
        let hash: IndexMap<HashKey, Value> = pairs
            .into_iter()
            .map(|(k, v)| (HashKey::String(k.to_string()), v))
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
        let data = make_hash(vec![("name", Value::String("<script>".to_string()))]);
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
        let data = make_hash(vec![("html", Value::String("<b>bold</b>".to_string()))]);
        let result = render_nodes(&nodes, &data, None).unwrap();
        assert_eq!(result, "<b>bold</b>");
    }

    #[test]
    fn test_render_if_true() {
        let nodes = vec![TemplateNode::If {
            condition: Expr::Var("show".to_string()),
            body: vec![TemplateNode::Literal("visible".to_string())],
            else_body: None,
            line: 1,
        }];
        let data = make_hash(vec![("show", Value::Bool(true))]);
        let result = render_nodes(&nodes, &data, None).unwrap();
        assert_eq!(result, "visible");
    }

    #[test]
    fn test_render_if_false() {
        let nodes = vec![TemplateNode::If {
            condition: Expr::Var("show".to_string()),
            body: vec![TemplateNode::Literal("visible".to_string())],
            else_body: Some(vec![TemplateNode::Literal("hidden".to_string())]),
            line: 1,
        }];
        let data = make_hash(vec![("show", Value::Bool(false))]);
        let result = render_nodes(&nodes, &data, None).unwrap();
        assert_eq!(result, "hidden");
    }

    #[test]
    fn test_render_for_loop() {
        let nodes = vec![TemplateNode::For {
            var: "item".to_string(),
            index_var: None,
            iterable: Expr::Var("items".to_string()),
            body: vec![TemplateNode::Output {
                expr: Expr::Var("item".to_string()),
                escaped: true,
                line: 1,
            }],
            line: 1,
        }];
        let items = Value::Array(Rc::new(RefCell::new(vec![
            Value::String("a".to_string()),
            Value::String("b".to_string()),
            Value::String("c".to_string()),
        ])));
        let data = make_hash(vec![("items", items)]);
        let result = render_nodes(&nodes, &data, None).unwrap();
        assert_eq!(result, "abc");
    }

    #[test]
    fn test_hash_access() {
        let nodes = vec![TemplateNode::Output {
            expr: compile_expr("user[\"name\"]"),
            escaped: true,
            line: 1,
        }];
        let user = make_hash(vec![("name", Value::String("Alice".to_string()))]);
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
    fn test_code_block_assignment() {
        let nodes = vec![
            TemplateNode::CodeBlock {
                expr: Expr::Assign(
                    "colors".to_string(),
                    Box::new(Expr::ArrayLit(vec![
                        Expr::StringLit("#D3CCFF".to_string()),
                        Expr::StringLit("#FF8C8C".to_string()),
                    ])),
                ),
                line: 1,
            },
            TemplateNode::Output {
                expr: Expr::Var("colors".to_string()),
                escaped: true,
                line: 2,
            },
        ];
        let data = make_hash(vec![]);
        let result = render_nodes(&nodes, &data, None).unwrap();
        assert_eq!(result, "#D3CCFF, #FF8C8C");
    }

    #[test]
    fn test_code_block_for_loop() {
        let nodes = vec![
            TemplateNode::CodeBlock {
                expr: Expr::Assign(
                    "colors".to_string(),
                    Box::new(Expr::ArrayLit(vec![
                        Expr::StringLit("#D3CCFF".to_string()),
                        Expr::StringLit("#FF8C8C".to_string()),
                    ])),
                ),
                line: 1,
            },
            TemplateNode::For {
                var: "color".to_string(),
                index_var: None,
                iterable: Expr::Var("colors".to_string()),
                body: vec![TemplateNode::Output {
                    expr: Expr::Var("color".to_string()),
                    escaped: true,
                    line: 3,
                }],
                line: 2,
            },
        ];
        let data = make_hash(vec![]);
        let result = render_nodes(&nodes, &data, None).unwrap();
        assert_eq!(result, "#D3CCFF#FF8C8C");
    }

    #[test]
    fn test_render_for_loop_with_index() {
        let nodes = vec![TemplateNode::For {
            var: "item".to_string(),
            index_var: Some("i".to_string()),
            iterable: Expr::Var("items".to_string()),
            body: vec![TemplateNode::Output {
                expr: Expr::Var("i".to_string()),
                escaped: true,
                line: 1,
            }],
            line: 1,
        }];
        let items = Value::Array(Rc::new(RefCell::new(vec![
            Value::String("a".to_string()),
            Value::String("b".to_string()),
            Value::String("c".to_string()),
        ])));
        let data = make_hash(vec![("items", items)]);
        let result = render_nodes(&nodes, &data, None).unwrap();
        assert_eq!(result, "012");
    }
}
