//! Function call dispatch for the VM.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use crate::ast::stmt::{FunctionDecl, Program, Stmt, StmtKind};
use crate::error::RuntimeError;
use crate::interpreter::executor::MAX_CALL_DEPTH;
use crate::interpreter::value::{
    Class, ConstructPlan, FieldInit, Function, HashKey, Instance, InstanceMethodDef,
    NativeFunction, NativeSpecial, Value, NATIVE_CONSTRUCTOR,
};
use crate::span::Span;

use super::chunk::{Constant, FunctionProto};

/// A compiled method and the class that defines it (for `super`).
type MethodWithClass = (Rc<VmClosure>, Rc<Class>);
use super::compiler::Compiler;
use super::upvalue::{Upvalue, VmClosure};
use super::vm::{positional_supplied_mask, CallFrame, FrameReturn, Vm};

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

#[cfg(test)]
thread_local! {
    /// Test-only: the natives `call_plain_native_at` ran on this thread.
    static PLAIN_NATIVE_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Test-only: how many natives this thread called on the stack slice.
#[cfg(test)]
pub(crate) fn plain_native_calls() -> usize {
    PLAIN_NATIVE_CALLS.with(std::cell::Cell::get)
}

#[cfg(test)]
thread_local! {
    /// Test-only: the `call_method_slow_path` entries on this thread.
    static SLOW_PATH_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Test-only: how many times `call_method_slow_path` has run on this thread,
/// for a test to tell a call that entered its method directly from one that
/// dispatched by name.
#[cfg(test)]
pub(crate) fn slow_path_calls() -> usize {
    SLOW_PATH_CALLS.with(std::cell::Cell::get)
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

/// A labelled argument that [`plan_named_binding`] drops: a positional
/// argument already filled the parameter it names.
pub(crate) const DROPPED_LABEL: usize = usize::MAX;

/// How many labels a named call to a compiled callee resolves without a heap
/// allocation; a call with more spills its plan to a `Vec`.
const INLINE_LABELS: usize = 16;

/// Work out where each argument of a labelled call lands in `proto`'s
/// parameters, and return the mask of the parameters it supplies.
///
/// `positional` unlabelled arguments fill the leading parameters in order;
/// `names` yields the labels in source order, and `named_targets[j]` receives
/// the parameter index of the `j`-th, or [`DROPPED_LABEL`].
///
/// Follows the tree-walking interpreter's rules: positional arguments fill the
/// leading parameters, labelled arguments fill by name, an unknown label is an
/// undefined-variable error, and a parameter that ends up unfilled is an arity
/// error unless it declares a default. Unfilled defaulted slots are left null
/// and written by the callee's prologue, so a default expression is evaluated
/// in the callee — and only when it is actually needed. Only names are read,
/// so a call can plan before it moves a single value.
pub(crate) fn plan_named_binding<'a>(
    proto: &FunctionProto,
    positional: usize,
    names: impl Iterator<Item = &'a str>,
    named_targets: &mut [usize],
    span: Span,
) -> Result<u64, RuntimeError> {
    let total_params = proto.param_names.len();

    if positional > total_params {
        return Err(RuntimeError::wrong_arity(
            total_params,
            positional + named_targets.len(),
            span,
        ));
    }

    let mut supplied = positional_supplied_mask(positional);

    for (target, name) in named_targets.iter_mut().zip(names) {
        let Some(index) = proto.param_names.iter().position(|p| p.as_str() == name) else {
            // Same error the interpreter raises for `f(nope: 1)`.
            return Err(RuntimeError::undefined_variable(name.to_string(), span));
        };
        // A label that names an already-positionally-filled parameter is
        // dropped, and the positional value wins. That is not an obviously good
        // rule — arguably `f(3, a: 9)` should be an error — but it is what the
        // tree-walking interpreter does (`used_params.contains(...) { continue }`
        // in `call_value_with_named`), and the engines must agree.
        if index < 64 && supplied & (1u64 << index) != 0 {
            *target = DROPPED_LABEL;
            continue;
        }
        *target = index;
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

    Ok(supplied)
}

/// Lay labelled arguments out in parameter order for `proto`, returning the
/// slot values and a mask of which parameters were actually supplied — the
/// rules of [`plan_named_binding`], for a caller that holds the values in
/// vectors rather than on the stack.
pub(crate) fn bind_named_arguments(
    proto: &FunctionProto,
    positional: Vec<Value>,
    named: Vec<(crate::interpreter::value::SoliStr, Value)>,
    span: Span,
) -> Result<(Vec<Value>, u64), RuntimeError> {
    let mut targets = vec![0; named.len()];
    let supplied = plan_named_binding(
        proto,
        positional.len(),
        named.iter().map(|(name, _)| name.as_str()),
        &mut targets,
        span,
    )?;
    let mut slots = positional;
    slots.resize(proto.param_names.len(), Value::Null);
    for ((_, value), target) in named.into_iter().zip(targets) {
        if target != DROPPED_LABEL {
            slots[target] = value;
        }
    }
    Ok((slots, supplied))
}

/// What a static call on a class resolves to; see [`resolve_static_callee`].
enum StaticCallee {
    /// Bytecode registered on a class of the chain (`vm_static_methods`):
    /// a static compiled by the VM, or a tree-walker one that reads `this`.
    /// It runs with the class it was called on as its frame's class.
    Compiled(Rc<VmClosure>),
    /// A tree-walker `static def` that does not read `this`, already
    /// compiled as a plain function (`vm_plain_static_methods`).
    Plain(Rc<VmClosure>),
    /// A tree-walker `static def` not compiled yet, the class that defines
    /// it, and whether it is one of that class's own `static_methods`
    /// rather than a method mixed in by `extend`.
    TreeWalker {
        func: Rc<Function>,
        defining: Rc<Class>,
        own: bool,
    },
}

/// Resolve the static `name` of `class` in one walk up the chain. On each
/// class, from the receiver up: the bytecode statics, then the cached plain
/// ones (skipped when `plain` is false, in a view), then the tree-walker's
/// own statics and the ones `extend` mixed in — the order
/// `Class::find_static_method` keeps — so a static of a subclass hides
/// its superclass's static of the same name, whichever form either is in.
/// `None` when no class of the chain has one: natives and the rest are the
/// caller's business.
fn resolve_static_callee(class: &Rc<Class>, name: &str, plain: bool) -> Option<StaticCallee> {
    let mut level = class;
    loop {
        if let Some(closure) = level.vm_static_methods.borrow().get(name) {
            return Some(StaticCallee::Compiled(closure.clone()));
        }
        if plain {
            if let Some(closure) = level.vm_plain_static_methods.get(name) {
                return Some(StaticCallee::Plain(closure));
            }
        }
        if let Some(func) = level.static_methods.get(name) {
            return Some(StaticCallee::TreeWalker {
                func: func.clone(),
                defining: level.clone(),
                own: true,
            });
        }
        if let Some(func) = level.mixin_static_methods.borrow().get(name) {
            return Some(StaticCallee::TreeWalker {
                func: func.clone(),
                defining: level.clone(),
                own: false,
            });
        }
        level = level.superclass.as_ref()?;
    }
}

/// [`bind_named_arguments`] for a method call that may carry a block
/// (`obj.m(k: v) do … end`), as the tree-walker's `call_value_with_named`
/// binds it. With no `&block` parameter the block is dropped and the rest binds
/// as for a function. With one, the block (or `nil`) goes to that parameter,
/// which takes no positional or labelled argument and does not count in the
/// arity an error reports.
pub(crate) fn bind_method_arguments(
    proto: &FunctionProto,
    positional: Vec<Value>,
    named: Vec<(crate::interpreter::value::SoliStr, Value)>,
    block: Option<Value>,
    span: Span,
) -> Result<(Vec<Value>, u64), RuntimeError> {
    let total_params = proto.param_names.len();
    let block_param = match proto.block_param {
        Some(index) if (index as usize) < total_params && total_params <= 64 => index as usize,
        _ => return bind_named_arguments(proto, positional, named, span),
    };
    let regular = total_params - 1;
    if positional.len() > regular {
        return Err(RuntimeError::wrong_arity(
            regular,
            positional.len() + named.len(),
            span,
        ));
    }

    let mut slots = vec![Value::Null; total_params];
    let mut supplied = 0u64;
    for (i, value) in positional.into_iter().enumerate() {
        let index = if i < block_param { i } else { i + 1 };
        slots[index] = value;
        supplied |= 1u64 << index;
    }
    for (name, value) in named {
        let Some(index) = proto
            .param_names
            .iter()
            .enumerate()
            .position(|(i, p)| i != block_param && p.as_str() == name.as_ref())
        else {
            return Err(RuntimeError::undefined_variable(name.to_string(), span));
        };
        // A label naming a parameter already filled by position is dropped,
        // as in `bind_named_arguments`.
        if supplied & (1u64 << index) != 0 {
            continue;
        }
        slots[index] = value;
        supplied |= 1u64 << index;
    }

    let required = (0..total_params)
        .filter(|&i| i != block_param && proto.defaults_mask & (1u64 << i) == 0)
        .count();
    for index in 0..total_params {
        if index == block_param {
            continue;
        }
        let filled = supplied & (1u64 << index) != 0;
        let has_default = proto.defaults_mask & (1u64 << index) != 0;
        if !filled && !has_default {
            let bound_before = (0..index)
                .filter(|&i| i != block_param && supplied & (1u64 << i) != 0)
                .count();
            return Err(RuntimeError::wrong_arity(required, bound_before, span));
        }
    }

    // The block, or `nil` without one — never the parameter's default.
    slots[block_param] = block.unwrap_or(Value::Null);
    supplied |= 1u64 << block_param;
    Ok((slots, supplied))
}

/// `labels` as `CallNamed` carries them: a block's slot is positional.
pub(crate) fn without_block_label(
    labels: &[Option<crate::interpreter::value::SoliStr>],
) -> Vec<Option<crate::interpreter::value::SoliStr>> {
    labels
        .iter()
        .map(|label| match label {
            Some(name) if name.as_str() == super::chunk::BLOCK_LABEL => None,
            other => other.clone(),
        })
        .collect()
}

/// How many of the `passed` arguments an iterator hands its block.
///
/// A block binds only the parameters it declares, and the iterator's other
/// arguments are dropped. That is what the tree-walker's iterators do: they
/// bind `func.params` and nothing else. So `3.times { … }` and
/// `xs.each { print("tick") }` run, and `xs.each_with_index { |x| … }` gets
/// the element without its index. A callee whose parameters are not known
/// here (a native function, a bound method, a class) gets every argument and
/// checks its own arity, as before.
///
/// Only iterator drivers ask this. A direct call (`f(5)` on `fn() { … }`, or
/// `h.f(5)` on a hash entry) stays strict on both engines.
#[inline]
pub(crate) fn block_argc(callee: &Value, passed: usize) -> usize {
    match callee {
        Value::VmClosure(closure) => closure.proto.param_names.len().min(passed),
        Value::Function(func) => func.params.len().min(passed),
        _ => passed,
    }
}

impl Vm {
    /// Refuse calls past [`MAX_CALL_DEPTH`]. The bytecode loop itself is
    /// iterative (deep recursion grows `frames` on the heap), but builtins
    /// that call back into Soli recurse natively — and a stack overflow
    /// aborts without unwinding, so it must be prevented, not caught.
    ///
    /// Every call that opens a frame passes here, so it is also where the
    /// handler budget is checked for calls — `Op::Loop` alone missed a
    /// program that loops without a backward jump: recursion, and the
    /// blocks a native iterator (`n.times`, `upto`, `each`…) calls once per
    /// element. A countdown decrement, reading the clock every few thousand.
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
        if crate::interpreter::deadline::expired() {
            return Err(crate::interpreter::deadline::timeout_error(span));
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

        // A plain native (anything but the few `call_native` intercepts) is
        // called where it lies on the stack: no clone of the callee.
        if let Value::NativeFunction(native) = &self.stack[callee_idx] {
            if native.vm_special == NativeSpecial::None {
                return self.call_plain_native_at(callee_idx, argc, span);
            }
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
        match self.kernel_value(closure, argc) {
            Some(value) => {
                let base = self.stack.len() - argc;
                self.stack.truncate(base - 1);
                self.stack.push(value);
                true
            }
            None => false,
        }
    }

    /// What `closure`'s native kernel returns for the `argc` arguments at the
    /// top of the stack, touching nothing: `None` when it has no kernel or
    /// the kernel declined. The fast tier's `Call`/`CallGlobal` use it while
    /// still borrowing the callee, then settle the stack themselves.
    #[inline]
    pub(crate) fn kernel_value(&self, closure: &VmClosure, argc: usize) -> Option<Value> {
        let kernel = closure.proto.kernel.as_ref()?;
        let base = self.stack.len() - argc;
        let globals = &self.globals;
        let bound = |name: &str| match globals.get(name) {
            Some(Value::VmClosure(c)) => c.proto.kernel.as_ref().map(|k| k.id()),
            _ => None,
        };
        kernel.try_call(&self.stack[base..], self.frames.len(), bound)
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
        // The span only on the cold branch: building it every call showed
        // in profiles. The handler budget is checked on every frame, as
        // `ensure_call_depth` does: recursion through a method has no
        // backward jump, so this is where it stops (expiry is sticky, so
        // `ensure_call_depth` reports it).
        if self.frames.len() >= MAX_CALL_DEPTH || crate::interpreter::deadline::expired() {
            self.ensure_call_depth(&closure.proto, self.current_span())?;
        }
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
        let label = |i: usize| labels.get(i).and_then(|l| l.as_ref());
        // A label given twice is refused whatever the callee turns out to be.
        for i in 0..argc {
            if let Some(name) = label(i) {
                if (0..i).any(|earlier| label(earlier) == Some(name)) {
                    return Err(RuntimeError::type_error(
                        format!("duplicate named argument '{}'", name),
                        span,
                    ));
                }
            }
        }

        // A compiled callee — a compiled function, a tree-walking one compiled
        // now, or a class whose constructor is compiled — binds in place on
        // the stack.
        let callee_idx = self.stack.len() - 1 - argc;
        let compiled = match &self.stack[callee_idx] {
            Value::VmClosure(closure) => Some((closure.clone(), None)),
            // A view's functions (`tree_walk_functions`) bind below.
            Value::Function(func) if !self.tree_walk_functions => {
                let func = func.clone();
                let proto =
                    jit_compile_function(&func, self.globals.keys().cloned()).map_err(|e| {
                        RuntimeError::EngineFallback(
                            format!("a function the VM cannot compile ({})", e),
                            span,
                        )
                    })?;
                let closure = Rc::new(VmClosure::new(proto, Vec::new()));
                self.stack[callee_idx] = Value::VmClosure(closure.clone());
                Some((closure, None))
            }
            // `Config(port: 3000)` — bind against the compiled constructor and
            // let it run with `this` in the callee slot.
            Value::Class(class)
                if !(crate::interpreter::domain::value_objects_declared()
                    && class.is_value_object()) =>
            {
                class
                    .find_vm_init_with_class()
                    .map(|(ctor, defining_class)| (ctor, Some((class.clone(), defining_class))))
            }
            _ => None,
        };
        if let Some((closure, construct)) = compiled {
            return self.call_compiled_named(closure, argc, labels, construct, span);
        }

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
            match label(i) {
                Some(name) => named.push((name.clone(), value)),
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
                        pairs.insert(crate::interpreter::value::HashKey::String(name), value);
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
            // A view (`tree_walk_functions`): a view helper binds its labels
            // and runs here, its closure's names refilled as for a positional
            // call; anything else, or a call `bind` refuses, runs on the
            // tree-walker with its labels, as the positional path does.
            // Compiling it against the view VM's globals would lose what the
            // function reads from its closure (`req`, the other helpers).
            Value::Function(ref func) if self.tree_walk_functions => {
                if let Some(helper) = super::vm_view_helpers::view_helper(func) {
                    let (slots, supplied) = bind_named_arguments(
                        helper.proto(),
                        positional.clone(),
                        named.clone(),
                        span,
                    )?;
                    if let Some(closure) = helper.bind(func, &slots) {
                        self.stack[callee_idx] = Value::VmClosure(Rc::clone(&closure));
                        return self.call_closure_with_slots(closure, slots, supplied, None);
                    }
                }
                let func = Rc::clone(func);
                let named: std::collections::HashMap<String, Value> = named
                    .into_iter()
                    .map(|(name, value)| (name.to_string(), value))
                    .collect();
                let mut interpreter = match &self.tree_walk_env {
                    Some(env) => crate::interpreter::Interpreter::with_environment(env.clone()),
                    None => crate::interpreter::Interpreter::for_bound_body(),
                };
                let result = interpreter.call_value_with_named(
                    Value::Function(func),
                    positional,
                    named,
                    None,
                    span,
                )?;
                self.stack.pop();
                self.push(result);
                Ok(())
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
            // A value object, or a class with no compiled constructor (bound
            // above when it has one).
            Value::Class(ref class) => {
                if crate::interpreter::domain::value_objects_declared() && class.is_value_object() {
                    self.stack.truncate(callee_idx);
                    return self.build_value_object(class, positional, named, span);
                }
                if class.find_constructor().is_none()
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

    /// Bind a labelled call to a compiled `closure` in place: the arguments
    /// sitting above the callee are moved into a parameter-ordered run of
    /// slots pushed after them, and the arguments' region is then closed up,
    /// with no vector in between. With `construct`, the callee slot first
    /// becomes a new instance of the class, the constructor's `this`.
    ///
    /// The plan is made from the labels alone, so an error leaves the stack
    /// as it was.
    fn call_compiled_named(
        &mut self,
        closure: Rc<VmClosure>,
        argc: usize,
        labels: &[Option<crate::interpreter::value::SoliStr>],
        construct: Option<(Rc<Class>, Rc<Class>)>,
        span: Span,
    ) -> Result<(), RuntimeError> {
        let label = |i: usize| labels.get(i).and_then(|l| l.as_ref());
        let named = (0..argc).filter(|&i| label(i).is_some()).count();
        let mut inline = [0usize; INLINE_LABELS];
        let mut spilled = Vec::new();
        let targets: &mut [usize] = if named <= INLINE_LABELS {
            &mut inline[..named]
        } else {
            spilled.resize(named, 0);
            &mut spilled
        };
        let supplied = plan_named_binding(
            &closure.proto,
            argc - named,
            (0..argc).filter_map(label).map(|name| name.as_str()),
            targets,
            span,
        )?;

        let args_base = self.stack.len() - argc;
        // The declared fields start before the constructor runs, as on the
        // tree-walker (`init_fields`).
        let mut defaults = None;
        let defining_class = match construct {
            Some((class, defining_class)) => {
                let instance = Rc::new(RefCell::new(Instance::new(class.clone())));
                if let Some(plan) = self.start_fields(&class, &instance, span)? {
                    defaults = Some((plan, class, instance.clone()));
                }
                self.stack[args_base - 1] = Value::Instance(instance);
                Some(defining_class)
            }
            None => None,
        };

        let total_params = closure.proto.param_names.len();
        let slots_base = self.stack.len();
        self.stack.resize(slots_base + total_params, Value::Null);
        let (mut next_positional, mut next_named) = (0, 0);
        for i in 0..argc {
            let target = if label(i).is_some() {
                next_named += 1;
                targets[next_named - 1]
            } else {
                next_positional += 1;
                next_positional - 1
            };
            if target != DROPPED_LABEL {
                self.stack.swap(args_base + i, slots_base + target);
            }
        }
        // What is left there: nulls, and the values of dropped labels.
        self.stack.drain(args_base..slots_base);

        let stack_base = self.stack.len() - total_params - 1;
        self.ensure_call_depth(&closure.proto, span)?;
        self.frames.push(CallFrame::new(
            closure,
            stack_base,
            self.iter_stack.len(),
            defining_class,
            supplied,
        ));
        // The field defaults that are not literals run before the
        // constructor's body, as in `call_class`.
        match defaults {
            Some((plan, class, instance)) => {
                self.push_field_defaults(&plan, &class, &instance, span)
            }
            None => Ok(()),
        }
    }

    /// Take the `labels.len()` arguments of a labelled method call off the
    /// stack, split into positional ones, labelled ones (a duplicate label is
    /// an error, as on the tree-walker) and the block.
    #[allow(clippy::type_complexity)]
    fn take_labelled_arguments(
        &mut self,
        labels: &[Option<crate::interpreter::value::SoliStr>],
        span: Span,
    ) -> Result<
        (
            Vec<Value>,
            Vec<(crate::interpreter::value::SoliStr, Value)>,
            Option<Value>,
        ),
        RuntimeError,
    > {
        let values = self.stack.split_off(self.stack.len() - labels.len());
        let mut positional = Vec::new();
        let mut named: Vec<(crate::interpreter::value::SoliStr, Value)> =
            Vec::with_capacity(labels.len());
        let mut block = None;
        for (label, value) in labels.iter().zip(values) {
            match label {
                Some(name) if name.as_str() == super::chunk::BLOCK_LABEL => block = Some(value),
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
        Ok((positional, named, block))
    }

    /// A method call with labelled arguments (`Op::CallMethodNamed`,
    /// `Op::CallSelfOrGlobalNamed`): the receiver at `receiver_idx`, one
    /// argument slot per entry of `labels` above it.
    ///
    /// The method resolves as [`Self::call_method_slow_path`] resolves it: on
    /// an instance its compiled method, else its tree-walker method compiled as
    /// a method (unless a field of that name shadows it); on a class its
    /// static. The labels bind by name ([`bind_method_arguments`]) and the
    /// frame opens with the receiver still in the callee slot, so it is
    /// `this`. Nothing is called by a property read first.
    ///
    /// Otherwise, on an instance with no field of that name (a native method,
    /// `with`, `method_missing`), the labels become one trailing options hash
    /// and the block follows — the tree-walker's native convention — and the
    /// call takes the positional path. Any other receiver or member (a field,
    /// `Klass.new`, a model scope, a native value's method) is read and called
    /// as `CallNamed` calls it.
    pub(crate) fn call_method_named(
        &mut self,
        receiver_idx: usize,
        labels: &[Option<crate::interpreter::value::SoliStr>],
        name: &str,
        check_privacy: bool,
        span: Span,
    ) -> Result<(), RuntimeError> {
        if matches!(
            &self.stack[receiver_idx],
            Value::Future(_) | Value::Deferred(_)
        ) {
            let resolved = Self::force_lazy_receiver(&self.stack[receiver_idx], span)?;
            self.stack[receiver_idx] = resolved;
        }
        if check_privacy {
            self.check_private_receiver(receiver_idx, name)?;
        }
        if name == "new" {
            if let Value::Class(class) = &self.stack[receiver_idx] {
                if crate::interpreter::executor::access::member::intercepts_new(class) {
                    let class = class.clone();
                    return self.class_new_named(&class, receiver_idx, labels, span);
                }
            }
        }

        let target: Option<MethodWithClass> = match &self.stack[receiver_idx] {
            Value::Instance(inst) => {
                let class = inst.borrow().class.clone();
                // As `call_method_slow_path`: a field of that name shadows a
                // method not compiled yet; otherwise the nearest definition
                // wins, level by level.
                if inst.borrow().fields.contains_key(name) {
                    match class.find_instance_method_level(name) {
                        Some((InstanceMethodDef::Compiled(closure), defining)) => {
                            Some((closure, defining))
                        }
                        _ => None,
                    }
                } else {
                    self.resolve_instance_method(&class, name)?
                }
            }
            Value::Class(class) => {
                let class = class.clone();
                if name == "new"
                    || (class.is_model_subclass() && Self::is_model_class_scope(&class, name))
                {
                    None
                } else {
                    // The bytecode static, or a tree-walker one that reads
                    // `this` compiled as a method, as `call_method_slow_path`
                    // resolves them; a plain tree-walker static is read and
                    // called below.
                    match resolve_static_callee(&class, name, false) {
                        Some(StaticCallee::Compiled(closure)) => Some((closure, class.clone())),
                        Some(StaticCallee::TreeWalker { func, defining, .. })
                            if func.binds_this =>
                        {
                            let closure =
                                self.compile_tree_walking_static(&func, &defining, name)?;
                            Some((closure, class.clone()))
                        }
                        _ => None,
                    }
                }
            }
            _ => None,
        };

        if let Some((closure, defining_class)) = target {
            let (positional, named, block) = self.take_labelled_arguments(labels, span)?;
            let (slots, supplied) =
                bind_method_arguments(&closure.proto, positional, named, block, span)?;
            return self.call_closure_with_slots(closure, slots, supplied, Some(defining_class));
        }

        let instance_without_field = matches!(
            &self.stack[receiver_idx],
            Value::Instance(inst) if !inst.borrow().fields.contains_key(name)
        );
        if instance_without_field {
            let (positional, named, block) = self.take_labelled_arguments(labels, span)?;
            let mut argc = positional.len();
            for value in positional {
                self.push(value);
            }
            if !named.is_empty() {
                let mut pairs = crate::interpreter::value::HashPairs::default();
                for (name, value) in named {
                    pairs.insert(
                        crate::interpreter::value::HashKey::String(name.to_string().into()),
                        value,
                    );
                }
                self.push(Value::Hash(Rc::new(RefCell::new(pairs))));
                argc += 1;
            }
            if let Some(block) = block {
                self.push(block);
                argc += 1;
            }
            return self.call_method_slow_path(receiver_idx, argc, name);
        }

        let object = self.stack[receiver_idx].clone();
        let callee = self.op_get_property_member(&object, name, span)?;
        self.stack[receiver_idx] = callee;
        let labels = without_block_label(labels);
        self.call_value_named(labels.len(), &labels, span)
    }

    fn call_native(
        &mut self,
        native: &NativeFunction,
        argc: usize,
        span: Span,
    ) -> Result<(), RuntimeError> {
        // The callee sits below its arguments on every path that gets here.
        let callee_idx = self.stack.len() - 1 - argc;
        if native.vm_special == NativeSpecial::None {
            return self.call_plain_native_at(callee_idx, argc, span);
        }

        // Check arity
        if let Some(expected) = native.arity {
            if argc != expected {
                return Err(RuntimeError::wrong_arity(expected, argc, span));
            }
        }

        // An intercepted native: take the arguments off the stack, owned.
        let mut args: Vec<Value> = self.stack.drain(callee_idx + 1..).collect();
        self.stack.truncate(callee_idx);

        match native.vm_special {
            NativeSpecial::Grouped => return self.call_grouped_block(args, span),
            NativeSpecial::WithTransaction => return self.call_with_transaction_block(args, span),
            NativeSpecial::AfterCommit => return self.call_after_commit_block(args, span),
            NativeSpecial::WithTenant => return self.call_with_tenant_block(args, span),
            _ => {}
        }
        // SEC-013b: `render_json(record)` sends what the class's `as_json`
        // returns, as on the tree-walker (`try_evaluate_render_json_with_as_json`).
        // The native has no engine to call a user method with, so on the VM
        // `as_json` was skipped and every field went out — the very fields an
        // `as_json` is written to leave out (a password digest, a token).
        if native.vm_special == NativeSpecial::RenderJson {
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
        if native.vm_special == NativeSpecial::AssertRaises {
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
        if matches!(
            native.vm_special,
            NativeSpecial::Defined | NativeSpecial::ConstGet
        ) {
            if let Some(Value::String(name)) = args.first() {
                if let Some(value) = self.globals.get(name.as_str()) {
                    let result = if native.vm_special == NativeSpecial::Defined {
                        Value::Bool(true)
                    } else {
                        value.clone()
                    };
                    self.push(result);
                    return Ok(());
                }
            }
        }

        // Call the native function, in a flamegraph span as above.
        let _native_span = crate::serve::span_log::maybe_instrument_native(&native.name);
        let result = (native.func)(&args).map_err(|e| RuntimeError::new(e, span))?;
        drop(_native_span);
        self.push(result);
        Ok(())
    }

    /// Call the plain native at `stack[callee_idx]` on the `argc` arguments
    /// above it, straight on the stack slice: no argument `Vec`, no callee
    /// clone. A native gets only `&[Value]` — it has no way back to this
    /// VM's stack — so the slice cannot move under it. The callee and the
    /// arguments leave the stack before an error propagates, as they did when
    /// they were popped first.
    #[inline]
    fn call_plain_native_at(
        &mut self,
        callee_idx: usize,
        argc: usize,
        span: Span,
    ) -> Result<(), RuntimeError> {
        let Value::NativeFunction(native) = &self.stack[callee_idx] else {
            unreachable!("call_plain_native_at: callee is not a native");
        };
        debug_assert_eq!(native.vm_special, NativeSpecial::None);
        debug_assert_eq!(self.stack.len(), callee_idx + 1 + argc);
        if let Some(expected) = native.arity {
            if argc != expected {
                return Err(RuntimeError::wrong_arity(expected, argc, span));
            }
        }
        // Wrap in a flamegraph `Fn` span when the native is on the
        // request-path whitelist (see `span_log::is_request_path_native`);
        // cheap builtins are skipped to keep the chart readable.
        #[cfg(test)]
        PLAIN_NATIVE_CALLS.with(|n| n.set(n.get() + 1));
        let native_span = crate::serve::span_log::maybe_instrument_native(&native.name);
        let result = (native.func)(&self.stack[callee_idx + 1..]);
        drop(native_span);
        self.stack.truncate(callee_idx);
        let value = result.map_err(|e| RuntimeError::new(e, span))?;
        self.push(value);
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
                if let Some((InstanceMethodDef::Source(method), _)) =
                    class.find_instance_method_level(&cb_name)
                {
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
            // Level by level, as instance dispatch: a subclass's own callback
            // beats an ancestor's compiled one.
            let nearest = class.find_instance_method_level(cb_name);
            if let Some((InstanceMethodDef::Compiled(closure), defining)) = nearest {
                let result =
                    self.invoke_zero_arg_instance_method(instance, closure, defining, span)?;
                if matches!(result, Value::Bool(false)) {
                    return Ok(false);
                }
                continue;
            }
            if let Some((InstanceMethodDef::Source(method), declaring)) = nearest {
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

        if let Some((InstanceMethodDef::Compiled(closure), _)) =
            class.find_static_method_level("method_missing")
        {
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
            return Err(RuntimeError::no_such_property(
                class.name.clone(),
                name.to_string(),
                span,
            ));
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
        // A view sets this flag. A view helper compiles and runs here, its
        // closure's names bound for this call (`vm_view_helpers`); every other
        // function still takes the interpreter below, and so does a helper the
        // compiler refuses or a call its arguments keep there — an
        // `EngineFallback` from a view helper would demote the whole view.
        if self.tree_walk_functions {
            let callee_idx = self.stack.len() - 1 - argc;
            if let Some(helper) = super::vm_view_helpers::view_helper(func) {
                if let Some(closure) = helper.bind(func, &self.stack[callee_idx + 1..]) {
                    self.stack[callee_idx] = Value::VmClosure(Rc::clone(&closure));
                    return self.call_closure(closure, argc, span);
                }
            }
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
        // The fields start now; the defaults that are not literals run as
        // frames above the constructor's (or above the instance, the call's
        // value, when there is none), so they still run first.
        let defaults = self
            .start_fields(class, &instance, span)?
            .map(|plan| (plan, instance.clone()));
        let instance_val = Value::Instance(instance);

        match self.resolve_constructor(class, span)? {
            // A compiled constructor — the class's bytecode one, or a
            // tree-walker one compiled once in constructor mode — returns
            // `this`. The instance takes the callee slot, so it becomes slot
            // 0 (`this`) under the method calling convention, and the frame
            // records the class that declares the constructor, so the
            // `super(...)` inside reaches that class's superclass.
            Constructor::Compiled(ctor, defining_class) => {
                self.stack[callee_idx] = instance_val;
                self.call_closure_in_class(ctor, argc, span, Some(defining_class))?;
            }
            // Native constructor (a Rust-defined class such as RateLimiter):
            // it receives the instance and the arguments and fills the
            // fields — after the defaults, which run to completion first.
            Constructor::Native(init) => {
                if let Some((plan, instance)) = &defaults {
                    for (computed, owner) in &plan.computed {
                        let owner = owner.clone().unwrap_or_else(|| class.clone());
                        self.run_field_defaults(computed, owner, instance, span)?;
                    }
                }
                let mut init_args = Vec::with_capacity(argc + 1);
                init_args.push(instance_val.clone());
                init_args.extend(self.stack[callee_idx + 1..].iter().cloned());
                self.stack.truncate(callee_idx);
                (init.func)(&init_args).map_err(|e| RuntimeError::new(e, span))?;
                self.push(instance_val);
                return Ok(());
            }
            // No constructor: drop any args (tree-walker parity) and yield the
            // instance.
            Constructor::Absent => {
                self.stack.truncate(callee_idx);
                self.push(instance_val);
            }
        }
        match defaults {
            Some((plan, instance)) => self.push_field_defaults(&plan, class, &instance, span),
            None => Ok(()),
        }
    }

    /// The constructor a construction of `class` runs: the nearest class up
    /// the chain that declares one — compiled by the VM (`Class::vm_init`)
    /// or by the tree-walker (`Class::constructor`) — else a native one. A
    /// method, whatever its name, is never the constructor.
    pub(crate) fn resolve_constructor(
        &self,
        class: &Rc<Class>,
        span: Span,
    ) -> Result<Constructor, RuntimeError> {
        let mut level = class.clone();
        loop {
            let compiled = level.vm_init.borrow().clone();
            if let Some(init) = compiled {
                return Ok(Constructor::Compiled(init, level));
            }
            if let Some(ctor) = level.constructor.clone() {
                let closure = self.compiled_constructor(&ctor, &level, span)?;
                return Ok(Constructor::Compiled(closure, level));
            }
            match level.superclass.clone() {
                Some(superclass) => level = superclass,
                None => break,
            }
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
    /// `instance`, run to completion: the farthest class first, each class
    /// starting its fields (a literal default, else `nil`), then running its
    /// defaults that are not literals on the instance, before the next class
    /// down starts its own — what the tree-walker's
    /// `initialize_instance_fields` does. So a subclass's default wins, a
    /// field no constructor assigns reads as `nil`, and `super(...)` never
    /// sets them again. Every write is raw, as the tree-walker's: a `const`,
    /// `private` or frozen field takes its default.
    ///
    /// For the constructions that need the fields set before they go on (the
    /// attribute form, value objects, a native constructor). `call_class`
    /// starts them with [`Self::start_fields`] and runs the defaults as
    /// frames instead ([`Self::push_field_defaults`]).
    pub(crate) fn init_fields(
        &mut self,
        class: &Rc<Class>,
        instance: &Rc<RefCell<Instance>>,
        span: Span,
    ) -> Result<(), RuntimeError> {
        if let Some(plan) = self.start_fields(class, instance, span)? {
            for (computed, owner) in &plan.computed {
                let owner = owner.clone().unwrap_or_else(|| class.clone());
                self.run_field_defaults(computed, owner, instance, span)?;
            }
        }
        Ok(())
    }

    /// Start the declared fields of a new instance of `class` (see
    /// [`ConstructPlan`]): `Some(plan)` when defaults that are not literals
    /// are left to run, in `plan.computed`'s order, once every field has
    /// started. A chain that must start its fields class by class
    /// (`ConstructPlan::class_by_class`) runs them all here and answers
    /// `None`. One cached plan per class: a field map filled in one loop,
    /// with no walk of the chain.
    pub(crate) fn start_fields(
        &mut self,
        class: &Rc<Class>,
        instance: &Rc<RefCell<Instance>>,
        span: Span,
    ) -> Result<Option<Rc<ConstructPlan>>, RuntimeError> {
        let cached = class.construct_plan.borrow().clone();
        let plan = match cached {
            Some(plan) => plan,
            None => self.construct_plan(class, span)?,
        };
        if plan.class_by_class {
            self.init_fields_class_by_class(class, instance, span)?;
            return Ok(None);
        }
        plan.start(&mut instance.borrow_mut());
        Ok((!plan.computed.is_empty()).then_some(plan))
    }

    /// Run the defaults `plan` left to `instance`, the instance already in
    /// place as the call's value or its constructor's `this`: one frame per
    /// class that has some, each dropping its value
    /// ([`FrameReturn::Discard`]), pushed so the farthest class's runs
    /// first and all of them before the frame beneath (the constructor's)
    /// starts. In place of a nested run per instance that returned a hash.
    pub(crate) fn push_field_defaults(
        &mut self,
        plan: &ConstructPlan,
        class: &Rc<Class>,
        instance: &Rc<RefCell<Instance>>,
        span: Span,
    ) -> Result<(), RuntimeError> {
        for (computed, owner) in plan.computed.iter().rev() {
            let owner = owner.clone().unwrap_or_else(|| class.clone());
            self.push(Value::Instance(instance.clone()));
            self.call_closure_in_class(computed.clone(), 0, span, Some(owner))?;
            if let Some(frame) = self.frames.last_mut() {
                frame.on_return = FrameReturn::Discard;
            }
        }
        Ok(())
    }

    /// One class's defaults that are not literals, run on `instance` now.
    fn run_field_defaults(
        &mut self,
        computed: &Rc<VmClosure>,
        owner: Rc<Class>,
        instance: &Rc<RefCell<Instance>>,
        span: Span,
    ) -> Result<(), RuntimeError> {
        self.push(Value::Instance(instance.clone()));
        self.run_to_completion(|vm| {
            vm.call_closure_in_class(computed.clone(), 0, span, Some(owner))
        })?;
        Ok(())
    }

    /// [`Self::init_fields`] one class at a time, the farthest first.
    fn init_fields_class_by_class(
        &mut self,
        class: &Rc<Class>,
        instance: &Rc<RefCell<Instance>>,
        span: Span,
    ) -> Result<(), RuntimeError> {
        if let Some(superclass) = &class.superclass {
            self.init_fields_class_by_class(superclass, instance, span)?;
        }
        let Some(init) = self.field_init_of(class, span)? else {
            return Ok(());
        };
        init.start(&mut instance.borrow_mut());
        if let Some(computed) = &init.computed {
            self.run_field_defaults(computed, class.clone(), instance, span)?;
        }
        Ok(())
    }

    /// `class`'s [`ConstructPlan`], built from the [`FieldInit`] of each
    /// class of its chain and kept on the class.
    fn construct_plan(
        &mut self,
        class: &Rc<Class>,
        span: Span,
    ) -> Result<Rc<ConstructPlan>, RuntimeError> {
        let mut chain = Vec::new();
        let mut level = Some(class.clone());
        while let Some(current) = level {
            level = current.superclass.clone();
            chain.push(current);
        }
        chain.reverse();
        let mut plan = ConstructPlan::default();
        let mut computed_above = false;
        for current in &chain {
            let Some(init) = self.field_init_of(current, span)? else {
                continue;
            };
            if computed_above && !init.fields.is_empty() {
                plan.class_by_class = true;
            }
            for (name, start) in &init.fields {
                match plan.starts.iter_mut().find(|(started, _)| started == name) {
                    Some(slot) => slot.1 = start.clone(),
                    None => plan.starts.push((name.clone(), start.clone())),
                }
            }
            if let Some(computed) = &init.computed {
                computed_above = true;
                let owner = (!Rc::ptr_eq(current, class)).then(|| current.clone());
                plan.computed.push((computed.clone(), owner));
            }
        }
        let plan = Rc::new(plan);
        *class.construct_plan.borrow_mut() = Some(plan.clone());
        Ok(plan)
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
        let names = crate::interpreter::executor::access::member::declared_field_order(class);
        let mut fields = Vec::with_capacity(names.len());
        let mut computed = Vec::new();
        for name in names {
            let default = class.fields.get(&name).and_then(Option::as_ref);
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

        let computed = if computed.is_empty() {
            None
        } else {
            // Compiled as one of the class's methods: in their environment
            // (its namespace) and their file.
            let sample = class
                .constructor
                .clone()
                .or_else(|| class.methods.borrow().values().next().cloned());
            let context = Function {
                closure: match (&class.defaults_env, &sample) {
                    (Some(env), _) => env.clone(),
                    (None, Some(sample)) => sample.closure.clone(),
                    (None, None) => Function::default().closure,
                },
                source_path: sample.and_then(|f| f.source_path.clone()),
                ..Function::default()
            };
            let proto = Compiler::compile_field_defaults_standalone(
                &computed,
                &context,
                self.globals.keys().cloned(),
            )
            .map_err(|e| {
                RuntimeError::EngineFallback(
                    format!("a field default the VM cannot compile ({})", e),
                    span,
                )
            })?;
            Some(Rc::new(VmClosure::new(Arc::new(proto), Vec::new())))
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
        #[cfg(test)]
        SLOW_PATH_CALLS.with(|calls| calls.set(calls.get() + 1));
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
            // A static resolves once, in one walk of the chain, and is called
            // from here: the compiled form, or a tree-walker `static def`
            // compiled on its first call. Only what this cannot call directly
            // (a static run by the tree-walker in a view, a model native, a
            // static field, reflection, a finder) takes the property path
            // below, which looks the name up again.
            match resolve_static_callee(&class, name, !self.tree_walk_functions) {
                Some(StaticCallee::Compiled(closure)) => {
                    return self.push_method_frame(closure, argc, Some(class));
                }
                Some(StaticCallee::Plain(closure)) => {
                    self.stack[receiver_idx] = Value::VmClosure(closure.clone());
                    return self.push_method_frame(closure, argc, None);
                }
                Some(StaticCallee::TreeWalker {
                    func,
                    defining,
                    own,
                }) => {
                    if func.binds_this {
                        let closure = self.compile_tree_walking_static(&func, &defining, name)?;
                        return self.push_method_frame(closure, argc, Some(class));
                    }
                    if !self.tree_walk_functions {
                        let closure = self.compile_plain_static(&func, &defining, name, own)?;
                        self.stack[receiver_idx] = Value::VmClosure(closure.clone());
                        return self.push_method_frame(closure, argc, None);
                    }
                    // A view runs the other functions on the tree-walker
                    // (`call_native_wrapper`): the property path hands it
                    // the `Function`.
                }
                None => {
                    // A native static: `Math.floor(x)`, `DateTime.utc()`. A
                    // model's natives take the class as their first argument
                    // (`bind_native_static_to_model_class`), and a static
                    // field of the same name is read first, as
                    // `op_get_property` orders them: both stay on that path.
                    let native = class.find_native_static_method(name);
                    if let Some(native) = &native {
                        if !is_model && !class.static_fields.borrow().contains_key(name) {
                            let native = (**native).clone();
                            let span = self.current_span();
                            self.stack[receiver_idx] = Value::NativeFunction(native.clone());
                            return self.call_native(&native, argc, span);
                        }
                    }
                    // Class-level `method_missing` is the LAST resort,
                    // matching `class_member_access`: statics, then the
                    // reflection surface (`send`, `methods`,
                    // `define_method`, …), then dynamic finders. Consulting
                    // it first meant that on any class defining a static
                    // `method_missing` — the `Mailer` prelude shape —
                    // `Foo.send("bar")` and `User.find_by_email("x")`
                    // dispatched into `method_missing` and returned a wrong
                    // value instead of the reflection result.
                    //
                    // A parsed dynamic finder (`find_by_email`) is known, so
                    // `method_missing` does not take it. `find_by_` parses as
                    // nothing and stays free for `method_missing`;
                    // `find_by_sql` is a native static, matched above.
                    let known = native.is_some()
                        || crate::vm::vm_classes::is_class_reflection_member(name)
                        || (is_model
                            && crate::interpreter::executor::access::member::parse_dynamic_finder(
                                name,
                            )
                            .is_ok());
                    if !known
                        && (class.find_static_method("method_missing").is_some()
                            || class.find_vm_static_method("method_missing").is_some())
                    {
                        let span = self.current_span();
                        return self.call_class_method_missing(receiver_idx, argc, name, span);
                    }
                }
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
                        Value::String(s) | Value::Symbol(s) => Some(s.clone()),
                        _ => None,
                    };
                    if let Some(target) = target {
                        self.stack.remove(receiver_idx + 1);
                        return self.call_method_slow_path(receiver_idx, argc - 1, &target);
                    }
                }
            }
        }
        // `Klass.send(...)` is not re-dispatched here: it falls to the class
        // reflection refusal (`is_class_reflection_member`), and `soli serve`
        // re-runs the action on the tree-walker, whose class `send` binds by
        // position, applies no defaults and wraps a throw ("Exception in
        // send: …"). Calling the static with full method semantics answered
        // a `DomainError` thrown through it with a 422 in production and a
        // 500 under `--dev`.
        let compiled = match &self.stack[receiver_idx] {
            Value::Instance(inst) => {
                let class = inst.borrow().class.clone();
                // A field of that name shadows a method not compiled yet,
                // as before; otherwise the nearest definition wins.
                if inst.borrow().fields.contains_key(name) {
                    match class.find_instance_method_level(name) {
                        Some((InstanceMethodDef::Compiled(closure), defining)) => {
                            Some((closure, defining))
                        }
                        _ => None,
                    }
                } else {
                    self.resolve_instance_method(&class, name)?
                }
            }
            // A class receiver's statics were resolved above.
            _ => None,
        };
        if let Some((closure, defining_class)) = compiled {
            return self.push_method_frame(closure, argc, Some(defining_class));
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

        if argc == 1 {
            if let Some(answer) = Self::instance_reflection_answer(
                &self.stack[receiver_idx],
                name,
                &self.stack[receiver_idx + 1],
            ) {
                self.stack.truncate(receiver_idx);
                self.stack.push(Value::Bool(answer));
                return Ok(());
            }
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
                    let found = self.resolve_instance_method(&class, "method_missing")?;
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

    /// `obj.is_a?("Base")` and `obj.respond_to?("m")` on a plain (non-model)
    /// instance, answered by walking the class: the result the universal
    /// members of `op_get_property` and the tree-walker's
    /// `instance_member_access` give, without the `NativeFunction` closure
    /// built per access for `is_a?` or the tree-walker `Interpreter` built
    /// per access for `respond_to?`.
    ///
    /// Called by `call_method_slow_path` once no compiled, tree-walker or
    /// native method of that name exists; a field of that name, a model
    /// instance (whose reads go to the model resolver) or an argument of
    /// another type returns `None`, and the general path answers — or raises
    /// the same error as before.
    fn instance_reflection_answer(receiver: &Value, name: &str, arg: &Value) -> Option<bool> {
        let Value::Instance(inst) = receiver else {
            return None;
        };
        if !matches!(name, "is_a?" | "respond_to?") {
            return None;
        }
        let inst = inst.borrow();
        if inst.class.is_model_subclass() || inst.fields.contains_key(name) {
            return None;
        }
        if name == "is_a?" {
            let Value::String(class_name) = arg else {
                return None;
            };
            let mut current = Some(&inst.class);
            while let Some(class) = current {
                if class.name == class_name.as_str() {
                    return Some(true);
                }
                current = class.superclass.as_ref();
            }
            return Some(false);
        }
        let method = match arg {
            Value::String(method) | Value::Symbol(method) => method.as_str(),
            _ => return None,
        };
        // The tree-walker's `respond_to?` list, kept in the same order.
        let class = &inst.class;
        Some(
            class.find_method(method).is_some()
                || class.find_vm_method(method).is_some()
                || class.find_native_method(method).is_some()
                || matches!(
                    method,
                    "inspect"
                        | "class"
                        | "nil?"
                        | "blank?"
                        | "present?"
                        | "respond_to?"
                        | "send"
                        | "instance_variables"
                        | "instance_variable_get"
                        | "instance_variable_set"
                        | "methods"
                        | "method_missing"
                ),
        )
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
        // The span only on the cold branch: building it every call showed
        // in profiles. The handler budget is checked on every frame, as
        // `ensure_call_depth` does: recursion through a method has no
        // backward jump, so this is where it stops (expiry is sticky, so
        // `ensure_call_depth` reports it).
        if self.frames.len() >= MAX_CALL_DEPTH || crate::interpreter::deadline::expired() {
            self.ensure_call_depth(&closure.proto, self.current_span())?;
        }
        self.frames.push(CallFrame::new(
            closure,
            receiver_idx,
            self.iter_stack.len(),
            Some(defining_class),
            crate::vm::vm::positional_supplied_mask(params),
        ));
        Ok(())
    }

    /// The instance method `name` of `class`, compiled. One that exists only
    /// as AST — such as one inherited from
    /// a class the VM never compiled, such as a controller's base class — as
    /// a *method* and register the bytecode on the class that defines it, so
    /// later calls take the compiled-method hot path.
    ///
    /// Without this the call fell through to `op_get_property`, which returns
    /// the unbound `Function`, and `call_native_wrapper` compiled it as a plain
    /// function and put the closure in the callee slot — the slot a method
    /// reads as `this`. `this._base()` inside it then failed with
    /// "Cannot access property '_base' on Function".
    ///
    /// The lookup goes class by class from the receiver's up — that class's
    /// compiled method, then its AST one — so a subclass override that was
    /// never compiled still beats an ancestor's method that was. Taking the
    /// first compiled method anywhere in the chain before compiling the
    /// receiver's own skipped the override for good once the parent had been
    /// called on that worker (an authorization fail-open for policies).
    pub(crate) fn resolve_instance_method(
        &self,
        class: &Rc<Class>,
        name: &str,
    ) -> Result<Option<MethodWithClass>, RuntimeError> {
        let (method, defining) = match class.find_instance_method_level(name) {
            None => return Ok(None),
            Some((InstanceMethodDef::Compiled(closure), defining)) => {
                return Ok(Some((closure, defining)))
            }
            Some((InstanceMethodDef::Source(method), defining)) => (method, defining),
        };
        let proto = jit_compile_method(&method, self.globals.keys().cloned()).map_err(|e| {
            RuntimeError::EngineFallback(
                format!("a method the VM cannot compile ({})", e),
                self.current_span(),
            )
        })?;
        let closure = Rc::new(VmClosure::new(proto, Vec::new()));
        defining.insert_vm_method(name.to_string(), closure.clone());
        Ok(Some((closure, defining)))
    }

    /// Open the frame for a compiled method or static whose callee slot and
    /// `argc` arguments are on the stack: missing optional parameters are
    /// padded with `nil`, which the callee's prologue replaces with their
    /// defaults. `class` is the frame's class (`None` for a plain function).
    /// The span is computed only on the cold error branches.
    #[inline]
    pub(super) fn push_method_frame(
        &mut self,
        closure: Rc<VmClosure>,
        argc: usize,
        class: Option<Rc<Class>>,
    ) -> Result<(), RuntimeError> {
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
        // The span only on the cold branch: building it every call showed
        // in profiles. The handler budget is checked on every frame, as
        // `ensure_call_depth` does: recursion through a method has no
        // backward jump, so this is where it stops (expiry is sticky, so
        // `ensure_call_depth` reports it).
        if self.frames.len() >= MAX_CALL_DEPTH || crate::interpreter::deadline::expired() {
            self.ensure_call_depth(&closure.proto, self.current_span())?;
        }
        self.frames.push(CallFrame::new(
            closure,
            stack_base,
            self.iter_stack.len(),
            class,
            crate::vm::vm::positional_supplied_mask(argc),
        ));
        Ok(())
    }

    /// A tree-walker `static def` that reads `this`, compiled as a method
    /// and registered on the class that defines it: the class it is called
    /// on, in the callee slot, is its `this`.
    fn compile_tree_walking_static(
        &self,
        func: &Function,
        defining: &Rc<Class>,
        name: &str,
    ) -> Result<Rc<VmClosure>, RuntimeError> {
        let proto = jit_compile_method(func, self.globals.keys().cloned()).map_err(|e| {
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
        Ok(closure)
    }

    /// Any other tree-walker `static def`, compiled as the plain function
    /// `call_native_wrapper` makes of it (the same bytecode, cached on the
    /// `Function`). One of the class's own `static_methods` is remembered
    /// on it (`own`), so the next call finds the closure in one probe and
    /// allocates nothing. A method mixed in by `extend` is not: a reopened
    /// module replaces those, and nothing would tell the cache.
    fn compile_plain_static(
        &self,
        func: &Function,
        defining: &Rc<Class>,
        name: &str,
        own: bool,
    ) -> Result<Rc<VmClosure>, RuntimeError> {
        let proto = jit_compile_function(func, self.globals.keys().cloned()).map_err(|e| {
            RuntimeError::EngineFallback(
                format!("a function the VM cannot compile ({})", e),
                self.current_span(),
            )
        })?;
        let closure = Rc::new(VmClosure::new(proto, Vec::new()));
        if own {
            defining
                .vm_plain_static_methods
                .insert(name, closure.clone());
        }
        Ok(closure)
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
                return Err(RuntimeError::no_such_property(
                    receiver.type_name(),
                    method_name.to_string(),
                    span,
                ))
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
        //
        // Run to completion at this depth: called while the VM is already
        // running (a bare `Post.recent` static that reads `this`), a plain
        // `self.run()` kept going past this frame's return into the caller's
        // frames — running the rest of the action inside this call, side
        // effects included — and the outer loop then panicked on an empty
        // frame stack. At the top of a request the depth is 0, as before.
        self.run_to_completion(|vm| {
            vm.push(instance);
            for arg in args {
                vm.push(arg.clone());
            }
            vm.call_closure_in_class(closure, args.len(), span, class)
        })
    }

    /// Reset VM state between requests (preserves globals).
    ///
    /// Upvalues still open are closed first, over the value their slot holds
    /// now. A request that ended with an error (or a panic) leaves its frames
    /// on the VM, and a closure it stored somewhere that outlives it
    /// (`const STORE = []`) still points at an absolute stack slot: once the
    /// stack is cleared, that slot belongs to whatever request runs next, so
    /// the closure read and wrote another visitor's locals.
    pub fn reset(&mut self) {
        if !self.open_upvalues.is_empty() {
            self.close_all_upvalues();
        }
        self.stack.clear();
        self.frames.clear();
        self.open_upvalues.clear();
        self.open_upvalue_top = 0;
        self.exception_handlers.clear();
        self.iter_stack.clear();
        self.return_depth = 0;
    }

    /// Close every open upvalue over its slot's current value. Tolerates a
    /// stack shorter than a slot (a panic can stop the VM anywhere): the cell
    /// is closed over `nil` rather than left pointing past the end.
    ///
    /// A cell nothing else holds can never be read again: it is dropped, as
    /// `close_upvalues` would pool it, rather than given a value.
    fn close_all_upvalues(&mut self) {
        for (slot, upvalue) in std::mem::take(&mut self.open_upvalues) {
            if Rc::strong_count(&upvalue) == 1 && Rc::weak_count(&upvalue) == 0 {
                continue;
            }
            let Ok(mut cell) = upvalue.try_borrow_mut() else {
                continue;
            };
            if matches!(*cell, Upvalue::Open(_)) {
                *cell = Upvalue::Closed(self.stack.get(slot).cloned().unwrap_or(Value::Null));
            }
        }
        self.open_upvalue_top = 0;
    }

    /// Put the VM back in a state a new request can run on after a handler
    /// panicked somewhere inside it.
    ///
    /// A panic unwinds past every `vm.reset()` the serve paths make after a
    /// call, so the stack, the frames, `iter_stack` and `return_depth` of the
    /// dead request stayed. The next request's action frame then returned
    /// *into* the dead request's frames: its remaining code ran under the new
    /// request and its value became the new request's response. Also drops
    /// what a loop or a render had in hand when it stopped.
    pub fn recover_after_panic(&mut self) {
        self.reset();
        self.spare_callback_loop = None;
        self.template_out.clear();
        self.template_captures.clear();
        self.template_content_before = None;
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
    ///
    /// Only for an iterator's block. The block binds as many arguments as it
    /// declares (see `block_argc`), so a block with no parameter runs
    /// without the element.
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
            // `arity` counts the required parameters only.
            let arity = closure.proto.arity as usize;
            if arity >= 2 {
                // Missing required positions read as nil. A defaulted one
                // takes its part when the element has one, and otherwise
                // keeps its default. The parts go straight onto the stack,
                // with no argument vector built and then copied there.
                let declared = closure.proto.param_names.len();
                self.push(callee.clone());
                let count = {
                    let parts = parts.borrow();
                    let count = parts.len().min(declared).max(arity);
                    for index in 0..count {
                        self.stack
                            .push(parts.get(index).cloned().unwrap_or(Value::Null));
                    }
                    count
                };
                self.call_value(count, span)?;
                if self.frames.len() == batch.frames_before {
                    return Ok(self.pop());
                }
                return self.run();
            }
        }
        self.push(callee.clone());
        // A block that declares no parameter ignores the element:
        // `3.times { … }`, `xs.each { print("tick") }`.
        let argc = block_argc(callee, 1);
        if argc == 1 {
            self.push(arg);
        }
        self.call_value(argc, span)?;
        if self.frames.len() == batch.frames_before {
            return Ok(self.pop());
        }
        self.run()
    }

    /// Two-argument form of [`Self::invoke_in_batch_one`]. The block takes as
    /// many of `a`, `b` as it declares (see `block_argc`):
    /// `each_with_index { |x| … }` gets the element alone, and `{ … }` gets
    /// neither.
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
        let argc = block_argc(callee, 2);
        if argc >= 1 {
            self.push(a);
        }
        if argc == 2 {
            self.push(b);
        }
        self.call_value(argc, span)?;
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
    ///
    /// Strict, unlike the block forms. Every argument is passed, so `h.f(5)`
    /// on `fn() { … }` raises here as it does on the tree-walker.
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
mod implicit_self_call_tests {
    //! A bare call inside an instance method (`_step(i)`, no `@`) resolves
    //! once, in `SelfOrGlobalCallee`, which leaves its decision on the stack:
    //! `CallSelfOrGlobal` enters a compiled method directly instead of
    //! dispatching by name, which probed globals and walked the class chain a
    //! second time.
    use super::slow_path_calls;
    use crate::interpreter::value::Value;
    use crate::vm::compiler::Compiler;
    use crate::vm::Vm;

    /// Run `src` on the VM; return its `result` global and how many times it
    /// entered `call_method_slow_path`.
    fn run(src: &str) -> (Value, usize) {
        let tokens = crate::lexer::Scanner::new(src).scan_tokens().expect("lex");
        let program = crate::parser::Parser::new(tokens).parse().expect("parse");
        let module = Compiler::compile(&program).expect("compile");
        let mut vm = Vm::new();
        let start = slow_path_calls();
        vm.execute(&module.main).expect("vm");
        let spent = slow_path_calls() - start;
        (vm.globals.get("result").cloned().expect("result"), spent)
    }

    const LOOP: &str = r#"
class Base
  def inherited(i)
    i * 10
  end
end

class Counter < Base
  def run(n)
    total = 0
    i = 0
    while i < n
      total = total + _step(i) + inherited(1) + _add(_step(1), _step(2))
      i = i + 1
    end
    total
  end

  private

  def _step(i)
    i
  end

  def _add(a, b)
    a + b
  end
end
result = new Counter().run(__N__)
"#;

    /// The bare calls in the loop never reach the slow path, whatever `n` is.
    #[test]
    fn bare_calls_to_compiled_methods_skip_the_slow_path() {
        let (one, spent_one) = run(&LOOP.replace("__N__", "1"));
        let (ten, spent_ten) = run(&LOOP.replace("__N__", "10"));
        assert_eq!(one, Value::Int(13));
        assert_eq!(ten, Value::Int(45 + 10 * 13));
        assert_eq!(
            spent_one, spent_ten,
            "a bare call to a compiled method must not dispatch by name"
        );
        // Not even `counter.run(n)`: a call on an instance whose compiled
        // method takes exactly its arguments stays in the fast tier.
        assert_eq!(spent_one, 0, "no call takes the slow path");
    }

    /// The marker `SelfOrGlobalCallee` pushes is dropped on every path: a
    /// global callee, a method callee, and an error raised among the
    /// arguments and rescued in the same method.
    #[test]
    fn the_marker_never_outlives_its_call() {
        let src = r#"
def twice(x)
  x * 2
end

class Probe
  def run(n)
    out = []
    i = 0
    while i < n
      out.push(twice(_id(i)))
      boom = 0
      try
        boom = _id(_fail(i))
      catch e
        boom = -1
      end
      out.push(boom)
      i = i + 1
    end
    out
  end

  def _id(x)
    x
  end

  def _fail(x)
    throw "no"
  end
end
result = new Probe().run(3)
"#;
        let (result, _) = run(src);
        let Value::Array(items) = result else {
            panic!("expected an array, got {result:?}");
        };
        let items: Vec<Value> = items.borrow().clone();
        assert_eq!(
            items,
            vec![
                Value::Int(0),
                Value::Int(-1),
                Value::Int(2),
                Value::Int(-1),
                Value::Int(4),
                Value::Int(-1),
            ]
        );
    }

    /// An unknown bare name raises "Undefined variable" before its arguments
    /// run, as the tree-walker resolves the callee first.
    #[test]
    fn unknown_bare_name_raises_before_the_arguments_run() {
        let src = r##"
class Probe
  ran: Int

  def run
    @ran = 0
    msg = ""
    try
      nope(@_bump)
    catch e
      msg = "#{e}"
    end
    [msg, @ran]
  end

  def _bump
    @ran = @ran + 1
    1
  end
end
result = new Probe().run
"##;
        let (result, _) = run(src);
        let Value::Array(items) = result else {
            panic!("expected an array, got {result:?}");
        };
        let items: Vec<Value> = items.borrow().clone();
        let Value::String(msg) = &items[0] else {
            panic!("expected a message, got {items:?}");
        };
        assert!(
            msg.as_str().contains("Undefined variable") && msg.as_str().contains("nope"),
            "unexpected message: {msg}"
        );
        assert_eq!(items[1], Value::Int(0), "the arguments must not run");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A VM seeded with the globals `source` defines, as a serve worker seeds
    /// its VM from the loaded app.
    fn vm_for(source: &str) -> (crate::interpreter::Interpreter, Vm) {
        let tokens = crate::lexer::Scanner::new(source)
            .scan_tokens()
            .expect("lexer error");
        let program = crate::parser::Parser::new(tokens)
            .parse()
            .expect("parser error");
        let mut interpreter = crate::interpreter::Interpreter::new();
        interpreter.interpret(&program).expect("program runs");
        let mut vm = Vm::new();
        for (name, value) in interpreter.global_env().borrow().get_all_bindings() {
            vm.globals.insert(name, value);
        }
        (interpreter, vm)
    }

    /// A closure that escaped a request which then failed must not read or
    /// write the next request's stack. The request's frames stay on the VM
    /// after an uncaught error, so the closure's upvalue was still open on an
    /// absolute slot when `reset` cleared the stack — and that slot was the
    /// next visitor's local.
    #[test]
    fn reset_closes_the_upvalues_a_failed_request_left_open() {
        let (interpreter, mut vm) = vm_for(
            r#"
const STORE = []
def leak()
  secret = "alice-secret"
  STORE.push(fn() { secret })
  throw "boom"
end
def read()
  bob_token = "bob-token-123"
  STORE[0]()
end
"#,
        );
        let leak = interpreter.global_env().borrow().get("leak").expect("leak");
        let read = interpreter.global_env().borrow().get("read").expect("read");

        assert!(vm.call_value_direct(leak, &[], Span::default()).is_err());
        vm.reset();
        assert!(vm.open_upvalues.is_empty());

        let seen = vm
            .call_value_direct(read, &[], Span::default())
            .expect("read runs");
        vm.reset();
        assert_eq!(
            seen,
            Value::String("alice-secret".into()),
            "the stored closure must keep its own request's value, not read the next request's local"
        );
    }

    /// The same through the branch's reuse paths: the closures escape from
    /// a block the in-VM callback loop reruns in place (its argument slot
    /// overwritten per element), the request fails mid-loop, and the next
    /// request makes capturing blocks of its own, which take cells from
    /// `upvalue_pool` and closures from `closure_spares`. Neither `reset`
    /// nor `recover_after_panic` may leave a cell open on a slot, pool a cell
    /// a stored closure still holds, or keep the dead loop.
    #[test]
    fn a_failed_block_loop_leaves_no_cell_open_or_pooled_for_the_next_request() {
        let (interpreter, mut vm) = vm_for(
            r##"
const STORE = []
def leak()
  secret = "alice-secret"
  [1, 2, 3].each do |n|
    STORE.push(fn() { "#{secret}-#{n}" })
    throw "boom" if n == 2
  end
end
def read()
  bob_token = "bob-token-123"
  shown = [10, 20].map { |k| "#{bob_token}:#{k}" }
  "#{STORE[0]()} #{STORE[1]()} #{shown.join(",")}"
end
"##,
        );
        let leak = interpreter.global_env().borrow().get("leak").expect("leak");
        let read = interpreter.global_env().borrow().get("read").expect("read");
        let expected =
            "alice-secret-1 alice-secret-2 bob-token-123:10,bob-token-123:20".to_string();

        for recover in [false, true] {
            if let Value::Array(store) = vm.globals.get("STORE").cloned().expect("STORE") {
                store.borrow_mut().clear();
            }
            assert!(vm
                .call_value_direct(leak.clone(), &[], Span::default())
                .is_err());
            if recover {
                vm.recover_after_panic();
                assert!(vm.spare_callback_loop.is_none());
            } else {
                vm.reset();
            }
            assert!(vm.open_upvalues.is_empty());
            assert!(vm.iter_stack.is_empty());
            assert!(vm.frames.is_empty() && vm.stack.is_empty());
            for _ in 0..2 {
                let seen = vm
                    .call_value_direct(read.clone(), &[], Span::default())
                    .expect("read runs");
                vm.reset();
                assert_eq!(seen, Value::String(expected.clone().into()));
                assert!(
                    vm.upvalue_pool
                        .iter()
                        .all(|cell| Rc::strong_count(cell) == 1
                            && matches!(*cell.borrow(), Upvalue::Open(_))),
                    "a pooled cell is held by nothing else"
                );
            }
        }
    }

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

    // --- Static calls on classes the tree-walker loaded (`soli serve`) ---

    const STATIC_CLASSES: &str = r#"
module Twice
  def twice(n)
    n * 2
  end
end

class Base
  static def who
    this
  end

  static def label(n)
    "base #{n}"
  end
end

class Svc < Base
  extend Twice

  static def who
    "svc"
  end

  static def compute(n, m = 10)
    n + m
  end

  static def fact(n)
    return 1 if n <= 1

    n * Svc.fact(n - 1)
  end
end
"#;

    /// Load `STATIC_CLASSES` on the tree-walker and seed a VM with its
    /// globals, as a serve worker loads an app.
    fn static_classes_vm() -> (crate::interpreter::Interpreter, Vm) {
        let tokens = crate::lexer::Scanner::new(STATIC_CLASSES)
            .scan_tokens()
            .expect("lexer error");
        let program = crate::parser::Parser::new(tokens)
            .parse()
            .expect("parser error");
        let mut interpreter = crate::interpreter::Interpreter::new();
        interpreter.interpret(&program).expect("classes load");
        let mut vm = Vm::new();
        for (name, value) in interpreter.environment.borrow().get_all_bindings() {
            vm.globals.insert(name, value);
        }
        (interpreter, vm)
    }

    fn loaded_class(interpreter: &crate::interpreter::Interpreter, name: &str) -> Rc<Class> {
        match interpreter.environment.borrow().get(name) {
            Some(Value::Class(class)) => class,
            other => panic!("{name} is not a class: {other:?}"),
        }
    }

    /// Run `body` as a function compiled by this VM.
    fn run_probe(vm: &mut Vm, body: &str) -> Result<Value, RuntimeError> {
        let source = format!("def probe()\n{body}\nend\n");
        let tokens = crate::lexer::Scanner::new(&source)
            .scan_tokens()
            .expect("lexer error");
        let program = crate::parser::Parser::new(tokens)
            .parse()
            .expect("parser error");
        let module = Compiler::compile_with_globals(&program, vm.globals.keys().cloned())
            .expect("probe compiles");
        let proto = module
            .main
            .chunk
            .constants
            .iter()
            .find_map(|c| match c {
                Constant::Function(p) => Some(p.clone()),
                _ => None,
            })
            .expect("probe proto");
        vm.execute(&proto)
    }

    /// A `static def` that does not read `this` is compiled once and kept on
    /// its class: the next call reuses that closure instead of resolving the
    /// name through the property path and allocating a new one. It is not a
    /// bytecode static, which runs under another calling convention.
    #[test]
    fn plain_tree_walker_static_is_compiled_once_and_cached() {
        let (interpreter, mut vm) = static_classes_vm();
        let svc = loaded_class(&interpreter, "Svc");
        assert!(svc.vm_plain_static_methods.get("compute").is_none());

        let value = run_probe(&mut vm, "Svc.compute(1) + Svc.compute(2, 3)").expect("runs");
        assert_eq!(value, Value::Int(16));
        let cached = svc
            .vm_plain_static_methods
            .get("compute")
            .expect("compute is cached on Svc");
        assert!(!svc.vm_static_methods.borrow().contains_key("compute"));

        let again = run_probe(&mut vm, "Svc.compute(4)").expect("runs");
        assert_eq!(again, Value::Int(14));
        let still = svc.vm_plain_static_methods.get("compute").expect("cached");
        assert!(Rc::ptr_eq(&cached, &still), "the closure is reused");

        assert_eq!(
            run_probe(&mut vm, "Svc.fact(5)").expect("runs"),
            Value::Int(120)
        );
    }

    /// `Base.who` reads `this`, so its first call registers it as a bytecode
    /// static of `Base`. `Svc.who` hides it. The bytecode lookup used to walk
    /// the whole chain before any tree-walker static, so once `Base.who` had
    /// run, `Svc.who()` ran Base's with `Svc` as `this`.
    #[test]
    fn subclass_static_hides_a_compiled_superclass_static() {
        let (interpreter, mut vm) = static_classes_vm();
        let base = loaded_class(&interpreter, "Base");

        let first = run_probe(&mut vm, "Base.who()").expect("runs");
        assert_eq!(first, Value::Class(base.clone()));
        assert!(base.vm_static_methods.borrow().contains_key("who"));

        let value = run_probe(&mut vm, "Svc.who()").expect("runs");
        assert_eq!(value, Value::str("svc"));
    }

    /// An inherited static is cached on the class that defines it.
    #[test]
    fn inherited_plain_static_is_cached_on_its_defining_class() {
        let (interpreter, mut vm) = static_classes_vm();
        let value = run_probe(&mut vm, "Svc.label(4)").expect("runs");
        assert_eq!(value, Value::str("base 4"));
        let base = loaded_class(&interpreter, "Base");
        let svc = loaded_class(&interpreter, "Svc");
        assert!(base.vm_plain_static_methods.get("label").is_some());
        assert!(svc.vm_plain_static_methods.get("label").is_none());
    }

    /// A method `extend` mixed in runs, but is not cached: a reopened module
    /// replaces those entries, and nothing would tell the cache.
    #[test]
    fn extended_module_method_runs_without_being_cached() {
        let (interpreter, mut vm) = static_classes_vm();
        let value = run_probe(&mut vm, "Svc.twice(21)").expect("runs");
        assert_eq!(value, Value::Int(42));
        let svc = loaded_class(&interpreter, "Svc");
        assert!(svc.vm_plain_static_methods.get("twice").is_none());
    }

    /// A view runs plain functions on the tree-walker; a static is no
    /// exception, and is neither compiled nor cached there.
    #[test]
    fn view_vm_leaves_plain_statics_to_the_tree_walker() {
        let (interpreter, mut vm) = static_classes_vm();
        vm.tree_walk_functions = true;
        let value = run_probe(&mut vm, "Svc.compute(1, 2)").expect("runs");
        assert_eq!(value, Value::Int(3));
        let svc = loaded_class(&interpreter, "Svc");
        assert!(svc.vm_plain_static_methods.get("compute").is_none());
    }

    /// Arity is checked on the cached path as on the first call.
    #[test]
    fn plain_static_arity_is_checked() {
        let (_interpreter, mut vm) = static_classes_vm();
        assert!(run_probe(&mut vm, "Svc.compute(1)").is_ok());
        let err = run_probe(&mut vm, "Svc.compute()").expect_err("too few arguments");
        assert!(matches!(err, RuntimeError::WrongArity { .. }), "{err}");
    }

    /// A native static is called straight from the resolution.
    #[test]
    fn native_static_is_called_directly() {
        let (_interpreter, mut vm) = static_classes_vm();
        let value = run_probe(&mut vm, "str(Math.floor(2.7))").expect("runs");
        assert_eq!(value, Value::str("2"));
    }
}

#[cfg(test)]
mod plain_native_call_tests {
    //! A global native call runs on the stack slice (`call_plain_native_at`):
    //! no argument `Vec`, no callee clone, no name compares. The few natives
    //! the VM intercepts (`grouped`, `defined`, …) are tagged once at
    //! construction and still take their own path.
    use super::plain_native_calls;
    use crate::interpreter::value::{NativeFunction, NativeSpecial, Value};
    use crate::vm::compiler::Compiler;
    use crate::vm::Vm;

    /// Run `src` on a VM seeded with the builtins, as `soli --vm` seeds it;
    /// return its `result` global and how many natives it called on the
    /// stack slice.
    fn run(src: &str) -> (Value, usize) {
        let tokens = crate::lexer::Scanner::new(src).scan_tokens().expect("lex");
        let program = crate::parser::Parser::new(tokens).parse().expect("parse");
        let module = Compiler::compile(&program).expect("compile");
        let interpreter = crate::interpreter::Interpreter::new();
        let mut vm = Vm::new();
        for (name, value) in interpreter.environment.borrow().get_all_bindings() {
            vm.globals.insert(name, value);
        }
        let start = plain_native_calls();
        vm.execute(&module.main).expect("vm");
        let spent = plain_native_calls() - start;
        let result = vm.globals.get("result").cloned().expect("result");
        (result, spent)
    }

    #[test]
    fn global_natives_run_on_the_stack_slice() {
        let (result, calls) = run(r#"
total = 0
i = 0
while i < 100
  total = total + int(str(i)) + len([i, i, i])
  i = i + 1
end
result = total
"#);
        assert_eq!(result, Value::Int(4950 + 300));
        // `int`, `str` and `len` once per iteration, each on the slice.
        assert!(calls >= 300, "only {calls} natives took the slice path");
    }

    #[test]
    fn a_native_through_a_local_keeps_its_arguments_in_order() {
        let (result, calls) = run(r#"
f = int
g = range
result = [f("41") + 1, g(1, 4), g(4, 6), len(str(int("12")) + "ab")]
"#);
        assert_eq!(result.to_string(), "[42, [1, 2, 3], [4, 5], 4]");
        assert!(calls >= 6, "only {calls} natives took the slice path");
    }

    #[test]
    fn a_failing_native_leaves_the_stack_balanced() {
        // The native errors (and the arity check fails) with the callee and
        // its arguments on the stack: they must be gone before the error
        // propagates, or the values around the call would read wrong slots.
        let (result, _) = run(r#"
def pick(v)
  v
end
a = 1
out = []
[1, 2, 3].each do |x|
  try
    out.push(int(pick(x == 2 ? "zz" : str(x))))
  catch e
    out.push(-1)
  end
end
try
  len(1, 2)
catch e
  out.push(-2)
end
b = 2
result = [a, out, b, len("xyz")]
"#);
        assert_eq!(result.to_string(), "[1, [1, -1, 3, -2], 2, 3]");
    }

    #[test]
    fn intercepted_natives_are_tagged_once_and_still_intercepted() {
        for (name, tag) in [
            ("grouped", NativeSpecial::Grouped),
            ("with_transaction", NativeSpecial::WithTransaction),
            ("after_commit", NativeSpecial::AfterCommit),
            ("with_tenant", NativeSpecial::WithTenant),
            ("render_json", NativeSpecial::RenderJson),
            ("assert_raises", NativeSpecial::AssertRaises),
            ("defined", NativeSpecial::Defined),
            ("const_get", NativeSpecial::ConstGet),
            ("int", NativeSpecial::None),
            ("json_encode", NativeSpecial::None),
        ] {
            let native = NativeFunction::new(name, None, |_| Ok(Value::Null));
            assert_eq!(native.vm_special, tag, "{name}");
        }

        // `defined` answers from the VM's globals, which the native itself
        // cannot see: it must not have gone down the plain path.
        let (result, calls) = run(r#"
class Widget
end
result = [defined("Widget"), const_get("Widget") == Widget]
"#);
        assert_eq!(result.to_string(), "[true, true]");
        assert_eq!(calls, 0, "an intercepted native took the plain path");
    }
}

#[cfg(test)]
mod bind_method_arguments_tests {
    //! C583: a labelled method call binds its block to the `&block` parameter
    //! (as the tree-walker's `call_value_with_named` does), or drops it.
    use super::{bind_method_arguments, FunctionProto};
    use crate::interpreter::value::Value;
    use crate::span::Span;

    /// `def m(a, b = 2, &blk)`.
    fn proto_with_block() -> FunctionProto {
        let mut proto = FunctionProto::new("m".to_string());
        proto.param_names = vec!["a".into(), "b".into(), "blk".into()];
        proto.arity = 2;
        proto.defaults = 1;
        proto.defaults_mask = 0b010;
        proto.block_param = Some(2);
        proto
    }

    #[test]
    fn a_labelled_call_binds_its_block_to_the_block_parameter() {
        let proto = proto_with_block();
        let (slots, supplied) = bind_method_arguments(
            &proto,
            vec![],
            vec![("a".into(), Value::Int(4))],
            Some(Value::Int(99)),
            Span::default(),
        )
        .expect("binds");
        assert_eq!(slots, vec![Value::Int(4), Value::Null, Value::Int(99)]);
        // `b` is left to its default; the block slot is never defaulted.
        assert_eq!(supplied, 0b101);

        let (slots, _) = bind_method_arguments(
            &proto,
            vec![Value::Int(1)],
            vec![("b".into(), Value::Int(3))],
            None,
            Span::default(),
        )
        .expect("binds");
        assert_eq!(slots, vec![Value::Int(1), Value::Int(3), Value::Null]);
    }

    #[test]
    fn the_block_parameter_takes_no_argument_and_no_label() {
        let proto = proto_with_block();
        // A third positional argument has no regular parameter to go to.
        assert!(bind_method_arguments(
            &proto,
            vec![Value::Int(1), Value::Int(2), Value::Int(3)],
            vec![],
            None,
            Span::default(),
        )
        .is_err());
        // Nor may a label name the block parameter.
        assert!(bind_method_arguments(
            &proto,
            vec![Value::Int(1)],
            vec![("blk".into(), Value::Int(3))],
            None,
            Span::default(),
        )
        .is_err());
        // `a` is required.
        assert!(bind_method_arguments(
            &proto,
            vec![],
            vec![("b".into(), Value::Int(3))],
            None,
            Span::default(),
        )
        .is_err());
    }

    #[test]
    fn without_a_block_parameter_the_block_is_dropped() {
        let mut proto = FunctionProto::new("n".to_string());
        proto.param_names = vec!["a".into(), "f".into()];
        proto.arity = 1;
        proto.defaults = 1;
        proto.defaults_mask = 0b10;
        let (slots, supplied) = bind_method_arguments(
            &proto,
            vec![],
            vec![("a".into(), Value::Int(1))],
            Some(Value::Int(7)),
            Span::default(),
        )
        .expect("binds");
        assert_eq!(slots, vec![Value::Int(1), Value::Null]);
        assert_eq!(supplied, 0b01);
    }
}
