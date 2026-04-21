//! Int method call implementations.

use crate::error::RuntimeError;
use crate::interpreter::environment::Environment;
use crate::interpreter::executor::{ControlFlow, Interpreter, RuntimeResult};
use crate::interpreter::value::Value;
use crate::span::Span;

impl Interpreter {
    /// Handle int methods that require arguments.
    pub(crate) fn call_int_method(
        &mut self,
        n: i64,
        method_name: &str,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        match method_name {
            "times" => self.int_times(n, arguments, span),
            "upto" => self.int_upto(n, arguments, span),
            "downto" => self.int_downto(n, arguments, span),
            "pow" => self.int_pow(n, arguments, span),
            "gcd" => self.int_gcd(n, arguments, span),
            "lcm" => self.int_lcm(n, arguments, span),
            "between?" => self.int_between(n, arguments, span),
            "clamp" => self.int_clamp(n, arguments, span),
            "is_a?" => self.int_is_a(arguments, span),
            _ => Err(RuntimeError::NoSuchProperty {
                value_type: "int".to_string(),
                property: method_name.to_string(),
                span,
            }),
        }
    }

    fn int_times(&mut self, n: i64, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let func = match &arguments[0] {
            Value::Function(f) => f.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "times expects a function argument",
                    span,
                ))
            }
        };

        let param_name = func
            .params
            .first()
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "it".to_string());

        for i in 0..n {
            let mut call_env = Environment::with_enclosing(func.closure.clone());
            call_env.define(param_name.clone(), Value::Int(i));

            match self.execute_block(&func.body, call_env)? {
                ControlFlow::Return(_) | ControlFlow::Normal(_) | ControlFlow::Continue => {}
                ControlFlow::Throw(_) => {
                    return Err(RuntimeError::new("Exception in int.times", span));
                }
            }
        }

        Ok(Value::Int(n))
    }

    fn int_upto(&mut self, n: i64, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if arguments.len() != 2 {
            return Err(RuntimeError::wrong_arity(2, arguments.len(), span));
        }
        let limit = match &arguments[0] {
            Value::Int(m) => *m,
            _ => {
                return Err(RuntimeError::type_error(
                    "upto expects an integer limit",
                    span,
                ))
            }
        };
        let func = match &arguments[1] {
            Value::Function(f) => f.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "upto expects a function argument",
                    span,
                ))
            }
        };

        let param_name = func
            .params
            .first()
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "it".to_string());

        for i in n..=limit {
            let mut call_env = Environment::with_enclosing(func.closure.clone());
            call_env.define(param_name.clone(), Value::Int(i));

            match self.execute_block(&func.body, call_env)? {
                ControlFlow::Return(_) | ControlFlow::Normal(_) | ControlFlow::Continue => {}
                ControlFlow::Throw(_) => {
                    return Err(RuntimeError::new("Exception in int.upto", span));
                }
            }
        }

        Ok(Value::Int(n))
    }

    fn int_downto(&mut self, n: i64, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if arguments.len() != 2 {
            return Err(RuntimeError::wrong_arity(2, arguments.len(), span));
        }
        let limit = match &arguments[0] {
            Value::Int(m) => *m,
            _ => {
                return Err(RuntimeError::type_error(
                    "downto expects an integer limit",
                    span,
                ))
            }
        };
        let func = match &arguments[1] {
            Value::Function(f) => f.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "downto expects a function argument",
                    span,
                ))
            }
        };

        let param_name = func
            .params
            .first()
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "it".to_string());

        let mut i = n;
        while i >= limit {
            let mut call_env = Environment::with_enclosing(func.closure.clone());
            call_env.define(param_name.clone(), Value::Int(i));

            match self.execute_block(&func.body, call_env)? {
                ControlFlow::Return(_) | ControlFlow::Normal(_) | ControlFlow::Continue => {}
                ControlFlow::Throw(_) => {
                    return Err(RuntimeError::new("Exception in int.downto", span));
                }
            }
            i -= 1;
        }

        Ok(Value::Int(n))
    }

    fn int_pow(&self, n: i64, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        match &arguments[0] {
            Value::Int(exp) => {
                if *exp < 0 {
                    Ok(Value::Float((n as f64).powi(*exp as i32)))
                } else {
                    Ok(Value::Int(n.pow(*exp as u32)))
                }
            }
            Value::Float(exp) => Ok(Value::Float((n as f64).powf(*exp))),
            _ => Err(RuntimeError::type_error(
                "pow expects a numeric argument",
                span,
            )),
        }
    }

    fn int_gcd(&self, n: i64, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let other = match &arguments[0] {
            Value::Int(m) => *m,
            _ => {
                return Err(RuntimeError::type_error(
                    "gcd expects an integer argument",
                    span,
                ))
            }
        };

        fn gcd(mut a: i64, mut b: i64) -> i64 {
            a = a.abs();
            b = b.abs();
            while b != 0 {
                let t = b;
                b = a % b;
                a = t;
            }
            a
        }

        Ok(Value::Int(gcd(n, other)))
    }

    fn int_lcm(&self, n: i64, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let other = match &arguments[0] {
            Value::Int(m) => *m,
            _ => {
                return Err(RuntimeError::type_error(
                    "lcm expects an integer argument",
                    span,
                ))
            }
        };

        fn gcd(mut a: i64, mut b: i64) -> i64 {
            a = a.abs();
            b = b.abs();
            while b != 0 {
                let t = b;
                b = a % b;
                a = t;
            }
            a
        }

        if n == 0 && other == 0 {
            Ok(Value::Int(0))
        } else {
            Ok(Value::Int((n / gcd(n, other) * other).abs()))
        }
    }

    fn int_between(&self, n: i64, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if arguments.len() != 2 {
            return Err(RuntimeError::wrong_arity(2, arguments.len(), span));
        }
        let min = match &arguments[0] {
            Value::Int(m) => *m as f64,
            Value::Float(m) => *m,
            _ => {
                return Err(RuntimeError::type_error(
                    "between? expects numeric arguments",
                    span,
                ))
            }
        };
        let max = match &arguments[1] {
            Value::Int(m) => *m as f64,
            Value::Float(m) => *m,
            _ => {
                return Err(RuntimeError::type_error(
                    "between? expects numeric arguments",
                    span,
                ))
            }
        };
        Ok(Value::Bool((n as f64) >= min && (n as f64) <= max))
    }

    fn int_clamp(&self, n: i64, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if arguments.len() != 2 {
            return Err(RuntimeError::wrong_arity(2, arguments.len(), span));
        }
        let min = match &arguments[0] {
            Value::Int(m) => *m,
            _ => {
                return Err(RuntimeError::type_error(
                    "clamp expects integer arguments",
                    span,
                ))
            }
        };
        let max = match &arguments[1] {
            Value::Int(m) => *m,
            _ => {
                return Err(RuntimeError::type_error(
                    "clamp expects integer arguments",
                    span,
                ))
            }
        };
        Ok(Value::Int(n.max(min).min(max)))
    }

    fn int_is_a(&self, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let class_name = match &arguments[0] {
            Value::String(s) => s.as_str(),
            _ => {
                return Err(RuntimeError::type_error(
                    "is_a? expects a string argument",
                    span,
                ))
            }
        };
        Ok(Value::Bool(
            class_name == "int" || class_name == "numeric" || class_name == "object",
        ))
    }
}
