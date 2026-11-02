//! Function call dispatch for the VM.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use crate::ast::stmt::{FunctionDecl, Program, Stmt, StmtKind};
use crate::error::RuntimeError;
use crate::interpreter::executor::MAX_CALL_DEPTH;
use crate::interpreter::value::{
    Class, FieldInit, Function, HashKey, Instance, NativeFunction, Value, NATIVE_CONSTRUCTOR,
};
use crate::span::Span;

use super::chunk::{Constant, FunctionProto};

/// A compiled method and the class that defines it (for `super`).
type MethodWithClass = (Rc<VmClosure>, Rc<Class>);
use super::compiler::Compiler;
use super::upvalue::VmClosure;
use super::vm::{CallFrame, Vm};

/// The constructor a construction runs, as the VM runs it
/// ([`Vm::resolve_constructor`]).
pub(crate) enum Constructor {
    /// A compiled constructor and the class that declares it: the bytecode
    /// `init` of a class the VM compiled, or a tree-walker constructor
    /// compiled in constructor mode ([`Vm::compiled_constructor`]). It runs
    /// as an ordinary frame with `this` in the callee slot, and returns it.
    Compiled(Rc<VmClosure>, Rc<Class>),
    /// A native one (`__construct__`, e.g. `Error`'s message): it fills the
    /// instance it is handed.
    Native(Rc<NativeFunction>),
    /// No class of the chain has one.
    Absent,
}

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
    jit_compile_function_by(func, |program, source_path| {
        Compiler::compile_with_globals_from(program, globals, source_path)
    })
}

/// [`jit_compile_function`] with the known globals as a shared set: what the
/// worker warmup uses, so its thousand-odd compiles do not each copy the
/// worker's whole global table into a fresh set.
pub(crate) fn jit_compile_function_with_base(
    func: &Function,
    base: &super::compiler::GlobalNames,
) -> Result<Arc<FunctionProto>, String> {
    if let Some(proto) = func.jit_cache.borrow().clone() {
        return Ok(proto);
    }
    jit_compile_function_by(func, |program, source_path| {
        Compiler::compile_with_base_globals_from(program, base, source_path)
    })
}

fn compile_function_proto(
    func: &Function,
    compile: impl FnOnce(
        &Program,
        Option<Arc<std::path::PathBuf>>,
    ) -> crate::vm::compiler::CompileResult<crate::vm::chunk::CompiledModule>,
) -> Result<Arc<FunctionProto>, String> {
    let func_decl = FunctionDecl {
        name: func.name.clone(),
        params: func.params.to_vec(),
        return_type: None,
        body: (*func.body).clone(),
        span: func.span.unwrap_or_default(),
    };

    let program = Program::new(vec![Stmt {
        kind: StmtKind::function(func_decl),
        span: func.span.unwrap_or_default(),
        source_path: None,
    }]);

    // The function's file travels with it, so a coverage build marks its lines
    // (a service's static methods were compiled without one, and counted 0%).
    let source_path = func
        .source_path
        .as_ref()
        .map(|p| Arc::new(std::path::PathBuf::from(p)));
    let module = compile(&program, source_path).map_err(|e| e.to_string())?;

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
    // A function declared in a class nested in a module reads names in its
    // namespace — it, and the blocks compiled inside it.
    Ok(match crate::vm::compiler::namespace_of(func) {
        Some(namespace) => Arc::new(with_namespace(&proto, &namespace)),
        None => proto,
    })
}

/// `proto` and every function compiled inside it, with `namespace`.
fn with_namespace(proto: &FunctionProto, namespace: &Arc<str>) -> FunctionProto {
    let mut proto = proto.clone();
    proto.namespace = Some(namespace.clone());
    for constant in proto.chunk.constants.iter_mut() {
        if let Constant::Function(inner) = constant {
            *inner = Arc::new(with_namespace(inner, namespace));
        }
    }
    proto
}

fn jit_compile_function_by(
    func: &Function,
    compile: impl FnOnce(
        &Program,
        Option<Arc<std::path::PathBuf>>,
    ) -> crate::vm::compiler::CompileResult<crate::vm::chunk::CompiledModule>,
) -> Result<Arc<FunctionProto>, String> {
    let proto = compile_function_proto(func, compile)?;
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

/// [`jit_compile_method`] with the known globals as a shared set; see
/// [`jit_compile_function_with_base`].
pub(crate) fn jit_compile_method_with_base(
    func: &Function,
    base: &super::compiler::GlobalNames,
) -> Result<Arc<FunctionProto>, String> {
    if let Some(proto) = func.jit_cache.borrow().clone() {
        return Ok(proto);
    }
    let proto =
        Compiler::compile_method_standalone_with_base(func, base).map_err(|e| e.to_string())?;
    let arc = std::sync::Arc::new(proto);
    *func.jit_cache.borrow_mut() = Some(arc.clone());
    Ok(arc)
}

/// The closure a VM call to `func` runs: the one cached on `func` when its
/// proto is still `func.jit_cache`'s, so a call allocates nothing. `None`
/// before the first call has made one, or once the proto changed.
#[inline]
pub(crate) fn cached_function_closure(func: &Function) -> Option<Rc<VmClosure>> {
    let closure = func.jit_closure.borrow();
    let closure = closure.as_ref()?;
    let proto = func.jit_cache.borrow();
    if Arc::ptr_eq(&closure.proto, proto.as_ref()?) {
        Some(closure.clone())
    } else {
        None
    }
}

/// The closure over `proto` for `func`: the cached one when it wraps this
/// very proto, otherwise a new one, kept for the next call. The closure has
/// no upvalues and is never mutated, so one serves every call.
fn function_closure(func: &Function, proto: Arc<FunctionProto>) -> Rc<VmClosure> {
    let mut slot = func.jit_closure.borrow_mut();
    if let Some(closure) = slot.as_ref() {
        if Arc::ptr_eq(&closure.proto, &proto) {
            return closure.clone();
        }
    }
    let closure = Rc::new(VmClosure::new(proto, Vec::new()));
    *slot = Some(closure.clone());
    closure
}

// Whether a view helper's body can run on the view VM. Keyed by the body
// allocation. Only consulted for a function whose closure is the shared
// helper env, and dropped whenever helpers are reloaded — an address the
// allocator reused for some other function is never looked up.
thread_local! {
    #[allow(clippy::missing_const_for_thread_local)]
    static VIEW_HELPER_VM: RefCell<std::collections::HashMap<usize, bool>> =
        RefCell::new(std::collections::HashMap::new());
}

/// Drop every cached "this helper compiles" decision. Called when view
/// helpers are cleared or loaded again.
pub(crate) fn clear_view_helper_vm_cache() {
    VIEW_HELPER_VM.with(|cache| cache.borrow_mut().clear());
}

/// Bytecode for a view helper this VM can run itself, or `None` when the
/// call stays on the tree-walker.
///
/// A view runs with `tree_walk_functions` because a helper reads `req` and
/// the other request bindings through its closure, which the VM's globals
/// do not follow. A helper that names nothing outside its parameters needs
/// none of that, so it compiles. A free name, an assignment to a name the
/// closure already holds, a method, a function from anywhere else, or a
/// body the compiler refuses: the tree-walker, and not an `EngineFallback`
/// — that would demote the whole view.
fn view_helper_proto(func: &Function) -> Option<Arc<FunctionProto>> {
    if func.is_method
        || !crate::interpreter::builtins::template::is_view_helper_closure(&func.closure)
    {
        return None;
    }
    // The data address of the shared statements identifies the body.
    let key = func.body.as_ptr() as usize;
    let cached = VIEW_HELPER_VM.with(|cache| cache.borrow().get(&key).copied());
    if cached == Some(false) {
        return None;
    }
    if cached == Some(true) {
        if let Some(proto) = func.jit_cache.borrow().clone() {
            return Some(proto);
        }
    }

    // Names the closure already has, so an assignment to one resolves as a
    // global and is recorded. The worker's globals are the wrong set: `req`
    // is not one of them, and a helper assigning it must stay interpreted.
    // The proto is stored only when nothing was recorded, so the known-global
    // set never changes the bytecode that actually runs.
    let seen = Rc::new(RefCell::new(std::collections::HashSet::new()));
    let names = func.closure.borrow().get_all_bindings().into_keys();
    let seen_for_compile = Rc::clone(&seen);
    let compiled = compile_function_proto(func, move |program, source_path| {
        Compiler::compile_with_globals_recording(program, names, source_path, seen_for_compile)
    });
    let accepted = match compiled {
        Ok(proto) if seen.borrow().is_empty() => {
            *func.jit_cache.borrow_mut() = Some(proto);
            true
        }
        _ => false,
    };
    VIEW_HELPER_VM.with(|cache| cache.borrow_mut().insert(key, accepted));
    if accepted {
        func.jit_cache.borrow().clone()
    } else {
        None
    }
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
pub(crate) fn bind_named_arguments(
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
    pub(crate) fn ensure_call_depth(
        &self,
        callee: &crate::vm::chunk::FunctionProto,
        span: Span,
    ) -> Result<(), RuntimeError> {
        if self.frames.len() >= MAX_CALL_DEPTH {
            // Worded and placed as the tree-walker words and places it: the
            // function's name, at its declaration (a lambda has none, so the
            // call site stands in).
            let at = if callee.decl_span.line > 0 {
                callee.decl_span
            } else {
                span
            };
            return Err(RuntimeError::new(
                format!(
                    "call stack too deep ({MAX_CALL_DEPTH} frames) in \"{}\" — unbounded recursion?",
                    callee.name
                ),
                at,
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
        if class.is_none() && closure.proto.kernel.is_some() && self.try_kernel(&closure, argc) {
            return Ok(());
        }
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

        self.ensure_call_depth(&closure.proto, span)?;

        self.frames.push(CallFrame::new(
            closure,
            stack_base,
            self.iter_stack.len(),
            class,
            crate::vm::vm::positional_supplied_mask(argc),
        ));

        Ok(())
    }

    /// Run `closure`'s native kernel on the `argc` arguments at the top of the
    /// stack (the callee sits just beneath them). On success the callee and
    /// its arguments are replaced by the value, as a native call leaves them,
    /// and no frame is opened. `false` means declined: nothing was touched,
    /// and the caller opens the frame as usual.
    #[inline]
    pub(crate) fn try_kernel(&mut self, closure: &VmClosure, argc: usize) -> bool {
        let Some(kernel) = &closure.proto.kernel else {
            return false;
        };
        let base = self.stack.len() - argc;
        let globals = &self.globals;
        let bound = |name: &str| match globals.get(name) {
            Some(Value::VmClosure(c)) => c.proto.kernel.as_ref().map(|k| k.id()),
            _ => None,
        };
        match kernel.try_call(&self.stack[base..], self.frames.len(), bound) {
            Some(value) => {
                self.stack.truncate(base - 1);
                self.stack.push(value);
                true
            }
            None => false,
        }
    }

    /// Push `slots` (already in parameter order) and open a frame with an
    /// explicit supplied-parameter mask.
    ///
    /// The positional entry points derive the mask from `argc` because the
    /// caller fills a prefix of the parameters. A named call can fill any
    /// subset, so it computes the mask itself and hands it over here; the
    /// callee's default-value prologue then runs for exactly the unfilled
    /// parameters.
    pub(crate) fn call_closure_with_slots(
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
        self.ensure_call_depth(&closure.proto, self.current_span())?;
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
            // `amount.with(cents: 600)` — the property read handed back a
            // marker for it (see `op_get_property`).
            Value::Method(ref method)
                if method.method_name == "with"
                    && Self::is_value_object_for_with(&method.receiver).is_some() =>
            {
                let inst = Self::is_value_object_for_with(&method.receiver).expect("checked");
                self.stack.truncate(callee_idx);
                let mut changes = match positional.as_slice() {
                    [] => std::collections::HashMap::new(),
                    [single] => crate::interpreter::domain::changes_from(Some(single), "with")
                        .map_err(|e| RuntimeError::new(e, span))?,
                    _ => {
                        return Err(RuntimeError::new(
                            "with(...) takes the changes as named arguments or one hash",
                            span,
                        ))
                    }
                };
                for (name, value) in named {
                    changes.insert(name.to_string(), value);
                }
                self.value_object_with(&inst, changes, span)
            }
            // `Config(port: 3000)` — bind against the compiled constructor and
            // let it run with `this` in the callee slot.
            Value::Class(ref class) => {
                if crate::interpreter::domain::value_objects_declared() && class.is_value_object() {
                    self.stack.truncate(callee_idx);
                    return self.build_value_object(class, positional, named, span);
                }
                if let Some((ctor, defining_class)) = class.find_vm_method_with_class("init") {
                    let instance = Rc::new(RefCell::new(Instance::new(class.clone())));
                    self.init_fields(class, &instance, span)?;
                    let (slots, supplied) =
                        bind_named_arguments(&ctor.proto, positional, named, span)?;
                    self.stack[callee_idx] = Value::Instance(instance);
                    self.call_closure_with_slots(ctor, slots, supplied, Some(defining_class))
                } else if class.find_constructor().is_none()
                    && class.find_native_method(NATIVE_CONSTRUCTOR).is_none()
                {
                    // No constructor anywhere: the arguments go nowhere, as
                    // on the tree-walker (`new Point(x: 1)` on a class that
                    // only declares fields).
                    let instance = Rc::new(RefCell::new(Instance::new(class.clone())));
                    self.init_fields(class, &instance, span)?;
                    self.stack.truncate(callee_idx);
                    self.push(Value::Instance(instance));
                    Ok(())
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
        if native.name == "after_commit" {
            return self.call_after_commit_block(args, span);
        }
        if native.name == "with_tenant" {
            return self.call_with_tenant_block(args, span);
        }
        // SEC-013b: `render_json(record)` sends what the class's `as_json`
        // returns, as on the tree-walker (`try_evaluate_render_json_with_as_json`).
        // The native has no engine to call a user method with, so on the VM
        // `as_json` was skipped and every field went out — the very fields an
        // `as_json` is written to leave out (a password digest, a token).
        if native.name == "render_json" {
            if let Some(Value::Instance(inst)) = args.first() {
                let class = inst.borrow().class.clone();
                if class.find_vm_method_with_class("as_json").is_some()
                    || class.find_method("as_json").is_some()
                {
                    let record = args[0].clone();
                    args[0] = self.op_get_property_member(&record, "as_json", span)?;
                }
            }
        }
        if native.name == "assert_raises" {
            if let Some((fragment, block)) =
                crate::interpreter::builtins::assertions::assert_raises_args(args.clone())
            {
                crate::interpreter::builtins::assertions::clear_assertion_failed();
                let outcome = self.invoke_callable(block, &[], span);
                let value = crate::interpreter::builtins::assertions::judge_raised(
                    outcome,
                    fragment.as_deref(),
                    span,
                )?;
                self.push(value);
                return Ok(());
            }
        }
        // `defined` and `const_get` read the tree-walker's `CURRENT_ENV`,
        // which the VM never sets: on the VM, `defined("ShopChannel")` was
        // false for a class the app had loaded, and an action took the branch
        // meant for an app without it. The VM's globals answer first; a name
        // they lack still goes to the native, as before.
        if native.name == "defined" || native.name == "const_get" {
            if let Some(Value::String(name)) = args.first() {
                if let Some(value) = self.globals.get(name.as_str()) {
                    let result = if native.name == "defined" {
                        Value::Bool(true)
                    } else {
                        value.clone()
                    };
                    self.push(result);
                    return Ok(());
                }
            }
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

    /// `with_tenant(key, options?) do … end` — see the tree-walker's
    /// `try_evaluate_with_tenant`.
    fn call_with_tenant_block(
        &mut self,
        mut args: Vec<Value>,
        span: Span,
    ) -> Result<(), RuntimeError> {
        if !args.last().is_some_and(Self::is_callable_block) {
            return Err(RuntimeError::new(
                "with_tenant(key, options?) expects a block: with_tenant(\"acme\") do … end",
                span,
            ));
        }
        let block = args.pop().expect("checked above");
        let general = |message: String| RuntimeError::General { message, span };
        let tenant = crate::interpreter::builtins::tenancy::tenant_from_args(&args, "with_tenant")
            .map_err(general)?;
        let result = crate::interpreter::builtins::tenancy::with_tenant(
            tenant,
            || self.invoke_callable(block.clone(), &[], span),
            general,
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
            |e| e,
        )?;
        for block in crate::interpreter::events::take_after_commit() {
            self.invoke_callable(block, &[], span)?;
        }
        self.push(result);
        Ok(())
    }

    /// `after_commit { … }`: queued for the outermost commit inside a
    /// transaction, run now outside one.
    fn call_after_commit_block(
        &mut self,
        args: Vec<Value>,
        span: Span,
    ) -> Result<(), RuntimeError> {
        let Some(block) = args.into_iter().next().filter(Self::is_callable_block) else {
            return Err(RuntimeError::new("after_commit expects a block", span));
        };
        if crate::interpreter::builtins::model::crud::has_active_tx() {
            crate::interpreter::events::queue_after_commit(block);
        } else {
            self.invoke_callable(block, &[], span)?;
        }
        self.push(Value::Null);
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
    /// for these events cannot run correctly on the VM: a method whose body the
    /// compiler rejects. It must be detected here rather than at invocation
    /// time, because an `after_*` callback runs when the native write has
    /// already happened — demoting then re-runs the whole handler on the
    /// tree-walker and writes the row a second time. Refusing up front is
    /// side-effect-free, and the tree-walker runs these correctly.
    fn ensure_callbacks_vm_ready(
        &self,
        class: &Rc<Class>,
        event_sets: &[&[&str]],
        span: Span,
    ) -> Result<(), RuntimeError> {
        use crate::interpreter::executor::calls::function::callback_names_for;
        for events in event_sets {
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

    /// Run model callbacks, by method name, with `this` bound to
    /// `instance`. `Ok(false)` is a `before_*` veto (first `false` wins).
    fn run_model_callbacks_vm(
        &mut self,
        class: &Rc<Class>,
        instance: &Rc<RefCell<Instance>>,
        callback_names: &[String],
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
            if let Some((method, declaring)) = class.find_method_with_class(cb_name) {
                // Compiled as a method with the instance in the callee slot, so
                // `this` inside the callback is the record — see `callback_proto`.
                // A callback inherited from a parent model runs as that
                // parent's: a `super` in it goes past the parent, not back to it.
                let proto = self.callback_proto(&method, span)?;
                let closure = Rc::new(VmClosure::new(proto, Vec::new()));
                let result =
                    self.invoke_zero_arg_instance_method(instance, closure, declaring, span)?;
                if matches!(result, Value::Bool(false)) {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    fn persist_wrap_needed(class_name: &str, before: &[&str], after: &[&str]) -> bool {
        use crate::interpreter::executor::calls::function::callback_names_for;
        !callback_names_for(class_name, before).is_empty()
            || !callback_names_for(class_name, after).is_empty()
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
                if !self.run_model_callbacks_vm(&class, &inst, &before_names, span)? {
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
                    self.run_model_callbacks_vm(&class, &inst, &after_names, span)?;
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
            callback_names_for, set_callback_aborted_error,
        };

        let before_events: &[&str] = &["before_delete"];
        let after_events: &[&str] = &["after_delete"];
        let has_dependents = cascade::class_declares_dependents(&class.name);
        let has_attachments =
            !crate::interpreter::builtins::model::get_uploaders(&class.name).is_empty();
        if !Self::persist_wrap_needed(&class.name, before_events, after_events)
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
        if !self.run_model_callbacks_vm(class, inst, &before_names, span)? {
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
            self.run_model_callbacks_vm(class, inst, &after_names, span)?;
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
            callback_names_for, persisted_record_for_after_callbacks, set_callback_aborted_error,
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
        if before_names.is_empty() && after_names.is_empty() {
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

        if !self.run_model_callbacks_vm(&class, &inst_rc, &before_names, span)? {
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

        if !after_names.is_empty() {
            let updated_id = (name == "update").then(|| native_args[1].clone());
            if let Some(record) =
                persisted_record_for_after_callbacks(&result, &inst_rc, updated_id.as_ref())
            {
                self.run_model_callbacks_vm(&class, &record, &after_names, span)?;
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

    /// State-machine guards and transition hooks are closures declared in the
    /// model body, so they carry a captured environment the bytecode path cannot
    /// reconstruct. Refuse so the tree-walker runs them, which is what happened
    /// before instance methods ran on the VM at all.
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
                    let current = machine.current_tag(inst);
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
        let Some(current) = machine.current_tag(inst) else {
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
        let current = machine
            .current_tag(inst)
            .ok_or_else(|| RuntimeError::General {
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
            // Bound before the call: an `if let` on `inst.borrow()` keeps the
            // borrow alive through its body, and `save` borrows `inst`
            // mutably (callbacks, the DB write).
            let save = inst.borrow().class.find_native_method("save");
            if let Some(native) = save {
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
        //
        // A view sets this flag. A helper that needs nothing from its closure
        // compiles and runs here; every other function still takes the
        // interpreter below, and a body the compiler refuses does too — an
        // `EngineFallback` from a view helper would demote the whole view.
        if self.tree_walk_functions {
            if let Some(proto) = view_helper_proto(func) {
                return self.call_compiled_function(func, proto, argc, span);
            }
            let callee_idx = self.stack.len() - 1 - argc;
            let args: Vec<Value> = self.stack.drain(callee_idx + 1..).collect();
            self.stack.pop();
            let mut interpreter = match &self.tree_walk_env {
                Some(env) => crate::interpreter::Interpreter::with_environment(env.clone()),
                None => crate::interpreter::Interpreter::for_bound_body(),
            };
            let result = interpreter.call_function_with_this(func, None, args)?;
            self.push(result);
            return Ok(());
        }
        let proto = jit_compile_function(func, self.globals.keys().cloned()).map_err(|e| {
            RuntimeError::EngineFallback(format!("a function the VM cannot compile ({})", e), span)
        })?;
        self.call_compiled_function(func, proto, argc, span)
    }

    fn call_compiled_function(
        &mut self,
        func: &Function,
        proto: Arc<FunctionProto>,
        argc: usize,
        span: Span,
    ) -> Result<(), RuntimeError> {
        let closure = function_closure(func, proto);
        // Replace the Function value on the stack with the compiled VmClosure.
        let callee_idx = self.stack.len() - 1 - argc;
        self.stack[callee_idx] = Value::VmClosure(closure.clone());
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
        if crate::interpreter::domain::value_objects_declared() && class.is_value_object() {
            return self.construct_value_object(class, argc, span);
        }
        let instance = Rc::new(RefCell::new(Instance::new(class.clone())));
        self.init_fields(class, &instance, span)?;
        let instance_val = Value::Instance(instance);

        match self.resolve_constructor(class, span)? {
            // A compiled constructor — the class's bytecode `init`, or a
            // tree-walker one compiled once in constructor mode — returns
            // `this`. The instance takes the callee slot, so it becomes slot
            // 0 (`this`) under the method calling convention, and the frame
            // records the class that declares the constructor, so the
            // `super(...)` inside reaches that class's superclass.
            Constructor::Compiled(ctor, defining_class) => {
                self.stack[callee_idx] = instance_val;
                self.call_closure_in_class(ctor, argc, span, Some(defining_class))
            }
            // Native constructor (a Rust-defined class such as RateLimiter):
            // it receives the instance and the arguments and fills the fields.
            Constructor::Native(init) => {
                let mut init_args = Vec::with_capacity(argc + 1);
                init_args.push(instance_val.clone());
                init_args.extend(self.stack[callee_idx + 1..].iter().cloned());
                self.stack.truncate(callee_idx);
                (init.func)(&init_args).map_err(|e| RuntimeError::new(e, span))?;
                self.push(instance_val);
                Ok(())
            }
            // No constructor: drop any args (tree-walker parity) and yield the
            // instance.
            Constructor::Absent => {
                self.stack.truncate(callee_idx);
                self.push(instance_val);
                Ok(())
            }
        }
    }

    /// The constructor a construction of `class` runs, in the order
    /// `call_class` has always looked: a bytecode `init` up the chain, then a
    /// tree-walker constructor, then a native one.
    pub(crate) fn resolve_constructor(
        &self,
        class: &Rc<Class>,
        span: Span,
    ) -> Result<Constructor, RuntimeError> {
        if let Some((init, defining_class)) = class.find_vm_method_with_class("init") {
            return Ok(Constructor::Compiled(init, defining_class));
        }
        if let Some((ctor, defining_class)) = class.find_constructor_with_class() {
            let closure = self.compiled_constructor(&ctor, &defining_class, span)?;
            return Ok(Constructor::Compiled(closure, defining_class));
        }
        Ok(match class.find_native_method(NATIVE_CONSTRUCTOR) {
            Some(init) => Constructor::Native(init),
            None => Constructor::Absent,
        })
    }

    /// `ctor`, the tree-walker constructor `declaring` declares (every class
    /// of an app under `soli serve` has its constructor as AST), compiled in
    /// constructor mode: the frame returns `this` however the body ends, so
    /// it runs as one ordinary frame, like the `init` of a class the VM
    /// compiled. Compiled the first time it runs and kept on the class
    /// (`Class::vm_constructor`); it was compiled as a method, wrapped in a
    /// new closure and run to completion in a nested `run()` on every `new`.
    pub(crate) fn compiled_constructor(
        &self,
        ctor: &Rc<Function>,
        declaring: &Class,
        span: Span,
    ) -> Result<Rc<VmClosure>, RuntimeError> {
        if let Some((compiled_from, closure)) = declaring.vm_constructor.borrow().as_ref() {
            if Rc::ptr_eq(compiled_from, ctor) {
                return Ok(closure.clone());
            }
        }
        let proto = Compiler::compile_constructor_standalone(ctor, self.globals.keys().cloned())
            .map_err(|e| {
                RuntimeError::EngineFallback(
                    format!("a function the VM cannot compile ({})", e),
                    span,
                )
            })?;
        let closure = Rc::new(VmClosure::new(Arc::new(proto), Vec::new()));
        *declaring.vm_constructor.borrow_mut() = Some((ctor.clone(), closure.clone()));
        Ok(closure)
    }

    /// Run `class`'s constructor on the instance in the callee slot
    /// (`[this, args…]`), leaving `this` — or a frame that returns it — in
    /// its place: `super(...)`. The fields were set when the instance was
    /// built (`init_fields`); a constructor only runs its body.
    pub(crate) fn call_init_of(
        &mut self,
        class: &Rc<Class>,
        argc: usize,
        span: Span,
    ) -> Result<(), RuntimeError> {
        let receiver_idx = self.stack.len() - 1 - argc;
        match self.resolve_constructor(class, span)? {
            // A compiled constructor — `this` is already in the callee slot,
            // and the frame returns it. A tree-walker one may be inherited
            // (`class K2 < K1` with no `new` of its own): it runs as K1's, so
            // a `super(...)` in it reaches K1's superclass, not K1 again.
            Constructor::Compiled(ctor, defining_class) => {
                self.call_closure_in_class(ctor, argc, span, Some(defining_class))?;
            }
            // A native constructor — `super(message)` in an `Error`
            // subclass. It takes `this` first, then the arguments; the
            // expression evaluates to `this`.
            Constructor::Native(init) => {
                let call_args: Vec<Value> = self.stack[receiver_idx..].to_vec();
                self.stack.truncate(receiver_idx + 1);
                (init.func)(&call_args).map_err(|e| RuntimeError::new(e, span))?;
            }
            // No constructor: drop args, evaluate to `this` (matching
            // tree-walker behavior).
            Constructor::Absent => self.stack.truncate(receiver_idx + 1),
        }
        Ok(())
    }

    /// Every declared instance field of `class` and its ancestors on
    /// `instance`, the farthest class first: its default, or `nil`. The
    /// tree-walker sets them before any constructor runs
    /// (`initialize_instance_fields`), whatever the constructor does and
    /// whether it calls `super`. So a subclass's default wins, a field no
    /// constructor assigns reads as `nil`, and `super(...)` never sets
    /// them again.
    ///
    /// Each class starts its fields (a literal default, else `nil`), then
    /// runs its defaults that are not literals, before the next class down
    /// starts its own: a subclass that declares a field again without a
    /// default resets it to `nil`, as on the tree-walker. Every write is
    /// raw, as the tree-walker's: a `const`, `private` or frozen field takes
    /// its default.
    pub(crate) fn init_fields(
        &mut self,
        class: &Rc<Class>,
        instance: &Rc<RefCell<Instance>>,
        span: Span,
    ) -> Result<(), RuntimeError> {
        if let Some(superclass) = &class.superclass {
            self.init_fields(superclass, instance, span)?;
        }
        let Some(init) = self.field_init_of(class, span)? else {
            return Ok(());
        };
        init.start(&mut instance.borrow_mut());
        if let Some(computed) = &init.computed {
            self.push(Value::Instance(instance.clone()));
            let values = self.run_to_completion(|vm| {
                vm.call_closure_in_class(computed.clone(), 0, span, Some(class.clone()))
            })?;
            if let Value::Hash(values) = values {
                let mut instance = instance.borrow_mut();
                for (name, value) in values.borrow().iter() {
                    if let HashKey::String(name) = name {
                        instance.fields.insert(name.clone(), value.clone());
                    }
                }
            }
        }
        Ok(())
    }

    /// `class`'s [`FieldInit`], or `None` when it declares no instance
    /// field. The VM records it when it compiles a class; a class the
    /// tree-walker defined (every class of a served app) gets one the first
    /// time the VM builds an instance of it, from its AST: the literal
    /// defaults, and one method compiled for the others.
    fn field_init_of(
        &mut self,
        class: &Rc<Class>,
        span: Span,
    ) -> Result<Option<Rc<FieldInit>>, RuntimeError> {
        if let Some(init) = class.field_init.borrow().as_ref() {
            return Ok(Some(init.clone()));
        }
        if class.fields.is_empty() {
            return Ok(None);
        }
        // Declaration order, then any field the declarations do not list.
        let declared = class.declared_fields.borrow();
        let mut names: Vec<&String> = declared
            .iter()
            .filter(|name| class.fields.contains_key(*name))
            .collect();
        let mut rest: Vec<&String> = class
            .fields
            .keys()
            .filter(|name| !declared.contains(*name))
            .collect();
        rest.sort();
        names.extend(rest);

        let mut fields = Vec::with_capacity(names.len());
        let mut computed = Vec::new();
        for name in names {
            let default = class.fields.get(name).and_then(Option::as_ref);
            let start = match default {
                None => Value::Null,
                Some(expr) => {
                    match crate::interpreter::executor::access::member::literal_value(expr) {
                        Some(value) => value,
                        None => {
                            computed.push((name.clone(), expr.clone()));
                            Value::Null
                        }
                    }
                }
            };
            fields.push((name.as_str().into(), start));
        }
        drop(declared);

        let computed = match super::compiler_classes::computed_defaults_method(&computed) {
            None => None,
            Some(method) => {
                // Compiled as one of the class's methods: with their
                // environment (its namespace) and their file.
                let (closure, source_path) = class
                    .constructor
                    .clone()
                    .or_else(|| class.methods.borrow().values().next().cloned())
                    .map(|f| (f.closure.clone(), f.source_path.clone()))
                    .unwrap_or_else(|| {
                        let env = crate::interpreter::environment::Environment::new();
                        (Rc::new(RefCell::new(env)), None)
                    });
                let func = Function::from_method(&method, closure, source_path);
                let proto =
                    jit_compile_method(&func, self.globals.keys().cloned()).map_err(|e| {
                        RuntimeError::EngineFallback(
                            format!("a field default the VM cannot compile ({})", e),
                            span,
                        )
                    })?;
                Some(Rc::new(VmClosure::new(proto, Vec::new())))
            }
        };
        let init = Rc::new(FieldInit { fields, computed });
        *class.field_init.borrow_mut() = Some(init.clone());
        Ok(Some(init))
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
    ///
    /// `class` is the class that declares the method. It goes on the frame,
    /// so a `super` inside the method resolves against that class's
    /// superclass, one level further up a chain each time — as the
    /// tree-walker resolves it from the class the method was written in.
    pub(crate) fn run_jit_method_to_completion(
        &mut self,
        method: &Rc<Function>,
        argc: usize,
        span: Span,
        class: Rc<Class>,
    ) -> Result<Value, RuntimeError> {
        let proto = jit_compile_method(method, self.globals.keys().cloned()).map_err(|e| {
            RuntimeError::EngineFallback(format!("a function the VM cannot compile ({})", e), span)
        })?;
        let closure = Rc::new(VmClosure::new(proto, Vec::new()));
        let saved_depth = self.return_depth;
        let frames_before = self.frames.len();
        self.return_depth = frames_before;
        let result = (|| -> Result<Value, RuntimeError> {
            self.call_closure_in_class(closure, argc, span, Some(class))?;
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
        if matches!(self.stack[receiver_idx], Value::QueryBuilder(_)) {
            let span = self.current_span();
            let args = self.stack.split_off(receiver_idx + 1);
            let receiver = self.stack.pop().expect("receiver below the arguments");
            let result = self.call_query_builder_method(receiver, name, args, span)?;
            self.push(result);
            return Ok(());
        }
        let class_receiver = match &self.stack[receiver_idx] {
            Value::Class(class) => Some(class.clone()),
            _ => None,
        };
        if let Some(class) = class_receiver {
            // `Klass.new(args)`: the constructor, as `new Klass(args)` — or,
            // for no arguments / one hash, the attribute form (see
            // `Interpreter::class_new`, the same rules).
            if name == "new" && crate::interpreter::executor::access::member::intercepts_new(&class)
            {
                return self.class_new(&class, receiver_idx);
            }
            let is_model = class.is_model_subclass();
            if is_model {
                // `Model.scope_name(args)` — `Post.by_user(5)`, `Measure.t()`.
                // The property path runs a bare scope read; a call must pass
                // its arguments to the scope closure instead, as
                // `call_query_builder_method` does for a scope on a builder.
                if Self::is_model_class_scope(&class, name) {
                    let span = self.current_span();
                    let args = self.stack.split_off(receiver_idx + 1);
                    let receiver = self.stack.pop().expect("receiver below the arguments");
                    let (mut interp, scope) = self.model_class_scope(&receiver, name, span)?;
                    let result = interp.call_value(scope, args, span)?;
                    self.push(result);
                    return Ok(());
                }
                if name == "transaction"
                    && argc == 1
                    && Self::is_callable_block(&self.stack[receiver_idx + 1])
                {
                    let span = self.current_span();
                    return self.call_model_transaction_block(receiver_idx, span);
                }
                // Batch iteration runs the block once per record while walking a
                // keyset cursor, which is query-builder machinery the VM does
                // not implement (see the `Value::QueryBuilder` arm in
                // `vm_classes.rs`). Demote the handler rather than failing it:
                // without this the call dies with "Cannot access property".
                if matches!(name, "find_each" | "in_batches" | "find_in_batches") {
                    return Err(RuntimeError::EngineFallback(
                        format!("model batch iteration '{}'", name),
                        self.current_span(),
                    ));
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
                // `Post.new()` / `Post.new(attrs)`: the attribute form, built
                // here as `class_member_access`'s `new` builds it — no
                // constructor runs for a model's `new`. Reaching it through
                // `op_get_property` cost a native per call, and before that an
                // interpreter seeded with every global (~35 µs).
                if name == "new"
                    && crate::interpreter::executor::access::member::model_attribute_new(&class)
                {
                    let span = self.current_span();
                    let instance =
                        crate::interpreter::executor::access::member::new_with_attributes(
                            &class,
                            &self.stack[receiver_idx + 1..receiver_idx + 1 + argc],
                        )
                        .map_err(|e| RuntimeError::new(e, span))?;
                    self.stack.truncate(receiver_idx);
                    self.push(instance);
                    return Ok(());
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
            // A parsed dynamic finder (`find_by_email`) is known, so
            // `method_missing` does not take it. `find_by_` parses as nothing
            // and stays free for `method_missing`; `find_by_sql` is a native
            // static, matched above.
            let known = class.find_vm_static_method(name).is_some()
                || class.find_static_method(name).is_some()
                || class.find_native_static_method(name).is_some()
                || crate::vm::vm_classes::is_class_reflection_member(name)
                || (is_model
                    && crate::interpreter::executor::access::member::parse_dynamic_finder(name)
                        .is_ok());
            if !known
                && (class.find_static_method("method_missing").is_some()
                    || class.find_vm_static_method("method_missing").is_some())
            {
                let span = self.current_span();
                return self.call_class_method_missing(receiver_idx, argc, name, span);
            }
        }
        // `obj.send("name", args…)` calls `name` with the rest of the
        // arguments, unless the class defines a `send` of its own — the
        // tree-walker's universal `send`. Without it a policy that dispatched
        // on the action (`policy.send(action + "?")`) failed on the VM and
        // sent its `before_action`, and the actions behind it, to the
        // interpreter for good.
        if name == "send" && argc >= 1 {
            if let Value::Instance(inst) = &self.stack[receiver_idx] {
                let class = inst.borrow().class.clone();
                let own_send = class.find_vm_method_with_class("send").is_some()
                    || class.find_method("send").is_some()
                    || class.find_native_method("send").is_some();
                if !own_send {
                    let target = match &self.stack[receiver_idx + 1] {
                        Value::String(s) | Value::Symbol(s) => Some(s.to_string()),
                        _ => None,
                    };
                    if let Some(target) = target {
                        self.stack.remove(receiver_idx + 1);
                        return self.call_method_slow_path(receiver_idx, argc - 1, &target);
                    }
                }
            }
        }
        let compiled = match &self.stack[receiver_idx] {
            Value::Instance(inst) => {
                let class = inst.borrow().class.clone();
                match class.find_vm_method_with_class(name) {
                    Some(found) => Some(found),
                    None => {
                        let shadowed_by_field = inst.borrow().fields.contains_key(name);
                        if shadowed_by_field {
                            None
                        } else {
                            self.compile_tree_walking_method(&class, name)?
                        }
                    }
                }
            }
            // Statics compile as plain functions; the class value left in
            // the callee slot is ignored by the bytecode — except by a
            // tree-walker `static def` that reads `this`, compiled as a
            // method of the class it is called on.
            Value::Class(class) => match class.find_vm_static_method(name) {
                Some(closure) => Some((closure, class.clone())),
                None => self.compile_tree_walking_static(class, name)?,
            },
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
            self.ensure_call_depth(&closure.proto, self.current_span())?;
            self.frames.push(CallFrame::new(
                closure,
                stack_base,
                self.iter_stack.len(),
                Some(defining_class),
                crate::vm::vm::positional_supplied_mask(argc),
            ));
            return Ok(());
        }

        // `amount.with(...)` on a value object with no `with` of its own.
        if name == "with" {
            if let Some(inst) = Self::is_value_object_for_with(&self.stack[receiver_idx]) {
                let span = self.current_span();
                let args: Vec<Value> = self.stack.drain(receiver_idx + 1..).collect();
                self.stack.truncate(receiver_idx);
                if args.len() > 1 {
                    return Err(RuntimeError::new(
                        "with(...) takes the changes as named arguments or one hash",
                        span,
                    ));
                }
                let changes = crate::interpreter::domain::changes_from(args.first(), "with")
                    .map_err(|e| RuntimeError::new(e, span))?;
                return self.value_object_with(&inst, changes, span);
            }
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
        let method_val = match self.op_get_property(&object, name, span) {
            Ok(method_val) => method_val,
            Err(err) => {
                // No such method: an instance `method_missing` takes the call,
                // as on the tree-walker.
                if let Value::Instance(inst) = &object {
                    let class = inst.borrow().class.clone();
                    let found = match class.find_vm_method_with_class("method_missing") {
                        Some(found) => Some(found),
                        None => self.compile_tree_walking_method(&class, "method_missing")?,
                    };
                    if let Some((closure, defining_class)) = found {
                        return self.call_instance_method_missing(
                            closure,
                            defining_class,
                            receiver_idx,
                            name,
                        );
                    }
                }
                return Err(err);
            }
        };
        if argc == 0 && !method_val.is_callable() {
            self.stack.truncate(receiver_idx);
            self.stack.push(method_val);
        } else {
            self.stack[receiver_idx] = method_val;
            self.call_value(argc, span)?;
        }
        Ok(())
    }

    /// `obj.name(args…)` answered by the instance's `method_missing`, the
    /// receiver at `receiver_idx` and the arguments above it. Its parameters
    /// bind as on the tree-walker (`bind_instance_method_missing_params`):
    /// the method name first, then the arguments by position; a last
    /// parameter named `args` takes every argument as an array, and one the
    /// call did not supply is `nil`.
    fn call_instance_method_missing(
        &mut self,
        closure: Rc<VmClosure>,
        defining_class: Rc<Class>,
        receiver_idx: usize,
        name: &str,
    ) -> Result<(), RuntimeError> {
        let mut mm_args = vec![Value::String(name.into())];
        mm_args.extend(self.stack.drain(receiver_idx + 1..));
        let params = closure.proto.param_names.len();
        for i in 0..params {
            let value = if i > 0 && i + 1 == params && closure.proto.param_names[i] == "args" {
                Value::Array(Rc::new(RefCell::new(mm_args[1..].to_vec())))
            } else {
                mm_args.get(i).cloned().unwrap_or(Value::Null)
            };
            self.stack.push(value);
        }
        self.ensure_call_depth(&closure.proto, self.current_span())?;
        self.frames.push(CallFrame::new(
            closure,
            receiver_idx,
            self.iter_stack.len(),
            Some(defining_class),
            crate::vm::vm::positional_supplied_mask(params),
        ));
        Ok(())
    }

    /// Compile an instance method that exists only as AST — one inherited from
    /// a class the VM never compiled, such as a controller's base class — as
    /// a *method* and register the bytecode on the class that defines it, so
    /// later calls take the compiled-method hot path.
    ///
    /// Without this the call fell through to `op_get_property`, which returns
    /// the unbound `Function`, and `call_native_wrapper` compiled it as a plain
    /// function and put the closure in the callee slot — the slot a method
    /// reads as `this`. `this._base()` inside it then failed with
    /// "Cannot access property '_base' on Function".
    pub(crate) fn compile_tree_walking_method(
        &self,
        class: &Rc<Class>,
        name: &str,
    ) -> Result<Option<MethodWithClass>, RuntimeError> {
        let mut defining = class.clone();
        let method = loop {
            let own = defining.methods.borrow().get(name).cloned();
            if let Some(method) = own {
                break method;
            }
            match defining.superclass.clone() {
                Some(superclass) => defining = superclass,
                None => return Ok(None),
            }
        };
        let proto = jit_compile_method(&method, self.globals.keys().cloned()).map_err(|e| {
            RuntimeError::EngineFallback(
                format!("a method the VM cannot compile ({})", e),
                self.current_span(),
            )
        })?;
        let closure = Rc::new(VmClosure::new(proto, Vec::new()));
        defining
            .vm_methods
            .borrow_mut()
            .insert(name.to_string(), closure.clone());
        Ok(Some((closure, defining)))
    }

    /// A tree-walker `static def` that reads `this`, compiled as a method:
    /// the class it is called on, in the callee slot, is its `this`. The
    /// other tree-walker statics keep the plain-function path.
    fn compile_tree_walking_static(
        &self,
        class: &Rc<Class>,
        name: &str,
    ) -> Result<Option<MethodWithClass>, RuntimeError> {
        let mut defining = class.clone();
        let method = loop {
            if let Some(method) = defining.static_methods.get(name) {
                break method.clone();
            }
            match defining.superclass.clone() {
                Some(superclass) => defining = superclass,
                None => return Ok(None),
            }
        };
        if !method.binds_this {
            return Ok(None);
        }
        let proto = jit_compile_method(&method, self.globals.keys().cloned()).map_err(|e| {
            RuntimeError::EngineFallback(
                format!("a method the VM cannot compile ({})", e),
                self.current_span(),
            )
        })?;
        let closure = Rc::new(VmClosure::new(proto, Vec::new()));
        defining
            .vm_static_methods
            .borrow_mut()
            .insert(name.to_string(), closure.clone());
        Ok(Some((closure, class.clone())))
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
            | Value::DateTime(_, _) => {
                self.vm_call_primitive_method(&receiver, method_name, &args, span)?
            }
            Value::QueryBuilder(_) => {
                self.call_query_builder_method(receiver, method_name, args, span)?
            }
            // A model helper resolved by the tree-walker's instance access —
            // uploader (`attach_photo`, `photo_url`), HABTM (`add_tag`) —
            // comes back as a bound method.
            Value::Instance(ref inst) if inst.borrow().class.is_model_subclass() => {
                self.call_model_helper_method(inst, method_name, args, span)?
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

    /// A model instance's HABTM (`add_tag`, `remove_tag`) or uploader
    /// method (`attach_photo`, `detach_photo`, `photo_url`, `photo_urls`),
    /// run as the tree-walker's `call_method` runs it: a HABTM join write, or
    /// a call to the app helper the uploader method forwards to, looked up in
    /// the VM's globals and called here. Running it in a tree-walker meant one
    /// seeded with a copy of every global on each call (~40 µs for a
    /// `photo_url()` in a view row), and a VM-compiled helper then cost a
    /// second VM.
    fn call_model_helper_method(
        &mut self,
        inst: &Rc<RefCell<Instance>>,
        method_name: &str,
        args: Vec<Value>,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        use crate::interpreter::builtins::model::habtm::{
            habtm_add, habtm_remove, match_habtm_method, HabtmAction,
        };
        use crate::interpreter::executor::access::member::{
            uploader_call, uploader_helper_args, uploader_helper_missing,
        };
        let class_name = inst.borrow().class.name.clone();
        if let Some(matched) = match_habtm_method(&class_name, method_name) {
            let result = match matched.action {
                HabtmAction::Add => habtm_add(inst, &matched.relation, &args),
                HabtmAction::Remove => habtm_remove(inst, &matched.relation, &args),
            };
            return result.map_err(|e| RuntimeError::new(e, span));
        }
        let (kind, field, helper_name) = uploader_call(method_name, span)?;
        let helper = self
            .globals
            .get(helper_name)
            .cloned()
            .ok_or_else(|| uploader_helper_missing(method_name, helper_name, span))?;
        let forwarded =
            uploader_helper_args(kind, field, Value::Instance(inst.clone()), args, span)?;
        self.invoke_callable(helper, &forwarded, span)
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
    /// seeds the call frame with `instance` at slot 0 and `arg` (when `Some`)
    /// at slot 1. A zero-parameter action (`def index`) passes `None`: it reads
    /// the request through the `req` global the server publishes beforehand.
    ///
    /// The compiled `FunctionProto` is cached on `method.jit_cache` so the AST
    /// walk in `Compiler::compile_method_standalone` only runs once per worker
    /// per method. (Each worker loads its own `Rc<Function>` instances in
    /// `load_controllers_in_worker`, so the `RefCell` cache is per-worker and
    /// has no cross-thread aliasing.) `warm_vm_handlers` pre-fills the cache at
    /// boot so the first request to a method is a cache hit, not a compile.
    ///
    /// The frame records the class that declares the action — the routed
    /// controller, or the ancestor it inherits the action from — so a
    /// `super.show(req)` in it resolves against that class's superclass, as
    /// on the tree-walker.
    pub fn call_method_bound(
        &mut self,
        method: &Function,
        instance: Value,
        arg: Option<Value>,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        let declaring = match &instance {
            Value::Instance(inst) => inst.borrow().class.declaring_class_of(method),
            _ => None,
        };
        self.call_method_bound_in(method, instance, arg.as_slice(), span, declaring)
    }

    /// `call_method_bound` with any number of arguments (a controller's
    /// after-action hook takes the request and the response). A hook is a
    /// lambda no class declares, and a bare static's receiver is the class
    /// itself: neither frame gets a class.
    pub fn call_method_bound_args(
        &mut self,
        method: &Function,
        instance: Value,
        args: &[Value],
        span: Span,
    ) -> Result<Value, RuntimeError> {
        self.call_method_bound_in(method, instance, args, span, None)
    }

    /// `call_method_bound_args`, with `class` — the class declaring `method`,
    /// when known — on the frame.
    fn call_method_bound_in(
        &mut self,
        method: &Function,
        instance: Value,
        args: &[Value],
        span: Span,
        class: Option<Rc<Class>>,
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

        // Stack layout after these pushes: [..., instance, args...].
        // call_closure derives stack_base = len - total_params - 1, placing
        // `instance` at slot 0 (i.e., `this`) and the arguments from slot 1 —
        // matching the layout the method bytecode expects. Omitted parameters
        // are padded with null (and their defaults applied) by call_closure
        // itself.
        self.push(instance);
        for arg in args {
            self.push(arg.clone());
        }
        self.call_closure_in_class(closure, args.len(), span, class)?;
        self.run()
    }

    /// Reset VM state between requests (preserves globals).
    pub fn reset(&mut self) {
        self.stack.clear();
        self.frames.clear();
        self.open_upvalues.clear();
        self.open_upvalue_top = 0;
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
        // A block declaring two or more parameters destructures an array
        // element, as a Ruby block does: `pairs.map { |x, i| … }`. Every
        // array iteration method reaches its block through here.
        if let (Value::VmClosure(closure), Value::Array(parts)) = (callee, &arg) {
            let arity = closure.proto.arity as usize;
            if arity >= 2 {
                // Missing required positions read as nil; a defaulted one
                // left unfilled keeps its default.
                let required = arity - closure.proto.defaults as usize;
                let parts: Vec<Value> = {
                    let parts = parts.borrow();
                    let count = parts.len().min(arity).max(required);
                    (0..count)
                        .map(|index| parts.get(index).cloned().unwrap_or(Value::Null))
                        .collect()
                };
                return self.invoke_in_batch(batch, callee, &parts, span);
            }
        }
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
            body: Rc::new(body.into()),
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

    /// The classes `source` declares, defined by the tree-walker as the app
    /// loader defines an application's, and a VM seeded with its globals.
    fn vm_with_app_classes(source: &str) -> Vm {
        use crate::lexer::Scanner;
        use crate::parser::Parser;

        let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
        let program = Parser::new(tokens).parse().expect("parser error");
        let mut interpreter = crate::interpreter::Interpreter::new();
        interpreter.interpret(&program).expect("the classes load");
        let mut vm = Vm::new();
        for (name, value) in interpreter.global_env().borrow().get_all_bindings() {
            vm.globals.insert(name, value);
        }
        vm
    }

    /// A callback the record's class inherits runs as the class that declares
    /// it. It ran as the record's class, so the `super.stamp()` in it resolved
    /// against the declaring class itself: the callback ran a second time
    /// before the grandparent's was reached.
    #[test]
    fn an_inherited_callback_runs_as_the_class_that_declares_it() {
        let mut vm = vm_with_app_classes(
            r#"
class Root
  def stamp
    @log = "root;"
  end
end

class Base < Root
  def stamp
    super.stamp()
    @log = @log + "base;"
  end
end

class Rec < Base
end
"#,
        );
        let Some(Value::Class(rec)) = vm.globals.get("Rec").cloned() else {
            panic!("Rec is a class");
        };
        let inst = Rc::new(RefCell::new(Instance::new(rec.clone())));
        let kept = vm
            .run_model_callbacks_vm(&rec, &inst, &["stamp".to_string()], Span::default())
            .expect("the callback runs on the VM");
        assert!(kept, "the callback did not veto");
        assert_eq!(inst.borrow().get("log"), Some(Value::str("root;base;")));
    }

    /// `name` of a fresh `class_name` instance on the VM, entered as a served
    /// action is.
    fn call_on_vm(vm: &mut Vm, class_name: &str, name: &str) -> Value {
        let Some(Value::Class(class)) = vm.globals.get(class_name).cloned() else {
            panic!("{class_name} is a class");
        };
        let method = class.find_method(name).expect("the method exists");
        let instance = Value::Instance(Rc::new(RefCell::new(Instance::new(class))));
        let result = vm
            .call_method_bound(&method, instance, None, Span::default())
            .unwrap_or_else(|err| panic!("{class_name}#{name}: {err}"));
        vm.reset();
        result
    }

    /// A tree-walker constructor runs with the class that declares it on its
    /// frame, inherited or not, and so does a method reached through
    /// `super.m()`: each `super` goes one level up, past a class that only
    /// inherits. With no class there, the first `super` raised "super used
    /// outside of a subclass method"; with the class it was looked up from
    /// instead, K1's would have called itself.
    #[test]
    fn super_in_tree_walker_constructors_and_methods_climbs_the_chain() {
        let mut vm = vm_with_app_classes(
            r##"
class K0
  def tag
    "k0"
  end

  new(a)
    @k0 = a
  end
end

class K1 < K0
  def tag
    "k1>" + super.tag()
  end

  new(a)
    super(a + 1)
    @k1 = a
  end
end

class K2 < K1
end

class K3 < K2
  def tag
    "k3>" + super.tag()
  end

  new(a)
    super(a * 2)
    @k3 = a
  end
end

class Probe
  def built
    k = K3.new(3)
    "#{k.k0} #{k.k1} #{k.k3}"
  end

  def tagged
    K3.new(1).tag()
  end
end
"##,
        );
        assert_eq!(call_on_vm(&mut vm, "Probe", "built"), Value::str("7 6 3"));
        assert_eq!(
            call_on_vm(&mut vm, "Probe", "tagged"),
            Value::str("k3>k1>k0")
        );
    }

    /// A class the tree-walker defined (a served app's) gets its fields on
    /// the VM: a literal default copied per instance, one that is not a
    /// literal run per instance, nil without one — by every spelling. The
    /// plan is built from the AST on the first instance and kept. The VM set
    /// none of them: the first read raised "Cannot access property".
    #[test]
    fn a_served_class_gets_literal_computed_and_nil_fields() {
        let mut vm = vm_with_app_classes(
            r##"
calls = 0

def tick()
  calls = calls + 1
  calls
end

class Ticket
  n: Int = tick()
  tags: Array = []
  note: String
  seen: Any

  new()
    @seen = @note.nil?
  end
end

class Probe
  def run
    a = Ticket.new()
    b = new Ticket()
    a.tags.push(1)
    "#{a.n} #{b.n} #{a.tags.length} #{b.tags.length} #{b.note.nil?} #{b.seen}"
  end
end
"##,
        );
        assert_eq!(
            call_on_vm(&mut vm, "Probe", "run"),
            Value::str("1 2 1 0 true true")
        );
        let Some(Value::Class(ticket)) = vm.globals.get("Ticket").cloned() else {
            panic!("Ticket is a class");
        };
        let init = ticket
            .field_init
            .borrow()
            .clone()
            .expect("built on the first instance");
        let names: Vec<&str> = init.fields.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, ["n", "tags", "note", "seen"]);
        assert!(init.computed.is_some(), "`n` is computed by one method");
        assert_eq!(
            call_on_vm(&mut vm, "Probe", "run"),
            Value::str("3 4 1 0 true true")
        );
        let again = ticket.field_init.borrow().clone().expect("kept");
        assert!(Rc::ptr_eq(&init, &again), "built once per class");
    }

    /// `class` built from `args` the way `new Klass(args)` builds it:
    /// `call_class` must leave the constructor as one more frame of this
    /// dispatch loop (not run it to completion itself), and the loop then
    /// returns the instance.
    fn construct_as_a_frame(vm: &mut Vm, class: &Rc<Class>, args: &[Value]) -> Value {
        vm.push(Value::Class(class.clone()));
        for arg in args {
            vm.push(arg.clone());
        }
        let frames = vm.frames.len();
        vm.call_class(class, args.len(), Span::default())
            .expect("the constructor starts");
        assert_eq!(
            vm.frames.len(),
            frames + 1,
            "{}'s constructor is a frame of the running loop, not a nested run",
            class.name
        );
        vm.run().expect("the constructor runs")
    }

    fn field_of(value: &Value, name: &str) -> Option<Value> {
        match value {
            Value::Instance(instance) => instance.borrow().get(name),
            other => panic!("expected an instance, got {}", other.type_name()),
        }
    }

    /// A served class's tree-walker constructor is compiled once, in
    /// constructor mode, kept on the class that declares it, and entered as
    /// an ordinary frame by every construction and by a subclass's
    /// `super(...)`. It was compiled into a fresh closure and run in a nested
    /// `run()` on every `new`, which threw its value away.
    #[test]
    fn a_served_constructor_runs_as_a_frame_of_one_cached_closure() {
        let mut vm = vm_with_app_classes(
            r##"
class Pt
  x: Int
  y: Int

  new(x, y)
    @x = x
    @y = y
  end
end

class Diagonal < Pt
  new(n)
    super(n, n)
  end
end
"##,
        );
        let Some(Value::Class(pt)) = vm.globals.get("Pt").cloned() else {
            panic!("Pt is a class");
        };
        let Some(Value::Class(diagonal)) = vm.globals.get("Diagonal").cloned() else {
            panic!("Diagonal is a class");
        };
        assert!(
            pt.vm_constructor.borrow().is_none(),
            "compiled on first use"
        );

        let first = construct_as_a_frame(&mut vm, &pt, &[Value::Int(1), Value::Int(2)]);
        assert_eq!(field_of(&first, "x"), Some(Value::Int(1)));
        assert_eq!(field_of(&first, "y"), Some(Value::Int(2)));
        let (compiled_from, closure) = pt
            .vm_constructor
            .borrow()
            .clone()
            .expect("kept on the class");
        let declared = pt.constructor.clone().expect("Pt declares one");
        assert!(
            Rc::ptr_eq(&compiled_from, &declared),
            "compiled from Pt's own constructor"
        );

        let second = construct_as_a_frame(&mut vm, &pt, &[Value::Int(3), Value::Int(4)]);
        assert_eq!(field_of(&second, "x"), Some(Value::Int(3)));
        let diagonal_built = construct_as_a_frame(&mut vm, &diagonal, &[Value::Int(5)]);
        assert_eq!(field_of(&diagonal_built, "x"), Some(Value::Int(5)));
        assert_eq!(field_of(&diagonal_built, "y"), Some(Value::Int(5)));

        let (_, kept) = pt.vm_constructor.borrow().clone().expect("still kept");
        assert!(
            Rc::ptr_eq(&closure, &kept),
            "the next `new` and the subclass's `super(...)` reuse Pt's closure"
        );
        assert!(
            diagonal.vm_constructor.borrow().is_some(),
            "Diagonal keeps its own constructor"
        );
    }

    /// The frame a served constructor runs in returns its instance however
    /// the body ends — a guard's early `return`, a `return` inside
    /// `try`/`finally` — by every spelling. Run to completion, its value was
    /// thrown away; as a frame, constructor mode is what keeps `new` from
    /// evaluating to what the body returned.
    #[test]
    fn a_served_constructor_returning_early_yields_the_instance() {
        let mut vm = vm_with_app_classes(
            r##"
class Guarded
  x: Int

  new(a)
    @x = a
    return if a > 1

    @x = 0
  end
end

class Finally
  log: String

  new
    @log = ""
    try
      @log = @log + "t"
      return
    finally
      @log = @log + "f"
    end
  end
end

class Probe
  def run
    "#{Guarded.new(5).x} #{new Guarded(0).x} #{Guarded(9).x} #{Finally.new().log}"
  end
end
"##,
        );
        assert_eq!(call_on_vm(&mut vm, "Probe", "run"), Value::str("5 0 9 tf"));
        let Some(Value::Class(guarded)) = vm.globals.get("Guarded").cloned() else {
            panic!("Guarded is a class");
        };
        let early = construct_as_a_frame(&mut vm, &guarded, &[Value::Int(7)]);
        assert_eq!(field_of(&early, "x"), Some(Value::Int(7)));
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

    /// An app-level `def` under serve is a tree-walker `Function` in the VM
    /// globals. Its first VM call compiles it and keeps the closure; every
    /// later call runs that same closure — in the fast tier, in the general
    /// arm (an omitted default), recursively — instead of allocating one per
    /// call, and returns what the tree-walker returns.
    #[test]
    fn a_tree_walker_def_is_called_through_its_cached_closure() {
        use crate::lexer::Scanner;
        use crate::parser::Parser;
        use crate::vm::Compiler;

        let mut vm = vm_with_app_classes(
            r#"
def util_add(acc, i)
  acc + i
end

def with_default(a, b = 10)
  a + b
end

def fact(n)
  return 1 if n < 2

  n * fact(n - 1)
end
"#,
        );
        let function = |vm: &Vm, name: &str| match vm.globals.get(name) {
            Some(Value::Function(func)) => Rc::clone(func),
            other => panic!("{name} is a tree-walker function, got {other:?}"),
        };
        let run = |vm: &mut Vm, source: &str| -> Result<Value, RuntimeError> {
            let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
            let program = Parser::new(tokens).parse().expect("parser error");
            let module = Compiler::compile(&program).expect("compile error");
            vm.reset();
            vm.execute(&module.main)?;
            Ok(vm.globals.get("x").cloned().unwrap_or(Value::Null))
        };
        let loop_source = "let x = 0\nfor i in 0..1000\n  x = util_add(x, i)\nend\n\
                           x = x + with_default(1) + with_default(1, 2) + fact(10)";
        let expected = Value::Int(499_500 + 11 + 3 + 3_628_800);

        assert_eq!(run(&mut vm, loop_source).unwrap(), expected);
        let add = function(&vm, "util_add");
        let closure = add
            .jit_closure
            .borrow()
            .clone()
            .expect("the closure is kept");
        assert!(Arc::ptr_eq(
            &closure.proto,
            add.jit_cache.borrow().as_ref().unwrap()
        ));
        let defaults = function(&vm, "with_default");
        let defaults_closure = cached_function_closure(&defaults).expect("kept");
        let fact = function(&vm, "fact");
        let fact_closure = cached_function_closure(&fact).expect("kept");

        // A second run reuses each closure.
        assert_eq!(run(&mut vm, loop_source).unwrap(), expected);
        assert!(Rc::ptr_eq(
            &closure,
            &cached_function_closure(&add).unwrap()
        ));
        assert!(Rc::ptr_eq(
            &defaults_closure,
            &cached_function_closure(&defaults).unwrap()
        ));
        assert!(Rc::ptr_eq(
            &fact_closure,
            &cached_function_closure(&fact).unwrap()
        ));

        // The function itself is still what the globals hold: `type` and
        // arity probes see a function, not the closure.
        assert!(matches!(
            vm.globals.get("util_add"),
            Some(Value::Function(_))
        ));

        // A wrong argument count is the arity error it was.
        let err = run(&mut vm, "let x = util_add(1)").unwrap_err();
        assert!(
            matches!(
                err,
                RuntimeError::WrongArity {
                    expected: 2,
                    got: 1,
                    ..
                }
            ),
            "an arity error: {err}"
        );

        // A closure over a proto that is no longer the cached one is never
        // trusted: the next call makes a new one over the current proto.
        let fresh = jit_compile_function(&Function::default(), std::iter::empty()).unwrap();
        *add.jit_cache.borrow_mut() = Some(fresh);
        assert!(cached_function_closure(&add).is_none());
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
            body: Rc::new(body.into()),
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

    /// Puts `env` in the view-helper slot for the test and puts the previous
    /// one back, including when the test panics.
    struct HelperEnvGuard {
        prev: Option<Rc<RefCell<crate::interpreter::environment::Environment>>>,
    }

    impl HelperEnvGuard {
        fn set(env: Option<Rc<RefCell<crate::interpreter::environment::Environment>>>) -> Self {
            let prev = crate::interpreter::builtins::template::set_view_helper_env_for_test(env);
            clear_view_helper_vm_cache();
            Self { prev }
        }
    }

    impl Drop for HelperEnvGuard {
        fn drop(&mut self) {
            crate::interpreter::builtins::template::set_view_helper_env_for_test(self.prev.take());
            clear_view_helper_vm_cache();
        }
    }

    fn helper_fn(
        source: &str,
        closure: Rc<RefCell<crate::interpreter::environment::Environment>>,
    ) -> Rc<Function> {
        use crate::ast::stmt::StmtKind;
        use crate::lexer::Scanner;
        use crate::parser::Parser;
        let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
        let program = Parser::new(tokens).parse().expect("parser error");
        let StmtKind::Function(decl) = &program.statements[0].kind else {
            panic!("expected a function declaration");
        };
        Rc::new(Function::from_decl(decl, closure, None))
    }

    /// Run `source` (it binds `x`) on a view VM: tree-walker functions, except
    /// a helper the VM can compile.
    fn call_on_view_vm(func: &Rc<Function>, source: &str) -> Value {
        use crate::lexer::Scanner;
        use crate::parser::Parser;
        let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
        let program = Parser::new(tokens).parse().expect("parser error");
        let module = Compiler::compile(&program).expect("compile error");
        let mut vm = Vm::new();
        vm.tree_walk_functions = true;
        vm.globals
            .insert(func.name.clone(), Value::Function(Rc::clone(func)));
        vm.execute(&module.main)
            .unwrap_or_else(|err| panic!("{source}: {err}"));
        vm.globals.get("x").cloned().unwrap_or(Value::Null)
    }

    #[test]
    fn a_helper_with_no_free_name_runs_on_the_view_vm() {
        let env = Rc::new(RefCell::new(
            crate::interpreter::environment::Environment::new(),
        ));
        let _guard = HelperEnvGuard::set(Some(Rc::clone(&env)));
        let double = helper_fn("def double(n)\n  n * 2\nend\n", Rc::clone(&env));

        assert_eq!(
            call_on_view_vm(&double, "let x = double(21)"),
            Value::Int(42)
        );
        let cached = double
            .jit_cache
            .borrow()
            .clone()
            .expect("a helper with no free name is compiled");
        let closure = double
            .jit_closure
            .borrow()
            .clone()
            .expect("the helper's closure is kept");

        // The second call is the cache hit: same proto, still the VM, and
        // the same closure rather than a new one.
        assert_eq!(
            call_on_view_vm(&double, "let x = double(22)"),
            Value::Int(44)
        );
        assert!(std::sync::Arc::ptr_eq(
            &cached,
            &double.jit_cache.borrow().clone().unwrap()
        ));
        assert!(Rc::ptr_eq(
            &closure,
            &double.jit_closure.borrow().clone().unwrap()
        ));

        // The bench witness: a method call on the argument, no free name.
        let label = helper_fn("def label(row)\n  row.title.upcase\nend\n", Rc::clone(&env));
        assert_eq!(
            call_on_view_vm(&label, "let row = {\"title\": \"hi\"}\nlet x = label(row)"),
            Value::str("HI")
        );
        assert!(label.jit_cache.borrow().is_some());
    }

    #[test]
    fn a_helper_that_reads_or_assigns_a_closure_name_stays_interpreted() {
        let env = Rc::new(RefCell::new(
            crate::interpreter::environment::Environment::new(),
        ));
        env.borrow_mut()
            .define("req".to_string(), Value::str("Ada"));
        let _guard = HelperEnvGuard::set(Some(Rc::clone(&env)));

        let who = helper_fn("def who()\n  req\nend\n", Rc::clone(&env));
        assert_eq!(call_on_view_vm(&who, "let x = who()"), Value::str("Ada"));
        assert!(
            who.jit_cache.borrow().is_none(),
            "a helper that reads req stays on the tree-walker"
        );

        let clobber = helper_fn("def clobber()\n  req = \"no\"\nend\n", Rc::clone(&env));
        call_on_view_vm(&clobber, "let x = clobber()");
        assert_eq!(
            env.borrow().get("req"),
            Some(Value::str("no")),
            "assigning req updates the helper closure"
        );
        assert!(clobber.jit_cache.borrow().is_none());

        // A nested function's free name counts too. `inner` with no
        // parentheses runs, so the value is what `req` holds.
        env.borrow_mut()
            .define("req".to_string(), Value::str("Ada"));
        let outer = helper_fn(
            "def outer()\n  def inner()\n    req\n  end\n  inner\nend\n",
            Rc::clone(&env),
        );
        assert_eq!(
            call_on_view_vm(&outer, "let x = outer()"),
            Value::str("Ada")
        );
        assert!(outer.jit_cache.borrow().is_none());
    }

    #[test]
    fn a_function_outside_the_helper_env_stays_interpreted() {
        let closure = Rc::new(RefCell::new(
            crate::interpreter::environment::Environment::new(),
        ));
        closure
            .borrow_mut()
            .define("req".to_string(), Value::str("Ada"));
        // The helper slot points somewhere else, so `who` is not a view helper
        // even though its own closure has `req`.
        let other = Rc::new(RefCell::new(
            crate::interpreter::environment::Environment::new(),
        ));
        let _guard = HelperEnvGuard::set(Some(other));
        let who = helper_fn("def who()\n  req\nend\n", closure);
        assert_eq!(call_on_view_vm(&who, "let x = who()"), Value::str("Ada"));
        assert!(who.jit_cache.borrow().is_none());
    }
}
