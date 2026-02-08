//! Exception handling for the VM: throw, catch, finally unwinding.

use crate::error::RuntimeError;
use crate::interpreter::value::Value;
use crate::span::Span;

use super::vm::Vm;

impl Vm {
    /// Throw an exception value, unwinding the stack to the nearest catch handler.
    pub fn throw_exception(&mut self, value: Value, span: Span) -> Result<(), RuntimeError> {
        // Look for an exception handler
        while let Some(handler) = self.exception_handlers.pop() {
            // Unwind call frames
            while self.frames.len() > handler.frame_depth {
                let frame = self.frames.pop().unwrap();
                self.close_upvalues(frame.stack_base);
            }

            // Unwind the stack
            self.stack.truncate(handler.stack_depth);

            // Push the exception value for the catch block
            self.push(value.clone());

            // Jump to the catch handler
            if let Some(frame) = self.frames.last_mut() {
                frame.ip = handler.catch_ip;
                return Ok(());
            }
        }

        // No handler found — convert to a RuntimeError
        let message = match &value {
            Value::String(s) => s.clone(),
            Value::Instance(inst) => {
                let inst = inst.borrow();
                if let Some(msg) = inst.fields.get("message") {
                    format!("{}", msg)
                } else {
                    format!("<{} instance>", inst.class.name)
                }
            }
            other => format!("{}", other),
        };

        Err(RuntimeError::new(message, span))
    }
}
