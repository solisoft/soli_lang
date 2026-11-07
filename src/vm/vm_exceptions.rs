//! Exception handling for the VM: throw, catch, finally unwinding.

use crate::error::RuntimeError;
use crate::interpreter::value::Value;
use crate::span::Span;

use super::vm::Vm;

impl Vm {
    /// Throw an exception value, unwinding the stack to the nearest catch handler.
    pub fn throw_exception(&mut self, value: Value, span: Span) -> Result<(), RuntimeError> {
        // Look for an exception handler, but do not unwind past the current
        // `return_depth` — handlers at a shallower frame depth belong to an
        // outer native invocation (e.g. array.map) that needs the exception
        // to surface as a Rust `Err` so it can clean up its own state.
        while let Some(handler) = self.exception_handlers.last() {
            if handler.frame_depth <= self.return_depth {
                break;
            }
            let handler = self.exception_handlers.pop().unwrap();

            // Unwind call frames
            let mut close_from = handler.stack_depth;
            while self.frames.len() > handler.frame_depth {
                let frame = self.frames.pop().unwrap();
                close_from = close_from.min(frame.stack_base);
            }
            self.debug_left();

            // Close the cells of every variable the unwinding drops — the
            // popped frames', and the `try` body's own locals above the
            // handler's depth — while their slots still hold them. Only the
            // frames' used to be closed: a closure made in the `try` body
            // kept a cell open on a slot the truncation below gave back, and
            // read whatever the catch block or the code after it put there.
            self.close_upvalues(close_from);

            // Unwind the stack
            self.stack.truncate(handler.stack_depth);
            // …and any iterators the abandoned loops left behind.
            self.iter_stack.truncate(handler.iter_depth);

            // Push the exception value for the catch block
            self.push(value.clone());

            // Jump to the catch handler
            if let Some(frame) = self.frames.last_mut() {
                frame.ip = handler.catch_ip;
                return Ok(());
            }
        }

        // No handler found *here* — but "here" includes the case where a
        // native driver (array.map and friends) is between the throw and the
        // handler, because those handlers are gated off by `return_depth` so
        // the driver can unwind its own state first. The error then travels
        // back through `run`, which re-throws it at the outer handler.
        //
        // So this must keep the value, not a rendering of it: flattening here
        // is what made `[1].map(fn(x) { throw {"code": 404} })` arrive at the
        // caller's `catch` as a string.
        Err(RuntimeError::Thrown { value, span })
    }
}
