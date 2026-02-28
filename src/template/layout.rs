//! Layout support for templates.
//!
//! Handles wrapping rendered content with layout templates that use `<%= yield %>`.
//! Uses a single interpreter per render call for optimal performance.
//! Writes directly into a shared output buffer (no intermediate String allocations).

use std::cell::RefCell;
use std::rc::Rc;

use crate::interpreter::executor::Interpreter;
use crate::interpreter::value::Value;
use crate::span::Span;
use crate::template::core_eval;
use crate::template::parser::{parse_template, Expr, TemplateNode};

/// Type alias for partial renderer callback to reduce type complexity.
type PartialRenderer<'a> = Option<&'a dyn Fn(&str, &Value) -> Result<String, String>>;

/// Render content with a layout that has a yield placeholder.
pub fn render_with_layout(
    layout_source: &str,
    content: &str,
    data: &Value,
    partial_renderer: PartialRenderer<'_>,
) -> Result<String, String> {
    render_with_layout_path(layout_source, content, data, partial_renderer, None)
}

/// Render content with a layout, including layout path for error reporting.
pub fn render_with_layout_path(
    layout_source: &str,
    content: &str,
    data: &Value,
    partial_renderer: PartialRenderer<'_>,
    layout_path: Option<&str>,
) -> Result<String, String> {
    let layout_nodes = parse_template(layout_source).map_err(|e| {
        if let Some(path) = layout_path {
            format!("{} in {}", e, path)
        } else {
            e
        }
    })?;
    render_layout_nodes_with_path(&layout_nodes, content, data, partial_renderer, layout_path)
}

/// Render layout nodes, replacing Yield nodes with the content.
pub fn render_layout_nodes(
    nodes: &[TemplateNode],
    content: &str,
    data: &Value,
    partial_renderer: PartialRenderer<'_>,
) -> Result<String, String> {
    render_layout_nodes_with_path(nodes, content, data, partial_renderer, None)
}

/// Render layout nodes with path for error reporting.
pub fn render_layout_nodes_with_path(
    nodes: &[TemplateNode],
    content: &str,
    data: &Value,
    partial_renderer: PartialRenderer<'_>,
    layout_path: Option<&str>,
) -> Result<String, String> {
    let mut interpreter = core_eval::create_template_interpreter(data);
    let mut output = String::new();
    render_layout_inner(
        &mut interpreter,
        nodes,
        content,
        data,
        partial_renderer,
        layout_path,
        &mut output,
    )?;
    Ok(output)
}

/// Render layout nodes with an existing interpreter (avoids creating a new one).
/// Used to share a single interpreter across view + layout rendering.
pub fn render_layout_with_interpreter(
    interpreter: &mut Interpreter,
    nodes: &[TemplateNode],
    content: &str,
    data: &Value,
    partial_renderer: PartialRenderer<'_>,
    layout_path: Option<&str>,
) -> Result<String, String> {
    // Pre-allocate: layout wraps content, so output ≈ content + layout overhead
    let mut output = String::with_capacity(content.len() + 2048);
    render_layout_inner(
        interpreter,
        nodes,
        content,
        data,
        partial_renderer,
        layout_path,
        &mut output,
    )?;
    Ok(output)
}

/// Internal layout render function that writes directly into the output buffer.
fn render_layout_inner(
    interpreter: &mut Interpreter,
    nodes: &[TemplateNode],
    content: &str,
    data: &Value,
    partial_renderer: PartialRenderer<'_>,
    layout_path: Option<&str>,
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
                        render_layout_inner(
                            interpreter,
                            body,
                            content,
                            data,
                            partial_renderer,
                            layout_path,
                            output,
                        )?;
                    } else if let Some(else_nodes) = else_body {
                        render_layout_inner(
                            interpreter,
                            else_nodes,
                            content,
                            data,
                            partial_renderer,
                            layout_path,
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
                                render_layout_inner(
                                    interpreter,
                                    body,
                                    content,
                                    data,
                                    partial_renderer,
                                    layout_path,
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
                                render_layout_inner(
                                    interpreter,
                                    body,
                                    content,
                                    data,
                                    partial_renderer,
                                    layout_path,
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
                    output.push_str(content);
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
                TemplateNode::CoreCodeBlock { stmts, line: _ } => {
                    for stmt in stmts {
                        interpreter.execute(stmt)
                            .map_err(|e| format!("Evaluation error: {}", e))?;
                    }
                }
                TemplateNode::CoreOutput { expr, escaped, line: _ } => {
                    let value = interpreter.evaluate(expr)
                        .map_err(|e| format!("Evaluation error: {}", e))?;
                    // Auto-call methods/functions with no args (parentheses optional in templates)
                    let value = auto_call_if_callable(interpreter, value)?;
                    write_value_to_output(&value, *escaped, output);
                }
            }
            Ok(())
        })();

        if let Err(e) = result {
            if let Some(path) = layout_path {
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
                output.push_str(&crate::template::renderer::html_escape(s));
            } else {
                output.push_str(s);
            }
        }
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

/// Auto-call callable values (Function, NativeFunction, Method) with no arguments.
/// This allows templates to omit parentheses for no-arg method calls: `<%= now.to_iso %>`.
#[inline]
fn auto_call_if_callable(interpreter: &mut Interpreter, value: Value) -> Result<Value, String> {
    match &value {
        Value::Function(_) | Value::NativeFunction(_) | Value::Method(_) => {
            interpreter
                .call_value(value, vec![], Span::default())
                .map_err(|e| format!("Evaluation error: {}", e))
        }
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
    use crate::interpreter::value::HashKey;
    use indexmap::IndexMap;

    fn make_hash(pairs: Vec<(&str, Value)>) -> Value {
        let hash: IndexMap<HashKey, Value> = pairs
            .into_iter()
            .map(|(k, v)| (HashKey::String(k.to_string()), v))
            .collect();
        Value::Hash(Rc::new(RefCell::new(hash)))
    }

    #[test]
    fn test_render_with_layout() {
        let layout = "<!DOCTYPE html><html><body><%= yield %></body></html>";
        let content = "<h1>Hello</h1>";
        let data = make_hash(vec![]);

        let result = render_with_layout(layout, content, &data, None).unwrap();
        assert_eq!(
            result,
            "<!DOCTYPE html><html><body><h1>Hello</h1></body></html>"
        );
    }

    #[test]
    fn test_layout_with_variables() {
        let layout = "<!DOCTYPE html><title><%= title %></title><body><%= yield %></body>";
        let content = "Page content";
        let data = make_hash(vec![("title", Value::String("My Page".to_string()))]);

        let result = render_with_layout(layout, content, &data, None).unwrap();
        assert_eq!(
            result,
            "<!DOCTYPE html><title>My Page</title><body>Page content</body>"
        );
    }

    #[test]
    fn test_layout_with_conditional() {
        let layout = "<% if show_nav %><nav>Nav</nav><% end %><%= yield %>";
        let content = "Content";
        let data = make_hash(vec![("show_nav", Value::Bool(true))]);

        let result = render_with_layout(layout, content, &data, None).unwrap();
        assert_eq!(result, "<nav>Nav</nav>Content");
    }
}
