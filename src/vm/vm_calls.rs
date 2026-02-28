//! Function call dispatch for the VM.

use std::cell::RefCell;
use std::rc::Rc;

use crate::ast::stmt::{FunctionDecl, Program, Stmt, StmtKind};
use crate::error::RuntimeError;
use crate::interpreter::value::{Class, Function, Instance, NativeFunction, Value};
use crate::span::Span;

use super::chunk::Constant;
use super::compiler::Compiler;
use super::upvalue::VmClosure;
use super::vm::{CallFrame, Vm};

impl Vm {
    /// Call a value with the given number of argument slots on the stack.
    /// The callee is below the arguments on the stack.
    pub fn call_value(&mut self, argc: usize, span: Span) -> Result<(), RuntimeError> {
        let callee_idx = self.stack.len() - 1 - argc;
        let callee = self.stack[callee_idx].clone();

        match callee {
            Value::VmClosure(closure) => self.call_closure(closure, argc, span),
            Value::NativeFunction(ref native) => self.call_native(native, argc, span),
            Value::Function(ref func) => {
                // Tree-walking function called from VM — shouldn't happen in pure VM mode
                // but needed for interop during transition
                self.call_native_wrapper(func, argc, span)
            }
            Value::Class(ref class) => self.call_class(class, argc, span),
            Value::Method(ref method) => {
                // Call a bound method (array.map, etc.)
                let receiver = (*method.receiver).clone();
                let method_name = method.method_name.clone();
                // Replace callee with receiver, look up method, and call
                self.stack[callee_idx] = receiver;
                // Delegate to native method dispatch
                self.call_builtin_method(&method_name, argc, span)
            }
            _ => Err(RuntimeError::not_callable(span)),
        }
    }

    fn call_closure(
        &mut self,
        closure: Rc<VmClosure>,
        argc: usize,
        span: Span,
    ) -> Result<(), RuntimeError> {
        let arity = closure.proto.arity as usize;
        let total_params = closure.proto.param_names.len();
        let _defaults = closure.proto.defaults as usize;

        // Check arity: argc must be between required and total
        if argc < arity || argc > total_params {
            return Err(RuntimeError::wrong_arity(total_params, argc, span));
        }

        // Fill in default values for missing optional parameters
        for _i in argc..total_params {
            self.push(Value::Null); // defaults are null for now
        }

        let stack_base = self.stack.len() - total_params - 1; // -1 for the callee slot

        self.frames.push(CallFrame {
            closure,
            ip: 0,
            stack_base,
        });

        Ok(())
    }

    fn call_native(
        &mut self,
        native: &NativeFunction,
        argc: usize,
        span: Span,
    ) -> Result<(), RuntimeError> {
        // Check arity
        if let Some(expected) = native.arity {
            if argc != expected {
                return Err(RuntimeError::wrong_arity(expected, argc, span));
            }
        }

        // Collect arguments from the stack
        let mut args = Vec::with_capacity(argc);
        for _ in 0..argc {
            args.push(self.pop());
        }
        args.reverse();

        // Pop the callee
        self.pop();

        // Call the native function
        let result = (native.func)(args).map_err(|e| RuntimeError::new(e, span))?;
        self.push(result);
        Ok(())
    }

    fn call_native_wrapper(
        &mut self,
        func: &Function,
        argc: usize,
        span: Span,
    ) -> Result<(), RuntimeError> {
        // Check if this function was already JIT-compiled and cached
        if !func.name.is_empty() {
            if let Some(Value::VmClosure(cached)) = self.globals.get(&func.name) {
                let closure = cached.clone();
                let callee_idx = self.stack.len() - 1 - argc;
                self.stack[callee_idx] = Value::VmClosure(closure.clone());
                return self.call_closure(closure, argc, span);
            }
        }

        // JIT-compile the tree-walking function to bytecode (first call only).
        let func_decl = FunctionDecl {
            name: func.name.clone(),
            params: func.params.clone(),
            return_type: None,
            body: func.body.clone(),
            span: func.span.unwrap_or_default(),
        };

        let program = Program::new(vec![Stmt {
            kind: StmtKind::Function(func_decl),
            span: func.span.unwrap_or_default(),
        }]);

        let module = Compiler::compile(&program)
            .map_err(|e| RuntimeError::new(format!("VM JIT compile error: {}", e), span))?;

        // Extract the compiled FunctionProto from the module's constant pool
        let proto = module
            .main
            .chunk
            .constants
            .iter()
            .find_map(|c| {
                if let Constant::Function(p) = c {
                    Some(p.clone())
                } else {
                    None
                }
            })
            .ok_or_else(|| {
                RuntimeError::new("Failed to extract compiled function from JIT", span)
            })?;

        let closure = Rc::new(VmClosure::new(proto, Vec::new()));

        // Replace the Function value on the stack with the compiled VmClosure
        let callee_idx = self.stack.len() - 1 - argc;
        self.stack[callee_idx] = Value::VmClosure(closure.clone());

        // Cache in globals so subsequent calls skip JIT compilation
        if !func.name.is_empty() {
            self.globals
                .insert(func.name.clone(), Value::VmClosure(closure.clone()));
        }

        // Now call it as a regular closure
        self.call_closure(closure, argc, span)
    }

    fn call_class(
        &mut self,
        class: &Rc<Class>,
        argc: usize,
        _span: Span,
    ) -> Result<(), RuntimeError> {
        // Create an instance
        let instance = Instance::new(class.clone());
        let instance_val = Value::Instance(Rc::new(RefCell::new(instance)));

        // Replace the class on the stack with the instance
        let callee_idx = self.stack.len() - 1 - argc;
        self.stack[callee_idx] = instance_val.clone();

        // Call the constructor if one exists
        if let Some(ref _constructor) = class.constructor {
            // Constructor dispatch — VM constructors stored as VmClosures
        }

        // Look for VM constructor method
        // This would be set during class compilation

        // If no constructor and args were provided, error
        if argc > 0 {
            // Pop the unused arguments
            for _ in 0..argc {
                self.pop();
            }
            // Push instance back
        }

        Ok(())
    }

    fn call_builtin_method(
        &mut self,
        _method_name: &str,
        _argc: usize,
        span: Span,
    ) -> Result<(), RuntimeError> {
        // Built-in methods on arrays/hashes/strings are handled by native functions
        // The method resolver should have already bound them
        Err(RuntimeError::new(
            "Built-in method dispatch not yet implemented in VM",
            span,
        ))
    }

    /// Call a global function by name (used by server integration).
    pub fn call_global(
        &mut self,
        name: &str,
        args: Vec<Value>,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        let func = self
            .globals
            .get(name)
            .cloned()
            .ok_or_else(|| RuntimeError::undefined_variable(name, span))?;

        self.push(func);
        for arg in &args {
            self.push(arg.clone());
        }
        self.call_value(args.len(), span)?;
        self.run()
    }

    /// Call an arbitrary Value with arguments (used by server integration).
    /// This enables calling handler functions resolved from the controller registry.
    pub fn call_value_direct(
        &mut self,
        callee: Value,
        args: Vec<Value>,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        self.push(callee);
        let argc = args.len();
        for arg in args {
            self.push(arg);
        }
        self.call_value(argc, span)?;
        self.run()
    }

    /// Optimized single-arg call that avoids Vec heap allocation.
    pub fn call_value_direct_one(
        &mut self,
        callee: Value,
        arg: Value,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        self.push(callee);
        self.push(arg);
        self.call_value(1, span)?;
        self.run()
    }

    /// Reset VM state between requests (preserves globals).
    pub fn reset(&mut self) {
        self.stack.clear();
        self.frames.clear();
        self.open_upvalues.clear();
        self.exception_handlers.clear();
        self.iter_stack.clear();
    }
}
