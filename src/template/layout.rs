//! Layout support for templates.
//!
//! Handles wrapping rendered content with layout templates that use `<%= yield %>`.

use std::cell::RefCell;
use std::rc::Rc;

use indexmap::IndexMap;

use crate::interpreter::value::{HashKey, Value};
use crate::interpreter::Interpreter;
use crate::template::helpers::{call_array_method, call_string_method};
use crate::template::parser::{parse_template, BinaryOp, CompareOp, Expr, TemplateNode};

/// Type alias for partial renderer callback to reduce type complexity.
type PartialRenderer<'a> = Option<&'a dyn Fn(&str, &Value) -> Result<String, String>>;

/// Resolve a value if it's a Future, otherwise return as-is.
/// This enables auto-resolution of async HTTP responses in templates.
#[inline]
fn resolve_if_future(value: Value) -> Result<Value, String> {
    if value.is_future() {
        value.resolve()
    } else {
        Ok(value)
    }
}

/// Render content with a layout that has a yield placeholder.
///
/// The layout template should contain `<%= yield %>` where the content should be inserted.
///
/// # Arguments
/// * `layout_source` - The layout template source
/// * `content` - The already-rendered content to insert at yield
/// * `data` - The data context for the layout
/// * `partial_renderer` - Optional callback for rendering partials in the layout
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
    let mut output = String::new();
    let mut current_data = data.clone();

    for node in nodes {
        // Track line number for error reporting
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
                    let value = evaluate_expr(expr, &current_data)?;
                    let s = value_to_string(&value);
                    if *escaped {
                        output.push_str(&crate::template::renderer::html_escape(&s));
                    } else {
                        output.push_str(&s);
                    }
                }
                TemplateNode::If {
                    condition,
                    body,
                    else_body,
                    line: _,
                } => {
                    let cond_value = evaluate_expr(condition, &current_data)?;
                    if is_truthy(&cond_value) {
                        output.push_str(&render_layout_nodes_with_path(
                            body,
                            content,
                            &current_data,
                            partial_renderer,
                            layout_path,
                        )?);
                    } else if let Some(else_nodes) = else_body {
                        output.push_str(&render_layout_nodes_with_path(
                            else_nodes,
                            content,
                            &current_data,
                            partial_renderer,
                            layout_path,
                        )?);
                    }
                }
                TemplateNode::For {
                    var,
                    index_var,
                    iterable,
                    body,
                    line: _,
                } => {
                    // Get the iterable value
                    let iterable_value = evaluate_expr(iterable, &current_data)?;
                    match &iterable_value {
                        Value::Array(arr) => {
                            for (i, item) in arr.borrow().iter().enumerate() {
                                let loop_data = with_variable(&current_data, var, item.clone())?;
                                let loop_data = if let Some(idx_var) = index_var {
                                    with_variable(&loop_data, idx_var, Value::Int(i as i64))?
                                } else {
                                    loop_data
                                };
                                output.push_str(&render_layout_nodes_with_path(
                                    body,
                                    content,
                                    &loop_data,
                                    partial_renderer,
                                    layout_path,
                                )?);
                            }
                        }
                        Value::Hash(hash) => {
                            for (k, v) in hash.borrow().iter() {
                                let pair = Value::Array(Rc::new(RefCell::new(vec![
                                    k.to_value(),
                                    v.clone(),
                                ])));
                                let loop_data = with_variable(&current_data, var, pair)?;
                                output.push_str(&render_layout_nodes_with_path(
                                    body,
                                    content,
                                    &loop_data,
                                    partial_renderer,
                                    layout_path,
                                )?);
                            }
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
                    // This is where we insert the rendered content
                    output.push_str(content);
                }
                TemplateNode::Partial {
                    name,
                    context,
                    line: _,
                } => {
                    if let Some(renderer) = partial_renderer {
                        let partial_data = if let Some(ctx_expr) = context {
                            evaluate_expr(ctx_expr, &current_data)?
                        } else {
                            current_data.clone()
                        };
                        output.push_str(&renderer(name, &partial_data)?);
                    } else {
                        return Err(format!("Partial rendering not available for '{}'", name));
                    }
                }
                TemplateNode::CodeBlock { expr, line: _ } => {
                    current_data = evaluate_code_block(expr, &current_data)?;
                }
            }
            Ok(())
        })();

        // Add layout path and line context to errors
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

    Ok(output)
}

/// Evaluate a pre-compiled expression in the context of the data.
#[inline]
fn evaluate_expr(expr: &Expr, data: &Value) -> Result<Value, String> {
    match expr {
        Expr::StringLit(s) => Ok(Value::String(s.clone())),
        Expr::IntLit(n) => Ok(Value::Int(*n)),
        Expr::FloatLit(n) => Ok(Value::Float(*n)),
        Expr::BoolLit(b) => Ok(Value::Bool(*b)),
        Expr::Null => Ok(Value::Null),

        Expr::ArrayLit(elements) => {
            let values: Result<Vec<Value>, String> =
                elements.iter().map(|e| evaluate_expr(e, data)).collect();
            Ok(Value::Array(Rc::new(RefCell::new(values?))))
        }

        Expr::Var(name) => get_hash_value(data, name),

        Expr::Field(base, field) => {
            let base_value = evaluate_expr(base, data)?;
            get_hash_value(&base_value, field)
        }

        Expr::Index(base, key) => {
            let base_value = evaluate_expr(base, data)?;
            let key_value = evaluate_expr(key, data)?;
            index_value(&base_value, &key_value)
        }

        Expr::Binary(left, op, right) => {
            let left_val = evaluate_expr(left, data)?;
            let right_val = evaluate_expr(right, data)?;
            evaluate_binary_op(&left_val, *op, &right_val)
        }

        Expr::Compare(left, op, right) => {
            let left_val = evaluate_expr(left, data)?;
            let right_val = evaluate_expr(right, data)?;
            let result = match op {
                CompareOp::Eq => values_equal(&left_val, &right_val),
                CompareOp::Ne => !values_equal(&left_val, &right_val),
                CompareOp::Gt => compare_values(&left_val, &right_val)? > 0,
                CompareOp::Lt => compare_values(&left_val, &right_val)? < 0,
                CompareOp::Ge => compare_values(&left_val, &right_val)? >= 0,
                CompareOp::Le => compare_values(&left_val, &right_val)? <= 0,
            };
            Ok(Value::Bool(result))
        }

        Expr::And(left, right) => {
            let left_val = evaluate_expr(left, data)?;
            let right_val = evaluate_expr(right, data)?;
            Ok(Value::Bool(is_truthy(&left_val) && is_truthy(&right_val)))
        }

        Expr::Or(left, right) => {
            let left_val = evaluate_expr(left, data)?;
            let right_val = evaluate_expr(right, data)?;
            Ok(Value::Bool(is_truthy(&left_val) || is_truthy(&right_val)))
        }

        Expr::Not(inner) => {
            let inner_val = evaluate_expr(inner, data)?;
            Ok(Value::Bool(!is_truthy(&inner_val)))
        }

        Expr::Method(base, method) => {
            let base_value = evaluate_expr(base, data)?;
            match method.as_str() {
                "length" | "len" | "size" => match &base_value {
                    Value::Array(arr) => Ok(Value::Int(arr.borrow().len() as i64)),
                    Value::String(s) => Ok(Value::Int(s.len() as i64)),
                    Value::Hash(h) => Ok(Value::Int(h.borrow().len() as i64)),
                    _ => Err(format!("Cannot get length of {}", base_value.type_name())),
                },
                _ => Err(format!("Unknown method: {}", method)),
            }
        }

        Expr::MethodCall { base, method, args } => {
            let base_value = evaluate_expr(base, data)?;

            // Evaluate arguments
            let evaluated_args: Result<Vec<Value>, String> =
                args.iter().map(|arg| evaluate_expr(arg, data)).collect();
            let evaluated_args = evaluated_args?;

            match &base_value {
                Value::Array(arr) => {
                    let items: Vec<Value> = arr.borrow().clone();
                    call_array_method(&items, method, evaluated_args)
                }
                Value::String(s) => call_string_method(s, method, evaluated_args),
                _ => Err(format!(
                    "Cannot call method '{}' on {}",
                    method,
                    base_value.type_name()
                )),
            }
        }

        Expr::Call(name, args) => {
            // Evaluate arguments
            let evaluated_args: Result<Vec<Value>, String> =
                args.iter().map(|arg| evaluate_expr(arg, data)).collect();

            let evaluated_args = evaluated_args?;

            // Look up the function in the data context
            let func_value = get_hash_value(data, name)?;

            match func_value {
                Value::NativeFunction(nf) => {
                    // Call the native function
                    (nf.func)(evaluated_args)
                }
                Value::Function(func) => {
                    // Call user-defined function using interpreter
                    call_user_function(&func, evaluated_args)
                }
                Value::Null => Err(format!(
                    "'{}' is not defined (function not found in layout context)",
                    name
                )),
                _ => Err(format!(
                    "'{}' is not a function, got {}",
                    name,
                    func_value.type_name()
                )),
            }
        }

        Expr::Assign(name, value_expr) => {
            let value = evaluate_expr(value_expr, data)?;
            with_variable(data, name, value)
        }

        Expr::Range(start, end) => {
            let start_val = evaluate_expr(start, data)?;
            let end_val = evaluate_expr(end, data)?;
            match (start_val, end_val) {
                (Value::Int(s), Value::Int(e)) => {
                    let mut arr = Vec::new();
                    for i in s..e {
                        arr.push(Value::Int(i));
                    }
                    Ok(Value::Array(Rc::new(RefCell::new(arr))))
                }
                _ => Err("Range requires integer values".to_string()),
            }
        }
    }
}

/// Index into a value (array or hash access)
#[inline]
fn index_value(base: &Value, key: &Value) -> Result<Value, String> {
    match (base, key) {
        (Value::Array(arr), Value::Int(idx)) => {
            let arr = arr.borrow();
            let idx = if *idx < 0 {
                (arr.len() as i64 + idx) as usize
            } else {
                *idx as usize
            };
            if idx < arr.len() {
                Ok(arr[idx].clone())
            } else {
                Ok(Value::Null)
            }
        }
        (Value::Hash(hash), key) => {
            let hash = hash.borrow();
            if let Some(hash_key) = HashKey::from_value(key) {
                if let Some(v) = hash.get(&hash_key) {
                    return Ok(v.clone());
                }
            }
            Ok(Value::Null)
        }
        _ => Ok(Value::Null),
    }
}

/// Get a value from a hash by string key.
/// Optimized to avoid allocating a Value::String for comparison.
#[inline]
fn get_hash_value(value: &Value, key: &str) -> Result<Value, String> {
    match value {
        Value::Hash(hash) => {
            let hash = hash.borrow();
            // Direct lookup using HashKey
            let hash_key = HashKey::String(key.to_string());
            if let Some(v) = hash.get(&hash_key) {
                // Auto-resolve Futures when retrieving values from template data
                return resolve_if_future(v.clone());
            }
            Ok(Value::Null)
        }
        Value::Null => Ok(Value::Null),
        _ => Ok(Value::Null),
    }
}

/// Convert a Value to its string representation
#[inline]
fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Int(n) => n.to_string(),
        Value::Float(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
        Value::Array(arr) => {
            let arr = arr.borrow();
            let items: Vec<String> = arr.iter().map(value_to_string).collect();
            items.join(", ")
        }
        Value::Hash(_) => "[Hash]".to_string(),
        _ => format!("{}", value),
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

/// Check if two values are equal
#[inline]
fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Int(a), Value::Int(b)) => a == b,
        (Value::Float(a), Value::Float(b)) => a == b,
        (Value::Int(a), Value::Float(b)) => (*a as f64) == *b,
        (Value::Float(a), Value::Int(b)) => *a == (*b as f64),
        (Value::String(a), Value::String(b)) => a == b,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Null, Value::Null) => true,
        _ => false,
    }
}

/// Compare two values, returning -1, 0, or 1
#[inline]
fn compare_values(a: &Value, b: &Value) -> Result<i32, String> {
    match (a, b) {
        (Value::Int(a), Value::Int(b)) => Ok(a.cmp(b) as i32),
        (Value::Float(a), Value::Float(b)) => {
            Ok(a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal) as i32)
        }
        (Value::Int(a), Value::Float(b)) => {
            let a = *a as f64;
            Ok(a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal) as i32)
        }
        (Value::Float(a), Value::Int(b)) => {
            let b = *b as f64;
            Ok(a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal) as i32)
        }
        (Value::String(a), Value::String(b)) => Ok(a.cmp(b) as i32),
        _ => Err(format!(
            "Cannot compare {} and {}",
            a.type_name(),
            b.type_name()
        )),
    }
}

/// Evaluate a binary operation
#[inline]
fn evaluate_binary_op(left: &Value, op: BinaryOp, right: &Value) -> Result<Value, String> {
    match op {
        BinaryOp::Add => match (left, right) {
            // String concatenation
            (Value::String(a), Value::String(b)) => Ok(Value::String(format!("{}{}", a, b))),
            (Value::String(a), b) => Ok(Value::String(format!("{}{}", a, value_to_string(b)))),
            (a, Value::String(b)) => Ok(Value::String(format!("{}{}", value_to_string(a), b))),
            // Numeric addition
            (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a + b)),
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a + b)),
            (Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 + b)),
            (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a + *b as f64)),
            _ => Err(format!(
                "Cannot add {} and {}",
                left.type_name(),
                right.type_name()
            )),
        },
        BinaryOp::Subtract => match (left, right) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a - b)),
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a - b)),
            (Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 - b)),
            (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a - *b as f64)),
            _ => Err(format!(
                "Cannot subtract {} from {}",
                right.type_name(),
                left.type_name()
            )),
        },
        BinaryOp::Multiply => match (left, right) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a * b)),
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a * b)),
            (Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 * b)),
            (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a * *b as f64)),
            _ => Err(format!(
                "Cannot multiply {} and {}",
                left.type_name(),
                right.type_name()
            )),
        },
        BinaryOp::Divide => match (left, right) {
            (Value::Int(a), Value::Int(b)) => {
                if *b == 0 {
                    Err("Division by zero".to_string())
                } else {
                    Ok(Value::Int(a / b))
                }
            }
            (Value::Float(a), Value::Float(b)) => {
                if *b == 0.0 {
                    Err("Division by zero".to_string())
                } else {
                    Ok(Value::Float(a / b))
                }
            }
            (Value::Int(a), Value::Float(b)) => {
                if *b == 0.0 {
                    Err("Division by zero".to_string())
                } else {
                    Ok(Value::Float(*a as f64 / b))
                }
            }
            (Value::Float(a), Value::Int(b)) => {
                if *b == 0 {
                    Err("Division by zero".to_string())
                } else {
                    Ok(Value::Float(a / *b as f64))
                }
            }
            _ => Err(format!(
                "Cannot divide {} by {}",
                left.type_name(),
                right.type_name()
            )),
        },
        BinaryOp::Modulo => match (left, right) {
            (Value::Int(a), Value::Int(b)) => {
                if *b == 0 {
                    Err("Modulo by zero".to_string())
                } else {
                    Ok(Value::Int(a % b))
                }
            }
            _ => Err(format!(
                "Cannot perform modulo on {} and {}",
                left.type_name(),
                right.type_name()
            )),
        },
    }
}

/// Create a new data context with an additional variable.
/// Uses copy-on-write optimization: if the hash has only one reference,
/// mutate in place; otherwise create a shallow clone.
fn with_variable(data: &Value, name: &str, value: Value) -> Result<Value, String> {
    match data {
        Value::Hash(hash) => {
            // Check if we have exclusive access (Rc strong count == 1)
            if Rc::strong_count(hash) == 1 {
                // We have exclusive access - mutate in place
                let mut hash_ref = hash.borrow_mut();
                let key = HashKey::String(name.to_string());
                hash_ref.insert(key, value);
                drop(hash_ref);
                Ok(data.clone())
            } else {
                // Multiple references - need to clone
                let mut new_hash: IndexMap<HashKey, Value> = hash.borrow().clone();
                let key = HashKey::String(name.to_string());
                new_hash.insert(key, value);
                Ok(Value::Hash(Rc::new(RefCell::new(new_hash))))
            }
        }
        _ => Err("Data context must be a Hash".to_string()),
    }
}

/// Evaluate a code block expression (e.g., variable assignment)
fn evaluate_code_block(expr: &Expr, data: &Value) -> Result<Value, String> {
    evaluate_expr(expr, data)
}

/// Call a user-defined function (from helpers) using an interpreter.
fn call_user_function(
    func: &crate::interpreter::value::Function,
    args: Vec<Value>,
) -> Result<Value, String> {
    // Create a new interpreter with builtins
    let mut interpreter = Interpreter::new();

    // Copy variables from the function's closure into the interpreter's environment
    // This allows helpers to call other helpers
    let closure_vars = func.closure.borrow().get_all_variables();
    for (name, value) in closure_vars {
        interpreter.environment.borrow_mut().define(name, value);
    }

    // Bind arguments to parameters
    for (i, param) in func.params.iter().enumerate() {
        let arg_value = args.get(i).cloned().unwrap_or(Value::Null);
        interpreter
            .environment
            .borrow_mut()
            .define(param.name.clone(), arg_value);
    }

    // Execute statements and capture return value
    for stmt in &func.body {
        match interpreter.execute(stmt) {
            Ok(crate::interpreter::executor::ControlFlow::Return(value)) => {
                return Ok(value);
            }
            Ok(_) => continue,
            Err(e) => return Err(format!("Error in helper function '{}': {}", func.name, e)),
        }
    }

    // No explicit return - return null
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

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
