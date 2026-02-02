//! Lambda/anonymous function evaluation.

use std::rc::Rc;

use crate::ast::Parameter;
use crate::ast::Stmt;
use crate::interpreter::executor::{Interpreter, RuntimeResult};
use crate::interpreter::value::{Function, Value};
use crate::span::Span;

impl Interpreter {
    /// Evaluate lambda expression.
    pub(crate) fn evaluate_lambda(
        &mut self,
        params: &[Parameter],
        body: &[Stmt],
        span: Span,
    ) -> RuntimeResult<Value> {
        let func = Function {
            name: "<lambda>".to_string(),
            params: params.to_vec(),
            body: body.to_vec(),
            closure: self.environment.clone(),
            is_method: false,
            span: Some(span),
            source_path: self
                .current_source_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string()),
            defining_superclass: None,
        };
        Ok(Value::Function(Rc::new(func)))
    }
}
