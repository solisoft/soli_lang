//! Template renderer that executes template AST with a data context.
//!
//! Evaluates pre-compiled expressions for fast rendering.

use std::cell::RefCell;
use std::rc::Rc;

use crate::interpreter::value::Value;
use crate::template::parser::{BinaryOp, CompareOp, Expr, TemplateNode};

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

/// Render a template AST with the given data context.
///
/// # Arguments
/// * `nodes` - The template AST nodes to render
/// * `data` - The data context (Hash) for variable lookups
/// * `partial_renderer` - Optional callback for rendering partials
///
/// # Returns
/// The rendered HTML string, or an error message.
pub fn render_nodes(
    nodes: &[TemplateNode],
    data: &Value,
    partial_renderer: Option<&dyn Fn(&str, &Value) -> Result<String, String>>,
) -> Result<String, String> {
    render_nodes_with_path(nodes, data, partial_renderer, None)
}

/// Render a template AST with the given data context and template path for error reporting.
///
/// # Arguments
/// * `nodes` - The template AST nodes to render
/// * `data` - The data context (Hash) for variable lookups
/// * `partial_renderer` - Optional callback for rendering partials
/// * `template_path` - Optional path to the template file for error messages
///
/// # Returns
/// The rendered HTML string, or an error message including template path.
pub fn render_nodes_with_path(
    nodes: &[TemplateNode],
    data: &Value,
    partial_renderer: Option<&dyn Fn(&str, &Value) -> Result<String, String>>,
    template_path: Option<&str>,
) -> Result<String, String> {
    let mut output = String::new();

    for node in nodes {
        // Track line number for error reporting
        let node_line = match node {
            TemplateNode::Output { line, .. } => Some(*line),
            TemplateNode::If { line, .. } => Some(*line),
            TemplateNode::For { line, .. } => Some(*line),
            TemplateNode::Partial { line, .. } => Some(*line),
            _ => None,
        };

        let result: Result<(), String> = (|| {
            match node {
                TemplateNode::Literal(s) => {
                    output.push_str(s);
                }
                TemplateNode::Output { expr, escaped, line: _ } => {
                    let value = evaluate_expr(expr, data)?;
                    let s = value_to_string(&value);
                    if *escaped {
                        output.push_str(&html_escape(&s));
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
                    let cond_value = evaluate_expr(condition, data)?;
                    if is_truthy(&cond_value) {
                        output.push_str(&render_nodes_with_path(body, data, partial_renderer, template_path)?);
                    } else if let Some(else_nodes) = else_body {
                        output.push_str(&render_nodes_with_path(else_nodes, data, partial_renderer, template_path)?);
                    }
                }
                TemplateNode::For {
                    var,
                    iterable,
                    body,
                    line: _,
                } => {
                    let iterable_value = evaluate_expr(iterable, data)?;
                    match &iterable_value {
                        Value::Array(arr) => {
                            for item in arr.borrow().iter() {
                                // Create new context with loop variable
                                let loop_data = with_variable(data, var, item.clone())?;
                                output.push_str(&render_nodes_with_path(body, &loop_data, partial_renderer, template_path)?);
                            }
                        }
                        Value::Hash(hash) => {
                            // Iterate over hash entries as [key, value] pairs
                            for (k, v) in hash.borrow().iter() {
                                let pair =
                                    Value::Array(Rc::new(RefCell::new(vec![k.clone(), v.clone()])));
                                let loop_data = with_variable(data, var, pair)?;
                                output.push_str(&render_nodes_with_path(body, &loop_data, partial_renderer, template_path)?);
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
                    // Yield is handled by the layout system, not here
                    // If we encounter it during normal rendering, it's an error
                    return Err("yield encountered outside of layout context".to_string());
                }
                TemplateNode::Partial { name, context, line: _ } => {
                    if let Some(renderer) = partial_renderer {
                        let partial_data = if let Some(ctx_expr) = context {
                            evaluate_expr(ctx_expr, data)?
                        } else {
                            data.clone()
                        };
                        output.push_str(&renderer(name, &partial_data)?);
                    } else {
                        return Err(format!("Partial rendering not available for '{}'", name));
                    }
                }
            }
            Ok(())
        })();

        // If there was an error, add template path and line context
        if let Err(e) = result {
            if let Some(path) = template_path {
                // Check if error already has path info
                if !e.contains(".html.erb") && !e.contains(".erb") {
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
/// This is the fast path - no string parsing required.
#[inline]
fn evaluate_expr(expr: &Expr, data: &Value) -> Result<Value, String> {
    match expr {
        Expr::StringLit(s) => Ok(Value::String(s.clone())),
        Expr::IntLit(n) => Ok(Value::Int(*n)),
        Expr::FloatLit(n) => Ok(Value::Float(*n)),
        Expr::BoolLit(b) => Ok(Value::Bool(*b)),
        Expr::Null => Ok(Value::Null),

        Expr::ArrayLit(elements) => {
            let values: Result<Vec<Value>, String> = elements
                .iter()
                .map(|e| evaluate_expr(e, data))
                .collect();
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

        Expr::Call(name, args) => {
            // Evaluate arguments
            let evaluated_args: Result<Vec<Value>, String> =
                args.iter().map(|arg| evaluate_expr(arg, data)).collect();

            let evaluated_args = evaluated_args?;

            // Look up the function in the data context (should be a NativeFunction)
            let func_value = get_hash_value(data, name)?;

            match func_value {
                Value::NativeFunction(nf) => {
                    // Call the native function
                    (nf.func)(evaluated_args)
                }
                Value::Null => Err(format!("'{}' is not defined (function not found in template context)", name)),
                _ => Err(format!("'{}' is not a function, got {}", name, func_value.type_name())),
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
            for (k, v) in hash.iter() {
                if k.hash_eq(key) {
                    return Ok(v.clone());
                }
            }
            // Try string key if the key is a string
            if let Value::String(key_str) = key {
                let str_key = Value::String(key_str.clone());
                for (k, v) in hash.iter() {
                    if k.hash_eq(&str_key) {
                        return Ok(v.clone());
                    }
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
            // Direct string comparison without allocating Value::String
            for (k, v) in hash.iter() {
                if let Value::String(k_str) = k {
                    if k_str == key {
                        // Auto-resolve Futures when retrieving values from template data
                        return resolve_if_future(v.clone());
                    }
                }
            }
            // Return null for missing keys instead of error
            Ok(Value::Null)
        }
        Value::Null => Ok(Value::Null),
        _ => Err(format!(
            "Cannot access '{}' on {}: expected Hash",
            key,
            value.type_name()
        )),
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
                let key = Value::String(name.to_string());

                // Find and update existing key, or append
                let mut found = false;
                for (k, v) in hash_ref.iter_mut() {
                    if let Value::String(k_str) = k {
                        if k_str == name {
                            *v = value.clone();
                            found = true;
                            break;
                        }
                    }
                }
                if !found {
                    hash_ref.push((key, value));
                }
                drop(hash_ref);
                Ok(data.clone())
            } else {
                // Multiple references - need to clone
                let mut new_hash: Vec<(Value, Value)> = hash.borrow().clone();
                let key = Value::String(name.to_string());

                // Find and update existing key, or append
                let mut found = false;
                for (k, v) in new_hash.iter_mut() {
                    if let Value::String(k_str) = k {
                        if k_str == name {
                            *v = value.clone();
                            found = true;
                            break;
                        }
                    }
                }
                if !found {
                    new_hash.push((key, value));
                }

                Ok(Value::Hash(Rc::new(RefCell::new(new_hash))))
            }
        }
        _ => Err("Data context must be a Hash".to_string()),
    }
}

/// Convert a Value to its string representation for output
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

/// Escape HTML special characters
pub fn html_escape(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => result.push_str("&amp;"),
            '<' => result.push_str("&lt;"),
            '>' => result.push_str("&gt;"),
            '"' => result.push_str("&quot;"),
            '\'' => result.push_str("&#x27;"),
            _ => result.push(c),
        }
    }
    result
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template::parser::compile_expr;

    fn make_hash(pairs: Vec<(&str, Value)>) -> Value {
        let hash: Vec<(Value, Value)> = pairs
            .into_iter()
            .map(|(k, v)| (Value::String(k.to_string()), v))
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
}
