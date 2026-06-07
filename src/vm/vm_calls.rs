//! Function call dispatch for the VM.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use crate::ast::stmt::{FunctionDecl, Program, Stmt, StmtKind};
use crate::error::RuntimeError;
use crate::interpreter::value::{Class, Function, Instance, NativeFunction, Value};
use crate::span::Span;

use super::chunk::{Constant, FunctionProto};
use super::compiler::Compiler;
use super::upvalue::VmClosure;
use super::vm::{CallFrame, Vm};

/// JIT-compile a tree-walking [`Function`] to a bytecode [`FunctionProto`] and
/// cache it in `func.jit_cache`. Returns the cached proto on a hit, otherwise
/// compiles, stores, and returns it. Pure compilation — no execution, no side
/// effects — so it is safe to call ahead of time to warm a worker's handlers.
pub(crate) fn jit_compile_function<I: IntoIterator<Item = String>>(
    func: &Function,
    globals: I,
) -> Result<Arc<FunctionProto>, String> {
    if let Some(proto) = func.jit_cache.borrow().clone() {
        return Ok(proto);
    }

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
        source_path: None,
    }]);

    let module = Compiler::compile_with_globals(&program, globals).map_err(|e| e.to_string())?;

    // Extract the compiled FunctionProto from the module's constant pool.
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
        .ok_or_else(|| "Failed to extract compiled function from JIT".to_string())?;

    *func.jit_cache.borrow_mut() = Some(proto.clone());
    Ok(proto)
}

/// JIT-compile a class method (`FunctionType::Method`, slot 0 reserved for
/// `this`) to a bytecode [`FunctionProto`] and cache it in `func.jit_cache`.
///
/// Sibling of [`jit_compile_function`], used by the worker warmup pass for
/// OOP controller actions so the first request to a method is a cache hit
/// instead of paying the full AST-to-bytecode walk in [`Vm::call_method_bound`].
pub(crate) fn jit_compile_method<I: IntoIterator<Item = String>>(
    func: &Function,
    globals: I,
) -> Result<Arc<FunctionProto>, String> {
    if let Some(proto) = func.jit_cache.borrow().clone() {
        return Ok(proto);
    }
    let proto = Compiler::compile_method_standalone(func, globals).map_err(|e| e.to_string())?;
    let arc = std::sync::Arc::new(proto);
    *func.jit_cache.borrow_mut() = Some(arc.clone());
    Ok(arc)
}

impl Vm {
    /// Call a value with the given number of argument slots on the stack.
    /// The callee is below the arguments on the stack.
    #[inline]
    pub fn call_value(&mut self, argc: usize, span: Span) -> Result<(), RuntimeError> {
        let callee_idx = self.stack.len() - 1 - argc;

        // Fast path: check for VmClosure without cloning (most common case)
        if let Value::VmClosure(closure) = &self.stack[callee_idx] {
            let closure = closure.clone(); // Rc clone (cheap counter increment)
            return self.call_closure(closure, argc, span);
        }

        // Slow path: clone and dispatch other types
        let callee = self.stack[callee_idx].clone();
        match callee {
            Value::NativeFunction(ref native) => self.call_native(native, argc, span),
            Value::Function(ref func) => self.call_native_wrapper(func, argc, span),
            Value::Class(ref class) => self.call_class(class, argc, span),
            Value::Method(ref method) => {
                let receiver = (*method.receiver).clone();
                let method_name = method.method_name.clone();
                self.stack[callee_idx] = receiver;
                self.call_builtin_method(&method_name, argc, span)
            }
            _ => Err(RuntimeError::not_callable(span)),
        }
    }

    #[inline]
    pub(crate) fn call_closure(
        &mut self,
        closure: Rc<VmClosure>,
        argc: usize,
        span: Span,
    ) -> Result<(), RuntimeError> {
        self.call_closure_in_class(closure, argc, span, None)
    }

    /// Like `call_closure`, but records the class that defines the method on
    /// the frame so `super` inside it can resolve against the defining
    /// class's superclass.
    #[inline]
    pub(crate) fn call_closure_in_class(
        &mut self,
        closure: Rc<VmClosure>,
        argc: usize,
        span: Span,
        class: Option<Rc<Class>>,
    ) -> Result<(), RuntimeError> {
        let arity = closure.proto.arity as usize;
        let total_params = closure.proto.param_names.len();

        // Check arity: argc must be between required and total
        if argc < arity || argc > total_params {
            return Err(RuntimeError::wrong_arity(total_params, argc, span));
        }

        // Fill in default values for missing optional parameters
        if argc < total_params {
            for _ in argc..total_params {
                self.push(Value::Null);
            }
        }

        let stack_base = self.stack.len() - total_params - 1;

        self.frames.push(CallFrame {
            closure,
            ip: 0,
            stack_base,
            class,
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

        // Call the native function. Wrap in a flamegraph `Fn` span when
        // the native is on the request-path whitelist (see
        // `span_log::is_request_path_native`); cheap builtins are
        // skipped to keep the chart readable.
        let _native_span = crate::serve::span_log::maybe_instrument_native(&native.name);
        let result = (native.func)(args).map_err(|e| RuntimeError::new(e, span))?;
        drop(_native_span);
        self.push(result);
        Ok(())
    }

    fn call_native_wrapper(
        &mut self,
        func: &Function,
        argc: usize,
        span: Span,
    ) -> Result<(), RuntimeError> {
        // JIT-compile (or reuse the cached bytecode for) the tree-walking
        // function. `jit_compile_function` returns the cached proto on a hit
        // and compiles+caches on the first call.
        let proto = jit_compile_function(func, self.globals.keys().cloned())
            .map_err(|e| RuntimeError::new(format!("VM JIT compile error: {}", e), span))?;

        let closure = Rc::new(VmClosure::new(proto, Vec::new()));

        // Replace the Function value on the stack with the compiled VmClosure
        let callee_idx = self.stack.len() - 1 - argc;
        self.stack[callee_idx] = Value::VmClosure(closure.clone());

        // Now call it as a regular closure
        self.call_closure(closure, argc, span)
    }

    pub(crate) fn call_class(
        &mut self,
        class: &Rc<Class>,
        argc: usize,
        span: Span,
    ) -> Result<(), RuntimeError> {
        let callee_idx = self.stack.len() - 1 - argc;
        let instance_val = Value::Instance(Rc::new(RefCell::new(Instance::new(class.clone()))));

        // Bytecode constructor (classes compiled in the VM): registered as
        // "init" by compile_constructor and returns `this`, so the frame's
        // return value is already the instance. The instance takes the
        // callee slot → it becomes slot 0 (`this`) under the method calling
        // convention.
        if let Some((ctor, defining_class)) = class.find_vm_method_with_class("init") {
            self.stack[callee_idx] = instance_val;
            return self.call_closure_in_class(ctor, argc, span, Some(defining_class));
        }

        // Tree-walking constructor (classes copied from interpreter globals,
        // e.g. native classes in serve mode): JIT-compile as a method, run
        // it to completion, discard its return value, and yield the instance.
        if let Some(ctor) = class.find_constructor() {
            let proto = jit_compile_method(&ctor, self.globals.keys().cloned())
                .map_err(|e| RuntimeError::new(format!("VM JIT compile error: {}", e), span))?;
            let closure = Rc::new(VmClosure::new(proto, Vec::new()));
            self.stack[callee_idx] = instance_val.clone();
            let saved_depth = self.return_depth;
            let frames_before = self.frames.len();
            self.return_depth = frames_before;
            let outcome = (|| -> Result<(), RuntimeError> {
                self.call_closure(closure, argc, span)?;
                if self.frames.len() != frames_before {
                    self.run()?; // discard the constructor's return value
                }
                Ok(())
            })();
            self.return_depth = saved_depth;
            outcome?;
            self.push(instance_val);
            return Ok(());
        }

        // No constructor: drop any args (tree-walker parity) and yield the
        // instance.
        self.stack.truncate(callee_idx);
        self.push(instance_val);
        Ok(())
    }

    /// Resolve the superclass of the class defining the currently executing
    /// method — the target of `super` dispatch (CallSuperInit /
    /// CallSuperMethod).
    pub(crate) fn frame_superclass(&self, span: Span) -> Result<Rc<Class>, RuntimeError> {
        self.frames
            .last()
            .and_then(|frame| frame.class.clone())
            .and_then(|class| class.superclass.clone())
            .ok_or_else(|| {
                RuntimeError::type_error("super used outside of a subclass method", span)
            })
    }

    /// JIT-compile a tree-walking method and run it to completion with the
    /// receiver already in the callee slot (`[this, args…]`). Returns the
    /// method's return value; the stack is left at the callee slot.
    pub(crate) fn run_jit_method_to_completion(
        &mut self,
        method: &Rc<Function>,
        argc: usize,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        let proto = jit_compile_method(method, self.globals.keys().cloned())
            .map_err(|e| RuntimeError::new(format!("VM JIT compile error: {}", e), span))?;
        let closure = Rc::new(VmClosure::new(proto, Vec::new()));
        let saved_depth = self.return_depth;
        let frames_before = self.frames.len();
        self.return_depth = frames_before;
        let result = (|| -> Result<Value, RuntimeError> {
            self.call_closure(closure, argc, span)?;
            if self.frames.len() == frames_before {
                Ok(self.pop())
            } else {
                self.run()
            }
        })();
        self.return_depth = saved_depth;
        result
    }

    /// Shared CallMethod/CallMethodById slow path for instance/class/other
    /// receivers. Compiled (VmClosure) methods run under the method calling
    /// convention — the receiver stays in the callee slot and becomes
    /// `this` — and empty-parens calls on plain values behave like bare
    /// access (tree-walker parity). Everything else goes through property
    /// lookup + call_value.
    pub(crate) fn call_method_slow_path(
        &mut self,
        receiver_idx: usize,
        argc: usize,
        name: &str,
    ) -> Result<(), RuntimeError> {
        let compiled = match &self.stack[receiver_idx] {
            Value::Instance(inst) => {
                let class = inst.borrow().class.clone();
                class.find_vm_method_with_class(name)
            }
            // Statics compile as plain functions; the class value left in
            // the callee slot is ignored by the bytecode.
            Value::Class(class) => class
                .find_vm_static_method(name)
                .map(|closure| (closure, class.clone())),
            _ => None,
        };
        if let Some((closure, defining_class)) = compiled {
            // Hot path for compiled method calls — span is computed only on
            // the cold arity-error branch.
            let arity = closure.proto.arity as usize;
            let total_params = closure.proto.param_names.len();
            if argc < arity || argc > total_params {
                return Err(RuntimeError::wrong_arity(
                    total_params,
                    argc,
                    self.current_span(),
                ));
            }
            for _ in argc..total_params {
                self.stack.push(Value::Null);
            }
            let stack_base = self.stack.len() - total_params - 1;
            self.frames.push(CallFrame {
                closure,
                ip: 0,
                stack_base,
                class: Some(defining_class),
            });
            return Ok(());
        }
        let span = self.current_span();
        let object = self.stack[receiver_idx].clone();
        let method_val = self.op_get_property(&object, name, span)?;
        if argc == 0 && !method_val.is_callable() {
            self.stack.truncate(receiver_idx);
            self.stack.push(method_val);
        } else {
            self.stack[receiver_idx] = method_val;
            self.call_value(argc, span)?;
        }
        Ok(())
    }

    fn call_builtin_method(
        &mut self,
        method_name: &str,
        argc: usize,
        span: Span,
    ) -> Result<(), RuntimeError> {
        // Stack layout: [receiver, arg1, .., argN] — take the args off the
        // top first, then the receiver, and delegate to the same per-type
        // dispatchers CallMethod uses so stored bound methods (e.g.
        // `m = arr.contains; m(5)`) behave identically to direct calls.
        let args = self.stack.split_off(self.stack.len() - argc);
        let receiver = self.pop();

        let result = match &receiver {
            Value::Array(arr) => self.vm_call_array_method(arr, method_name, &args, span)?,
            Value::String(s) => self.vm_call_string_method(s.as_ref(), method_name, &args, span)?,
            Value::Hash(hash) => self.vm_call_hash_method(hash, method_name, &args, span)?,
            Value::Int(_) | Value::Float(_) | Value::Bool(_) | Value::Null | Value::Decimal(_) => {
                self.vm_call_primitive_method(&receiver, method_name, &args, span)?
            }
            _ => {
                return Err(RuntimeError::NoSuchProperty {
                    value_type: receiver.type_name(),
                    property: method_name.to_string(),
                    span,
                })
            }
        };
        self.push(result);
        Ok(())
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

    /// Invoke a class method with `this` bound to the given instance.
    ///
    /// Used by the server's class-based controller dispatch: JIT-compiles the
    /// method as `FunctionType::Method` so slot 0 is reserved for `this`, then
    /// seeds the call frame with `instance` at slot 0 and `arg` at slot 1.
    ///
    /// The compiled `FunctionProto` is cached on `method.jit_cache` so the AST
    /// walk in `Compiler::compile_method_standalone` only runs once per worker
    /// per method. (Each worker loads its own `Rc<Function>` instances in
    /// `load_controllers_in_worker`, so the `RefCell` cache is per-worker and
    /// has no cross-thread aliasing.) `warm_vm_handlers` pre-fills the cache at
    /// boot so the first request to a method is a cache hit, not a compile.
    pub fn call_method_bound(
        &mut self,
        method: &Function,
        instance: Value,
        arg: Value,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        // The `let cached = ...borrow().clone()` line scopes the
        // `RefCell` borrow to the let statement so it's released before
        // the `else` branch runs. The earlier `if let Some(...) = borrow()`
        // form held the `Ref` across the whole if-else, which panicked
        // with "RefCell already borrowed" when the else arm took
        // `borrow_mut()` to install the freshly-compiled proto.
        let proto = {
            let cached = method.jit_cache.borrow().clone();
            if let Some(cached) = cached {
                cached
            } else {
                let compiled =
                    Compiler::compile_method_standalone(method, self.globals.keys().cloned())
                        .map_err(|e| {
                            RuntimeError::new(format!("VM JIT compile error: {}", e), span)
                        })?;
                let arc = Arc::new(compiled);
                *method.jit_cache.borrow_mut() = Some(arc.clone());
                arc
            }
        };
        let closure = Rc::new(VmClosure::new(proto, Vec::new()));

        // Stack layout after these pushes: [..., instance, arg]. call_closure
        // derives stack_base = len - total_params - 1, placing `instance` at
        // slot 0 (i.e., `this`) and `arg` at slot 1 — matching the layout the
        // method bytecode expects.
        self.push(instance);
        self.push(arg);
        self.call_closure(closure, 1, span)?;
        self.run()
    }

    /// Reset VM state between requests (preserves globals).
    pub fn reset(&mut self) {
        self.stack.clear();
        self.frames.clear();
        self.open_upvalues.clear();
        self.exception_handlers.clear();
        self.iter_stack.clear();
        self.return_depth = 0;
    }

    /// Invoke a callable synchronously from within a native method.
    /// Bumps `return_depth` so nested `run()` exits when this specific call returns,
    /// letting the native caller resume with the result on its own path.
    pub fn invoke_callable(
        &mut self,
        callee: Value,
        args: Vec<Value>,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        let saved_depth = self.return_depth;
        let frames_before = self.frames.len();
        self.push(callee);
        let argc = args.len();
        for arg in args {
            self.push(arg);
        }
        self.call_value(argc, span)?;
        if self.frames.len() == frames_before {
            return Ok(self.pop());
        }
        self.return_depth = frames_before;
        let result = self.run();
        self.return_depth = saved_depth;
        result
    }

    /// Optimized single-arg variant — borrows the callee (clones once for the stack
    /// push) and avoids the Vec allocation. Hot path for array.map/filter/each.
    #[inline]
    pub fn invoke_callable_one(
        &mut self,
        callee: &Value,
        arg: Value,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        let saved_depth = self.return_depth;
        let frames_before = self.frames.len();
        self.push(callee.clone());
        self.push(arg);
        self.call_value(1, span)?;
        if self.frames.len() == frames_before {
            return Ok(self.pop());
        }
        self.return_depth = frames_before;
        let result = self.run();
        self.return_depth = saved_depth;
        result
    }

    /// Optimized two-arg variant — hot path for array.reduce.
    #[inline]
    pub fn invoke_callable_two(
        &mut self,
        callee: &Value,
        a: Value,
        b: Value,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        let saved_depth = self.return_depth;
        let frames_before = self.frames.len();
        self.push(callee.clone());
        self.push(a);
        self.push(b);
        self.call_value(2, span)?;
        if self.frames.len() == frames_before {
            return Ok(self.pop());
        }
        self.return_depth = frames_before;
        let result = self.run();
        self.return_depth = saved_depth;
        result
    }

    /// Pre-arrange `return_depth` for a batch of closure invocations.
    /// Returns a guard struct that restores the original depth on drop.
    /// Use the `_unguarded` invoke variants below within the scope.
    #[inline]
    pub fn enter_callable_batch(&mut self) -> CallableBatch {
        let saved_depth = self.return_depth;
        let frames_before = self.frames.len();
        self.return_depth = frames_before;
        CallableBatch {
            saved_depth,
            frames_before,
        }
    }

    #[inline]
    pub fn exit_callable_batch(&mut self, batch: CallableBatch) {
        self.return_depth = batch.saved_depth;
    }

    /// Single-arg invoke that assumes `return_depth` is already set up by
    /// `enter_callable_batch`. Saves the per-iteration save/restore writes.
    #[inline]
    pub fn invoke_in_batch_one(
        &mut self,
        batch: &CallableBatch,
        callee: &Value,
        arg: Value,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        self.push(callee.clone());
        self.push(arg);
        self.call_value(1, span)?;
        if self.frames.len() == batch.frames_before {
            return Ok(self.pop());
        }
        self.run()
    }

    #[inline]
    pub fn invoke_in_batch_two(
        &mut self,
        batch: &CallableBatch,
        callee: &Value,
        a: Value,
        b: Value,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        self.push(callee.clone());
        self.push(a);
        self.push(b);
        self.call_value(2, span)?;
        if self.frames.len() == batch.frames_before {
            return Ok(self.pop());
        }
        self.run()
    }
}

/// Snapshot of VM state for a batch of closure invocations made by a single
/// native method (e.g. array.map's loop). Captured by `enter_callable_batch`
/// and consumed by `exit_callable_batch`.
pub struct CallableBatch {
    saved_depth: usize,
    frames_before: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jit_compile_function_caches_proto() {
        // An empty-bodied function is enough to exercise the compile path.
        let func = Function {
            name: "warm_me".to_string(),
            ..Function::default()
        };
        assert!(func.jit_cache.borrow().is_none());

        let proto = jit_compile_function(&func, std::iter::empty());
        assert!(
            proto.is_ok(),
            "warmup compile should succeed: {:?}",
            proto.err()
        );

        // The proto is now cached on the function...
        assert!(func.jit_cache.borrow().is_some());

        // ...and a second call returns the same cached proto (no recompile).
        let again = jit_compile_function(&func, std::iter::empty()).expect("cached compile");
        assert!(Arc::ptr_eq(&proto.unwrap(), &again));
    }
}
