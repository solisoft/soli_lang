//! Function call dispatch for the VM.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use crate::ast::stmt::{FunctionDecl, Program, Stmt, StmtKind};
use crate::error::RuntimeError;
use crate::interpreter::executor::MAX_CALL_DEPTH;
use crate::interpreter::value::{Class, Function, HashKey, Instance, NativeFunction, Value};
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
        params: func.params.to_vec(),
        return_type: None,
        body: func.body.to_vec(),
        span: func.span.unwrap_or_default(),
    };

    let program = Program::new(vec![Stmt {
        kind: StmtKind::function(func_decl),
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

/// Lay labelled arguments out in parameter order for `proto`, returning the
/// slot values and a mask of which parameters were actually supplied.
///
/// Follows the tree-walking interpreter's rules: positional arguments fill the
/// leading parameters, labelled arguments fill by name, an unknown label is an
/// undefined-variable error, and a parameter that ends up unfilled is an arity
/// error unless it declares a default. Unfilled defaulted slots are left null
/// here and written by the callee's prologue, so a default expression is
/// evaluated in the callee — and only when it is actually needed.
fn bind_named_arguments(
    proto: &FunctionProto,
    positional: Vec<Value>,
    named: Vec<(crate::interpreter::value::SoliStr, Value)>,
    span: Span,
) -> Result<(Vec<Value>, u64), RuntimeError> {
    let total_params = proto.param_names.len();

    if positional.len() > total_params {
        return Err(RuntimeError::wrong_arity(
            total_params,
            positional.len() + named.len(),
            span,
        ));
    }

    let mut slots = vec![Value::Null; total_params];
    let mut supplied = 0u64;

    for (i, value) in positional.into_iter().enumerate() {
        slots[i] = value;
        if i < 64 {
            supplied |= 1u64 << i;
        }
    }

    for (name, value) in named {
        let Some(index) = proto
            .param_names
            .iter()
            .position(|p| p.as_str() == name.as_ref())
        else {
            // Same error the interpreter raises for `f(nope: 1)`.
            return Err(RuntimeError::undefined_variable(name.to_string(), span));
        };
        // A label that names an already-positionally-filled parameter is
        // dropped, and the positional value wins. That is not an obviously good
        // rule — arguably `f(3, a: 9)` should be an error — but it is what the
        // tree-walking interpreter does (`used_params.contains(...) { continue }`
        // in `call_value_with_named`), and the engines must agree.
        if index < 64 && supplied & (1u64 << index) != 0 {
            continue;
        }
        slots[index] = value;
        if index < 64 {
            supplied |= 1u64 << index;
        }
    }

    // Anything still unfilled must have a default to fall back on.
    for index in 0..total_params {
        let filled = index >= 64 || supplied & (1u64 << index) != 0;
        let has_default = index >= 64 || proto.defaults_mask & (1u64 << index) != 0;
        if !filled && !has_default {
            // Report how many parameters were bound before this one, matching
            // the interpreter's `final_args.len()` at the point it gives up.
            let bound_before = if index >= 64 {
                index
            } else {
                (supplied & ((1u64 << index) - 1)).count_ones() as usize
            };
            return Err(RuntimeError::wrong_arity(
                proto.arity as usize,
                bound_before,
                span,
            ));
        }
    }

    Ok((slots, supplied))
}

impl Vm {
    /// Refuse calls past [`MAX_CALL_DEPTH`]. The bytecode loop itself is
    /// iterative (deep recursion grows `frames` on the heap), but builtins
    /// that call back into Soli recurse natively — and a stack overflow
    /// aborts without unwinding, so it must be prevented, not caught.
    #[inline]
    pub(crate) fn ensure_call_depth(&self, span: Span) -> Result<(), RuntimeError> {
        if self.frames.len() >= MAX_CALL_DEPTH {
            return Err(RuntimeError::new(
                format!("call stack too deep ({MAX_CALL_DEPTH} frames) — unbounded recursion?"),
                span,
            ));
        }
        Ok(())
    }

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

        // Reserve a stack slot for every parameter the caller omitted. The slot
        // starts as null; the callee's `JumpIfParamSupplied` prologue overwrites
        // it with the declared default (if any) before the body runs.
        if argc < total_params {
            for _ in argc..total_params {
                self.push(Value::Null);
            }
        }

        let stack_base = self.stack.len() - total_params - 1;

        self.ensure_call_depth(span)?;

        self.frames.push(CallFrame::new(
            closure,
            stack_base,
            self.iter_stack.len(),
            class,
            crate::vm::vm::positional_supplied_mask(argc),
        ));

        Ok(())
    }

    /// Push `slots` (already in parameter order) and open a frame with an
    /// explicit supplied-parameter mask.
    ///
    /// The positional entry points derive the mask from `argc` because the
    /// caller fills a prefix of the parameters. A named call can fill any
    /// subset, so it computes the mask itself and hands it over here; the
    /// callee's default-value prologue then runs for exactly the unfilled
    /// parameters.
    fn call_closure_with_slots(
        &mut self,
        closure: Rc<VmClosure>,
        slots: Vec<Value>,
        supplied: u64,
        class: Option<Rc<Class>>,
    ) -> Result<(), RuntimeError> {
        let total_params = slots.len();
        for value in slots {
            self.push(value);
        }
        let stack_base = self.stack.len() - total_params - 1;
        self.ensure_call_depth(self.current_span())?;
        self.frames.push(CallFrame::new(
            closure,
            stack_base,
            self.iter_stack.len(),
            class,
            supplied,
        ));
        Ok(())
    }

    /// Call the value on the stack beneath `argc` argument slots, where
    /// `labels[i]` names slot `i` (or is `None` when that slot is positional).
    ///
    /// Mirrors the tree-walking interpreter's `call_value_with_named`, which
    /// applies two different conventions depending on what the callee turns out
    /// to be — hence the dispatch happens here, at call time, rather than in the
    /// compiler. Callee shapes the VM has no binding rule for surface as
    /// `EngineFallback` so serve mode re-runs the request on the interpreter
    /// instead of failing the request.
    pub(crate) fn call_value_named(
        &mut self,
        argc: usize,
        labels: &[Option<crate::interpreter::value::SoliStr>],
        span: Span,
    ) -> Result<(), RuntimeError> {
        // Arguments were evaluated in source order, so slot i pairs with
        // labels[i]. Lift them off the stack, leaving the callee on top.
        let mut values = Vec::with_capacity(argc);
        for _ in 0..argc {
            values.push(self.pop());
        }
        values.reverse();

        let mut positional: Vec<Value> = Vec::new();
        let mut named: Vec<(crate::interpreter::value::SoliStr, Value)> = Vec::new();
        for (i, value) in values.into_iter().enumerate() {
            match labels.get(i).and_then(|l| l.as_ref()) {
                Some(name) => {
                    if named.iter().any(|(existing, _)| existing == name) {
                        return Err(RuntimeError::type_error(
                            format!("duplicate named argument '{}'", name),
                            span,
                        ));
                    }
                    named.push((name.clone(), value));
                }
                None => positional.push(value),
            }
        }

        let callee_idx = self.stack.len() - 1;
        let callee = self.stack[callee_idx].clone();
        match callee {
            // Natives take an options hash: Ruby-style, the labelled arguments
            // collapse into a single trailing positional hash. This is what
            // makes `get("/", "home#index", name: "root")` work.
            Value::NativeFunction(ref native) => {
                let hash = {
                    let mut pairs = crate::interpreter::value::HashPairs::default();
                    for (name, value) in named {
                        pairs.insert(
                            crate::interpreter::value::HashKey::String(name.to_string().into()),
                            value,
                        );
                    }
                    Value::Hash(Rc::new(RefCell::new(pairs)))
                };
                let count = positional.len() + 1;
                for value in positional {
                    self.push(value);
                }
                self.push(hash);
                let native = native.clone();
                self.call_native(&native, count, span)
            }
            // Compiled function: reorder into parameter slots.
            Value::VmClosure(closure) => {
                let (slots, supplied) =
                    bind_named_arguments(&closure.proto, positional, named, span)?;
                self.call_closure_with_slots(closure, slots, supplied, None)
            }
            // Tree-walking function reached from compiled code: compile it,
            // then bind exactly as above.
            Value::Function(ref func) => {
                let proto =
                    jit_compile_function(func, self.globals.keys().cloned()).map_err(|e| {
                        RuntimeError::EngineFallback(
                            format!("a function the VM cannot compile ({})", e),
                            span,
                        )
                    })?;
                let closure = Rc::new(VmClosure::new(proto, Vec::new()));
                self.stack[callee_idx] = Value::VmClosure(closure.clone());
                let (slots, supplied) =
                    bind_named_arguments(&closure.proto, positional, named, span)?;
                self.call_closure_with_slots(closure, slots, supplied, None)
            }
            // `Config(port: 3000)` — bind against the compiled constructor and
            // let it run with `this` in the callee slot.
            Value::Class(ref class) => {
                if let Some((ctor, defining_class)) = class.find_vm_method_with_class("init") {
                    let (slots, supplied) =
                        bind_named_arguments(&ctor.proto, positional, named, span)?;
                    let instance =
                        Value::Instance(Rc::new(RefCell::new(Instance::new(class.clone()))));
                    self.stack[callee_idx] = instance;
                    self.call_closure_with_slots(ctor, slots, supplied, Some(defining_class))
                } else {
                    // Tree-walking constructors need the interpreter's
                    // run-to-completion dance; punt rather than half-bind.
                    Err(RuntimeError::EngineFallback(
                        format!("named arguments to {}'s constructor", class.name),
                        span,
                    ))
                }
            }
            _ => Err(RuntimeError::EngineFallback(
                "named arguments to this callee".to_string(),
                span,
            )),
        }
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

        if native.name == "grouped" {
            return self.call_grouped_block(args, span);
        }
        if native.name == "with_transaction" {
            return self.call_with_transaction_block(args, span);
        }

        // Call the native function. Wrap in a flamegraph `Fn` span when
        // the native is on the request-path whitelist (see
        // `span_log::is_request_path_native`); cheap builtins are
        // skipped to keep the chart readable.
        let _native_span = crate::serve::span_log::maybe_instrument_native(&native.name);
        let result = (native.func)(&args).map_err(|e| RuntimeError::new(e, span))?;
        drop(_native_span);
        self.push(result);
        Ok(())
    }

    fn is_callable_block(value: &Value) -> bool {
        matches!(
            value,
            Value::Function(_) | Value::NativeFunction(_) | Value::VmClosure(_)
        )
    }

    fn call_grouped_block(&mut self, args: Vec<Value>, span: Span) -> Result<(), RuntimeError> {
        if args.len() != 1 || !Self::is_callable_block(&args[0]) {
            return Err(RuntimeError::new(
                "grouped() expects a function block: grouped(fn() { ... })",
                span,
            ));
        }
        let block = args.into_iter().next().unwrap();
        let coalesce = crate::interpreter::builtins::model::batch::should_coalesce(
            crate::interpreter::builtins::template::is_dev_mode(),
            crate::interpreter::builtins::test_server::is_test_runner_process(),
        );
        let result = crate::interpreter::builtins::model::batch::with_grouped_block(
            coalesce,
            || self.invoke_callable(block.clone(), &[], span),
            |e| RuntimeError::General {
                message: format!("grouped: failed to flush queries: {}", e),
                span,
            },
        )?;
        self.push(result);
        Ok(())
    }

    fn call_with_transaction_block(
        &mut self,
        args: Vec<Value>,
        span: Span,
    ) -> Result<(), RuntimeError> {
        use crate::interpreter::builtins::model::crud::{
            begin_transaction, clear_current_tx, has_active_tx, rollback_transaction,
        };
        if args.len() != 1 || !Self::is_callable_block(&args[0]) {
            return Err(RuntimeError::new(
                "with_transaction() expects a function block",
                span,
            ));
        }
        let block = args.into_iter().next().unwrap();
        if has_active_tx() {
            let result = self.invoke_callable(block, &[], span)?;
            self.push(result);
            return Ok(());
        }
        begin_transaction(None).map_err(|e| RuntimeError::General {
            message: format!("with_transaction: failed to begin: {}", e),
            span,
        })?;
        match self.invoke_callable(block, &[], span) {
            Ok(value) => {
                if let Err(e) = rollback_transaction() {
                    clear_current_tx();
                    return Err(RuntimeError::General {
                        message: format!("with_transaction: rollback failed: {}", e),
                        span,
                    });
                }
                self.push(value);
                Ok(())
            }
            Err(err) => {
                let _ = rollback_transaction();
                clear_current_tx();
                Err(err)
            }
        }
    }

    fn call_model_transaction_block(
        &mut self,
        receiver_idx: usize,
        span: Span,
    ) -> Result<(), RuntimeError> {
        let class_name = match &self.stack[receiver_idx] {
            Value::Class(class) => class.name.clone(),
            _ => unreachable!(),
        };
        let block = self.stack.pop().expect("transaction block");
        let _receiver = self.stack.pop().expect("transaction receiver");
        let result = crate::interpreter::builtins::model::crud::with_model_transaction(
            &class_name,
            || self.invoke_callable(block.clone(), &[], span),
            |e| RuntimeError::General {
                message: format!("transaction: failed to begin: {}", e),
                span,
            },
            |e| RuntimeError::General {
                message: format!("transaction: commit failed: {}", e),
                span,
            },
        )?;
        self.push(result);
        Ok(())
    }

    /// Compile `method` as a standalone *method* — slot 0 reserved for `this` —
    /// and cache the proto on `method.jit_cache`.
    ///
    /// Defining `this` in a bound `Environment` (the old `bind_callback_method`)
    /// did not survive the call: `invoke_callable` routes a `Value::Function`
    /// through `jit_compile_function`, which recompiles `params`/`body` as a
    /// plain `FunctionType::Function` and discards the environment entirely.
    /// `compile_this` then emitted `GetLocal(0)`, which in a non-method frame is
    /// the *callee* slot — so `this.field = x` in a callback failed with
    /// "Cannot set property on Function". Compiling as a method is what
    /// `call_method_bound` already does for controller actions.
    ///
    /// The cache also keeps this off the per-invocation compile path: a fresh
    /// `Rc<Function>` with an empty `jit_cache` meant a full AST walk on every
    /// `save()`.
    fn callback_proto(
        &self,
        method: &Function,
        span: Span,
    ) -> Result<Arc<FunctionProto>, RuntimeError> {
        // Scope the borrow to the `let` so the else arm can `borrow_mut()`.
        let cached = method.jit_cache.borrow().clone();
        if let Some(cached) = cached {
            return Ok(cached);
        }
        let compiled = Compiler::compile_method_standalone(method, self.globals.keys().cloned())
            .map_err(|e| {
                RuntimeError::EngineFallback(
                    format!("a model callback the VM cannot compile ({})", e),
                    span,
                )
            })?;
        let arc = Arc::new(compiled);
        *method.jit_cache.borrow_mut() = Some(arc.clone());
        Ok(arc)
    }

    /// Refuse the whole operation *before* it touches the row when any callback
    /// for these events cannot run correctly on the VM.
    ///
    /// Two cases refuse: a closure-form callback (`before_save do … end`) needs
    /// the environment it captured, which the bytecode path cannot reconstruct;
    /// and a method whose body the compiler rejects. Both must be detected here
    /// rather than at invocation time, because an `after_*` callback runs when
    /// the native write has already happened — demoting then re-runs the whole
    /// handler on the tree-walker and writes the row a second time. Refusing up
    /// front is side-effect-free, and the tree-walker runs these correctly.
    fn ensure_callbacks_vm_ready(
        &self,
        class: &Rc<Class>,
        event_sets: &[&[&str]],
        span: Span,
    ) -> Result<(), RuntimeError> {
        use crate::interpreter::executor::calls::function::{
            callback_names_for, has_closure_callbacks,
        };
        for events in event_sets {
            if has_closure_callbacks(&class.name, events) {
                return Err(RuntimeError::EngineFallback(
                    format!("closure-form model callback on '{}'", class.name),
                    span,
                ));
            }
            for cb_name in callback_names_for(&class.name, events) {
                if class.find_vm_method_with_class(&cb_name).is_some() {
                    continue;
                }
                if let Some(method) = class.find_method(&cb_name) {
                    self.callback_proto(&method, span)?;
                }
            }
        }
        Ok(())
    }

    fn invoke_zero_arg_instance_method(
        &mut self,
        instance: &Rc<RefCell<Instance>>,
        closure: Rc<VmClosure>,
        defining_class: Rc<Class>,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        let saved_depth = self.return_depth;
        let frames_before = self.frames.len();
        self.push(Value::Instance(instance.clone()));
        self.call_closure_in_class(closure, 0, span, Some(defining_class))?;
        let result = if self.frames.len() == frames_before {
            Ok(self.pop())
        } else {
            self.return_depth = frames_before;
            self.run()
        };
        self.return_depth = saved_depth;
        result
    }

    /// Run method-name and closure model callbacks with `this` bound to
    /// `instance`. `Ok(false)` is a `before_*` veto (first `false` wins).
    fn run_model_callbacks_vm(
        &mut self,
        class: &Rc<Class>,
        instance: &Rc<RefCell<Instance>>,
        callback_names: &[String],
        events: &[&str],
        span: Span,
    ) -> Result<bool, RuntimeError> {
        for cb_name in callback_names {
            if let Some((closure, defining)) = class.find_vm_method_with_class(cb_name) {
                let result =
                    self.invoke_zero_arg_instance_method(instance, closure, defining, span)?;
                if matches!(result, Value::Bool(false)) {
                    return Ok(false);
                }
                continue;
            }
            if let Some(method) = class.find_method(cb_name) {
                // Compiled as a method with the instance in the callee slot, so
                // `this` inside the callback is the record — see `callback_proto`.
                let proto = self.callback_proto(&method, span)?;
                let closure = Rc::new(VmClosure::new(proto, Vec::new()));
                let result =
                    self.invoke_zero_arg_instance_method(instance, closure, class.clone(), span)?;
                if matches!(result, Value::Bool(false)) {
                    return Ok(false);
                }
            }
        }
        // Closure-form callbacks need the environment they captured, which the
        // bytecode path cannot reconstruct. `ensure_callbacks_vm_ready` refuses
        // before any write happens, so reaching this is a bug — refuse rather
        // than run the body against the wrong scope.
        for ev in events {
            if !crate::interpreter::builtins::model::callbacks::closure_callbacks_for(
                &class.name,
                ev,
            )
            .is_empty()
            {
                return Err(RuntimeError::EngineFallback(
                    format!("closure-form model callback on '{}'", class.name),
                    span,
                ));
            }
        }
        Ok(true)
    }

    fn persist_wrap_needed(class_name: &str, before: &[&str], after: &[&str]) -> bool {
        use crate::interpreter::executor::calls::function::{
            callback_names_for, has_closure_callbacks,
        };
        !callback_names_for(class_name, before).is_empty()
            || !callback_names_for(class_name, after).is_empty()
            || has_closure_callbacks(class_name, before)
            || has_closure_callbacks(class_name, after)
    }

    /// Call a Model instance native, firing lifecycle callbacks when any are
    /// registered. No `EngineFallback`: handlers stay on the VM.
    pub(crate) fn call_model_instance_native(
        &mut self,
        inst: Rc<RefCell<Instance>>,
        native: Rc<NativeFunction>,
        receiver_idx: usize,
        argc: usize,
        name: &str,
        span: Span,
    ) -> Result<(), RuntimeError> {
        use crate::interpreter::executor::calls::function::{
            callback_names_for, persist_events_for, set_callback_aborted_error,
        };

        if let Some(expected) = native.arity {
            if argc != expected {
                return Err(RuntimeError::wrong_arity(expected, argc, span));
            }
        }

        let class = inst.borrow().class.clone();
        let user_args: Vec<Value> = self.stack[receiver_idx + 1..receiver_idx + 1 + argc].to_vec();

        let has_key = matches!(inst.borrow().get("_key"), Some(Value::String(_)));
        if let Some((before_events, after_events)) = persist_events_for(name, has_key) {
            if Self::persist_wrap_needed(&class.name, before_events, after_events) {
                // Refuse before the write when any callback can't run here: an
                // after_* refusal would arrive with the row already written.
                self.ensure_callbacks_vm_ready(&class, &[before_events, after_events], span)?;
                let before_names = callback_names_for(&class.name, before_events);
                if !self.run_model_callbacks_vm(
                    &class,
                    &inst,
                    &before_names,
                    before_events,
                    span,
                )? {
                    let kind = if before_events.contains(&"before_create") {
                        "before_create / before_save"
                    } else {
                        "before_update / before_save"
                    };
                    set_callback_aborted_error(&inst, kind);
                    self.stack.truncate(receiver_idx);
                    self.stack.push(Value::Bool(false));
                    return Ok(());
                }
                let result =
                    crate::interpreter::executor::access::member::call_native_instance_method(
                        &inst, &native, &user_args,
                    )
                    .map_err(|e| RuntimeError::new(e, span))?;
                let failed = matches!(&result, Value::Bool(false));
                if !failed {
                    let after_names = callback_names_for(&class.name, after_events);
                    self.run_model_callbacks_vm(&class, &inst, &after_names, after_events, span)?;
                }
                self.stack.truncate(receiver_idx);
                self.stack.push(result);
                return Ok(());
            }
        }

        if name == "delete"
            && argc == 0
            && self.try_wrap_model_delete(&class, &inst, &native, receiver_idx, span)?
        {
            return Ok(());
        }

        let result = crate::interpreter::executor::access::member::call_native_instance_method(
            &inst, &native, &user_args,
        )
        .map_err(|e| RuntimeError::new(e, span))?;
        self.stack.truncate(receiver_idx);
        self.stack.push(result);
        Ok(())
    }

    /// Full instance-delete wrap: cycle guard, before_delete, cascades,
    /// attachment purge, native, after_delete. Returns `false` when this
    /// delete has no callbacks/dependents/attachments (plain native).
    fn try_wrap_model_delete(
        &mut self,
        class: &Rc<Class>,
        inst: &Rc<RefCell<Instance>>,
        native: &Rc<NativeFunction>,
        receiver_idx: usize,
        span: Span,
    ) -> Result<bool, RuntimeError> {
        use crate::interpreter::executor::calls::cascade;
        use crate::interpreter::executor::calls::function::{
            callback_names_for, has_closure_callbacks, set_callback_aborted_error,
        };

        let before_events: &[&str] = &["before_delete"];
        let after_events: &[&str] = &["after_delete"];
        let has_dependents = cascade::class_declares_dependents(&class.name);
        let has_attachments =
            !crate::interpreter::builtins::model::get_uploaders(&class.name).is_empty();
        if !Self::persist_wrap_needed(&class.name, before_events, after_events)
            && !has_closure_callbacks(&class.name, before_events)
            && !has_closure_callbacks(&class.name, after_events)
            && !has_dependents
            && !has_attachments
        {
            return Ok(false);
        }

        let mut _cascade_guard = None;
        if has_dependents {
            let (collection, key) = {
                let inst_ref = inst.borrow();
                let collection = crate::interpreter::builtins::model::class_name_to_collection(
                    &inst_ref.class.name,
                );
                let key = match inst_ref.get("_key") {
                    Some(Value::String(s)) => Some(s.to_string()),
                    _ => None,
                };
                (collection, key)
            };
            if let Some(key) = key {
                match cascade::enter_cascade(&collection, &key) {
                    Some(guard) => _cascade_guard = Some(guard),
                    None => {
                        self.stack.truncate(receiver_idx);
                        self.stack.push(Value::Bool(true));
                        return Ok(true);
                    }
                }
            }
        }

        // Same up-front refusal as the persist path: after_delete runs once the
        // row (and its cascades) are gone, so it is too late to demote there.
        self.ensure_callbacks_vm_ready(class, &[before_events, after_events], span)?;
        let before_names = callback_names_for(&class.name, before_events);
        if !self.run_model_callbacks_vm(class, inst, &before_names, before_events, span)? {
            set_callback_aborted_error(inst, "before_delete");
            self.stack.truncate(receiver_idx);
            self.stack.push(Value::Bool(false));
            return Ok(true);
        }

        if has_dependents && !crate::interpreter::builtins::model::is_soft_delete(&class.name) {
            cascade::run_dependent_cascades_with(inst, span, |child, span| {
                self.delete_model_instance_vm(child, span)
            })?;
        }

        if has_attachments {
            if let Some(helper) = self.globals.get("detach_all_uploads").cloned() {
                let _ = self.invoke_callable(helper, &[Value::Instance(inst.clone())], span)?;
            }
        }

        let result = crate::interpreter::executor::access::member::call_native_instance_method(
            inst,
            native,
            &[],
        )
        .map_err(|e| RuntimeError::new(e, span))?;
        let failed = matches!(&result, Value::String(s) if s.starts_with("Error:"))
            || matches!(&result, Value::Bool(false));
        if !failed {
            let after_names = callback_names_for(&class.name, after_events);
            self.run_model_callbacks_vm(class, inst, &after_names, after_events, span)?;
        }
        self.stack.truncate(receiver_idx);
        self.stack.push(result);
        Ok(true)
    }

    fn delete_model_instance_vm(
        &mut self,
        instance_value: &Value,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        let inst = match instance_value {
            Value::Instance(i) => i.clone(),
            _ => {
                return Err(RuntimeError::new(
                    "dependent delete expected a model instance",
                    span,
                ))
            }
        };
        let native = inst
            .borrow()
            .class
            .find_native_method("delete")
            .ok_or_else(|| {
                RuntimeError::new(
                    format!(
                        "dependent delete: {} has no delete method",
                        inst.borrow().class.name
                    ),
                    span,
                )
            })?;
        let receiver_idx = self.stack.len();
        self.push(instance_value.clone());
        self.call_model_instance_native(inst, native, receiver_idx, 0, "delete", span)?;
        Ok(self.pop())
    }

    /// `Model.create(data)` / `Model.update(id, data)` with before/after
    /// callbacks. Returns `false` when this call should use the ordinary
    /// native-static path (no callbacks, or the args aren't a data hash).
    /// Reject the inherited document-API statics on a columnar model.
    ///
    /// `bind_native_static_to_model_class` documents itself as "the one choke
    /// point shared by the tree-walker and the VM" for this rule, but the
    /// callback-wrapping paths below call the native (and `crud`) directly and
    /// therefore skip it. Without repeating the check, a columnar model that
    /// happens to declare a save callback silently ran the document API instead
    /// of raising.
    fn reject_columnar_document_api(
        class: &Rc<Class>,
        name: &str,
        span: Span,
    ) -> Result<(), RuntimeError> {
        use crate::interpreter::builtins::model::columnar;
        if columnar::is_document_api_method(name)
            && crate::interpreter::builtins::model::is_columnar_model(&class.name)
        {
            return Err(RuntimeError::new(
                columnar::columnar_no_document_api_error(&class.name, name),
                span,
            ));
        }
        Ok(())
    }

    fn try_call_model_class_persist(
        &mut self,
        receiver_idx: usize,
        argc: usize,
        name: &str,
        span: Span,
    ) -> Result<bool, RuntimeError> {
        use crate::interpreter::executor::calls::function::{
            callback_names_for, has_closure_callbacks, set_callback_aborted_error,
        };

        let data_index = if name == "create" { 0 } else { 1 };
        if argc <= data_index {
            return Ok(false);
        }
        let class = match &self.stack[receiver_idx] {
            Value::Class(c) => c.clone(),
            _ => return Ok(false),
        };
        let before_events: &[&str] = if name == "create" {
            &["before_save", "before_create"]
        } else {
            &["before_save", "before_update"]
        };
        let after_events: &[&str] = if name == "create" {
            &["after_create", "after_save"]
        } else {
            &["after_update", "after_save"]
        };
        let before_names = callback_names_for(&class.name, before_events);
        let after_names = callback_names_for(&class.name, after_events);
        if before_names.is_empty()
            && after_names.is_empty()
            && !has_closure_callbacks(&class.name, before_events)
            && !has_closure_callbacks(&class.name, after_events)
        {
            return Ok(false);
        }

        let Some(native) = class.find_native_static_method(name) else {
            return Ok(false);
        };
        // Both checks run before any callback or write: a columnar model must
        // raise rather than reach the document API, and a callback the VM can't
        // run correctly must demote while the row is still untouched (an
        // after_* refusal would arrive with the row already written, and the
        // handler would write it again on the tree-walker).
        Self::reject_columnar_document_api(&class, name, span)?;
        self.ensure_callbacks_vm_ready(&class, &[before_events, after_events], span)?;
        let mut user_args: Vec<Value> =
            self.stack[receiver_idx + 1..receiver_idx + 1 + argc].to_vec();
        let data_hash = match &user_args[data_index] {
            Value::Hash(h) => h.clone(),
            _ => return Ok(false),
        };

        let mut instance = Instance::new(class.clone());
        for (k, v) in data_hash.borrow().iter() {
            if let HashKey::String(field) = k {
                instance.set(field.clone().to_string(), v.clone());
            }
        }
        let inst_rc = Rc::new(RefCell::new(instance));

        if !self.run_model_callbacks_vm(&class, &inst_rc, &before_names, before_events, span)? {
            let kind = if name == "create" {
                "before_create / before_save"
            } else {
                "before_update / before_save"
            };
            set_callback_aborted_error(&inst_rc, kind);
            self.stack.truncate(receiver_idx);
            self.stack.push(Value::Instance(inst_rc));
            return Ok(true);
        }

        let inst_ref = inst_rc.borrow();
        let mut new_pairs = crate::interpreter::value::HashPairs::default();
        for (k, v) in &inst_ref.fields {
            new_pairs.insert(HashKey::String(k.clone()), v.clone());
        }
        drop(inst_ref);
        user_args[data_index] = Value::Hash(Rc::new(RefCell::new(new_pairs)));

        let mut native_args = Vec::with_capacity(user_args.len() + 1);
        native_args.push(Value::Class(class.clone()));
        native_args.extend(user_args);
        let result = (native.func)(&native_args).map_err(|e| RuntimeError::new(e, span))?;

        if !after_names.is_empty() || has_closure_callbacks(&class.name, after_events) {
            if let Value::Hash(result_hash) = &result {
                let valid = result_hash
                    .borrow()
                    .get(&HashKey::String("valid".into()))
                    .cloned();
                let record = result_hash
                    .borrow()
                    .get(&HashKey::String("record".into()))
                    .cloned();
                if matches!(valid, Some(Value::Bool(true))) {
                    if let Some(Value::Instance(inst)) = record {
                        self.run_model_callbacks_vm(
                            &class,
                            &inst,
                            &after_names,
                            after_events,
                            span,
                        )?;
                    }
                }
            }
        }

        self.stack.truncate(receiver_idx);
        self.stack.push(result);
        Ok(true)
    }

    /// `Model.delete(id)` when the class has dependents or attachments:
    /// load the row and run the instance-delete wrap. Returns `false` when
    /// the class has neither, so the ordinary native static runs.
    fn try_call_model_class_delete(
        &mut self,
        receiver_idx: usize,
        span: Span,
    ) -> Result<bool, RuntimeError> {
        use crate::interpreter::builtins::model::{
            class_name_to_collection, crud, get_model_class,
        };
        use crate::interpreter::executor::calls::cascade;

        let class = match &self.stack[receiver_idx] {
            Value::Class(c) => c.clone(),
            _ => return Ok(false),
        };
        let has_dependents = cascade::class_declares_dependents(&class.name);
        let has_attachments =
            !crate::interpreter::builtins::model::get_uploaders(&class.name).is_empty();
        if !has_dependents && !has_attachments {
            return Ok(false);
        }
        // This path reaches `crud::exec_get` directly, bypassing the bound
        // native's columnar check — raise here instead of reading a document
        // out of a columnar store.
        Self::reject_columnar_document_api(&class, "delete", span)?;
        let id_val = self.stack[receiver_idx + 1].clone();
        let id = match &id_val {
            Value::String(s) => s.to_string(),
            other => {
                return Err(RuntimeError::new(
                    format!(
                        "Model.delete() expects string id, got {}",
                        other.type_name()
                    ),
                    span,
                ))
            }
        };

        let instance_value = if let Some(inst) = cascade::take_test_loaded_instance(&id) {
            inst
        } else {
            let collection = class_name_to_collection(&class.name);
            let doc = match crud::exec_get(&collection, &id) {
                Ok(doc) => doc,
                Err(e) => {
                    self.stack.truncate(receiver_idx);
                    self.stack
                        .push(Value::String(format!("Error: {}", e).into()));
                    return Ok(true);
                }
            };
            let model_class = get_model_class(&class.name).unwrap_or(class);
            crud::json_doc_to_instance(&model_class, &doc)
        };
        let result = self.delete_model_instance_vm(&instance_value, span)?;
        self.stack.truncate(receiver_idx);
        self.stack.push(result);
        Ok(true)
    }

    fn sm_current_tag(
        inst: &Rc<RefCell<Instance>>,
        machine: &crate::interpreter::builtins::model::state_machine::StateMachineDef,
    ) -> Option<String> {
        let b = inst.borrow();
        match b.fields.get(machine.field.as_str()) {
            Some(Value::Instance(e)) => e.borrow().fields.get("__variant").and_then(|v| match v {
                Value::String(s) => Some(s.to_string()),
                _ => None,
            }),
            Some(Value::String(s)) => Some(s.to_string()),
            _ => machine.initial.clone(),
        }
    }

    /// State-machine guards and transition hooks are closures declared in the
    /// model body, so they carry a captured environment the bytecode path cannot
    /// reconstruct — the same reason `ensure_callbacks_vm_ready` refuses
    /// closure-form model callbacks. Refuse so the tree-walker runs them, which
    /// is what happened before instance methods ran on the VM at all.
    ///
    /// Guards and `before_transition` hooks run before the state is written, so
    /// refusing here costs nothing. `after_transition` runs after the write, so
    /// `sm_fire_event_vm` checks for those up front instead.
    fn run_sm_closure_vm(
        &mut self,
        _inst: &Rc<RefCell<Instance>>,
        closure: &Function,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        Err(RuntimeError::EngineFallback(
            format!("state-machine closure '{}'", closure.name),
            span,
        ))
    }

    /// State-machine event/predicate on a model instance (`pay`, `pay!`,
    /// `can_pay?`, `paid?`). `None` if this isn't an SM member.
    pub(crate) fn try_sm_dispatch(
        &mut self,
        inst: &Rc<RefCell<Instance>>,
        name: &str,
        span: Span,
    ) -> Result<Option<Value>, RuntimeError> {
        use crate::interpreter::builtins::model::state_machine as sm;

        let class = inst.borrow().class.clone();
        if !class.is_model_subclass() {
            return Ok(None);
        }
        let machines = sm::machines_for(&class.name);
        if machines.is_empty() {
            return Ok(None);
        }
        if class.find_method(name).is_some() || class.find_vm_method(name).is_some() {
            return Ok(None);
        }

        for machine in &machines {
            if let Some(stem) = name.strip_suffix('?') {
                if let Some(event) = stem.strip_prefix("can_") {
                    if machine.event(event).is_some() {
                        let ok = self.sm_can_vm(inst, machine, event, span)?;
                        return Ok(Some(Value::Bool(ok)));
                    }
                }
                if let Some(tag) = machine.states.iter().find(|t| sm::snake_case(t) == stem) {
                    let current = Self::sm_current_tag(inst, machine);
                    return Ok(Some(Value::Bool(current.as_deref() == Some(tag.as_str()))));
                }
            }
            let (event_name, persist) = match name.strip_suffix('!') {
                Some(stem) => (stem, true),
                None => (name, false),
            };
            if machine.event(event_name).is_some() {
                let result = self.sm_fire_event_vm(inst, machine, event_name, persist, span)?;
                return Ok(Some(result));
            }
        }
        Ok(None)
    }

    fn sm_can_vm(
        &mut self,
        inst: &Rc<RefCell<Instance>>,
        machine: &crate::interpreter::builtins::model::state_machine::StateMachineDef,
        event: &str,
        span: Span,
    ) -> Result<bool, RuntimeError> {
        use crate::interpreter::builtins::model::state_machine as sm;
        let Some(current) = Self::sm_current_tag(inst, machine) else {
            return Ok(false);
        };
        if machine.target_for(event, &current).is_none() {
            return Ok(false);
        }
        let class_name = inst.borrow().class.name.clone();
        if let Some(guard) = sm::lookup_guard(&class_name, event) {
            let v = self.run_sm_closure_vm(inst, &guard, span)?;
            return Ok(!matches!(v, Value::Bool(false) | Value::Null));
        }
        Ok(true)
    }

    fn sm_fire_event_vm(
        &mut self,
        inst: &Rc<RefCell<Instance>>,
        machine: &crate::interpreter::builtins::model::state_machine::StateMachineDef,
        event: &str,
        persist: bool,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        use crate::interpreter::builtins::model::state_machine as sm;

        let class_name = inst.borrow().class.name.clone();
        let current = Self::sm_current_tag(inst, machine).ok_or_else(|| RuntimeError::General {
            message: format!(
                "{}: state field '{}' is unset and the machine has no initial state",
                class_name, machine.field
            ),
            span,
        })?;
        let to_tag = match machine.target_for(event, &current) {
            Some(t) => t.to_string(),
            None => {
                return Err(RuntimeError::General {
                    message: format!(
                        "{}: cannot '{}' from state '{}'",
                        class_name, event, current
                    ),
                    span,
                })
            }
        };

        // `after_transition` hooks run once the new state is written (and, with
        // `!`, persisted), so a refusal from inside that loop would arrive with
        // the row already changed and the handler would repeat the write on the
        // tree-walker. Decide now, while nothing has been mutated.
        if !sm::lookup_after(&class_name, &to_tag).is_empty() {
            return Err(RuntimeError::EngineFallback(
                format!("after_transition hook on '{}'", class_name),
                span,
            ));
        }

        if let Some(guard) = sm::lookup_guard(&class_name, event) {
            let v = self.run_sm_closure_vm(inst, &guard, span)?;
            if matches!(v, Value::Bool(false) | Value::Null) {
                return Err(RuntimeError::General {
                    message: format!("{}: guard for '{}' failed", class_name, event),
                    span,
                });
            }
        }

        for hook in sm::lookup_before(&class_name, &to_tag) {
            let r = self.run_sm_closure_vm(inst, &hook, span)?;
            if matches!(r, Value::Bool(false)) {
                return Err(RuntimeError::General {
                    message: format!(
                        "{}: before_transition to '{}' vetoed '{}'",
                        class_name, to_tag, event
                    ),
                    span,
                });
            }
        }

        let new_value =
            sm::build_state_value(&class_name, &machine.field, &to_tag).ok_or_else(|| {
                RuntimeError::General {
                    message: format!(
                        "{}: state_machine field '{}' is not declared with enum_field",
                        class_name, machine.field
                    ),
                    span,
                }
            })?;
        inst.borrow_mut().set(machine.field.clone(), new_value);

        if persist {
            if let Some(native) = inst.borrow().class.find_native_method("save") {
                let receiver_idx = self.stack.len();
                self.push(Value::Instance(inst.clone()));
                self.call_model_instance_native(
                    inst.clone(),
                    native,
                    receiver_idx,
                    0,
                    "save",
                    span,
                )?;
                let _ = self.pop();
            }
        }

        for hook in sm::lookup_after(&class_name, &to_tag) {
            self.run_sm_closure_vm(inst, &hook, span)?;
        }

        Ok(Value::Bool(true))
    }

    fn call_class_method_missing(
        &mut self,
        receiver_idx: usize,
        argc: usize,
        name: &str,
        span: Span,
    ) -> Result<(), RuntimeError> {
        let class = match &self.stack[receiver_idx] {
            Value::Class(c) => c.clone(),
            _ => unreachable!(),
        };
        let user_args: Vec<Value> = self.stack[receiver_idx + 1..receiver_idx + 1 + argc].to_vec();
        let name_val = Value::String(name.to_string().into());
        let args_arr = Value::Array(Rc::new(RefCell::new(user_args.clone())));

        if let Some(closure) = class.find_vm_static_method("method_missing") {
            self.stack.truncate(receiver_idx);
            self.push(Value::Class(class));
            self.push(name_val);
            self.push(args_arr);
            return self.call_closure(closure, 2, span);
        }

        let class_val = Value::Class(class.clone());
        let Some(bound) = crate::interpreter::executor::access::member::bind_class_method_missing(
            &class, &class_val, name,
        ) else {
            return Err(RuntimeError::NoSuchProperty {
                value_type: class.name.clone(),
                property: name.to_string(),
                span,
            });
        };
        let result = self.invoke_callable(bound, &user_args, span)?;
        self.stack.truncate(receiver_idx);
        self.stack.push(result);
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
        //
        // A compile failure is an `EngineFallback`, never a general error: the
        // callee is fine, only *this engine* can't run it, so serve mode must
        // re-run the request on the tree-walker. A general error would be
        // routed through user-level `try`/`rescue`, and a handler that wrapped
        // the call would swallow the VM's internal limitation as if it were an
        // application error — returning a rescue value instead of demoting.
        let proto = jit_compile_function(func, self.globals.keys().cloned()).map_err(|e| {
            RuntimeError::EngineFallback(format!("a function the VM cannot compile ({})", e), span)
        })?;

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
        if class.is_module {
            return Err(RuntimeError::type_error(
                format!("cannot instantiate module {}", class.name),
                span,
            ));
        }
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
            let proto = jit_compile_method(&ctor, self.globals.keys().cloned()).map_err(|e| {
                RuntimeError::EngineFallback(
                    format!("a function the VM cannot compile ({})", e),
                    span,
                )
            })?;
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
        let proto = jit_compile_method(method, self.globals.keys().cloned()).map_err(|e| {
            RuntimeError::EngineFallback(format!("a function the VM cannot compile ({})", e), span)
        })?;
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
    /// access (tree-walker parity). Native instance methods invoke directly
    /// (no per-call bound-wrapper alloc). Everything else goes through
    /// property lookup + call_value.
    pub(crate) fn call_method_slow_path(
        &mut self,
        receiver_idx: usize,
        argc: usize,
        name: &str,
    ) -> Result<(), RuntimeError> {
        let class_receiver = match &self.stack[receiver_idx] {
            Value::Class(class) => Some(class.clone()),
            _ => None,
        };
        if let Some(class) = class_receiver {
            let is_model = class.is_model_subclass();
            if is_model {
                if name == "transaction"
                    && argc == 1
                    && Self::is_callable_block(&self.stack[receiver_idx + 1])
                {
                    let span = self.current_span();
                    return self.call_model_transaction_block(receiver_idx, span);
                }
                if matches!(name, "create" | "update") {
                    let span = self.current_span();
                    if self.try_call_model_class_persist(receiver_idx, argc, name, span)? {
                        return Ok(());
                    }
                }
                if name == "delete" && argc == 1 {
                    let span = self.current_span();
                    if self.try_call_model_class_delete(receiver_idx, span)? {
                        return Ok(());
                    }
                }
            }
            // Class-level `method_missing` is the LAST resort, matching
            // `class_member_access`: statics, then the reflection surface
            // (`send`, `methods`, `define_method`, …), then dynamic finders.
            // Consulting it first meant that on any class defining a static
            // `method_missing` — the `Mailer` prelude shape — `Foo.send("bar")`
            // and `User.find_by_email("x")` dispatched into `method_missing`
            // and returned a wrong value instead of the reflection result.
            //
            // `known` short-circuits, and `method_missing` is looked up only
            // when nothing else matched, so a resolved static (`User.where`)
            // costs one superclass walk rather than the five this used to do on
            // every class-receiver call.
            let known = class.find_vm_static_method(name).is_some()
                || class.find_static_method(name).is_some()
                || class.find_native_static_method(name).is_some()
                || crate::vm::vm_classes::is_class_reflection_member(name)
                || (is_model && name.starts_with("find_by_"));
            if !known
                && (class.find_static_method("method_missing").is_some()
                    || class.find_vm_static_method("method_missing").is_some())
            {
                let span = self.current_span();
                return self.call_class_method_missing(receiver_idx, argc, name, span);
            }
        }
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
            self.ensure_call_depth(self.current_span())?;
            self.frames.push(CallFrame::new(
                closure,
                stack_base,
                self.iter_stack.len(),
                Some(defining_class),
                crate::vm::vm::positional_supplied_mask(argc),
            ));
            return Ok(());
        }

        // Direct native instance-method call: skip bind_native_method_to_instance
        // (which allocated a fresh NativeFunction + closure on every call).
        // Fields shadow methods — same order as instance_member_access.
        // Model persist/delete natives go through `call_model_instance_native`
        // so lifecycle callbacks, cascades, and attachment purge run here.
        // Resolve the native method and release the borrow on the value stack
        // before calling, so the arguments can be passed as a *slice* of the
        // stack rather than copied into a fresh `Vec`. That copy ran on every
        // native instance-method call including zero-argument ones, which is
        // most of them (`dt.year()`, `dt.to_unix()`, `d.total_seconds()`).
        let resolved = if let Value::Instance(inst) = &self.stack[receiver_idx] {
            let inst_ref = inst.borrow();
            if inst_ref.fields.contains_key(name) {
                None
            } else {
                let is_model = inst_ref.class.is_model_subclass();
                inst_ref
                    .class
                    .find_native_method(name)
                    .map(|native| (native, is_model, inst.clone()))
            }
        } else {
            None
        };
        if let Some((native, is_model, inst)) = resolved {
            let span = self.current_span();
            if is_model {
                return self.call_model_instance_native(
                    inst,
                    native,
                    receiver_idx,
                    argc,
                    name,
                    span,
                );
            }
            if let Some(expected) = native.arity {
                if argc != expected {
                    return Err(RuntimeError::wrong_arity(expected, argc, span));
                }
            }
            let _native_span = crate::serve::span_log::maybe_instrument_native(&native.name);
            let result = {
                let user_args = &self.stack[receiver_idx + 1..receiver_idx + 1 + argc];
                crate::interpreter::executor::access::member::call_native_instance_method(
                    &inst, &native, user_args,
                )
            }
            .map_err(|e| RuntimeError::new(e, span))?;
            drop(_native_span);
            self.stack.truncate(receiver_idx);
            self.stack.push(result);
            return Ok(());
        }

        let span = self.current_span();
        if argc == 0 {
            if let Value::Instance(inst) = &self.stack[receiver_idx] {
                let inst = inst.clone();
                if let Some(result) = self.try_sm_dispatch(&inst, name, span)? {
                    self.stack.truncate(receiver_idx);
                    self.stack.push(result);
                    return Ok(());
                }
            }
        }
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
            Value::String(s) => self.vm_call_string_method(s, method_name, &args, span)?,
            Value::Hash(hash) => self.vm_call_hash_method(hash, method_name, &args, span)?,
            Value::Int(_)
            | Value::Float(_)
            | Value::Bool(_)
            | Value::Null
            | Value::Decimal(_)
            | Value::DateTime(_) => {
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
        args: &[Value],
        span: Span,
    ) -> Result<Value, RuntimeError> {
        let func = self
            .globals
            .get(name)
            .cloned()
            .ok_or_else(|| RuntimeError::undefined_variable(name, span))?;

        self.push(func);
        for arg in args {
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
        args: &[Value],
        span: Span,
    ) -> Result<Value, RuntimeError> {
        self.push(callee);
        let argc = args.len();
        for arg in args {
            self.push(arg.clone());
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
                            RuntimeError::EngineFallback(
                                format!("a function the VM cannot compile ({})", e),
                                span,
                            )
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
        args: &[Value],
        span: Span,
    ) -> Result<Value, RuntimeError> {
        let saved_depth = self.return_depth;
        let frames_before = self.frames.len();
        self.push(callee);
        let argc = args.len();
        for arg in args {
            self.push(arg.clone());
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

    /// Arbitrary-arity form of [`Self::invoke_in_batch_one`], for callers that
    /// forward a caller-supplied argument list — a function held in a hash
    /// entry, say. The batch is what bounds the nested `run()`; calling
    /// `call_value_direct` from inside a dispatch handler instead lets `run()`
    /// unwind past the bottom frame and panic.
    pub fn invoke_in_batch(
        &mut self,
        batch: &CallableBatch,
        callee: &Value,
        args: &[Value],
        span: Span,
    ) -> Result<Value, RuntimeError> {
        self.push(callee.clone());
        for arg in args {
            self.push(arg.clone());
        }
        self.call_value(args.len(), span)?;
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

    /// A model callback must see the record as `this`.
    ///
    /// The previous `bind_callback_method` defined `this` in a bound
    /// `Environment`, but `invoke_callable` recompiled the body as a plain
    /// function and dropped that environment, so `this` resolved to the callee
    /// slot and `this.touched = 1` failed with "Cannot set property on
    /// Function". For an `after_save` callback the row was already written when
    /// that error surfaced, so the handler demoted and wrote it a second time.
    #[test]
    fn callback_binds_this_to_the_record() {
        use crate::lexer::Scanner;
        use crate::parser::Parser;

        let body = Parser::new(
            Scanner::new("this.touched = 1")
                .scan_tokens()
                .expect("lexer error"),
        )
        .parse()
        .expect("parser error")
        .statements;
        let method = Function {
            name: "stamp".to_string(),
            body: body.into(),
            is_method: true,
            ..Function::default()
        };

        let class = Rc::new(Class {
            name: "Rec".to_string(),
            ..Default::default()
        });
        let inst = Rc::new(RefCell::new(Instance::new(class.clone())));

        let mut vm = Vm::new();
        let proto = vm
            .callback_proto(&method, Span::default())
            .expect("callback should compile as a method");
        let closure = Rc::new(VmClosure::new(proto, Vec::new()));
        vm.invoke_zero_arg_instance_method(&inst, closure, class, Span::default())
            .expect("callback should run with `this` bound to the instance");

        assert_eq!(
            inst.borrow().get("touched"),
            Some(Value::Int(1)),
            "the callback must write to the record, not to its own callee slot"
        );
    }

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

    #[test]
    fn jit_compile_failure_is_uncatchable_fallback() {
        // A tree-walker function the VM cannot compile (here: its body makes a
        // named-argument call) must surface as an EngineFallback, so serve mode
        // demotes the handler to the interpreter. A general error would be
        // routed through user-level try/rescue, letting a handler that wrapped
        // the call swallow the VM's own limitation as an application error and
        // silently return a rescue value instead.
        use crate::lexer::Scanner;
        use crate::parser::Parser;
        use crate::vm::Compiler;

        // `helper` is a tree-walking Function whose body cannot be compiled.
        //
        // The construct does not matter — this test is about EngineFallback's
        // routing — but every hard-coded choice so far has been overtaken by
        // the compiler learning to handle it: `break`, then `next`, then safe
        // navigation, each breaking this test's premise in turn. So rather than
        // name a fourth, take the first candidate the compiler still refuses.
        // The test then keeps testing what it means to test, and only fails for
        // real if NOTHING is refused any more — at which point EngineFallback
        // has no callers left and this test should be deleted, which is exactly
        // what the message says.
        let candidates = [
            "let x = `echo hi`",                             // command substitution
            "for i in [1, 2] { debug }",                     // sentinel builtin
            "for i in [1, 2] { try { break } catch e { } }", // break under a try
            "let r = [[y for y in [1, 2]]]",                 // comprehension as a sub-expression
        ];
        let body_src = candidates
            .iter()
            .copied()
            .find(|src| {
                let toks = match Scanner::new(src).scan_tokens() {
                    Ok(t) => t,
                    Err(_) => return false,
                };
                let prog = match Parser::new(toks).parse() {
                    Ok(p) => p,
                    Err(_) => return false,
                };
                Compiler::compile(&prog).is_err()
            })
            .expect(
                "no candidate is refused by the compiler any more — if nothing \
                 is, EngineFallback has no callers and this test should go",
            );
        let body_tokens = Scanner::new(body_src).scan_tokens().expect("lexer error");
        let body = Parser::new(body_tokens)
            .parse()
            .expect("parser error")
            .statements;
        let helper = Value::Function(Rc::new(Function {
            name: "helper".to_string(),
            body: body.into(),
            ..Function::default()
        }));

        // Confirm the premise: this function really is uncompilable.
        let Value::Function(ref f) = helper else {
            unreachable!()
        };
        assert!(
            jit_compile_function(f, std::iter::empty()).is_err(),
            "test premise broken: `helper` should fail to compile"
        );

        for source in [
            "let x = helper();",
            "let x = \"start\"\ntry { helper() } catch (e) { x = \"caught\" }",
            "let x = helper() rescue \"caught\";",
        ] {
            let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
            let program = Parser::new(tokens).parse().expect("parser error");
            let module = Compiler::compile(&program).expect("compile error");
            let mut vm = Vm::new();
            vm.globals.insert("helper".to_string(), helper.clone());
            match vm.execute(&module.main) {
                Err(err) => assert!(err.is_engine_fallback(), "{}: {}", source, err),
                Ok(_) => panic!("{}: expected EngineFallback, got Ok", source),
            }
        }
    }
}
