//! Core language expression evaluation for templates.
//!
//! This module provides expression evaluation using the core language's
//! interpreter, giving templates access to all builtins.
//!
//! Optimizations:
//! - Direct AST translation: template Expr → core ExprKind (no string round-trip)
//! - Shared builtins: thread-local Rc<RefCell<Environment>> avoids cloning builtins
//! - One interpreter per render: created once, reused for all expressions

use std::cell::RefCell;
use std::rc::Rc;

use crate::ast::expr::{self as core_expr, Argument, BinaryOp, ExprKind, UnaryOp};
use crate::interpreter::environment::Environment;
use crate::interpreter::executor::Interpreter;
use crate::interpreter::value::{StrKey, Value};
use crate::span::Span;
use crate::template::parser::{self as tpl, Expr};

// ---------------------------------------------------------------------------
// Thread-local shared builtins environment (Rc, not cloned)
// ---------------------------------------------------------------------------

thread_local! {
    /// Shared builtins environment. Uses Rc so child scopes can reference it
    /// without cloning the entire HashMap of builtins.
    static BUILTINS_RC: RefCell<Option<Rc<RefCell<Environment>>>> = const { RefCell::new(None) };
}

/// Get the shared builtins environment Rc.
fn get_builtins_rc() -> Rc<RefCell<Environment>> {
    BUILTINS_RC.with(|cell| {
        let mut opt = cell.borrow_mut();
        if opt.is_none() {
            let mut env = Environment::new();
            crate::interpreter::builtins::register_builtins(&mut env);
            crate::interpreter::builtins::template::register_static_template_helpers(&mut env);
            *opt = Some(Rc::new(RefCell::new(env)));
        }
        opt.as_ref().unwrap().clone()
    })
}

// ---------------------------------------------------------------------------
// Interpreter lifecycle for template rendering
// ---------------------------------------------------------------------------

/// Create a template interpreter populated with data.
/// Uses shared builtins (no clone) + data hash reference (no copy).
/// Data variables are looked up directly in the hash via zero-alloc StrKey.
pub fn create_template_interpreter(data: &Value) -> Interpreter {
    let builtins = get_builtins_rc();
    let data_env = if let Value::Hash(map) = data {
        Environment::with_enclosing_and_data(builtins, map.clone())
    } else {
        Environment::with_enclosing(builtins)
    };
    Interpreter::with_environment(Rc::new(RefCell::new(data_env)))
}

/// Push a new child scope on the interpreter's environment.
/// Used for loop bodies so loop vars don't leak to outer scope.
#[inline]
pub fn push_scope(interpreter: &mut Interpreter) {
    let old_env = interpreter.environment.clone();
    let new_env = Environment::with_enclosing(old_env);
    interpreter.environment = Rc::new(RefCell::new(new_env));
}

/// Pop back to the enclosing scope.
#[inline]
pub fn pop_scope(interpreter: &mut Interpreter) {
    let enclosing = interpreter
        .environment
        .borrow()
        .enclosing()
        .expect("pop_scope called without enclosing scope");
    interpreter.environment = enclosing;
}

/// Define a variable in the interpreter's current scope.
/// Uses define_or_update to avoid String key allocation when variable already exists
/// (common in for-loops where the same variable is redefined every iteration).
#[inline]
pub fn define_var(interpreter: &mut Interpreter, name: &str, value: Value) {
    interpreter
        .environment
        .borrow_mut()
        .define_or_update(name, value);
}

/// Evaluate a template expression using an existing interpreter.
/// Fast paths bypass translate_expr → core Expr allocation → evaluate dispatch
/// for the most common template patterns (variable lookup, field access, literals).
#[inline]
pub fn evaluate_with_interpreter(
    expr: &Expr,
    interpreter: &mut Interpreter,
) -> Result<Value, String> {
    match expr {
        // Fast path: direct variable lookup (e.g., <%= title %>)
        // Bypasses: String clone + ExprKind alloc + match dispatch
        Expr::Var(name) => {
            return interpreter
                .environment
                .borrow()
                .get(name)
                .ok_or_else(|| format!("Evaluation error: undefined variable '{}'", name));
        }
        // Fast path: hash field access (e.g., <%= user.name %>)
        // Zero-alloc StrKey lookup; returns Null for missing keys on hashes.
        // Only falls through to full evaluate for non-hash bases (e.g., method calls on arrays/strings).
        Expr::Field(base, field) => {
            let base_val = evaluate_with_interpreter(base, interpreter)?;
            if let Value::Hash(ref hash) = base_val {
                return match hash.borrow().get(&StrKey(field)) {
                    Some(v) => Ok(v.clone()),
                    None => Ok(Value::Null),
                };
            }
            // Non-hash: fall through to full evaluate for methods
        }
        // Fast path: literals (no allocation except StringLit clone)
        Expr::IntLit(n) => return Ok(Value::Int(*n)),
        Expr::FloatLit(n) => return Ok(Value::Float(*n)),
        Expr::BoolLit(b) => return Ok(Value::Bool(*b)),
        Expr::Null => return Ok(Value::Null),
        Expr::StringLit(s) => return Ok(Value::String(s.clone())),
        // Fast path: common no-arg method calls (e.g., items.length, name.upcase)
        // Bypasses: translate_expr Box allocations + evaluate dispatch + method resolution
        Expr::MethodCall { base, method, args } if args.is_empty() => {
            let base_val = evaluate_with_interpreter(base, interpreter)?;
            match (&base_val, method.as_str()) {
                (Value::String(s), "length" | "len") => return Ok(Value::Int(s.len() as i64)),
                (Value::String(s), "upcase" | "uppercase") => return Ok(Value::String(s.to_uppercase())),
                (Value::String(s), "downcase" | "lowercase") => return Ok(Value::String(s.to_lowercase())),
                (Value::String(s), "strip" | "trim") => return Ok(Value::String(s.trim().to_string())),
                (Value::String(s), "empty?") => return Ok(Value::Bool(s.is_empty())),
                (Value::String(s), "reverse") => return Ok(Value::String(s.chars().rev().collect())),
                (Value::String(s), "to_i") => return Ok(Value::Int(s.parse::<i64>().unwrap_or(0))),
                (Value::Array(arr), "length" | "len") => return Ok(Value::Int(arr.borrow().len() as i64)),
                (Value::Array(arr), "empty?") => return Ok(Value::Bool(arr.borrow().is_empty())),
                (Value::Array(arr), "first") => return Ok(arr.borrow().first().cloned().unwrap_or(Value::Null)),
                (Value::Array(arr), "last") => return Ok(arr.borrow().last().cloned().unwrap_or(Value::Null)),
                (Value::Hash(h), "length" | "len") => return Ok(Value::Int(h.borrow().len() as i64)),
                (Value::Hash(h), "empty?") => return Ok(Value::Bool(h.borrow().is_empty())),
                (Value::Hash(h), "keys") => {
                    let keys: Vec<Value> = h.borrow().keys().map(|k| k.to_value()).collect();
                    return Ok(Value::Array(Rc::new(RefCell::new(keys))));
                }
                (Value::Hash(h), "values") => {
                    let vals: Vec<Value> = h.borrow().values().cloned().collect();
                    return Ok(Value::Array(Rc::new(RefCell::new(vals))));
                }
                (Value::Int(_) | Value::Float(_), "to_s" | "to_string") => {
                    return Ok(Value::String(format!("{}", base_val)));
                }
                _ => {} // Fall through to full evaluate
            }
        }
        _ => {}
    }
    // Full path for complex expressions
    let core_kind = translate_expr(expr);
    let core_expr = core_expr::Expr::new(core_kind, Span::default());
    interpreter
        .evaluate(&core_expr)
        .map_err(|e| format!("Evaluation error: {}", e))
}

// ---------------------------------------------------------------------------
// Fallback: standalone evaluate (creates interpreter per call)
// ---------------------------------------------------------------------------

/// Evaluate a template expression with the given data context.
/// Creates a new interpreter per call. Prefer create_template_interpreter +
/// evaluate_with_interpreter for rendering multiple expressions.
pub fn evaluate_expression(expr: &Expr, data: &Value) -> Result<Value, String> {
    let mut interpreter = create_template_interpreter(data);
    evaluate_with_interpreter(expr, &mut interpreter)
}

// ---------------------------------------------------------------------------
// Direct AST translation: template Expr → core ExprKind
// ---------------------------------------------------------------------------

/// Helper to wrap an ExprKind into a boxed core Expr with a default span.
#[inline]
fn boxed(kind: ExprKind) -> Box<core_expr::Expr> {
    Box::new(core_expr::Expr::new(kind, Span::default()))
}

/// Translate a template expression directly into a core language ExprKind.
/// This avoids the expensive to_source() → lex → parse round-trip.
pub fn translate_expr(expr: &Expr) -> ExprKind {
    match expr {
        Expr::StringLit(s) => ExprKind::StringLiteral(s.clone()),
        Expr::IntLit(n) => ExprKind::IntLiteral(*n),
        Expr::FloatLit(n) => ExprKind::FloatLiteral(*n),
        Expr::BoolLit(b) => ExprKind::BoolLiteral(*b),
        Expr::Null => ExprKind::Null,

        Expr::ArrayLit(elements) => ExprKind::Array(
            elements
                .iter()
                .map(|e| core_expr::Expr::new(translate_expr(e), Span::default()))
                .collect(),
        ),

        Expr::Var(name) => ExprKind::Variable(name.clone()),

        Expr::Field(base, field) => ExprKind::Member {
            object: boxed(translate_expr(base)),
            name: field.clone(),
        },

        Expr::Index(base, key) => ExprKind::Index {
            object: boxed(translate_expr(base)),
            index: boxed(translate_expr(key)),
        },

        Expr::Binary(left, op, right) => {
            let core_op = match op {
                tpl::BinaryOp::Add => BinaryOp::Add,
                tpl::BinaryOp::Subtract => BinaryOp::Subtract,
                tpl::BinaryOp::Multiply => BinaryOp::Multiply,
                tpl::BinaryOp::Divide => BinaryOp::Divide,
                tpl::BinaryOp::Modulo => BinaryOp::Modulo,
            };
            ExprKind::Binary {
                left: boxed(translate_expr(left)),
                operator: core_op,
                right: boxed(translate_expr(right)),
            }
        }

        Expr::Compare(left, op, right) => {
            let core_op = match op {
                tpl::CompareOp::Eq => BinaryOp::Equal,
                tpl::CompareOp::Ne => BinaryOp::NotEqual,
                tpl::CompareOp::Lt => BinaryOp::Less,
                tpl::CompareOp::Le => BinaryOp::LessEqual,
                tpl::CompareOp::Gt => BinaryOp::Greater,
                tpl::CompareOp::Ge => BinaryOp::GreaterEqual,
            };
            ExprKind::Binary {
                left: boxed(translate_expr(left)),
                operator: core_op,
                right: boxed(translate_expr(right)),
            }
        }

        Expr::And(left, right) => ExprKind::LogicalAnd {
            left: boxed(translate_expr(left)),
            right: boxed(translate_expr(right)),
        },

        Expr::Or(left, right) => ExprKind::LogicalOr {
            left: boxed(translate_expr(left)),
            right: boxed(translate_expr(right)),
        },

        Expr::Not(inner) => ExprKind::Unary {
            operator: UnaryOp::Not,
            operand: boxed(translate_expr(inner)),
        },

        Expr::Method(base, method) => ExprKind::Member {
            object: boxed(translate_expr(base)),
            name: method.clone(),
        },

        Expr::MethodCall { base, method, args } => ExprKind::Call {
            callee: boxed(ExprKind::Member {
                object: boxed(translate_expr(base)),
                name: method.clone(),
            }),
            arguments: args
                .iter()
                .map(|a| {
                    Argument::Positional(core_expr::Expr::new(translate_expr(a), Span::default()))
                })
                .collect(),
        },

        Expr::Call(name, args) => ExprKind::Call {
            callee: boxed(ExprKind::Variable(name.clone())),
            arguments: args
                .iter()
                .map(|a| {
                    Argument::Positional(core_expr::Expr::new(translate_expr(a), Span::default()))
                })
                .collect(),
        },

        Expr::Assign(name, value) => ExprKind::Assign {
            target: boxed(ExprKind::Variable(name.clone())),
            value: boxed(translate_expr(value)),
        },

        Expr::Range(start, end) => ExprKind::Binary {
            left: boxed(translate_expr(start)),
            operator: BinaryOp::Range,
            right: boxed(translate_expr(end)),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::value::HashKey;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn make_hash(pairs: Vec<(&str, Value)>) -> Value {
        let mut map = indexmap::IndexMap::new();
        for (k, v) in pairs {
            map.insert(HashKey::String(k.to_string()), v);
        }
        Value::Hash(Rc::new(RefCell::new(map)))
    }

    #[test]
    fn test_translate_int_literal() {
        let expr = Expr::IntLit(42);
        let core = translate_expr(&expr);
        assert!(matches!(core, ExprKind::IntLiteral(42)));
    }

    #[test]
    fn test_translate_string_literal() {
        let expr = Expr::StringLit("hello".to_string());
        let core = translate_expr(&expr);
        assert!(matches!(core, ExprKind::StringLiteral(s) if s == "hello"));
    }

    #[test]
    fn test_translate_variable() {
        let expr = Expr::Field(
            Box::new(Expr::Var("user".to_string())),
            "name".to_string(),
        );
        let core = translate_expr(&expr);
        assert!(matches!(core, ExprKind::Member { .. }));
    }

    #[test]
    fn test_evaluate_with_context() {
        let data = make_hash(vec![("name", Value::String("World".to_string()))]);
        let expr = Expr::Binary(
            Box::new(Expr::StringLit("Hello ".to_string())),
            tpl::BinaryOp::Add,
            Box::new(Expr::Var("name".to_string())),
        );
        let result = evaluate_expression(&expr, &data);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Value::String("Hello World".to_string()));
    }

    #[test]
    fn test_evaluate_nested_hash_access() {
        let user = make_hash(vec![("name", Value::String("Alice".to_string()))]);
        let data = make_hash(vec![("user", user)]);
        let expr = Expr::Field(
            Box::new(Expr::Var("user".to_string())),
            "name".to_string(),
        );
        let result = evaluate_expression(&expr, &data);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Value::String("Alice".to_string()));
    }

    #[test]
    fn test_reuse_interpreter() {
        let data = make_hash(vec![
            ("x", Value::Int(10)),
            ("y", Value::Int(20)),
        ]);
        let mut interp = create_template_interpreter(&data);

        let expr1 = Expr::Var("x".to_string());
        let r1 = evaluate_with_interpreter(&expr1, &mut interp).unwrap();
        assert_eq!(r1, Value::Int(10));

        let expr2 = Expr::Var("y".to_string());
        let r2 = evaluate_with_interpreter(&expr2, &mut interp).unwrap();
        assert_eq!(r2, Value::Int(20));
    }

    #[test]
    fn test_scope_push_pop() {
        let data = make_hash(vec![("x", Value::Int(1))]);
        let mut interp = create_template_interpreter(&data);

        push_scope(&mut interp);
        define_var(&mut interp, "x", Value::Int(99));
        let r = evaluate_with_interpreter(&Expr::Var("x".to_string()), &mut interp).unwrap();
        assert_eq!(r, Value::Int(99));
        pop_scope(&mut interp);

        let r = evaluate_with_interpreter(&Expr::Var("x".to_string()), &mut interp).unwrap();
        assert_eq!(r, Value::Int(1));
    }
}
