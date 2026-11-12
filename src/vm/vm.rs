//! The bytecode virtual machine — stack-based execution engine.

use ahash::AHashMap as HashMap;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use crate::error::RuntimeError;
use crate::interpreter::executor::MAX_CALL_DEPTH;
use crate::interpreter::value::{
    build_interpolated, hash_get_value, hash_set_value, same_string, Class, HashKey, HashPairs,
    StrKey, Value,
};
use crate::metrics::{metrics_enabled, Metrics};
use crate::span::Span;

use super::chunk::{Constant, FunctionProto};
use super::opcode::Op;
use super::upvalue::{Upvalue, VmClosure};

/// A call frame on the VM call stack.
#[derive(Clone)]
pub struct CallFrame {
    /// The closure being executed.
    pub closure: Rc<VmClosure>,
    /// Instruction pointer (index into chunk.code).
    pub ip: usize,
    /// Base index into the value stack for this frame's locals.
    pub stack_base: usize,
    /// Depth of `iter_stack` when this frame started. `Op::Return` truncates
    /// back to it, because a `return` from inside a `for` loop skips the
    /// `ForIter` that would have popped the iterator — leaking it to whatever
    /// loop is running in the caller, which then iterates the callee's
    /// leftovers. Same idea as `stack_base`, for the other stack.
    pub iter_base: usize,
    /// The class that *defines* the method this frame is executing, when
    /// known (set by compiled-method dispatch). `super` resolves against
    /// this class's superclass — not the instance's class, which would
    /// loop on multi-level hierarchies.
    pub class: Option<Rc<crate::interpreter::value::Class>>,
    /// Cached raw pointer to `closure.proto.chunk.code` — the dispatch
    /// loop's op fetch runs once per instruction, and going through
    /// `closure → proto → chunk → code[ip]` cost three pointer hops plus a
    /// bounds check every time. SAFETY: the frame's own `closure` Rc keeps
    /// the code vec alive for the frame's whole lifetime, and chunks are
    /// never mutated after compilation.
    code: *const Op,
    /// Length of the cached code slice (`ip >= code_len` ends the frame).
    code_len: usize,
    /// Bitmask of parameters the caller actually supplied: bit `i` is set when
    /// parameter `i` received a value. `Op::JumpIfParamSupplied` reads it to
    /// decide whether to run a parameter's default-value bytecode.
    ///
    /// A positional call of `argc` arguments sets the low `argc` bits; a
    /// named-argument call sets exactly the bits it filled, which is why this
    /// is a mask rather than a count. Parameters beyond bit 63 are treated as
    /// supplied (the mask saturates), so a function with more than 64
    /// parameters silently loses defaults past that point rather than
    /// misbinding — a limit no realistic signature reaches.
    supplied: u64,
    /// What the frame's `Return` does with its value: hand it to the
    /// caller, to an in-VM iterator loop, or drop it.
    pub on_return: FrameReturn,
}

/// What a frame's `Return` does with the value it returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameReturn {
    /// Pushes it for the caller: every ordinary call.
    Value,
    /// The callback of an in-VM iterator loop: the result goes to
    /// `Vm::callback_returned` and the frame runs again for the next element.
    DrivesLoop,
    /// Drops it: a frame run for its effect on the stack beneath it, which
    /// the frame leaves as it found it — the field defaults of a new
    /// instance (`Vm::push_field_defaults`), run before its constructor
    /// starts, or once the instance is the call's value.
    Discard,
}

/// Whether a positional call of `argc` arguments binds `proto` without an
/// arity error: every required parameter given, none too many. The omitted
/// tail is left to the callee's default-value prologue.
#[inline(always)]
fn accepts_positional(proto: &FunctionProto, argc: usize) -> bool {
    proto.arity as usize <= argc && argc <= proto.param_names.len()
}

/// Build the supplied-parameter mask for a plain positional call of `argc`
/// arguments: the low `argc` bits set, saturating at 64 parameters.
#[inline]
pub(crate) fn positional_supplied_mask(argc: usize) -> u64 {
    if argc >= 64 {
        u64::MAX
    } else {
        (1u64 << argc) - 1
    }
}

impl CallFrame {
    /// Build a frame for `closure` starting at ip 0, caching the raw code
    /// pointer for the dispatch loop's per-op fetch.
    #[inline]
    pub fn new(
        closure: Rc<VmClosure>,
        stack_base: usize,
        iter_base: usize,
        class: Option<Rc<crate::interpreter::value::Class>>,
        supplied: u64,
    ) -> Self {
        let code = closure.proto.chunk.code.as_ptr();
        let code_len = closure.proto.chunk.code.len();
        Self {
            closure,
            ip: 0,
            stack_base,
            iter_base,
            class,
            code,
            code_len,
            supplied,
            on_return: FrameReturn::Value,
        }
    }
}

/// An exception handler pushed by TryBegin.
#[derive(Debug, Clone)]
pub struct ExceptionHandler {
    /// Absolute IP to jump to for catch.
    pub catch_ip: usize,
    /// Absolute IP to jump to for finally.
    pub finally_ip: usize,
    /// Stack depth when TryBegin was executed (to unwind the stack).
    pub stack_depth: usize,
    /// `iter_stack` depth when TryBegin was executed. A `throw` from inside a
    /// `for` loop skips the `ForIter` that pops the iterator, exactly as an
    /// early `return` does, so unwinding must truncate here too — otherwise the
    /// enclosing loop resumes on the abandoned iterator.
    pub iter_depth: usize,
    /// Call frame depth when TryBegin was executed.
    pub frame_depth: usize,
}

/// Iterator state for for-in loops.
#[derive(Debug, Clone)]
pub enum IterState {
    Array {
        values: Rc<RefCell<Vec<Value>>>,
        index: usize,
    },
    Hash {
        values: Rc<RefCell<HashPairs>>,
        index: usize,
    },
    Range {
        current: i64,
        end: i64,
    },
    String {
        s: crate::interpreter::value::SoliStr,
        byte_offset: usize,
    },
    /// An iterator method (`each`, `map`, …) whose callback runs in the
    /// dispatch loop — see `vm_callback_loop.rs`.
    Callback(Box<super::vm_callback_loop::CallbackLoop>),
}

/// Slots in `Vm::closure_cache` and `Vm::closure_spares`.
pub(crate) const CLOSURE_CACHE_SLOTS: usize = 64;

/// The bytecode VM.
pub struct Vm {
    /// Value stack.
    pub stack: Vec<Value>,
    /// Call frame stack.
    pub frames: Vec<CallFrame>,
    /// Global variables.
    pub globals: HashMap<String, Value>,
    /// Open upvalues — cells pointing at stack slots that are still live —
    /// each with the slot it points at, sorted by slot. A returning frame's
    /// cells are then the tail (see `vm_upvalues.rs`).
    pub open_upvalues: Vec<(usize, Rc<RefCell<Upvalue>>)>,
    /// One past the highest stack slot an open upvalue points at (0 when
    /// none is open), so a return can tell in one comparison whether its
    /// frame has anything to close. Kept by `capture_upvalue`,
    /// `close_upvalues` and `reset` — the only writers of `open_upvalues`.
    pub(crate) open_upvalue_top: usize,
    /// The last finished iterator-method loop's state, kept for the next
    /// one so starting a loop does not allocate (see `vm_callback_loop.rs`).
    pub(crate) spare_callback_loop: Option<Box<super::vm_callback_loop::CallbackLoop>>,
    /// Closures that capture nothing, one per `fn` expression, shared by
    /// every evaluation of it — see `Op::Closure`. Direct-mapped by the
    /// prototype's address; each entry keeps its prototype alive.
    pub(crate) closure_cache: Box<[Option<Rc<VmClosure>>; CLOSURE_CACHE_SLOTS]>,
    /// What each `CallMethod` / `CallMethodById` site found on each receiver
    /// class it saw, for the fast tier's method calls (`Vm::fast_method`).
    method_cache: super::method_cache::MethodCache,
    /// Closures that capture, each kept by the iterator loop that held the
    /// last reference to it, with its upvalues emptied: the next evaluation
    /// of the same `fn` expression refills it instead of allocating.
    /// Mapped like `closure_cache`; each holds only its prototype.
    pub(crate) closure_spares: Box<[Option<Rc<VmClosure>>; CLOSURE_CACHE_SLOTS]>,
    /// Upvalue cells nothing referenced any more when their variable went
    /// out of scope, kept for `capture_upvalue` to reuse. They hold no
    /// value. Bounded.
    pub(crate) upvalue_pool: Vec<Rc<RefCell<Upvalue>>>,
    /// Emptied arrays nothing else references, handed back by an iterator
    /// loop (a `map` callback's `[key, value]` pair once its entry is taken
    /// out) for `Op::Array` to fill instead of allocating. Bounded.
    pub(crate) array_pool: Vec<Rc<RefCell<Vec<Value>>>>,
    /// Exception handler stack.
    pub exception_handlers: Vec<ExceptionHandler>,
    /// Iterator state stack (for for-in loops).
    pub iter_stack: Vec<IterState>,
    /// The page a compiled template writes into (`Op::TemplateWrite`).
    pub template_out: String,
    /// Pages set aside by `Op::TemplateCaptureStart`, innermost last.
    pub template_captures: Vec<String>,
    /// The `content_for` store as it was before this render first appended to
    /// it (`Op::TemplateContentFor`): what a failed render puts back. Taken
    /// only then, so a render with no `content_for` copies nothing.
    pub template_content_before: Option<Option<std::collections::HashMap<String, String>>>,
    /// Run a tree-walker function called from this VM on the tree-walker
    /// instead of compiling it here. Set on the VM views render on: a view
    /// helper resolves names such as the current `req` through its own
    /// closure, which this VM's globals (copied once) would not follow.
    /// A helper whose body names nothing outside its parameters still
    /// compiles — it needs nothing from that closure.
    pub tree_walk_functions: bool,
    /// The environment those functions are called from: the template
    /// interpreter's, for the render in progress. A helper that reads a name
    /// its closure lacks finds it there, as when the tree-walker renders.
    pub tree_walk_env: Option<Rc<RefCell<crate::interpreter::environment::Environment>>>,
    /// Handlers moved off the VM on this worker — skip VM for these and use
    /// interpreter directly. The VM refused them, failed where the
    /// tree-walker's re-run then succeeded, or failed after a durable write
    /// (`serve::vm_failure_verdict`); an error both engines raise is not one.
    pub failed_handlers: ahash::AHashSet<String>,
    /// Frame depth at which `run()` should stop. Bumped by native methods that
    /// need to synchronously invoke a user closure (e.g. array.map); `Op::Return`
    /// treats frames shrinking back to this depth as the exit condition.
    pub return_depth: usize,
    /// Check each function's declared return type when it returns, raising
    /// what the tree-walker raises. On for scripts; off under `soli serve`,
    /// where applications have always run without the check.
    pub check_return_types: bool,
    /// `SOLI_METRICS` is on: time this VM's outermost `run` for
    /// `soli_vm_execution_seconds`. Read once, when the VM is made.
    pub(crate) time_runs: bool,
    /// An outermost `run` is being timed. The `run`s it re-enters — a block
    /// called by `reduce` or `each_with_index`, a paren-less method call, a
    /// constructor — see it set and are not timed again: their time is
    /// already inside the outer one.
    pub(crate) timed_run_open: bool,
    /// Frames reported to `crate::debug` (flamegraph `fn` spans, profiler
    /// stacks) and not yet closed: frame depth, the frame's closure, and the
    /// mark `debug::enter` returned. Empty unless a listener is on.
    pub(crate) debug_open: Vec<(usize, usize, usize)>,
}

/// Times one outermost `Vm::run` while `SOLI_METRICS` is on, and records it
/// when dropped — on an early return and on a panic unwinding through the
/// run too, so `timed_run_open` can never stay set and switch the timing off
/// for the rest of the VM's life.
struct TimedRun<'a> {
    vm: &'a mut Vm,
    start: web_time::Instant,
}

impl<'a> TimedRun<'a> {
    #[inline]
    fn open(vm: &'a mut Vm) -> Self {
        vm.timed_run_open = true;
        Self {
            vm,
            start: web_time::Instant::now(),
        }
    }
}

impl Drop for TimedRun<'_> {
    #[inline]
    fn drop(&mut self) {
        self.vm.timed_run_open = false;
        Metrics::global().record_vm_execution(self.start.elapsed());
    }
}

/// `stack[ia] = stack[ia] + stack[ib]` when both are strings.
///
/// Appends into the left slot. `EcoString::push_str` grows in place when that
/// slot is the only owner, and detaches first when another value still shares
/// the buffer — so `b = a; a = a + "x"` leaves `b` unchanged.
fn append_strings_in_place(stack: &mut [Value], ia: usize, ib: usize) {
    if ia == ib {
        let extra = match &stack[ia] {
            Value::String(text) => text.clone(),
            _ => return,
        };
        if let Value::String(text) = &mut stack[ia] {
            text.push_str(&extra);
        }
        return;
    }
    let extra = match &stack[ib] {
        Value::String(text) => text.clone(),
        _ => return,
    };
    if let Value::String(text) = &mut stack[ia] {
        text.push_str(&extra);
    }
}

/// `before << rhs`, writing into `slot`, the variable `before` was read from
/// ahead of the right-hand side. Returns what `<<` yields: the left operand.
///
/// An array is pushed into (it is the one read first, even if the right-hand
/// side rebound the variable). A string appends in place when the variable
/// still holds it — `before` is dropped first, so the variable is the buffer's
/// only owner and grows without a copy; otherwise the variable gets
/// `before + rhs`, as the tree-walker's `name = name + rhs` gives it.
fn shovel_into(slot: &mut Value, before: Value, rhs: Value) -> Result<Value, String> {
    match before {
        Value::Array(arr) => {
            arr.borrow_mut().push(rhs);
            Ok(Value::Array(arr))
        }
        Value::String(old) => {
            let extra = match rhs {
                Value::String(extra) => extra,
                other => {
                    return Err(format!(
                        "string << expects a string, got {}",
                        other.type_name()
                    ))
                }
            };
            let unchanged = matches!(&*slot, Value::String(current) if same_string(current, &old));
            if unchanged {
                drop(old);
                if let Value::String(text) = slot {
                    text.push_str(&extra);
                }
            } else {
                let mut joined = old;
                joined.push_str(&extra);
                *slot = Value::String(joined);
            }
            Ok(slot.clone())
        }
        other => Err(format!(
            "<< expects an array or a string on the left, got {}",
            other.type_name()
        )),
    }
}

/// `before + rhs` for two strings, stored into `slot`, the variable `before`
/// was read from ahead of the right-hand side. Appends in place when the
/// variable still holds `before` (see [`shovel_into`]). Any other pair is
/// handed back for an ordinary add.
fn add_strings_into(slot: &mut Value, before: Value, rhs: Value) -> Result<Value, (Value, Value)> {
    match (before, rhs) {
        (Value::String(old), Value::String(extra)) => {
            let unchanged = matches!(&*slot, Value::String(current) if same_string(current, &old));
            if unchanged {
                drop(old);
                if let Value::String(text) = slot {
                    text.push_str(&extra);
                }
            } else {
                let mut joined = old;
                joined.push_str(&extra);
                *slot = Value::String(joined);
            }
            Ok(slot.clone())
        }
        pair => Err(pair),
    }
}

impl Vm {
    /// `name` read unqualified in code written in a namespace: the class it
    /// names there (`Invoice` in `Billing::PayInvoice` is `Billing::Invoice`).
    fn namespaced(&self, name: &str) -> Option<Value> {
        let namespace = self.frames.last()?.closure.proto.namespace.clone()?;
        crate::interpreter::domain::resolve_in_namespace(&namespace, name, |global| {
            self.globals.get(global).cloned()
        })
    }

    /// A global, or else `name` in the current code's namespace.
    fn global_or_namespaced(&self, name: &str) -> Option<Value> {
        self.globals
            .get(name)
            .cloned()
            .or_else(|| self.namespaced(name))
    }

    /// Materialise a `grouped {}` deferred result read out of an instance
    /// field.
    ///
    /// The `GetProperty` / `GetLocalProperty` field fast paths push the stored
    /// value straight onto the stack, so they never reach `op_get_property`'s
    /// force. That left the placeholder in play whenever the value went into a
    /// container or a native that matches `Value::Array` directly — including
    /// `render("posts/index", { "posts": @posts })`, the documented shape.
    /// The span is computed only on the rare deferred branch.
    #[inline]
    fn force_field_hit(&mut self, val: Value) -> Result<Value, RuntimeError> {
        if let Value::Deferred(cell) = &val {
            let span = self.current_span();
            return crate::interpreter::builtins::model::batch::force(cell)
                .map_err(|e| RuntimeError::General { message: e, span });
        }
        Ok(val)
    }

    /// `receiver.name` for the fast tier's `GetProperty` and
    /// `GetLocalProperty`: the value the general arms' instance-field hit
    /// pushes, when that hit completes with no error and no side effect.
    /// The receiver is an instance holding the field (a field shadows a
    /// method, so a present key decides the read), the value is no
    /// `grouped {}` placeholder still to force, the field is no model's
    /// relation or translated field (`field_hit_is_raw`), and the privacy
    /// check lets the read through. `None` hands the op to the general arm, which
    /// raises a refused read after checking it again.
    ///
    /// `slot_zero`: the receiver is the running frame's slot 0. In a method
    /// frame that is `this`, whose members the check never refuses, so `@x`
    /// skips the call, as in the general `GetLocalProperty` arm.
    #[inline]
    fn fast_field_read(&self, receiver: &Value, name: &str, slot_zero: bool) -> Option<Value> {
        let Value::Instance(inst) = receiver else {
            return None;
        };
        let value = {
            let instance = inst.borrow();
            match instance.fields.get(name) {
                None | Some(Value::Deferred(_)) => return None,
                // A model's relation or translated field is resolved by the
                // general arm (`model_instance_member`), not read raw.
                Some(_) if !Self::field_hit_is_raw(&instance, name) => return None,
                Some(value) => clone_scalar_fast(value),
            }
        };
        if crate::interpreter::value::restricted_methods_declared()
            && !(slot_zero && self.slot_zero_is_this())
            && self.check_private_value(receiver, name).is_err()
        {
            return None;
        }
        Some(value)
    }

    /// `Klass.NAME` for the fast tier's `GetProperty` and `GetLocalProperty`:
    /// a static field (a class constant, an enum's unit variant) whose read in
    /// the general arm ends at `op_get_property`'s `val.clone()` with nothing
    /// done after it. That is a field holding plain data — a scalar, a
    /// string, a collection, an instance, a DateTime. Everything else keeps
    /// the general arm: a nested class (its `check_nested_access`), a
    /// callable (`op_get_property_member` auto-invokes a zero-arity one), a
    /// `grouped {}` placeholder or a future, and a model class's scope of the
    /// same name, which outranks the field. The privacy check is no concern:
    /// it only ever refuses an instance receiver.
    #[inline]
    fn fast_static_read(class: &crate::interpreter::value::Class, name: &str) -> Option<Value> {
        let value = match class.static_fields.borrow().get(name)? {
            value @ (Value::Int(_)
            | Value::Float(_)
            | Value::Decimal(_)
            | Value::String(_)
            | Value::Symbol(_)
            | Value::Bool(_)
            | Value::Null
            | Value::Array(_)
            | Value::Hash(_)
            | Value::Instance(_)
            | Value::DateTime(_, _)) => clone_scalar_fast(value),
            _ => return None,
        };
        if Self::is_model_class_scope(class, name) {
            return None;
        }
        Some(value)
    }

    /// `receiver.name = value` for the fast tier's `SetProperty`: the general
    /// arm's instance-field write — an existing field overwritten in place, a
    /// new one inserted — when it completes with no error. The receiver is an
    /// instance of a class with no checked writes (no model, no `const`
    /// field: `has_checked_writes`), the privacy check lets the write
    /// through and the instance is no constructed value object. A hash's `h.key = v` always sets the
    /// key (no method precedence, no privacy, nothing frozen), so it is done
    /// here too, keyed by the pool's own `SoliStr`. `false` touches nothing
    /// and hands the op to the general arm, which raises the refusal.
    #[inline]
    fn fast_field_write(
        &self,
        receiver: &Value,
        name: &crate::interpreter::value::SoliStr,
        value: &Value,
    ) -> bool {
        let inst = match receiver {
            Value::Instance(inst) => inst,
            Value::Hash(hash) => {
                hash.borrow_mut().set_str(name, clone_scalar_fast(value));
                return true;
            }
            _ => return false,
        };
        // A model (read-only metadata, translated fields) or a class with
        // `const` fields: the general arm applies `set_instance_field`.
        if Self::has_checked_writes(&inst.borrow().class) {
            return false;
        }
        if crate::interpreter::value::restricted_methods_declared()
            && self.check_private_value(receiver, name).is_err()
        {
            return false;
        }
        if crate::interpreter::domain::value_objects_declared()
            && crate::interpreter::domain::is_frozen(inst, &inst.borrow().class)
        {
            return false;
        }
        let value = clone_scalar_fast(value);
        let mut instance = inst.borrow_mut();
        match instance.fields.get_mut(name.as_str()) {
            Some(field) => *field = value,
            None => {
                instance.fields.insert(name.clone(), value);
            }
        }
        true
    }

    /// The compiled method `receiver.name(args…)` runs, for the fast tier's
    /// `CallMethod` and `CallMethodById` at `site` with `argc` arguments above
    /// the receiver: the frame the general arms' compiled-method call
    /// (`call_method_slow_path` → `push_method_frame`) opens, when it opens
    /// one with nothing to pad or raise. The receiver is an instance whose
    /// class chain has `name` compiled, the method takes exactly `argc`
    /// parameters, the depth limit is not reached and the privacy check lets
    /// the call through. `None` hands the op to the general arm, which raises
    /// a refused call after checking it again.
    ///
    /// For an instance, the general path tries nothing before
    /// `resolve_instance_method` that could answer differently: `send` is
    /// re-dispatched only on a class with no `send` of its own, where the
    /// lookup misses too. So a hit — the nearest definition, compiled —
    /// decides the call, before fields, natives and `method_missing`; a
    /// nearest definition still AST is a miss, which the general arm
    /// compiles. A change there must be mirrored here. The lookup goes
    /// through `method_cache`; a miss is remembered too, so a native
    /// method's site hands off after one probe.
    #[inline]
    fn fast_method(
        &mut self,
        site: *const Op,
        argc: usize,
        name: &str,
    ) -> Option<super::method_cache::FoundMethod> {
        if self.frames.len() >= MAX_CALL_DEPTH {
            return None;
        }
        let receiver = &self.stack[self.stack.len() - 1 - argc];
        let Value::Instance(inst) = receiver else {
            return None;
        };
        let epoch = crate::interpreter::value::vm_methods_epoch();
        let frames = &self.frames;
        let found = {
            let instance = inst.borrow();
            self.method_cache
                .lookup(site, &instance.class, name, epoch, || {
                    frames.last().unwrap().closure.proto.clone()
                })
        };
        let (method, defining) = found?;
        if method.proto.param_names.len() != argc {
            return None;
        }
        if crate::interpreter::value::restricted_methods_declared()
            && self.check_private_value(receiver, name).is_err()
        {
            return None;
        }
        Some((method, defining))
    }

    /// Whether field `name`, present on `inst`, is read as its stored value.
    /// It is not on a model whose declared relation or translated field it
    /// is: `model_instance_member` turns a preloaded hash into an instance
    /// (whose serializer drops `password_digest` and the other sensitive
    /// fields) and reads a translation in the current locale. A plain object
    /// pays one memoized flag read.
    #[inline]
    fn field_hit_is_raw(inst: &crate::interpreter::value::Instance, name: &str) -> bool {
        !inst.class.is_model_subclass()
            || !crate::interpreter::builtins::model::member_needs_model_resolution(
                &inst.class.name,
                name,
            )
    }

    /// Whether a field write on an instance of `class` needs
    /// `set_instance_field`'s rules: a model (read-only `_key`/`_id`
    /// metadata, translated fields staged per locale) or a class with `const`
    /// fields. A plain object pays one memoized flag read and a length check.
    #[inline]
    fn has_checked_writes(class: &Class) -> bool {
        class.is_model_subclass() || !class.const_fields.is_empty()
    }

    pub fn new() -> Self {
        Self {
            stack: Vec::with_capacity(256),
            frames: Vec::with_capacity(64),
            globals: HashMap::new(),
            open_upvalues: Vec::new(),
            open_upvalue_top: 0,
            spare_callback_loop: None,
            closure_cache: Box::new([const { None }; CLOSURE_CACHE_SLOTS]),
            method_cache: Default::default(),
            closure_spares: Box::new([const { None }; CLOSURE_CACHE_SLOTS]),
            upvalue_pool: Vec::new(),
            array_pool: Vec::new(),
            exception_handlers: Vec::new(),
            iter_stack: Vec::new(),
            template_out: String::new(),
            template_captures: Vec::new(),
            template_content_before: None,
            tree_walk_functions: false,
            tree_walk_env: None,
            failed_handlers: ahash::AHashSet::new(),
            return_depth: 0,
            check_return_types: false,
            time_runs: metrics_enabled(),
            timed_run_open: false,
            debug_open: Vec::new(),
        }
    }

    /// Execute a compiled module (top-level script).
    pub fn execute(&mut self, proto: &Arc<FunctionProto>) -> Result<Value, RuntimeError> {
        let closure = Rc::new(VmClosure::new(proto.clone(), Vec::new()));
        self.push(Value::VmClosure(closure.clone()));

        let base = self.frames.len();
        self.frames
            .push(CallFrame::new(closure, 0, self.iter_stack.len(), None, 0));

        let result = self.run();
        // An uncaught error leaves its frames on the VM (until `reset`, or
        // never when the VM is then dropped, as a script's is before the
        // tree-walker fallback runs): close the ones the debug hooks saw, so
        // the profiler and flamegraph do not nest later work under them.
        if result.is_err() && !self.debug_open.is_empty() {
            self.debug_close_above(base);
        }
        result
    }

    /// An undefined-variable error for a global, with a "did you mean"
    /// suggestion drawn from the running function's locals, then the
    /// variables it captures, then the globals — the order the tree-walker's
    /// scopes give (bytecode slots have no names; the compiler keeps them in
    /// `FunctionProto::debug_names` for this).
    #[cold]
    pub(crate) fn undefined_global(&self, name: impl Into<String>) -> RuntimeError {
        let name = name.into();
        let globals: Vec<&str> = self.globals.keys().map(|k| &**k).collect();
        let suggestion = match self.frames.last() {
            Some(frame) => {
                let names = &frame.closure.proto.debug_names;
                crate::error::suggest::best_match_scoped(
                    &name,
                    [
                        names.locals.iter().map(|n| &**n).collect::<Vec<_>>(),
                        names.upvalues.iter().map(|n| &**n).collect(),
                        globals,
                    ],
                )
            }
            None => crate::error::suggest::best_match(&name, globals),
        };
        RuntimeError::undefined_variable_hinted(
            name,
            self.current_span(),
            crate::error::Hint::ready(suggestion),
        )
    }

    /// Get the current source span (only call on error paths).
    #[cold]
    pub(crate) fn current_span(&self) -> Span {
        let frame = self.frames.last().unwrap();
        let ip = frame.ip.saturating_sub(1);
        let line = frame
            .closure
            .proto
            .chunk
            .lines
            .get(ip)
            .copied()
            .unwrap_or(0);
        let column = frame
            .closure
            .proto
            .chunk
            .columns
            .get(ip)
            .copied()
            .unwrap_or(0);
        Span::new(0, 0, line, column as usize)
    }

    /// Whether the value about to be returned (top of the stack) is one the
    /// returning function's declared type admits — or it declares none. The
    /// fast tier asks before returning, and leaves the raising to the general
    /// arm.
    #[inline]
    fn return_value_admitted(&self) -> bool {
        let (Some(frame), Some(value)) = (self.frames.last(), self.stack.last()) else {
            return true;
        };
        match &frame.closure.proto.return_check {
            None => true,
            Some(check) => check.admits(value),
        }
    }

    /// Raise what the tree-walker raises when a function returns a value its
    /// declared return type does not admit.
    fn check_return_type(&self, result: &Value) -> Result<(), RuntimeError> {
        let Some(frame) = self.frames.last() else {
            return Ok(());
        };
        let proto = &frame.closure.proto;
        let (Some(check), Some(expected)) = (&proto.return_check, &proto.return_type) else {
            return Ok(());
        };
        if check.admits(result) {
            return Ok(());
        }
        Err(RuntimeError::General {
            message: format!(
                "function '{}' expected to return {}, got {}",
                proto.name,
                expected,
                result.type_name()
            ),
            span: proto.decl_span,
        })
    }

    /// `a + b` by [`Op::Add`]'s rules, for the fused add-assign ops.
    fn add_values(&self, a: Value, b: Value) -> Result<Value, RuntimeError> {
        match (&a, &b) {
            (Value::Int(x), Value::Int(y)) => match x.checked_add(*y) {
                Some(sum) => Ok(Value::Int(sum)),
                None => Err(crate::interpreter::executor::operators::int_overflow(
                    *x,
                    "+",
                    *y,
                    self.current_span(),
                )),
            },
            (Value::Float(x), Value::Float(y)) => Ok(Value::Float(x + y)),
            (Value::Int(x), Value::Float(y)) => Ok(Value::Float(*x as f64 + y)),
            (Value::Float(x), Value::Int(y)) => Ok(Value::Float(x + *y as f64)),
            _ => self.op_add(a, b, self.current_span()),
        }
    }

    /// `name << rhs` for a global; see [`shovel_into`]. The result is
    /// pushed, matching `<<` returning its left operand.
    fn append_named(&mut self, name: &str, before: Value, rhs: Value) -> Result<(), RuntimeError> {
        let Some(slot) = self.globals.get_mut(name) else {
            return Err(self.undefined_global(name.to_string()));
        };
        let result = shovel_into(slot, before, rhs)
            .map_err(|m| RuntimeError::type_error(m, self.current_span()))?;
        self.stack.push(result);
        Ok(())
    }

    /// Answer a borrowed-tier hash method (`slice`, `merge`, `dig`, …)
    /// straight off the operand stack: the receiver and arguments stay where
    /// they are, so neither the `Rc` nor the arguments are copied. `None`
    /// when the method needs the full dispatcher (callbacks, mutation).
    #[inline]
    fn hash_method_on_stack(
        &self,
        receiver_idx: usize,
        argc: usize,
        name: &str,
    ) -> Option<Result<Value, RuntimeError>> {
        let Value::Hash(hash) = &self.stack[receiver_idx] else {
            return None;
        };
        let args = &self.stack[receiver_idx + 1..receiver_idx + 1 + argc];
        crate::interpreter::executor::calls::hash_pure::hash_method_borrowed(
            &hash.borrow(),
            name,
            args,
            self.current_span(),
        )
    }

    /// A string constant, shared rather than copied: a short name (`upcase`,
    /// `title`) is stored inline, a long one is reference-counted. Property
    /// reads use it on every access; `read_string_constant_owned` allocated.
    #[inline]
    fn read_string_constant(&self, idx: u16) -> crate::interpreter::value::SoliStr {
        let frame = self.frames.last().unwrap();
        match &frame.closure.proto.chunk.constants[idx as usize] {
            Constant::String(s) => s.clone(),
            _ => crate::interpreter::value::SoliStr::default(),
        }
    }

    /// The `Constant::ArgNames` labels at `idx` (an `Arc` clone).
    fn read_arg_names(
        &self,
        idx: u16,
    ) -> std::sync::Arc<Vec<Option<crate::interpreter::value::SoliStr>>> {
        let frame = self.frames.last().unwrap();
        match &frame.closure.proto.chunk.constants[idx as usize] {
            Constant::ArgNames(names) => names.clone(),
            _ => unreachable!("a labelled call must reference an ArgNames constant"),
        }
    }

    /// A string constant as an owned `String`, for the ops that store it (a
    /// global-map key, a class or method name). An op that only compares the
    /// name or looks it up reads `read_string_constant` instead, which does not
    /// allocate. The copy is a plain `String::from(&str)`: `EcoString`'s
    /// `to_string()` went through `Display`, `Formatter::pad` and `write_str`.
    fn read_string_constant_owned(&self, idx: u16) -> String {
        let frame = self.frames.last().unwrap();
        match &frame.closure.proto.chunk.constants[idx as usize] {
            Constant::String(s) => String::from(s.as_str()),
            _ => String::new(),
        }
    }

    /// A string constant as the pool's own `SoliStr` (an empty one for any
    /// other constant): a hash write keyed by it appends the key as a
    /// refcount clone instead of copying its bytes. A raw pointer, like the
    /// arms' `*const str` keys, so the arm can go on using `self`; the
    /// constant lives as long as the frame's prototype.
    #[inline(always)]
    fn string_constant_key(&self, idx: u16) -> *const crate::interpreter::value::SoliStr {
        static EMPTY: crate::interpreter::value::SoliStr =
            crate::interpreter::value::SoliStr::new();
        match &self.frames.last().unwrap().closure.proto.chunk.constants[idx as usize] {
            Constant::String(s) => s as *const _,
            _ => &EMPTY as *const _,
        }
    }

    /// A string constant, borrowed: for a name that is only looked at, like
    /// the one a privacy check reads (`""` for any other constant).
    #[inline]
    fn string_constant(&self, idx: u16) -> &str {
        match &self.frames.last().unwrap().closure.proto.chunk.constants[idx as usize] {
            Constant::String(s) => s.as_str(),
            _ => "",
        }
    }

    /// Call the method `define_method` gave the receiver's primitive type
    /// (`Array.define_method("includes?", …)`), when it outranks the builtin
    /// for this call; returns false, touching nothing, when the builtin runs.
    ///
    /// The precedence is the tree-walker's, through the one predicate both
    /// engines share: a block-less call on a Hash, Array or String whose name
    /// is in that type's direct-dispatch list runs the builtin; every other
    /// call — with a block, on another primitive, or a name off the list —
    /// runs the user method. `CallMethod` used to consult user methods first
    /// for every name and `CallMethodById` never did, so which one ran
    /// depended on whether the name happened to have a method id.
    ///
    /// Only reached once a user method exists somewhere (the caller tests
    /// `user_method_flags()`).
    #[cold]
    fn call_primitive_user_method(
        &mut self,
        receiver_idx: usize,
        argc: usize,
        name: &str,
        block: bool,
    ) -> Result<bool, RuntimeError> {
        use crate::interpreter::executor::calls::user_methods::{lookup_user_method, PrimType};
        let receiver = &self.stack[receiver_idx];
        let prim = match receiver {
            Value::Int(_) => PrimType::Int,
            Value::Float(_) => PrimType::Float,
            Value::Bool(_) => PrimType::Bool,
            Value::Null => PrimType::Null,
            Value::Decimal(_) => PrimType::Decimal,
            Value::String(_) => PrimType::String,
            Value::Array(_) => PrimType::Array,
            Value::Hash(_) => PrimType::Hash,
            Value::Symbol(_) => PrimType::Symbol,
            _ => return Ok(false),
        };
        if !block
            && crate::interpreter::executor::calls::function::builtin_outranks_user_method(
                receiver, name,
            )
        {
            return Ok(false);
        }
        let Some(method) = lookup_user_method(prim, name) else {
            return Ok(false);
        };
        let span = self.current_span();
        let object = self.stack[receiver_idx].clone();
        self.stack[receiver_idx] =
            crate::interpreter::executor::access::member::bind_user_method_to_receiver(
                object, method,
            );
        self.call_value(argc, span)?;
        Ok(true)
    }

    /// Run the dispatch loop.
    /// Execute until the program completes or an error escapes.
    ///
    /// RuntimeErrors raised by ops (native methods, type errors, missing
    /// properties) are routed through the innermost active `try`/`rescue`
    /// handler — matching the tree-walker, where `catch (e)` binds the
    /// error text. Without an active handler the error propagates (which
    /// is what serve-mode's interpreter fallback relies on).
    ///
    /// With `SOLI_METRICS` on, only the outermost activation is timed: one
    /// clock pair and one pair of atomic adds per handler, view or script,
    /// however many times a block or a paren-less call re-enters `run`.
    #[inline]
    pub fn run(&mut self) -> Result<Value, RuntimeError> {
        if !self.time_runs || self.timed_run_open {
            return self.run_catching();
        }
        let timed = TimedRun::open(self);
        timed.vm.run_catching()
    }

    /// `run`'s body: dispatch, and resume at the innermost handler when an
    /// error is catchable. A resumed `catch` stays in the same activation.
    fn run_catching(&mut self) -> Result<Value, RuntimeError> {
        loop {
            let err = match self.run_dispatch() {
                Ok(value) => return Ok(value),
                Err(err) => err,
            };
            // Same gating as throw_exception: handlers at or below
            // return_depth belong to an outer native invocation (e.g.
            // array.map's callback driver) that must see a Rust Err.
            let catchable = self
                .exception_handlers
                .last()
                .is_some_and(|handler| handler.frame_depth > self.return_depth);
            if !catchable || err.is_engine_fallback() {
                // The frame that raised it is still on top: note its file for
                // the code frame `soli run` draws (VM spans carry no file).
                if let Some(frame) = self.frames.last() {
                    crate::error::source_map::note_vm_origin(
                        err.span(),
                        frame.closure.proto.source_path.as_ref(),
                    );
                }
                return Err(err);
            }
            let span = err.span();
            // A user `throw` that came back out of a native driver still holds
            // its value — re-throw the value itself, so a hash thrown inside
            // `array.map` is still a hash at the caller's `catch`. Everything
            // else is a Rust-side failure with no value to recover, and binds
            // its message; `catchable_message` is the same helper the
            // tree-walker uses, so the text cannot drift between engines.
            match err {
                RuntimeError::Thrown { value, .. } => self.throw_exception(value, span)?,
                other => {
                    self.throw_exception(Value::String(other.catchable_message().into()), span)?
                }
            }
        }
    }

    /// The dispatch loop's fast tier: the stack, local, constant, jump,
    /// integer/float arithmetic and comparison ops, run with the current
    /// frame's instruction pointer, code, constants and stack base held in
    /// locals.
    ///
    /// `run_dispatch` re-read all of that through `self.frames` on every op
    /// — `&mut self` means the compiler cannot keep it in registers across
    /// an op — and the reloads were a large share of what a simple op cost.
    /// None of these ops can push or pop a frame, so the cached state stays
    /// valid until this returns.
    ///
    /// An arm either completes its op or hands it back untouched: every
    /// type and overflow check happens by peeking, before anything is
    /// popped, and whatever it declines (a string `+`, an overflowing add,
    /// a constant that needs allocating) is returned for `run_dispatch`'s
    /// general arm to execute — so the two tiers cannot disagree on a
    /// result or an error. `frame.ip` is written back before returning;
    /// `Ok(None)` means the frame's code ran out.
    ///
    /// The operand stack's top is read and popped unchecked (`pop`, `pop2`,
    /// `peek`, `peek_mut`, `peek_two!`), as the general tier already does: an
    /// op is only emitted after the pushes it consumes, `stack_effect` states
    /// that per op and a test measures it against every handler, and the
    /// frame's callee sits at `stack_base` beneath its operands, so a checked
    /// `pop().unwrap()` could only ever catch a stack already corrupt below
    /// the frame. Local slots (`self.stack[base + slot]`) stay bounds-checked,
    /// and so do `ip < code_len` and `Vec::push`: dropping those needs a
    /// bytecode verifier (slot and jump ranges, a per-proto max depth), and
    /// without one a compiler bug would turn from a panic into memory
    /// corruption.
    #[inline(always)]
    fn run_fast(&mut self) -> Result<Option<Op>, RuntimeError> {
        /// Load the top frame's state into the cached locals.
        macro_rules! load_frame {
            () => {{
                let frame = self.frames.last().unwrap();
                (
                    frame.code,
                    frame.code_len,
                    frame.closure.proto.chunk.constants.as_ptr(),
                    frame.stack_base,
                    frame.ip,
                )
            }};
        }
        let (mut code, mut code_len, mut constants, mut base, mut ip) = load_frame!();

        /// Enter a compiled closure whose callee and `argc` arguments are on
        /// top of the stack, `arity <= argc <= params` — the range in which
        /// the general `Call` arm raises nothing — and switch the cached state
        /// to it. As there, each omitted parameter gets a null slot that the
        /// callee's default-value prologue fills, and the supplied mask
        /// covers only the `argc` given. A method call passes the class
        /// defining the method, the frame's class for `super`; its receiver,
        /// in the callee slot, is the method's `this`.
        macro_rules! enter_closure {
            ($closure:expr, $argc:expr, $params:expr) => {
                enter_closure!($closure, $argc, $params, None)
            };
            ($closure:expr, $argc:expr, $params:expr, $class:expr) => {{
                self.frames.last_mut().unwrap().ip = ip;
                for _ in $argc..$params {
                    self.stack.push(Value::Null);
                }
                let stack_base = self.stack.len() - $params - 1;
                self.frames.push(CallFrame::new(
                    $closure,
                    stack_base,
                    self.iter_stack.len(),
                    $class,
                    positional_supplied_mask($argc),
                ));
                self.debug_entered();
                (code, code_len, constants, base, ip) = load_frame!();
            }};
        }

        /// Replace the top two stack slots, both scalars, with `result`.
        macro_rules! replace_top_two {
            ($len:expr, $result:expr) => {{
                let result = $result;
                // SAFETY: both operands were just matched as Int/Float/Bool
                // (or nil), which own no heap data, so overwriting the lower one
                // without dropping it and forgetting the upper one leaks
                // nothing; `len >= 2` held for the peek.
                unsafe {
                    std::ptr::write(self.stack.as_mut_ptr().add($len - 2), result);
                    self.stack.set_len($len - 1);
                }
            }};
        }

        /// Peek at the two operands; `None` hands the op to the general arm.
        /// Unchecked like `pop2`: an op that peeks two operands is only
        /// emitted after the two pushes it consumes (`stack_effect`, pinned
        /// by `stack_effect_matches_what_each_handler_does`), and every frame
        /// keeps its callee at `stack_base` besides.
        macro_rules! peek_two {
            () => {{
                let len = self.stack.len();
                debug_assert!(len >= 2, "VM stack underflow in peek_two!");
                // SAFETY: `len >= 2`, as above. Indexing the field, not
                // calling `self.peek`, borrows only `self.stack`.
                unsafe {
                    (
                        len,
                        self.stack.get_unchecked(len - 2),
                        self.stack.get_unchecked(len - 1),
                    )
                }
            }};
        }

        /// Pop two scalars that were matched by a peek.
        macro_rules! drop_top_two {
            ($len:expr) => {
                // SAFETY: both were matched as Int/Float scalars (no heap
                // data), and `len >= 2` held for the peek.
                unsafe { self.stack.set_len($len - 2) }
            };
        }

        /// Pop the two operands `primitive_equal` just answered for, and
        /// push `$result` in their place when there is one. Scalars go the
        /// way of `replace_top_two!`/`drop_top_two!`; a string or a symbol
        /// owns a reference count, so it is popped and dropped.
        macro_rules! settle_equality_operands {
            ($len:expr, $scalars:expr) => {{
                if $scalars {
                    drop_top_two!($len);
                } else {
                    let (a, b) = self.pop2();
                    discard(b);
                    discard(a);
                }
            }};
            ($len:expr, $scalars:expr, $result:expr) => {{
                if $scalars {
                    replace_top_two!($len, $result);
                } else {
                    settle_equality_operands!($len, false);
                    self.stack.push($result);
                }
            }};
        }

        let next = loop {
            if ip >= code_len {
                break None;
            }
            // SAFETY: `code`/`code_len` cache `closure.proto.chunk.code`, which
            // the frame's own Rc keeps alive; `ip < code_len` was just checked,
            // and chunks are immutable after compilation.
            let op = unsafe { *code.add(ip) };
            ip += 1;
            match op {
                Op::GetLocal(slot) => {
                    let value = clone_scalar_fast(&self.stack[base + slot as usize]);
                    self.stack.push(value);
                }
                // An array pattern's shape test: pure, never throws.
                Op::MatchArrayLen(slot, len, at_least) => {
                    let ok = array_len_matches(&self.stack[base + slot as usize], len, at_least);
                    self.stack.push(Value::Bool(ok));
                }
                Op::GetLocal2(slot_a, slot_b) => {
                    let a = clone_scalar_fast(&self.stack[base + slot_a as usize]);
                    let b = clone_scalar_fast(&self.stack[base + slot_b as usize]);
                    self.stack.push(a);
                    self.stack.push(b);
                }
                Op::SetLocal(slot) => {
                    let value = clone_scalar_fast(self.peek(0));
                    discard(std::mem::replace(
                        &mut self.stack[base + slot as usize],
                        value,
                    ));
                }
                Op::SetLocalPop(slot) => {
                    let value = self.pop();
                    discard(std::mem::replace(
                        &mut self.stack[base + slot as usize],
                        value,
                    ));
                }
                Op::Pop => discard(self.pop()),
                Op::PopNull => {
                    discard(self.pop());
                    self.stack.push(Value::Null);
                }
                Op::Dup => {
                    let value = clone_scalar_fast(self.peek(0));
                    self.stack.push(value);
                }
                Op::Null => self.stack.push(Value::Null),
                // The nils of the locals hoisted to function entry, in one
                // dispatch. Built in place: `resize` would call `Value::clone`
                // for each slot.
                Op::PushNulls(count) => self.stack.extend((0..count).map(|_| Value::Null)),
                Op::True => self.stack.push(Value::Bool(true)),
                Op::False => self.stack.push(Value::Bool(false)),
                Op::Nop => {}
                Op::Constant(idx) => {
                    // SAFETY: `constants` points into the chunk the frame's Rc
                    // keeps alive; the index was emitted by the compiler for
                    // this chunk, as the general arm's indexing assumes.
                    let constant = unsafe { &*constants.add(idx as usize) };
                    let value = match constant {
                        Constant::Int(n) => Value::Int(*n),
                        Constant::Float(n) => Value::Float(*n),
                        Constant::Bool(b) => Value::Bool(*b),
                        Constant::Null => Value::Null,
                        // An `EcoString` clone: an inline copy or a reference
                        // count bump, never an allocation.
                        Constant::String(s) => Value::String(s.clone()),
                        // Parsed at compile time; a Decimal owns no heap data.
                        Constant::Decimal(decimal) => Value::Decimal(decimal.clone()),
                        _ => break Some(op),
                    };
                    self.stack.push(value);
                }
                Op::Symbol(idx) => {
                    // SAFETY: as for `Op::Constant`.
                    let constant = unsafe { &*constants.add(idx as usize) };
                    let Constant::String(name) = constant else {
                        break Some(op);
                    };
                    self.stack.push(Value::Symbol(name.clone()));
                }
                Op::Jump(offset) => ip += offset as usize,
                Op::JumpIfParamSupplied(param_index, offset) => {
                    // The default-value prologue, as the general arm runs it:
                    // skip the default when the caller supplied the parameter
                    // (past bit 63 the mask saturates: read as supplied).
                    let supplied = self.frames.last().unwrap().supplied;
                    if param_index >= 64 || supplied & (1u64 << param_index) != 0 {
                        ip += offset as usize;
                    }
                }
                Op::JumpIfFalse(offset) => {
                    let value = self.pop();
                    if !value.is_truthy() {
                        ip += offset as usize;
                    }
                    discard(value);
                }
                Op::JumpIfTrue(offset) => {
                    let value = self.pop();
                    if value.is_truthy() {
                        ip += offset as usize;
                    }
                    discard(value);
                }
                Op::Loop(offset) => {
                    if crate::interpreter::deadline::expired() {
                        self.frames.last_mut().unwrap().ip = ip;
                        return Err(RuntimeError::General {
                            message: crate::interpreter::deadline::timeout_message(),
                            span: self.current_span(),
                        });
                    }
                    ip -= offset as usize;
                }
                Op::ForIterRange(exit_offset) => {
                    let state = self.iter_stack.last_mut().unwrap();
                    if let IterState::Range { current, end } = state {
                        // Exclusive of `end`, matching the tree-walker.
                        if *current < *end {
                            let value = Value::Int(*current);
                            *current += 1;
                            self.stack.push(value);
                        } else {
                            self.iter_stack.pop();
                            ip += exit_offset as usize;
                        }
                    } else {
                        unreachable!("ForIterRange used with non-range iterator");
                    }
                }
                // `for x in array`: the general arm's array case, without
                // leaving this tier on every iteration.
                Op::ForIter(exit_offset) => {
                    let next = match self.iter_stack.last_mut() {
                        Some(IterState::Array { values, index }) => {
                            let arr = values.borrow();
                            if *index < arr.len() {
                                let value = arr[*index].clone();
                                *index += 1;
                                Some(value)
                            } else {
                                None
                            }
                        }
                        _ => break Some(op),
                    };
                    match next {
                        Some(value) => self.stack.push(value),
                        None => {
                            self.iter_stack.pop();
                            ip += exit_offset as usize;
                        }
                    }
                }
                // `hash.key` that finds the key, when no user method on Hash
                // could outrank it, and `post.title` on an instance that has
                // the field (`Vm::fast_field_read`): the general arm's fast
                // paths, here so a field read does not leave this tier. A key
                // named like a builtin (`row.size`) is the key too, as
                // `Interpreter::hash_member_value` reads it; only a miss may
                // answer a method. A paren-less zero-argument builtin
                // (`h.length` after the key misses, `s.blank?`, `a.first`,
                // `n.to_s`) is answered by id from the tables `CallMethodById`
                // uses for `h.length()` (`method_table::bare_zero_arg`).
                // Anything else (a miss, a method, a deferred or refused
                // field) goes to the general arm.
                Op::GetProperty(idx, mid, mask) => {
                    let hit = match self.stack.last() {
                        Some(receiver @ Value::Hash(hash))
                            if !crate::interpreter::executor::calls::user_methods::has_user_methods(
                                crate::interpreter::executor::calls::user_methods::PrimType::Hash,
                            ) =>
                        {
                            // SAFETY: as for `Op::Constant` above.
                            let key = match unsafe { &*constants.add(idx as usize) } {
                                Constant::String(name) => {
                                    hash.borrow().get(&StrKey(name.as_ref())).cloned()
                                }
                                _ => None,
                            };
                            match key {
                                None if mask != 0 => {
                                    super::method_table::bare_zero_arg(receiver, mid, mask)
                                }
                                key => key,
                            }
                        }
                        Some(receiver @ Value::Instance(_)) => {
                            // SAFETY: as for `Op::Constant` above.
                            match unsafe { &*constants.add(idx as usize) } {
                                Constant::String(name) => {
                                    self.fast_field_read(receiver, name, false)
                                }
                                _ => None,
                            }
                        }
                        Some(Value::Class(class)) => {
                            // SAFETY: as for `Op::Constant` above.
                            match unsafe { &*constants.add(idx as usize) } {
                                Constant::String(name) => Self::fast_static_read(class, name),
                                _ => None,
                            }
                        }
                        Some(receiver) if mask != 0 => {
                            super::method_table::bare_zero_arg(receiver, mid, mask)
                        }
                        _ => None,
                    };
                    match hit {
                        Some(value) => {
                            discard(std::mem::replace(self.stack.last_mut().unwrap(), value));
                        }
                        None => break Some(op),
                    }
                }
                // `row.title` with `row` a local hash that has the key, and
                // `@title` or `post.title` with the receiver a local instance
                // that has the field — the fused form of the arm above, which
                // also answers a paren-less zero-argument builtin by id.
                Op::GetLocalProperty(slot, idx, mid, mask) => {
                    let hit = match &self.stack[base + slot as usize] {
                        receiver @ Value::Hash(hash)
                            if !crate::interpreter::executor::calls::user_methods::has_user_methods(
                                crate::interpreter::executor::calls::user_methods::PrimType::Hash,
                            ) =>
                        {
                            // SAFETY: as for `Op::Constant` above.
                            let key = match unsafe { &*constants.add(idx as usize) } {
                                Constant::String(name) => {
                                    hash.borrow().get(&StrKey(name.as_ref())).cloned()
                                }
                                _ => None,
                            };
                            match key {
                                None if mask != 0 => {
                                    super::method_table::bare_zero_arg(receiver, mid, mask)
                                }
                                key => key,
                            }
                        }
                        receiver @ Value::Instance(_) => {
                            // SAFETY: as for `Op::Constant` above.
                            match unsafe { &*constants.add(idx as usize) } {
                                Constant::String(name) => {
                                    self.fast_field_read(receiver, name, slot == 0)
                                }
                                _ => None,
                            }
                        }
                        Value::Class(class) => {
                            // SAFETY: as for `Op::Constant` above.
                            match unsafe { &*constants.add(idx as usize) } {
                                Constant::String(name) => Self::fast_static_read(class, name),
                                _ => None,
                            }
                        }
                        receiver if mask != 0 => {
                            super::method_table::bare_zero_arg(receiver, mid, mask)
                        }
                        _ => None,
                    };
                    match hit {
                        Some(value) => self.stack.push(value),
                        None => break Some(op),
                    }
                }
                // `h["k"]` with `h` a local or a global hash: the hit's value,
                // or nil on a miss, as the general arms answer. Any other
                // receiver (an array, a string, a model) is theirs.
                Op::HashGetLocalConst(slot, key_idx) => {
                    // SAFETY: as for `Op::Constant` above.
                    let key = unsafe { &*constants.add(key_idx as usize) };
                    let Constant::String(key) = key else {
                        break Some(op);
                    };
                    let Value::Hash(hash) = &self.stack[base + slot as usize] else {
                        break Some(op);
                    };
                    let value = hash
                        .borrow()
                        .get(&StrKey(key.as_ref()))
                        .cloned()
                        .unwrap_or(Value::Null);
                    self.stack.push(value);
                }
                Op::HashGetGlobalConst(global_idx, key_idx) => {
                    // SAFETY: as for `Op::Constant` above.
                    let (global, key) = unsafe {
                        (
                            &*constants.add(global_idx as usize),
                            &*constants.add(key_idx as usize),
                        )
                    };
                    let (Constant::String(global), Constant::String(key)) = (global, key) else {
                        break Some(op);
                    };
                    let Some(Value::Hash(hash)) = self.globals.get(global.as_ref()) else {
                        break Some(op);
                    };
                    let value = hash
                        .borrow()
                        .get(&StrKey(key.as_ref()))
                        .cloned()
                        .unwrap_or(Value::Null);
                    self.stack.push(value);
                }
                // `xs[i]` in range and `h[k]`: the general arms' own first
                // step (`try_get_index_hit`). An index out of range, a string
                // receiver, a model go to the general arm.
                Op::GetIndex => {
                    let (_, object, index) = peek_two!();
                    let Some(value) = Self::try_get_index_hit(object, index) else {
                        break Some(op);
                    };
                    let (a, b) = self.pop2();
                    discard(b);
                    discard(a);
                    self.stack.push(value);
                }
                Op::GetLocalIndex(slot, idx_slot) => {
                    let Some(value) = Self::try_get_index_hit(
                        &self.stack[base + slot as usize],
                        &self.stack[base + idx_slot as usize],
                    ) else {
                        break Some(op);
                    };
                    self.stack.push(value);
                }
                // `@total ||= 0`'s read: the instance-field hit of
                // `GetProperty` above, so a set field short-circuits without
                // leaving the tier. A miss (Null), a model relation or
                // translated field, a deferred or refused field goes to the
                // general arm.
                Op::GetPropertyOrNull(idx) => {
                    // SAFETY: as for `Op::Constant` above.
                    let hit = match (self.stack.last(), unsafe { &*constants.add(idx as usize) }) {
                        (Some(receiver @ Value::Instance(inst)), Constant::String(name))
                            if !field_needs_model_resolution(inst, name) =>
                        {
                            self.fast_field_read(receiver, name, false)
                        }
                        _ => None,
                    };
                    match hit {
                        Some(value) => {
                            discard(std::mem::replace(self.stack.last_mut().unwrap(), value));
                        }
                        None => break Some(op),
                    }
                }
                // `@title = t` or `post.title = t` on an instance, `h.key = v`
                // on a hash: the general arm's in-place write, here when it
                // completes with no error (`Vm::fast_field_write`). The value
                // takes the receiver's place on the stack, as the assignment's
                // result. A class, a refused or frozen write goes to the
                // general arm.
                Op::SetProperty(idx) => {
                    let len = self.stack.len();
                    // SAFETY: as for `Op::Constant` above.
                    let written = match unsafe { &*constants.add(idx as usize) } {
                        Constant::String(name) => {
                            self.fast_field_write(&self.stack[len - 2], name, &self.stack[len - 1])
                        }
                        _ => false,
                    };
                    if !written {
                        break Some(op);
                    }
                    let value = self.pop();
                    discard(std::mem::replace(self.stack.last_mut().unwrap(), value));
                }
                Op::TemplateWriteConst(idx) => {
                    // SAFETY: as for `Op::Constant` above.
                    if let Constant::String(text) = unsafe { &*constants.add(idx as usize) } {
                        self.template_out.push_str(text);
                    }
                }
                Op::TemplateWrite(escaped) => {
                    // A function value goes to the general arm, which hands
                    // the render back to the tree-walker.
                    if matches!(
                        self.stack.last(),
                        Some(
                            Value::Function(_)
                                | Value::NativeFunction(_)
                                | Value::VmClosure(_)
                                | Value::Method(_)
                        )
                    ) {
                        break Some(op);
                    }
                    let value = self.pop();
                    crate::template::renderer::write_value_to_output(
                        &value,
                        escaped,
                        &mut self.template_out,
                    );
                    discard(value);
                }
                Op::Add | Op::Subtract | Op::Multiply => {
                    let (len, a, b) = peek_two!();
                    let result = match (a, b) {
                        (Value::Int(x), Value::Int(y)) => {
                            let checked = match op {
                                Op::Add => x.checked_add(*y),
                                Op::Subtract => x.checked_sub(*y),
                                _ => x.checked_mul(*y),
                            };
                            match checked {
                                Some(n) => Value::Int(n),
                                None => break Some(op),
                            }
                        }
                        (Value::Float(_) | Value::Int(_), Value::Float(_) | Value::Int(_)) => {
                            let x = match a {
                                Value::Float(f) => *f,
                                Value::Int(n) => *n as f64,
                                _ => unreachable!(),
                            };
                            let y = match b {
                                Value::Float(f) => *f,
                                Value::Int(n) => *n as f64,
                                _ => unreachable!(),
                            };
                            Value::Float(match op {
                                Op::Add => x + y,
                                Op::Subtract => x - y,
                                _ => x * y,
                            })
                        }
                        _ => break Some(op),
                    };
                    replace_top_two!(len, result);
                }
                Op::Less | Op::LessEqual | Op::Greater | Op::GreaterEqual => {
                    let (len, a, b) = peek_two!();
                    let Some(ordering) = scalar_ordering(a, b) else {
                        break Some(op);
                    };
                    let result = match op {
                        Op::Less => ordering.is_lt(),
                        Op::LessEqual => ordering.is_le(),
                        Op::Greater => ordering.is_gt(),
                        _ => ordering.is_ge(),
                    };
                    replace_top_two!(len, Value::Bool(result));
                }
                Op::Equal | Op::NotEqual => {
                    let (len, a, b) = peek_two!();
                    let Some(equal) = primitive_equal(a, b) else {
                        break Some(op);
                    };
                    let scalars = owns_no_heap(a) && owns_no_heap(b);
                    let result = Value::Bool(equal == matches!(op, Op::Equal));
                    settle_equality_operands!(len, scalars, result);
                }
                Op::TestLessJump(offset)
                | Op::TestLessEqualJump(offset)
                | Op::TestGreaterJump(offset)
                | Op::TestGreaterEqualJump(offset) => {
                    let (len, a, b) = peek_two!();
                    let Some(ordering) = scalar_ordering(a, b) else {
                        break Some(op);
                    };
                    let holds = match op {
                        Op::TestLessJump(_) => ordering.is_lt(),
                        Op::TestLessEqualJump(_) => ordering.is_le(),
                        Op::TestGreaterJump(_) => ordering.is_gt(),
                        _ => ordering.is_ge(),
                    };
                    drop_top_two!(len);
                    if !holds {
                        ip += offset as usize;
                    }
                }
                Op::IncrLocal(slot) | Op::IncrLocalFast(slot) => {
                    let idx = base + slot as usize;
                    match self.stack[idx] {
                        Value::Int(n) if n != i64::MAX => self.stack[idx] = Value::Int(n + 1),
                        _ => break Some(op),
                    }
                }
                Op::DecrLocal(slot) => {
                    let idx = base + slot as usize;
                    match self.stack[idx] {
                        Value::Int(n) if n != i64::MIN => self.stack[idx] = Value::Int(n - 1),
                        _ => break Some(op),
                    }
                }
                // `/` and `%` on two Ints, and `/` on two Floats. A zero
                // divisor, `i64::MIN / -1`, a Float `%` and any mix go to the
                // general arm, which raises or converts.
                Op::Divide | Op::Modulo => {
                    let (len, a, b) = peek_two!();
                    let Some(result) = fast_quotient(a, b, matches!(op, Op::Divide)) else {
                        break Some(op);
                    };
                    replace_top_two!(len, result);
                }
                Op::DivLocalLocal(slot_a, slot_b) | Op::ModLocalLocal(slot_a, slot_b) => {
                    let Some(result) = fast_quotient(
                        &self.stack[base + slot_a as usize],
                        &self.stack[base + slot_b as usize],
                        matches!(op, Op::DivLocalLocal(..)),
                    ) else {
                        break Some(op);
                    };
                    self.stack.push(result);
                }
                Op::DivLocalConst(slot, const_idx) | Op::ModLocalConst(slot, const_idx) => {
                    // SAFETY: as for `Op::Constant`.
                    let constant = unsafe { &*constants.add(const_idx as usize) };
                    let divisor = match constant {
                        Constant::Int(n) => Value::Int(*n),
                        Constant::Float(f) => Value::Float(*f),
                        _ => break Some(op),
                    };
                    let Some(result) = fast_quotient(
                        &self.stack[base + slot as usize],
                        &divisor,
                        matches!(op, Op::DivLocalConst(..)),
                    ) else {
                        break Some(op);
                    };
                    self.stack.push(result);
                }
                Op::AddLocalConst(slot, const_idx) => {
                    // SAFETY: as for `Op::Constant`.
                    let constant = unsafe { &*constants.add(const_idx as usize) };
                    let sum = match (&self.stack[base + slot as usize], constant) {
                        (Value::Int(x), Constant::Int(y)) => x.checked_add(*y),
                        _ => None,
                    };
                    match sum {
                        Some(n) => self.stack.push(Value::Int(n)),
                        None => break Some(op),
                    }
                }
                // `k += 2`, `k -= 2`, `k *= 2` on two Ints or two Floats, in
                // place. An Int overflow and any mix go to the general arm,
                // which raises or converts.
                Op::AddLocalConstInPlace(slot, const_idx)
                | Op::SubLocalConstInPlace(slot, const_idx)
                | Op::MulLocalConstInPlace(slot, const_idx) => {
                    // SAFETY: as for `Op::Constant`.
                    let constant = unsafe { &*constants.add(const_idx as usize) };
                    let idx = base + slot as usize;
                    let result = match (&self.stack[idx], constant) {
                        (Value::Int(x), Constant::Int(y)) => match op {
                            Op::AddLocalConstInPlace(..) => x.checked_add(*y),
                            Op::SubLocalConstInPlace(..) => x.checked_sub(*y),
                            _ => x.checked_mul(*y),
                        }
                        .map(Value::Int),
                        (Value::Float(x), Constant::Float(y)) => Some(Value::Float(match op {
                            Op::AddLocalConstInPlace(..) => x + y,
                            Op::SubLocalConstInPlace(..) => x - y,
                            _ => x * y,
                        })),
                        _ => None,
                    };
                    match result {
                        Some(value) => self.stack[idx] = value,
                        None => break Some(op),
                    }
                }
                // Calls and returns stay in the tier for the common shape;
                // anything else — a native or a class callee, an arity
                // error, the depth limit, open upvalues, a return that
                // leaves this `run` — goes to the general arms, which own
                // every one of those rules and their errors. A function
                // with a native kernel is tried here as the general arm
                // tries it: the value replaces callee and arguments, and a
                // decline runs the bytecode, still in the tier.
                Op::Call(argc) => {
                    let argc = argc as usize;
                    let callee = self.stack.len() - 1 - argc;
                    if self.frames.len() >= MAX_CALL_DEPTH {
                        break Some(op);
                    }
                    // An app-level `def` under serve is a tree-walker
                    // `Function` whose VM closure is cached on it after the
                    // first call: it enters here as a `VmClosure` does when
                    // it takes exactly its parameters and has no native
                    // kernel, and takes that closure's place in the callee
                    // slot, as the general arm's `call_compiled_function`
                    // leaves it.
                    let (closure, from_function) = match &self.stack[callee] {
                        Value::VmClosure(closure) if accepts_positional(&closure.proto, argc) => {
                            if closure.proto.kernel.is_some() {
                                if let Some(value) = self.kernel_value(closure, argc) {
                                    self.stack.truncate(callee);
                                    self.stack.push(value);
                                    continue;
                                }
                            }
                            (closure.clone(), false)
                        }
                        Value::Function(func) if !self.tree_walk_functions => {
                            match super::vm_calls::cached_function_closure(func) {
                                Some(closure)
                                    if closure.proto.param_names.len() == argc
                                        && closure.proto.kernel.is_none() =>
                                {
                                    (closure, true)
                                }
                                _ => break Some(op),
                            }
                        }
                        _ => break Some(op),
                    };
                    // The handler budget, as `ensure_call_depth` checks it
                    // for the general arm — which raises it (expiry is
                    // sticky). Nothing is touched yet, so it can take over.
                    if crate::interpreter::deadline::expired() {
                        break Some(op);
                    }
                    if from_function {
                        self.stack[callee] = Value::VmClosure(closure.clone());
                    }
                    let params = closure.proto.param_names.len();
                    enter_closure!(closure, argc, params);
                }
                Op::CallGlobal(name_idx, argc) => {
                    let argc = argc as usize;
                    // SAFETY: as for `Op::Constant`.
                    let name = match unsafe { &*constants.add(name_idx as usize) } {
                        Constant::String(name) => name.as_ref(),
                        _ => break Some(op),
                    };
                    let closure = match self.globals.get(name) {
                        Some(Value::VmClosure(closure))
                            if accepts_positional(&closure.proto, argc)
                                && self.frames.len() < MAX_CALL_DEPTH =>
                        {
                            if closure.proto.kernel.is_some() {
                                if let Some(value) = self.kernel_value(closure, argc) {
                                    let args = self.stack.len() - argc;
                                    self.stack.truncate(args);
                                    self.stack.push(value);
                                    continue;
                                }
                            }
                            closure.clone()
                        }
                        _ => break Some(op),
                    };
                    // The handler budget, before the stack is touched (see
                    // `Op::Call` above).
                    if crate::interpreter::deadline::expired() {
                        break Some(op);
                    }
                    // The general arm inserts the callee below the arguments.
                    let insert_at = self.stack.len() - argc;
                    self.stack
                        .insert(insert_at, Value::VmClosure(closure.clone()));
                    let params = closure.proto.param_names.len();
                    enter_closure!(closure, argc, params);
                }
                // `post.summary()`, `@helper(x)`, `this.total(a, b)` on an
                // instance whose class chain has the method compiled: the
                // general arms' compiled-method call (`Vm::fast_method`), the
                // receiver staying below the arguments as `this`. A native or
                // AST-only method, `method_missing`, `send`, omitted
                // parameters, the depth limit and a refused call go to the
                // general arm.
                Op::CallMethod(name_idx, argc, _) | Op::CallMethodById(name_idx, argc, _, _) => {
                    let argc = argc as usize;
                    // SAFETY: as for `Op::Constant`.
                    let name = match unsafe { &*constants.add(name_idx as usize) } {
                        Constant::String(name) => name.as_ref(),
                        _ => break Some(op),
                    };
                    // The op's own address: `ip` is already past it.
                    let site = code.wrapping_add(ip - 1);
                    let Some((method, defining)) = self.fast_method(site, argc, name) else {
                        break Some(op);
                    };
                    // The handler budget, before the stack is touched (see
                    // `Op::Call` above): recursion through a method has no
                    // backward jump, so this is where it stops.
                    if crate::interpreter::deadline::expired() {
                        break Some(op);
                    }
                    enter_closure!(method, argc, argc, Some(defining));
                }
                Op::Return => {
                    let depth_after = self.frames.len() - 1;
                    if depth_after <= self.return_depth
                        || (self.check_return_types && !self.return_value_admitted())
                    {
                        break Some(op);
                    }
                    // The frame's own variables go out of scope: close the
                    // cells that closures made here hold on them, while the
                    // slots still have their values — what the general arm
                    // does. Cells on an outer frame's locals (a callback's
                    // captures of its caller's variables) stay open.
                    if self.has_open_upvalues_from(base) {
                        self.close_upvalues(base);
                    }
                    let (on_return, params, iter_base) = {
                        let frame = self.frames.last().unwrap();
                        (
                            frame.on_return,
                            frame.closure.proto.param_names.len(),
                            frame.iter_base,
                        )
                    };
                    if on_return != FrameReturn::Value {
                        if on_return == FrameReturn::Discard {
                            // Its value goes nowhere: the frame leaves the
                            // stack beneath it as it found it.
                            self.stack.pop();
                            let frame = self.frames.pop().unwrap();
                            self.debug_left();
                            self.stack.truncate(frame.stack_base);
                            self.iter_stack.truncate(frame.iter_base);
                            while self
                                .exception_handlers
                                .last()
                                .is_some_and(|handler| handler.frame_depth > depth_after)
                            {
                                self.exception_handlers.pop();
                            }
                            (code, code_len, constants, base, ip) = load_frame!();
                            continue;
                        }
                        // A loop callback: the frame is kept and run again for
                        // the next element. Clean up exactly what a return
                        // would — the callback's temporaries and locals above
                        // its arguments, its `for` loops, its `try` handlers —
                        // then let the loop record the result and refill the
                        // argument slots. Its cells were closed above: what a
                        // closure made in this pass captured keeps this pass's
                        // values, and the next pass's closures get new cells.
                        let result = self.pop();
                        self.stack.truncate(base + 1 + params);
                        self.iter_stack.truncate(iter_base);
                        while self
                            .exception_handlers
                            .last()
                            .is_some_and(|handler| handler.frame_depth > depth_after)
                        {
                            self.exception_handlers.pop();
                        }
                        if self.rerun_loop_callback(result, base)? {
                            ip = 0;
                        } else {
                            (code, code_len, constants, base, ip) = load_frame!();
                        }
                        continue;
                    }
                    // What the general arm does when it keeps running, in the
                    // same order: result, frame, stack, iterators, handlers.
                    let result = self.pop();
                    let frame = self.frames.pop().unwrap();
                    self.debug_left();
                    self.stack.truncate(frame.stack_base);
                    self.iter_stack.truncate(frame.iter_base);
                    while self
                        .exception_handlers
                        .last()
                        .is_some_and(|handler| handler.frame_depth > depth_after)
                    {
                        self.exception_handlers.pop();
                    }
                    self.stack.push(result);
                    (code, code_len, constants, base, ip) = load_frame!();
                }
                Op::GetUpvalue(idx) => {
                    let value = {
                        let frame = self.frames.last().unwrap();
                        let upvalue = frame.closure.upvalues[idx as usize].borrow();
                        match &*upvalue {
                            Upvalue::Open(slot) => self.stack[*slot].clone(),
                            Upvalue::Closed(value) => value.clone(),
                        }
                    };
                    self.stack.push(value);
                }
                Op::SetUpvalue(idx) => {
                    let value = self.peek(0).clone();
                    // `frames` and `stack` are borrowed apart, so the cell is
                    // reached without a reference-count round trip.
                    let frame = self.frames.last().unwrap();
                    match &mut *frame.closure.upvalues[idx as usize].borrow_mut() {
                        Upvalue::Open(slot) => self.stack[*slot] = value,
                        Upvalue::Closed(closed) => *closed = value,
                    }
                }
                // `captured = captured + value` (and `captured += value`) on
                // numbers, as `AddAssignLocal` here: a string, anything else
                // and an overflowing add are the general arm's.
                Op::AddAssignUpvalue(idx) => {
                    let (len, a, b) = peek_two!();
                    let sum = match (a, b) {
                        (Value::Int(x), Value::Int(y)) => match x.checked_add(*y) {
                            Some(n) => Value::Int(n),
                            None => break Some(op),
                        },
                        (Value::Float(x), Value::Float(y)) => Value::Float(x + y),
                        (Value::Int(x), Value::Float(y)) => Value::Float(*x as f64 + y),
                        (Value::Float(x), Value::Int(y)) => Value::Float(x + *y as f64),
                        _ => break Some(op),
                    };
                    let frame = self.frames.last().unwrap();
                    match &mut *frame.closure.upvalues[idx as usize].borrow_mut() {
                        Upvalue::Open(slot) => self.stack[*slot] = sum.clone(),
                        Upvalue::Closed(value) => *value = sum.clone(),
                    }
                    replace_top_two!(len, sum);
                }
                Op::Closure(idx) => {
                    // SAFETY: as for `Op::Constant`. `make_closure` pushes and
                    // pops no frame, so the chunk holding the prototype stays
                    // alive while it runs.
                    let proto = match unsafe { &*constants.add(idx as usize) } {
                        Constant::Function(proto) => proto,
                        _ => break Some(op),
                    };
                    let closure = self.make_closure(proto);
                    self.stack.push(Value::VmClosure(closure));
                }
                // A captured block-scoped local goes out of scope (a loop
                // variable at the end of its pass, a block's `let`).
                Op::CloseUpvalue => {
                    let slot = self.stack.len() - 1;
                    self.close_upvalues(slot);
                    discard(self.pop());
                }
                Op::GetGlobal(idx) => {
                    // SAFETY: as for `Op::Constant`.
                    let name = match unsafe { &*constants.add(idx as usize) } {
                        Constant::String(name) => name.as_ref(),
                        _ => break Some(op),
                    };
                    // A miss is the general arm's to report.
                    let Some(value) = self.globals.get(name).cloned() else {
                        break Some(op);
                    };
                    self.stack.push(value);
                }
                Op::AddLocalLocal(slot_a, slot_b)
                | Op::SubLocalLocal(slot_a, slot_b)
                | Op::MulLocalLocal(slot_a, slot_b) => {
                    let a = &self.stack[base + slot_a as usize];
                    let b = &self.stack[base + slot_b as usize];
                    let result = match (a, b) {
                        (Value::Int(x), Value::Int(y)) => match op {
                            Op::AddLocalLocal(..) => x.checked_add(*y),
                            Op::SubLocalLocal(..) => x.checked_sub(*y),
                            _ => x.checked_mul(*y),
                        }
                        .map(Value::Int),
                        (Value::Float(x), Value::Float(y)) => Some(Value::Float(match op {
                            Op::AddLocalLocal(..) => x + y,
                            Op::SubLocalLocal(..) => x - y,
                            _ => x * y,
                        })),
                        _ => None,
                    };
                    match result {
                        Some(value) => self.stack.push(value),
                        None => break Some(op),
                    }
                }
                Op::SubLocalConst(slot, const_idx) | Op::MulLocalConst(slot, const_idx) => {
                    // SAFETY: as for `Op::Constant`.
                    let constant = unsafe { &*constants.add(const_idx as usize) };
                    let result = match (&self.stack[base + slot as usize], constant) {
                        (Value::Int(x), Constant::Int(y)) => match op {
                            Op::SubLocalConst(..) => x.checked_sub(*y),
                            _ => x.checked_mul(*y),
                        },
                        _ => None,
                    };
                    match result {
                        Some(n) => self.stack.push(Value::Int(n)),
                        None => break Some(op),
                    }
                }
                Op::AddLocalInt(slot, n) => match self.stack[base + slot as usize] {
                    Value::Int(x) => match x.checked_add(n as i64) {
                        Some(sum) => self.stack.push(Value::Int(sum)),
                        None => break Some(op),
                    },
                    _ => break Some(op),
                },
                Op::AddLocalsInPlace(a, b) => {
                    let (ia, ib) = (base + a as usize, base + b as usize);
                    enum Pair {
                        Int(i64, i64),
                        Float(f64, f64),
                        Str,
                        Other,
                    }
                    let pair = match (&self.stack[ia], &self.stack[ib]) {
                        (Value::Int(x), Value::Int(y)) => Pair::Int(*x, *y),
                        (Value::Float(x), Value::Float(y)) => Pair::Float(*x, *y),
                        (Value::String(_), Value::String(_)) => Pair::Str,
                        _ => Pair::Other,
                    };
                    match pair {
                        Pair::Int(x, y) => match x.checked_add(y) {
                            Some(sum) => self.stack[ia] = Value::Int(sum),
                            None => break Some(op),
                        },
                        Pair::Float(x, y) => self.stack[ia] = Value::Float(x + y),
                        Pair::Str => append_strings_in_place(&mut self.stack, ia, ib),
                        Pair::Other => break Some(op),
                    }
                }
                // Numbers stay in this loop, as `Add` + `SetLocal` did before
                // the fused op existed; a string (or anything else) is the
                // general arm's.
                Op::AddAssignLocal(slot) => {
                    let (len, a, b) = peek_two!();
                    let sum = match (a, b) {
                        (Value::Int(x), Value::Int(y)) => match x.checked_add(*y) {
                            Some(n) => Value::Int(n),
                            None => break Some(op),
                        },
                        (Value::Float(x), Value::Float(y)) => Value::Float(x + y),
                        (Value::Int(x), Value::Float(y)) => Value::Float(*x as f64 + y),
                        (Value::Float(x), Value::Int(y)) => Value::Float(x + *y as f64),
                        _ => break Some(op),
                    };
                    self.stack[base + slot as usize] = sum.clone();
                    replace_top_two!(len, sum);
                }
                Op::Array(n) => {
                    let array = self.array_from_stack(n as usize);
                    self.stack.push(array);
                }
                // An array literal made of literals (`literal_list.rs`).
                Op::LiteralArray(list_idx) => {
                    // SAFETY: as for `Op::Constant`.
                    let list = match unsafe { &*constants.add(list_idx as usize) } {
                        Constant::LiteralList(list) => list,
                        _ => break Some(op),
                    };
                    let array = self.literal_array(list);
                    self.stack.push(array);
                }
                // `[<literals>].includes?(x)` for a scalar `x` when no user
                // method can take the call: the general arm's answer, without
                // its by-value comparison. Anything else goes to that arm.
                Op::LiteralListIncludes(list_idx, user_method) => {
                    // SAFETY: as for `Op::Constant`.
                    let list = match unsafe { &*constants.add(list_idx as usize) } {
                        Constant::LiteralList(list) => list,
                        _ => break Some(op),
                    };
                    if user_method != super::opcode::NO_USER_METHOD
                        && crate::interpreter::executor::calls::user_methods::has_user_methods(
                            crate::interpreter::executor::calls::user_methods::PrimType::Array,
                        )
                    {
                        break Some(op);
                    }
                    let Some(found) = list.contains_scalar(self.stack.last().unwrap()) else {
                        break Some(op);
                    };
                    discard(std::mem::replace(
                        self.stack.last_mut().unwrap(),
                        Value::Bool(found),
                    ));
                }
                Op::LessLocalConst(slot, const_idx)
                | Op::LessEqualLocalConst(slot, const_idx)
                | Op::GreaterLocalConst(slot, const_idx)
                | Op::GreaterEqualLocalConst(slot, const_idx) => {
                    // SAFETY: as for `Op::Constant`.
                    let constant = unsafe { &*constants.add(const_idx as usize) };
                    let ordering = match (&self.stack[base + slot as usize], constant) {
                        (Value::Int(x), Constant::Int(y)) => Some(x.cmp(y)),
                        (Value::Float(x), Constant::Float(y)) => x.partial_cmp(y),
                        _ => None,
                    };
                    let Some(ordering) = ordering else {
                        break Some(op);
                    };
                    let result = match op {
                        Op::LessLocalConst(..) => ordering.is_lt(),
                        Op::LessEqualLocalConst(..) => ordering.is_le(),
                        Op::GreaterLocalConst(..) => ordering.is_gt(),
                        _ => ordering.is_ge(),
                    };
                    self.stack.push(Value::Bool(result));
                }
                Op::LessLocalLocal(slot_a, slot_b)
                | Op::LessEqualLocalLocal(slot_a, slot_b)
                | Op::GreaterLocalLocal(slot_a, slot_b) => {
                    let ordering = scalar_ordering(
                        &self.stack[base + slot_a as usize],
                        &self.stack[base + slot_b as usize],
                    );
                    let Some(ordering) = ordering else {
                        break Some(op);
                    };
                    let result = match op {
                        Op::LessLocalLocal(..) => ordering.is_lt(),
                        Op::LessEqualLocalLocal(..) => ordering.is_le(),
                        _ => ordering.is_gt(),
                    };
                    self.stack.push(Value::Bool(result));
                }
                Op::EqualLocalLocal(slot_a, slot_b) | Op::NotEqualLocalLocal(slot_a, slot_b) => {
                    let Some(equal) = primitive_equal(
                        &self.stack[base + slot_a as usize],
                        &self.stack[base + slot_b as usize],
                    ) else {
                        break Some(op);
                    };
                    let want_equal = matches!(op, Op::EqualLocalLocal(..));
                    self.stack.push(Value::Bool(equal == want_equal));
                }
                Op::EqualLocalConst(slot, const_idx) | Op::NotEqualLocalConst(slot, const_idx) => {
                    // SAFETY: as for `Op::Constant`.
                    let constant = unsafe { &*constants.add(const_idx as usize) };
                    let Some(equal) =
                        primitive_equal_constant(&self.stack[base + slot as usize], constant)
                    else {
                        break Some(op);
                    };
                    let want_equal = matches!(op, Op::EqualLocalConst(..));
                    self.stack.push(Value::Bool(equal == want_equal));
                }
                // `if a == b` / `if a != b`: pop both, and branch past the
                // body when the condition is false.
                Op::TestEqualJump(offset) | Op::TestNotEqualJump(offset) => {
                    let (len, a, b) = peek_two!();
                    let Some(equal) = primitive_equal(a, b) else {
                        break Some(op);
                    };
                    let scalars = owns_no_heap(a) && owns_no_heap(b);
                    settle_equality_operands!(len, scalars);
                    if equal != matches!(op, Op::TestEqualJump(_)) {
                        ip += offset as usize;
                    }
                }
                Op::IsTruthyLocal(slot) => {
                    let truthy = self.stack[base + slot as usize].is_truthy();
                    self.stack.push(Value::Bool(truthy));
                }
                Op::IsFalsyLocal(slot) | Op::NotLocal(slot) => {
                    let truthy = self.stack[base + slot as usize].is_truthy();
                    self.stack.push(Value::Bool(!truthy));
                }
                // `if local` / `unless local`: test the slot in place, no
                // push and pop. A `Deferred` local is resolved by
                // `is_truthy`, as `IsTruthyLocal` did.
                Op::JumpIfLocalFalsy(slot, offset) => {
                    if !self.stack[base + slot as usize].is_truthy() {
                        ip += offset as usize;
                    }
                }
                Op::JumpIfLocalTruthy(slot, offset) => {
                    if self.stack[base + slot as usize].is_truthy() {
                        ip += offset as usize;
                    }
                }
                Op::IsZeroLocal(slot) | Op::NotZeroLocal(slot) => {
                    let zero = match &self.stack[base + slot as usize] {
                        Value::Int(x) => *x == 0,
                        Value::Float(x) => *x == 0.0,
                        _ => false,
                    };
                    let want_zero = matches!(op, Op::IsZeroLocal(_));
                    self.stack.push(Value::Bool(zero == want_zero));
                }
                Op::Not => {
                    let top = self.peek_mut(0);
                    let truthy = top.is_truthy();
                    discard(std::mem::replace(top, Value::Bool(!truthy)));
                }
                Op::JumpIfFalseNoPop(offset) => {
                    if !self.peek(0).is_truthy() {
                        ip += offset as usize;
                    }
                }
                Op::JumpIfTrueNoPop(offset) => {
                    if self.peek(0).is_truthy() {
                        ip += offset as usize;
                    }
                }
                Op::IsNull | Op::NotNull => {
                    let null = matches!(self.peek(0), Value::Null);
                    self.stack
                        .push(Value::Bool(null == matches!(op, Op::IsNull)));
                }
                Op::JumpIfNull(offset) => {
                    if matches!(self.peek(0), Value::Null) {
                        ip += offset as usize;
                    }
                }
                // `a ?? b` and `x ??= v`: the general arms of the two ops are
                // the same test.
                Op::JumpIfNotNull(offset) | Op::NullishJump(offset) => {
                    if !matches!(self.peek(0), Value::Null) {
                        ip += offset as usize;
                    }
                }
                Op::GetAndNullLocal(slot) => {
                    let value =
                        std::mem::replace(&mut self.stack[base + slot as usize], Value::Null);
                    self.stack.push(value);
                }
                // `try` and `expr rescue x` on the path that does not throw.
                // None of these pushes a frame, so the cached state stays
                // valid; the throw side (`throw_exception`, the catch-resume
                // in `run`) reads only the handler, never the tier it was
                // pushed from. Each records what the general arm records:
                // `ip` is already past the op, as `frame.ip` is there.
                Op::TryBegin(catch_offset, finally_offset) => {
                    self.exception_handlers.push(ExceptionHandler {
                        catch_ip: ip + catch_offset as usize,
                        finally_ip: ip + finally_offset as usize,
                        stack_depth: self.stack.len(),
                        frame_depth: self.frames.len(),
                        iter_depth: self.iter_stack.len(),
                    });
                }
                Op::TryEnd | Op::PopHandler => {
                    self.exception_handlers.pop();
                }
                // Only a Future has anything to settle; that stays general.
                Op::ForceFuture => {
                    if self.stack.last().is_some_and(Value::is_future) {
                        break Some(op);
                    }
                }
                Op::RescueJump(offset) => ip += offset as usize,
                _ => break Some(op),
            }
        };
        self.frames.last_mut().unwrap().ip = ip;
        Ok(next)
    }

    fn run_dispatch(&mut self) -> Result<Value, RuntimeError> {
        loop {
            // Run the simple ops in `run_fast`, which keeps the frame's state
            // in registers; it returns the first op it does not handle, with
            // `frame.ip` already past it — exactly what the fetch here used
            // to leave behind — and every arm below runs unchanged.
            let op = match self.run_fast()? {
                Some(op) => op,
                None => return Ok(Value::Null),
            };
            // self is now fully available for mutation

            match op {
                Op::Constant(idx) => {
                    let frame = self.frames.last().unwrap();
                    let constant = &frame.closure.proto.chunk.constants[idx as usize];
                    // Inline fast path for the most common constant types
                    let value = match constant {
                        Constant::Int(n) => Value::Int(*n),
                        Constant::Float(n) => Value::Float(*n),
                        Constant::Bool(b) => Value::Bool(*b),
                        Constant::Null => Value::Null,
                        _ => constant_to_value(constant),
                    };
                    self.stack.push(value);
                }
                Op::Symbol(idx) => {
                    let frame = self.frames.last().unwrap();
                    if let Constant::String(s) = &frame.closure.proto.chunk.constants[idx as usize]
                    {
                        self.stack.push(Value::Symbol(s.clone()));
                    }
                }
                Op::Null => self.stack.push(Value::Null),
                Op::PushNulls(count) => self.stack.extend((0..count).map(|_| Value::Null)),
                Op::True => self.stack.push(Value::Bool(true)),
                Op::False => self.stack.push(Value::Bool(false)),

                Op::Pop => {
                    self.stack.pop();
                }
                Op::Dup => {
                    let val = self.stack.last().unwrap().clone();
                    self.stack.push(val);
                }

                Op::GetLocal(slot) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let idx = base + slot as usize;
                    let val = self.stack[idx].clone();
                    self.stack.push(val);
                }
                Op::JumpIfParamSupplied(param_index, offset) => {
                    // Parameter-default prologue: skip the default-value
                    // bytecode when the caller supplied this parameter.
                    // Parameters past bit 63 read as supplied (mask saturates).
                    let frame = self.frames.last().unwrap();
                    let supplied =
                        param_index >= 64 || frame.supplied & (1u64 << param_index as u64) != 0;
                    if supplied {
                        self.frames.last_mut().unwrap().ip += offset as usize;
                    }
                }
                Op::SetLocal(slot) => {
                    let val = self.stack.last().unwrap().clone();
                    let base = self.frames.last().unwrap().stack_base;
                    self.stack[base + slot as usize] = val;
                }
                Op::GetGlobal(idx) => {
                    // Avoid cloning the string constant for lookup
                    let val = {
                        let frame = self.frames.last().unwrap();
                        let name = match &frame.closure.proto.chunk.constants[idx as usize] {
                            Constant::String(s) => s.as_ref(),
                            _ => "",
                        };
                        self.globals.get(name).cloned()
                    };
                    match val {
                        Some(v) => self.stack.push(v),
                        None => {
                            let name = self.read_string_constant_owned(idx);
                            // `Invoice` in a method of `Billing::PayInvoice`.
                            match self.namespaced(&name) {
                                Some(class) => self.stack.push(class),
                                None => return Err(self.undefined_global(name)),
                            }
                        }
                    }
                }
                Op::SetGlobal(idx) => {
                    // Bare assignment (`name = value`) creates the binding if it
                    // doesn't exist yet — matching the tree-walker, where a
                    // top-level assignment to a not-yet-defined name defines it.
                    // The compiler only emits SetGlobal for top-level assignments
                    // or for in-function assignments to names it knows are
                    // globals; brand-new in-function names become locals instead.
                    let val = self.stack.last().unwrap().clone();
                    // Avoid cloning the string constant for lookup
                    let updated = {
                        let frame = self.frames.last().unwrap();
                        let name = match &frame.closure.proto.chunk.constants[idx as usize] {
                            Constant::String(s) => s.as_ref(),
                            _ => "",
                        };
                        if let Some(entry) = self.globals.get_mut(name) {
                            *entry = val.clone();
                            true
                        } else {
                            false
                        }
                    };
                    if !updated {
                        let name = self.read_string_constant_owned(idx);
                        // Define-if-absent only when optional-`let` is enabled;
                        // otherwise a bare assignment to an undefined name is a
                        // runtime error (which triggers the interpreter fallback).
                        if crate::vm::compiler::optional_let_enabled() {
                            self.globals.insert(name, val);
                        } else {
                            return Err(self.undefined_global(name));
                        }
                    }
                }
                Op::DefineGlobal(idx) => {
                    let name = self.read_string_constant_owned(idx);
                    let val = self.stack.pop().unwrap();
                    self.globals.insert(name, val);
                }

                Op::GetUpvalue(idx) => {
                    let val = {
                        let frame = self.frames.last().unwrap();
                        let upvalue = &frame.closure.upvalues[idx as usize];
                        let uv = upvalue.borrow();
                        match &*uv {
                            Upvalue::Open(slot) => self.stack[*slot].clone(),
                            Upvalue::Closed(val) => val.clone(),
                        }
                    };
                    self.stack.push(val);
                }
                Op::SetUpvalue(idx) => {
                    let val = self.stack.last().unwrap().clone();
                    let frame = self.frames.last().unwrap();
                    match &mut *frame.closure.upvalues[idx as usize].borrow_mut() {
                        Upvalue::Open(slot) => self.stack[*slot] = val,
                        Upvalue::Closed(v) => *v = val,
                    }
                }
                Op::CloseUpvalue => {
                    let slot = self.stack.len() - 1;
                    self.close_upvalues(slot);
                    self.stack.pop();
                }

                // --- Arithmetic (inlined fast paths for Int) ---
                Op::Add => {
                    let (a, b) = self.pop2();
                    let result = match (&a, &b) {
                        (Value::Int(x), Value::Int(y)) => match x.checked_add(*y) {
                            Some(result) => Value::Int(result),
                            None => {
                                let span = self.current_span();
                                return Err(crate::interpreter::executor::operators::int_overflow(
                                    *x, "+", *y, span,
                                ));
                            }
                        },
                        (Value::Float(x), Value::Float(y)) => Value::Float(x + y),
                        (Value::Int(x), Value::Float(y)) => Value::Float(*x as f64 + y),
                        (Value::Float(x), Value::Int(y)) => Value::Float(x + *y as f64),
                        (Value::String(x), Value::String(y)) => {
                            // Build the SoliStr directly — short results stay
                            // inline, no String intermediate.
                            let mut s = crate::interpreter::value::SoliStr::with_capacity(
                                x.len() + y.len(),
                            );
                            s.push_str(x);
                            s.push_str(y);
                            Value::String(s)
                        }
                        _ => self.op_add(a, b, self.current_span())?,
                    };
                    self.stack.push(result);
                }
                Op::Subtract => {
                    let (a, b) = self.pop2();
                    let result = match (&a, &b) {
                        (Value::Int(x), Value::Int(y)) => match x.checked_sub(*y) {
                            Some(result) => Value::Int(result),
                            None => {
                                let span = self.current_span();
                                return Err(crate::interpreter::executor::operators::int_overflow(
                                    *x, "-", *y, span,
                                ));
                            }
                        },
                        (Value::Float(x), Value::Float(y)) => Value::Float(x - y),
                        (Value::Int(x), Value::Float(y)) => Value::Float(*x as f64 - y),
                        (Value::Float(x), Value::Int(y)) => Value::Float(x - *y as f64),
                        _ => {
                            let span = self.current_span();
                            self.op_subtract(a, b, span)?
                        }
                    };
                    self.stack.push(result);
                }
                Op::Multiply => {
                    let (a, b) = self.pop2();
                    let result = match (&a, &b) {
                        (Value::Int(x), Value::Int(y)) => match x.checked_mul(*y) {
                            Some(result) => Value::Int(result),
                            None => {
                                let span = self.current_span();
                                return Err(crate::interpreter::executor::operators::int_overflow(
                                    *x, "*", *y, span,
                                ));
                            }
                        },
                        (Value::Float(x), Value::Float(y)) => Value::Float(x * y),
                        (Value::Int(x), Value::Float(y)) => Value::Float(*x as f64 * y),
                        (Value::Float(x), Value::Int(y)) => Value::Float(x * *y as f64),
                        _ => {
                            let span = self.current_span();
                            self.op_multiply(a, b, span)?
                        }
                    };
                    self.stack.push(result);
                }
                Op::Divide => {
                    let (a, b) = self.pop2();
                    let result = match (&a, &b) {
                        (Value::Int(x), Value::Int(y)) => {
                            if *y == 0 {
                                return Err(RuntimeError::division_by_zero(self.current_span()));
                            }
                            match x.checked_div(*y) {
                                Some(result) => Value::Int(result),
                                None => {
                                    let span = self.current_span();
                                    return Err(
                                        crate::interpreter::executor::operators::int_overflow(
                                            *x, "/", *y, span,
                                        ),
                                    );
                                }
                            }
                        }
                        (Value::Float(x), Value::Float(y)) => {
                            if *y == 0.0 {
                                return Err(RuntimeError::division_by_zero(self.current_span()));
                            }
                            Value::Float(x / y)
                        }
                        _ => {
                            let span = self.current_span();
                            self.op_divide(a, b, span)?
                        }
                    };
                    self.stack.push(result);
                }
                Op::Modulo => {
                    let (a, b) = self.pop2();
                    let result = match (&a, &b) {
                        (Value::Int(x), Value::Int(y)) => {
                            if *y == 0 {
                                return Err(RuntimeError::division_by_zero(self.current_span()));
                            }
                            match x.checked_rem(*y) {
                                Some(result) => Value::Int(result),
                                None => {
                                    let span = self.current_span();
                                    return Err(
                                        crate::interpreter::executor::operators::int_overflow(
                                            *x, "%", *y, span,
                                        ),
                                    );
                                }
                            }
                        }
                        _ => {
                            let span = self.current_span();
                            self.op_modulo(a, b, span)?
                        }
                    };
                    self.stack.push(result);
                }
                Op::Negate => {
                    let val = self.pop();
                    match val {
                        Value::Int(n) => {
                            match n.checked_neg() {
                                Some(negated) => self.stack.push(Value::Int(negated)),
                                None => return Err(
                                    crate::interpreter::executor::operators::int_negate_overflow(
                                        n,
                                        self.current_span(),
                                    ),
                                ),
                            }
                        }
                        Value::Float(n) => self.stack.push(Value::Float(-n)),
                        Value::Decimal(d) => self.stack.push(Value::Decimal(
                            crate::interpreter::value::DecimalValue(-d.0, d.1),
                        )),
                        _ => {
                            return Err(RuntimeError::type_error(
                                format!("Cannot negate {}", val.type_name()),
                                self.current_span(),
                            ));
                        }
                    }
                }

                // --- Comparison (inlined fast paths for Int) ---
                Op::Equal => {
                    let (a, b) = self.pop2();
                    self.stack.push(Value::Bool(values_equal(&a, &b)));
                }
                Op::NotEqual => {
                    let (a, b) = self.pop2();
                    self.stack.push(Value::Bool(!values_equal(&a, &b)));
                }
                Op::Less => {
                    let (a, b) = self.pop2();
                    let result = match (&a, &b) {
                        (Value::Int(x), Value::Int(y)) => *x < *y,
                        (Value::Float(x), Value::Float(y)) => *x < *y,
                        // Strings order by bytes, as `compare_ordered` does,
                        // without the call or the span it is handed.
                        (Value::String(x), Value::String(y)) => x.as_str() < y.as_str(),
                        _ => {
                            let span = self.current_span();
                            self.op_compare_less(&a, &b, span)?
                        }
                    };
                    self.stack.push(Value::Bool(result));
                }
                Op::LessEqual => {
                    let (a, b) = self.pop2();
                    let result = match (&a, &b) {
                        (Value::Int(x), Value::Int(y)) => *x <= *y,
                        (Value::Float(x), Value::Float(y)) => *x <= *y,
                        (Value::String(x), Value::String(y)) => x.as_str() <= y.as_str(),
                        _ => {
                            let span = self.current_span();
                            self.op_compare_less_equal(&a, &b, span)?
                        }
                    };
                    self.stack.push(Value::Bool(result));
                }
                Op::Greater => {
                    let (a, b) = self.pop2();
                    let result = match (&a, &b) {
                        (Value::Int(x), Value::Int(y)) => *x > *y,
                        (Value::Float(x), Value::Float(y)) => *x > *y,
                        (Value::String(x), Value::String(y)) => x.as_str() > y.as_str(),
                        _ => {
                            let span = self.current_span();
                            // `a > b` is evaluated as `b < a` to reuse one
                            // comparator. Report the operands in source order,
                            // or the message names them backwards: `[1] > 1`
                            // said "Cannot compare int and array".
                            self.op_compare_less(&b, &a, span).map_err(|_| {
                                RuntimeError::type_error(
                                    format!(
                                        "Cannot compare {} and {}",
                                        a.type_name(),
                                        b.type_name()
                                    ),
                                    span,
                                )
                            })?
                        }
                    };
                    self.stack.push(Value::Bool(result));
                }
                Op::GreaterEqual => {
                    let (a, b) = self.pop2();
                    let result = match (&a, &b) {
                        (Value::Int(x), Value::Int(y)) => *x >= *y,
                        (Value::Float(x), Value::Float(y)) => *x >= *y,
                        (Value::String(x), Value::String(y)) => x.as_str() >= y.as_str(),
                        _ => {
                            let span = self.current_span();
                            // Same swap as `>` above: report source order.
                            self.op_compare_less_equal(&b, &a, span).map_err(|_| {
                                RuntimeError::type_error(
                                    format!(
                                        "Cannot compare {} and {}",
                                        a.type_name(),
                                        b.type_name()
                                    ),
                                    span,
                                )
                            })?
                        }
                    };
                    self.stack.push(Value::Bool(result));
                }
                Op::LessLocalConst(slot, const_idx)
                | Op::LessEqualLocalConst(slot, const_idx)
                | Op::GreaterLocalConst(slot, const_idx)
                | Op::GreaterEqualLocalConst(slot, const_idx) => {
                    // Exactly `GetLocal(slot), Constant(const_idx), <cmp>`: the
                    // same inline cases, then the same comparator and error.
                    let base = self.frames.last().unwrap().stack_base;
                    let a = self.stack[base + slot as usize].clone();
                    let b = {
                        let frame = self.frames.last().unwrap();
                        constant_to_value(&frame.closure.proto.chunk.constants[const_idx as usize])
                    };
                    let result = self.compare_fused(op, &a, &b)?;
                    self.stack.push(Value::Bool(result));
                }

                Op::Not => {
                    let val = self.stack.last().unwrap();
                    let truthy = val.is_truthy();
                    *self.stack.last_mut().unwrap() = Value::Bool(!truthy);
                }

                // --- Control flow ---
                Op::Jump(offset) => {
                    self.frames.last_mut().unwrap().ip += offset as usize;
                }
                Op::JumpIfFalse(offset) => {
                    let val = self.stack.pop().unwrap();
                    if !val.is_truthy() {
                        self.frames.last_mut().unwrap().ip += offset as usize;
                    }
                }
                Op::JumpIfTrue(offset) => {
                    let val = self.stack.pop().unwrap();
                    if val.is_truthy() {
                        self.frames.last_mut().unwrap().ip += offset as usize;
                    }
                }
                Op::Loop(offset) => {
                    // The one instruction a program cannot avoid to run
                    // forever, so it is where the handler budget is enforced.
                    // Reads the clock only every few thousand iterations.
                    if crate::interpreter::deadline::expired() {
                        return Err(RuntimeError::General {
                            message: crate::interpreter::deadline::timeout_message(),
                            span: self.current_span(),
                        });
                    }
                    self.frames.last_mut().unwrap().ip -= offset as usize;
                }
                Op::JumpIfFalseNoPop(offset) => {
                    if !self.stack.last().unwrap().is_truthy() {
                        self.frames.last_mut().unwrap().ip += offset as usize;
                    }
                }
                Op::JumpIfTrueNoPop(offset) => {
                    if self.stack.last().unwrap().is_truthy() {
                        self.frames.last_mut().unwrap().ip += offset as usize;
                    }
                }
                Op::NullishJump(offset) => {
                    if !matches!(self.stack.last().unwrap(), Value::Null) {
                        self.frames.last_mut().unwrap().ip += offset as usize;
                    }
                }

                // --- Functions ---
                Op::Call(argc) => {
                    let argc = argc as usize;
                    let callee_idx = self.stack.len() - 1 - argc;
                    // Inline VmClosure fast path: no eager span (it's only
                    // needed on the cold arity-error branch) and no
                    // call_value double dispatch — this is the hot path for
                    // every compiled function call (e.g. recursion).
                    // A `Function` with its VM closure cached (see the fast
                    // tier's `Op::Call`) is called as that closure.
                    let (closure, from_function) = match &self.stack[callee_idx] {
                        Value::VmClosure(closure) => (Some(closure.clone()), false),
                        Value::Function(func) if !self.tree_walk_functions => {
                            (super::vm_calls::cached_function_closure(func), true)
                        }
                        _ => (None, false),
                    };
                    if let Some(closure) = closure {
                        if from_function {
                            self.stack[callee_idx] = Value::VmClosure(closure.clone());
                        }
                        if closure.proto.kernel.is_some() && self.try_kernel(&closure, argc) {
                            continue;
                        }
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
                        let call_span = self.current_span();
                        self.ensure_call_depth(&closure.proto, call_span)?;
                        self.frames.push(CallFrame::new(
                            closure,
                            stack_base,
                            self.iter_stack.len(),
                            None,
                            positional_supplied_mask(argc),
                        ));
                        self.debug_entered();
                    } else {
                        let span = self.current_span();
                        self.call_value(argc, span)?;
                    }
                }
                Op::CallGlobal(name_idx, argc) => {
                    // Combined GetGlobal + Call: lookup global, push, and call in one step
                    let val = {
                        let frame = self.frames.last().unwrap();
                        let name = match &frame.closure.proto.chunk.constants[name_idx as usize] {
                            Constant::String(s) => s.as_ref(),
                            _ => "",
                        };
                        self.globals.get(name).cloned()
                    };
                    match val {
                        Some(func) => {
                            // Insert the function below the arguments
                            let insert_pos = self.stack.len() - argc as usize;
                            self.stack.insert(insert_pos, func);
                            let span = self.current_span();
                            self.call_value(argc as usize, span)?;
                        }
                        None => {
                            let name = self.read_string_constant_owned(name_idx);
                            let Some(func) = self.namespaced(&name) else {
                                return Err(self.undefined_global(name));
                            };
                            let insert_pos = self.stack.len() - argc as usize;
                            self.stack.insert(insert_pos, func);
                            let span = self.current_span();
                            self.call_value(argc as usize, span)?;
                        }
                    }
                }
                Op::SelfOrGlobal(name_idx) => {
                    // The pool's own `SoliStr`: inline for a short name, a
                    // refcount bump for a long one — no allocation.
                    let name = self.read_string_constant(name_idx);
                    let name = name.as_str();
                    let this = self.pop();
                    if let Some(value) = self.global_or_namespaced(name) {
                        // A zero-argument def read bare is called, as
                        // `Op::AutoInvoke` does for a global known at
                        // compile time.
                        let call = auto_invocable(&value);
                        self.push(value);
                        if call {
                            let span = self.current_span();
                            self.call_value(0, span)?;
                        }
                    } else if Self::has_instance_method(&this, name) {
                        let span = self.current_span();
                        let value = self.op_get_property_member(&this, name, span)?;
                        self.push(value);
                    } else {
                        return Err(self.undefined_global(name.to_string()));
                    }
                }
                Op::MarkPrivate(name_idx) => {
                    let name = self.read_string_constant_owned(name_idx);
                    if let Some(Value::Class(class)) = self.stack.last() {
                        crate::interpreter::domain::note_private_class(class, &name);
                        class.private_members.borrow_mut().insert(name);
                        crate::interpreter::value::note_restricted_methods_declared();
                    }
                }
                Op::MarkProtected(name_idx) => {
                    let name = self.read_string_constant_owned(name_idx);
                    if let Some(Value::Class(class)) = self.stack.last() {
                        class.protected_members.borrow_mut().insert(name);
                        crate::interpreter::value::note_restricted_methods_declared();
                    }
                }
                Op::SelfOrGlobalCallee(name_idx) => {
                    // Decide once, before the arguments run, and leave the
                    // decision on the stack for `CallSelfOrGlobal`: it neither
                    // probes globals nor walks the class chain again.
                    let name = self.read_string_constant(name_idx);
                    let name = name.as_str();
                    if let Some(value) = self.global_or_namespaced(name) {
                        let top = self.stack.len() - 1;
                        self.stack[top] = value;
                        // `Null`: call the global.
                        self.push(Value::Null);
                    } else {
                        let found = match self.stack.last() {
                            Some(Value::Instance(inst)) => {
                                let class = inst.borrow().class.clone();
                                // The nearest definition, level by level: a
                                // subclass override not compiled yet beats an
                                // ancestor's compiled method.
                                match class.find_instance_method_level(name) {
                                    // The class that defines the compiled
                                    // method: the call reads it from there.
                                    Some((
                                        crate::interpreter::value::InstanceMethodDef::Compiled(_),
                                        defining,
                                    )) => Some(Value::Class(defining)),
                                    // `true`: a tree-walker method, dispatched by name.
                                    Some(_) => Some(Value::Bool(true)),
                                    // An inherited native (`save` in a
                                    // `< Model` method): dispatched by name too.
                                    None if class.find_native_method(name).is_some() => {
                                        Some(Value::Bool(true))
                                    }
                                    None => None,
                                }
                            }
                            _ => None,
                        };
                        let Some(marker) = found else {
                            return Err(self.undefined_global(name.to_string()));
                        };
                        self.push(marker);
                    }
                }
                Op::CallSelfOrGlobal(name_idx, argc) => {
                    let argc = argc as usize;
                    // `[callee, marker, args…]`: drop the marker so the
                    // callee sits right below the arguments.
                    let marker = self.stack.remove(self.stack.len() - 1 - argc);
                    match marker {
                        Value::Null => {
                            let span = self.current_span();
                            self.call_value(argc, span)?;
                        }
                        marker => {
                            // `this` is the receiver: a call on self, so a
                            // private method is allowed.
                            let name = self.read_string_constant(name_idx);
                            let name = name.as_str();
                            let compiled = match &marker {
                                Value::Class(defining) => defining
                                    .vm_methods
                                    .borrow()
                                    .get(name)
                                    .cloned()
                                    .map(|closure| (closure, defining.clone())),
                                _ => None,
                            };
                            match compiled {
                                Some((closure, defining)) => {
                                    self.push_method_frame(closure, argc, Some(defining))?
                                }
                                None => {
                                    let receiver_idx = self.stack.len() - 1 - argc;
                                    self.call_method_slow_path(receiver_idx, argc, name)?;
                                }
                            }
                        }
                    }
                }
                Op::CallMethod(name_idx, argc, block) => {
                    let argc = argc as usize;
                    let receiver_idx = self.stack.len() - 1 - argc;
                    if matches!(
                        &self.stack[receiver_idx],
                        Value::Future(_) | Value::Deferred(_)
                    ) {
                        let span = self.current_span();
                        let resolved = Self::force_lazy_receiver(&self.stack[receiver_idx], span)?;
                        self.stack[receiver_idx] = resolved;
                    }

                    // Borrow method name from constant pool (no clone)
                    let name: *const str = {
                        let frame = self.frames.last().unwrap();
                        match &frame.closure.proto.chunk.constants[name_idx as usize] {
                            Constant::String(s) => s.as_ref() as *const str,
                            _ => "" as *const str,
                        }
                    };
                    // SAFETY: name points into the constant pool which is alive for
                    // the entire execution of this frame. We never mutate constants.
                    let name: &str = unsafe { &*name };

                    // A method `define_method` gave a primitive type. One load
                    // of the whole mask when none exist (the common case).
                    if crate::interpreter::executor::calls::user_methods::user_method_flags() != 0
                        && self.call_primitive_user_method(receiver_idx, argc, name, block)?
                    {
                        continue;
                    }

                    // Fast path: dispatch on receiver type without cloning
                    match &self.stack[receiver_idx] {
                        // No inline name matches here: every name that had one
                        // (`len`, `upcase`, `first`, `blank?`, …) has a method
                        // id, so the compiler emits `CallMethodById` for it and
                        // this arm only ever sees names without one.
                        Value::String(_) => {
                            // Borrow string and args from the stack
                            let result = {
                                let s = match &self.stack[receiver_idx] {
                                    Value::String(s) => s,
                                    _ => unreachable!(),
                                };
                                let args = &self.stack[receiver_idx + 1..receiver_idx + 1 + argc];
                                let span = self.current_span();
                                self.vm_call_string_method(s, name, args, span)?
                            };
                            self.stack.truncate(receiver_idx);
                            self.stack.push(result);
                        }
                        Value::Instance(_) | Value::Class(_) | Value::VmClosure(_) => {
                            if crate::interpreter::value::restricted_methods_declared() {
                                self.check_private_receiver(receiver_idx, name)?;
                            }
                            self.call_method_slow_path(receiver_idx, argc, name)?;
                        }
                        Value::Array(_) => {
                            // An iterator method with a compiled callback runs
                            // its loop in this dispatch loop; checked first so
                            // it skips the name matching below. The callback
                            // comes first: `reduce`'s seed follows it.
                            if (argc == 1 || argc == 2)
                                && matches!(self.stack[receiver_idx + 1], Value::VmClosure(_))
                                && self.try_start_callback_loop(receiver_idx, argc, name)?
                            {
                                continue;
                            }
                            let arr = match &self.stack[receiver_idx] {
                                Value::Array(a) => a.clone(),
                                _ => unreachable!(),
                            };
                            let args: Vec<Value> =
                                self.stack[receiver_idx + 1..receiver_idx + 1 + argc].to_vec();
                            let span = self.current_span();
                            self.stack.truncate(receiver_idx);
                            let result = self.vm_call_array_method(&arr, name, &args, span)?;
                            self.stack.push(result);
                        }
                        Value::Hash(_) => {
                            // An iterator method with a compiled callback runs
                            // its loop in this dispatch loop; checked first so
                            // it skips the name matching below.
                            if argc == 1
                                && matches!(self.stack[receiver_idx + 1], Value::VmClosure(_))
                                && self.try_start_callback_loop(receiver_idx, argc, name)?
                            {
                                continue;
                            }
                            if let Some(result) =
                                self.hash_method_on_stack(receiver_idx, argc, name)
                            {
                                let result = result?;
                                self.stack.truncate(receiver_idx);
                                self.stack.push(result);
                            } else {
                                let hash = match &self.stack[receiver_idx] {
                                    Value::Hash(h) => h.clone(),
                                    _ => unreachable!(),
                                };
                                let args: Vec<Value> =
                                    self.stack[receiver_idx + 1..receiver_idx + 1 + argc].to_vec();
                                let span = self.current_span();
                                self.stack.truncate(receiver_idx);
                                // `hash_method_on_stack` just tried the borrowed tier.
                                let result = self
                                    .vm_call_hash_method_past_borrowed(&hash, name, &args, span)?;
                                self.stack.push(result);
                            }
                        }
                        Value::Int(_)
                        | Value::Float(_)
                        | Value::Bool(_)
                        | Value::Null
                        | Value::Decimal(_)
                        | Value::DateTime(_, _) => {
                            // `n.times { … }`, `n.upto(m) { … }`: a compiled
                            // callback, the last argument, runs its loop in
                            // this dispatch loop too.
                            if matches!(self.stack[receiver_idx], Value::Int(_))
                                && (argc == 1 || argc == 2)
                                && matches!(self.stack[receiver_idx + argc], Value::VmClosure(_))
                                && self.try_start_callback_loop(receiver_idx, argc, name)?
                            {
                                continue;
                            }
                            let receiver = self.stack[receiver_idx].clone();
                            let args: Vec<Value> =
                                self.stack[receiver_idx + 1..receiver_idx + 1 + argc].to_vec();
                            let span = self.current_span();
                            self.stack.truncate(receiver_idx);
                            let result =
                                self.vm_call_primitive_method(&receiver, name, &args, span)?;
                            self.stack.push(result);
                        }
                        _ => {
                            self.call_method_slow_path(receiver_idx, argc, name)?;
                        }
                    }
                }
                Op::CallMethodById(name_idx, argc, method_id, block) => {
                    let argc = argc as usize;
                    let receiver_idx = self.stack.len() - 1 - argc;
                    if matches!(
                        &self.stack[receiver_idx],
                        Value::Future(_) | Value::Deferred(_)
                    ) {
                        let span = self.current_span();
                        let resolved = Self::force_lazy_receiver(&self.stack[receiver_idx], span)?;
                        self.stack[receiver_idx] = resolved;
                    }
                    // The id tables below are builtins only; a primitive user
                    // method has to be looked for first, under the same rule
                    // as `CallMethod`. Skipped in one load when none exist.
                    if crate::interpreter::executor::calls::user_methods::user_method_flags() != 0 {
                        let name: *const str = {
                            let frame = self.frames.last().unwrap();
                            match &frame.closure.proto.chunk.constants[name_idx as usize] {
                                Constant::String(s) => s.as_ref() as *const str,
                                _ => "" as *const str,
                            }
                        };
                        // SAFETY: as in `CallMethod` — the constant pool outlives
                        // this frame's execution and is never mutated.
                        let name: &str = unsafe { &*name };
                        if self.call_primitive_user_method(receiver_idx, argc, name, block)? {
                            continue;
                        }
                    }
                    // Fast path: zero-arg methods on primitives — pure integer dispatch
                    if argc == 0 {
                        let result = match &self.stack[receiver_idx] {
                            Value::String(s) => {
                                super::method_table::string_method_zero_arg(s, method_id)
                            }
                            Value::Array(a) => {
                                super::method_table::array_method_zero_arg(a, method_id)
                            }
                            Value::Hash(h) => {
                                super::method_table::hash_method_zero_arg(h, method_id)
                            }
                            _ => None,
                        };
                        if let Some(result) = result {
                            self.stack[receiver_idx] = result;
                            // No args to truncate since argc == 0
                            continue;
                        }
                    }
                    if argc == 1 {
                        let result = match &self.stack[receiver_idx] {
                            Value::String(s) => super::method_table::string_method_one_arg(
                                s.as_ref(),
                                method_id,
                                &self.stack[receiver_idx + 1],
                                self.current_span(),
                            ),
                            Value::Hash(h) => super::method_table::hash_method_one_arg(
                                h,
                                method_id,
                                &self.stack[receiver_idx + 1],
                                self.current_span(),
                            ),
                            Value::Array(a) => super::method_table::array_method_one_arg(
                                a,
                                method_id,
                                &self.stack[receiver_idx + 1],
                            )
                            .map(Ok),
                            _ => None,
                        };
                        if let Some(result) = result {
                            let result = result?;
                            self.stack.truncate(receiver_idx);
                            self.stack.push(result);
                            continue;
                        }
                    }
                    if argc == 2 {
                        let result = match &self.stack[receiver_idx] {
                            Value::String(s) => super::method_table::string_method_two_arg(
                                s.as_ref(),
                                method_id,
                                &self.stack[receiver_idx + 1],
                                &self.stack[receiver_idx + 2],
                                self.current_span(),
                            ),
                            Value::Hash(h) => super::method_table::hash_method_two_arg(
                                h,
                                method_id,
                                &self.stack[receiver_idx + 1],
                                &self.stack[receiver_idx + 2],
                                self.current_span(),
                            ),
                            _ => None,
                        };
                        if let Some(result) = result {
                            let result = result?;
                            self.stack.truncate(receiver_idx);
                            self.stack.push(result);
                            continue;
                        }
                    }

                    // Medium path: known method ID with args — use string dispatch
                    // (still avoids the name clone since we borrow from constants)
                    let name: *const str = {
                        let frame = self.frames.last().unwrap();
                        match &frame.closure.proto.chunk.constants[name_idx as usize] {
                            Constant::String(s) => s.as_ref() as *const str,
                            _ => "" as *const str,
                        }
                    };
                    let name: &str = unsafe { &*name };

                    match &self.stack[receiver_idx] {
                        Value::String(_) => {
                            let result = {
                                let s = match &self.stack[receiver_idx] {
                                    Value::String(s) => s,
                                    _ => unreachable!(),
                                };
                                let args = &self.stack[receiver_idx + 1..receiver_idx + 1 + argc];
                                let span = self.current_span();
                                self.vm_call_string_method(s, name, args, span)?
                            };
                            self.stack.truncate(receiver_idx);
                            self.stack.push(result);
                        }
                        Value::Array(_) => {
                            // `xs.count { … }`, the block method with an id,
                            // takes the in-VM loop as `CallMethod` does.
                            if argc == 1
                                && matches!(self.stack[receiver_idx + 1], Value::VmClosure(_))
                                && self.try_start_callback_loop(receiver_idx, argc, name)?
                            {
                                continue;
                            }
                            let arr = match &self.stack[receiver_idx] {
                                Value::Array(a) => a.clone(),
                                _ => unreachable!(),
                            };
                            let args: Vec<Value> =
                                self.stack[receiver_idx + 1..receiver_idx + 1 + argc].to_vec();
                            let span = self.current_span();
                            self.stack.truncate(receiver_idx);
                            let result = self.vm_call_array_method(&arr, name, &args, span)?;
                            self.stack.push(result);
                        }
                        Value::Hash(_) => {
                            if let Some(result) =
                                self.hash_method_on_stack(receiver_idx, argc, name)
                            {
                                let result = result?;
                                self.stack.truncate(receiver_idx);
                                self.stack.push(result);
                                continue;
                            }
                            let hash = match &self.stack[receiver_idx] {
                                Value::Hash(h) => h.clone(),
                                _ => unreachable!(),
                            };
                            let args: Vec<Value> =
                                self.stack[receiver_idx + 1..receiver_idx + 1 + argc].to_vec();
                            let span = self.current_span();
                            self.stack.truncate(receiver_idx);
                            // `hash_method_on_stack` just tried the borrowed tier.
                            let result =
                                self.vm_call_hash_method_past_borrowed(&hash, name, &args, span)?;
                            self.stack.push(result);
                        }
                        Value::Int(_)
                        | Value::Float(_)
                        | Value::Bool(_)
                        | Value::Null
                        | Value::Decimal(_)
                        | Value::DateTime(_, _) => {
                            let receiver = self.stack[receiver_idx].clone();
                            let args: Vec<Value> =
                                self.stack[receiver_idx + 1..receiver_idx + 1 + argc].to_vec();
                            let span = self.current_span();
                            self.stack.truncate(receiver_idx);
                            let result =
                                self.vm_call_primitive_method(&receiver, name, &args, span)?;
                            self.stack.push(result);
                        }
                        _ => {
                            // Class instances, closures: fall back to property dispatch
                            if crate::interpreter::value::restricted_methods_declared() {
                                self.check_private_receiver(receiver_idx, name)?;
                            }
                            self.call_method_slow_path(receiver_idx, argc, name)?;
                        }
                    }
                }
                Op::HashGetConst(key_idx) => {
                    let receiver = self.pop();
                    let key: *const str = {
                        let frame = self.frames.last().unwrap();
                        match &frame.closure.proto.chunk.constants[key_idx as usize] {
                            Constant::String(s) => s.as_ref() as *const str,
                            _ => "" as *const str,
                        }
                    };
                    let key: &str = unsafe { &*key };
                    match receiver {
                        Value::Hash(hash) => {
                            let value = hash
                                .borrow()
                                .get(&StrKey(key))
                                .cloned()
                                .unwrap_or(Value::Null);
                            self.push(value);
                        }
                        other => {
                            // Same reasoning as `HashGetGlobalConst`: the string
                            // literal made this look like a hash access, but an
                            // array or string reaching here still has a `get` —
                            // let normal dispatch produce the message that says
                            // what is actually wrong.
                            let receiver = other.clone();
                            let arg = Value::String(crate::interpreter::value::SoliStr::from(key));
                            let span = self.current_span();
                            let value = match &receiver {
                                Value::Array(arr) => {
                                    self.vm_call_array_method(&arr.clone(), "get", &[arg], span)?
                                }
                                Value::String(st) => {
                                    let st = st.clone();
                                    self.vm_call_string_method(&st, "get", &[arg], span)?
                                }
                                // Un modèle se lit aussi par clé — `user["_key"]`,
                                // `current_user()["_key"]`. `GetIndex` le sait ; ce
                                // raccourci ne le savait pas, et répondait
                                // « Cannot access property 'get' on User » dès
                                // qu'un gestionnaire tournait sur le VM.
                                Value::Instance(_) => self.op_get_index(&receiver, &arg, span)?,
                                _ => {
                                    return Err(RuntimeError::no_such_property(
                                        receiver.type_name(),
                                        "get".to_string(),
                                        span,
                                    ))
                                }
                            };
                            self.push(value);
                        }
                    }
                }
                Op::HashHasKeyConst(key_idx) => {
                    let receiver = self.pop();
                    let key: *const str = {
                        let frame = self.frames.last().unwrap();
                        match &frame.closure.proto.chunk.constants[key_idx as usize] {
                            Constant::String(s) => s.as_ref() as *const str,
                            _ => "" as *const str,
                        }
                    };
                    let key: &str = unsafe { &*key };
                    match receiver {
                        Value::Hash(hash) => {
                            self.push(Value::Bool(hash.borrow().contains_key(&StrKey(key))));
                        }
                        other => {
                            return Err(RuntimeError::no_such_property(
                                other.type_name(),
                                "has_key".to_string(),
                                self.current_span(),
                            ));
                        }
                    }
                }
                Op::HashDeleteConst(key_idx) => {
                    let receiver = self.pop();
                    let key: *const str = {
                        let frame = self.frames.last().unwrap();
                        match &frame.closure.proto.chunk.constants[key_idx as usize] {
                            Constant::String(s) => s.as_ref() as *const str,
                            _ => "" as *const str,
                        }
                    };
                    let key: &str = unsafe { &*key };
                    match receiver {
                        Value::Hash(hash) => {
                            let value = hash
                                .borrow_mut()
                                .shift_remove(&StrKey(key))
                                .unwrap_or(Value::Null);
                            self.push(value);
                        }
                        other => {
                            return Err(RuntimeError::no_such_property(
                                other.type_name(),
                                "delete".to_string(),
                                self.current_span(),
                            ));
                        }
                    }
                }
                Op::HashSetConst(key_idx) => {
                    let value = self.pop();
                    let receiver = self.pop();
                    // SAFETY: the constant outlives the arm (see `string_constant_key`).
                    let key_str: &crate::interpreter::value::SoliStr =
                        unsafe { &*self.string_constant_key(key_idx) };
                    let key: &str = key_str.as_str();
                    match receiver {
                        Value::Hash(hash) => {
                            // A miss appends the pool's key, not a copy of it.
                            hash.borrow_mut().set_str(key_str, value.clone());
                            // Same rule as `Op::SetIndex`: the expression's value
                            // is the assigned value, not null.
                            self.push(value);
                        }
                        other => {
                            self.set_const_key_fallback(&other, key, value.clone())?;
                            self.push(value);
                        }
                    }
                }
                Op::HashGetLocalConst(slot, key_idx) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let key: *const str = {
                        let frame = self.frames.last().unwrap();
                        match &frame.closure.proto.chunk.constants[key_idx as usize] {
                            Constant::String(s) => s.as_ref() as *const str,
                            _ => "" as *const str,
                        }
                    };
                    let key: &str = unsafe { &*key };
                    // Hash hit stays borrow-only — no receiver clone on the hot
                    // path, which is the whole point of this super-instruction.
                    let hit = match &self.stack[base + slot as usize] {
                        Value::Hash(hash) => Some(
                            hash.borrow()
                                .get(&StrKey(key))
                                .cloned()
                                .unwrap_or(Value::Null),
                        ),
                        _ => None,
                    };
                    let result = match hit {
                        Some(value) => value,
                        // Same reasoning as `HashGetGlobalConst`: the string
                        // literal made this look like a hash access, but an array
                        // or string reaching here still has a `get` — let normal
                        // dispatch produce the message that says what is actually
                        // wrong.
                        None => {
                            let receiver = self.stack[base + slot as usize].clone();
                            let arg = Value::String(crate::interpreter::value::SoliStr::from(key));
                            let span = self.current_span();
                            match &receiver {
                                Value::Array(arr) => {
                                    self.vm_call_array_method(&arr.clone(), "get", &[arg], span)?
                                }
                                Value::String(st) => {
                                    let st = st.clone();
                                    self.vm_call_string_method(&st, "get", &[arg], span)?
                                }
                                // Un modèle se lit aussi par clé — `user["_key"]`,
                                // `current_user()["_key"]`. `GetIndex` le sait ; ce
                                // raccourci ne le savait pas, et répondait
                                // « Cannot access property 'get' on User » dès
                                // qu'un gestionnaire tournait sur le VM.
                                Value::Instance(_) => self.op_get_index(&receiver, &arg, span)?,
                                _ => {
                                    return Err(RuntimeError::no_such_property(
                                        receiver.type_name(),
                                        "get".to_string(),
                                        span,
                                    ))
                                }
                            }
                        }
                    };
                    self.push(result);
                }
                Op::HashGetLocalConst2(slot, k1, k2) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let (p1, p2) = self.two_const_str_ptrs(k1, k2);
                    let receiver = self.stack[base + slot as usize].clone();
                    let result =
                        self.hash_get_nested(&receiver, unsafe { &*p1 }, unsafe { &*p2 })?;
                    self.push(result);
                }
                Op::HashGetGlobalConst2(global_idx, k1, k2) => {
                    let gname: *const str = {
                        let frame = self.frames.last().unwrap();
                        match &frame.closure.proto.chunk.constants[global_idx as usize] {
                            Constant::String(s) => s.as_ref() as *const str,
                            _ => "" as *const str,
                        }
                    };
                    let (p1, p2) = self.two_const_str_ptrs(k1, k2);
                    let gname: &str = unsafe { &*gname };
                    let receiver = self.globals.get(gname).cloned();
                    let result = match receiver {
                        Some(v) => self.hash_get_nested(&v, unsafe { &*p1 }, unsafe { &*p2 })?,
                        None => return Err(self.undefined_global(gname.to_string())),
                    };
                    self.push(result);
                }
                Op::HashGetConst2(k1, k2) => {
                    let receiver = self.pop();
                    let (p1, p2) = self.two_const_str_ptrs(k1, k2);
                    let result =
                        self.hash_get_nested(&receiver, unsafe { &*p1 }, unsafe { &*p2 })?;
                    self.push(result);
                }
                Op::HashHasKeyLocalConst(slot, key_idx) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let key: *const str = {
                        let frame = self.frames.last().unwrap();
                        match &frame.closure.proto.chunk.constants[key_idx as usize] {
                            Constant::String(s) => s.as_ref() as *const str,
                            _ => "" as *const str,
                        }
                    };
                    let key: &str = unsafe { &*key };
                    let result = match &self.stack[base + slot as usize] {
                        Value::Hash(hash) => Value::Bool(hash.borrow().contains_key(&StrKey(key))),
                        other => {
                            return Err(RuntimeError::no_such_property(
                                other.type_name(),
                                "has_key".to_string(),
                                self.current_span(),
                            ));
                        }
                    };
                    self.push(result);
                }
                Op::HashDeleteLocalConst(slot, key_idx) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let key: *const str = {
                        let frame = self.frames.last().unwrap();
                        match &frame.closure.proto.chunk.constants[key_idx as usize] {
                            Constant::String(s) => s.as_ref() as *const str,
                            _ => "" as *const str,
                        }
                    };
                    let key: &str = unsafe { &*key };
                    let result = match &self.stack[base + slot as usize] {
                        Value::Hash(hash) => hash
                            .borrow_mut()
                            .shift_remove(&StrKey(key))
                            .unwrap_or(Value::Null),
                        other => {
                            return Err(RuntimeError::no_such_property(
                                other.type_name(),
                                "delete".to_string(),
                                self.current_span(),
                            ));
                        }
                    };
                    self.push(result);
                }
                Op::HashSetLocalConst(slot, key_idx) => {
                    let value = self.pop();
                    let base = self.frames.last().unwrap().stack_base;
                    // SAFETY: the constant outlives the arm (see `string_constant_key`).
                    let key_str: &crate::interpreter::value::SoliStr =
                        unsafe { &*self.string_constant_key(key_idx) };
                    let key: &str = key_str.as_str();
                    match &self.stack[base + slot as usize] {
                        Value::Hash(hash) => {
                            // A miss appends the pool's key, not a copy of it.
                            hash.borrow_mut().set_str(key_str, value.clone());
                        }
                        other => {
                            let other = other.clone();
                            self.set_const_key_fallback(&other, key, value.clone())?;
                        }
                    }
                    // The expression's value is the assigned value, matching the
                    // tree-walker; pushing null made `h["k"] = v` as a
                    // function's last statement return null under `--vm`.
                    self.push(value);
                }
                Op::HashGetGlobalConst(global_idx, key_idx) => {
                    let global_name: *const str = {
                        let frame = self.frames.last().unwrap();
                        match &frame.closure.proto.chunk.constants[global_idx as usize] {
                            Constant::String(s) => s.as_ref() as *const str,
                            _ => "" as *const str,
                        }
                    };
                    let key: *const str = {
                        let frame = self.frames.last().unwrap();
                        match &frame.closure.proto.chunk.constants[key_idx as usize] {
                            Constant::String(s) => s.as_ref() as *const str,
                            _ => "" as *const str,
                        }
                    };
                    let global_name: &str = unsafe { &*global_name };
                    let key: &str = unsafe { &*key };
                    match self.globals.get(global_name) {
                        Some(Value::Hash(hash)) => {
                            let value = hash
                                .borrow()
                                .get(&StrKey(key))
                                .cloned()
                                .unwrap_or(Value::Null);
                            self.push(value);
                        }
                        Some(other) => {
                            // This super-instruction assumes a hash, because the
                            // key is a string literal. When the receiver turns
                            // out to be something else, hand the call back to
                            // normal dispatch instead of claiming the method does
                            // not exist: an array *does* have `get`, it just
                            // wants an integer index, and saying otherwise sends
                            // the reader hunting for the wrong bug.
                            let receiver = other.clone();
                            let arg = Value::String(crate::interpreter::value::SoliStr::from(key));
                            let span = self.current_span();
                            let value = match &receiver {
                                Value::Array(arr) => {
                                    self.vm_call_array_method(&arr.clone(), "get", &[arg], span)?
                                }
                                Value::String(st) => {
                                    let st = st.clone();
                                    self.vm_call_string_method(&st, "get", &[arg], span)?
                                }
                                // Un modèle se lit aussi par clé — `user["_key"]`,
                                // `current_user()["_key"]`. `GetIndex` le sait ; ce
                                // raccourci ne le savait pas, et répondait
                                // « Cannot access property 'get' on User » dès
                                // qu'un gestionnaire tournait sur le VM.
                                Value::Instance(_) => self.op_get_index(&receiver, &arg, span)?,
                                _ => {
                                    return Err(RuntimeError::no_such_property(
                                        receiver.type_name(),
                                        "get".to_string(),
                                        span,
                                    ))
                                }
                            };
                            self.push(value);
                        }
                        None => {
                            return Err(self.undefined_global(global_name.to_string()));
                        }
                    }
                }
                Op::HashHasKeyGlobalConst(global_idx, key_idx) => {
                    let global_name: *const str = {
                        let frame = self.frames.last().unwrap();
                        match &frame.closure.proto.chunk.constants[global_idx as usize] {
                            Constant::String(s) => s.as_ref() as *const str,
                            _ => "" as *const str,
                        }
                    };
                    let key: *const str = {
                        let frame = self.frames.last().unwrap();
                        match &frame.closure.proto.chunk.constants[key_idx as usize] {
                            Constant::String(s) => s.as_ref() as *const str,
                            _ => "" as *const str,
                        }
                    };
                    let global_name: &str = unsafe { &*global_name };
                    let key: &str = unsafe { &*key };
                    let result = match self.globals.get(global_name) {
                        Some(Value::Hash(hash)) => {
                            Value::Bool(hash.borrow().contains_key(&StrKey(key)))
                        }
                        Some(other) => {
                            return Err(RuntimeError::no_such_property(
                                other.type_name(),
                                "has_key".to_string(),
                                self.current_span(),
                            ));
                        }
                        None => {
                            return Err(self.undefined_global(global_name.to_string()));
                        }
                    };
                    self.push(result);
                }
                Op::HashDeleteGlobalConst(global_idx, key_idx) => {
                    let global_name: *const str = {
                        let frame = self.frames.last().unwrap();
                        match &frame.closure.proto.chunk.constants[global_idx as usize] {
                            Constant::String(s) => s.as_ref() as *const str,
                            _ => "" as *const str,
                        }
                    };
                    let key: *const str = {
                        let frame = self.frames.last().unwrap();
                        match &frame.closure.proto.chunk.constants[key_idx as usize] {
                            Constant::String(s) => s.as_ref() as *const str,
                            _ => "" as *const str,
                        }
                    };
                    let global_name: &str = unsafe { &*global_name };
                    let key: &str = unsafe { &*key };
                    match self.globals.get(global_name) {
                        Some(Value::Hash(hash)) => {
                            let value = hash
                                .borrow_mut()
                                .shift_remove(&StrKey(key))
                                .unwrap_or(Value::Null);
                            self.push(value);
                        }
                        Some(other) => {
                            return Err(RuntimeError::no_such_property(
                                other.type_name(),
                                "delete".to_string(),
                                self.current_span(),
                            ));
                        }
                        None => {
                            return Err(self.undefined_global(global_name.to_string()));
                        }
                    }
                }
                Op::HashSetGlobalConst(global_idx, key_idx) => {
                    let value = self.pop();
                    let global_name: *const str = {
                        let frame = self.frames.last().unwrap();
                        match &frame.closure.proto.chunk.constants[global_idx as usize] {
                            Constant::String(s) => s.as_ref() as *const str,
                            _ => "" as *const str,
                        }
                    };
                    // SAFETY: the constant outlives the arm (see `string_constant_key`).
                    let key_str: &crate::interpreter::value::SoliStr =
                        unsafe { &*self.string_constant_key(key_idx) };
                    let global_name: &str = unsafe { &*global_name };
                    let key: &str = key_str.as_str();
                    match self.globals.get(global_name) {
                        Some(Value::Hash(hash)) => {
                            // A miss appends the pool's key, not a copy of it.
                            hash.borrow_mut().set_str(key_str, value.clone());
                        }
                        Some(other) => {
                            let other = other.clone();
                            self.set_const_key_fallback(&other, key, value.clone())?;
                        }
                        None => {
                            return Err(self.undefined_global(global_name.to_string()));
                        }
                    }
                    // The expression's value is the assigned value, matching the
                    // tree-walker and the other three hash-set opcodes.
                    self.push(value);
                }
                // `run_fast` makes closures; this is the same, for a frame
                // that reaches it here. See `vm_upvalues.rs`.
                Op::Closure(idx) => {
                    let frame = self.frames.last().unwrap();
                    let constant = &frame.closure.proto.chunk.constants[idx as usize];
                    if let Constant::Function(proto) = constant {
                        let proto = proto.clone();
                        let closure = self.make_closure(&proto);
                        self.stack.push(Value::VmClosure(closure));
                    }
                }
                Op::Return => {
                    let result = self.pop();
                    if self.check_return_types {
                        self.check_return_type(&result)?;
                    }
                    let frame = self.frames.pop().unwrap();
                    self.debug_left();

                    // Close the cells on this frame's slots — one comparison
                    // when none is open there, however many are open below.
                    if self.has_open_upvalues_from(frame.stack_base) {
                        self.close_upvalues(frame.stack_base);
                    }

                    // Restore the stack
                    self.stack.truncate(frame.stack_base);
                    // …and the iterator stack. A `return` from inside a `for`
                    // loop never reaches the `ForIter` that pops its iterator,
                    // so without this the callee's iterator stays on top and the
                    // caller's loop starts consuming it.
                    self.iter_stack.truncate(frame.iter_base);

                    // …and any `try` handlers this frame registered. A `return`
                    // from inside a `try` skips the `TryEnd` that pops the
                    // handler, so it outlived its frame with a `catch_ip`
                    // pointing into a chunk that is no longer running. The next
                    // throw with no newer handler above it matched that corpse
                    // and jumped the *current* frame's ip to an offset from a
                    // different function — observed as the exception vanishing
                    // entirely. Handlers are keyed by the frame depth at
                    // `TryBegin`, so anything deeper than the frame we just
                    // returned into belongs to a frame that is gone.
                    //
                    // Third stack with this exact bug (value, iterator, now
                    // handlers): every piece of per-frame state has to be
                    // unwound here, because `return` is the one exit that skips
                    // the instruction that would have cleaned it up.
                    let depth = self.frames.len();
                    while self
                        .exception_handlers
                        .last()
                        .is_some_and(|h| h.frame_depth > depth)
                    {
                        self.exception_handlers.pop();
                    }

                    match frame.on_return {
                        FrameReturn::Value => {}
                        FrameReturn::DrivesLoop => {
                            self.callback_returned(result)?;
                            continue;
                        }
                        // The value goes nowhere. At the edge of a nested run
                        // the run's value is what the frame found beneath it:
                        // the instance whose field defaults it ran.
                        FrameReturn::Discard => {
                            if self.frames.len() <= self.return_depth {
                                return Ok(self.pop());
                            }
                            continue;
                        }
                    }

                    if self.frames.len() <= self.return_depth {
                        return Ok(result);
                    }

                    self.stack.push(result);
                }

                // --- Collections ---
                Op::Array(n) => {
                    let array = self.array_from_stack(n as usize);
                    self.stack.push(array);
                }
                Op::ArrayPush => {
                    let value = self.stack.pop().unwrap();
                    // Array is at stack.len() - 1 (right below the value we just popped)
                    // Stack: [..., array, value] -> pop value, push to array -> [..., array]
                    let arr_idx = self.stack.len() - 1;
                    if let Some(Value::Array(arr)) = self.stack.get(arr_idx).cloned() {
                        arr.borrow_mut().push(value);
                    } else {
                        return Err(RuntimeError::type_error(
                            "can only push to arrays",
                            self.current_span(),
                        ));
                    }
                }
                Op::Hash(n) => {
                    let n = n as usize;
                    let base = self.stack.len() - n * 2;
                    let mut map = HashPairs::with_capacity(n);
                    let mut drained = self.stack.drain(base..);
                    for _ in 0..n {
                        let key = drained.next().unwrap();
                        let value = drained.next().unwrap();
                        if let Some(hash_key) = HashKey::from_value_owned(key) {
                            map.insert(hash_key, value);
                        }
                    }
                    drop(drained);
                    self.stack.push(Value::Hash(Rc::new(RefCell::new(map))));
                }
                Op::HashWithKeys(keys_idx, n) => {
                    let n = n as usize;
                    // Borrow the precomputed keys from the constant pool.
                    let keys: *const Vec<HashKey> = {
                        let frame = self.frames.last().unwrap();
                        match &frame.closure.proto.chunk.constants[keys_idx as usize] {
                            Constant::HashKeys(ks) => &**ks as *const _,
                            _ => unreachable!("HashWithKeys must reference HashKeys constant"),
                        }
                    };
                    // SAFETY: constants live for the whole frame execution.
                    let keys: &Vec<HashKey> = unsafe { &*keys };
                    let base = self.stack.len() - n;
                    // The compiler emits this op only for distinct literal
                    // keys, so the entries go in without a duplicate search.
                    let map = HashPairs::from_distinct(keys, self.stack.drain(base..));
                    self.stack.push(Value::Hash(Rc::new(RefCell::new(map))));
                }
                Op::LiteralArray(list_idx) => self.op_literal_array(list_idx),
                Op::LiteralListIncludes(list_idx, user_method) => {
                    self.op_literal_list_includes(list_idx, user_method)?;
                }
                Op::Range => {
                    let (start, end) = self.pop2();
                    match (&start, &end) {
                        (Value::Int(a), Value::Int(b)) => {
                            // Exclusive of the end, matching the tree-walking
                            // interpreter's `eval_range` (`start..end`). Bounded
                            // for the same reason: an eager collect from a
                            // request-supplied bound is an allocation failure,
                            // which aborts the process instead of the request.
                            // (`GetIterRange` needs no check — it iterates
                            // lazily and allocates nothing.)
                            crate::interpreter::limits::check_range(*a, *b, "range (..)").map_err(
                                |message| RuntimeError::General {
                                    message,
                                    span: self.current_span(),
                                },
                            )?;
                            let arr: Vec<Value> = (*a..*b).map(Value::Int).collect();
                            self.stack.push(Value::Array(Rc::new(RefCell::new(arr))));
                        }
                        _ => {
                            return Err(RuntimeError::type_error(
                                "Range requires integer operands",
                                self.current_span(),
                            ));
                        }
                    }
                }
                Op::GetIndex => {
                    let index = self.stack.pop().unwrap();
                    let object = self.stack.pop().unwrap();
                    if let Some(result) = Self::try_get_index_hit(&object, &index) {
                        self.stack.push(result);
                    } else {
                        let result = self.op_get_index(&object, &index, self.current_span())?;
                        self.stack.push(result);
                    }
                }
                Op::SetIndex => {
                    let value = self.stack.pop().unwrap();
                    let index = self.stack.pop().unwrap();
                    let object = self.stack.pop().unwrap();
                    // An assignment expression evaluates to the assigned value,
                    // which is what the tree-walker returns. Pushing `object`
                    // here made `h[k] = v` as a function's last statement return
                    // the whole container under `--vm` and `v` under `--dev`.
                    // Statement position is unaffected: every emission site
                    // follows this with a `Pop`.
                    if Self::try_set_index_hit(&object, &index, value.clone()) {
                        self.stack.push(value);
                    } else {
                        self.op_set_index(&object, &index, value.clone(), self.current_span())?;
                        self.stack.push(value);
                    }
                }
                Op::BuildString(n) => {
                    let len = self.stack.len();
                    let start = len - n as usize;
                    // `"#{s}"` of a string is the string itself — share it.
                    if n == 1 && matches!(self.stack[start], Value::String(_)) {
                        continue;
                    }
                    let result = build_interpolated(&self.stack[start..]);
                    self.stack.truncate(start);
                    self.stack.push(Value::String(result));
                }
                Op::Spread => {
                    // `[..., ...xs]`: append xs's elements to the array being
                    // built below it. Stack: [..., array, xs] -> [..., array].
                    let spread = self.stack.pop().unwrap();
                    let Value::Array(items) = spread else {
                        return Err(RuntimeError::type_error(
                            "cannot spread non-array value",
                            self.current_span(),
                        ));
                    };
                    let arr_idx = self.stack.len() - 1;
                    if let Some(Value::Array(arr)) = self.stack.get(arr_idx) {
                        arr.borrow_mut().extend(items.borrow().iter().cloned());
                    }
                }

                // --- Properties ---
                Op::GetProperty(idx, _, _) => {
                    let object = self.stack.pop().unwrap();
                    #[cfg(test)]
                    tally_general_field_op(&object);
                    // Fast path: instance-field hit. Fields shadow methods
                    // (op_get_property's documented order), so a present key
                    // fully decides the access — no name copy, no span, and
                    // a single ahash probe instead of the contains_key +
                    // method-walk + re-probe of the general path. A model's
                    // relation or translated field is not its stored value:
                    // that read goes to the general path.
                    if let Value::Instance(inst) = &object {
                        let hit = {
                            let frame = self.frames.last().unwrap();
                            match &frame.closure.proto.chunk.constants[idx as usize] {
                                Constant::String(name) => {
                                    let inst_ref = inst.borrow();
                                    inst_ref
                                        .fields
                                        .get(name.as_ref())
                                        .filter(|_| {
                                            Self::field_hit_is_raw(&inst_ref, name.as_ref())
                                        })
                                        .cloned()
                                }
                                _ => None,
                            }
                        };
                        if let Some(val) = hit {
                            if crate::interpreter::value::restricted_methods_declared() {
                                self.check_private_value(&object, self.string_constant(idx))?;
                            }
                            let val = self.force_field_hit(val)?;
                            self.stack.push(val);
                            continue;
                        }
                    }
                    // Fast path: `hash.key` hit. A user-defined Hash method
                    // outranks the key, so only take it when none exist; a
                    // miss falls through to the method-value path below.
                    if let Value::Hash(hash) = &object {
                        use crate::interpreter::executor::calls::user_methods::{
                            has_user_methods, PrimType,
                        };
                        if !has_user_methods(PrimType::Hash) {
                            let hit = {
                                let frame = self.frames.last().unwrap();
                                match &frame.closure.proto.chunk.constants[idx as usize] {
                                    Constant::String(name) => {
                                        hash.borrow().get(&StrKey(name.as_ref())).cloned()
                                    }
                                    _ => None,
                                }
                            };
                            if let Some(val) = hit {
                                self.stack.push(val);
                                continue;
                            }
                        }
                    }
                    let name = self.read_string_constant(idx);
                    self.check_private_value(&object, &name)?;
                    let span = self.current_span();
                    let result = self.op_get_property_member(&object, &name, span)?;
                    self.stack.push(result);
                }
                Op::GetMethod(idx) => {
                    let object = self.stack.pop().unwrap();
                    let name = self.read_string_constant(idx);
                    self.check_private_value(&object, &name)?;
                    let result = match Self::callee_native(&object, &name) {
                        Some(bound) => bound,
                        None => {
                            let span = self.current_span();
                            self.op_get_property_member(&object, &name, span)?
                        }
                    };
                    self.stack.push(result);
                }
                Op::GetPropertyOrNull(idx) => {
                    let object = self.stack.pop().unwrap();
                    #[cfg(test)]
                    tally_general_field_op(&object);
                    // Fast path: an instance-field hit decides the read, as in
                    // `Op::GetProperty` — only a miss differs (Null here). A
                    // model's relation or translated field still goes to the
                    // resolver below, as it did before this path existed.
                    if let Value::Instance(inst) = &object {
                        let hit = {
                            let frame = self.frames.last().unwrap();
                            match &frame.closure.proto.chunk.constants[idx as usize] {
                                Constant::String(name)
                                    if !field_needs_model_resolution(inst, name) =>
                                {
                                    inst.borrow().fields.get(name.as_ref()).cloned()
                                }
                                _ => None,
                            }
                        };
                        if let Some(val) = hit {
                            if crate::interpreter::value::restricted_methods_declared() {
                                self.check_private_value(&object, self.string_constant(idx))?;
                            }
                            let val = self.force_field_hit(val)?;
                            self.stack.push(val);
                            continue;
                        }
                    }
                    let name = self.read_string_constant(idx);
                    self.check_private_value(&object, &name)?;
                    let span = self.current_span();
                    let result = match self.op_get_property_member(&object, &name, span) {
                        Err(RuntimeError::NoSuchProperty(ref info))
                            if matches!(object, Value::Instance(_)) && info.property == name =>
                        {
                            Value::Null
                        }
                        other => other?,
                    };
                    self.stack.push(result);
                }
                Op::SetProperty(idx) => {
                    let value = self.stack.pop().unwrap();
                    #[cfg(test)]
                    tally_general_field_op(self.stack.last().unwrap());
                    // A private/protected field written from outside: refused
                    // like the read. One relaxed load while no class declares
                    // a restricted member; only an instance holds one, so a
                    // hash write goes on without the check.
                    if crate::interpreter::value::restricted_methods_declared() {
                        if let Some(object @ Value::Instance(_)) = self.stack.last() {
                            self.check_private_value(object, self.string_constant(idx))?;
                        }
                    }
                    // A constructed value object is frozen. One relaxed load
                    // while no value-object class exists.
                    if crate::interpreter::domain::value_objects_declared() {
                        if let Some(Value::Instance(inst)) = self.stack.last() {
                            let frozen_class = {
                                let inst_ref = inst.borrow();
                                crate::interpreter::domain::is_frozen(inst, &inst_ref.class)
                                    .then(|| inst_ref.class.clone())
                            };
                            if let Some(class) = frozen_class {
                                let name = self.read_string_constant_owned(idx);
                                // A type error, like the tree-walker's for
                                // any refused field write.
                                return Err(RuntimeError::type_error(
                                    crate::interpreter::domain::frozen_error(&class, &name),
                                    self.current_span(),
                                ));
                            }
                        }
                    }
                    // Fast path: in-place update of an EXISTING instance
                    // field — no name copy, no span. First-time sets (key
                    // absent) fall through to the general path, which needs
                    // the owned name for the insert anyway. A model, or a
                    // class with `const` fields, goes to the general path,
                    // which applies `set_instance_field`'s rules.
                    let updated = {
                        let object = self.stack.last().unwrap();
                        if let Value::Hash(hash) = object {
                            // `h.key = v` always sets the key: one borrowed
                            // probe, and a miss appends the pool's key — no
                            // owned name, receiver clone or span.
                            // SAFETY: the constant outlives the arm.
                            let name = unsafe { &*self.string_constant_key(idx) };
                            hash.borrow_mut().set_str(name, value.clone());
                            true
                        } else if let Value::Instance(inst) = object {
                            if Self::has_checked_writes(&inst.borrow().class) {
                                false
                            } else {
                                let frame = self.frames.last().unwrap();
                                match &frame.closure.proto.chunk.constants[idx as usize] {
                                    Constant::String(name) => {
                                        let mut inst_mut = inst.borrow_mut();
                                        if let Some(slot) = inst_mut.fields.get_mut(name.as_ref()) {
                                            *slot = value.clone();
                                        } else {
                                            // First write to this field. The general
                                            // path below allocates the name *twice* —
                                            // once in `read_string_constant_owned`,
                                            // once in `op_set_property`'s
                                            // `name.to_string()` — and every field of
                                            // every constructor is a first write, so
                                            // that second allocation sat on the path
                                            // of every object Soli creates.
                                            inst_mut.fields.insert(name.clone(), value.clone());
                                        }
                                        true
                                    }
                                    _ => false,
                                }
                            }
                        } else {
                            false
                        }
                    };
                    if updated {
                        self.stack.pop(); // pop object
                        self.stack.push(value);
                        continue;
                    }
                    let name = self.read_string_constant_owned(idx);
                    let object = self.stack.last().unwrap().clone();
                    let span = self.current_span();
                    self.op_set_property(&object, &name, value.clone(), span)?;
                    self.stack.pop(); // pop object
                    self.stack.push(value);
                }

                // A constructor's field default: stored as is, the way the
                // tree-walker's `initialize_instance_fields` does.
                Op::InitProperty(idx) => {
                    let value = self.stack.pop().unwrap();
                    let name = self.read_string_constant(idx);
                    if let Some(Value::Instance(inst)) = self.stack.last() {
                        inst.borrow_mut().fields.insert(name, value.clone());
                    }
                    self.stack.pop(); // pop object
                    self.stack.push(value);
                }

                // --- Classes ---
                Op::Class(idx) => {
                    let name = self.read_string_constant_owned(idx);
                    let class = Class {
                        name,
                        ..Default::default()
                    };
                    self.stack.push(Value::Class(Rc::new(class)));
                }
                Op::Module(idx) => {
                    let name = self.read_string_constant_owned(idx);
                    // `module Billing` declared again (another file of the
                    // same context): its members join the existing module.
                    if let Some(Value::Class(existing)) = self.globals.get(&name) {
                        if existing.is_module && existing.name == name {
                            let existing = existing.clone();
                            self.stack.push(Value::Class(existing));
                            continue;
                        }
                    }
                    let class = Class {
                        name: name.clone(),
                        is_module: true,
                        ..Default::default()
                    };
                    if let Some(hooks) = crate::interpreter::mixin_registry::get(&name) {
                        *class.included_hook_stmts.borrow_mut() = hooks.included;
                        *class.extended_hook_stmts.borrow_mut() = hooks.extended;
                        *class.concern_method_names.borrow_mut() = hooks.concern_method_names;
                    }
                    self.stack.push(Value::Class(Rc::new(class)));
                }
                Op::Include => {
                    let module_val = self.stack.pop().unwrap();
                    let class_val = self.stack.last().unwrap().clone();
                    let span = self.current_span();
                    self.op_include(&class_val, &module_val, span)?;
                }
                Op::Extend => {
                    let module_val = self.stack.pop().unwrap();
                    let class_val = self.stack.last().unwrap().clone();
                    let span = self.current_span();
                    self.op_extend(&class_val, &module_val, span)?;
                }
                Op::Inherit => {
                    let superclass_val = self.stack.pop().unwrap();
                    let subclass_val = self.stack.last().unwrap().clone();
                    let span = self.current_span();
                    self.op_inherit(&subclass_val, &superclass_val, span)?;
                }
                Op::Method(idx) => {
                    let name = self.read_string_constant_owned(idx);
                    let method = self.stack.pop().unwrap();
                    let class = self.stack.last().unwrap().clone();
                    let span = self.current_span();
                    self.op_add_method(&class, &name, method, false, span)?;
                }
                Op::StaticMethod(idx) => {
                    let name = self.read_string_constant_owned(idx);
                    let method = self.stack.pop().unwrap();
                    let class = self.stack.last().unwrap().clone();
                    let span = self.current_span();
                    self.op_add_method(&class, &name, method, true, span)?;
                }
                Op::New(argc) => {
                    let span = self.current_span();
                    self.op_new(argc as usize, span)?;
                }
                // `New` and `Call` share a binding path once labels are
                // involved: `call_value_named` dispatches on the callee, and a
                // class callee runs its constructor with the instance bound.
                Op::CallNamed(argc, names_idx) | Op::NewNamed(argc, names_idx) => {
                    let labels = {
                        let frame = self.frames.last().unwrap();
                        match &frame.closure.proto.chunk.constants[names_idx as usize] {
                            Constant::ArgNames(names) => names.clone(),
                            _ => unreachable!("CallNamed must reference an ArgNames constant"),
                        }
                    };
                    let span = self.current_span();
                    self.call_value_named(argc as usize, &labels, span)?;
                }
                Op::CallMethodNamed(name_idx, argc, names_idx) => {
                    let name = self.read_string_constant(name_idx);
                    let labels = self.read_arg_names(names_idx);
                    let receiver_idx = self.stack.len() - 1 - argc as usize;
                    let span = self.current_span();
                    self.call_method_named(receiver_idx, &labels, &name, true, span)?;
                }
                Op::CallSelfOrGlobalNamed(name_idx, argc, names_idx) => {
                    // `[callee, marker, args…]`, as for `CallSelfOrGlobal`:
                    // the marker is what `SelfOrGlobalCallee` decided.
                    let marker = self.stack.remove(self.stack.len() - 1 - argc as usize);
                    let labels = self.read_arg_names(names_idx);
                    let span = self.current_span();
                    if matches!(marker, Value::Null) {
                        // A global of that name: `SelfOrGlobalCallee` put it in
                        // the callee slot, and it binds as `CallNamed` binds.
                        let labels = crate::vm::vm_calls::without_block_label(&labels);
                        self.call_value_named(argc as usize, &labels, span)?;
                    } else {
                        let name = self.read_string_constant(name_idx);
                        // `this` stayed as the receiver: a call on self, so a
                        // private method is allowed, as for `CallSelfOrGlobal`.
                        let receiver_idx = self.stack.len() - 1 - argc as usize;
                        self.call_method_named(receiver_idx, &labels, &name, false, span)?;
                    }
                }
                Op::GetThis => {
                    let base = self.frames.last().unwrap().stack_base;
                    let this = self.stack[base].clone();
                    self.stack.push(this);
                }
                Op::GetSuper(_idx) => {
                    let _this = self.stack.pop().unwrap();
                    self.stack.push(Value::Null);
                }
                Op::CallSuperInit(argc) => {
                    let span = self.current_span();
                    let superclass = self.frame_superclass(span)?;
                    self.call_init_of(&superclass, argc as usize, span)?;
                }
                Op::CallSuperMethod(name_idx, argc) => {
                    let argc = argc as usize;
                    let span = self.current_span();
                    let receiver_idx = self.stack.len() - 1 - argc;
                    // Shared, not copied: the name is only looked up, and
                    // copied into the error on a miss.
                    let name = self.read_string_constant(name_idx);
                    let superclass = self.frame_superclass(span)?;
                    // The nearest definition above the defining class, level
                    // by level: an ancestor's compiled method must not beat
                    // a nearer override that is still AST. A tree-walker one
                    // (a class copied from interpreter globals) is compiled
                    // as a method of the class that declares it, so its own
                    // `super` resolves from there.
                    if let Some((closure, defining_class)) =
                        self.resolve_instance_method(&superclass, &name)?
                    {
                        self.call_closure_in_class(closure, argc, span, Some(defining_class))?;
                    } else if let Some(native) = superclass.find_native_method(&name) {
                        let receiver = self.stack[receiver_idx].clone();
                        if let Value::Instance(ref inst) = receiver {
                            if inst.borrow().class.is_model_subclass() {
                                self.call_model_instance_native(
                                    inst.clone(),
                                    native,
                                    receiver_idx,
                                    argc,
                                    &name,
                                    span,
                                )?;
                            } else {
                                if let Some(expected) = native.arity {
                                    if argc != expected {
                                        return Err(RuntimeError::wrong_arity(
                                            expected, argc, span,
                                        ));
                                    }
                                }
                                let user_args: Vec<Value> =
                                    self.stack[receiver_idx + 1..receiver_idx + 1 + argc].to_vec();
                                let _native_span =
                                    crate::serve::span_log::maybe_instrument_native(&native.name);
                                let result = crate::interpreter::executor::access::member::call_native_instance_method(
                                    inst, &native, &user_args,
                                )
                                .map_err(|e| RuntimeError::new(e, span))?;
                                drop(_native_span);
                                self.stack.truncate(receiver_idx);
                                self.stack.push(result);
                            }
                        } else {
                            return Err(RuntimeError::type_error(
                                "super method called on non-instance",
                                span,
                            ));
                        }
                    } else {
                        return Err(RuntimeError::no_such_property(
                            format!("super({})", superclass.name),
                            String::from(name.as_str()),
                            span,
                        )
                        .with_hint(
                            crate::interpreter::value::Class::instance_member_hint(
                                &superclass,
                                &name,
                            ),
                        ));
                    }
                }
                Op::Field(idx) | Op::ConstField(idx) => {
                    // A declared instance field: its name, in declaration
                    // order (a value object's implicit constructor binds
                    // arguments by it), and the value every new instance
                    // starts it with before any constructor runs — its
                    // literal default, or nil (`Vm::init_fields`).
                    let name = self.read_string_constant_owned(idx);
                    let start = self.stack.pop().unwrap_or(Value::Null);
                    if let Some(Value::Class(class)) = self.stack.last() {
                        class.declared_fields.borrow_mut().push(name.clone());
                        let mut field_init = class.field_init.borrow_mut();
                        let field_init =
                            Rc::make_mut(field_init.get_or_insert_with(Default::default));
                        field_init.fields.push((name.into(), start));
                    }
                }
                Op::StaticField(idx) => {
                    let name = self.read_string_constant_owned(idx);
                    let value = self.stack.pop().unwrap();
                    let class = self.stack.last().unwrap().clone();
                    if let Value::Class(ref cls) = class {
                        cls.static_fields.borrow_mut().insert(name, value);
                    }
                }
                Op::StaticConstField(idx) => {
                    let name = self.read_string_constant_owned(idx);
                    let value = self.stack.pop().unwrap();
                    let class = self.stack.last().unwrap().clone();
                    if let Value::Class(ref cls) = class {
                        cls.static_fields.borrow_mut().insert(name, value);
                    }
                }

                // --- Exceptions ---
                Op::TryBegin(catch_offset, finally_offset) => {
                    // `*_offset` is the distance from the instruction *after*
                    // TryBegin to the catch/finally target; `frame.ip` already
                    // points at that next instruction (it was advanced during
                    // fetch), so the absolute target is `frame.ip + offset`.
                    // (A spurious `- 1` here landed the catch on the Jump that
                    // skips the catch block, so caught exceptions ran no catch
                    // body — assignments and `return`s inside `catch` were lost.)
                    let frame = self.frames.last().unwrap();
                    let handler = ExceptionHandler {
                        catch_ip: frame.ip + catch_offset as usize,
                        finally_ip: frame.ip + finally_offset as usize,
                        stack_depth: self.stack.len(),
                        frame_depth: self.frames.len(),
                        iter_depth: self.iter_stack.len(),
                    };
                    self.exception_handlers.push(handler);
                }
                Op::TryEnd => {
                    self.exception_handlers.pop();
                }
                Op::ForceFuture => {
                    if let Some(top) = self.stack.last_mut().filter(|top| top.is_future()) {
                        let future = std::mem::replace(top, Value::Null);
                        let span = self.current_span();
                        let value = future.resolve().map_err(|e| RuntimeError::new(e, span))?;
                        if let Some(top) = self.stack.last_mut() {
                            *top = value;
                        }
                    }
                }
                Op::Throw => {
                    let value = self.stack.pop().unwrap();
                    let span = self.current_span();
                    self.throw_exception(value, span)?;
                }
                Op::CatchMatch(name_idx, jump_offset) => {
                    // The class name is borrowed from the constant pool: this
                    // runs on every typed `catch` test, and copying the name
                    // into a String only to compare it was the whole cost.
                    let frame = self.frames.last().unwrap();
                    let type_name: &str =
                        match &frame.closure.proto.chunk.constants[name_idx as usize] {
                            Constant::String(s) => s.as_str(),
                            _ => "",
                        };
                    let matches = match self.stack.last().unwrap() {
                        Value::Instance(inst) => {
                            let inst = inst.borrow();
                            class_name_matches(&inst.class, type_name)
                        }
                        _ => false,
                    };
                    if !matches {
                        let frame = self.frames.last_mut().unwrap();
                        frame.ip += jump_offset as usize;
                    }
                }
                Op::EnumPayload(slot, variant_idx, index) => {
                    // Borrowed all the way down: the variant name from the
                    // constant pool, the metadata and field name in place. It
                    // used to build the variant String, a HashKey and a Vec of
                    // every field name per bound field.
                    let value = {
                        let frame = self.frames.last().unwrap();
                        let variant: &str =
                            match &frame.closure.proto.chunk.constants[variant_idx as usize] {
                                Constant::String(s) => s.as_str(),
                                _ => "",
                            };
                        match &self.stack[frame.stack_base + slot as usize] {
                            Value::Instance(inst) => {
                                crate::interpreter::executor::pattern_matching::enum_variant_payload(
                                    &inst.borrow(),
                                    variant,
                                    index as usize,
                                )
                                .unwrap_or(Value::Null)
                            }
                            _ => Value::Null,
                        }
                    };
                    self.push(value);
                }
                Op::MatchArrayLen(slot, len, at_least) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let ok = array_len_matches(&self.stack[base + slot as usize], len, at_least);
                    self.push(Value::Bool(ok));
                }
                Op::ArrayRestLocal(slot, start) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let rest = match &self.stack[base + slot as usize] {
                        Value::Array(arr) => arr
                            .borrow()
                            .get(start as usize..)
                            .map(<[Value]>::to_vec)
                            .unwrap_or_default(),
                        _ => Vec::new(),
                    };
                    self.push(Value::Array(Rc::new(RefCell::new(rest))));
                }
                Op::MatchType(name_idx, jump_offset) => {
                    // Borrowed, not copied: see `CatchMatch`.
                    let frame = self.frames.last().unwrap();
                    let type_name: &str =
                        match &frame.closure.proto.chunk.constants[name_idx as usize] {
                            Constant::String(s) => s.as_str(),
                            _ => "",
                        };
                    let matches = match self.stack.last().unwrap() {
                        Value::Int(_) => type_name == "Int",
                        Value::Float(_) => type_name == "Float",
                        Value::Bool(_) => type_name == "Bool",
                        Value::String(_) => type_name == "String",
                        Value::Null => type_name == "Void",
                        Value::Array(_) => type_name == "Array",
                        Value::Hash(_) => type_name == "Hash",
                        Value::Instance(inst) => inst.borrow().class.name == type_name,
                        _ => false,
                    };
                    if !matches {
                        let frame = self.frames.last_mut().unwrap();
                        frame.ip += jump_offset as usize;
                    }
                }
                Op::Rethrow => {
                    let value = self.stack.pop().unwrap();
                    let span = self.current_span();
                    self.throw_exception(value, span)?;
                }
                Op::PopHandler => {
                    self.exception_handlers.pop();
                }
                Op::RescueJump(offset) => {
                    let frame = self.frames.last_mut().unwrap();
                    frame.ip += offset as usize;
                }

                // --- Iterators ---
                Op::GetIter => {
                    let iterable = self.stack.pop().unwrap();
                    let span = self.current_span();
                    let state = self.create_iterator(iterable, span)?;
                    self.iter_stack.push(state);
                }
                Op::GetIterRange => {
                    let (start, end) = self.pop2();
                    match (&start, &end) {
                        (Value::Int(a), Value::Int(b)) => {
                            self.iter_stack.push(IterState::Range {
                                current: *a,
                                end: *b,
                            });
                        }
                        _ => {
                            return Err(RuntimeError::type_error(
                                "Range requires integer operands",
                                self.current_span(),
                            ));
                        }
                    }
                }
                Op::PopIter => {
                    self.iter_stack.pop();
                }
                Op::PopRangeComprehension(slot) => {
                    self.stack.pop();
                    let base = self.frames.last().unwrap().stack_base;
                    let len = match &self.stack[base + slot as usize] {
                        Value::Array(arr) => arr.borrow().len(),
                        Value::Hash(hash) => hash.borrow().len(),
                        _ => 0,
                    };
                    crate::interpreter::limits::check_comprehension_len(len).map_err(
                        |message| RuntimeError::General {
                            message,
                            span: self.current_span(),
                        },
                    )?;
                }
                Op::ForIter(exit_offset) => {
                    let next_val = self.iter_next();
                    if let Some(val) = next_val {
                        self.stack.push(val);
                    } else {
                        self.iter_stack.pop();
                        self.frames.last_mut().unwrap().ip += exit_offset as usize;
                    }
                }
                Op::ForIterRange(exit_offset) => {
                    // Inlined range iteration — no method call, no enum match
                    let state = self.iter_stack.last_mut().unwrap();
                    if let IterState::Range { current, end } = state {
                        // Exclusive of `end`, matching the tree-walker.
                        if *current < *end {
                            let val = Value::Int(*current);
                            *current += 1;
                            self.stack.push(val);
                        } else {
                            self.iter_stack.pop();
                            self.frames.last_mut().unwrap().ip += exit_offset as usize;
                        }
                    } else {
                        unreachable!("ForIterRange used with non-range iterator");
                    }
                }

                // --- I/O ---
                Op::Print(n) => {
                    // `print`/`puts`/`println`, as the tree-walker's builtins
                    // do them: Futures are resolved first, and the line goes
                    // through `write_captured_or_stdout`, so a
                    // `StdoutCaptureGuard` (test runner, graph builder) sees
                    // it. Nothing is retained: a serve worker keeps its VM for
                    // life, and a copy of every printed line was a leak.
                    use std::fmt::Write as _;
                    let start = self.stack.len() - n as usize;
                    let mut line = String::new();
                    let mut failed = None;
                    for (i, value) in self.stack.drain(start..).enumerate() {
                        if i > 0 {
                            line.push(' ');
                        }
                        match value.resolve() {
                            Ok(value) => {
                                let _ = write!(line, "{}", value);
                            }
                            Err(e) => {
                                failed = Some(e);
                                break;
                            }
                        }
                    }
                    if let Some(e) = failed {
                        return Err(RuntimeError::new(e, self.current_span()));
                    }
                    line.push('\n');
                    crate::interpreter::builtins::write_captured_or_stdout(&line);
                    self.stack.push(Value::Null);
                }

                Op::Nop => {}

                // A bare read of a name that may hold a zero-argument def or
                // builtin: call it, as the tree-walker does (see
                // `Op::AutoInvoke`). Anything else stays as read.
                Op::AutoInvoke => {
                    if auto_invocable(self.stack.last().unwrap()) {
                        let span = self.current_span();
                        self.call_value(0, span)?;
                    }
                }

                // Line coverage (only compiled in under a coverage tracker —
                // see `Op::CoverLine`). The fast tier does not handle it, so
                // it lands here; an ordinary server never emits it.
                Op::CoverLine(line) => {
                    let path = self
                        .frames
                        .last()
                        .and_then(|f| f.closure.proto.source_path.clone());
                    if let (Some(path), Some(global)) =
                        (path, crate::coverage::get_global_coverage_tracker())
                    {
                        if let Ok(guard) = global.lock() {
                            guard.record_line_hit(&path, line as usize);
                        }
                    }
                }

                // The step debugger's stop points (compiled in only with a
                // debugger attached; see `Op::DebugStmt`).
                Op::DebugStmt(line) => {
                    if crate::debug::active() && crate::debug::is_debuggee() {
                        self.debugger_statement(line);
                    }
                }
                Op::DebugBreak => {
                    if crate::debug::active() && crate::debug::is_debuggee() {
                        self.debugger_break();
                    }
                }

                // Imports are resolved before VM execution; the opcode is a
                // no-op marker at runtime.
                Op::Import(_) => {}

                // --- Combined compare+jump ---
                Op::TestLessEqualJump(offset) => {
                    let (a, b) = self.pop2();
                    let result = match (&a, &b) {
                        (Value::Int(x), Value::Int(y)) => *x <= *y,
                        (Value::Float(x), Value::Float(y)) => *x <= *y,
                        (Value::String(x), Value::String(y)) => x.as_str() <= y.as_str(),
                        _ => {
                            let span = self.current_span();
                            self.op_compare_less_equal(&a, &b, span)?
                        }
                    };
                    if !result {
                        self.frames.last_mut().unwrap().ip += offset as usize;
                    }
                }
                Op::TestLessJump(offset) => {
                    let (a, b) = self.pop2();
                    let result = match (&a, &b) {
                        (Value::Int(x), Value::Int(y)) => *x < *y,
                        (Value::Float(x), Value::Float(y)) => *x < *y,
                        (Value::String(x), Value::String(y)) => x.as_str() < y.as_str(),
                        _ => {
                            let span = self.current_span();
                            self.op_compare_less(&a, &b, span)?
                        }
                    };
                    if !result {
                        self.frames.last_mut().unwrap().ip += offset as usize;
                    }
                }

                // --- Super-instructions ---
                //
                // Every Int one of these computes is checked: past the Int
                // range it raises `Op::Add`'s overflow error and leaves the
                // local as it was. A bare `n + 1` wrapped instead (release
                // builds have no overflow checks), so `i += 1` on the largest
                // Int gave the smallest. `run_fast` hands its overflows here.
                Op::IncrLocal(slot) | Op::IncrLocalFast(slot) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let idx = base + slot as usize;
                    if let Value::Int(n) = self.stack[idx] {
                        match n.checked_add(1) {
                            Some(next) => self.stack[idx] = Value::Int(next),
                            None => {
                                let span = self.current_span();
                                return Err(crate::interpreter::executor::operators::int_overflow(
                                    n, "+", 1, span,
                                ));
                            }
                        }
                    } else {
                        // Fallback: treat as GetLocal + Constant(1) + Add + SetLocal + Pop
                        let val = self.stack[idx].clone();
                        let result = match val {
                            Value::Float(n) => Value::Float(n + 1.0),
                            _ => {
                                let span = self.current_span();
                                self.op_add(val, Value::Int(1), span)?
                            }
                        };
                        self.stack[idx] = result;
                    }
                }
                Op::DecrLocal(slot) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let idx = base + slot as usize;
                    if let Value::Int(n) = self.stack[idx] {
                        match n.checked_sub(1) {
                            Some(next) => self.stack[idx] = Value::Int(next),
                            None => {
                                let span = self.current_span();
                                return Err(crate::interpreter::executor::operators::int_overflow(
                                    n, "-", 1, span,
                                ));
                            }
                        }
                    } else {
                        let val = self.stack[idx].clone();
                        let result = match val {
                            Value::Float(n) => Value::Float(n - 1.0),
                            _ => {
                                let span = self.current_span();
                                self.op_subtract(val, Value::Int(1), span)?
                            }
                        };
                        self.stack[idx] = result;
                    }
                }
                Op::AddLocalLocal(slot_a, slot_b) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let a = &self.stack[base + slot_a as usize];
                    let b = &self.stack[base + slot_b as usize];
                    let result = match (a, b) {
                        (Value::Int(x), Value::Int(y)) => match x.checked_add(*y) {
                            Some(result) => Value::Int(result),
                            None => {
                                let span = self.current_span();
                                return Err(crate::interpreter::executor::operators::int_overflow(
                                    *x, "+", *y, span,
                                ));
                            }
                        },
                        (Value::Float(x), Value::Float(y)) => Value::Float(x + y),
                        _ => {
                            let a = a.clone();
                            let b = b.clone();
                            let span = self.current_span();
                            self.op_add(a, b, span)?
                        }
                    };
                    self.stack.push(result);
                }
                Op::LessEqualLocalLocal(slot_a, slot_b) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let a = &self.stack[base + slot_a as usize];
                    let b = &self.stack[base + slot_b as usize];
                    let result = match (a, b) {
                        (Value::Int(x), Value::Int(y)) => *x <= *y,
                        (Value::Float(x), Value::Float(y)) => *x <= *y,
                        (Value::String(x), Value::String(y)) => x.as_str() <= y.as_str(),
                        _ => {
                            let span = self.current_span();
                            self.op_compare_less_equal(a, b, span)?
                        }
                    };
                    self.stack.push(Value::Bool(result));
                }
                // --- In-place local arithmetic: no push, no pop ---
                //
                // The fast arms mutate the slot directly. `Value` is 24 bytes with
                // drop glue, so avoiding the push/pop pair matters more here than
                // the saved dispatches. The slow arms keep `Op::Add`'s overflow,
                // coercion and error behaviour exactly — an Int sum is checked,
                // anything else goes to the generic operator — so these opcodes
                // are a peephole, not new semantics.
                Op::AddLocalsInPlace(a, b) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let (ia, ib) = (base + a as usize, base + b as usize);
                    enum Pair {
                        Int(i64, i64),
                        Float(f64, f64),
                        Str,
                        Other,
                    }
                    let pair = match (&self.stack[ia], &self.stack[ib]) {
                        (Value::Int(x), Value::Int(y)) => Pair::Int(*x, *y),
                        (Value::Float(x), Value::Float(y)) => Pair::Float(*x, *y),
                        (Value::String(_), Value::String(_)) => Pair::Str,
                        _ => Pair::Other,
                    };
                    match pair {
                        Pair::Int(x, y) => match x.checked_add(y) {
                            Some(sum) => self.stack[ia] = Value::Int(sum),
                            None => {
                                let span = self.current_span();
                                return Err(crate::interpreter::executor::operators::int_overflow(
                                    x, "+", y, span,
                                ));
                            }
                        },
                        Pair::Float(x, y) => {
                            self.stack[ia] = Value::Float(x + y);
                        }
                        Pair::Str => append_strings_in_place(&mut self.stack, ia, ib),
                        Pair::Other => {
                            let (x, y) = (self.stack[ia].clone(), self.stack[ib].clone());
                            let span = self.current_span();
                            self.stack[ia] = self.op_add(x, y, span)?;
                        }
                    }
                }
                Op::AppendGlobal(idx) => {
                    let rhs = self.pop();
                    let before = self.pop();
                    let name = self.read_string_constant(idx);
                    self.append_named(&name, before, rhs)?;
                }
                Op::AppendGlobalConst(name_idx, cidx) => {
                    let (name, extra) = {
                        let frame = self.frames.last().unwrap();
                        let constants = &frame.closure.proto.chunk.constants;
                        let name = match &constants[name_idx as usize] {
                            Constant::String(text) => text.clone(),
                            _ => crate::interpreter::value::SoliStr::default(),
                        };
                        let extra = match &constants[cidx as usize] {
                            Constant::String(text) => text.clone(),
                            _ => {
                                return Err(RuntimeError::type_error(
                                    "internal: AppendGlobalConst constant is not a string",
                                    self.current_span(),
                                ));
                            }
                        };
                        (name, extra)
                    };
                    if let Some(Value::String(text)) = self.globals.get_mut(name.as_str()) {
                        text.push_str(&extra);
                    } else {
                        let left = self
                            .globals
                            .get(name.as_str())
                            .cloned()
                            .unwrap_or(Value::Null);
                        let span = self.current_span();
                        let updated = self.op_add(left, Value::String(extra), span)?;
                        self.globals.insert(String::from(name.as_str()), updated);
                    }
                }
                Op::AppendLocalConst(slot, cidx) => {
                    let extra = {
                        let frame = self.frames.last().unwrap();
                        match &frame.closure.proto.chunk.constants[cidx as usize] {
                            Constant::String(text) => text.clone(),
                            _ => {
                                return Err(RuntimeError::type_error(
                                    "internal: AppendLocalConst constant is not a string",
                                    self.current_span(),
                                ));
                            }
                        }
                    };
                    let base = self.frames.last().unwrap().stack_base;
                    let idx = base + slot as usize;
                    if let Value::String(text) = &mut self.stack[idx] {
                        text.push_str(&extra);
                    } else {
                        let left = self.stack[idx].clone();
                        let span = self.current_span();
                        self.stack[idx] = self.op_add(left, Value::String(extra), span)?;
                    }
                }
                Op::AppendLocal(slot) => {
                    let rhs = self.pop();
                    let before = self.pop();
                    let idx = self.frames.last().unwrap().stack_base + slot as usize;
                    let result = shovel_into(&mut self.stack[idx], before, rhs)
                        .map_err(|m| RuntimeError::type_error(m, self.current_span()))?;
                    self.stack.push(result);
                }
                Op::AddAssignLocal(slot) => {
                    let rhs = self.pop();
                    let before = self.pop();
                    let idx = self.frames.last().unwrap().stack_base + slot as usize;
                    let result = match add_strings_into(&mut self.stack[idx], before, rhs) {
                        Ok(updated) => updated,
                        Err((before, rhs)) => {
                            let sum = self.add_values(before, rhs)?;
                            self.stack[idx] = sum.clone();
                            sum
                        }
                    };
                    self.stack.push(result);
                }
                Op::AddAssignUpvalue(idx) => {
                    let rhs = self.pop();
                    let before = self.pop();
                    // `frames` and `stack` are borrowed apart: no reference
                    // count moves to reach the cell.
                    let appended = {
                        let frame = self.frames.last().unwrap();
                        match &mut *frame.closure.upvalues[idx as usize].borrow_mut() {
                            Upvalue::Open(slot) => {
                                add_strings_into(&mut self.stack[*slot], before, rhs)
                            }
                            Upvalue::Closed(value) => add_strings_into(value, before, rhs),
                        }
                    };
                    let result = match appended {
                        Ok(updated) => updated,
                        Err((before, rhs)) => {
                            let sum = self.add_values(before, rhs)?;
                            let frame = self.frames.last().unwrap();
                            match &mut *frame.closure.upvalues[idx as usize].borrow_mut() {
                                Upvalue::Open(slot) => self.stack[*slot] = sum.clone(),
                                Upvalue::Closed(value) => *value = sum.clone(),
                            }
                            sum
                        }
                    };
                    self.stack.push(result);
                }
                Op::AddAssignGlobal(idx) => {
                    let rhs = self.pop();
                    let before = self.pop();
                    // Shared, not copied: the key is built only when the
                    // global does not exist yet.
                    let name = self.read_string_constant(idx);
                    let appended = match self.globals.get_mut(name.as_str()) {
                        Some(slot) => add_strings_into(slot, before, rhs),
                        None => Err((before, rhs)),
                    };
                    let result = match appended {
                        Ok(updated) => updated,
                        Err((before, rhs)) => {
                            let sum = self.add_values(before, rhs)?;
                            // An Int or Float sum overwrites the slot in
                            // place; the key is copied only for a first write.
                            match self.globals.get_mut(name.as_str()) {
                                Some(slot) => *slot = sum.clone(),
                                None => {
                                    self.globals
                                        .insert(String::from(name.as_str()), sum.clone());
                                }
                            }
                            sum
                        }
                    };
                    self.stack.push(result);
                }
                Op::AppendUpvalue(idx) => {
                    let rhs = self.pop();
                    let before = self.pop();
                    let outcome = {
                        let frame = self.frames.last().unwrap();
                        match &mut *frame.closure.upvalues[idx as usize].borrow_mut() {
                            Upvalue::Open(slot) => shovel_into(&mut self.stack[*slot], before, rhs),
                            Upvalue::Closed(value) => shovel_into(value, before, rhs),
                        }
                    };
                    let result =
                        outcome.map_err(|m| RuntimeError::type_error(m, self.current_span()))?;
                    self.stack.push(result);
                }
                Op::AddLocalConst(slot, const_idx) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let local = &self.stack[base + slot as usize];
                    let frame = self.frames.last().unwrap();
                    let constant = &frame.closure.proto.chunk.constants[const_idx as usize];
                    let result = match (local, constant) {
                        (Value::Int(x), Constant::Int(y)) => match x.checked_add(*y) {
                            Some(result) => Value::Int(result),
                            None => {
                                let span = self.current_span();
                                return Err(crate::interpreter::executor::operators::int_overflow(
                                    *x, "+", *y, span,
                                ));
                            }
                        },
                        (Value::Float(x), Constant::Float(y)) => Value::Float(x + y),
                        _ => {
                            let a = local.clone();
                            let b = constant_to_value(constant);
                            let span = self.current_span();
                            self.op_add(a, b, span)?
                        }
                    };
                    self.stack.push(result);
                }
                Op::SetLocalPop(slot) => {
                    let val = self.pop();
                    let base = self.frames.last().unwrap().stack_base;
                    self.stack[base + slot as usize] = val;
                }
                Op::SubLocalLocal(slot_a, slot_b) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let a = &self.stack[base + slot_a as usize];
                    let b = &self.stack[base + slot_b as usize];
                    let result = match (a, b) {
                        (Value::Int(x), Value::Int(y)) => match x.checked_sub(*y) {
                            Some(result) => Value::Int(result),
                            None => {
                                let span = self.current_span();
                                return Err(crate::interpreter::executor::operators::int_overflow(
                                    *x, "-", *y, span,
                                ));
                            }
                        },
                        (Value::Float(x), Value::Float(y)) => Value::Float(x - y),
                        _ => {
                            let a = a.clone();
                            let b = b.clone();
                            let span = self.current_span();
                            self.op_subtract(a, b, span)?
                        }
                    };
                    self.stack.push(result);
                }
                Op::MulLocalLocal(slot_a, slot_b) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let a = &self.stack[base + slot_a as usize];
                    let b = &self.stack[base + slot_b as usize];
                    let result = match (a, b) {
                        (Value::Int(x), Value::Int(y)) => match x.checked_mul(*y) {
                            Some(result) => Value::Int(result),
                            None => {
                                let span = self.current_span();
                                return Err(crate::interpreter::executor::operators::int_overflow(
                                    *x, "*", *y, span,
                                ));
                            }
                        },
                        (Value::Float(x), Value::Float(y)) => Value::Float(x * y),
                        _ => {
                            let a = a.clone();
                            let b = b.clone();
                            let span = self.current_span();
                            self.op_multiply(a, b, span)?
                        }
                    };
                    self.stack.push(result);
                }
                Op::DivLocalLocal(slot_a, slot_b) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let a = &self.stack[base + slot_a as usize];
                    let b = &self.stack[base + slot_b as usize];
                    let result = match (a, b) {
                        (Value::Int(x), Value::Int(y)) => {
                            if *y == 0 {
                                return Err(RuntimeError::division_by_zero(self.current_span()));
                            }
                            match x.checked_div(*y) {
                                Some(result) => Value::Int(result),
                                None => {
                                    let span = self.current_span();
                                    return Err(
                                        crate::interpreter::executor::operators::int_overflow(
                                            *x, "/", *y, span,
                                        ),
                                    );
                                }
                            }
                        }
                        (Value::Float(x), Value::Float(y)) => {
                            if *y == 0.0 {
                                return Err(RuntimeError::division_by_zero(self.current_span()));
                            }
                            Value::Float(x / y)
                        }
                        _ => {
                            let a = a.clone();
                            let b = b.clone();
                            let span = self.current_span();
                            self.op_divide(a, b, span)?
                        }
                    };
                    self.stack.push(result);
                }
                Op::ModLocalLocal(slot_a, slot_b) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let a = &self.stack[base + slot_a as usize];
                    let b = &self.stack[base + slot_b as usize];
                    let result = match (a, b) {
                        (Value::Int(x), Value::Int(y)) => {
                            if *y == 0 {
                                return Err(RuntimeError::division_by_zero(self.current_span()));
                            }
                            match x.checked_rem(*y) {
                                Some(result) => Value::Int(result),
                                None => {
                                    let span = self.current_span();
                                    return Err(
                                        crate::interpreter::executor::operators::int_overflow(
                                            *x, "%", *y, span,
                                        ),
                                    );
                                }
                            }
                        }
                        _ => {
                            let a = a.clone();
                            let b = b.clone();
                            let span = self.current_span();
                            self.op_modulo(a, b, span)?
                        }
                    };
                    self.stack.push(result);
                }
                Op::SubLocalConst(slot, const_idx) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let local = &self.stack[base + slot as usize];
                    let frame = self.frames.last().unwrap();
                    let constant = &frame.closure.proto.chunk.constants[const_idx as usize];
                    let result = match (local, constant) {
                        (Value::Int(x), Constant::Int(y)) => match x.checked_sub(*y) {
                            Some(result) => Value::Int(result),
                            None => {
                                let span = self.current_span();
                                return Err(crate::interpreter::executor::operators::int_overflow(
                                    *x, "-", *y, span,
                                ));
                            }
                        },
                        (Value::Float(x), Constant::Float(y)) => Value::Float(x - y),
                        _ => {
                            let a = local.clone();
                            let b = constant_to_value(constant);
                            let span = self.current_span();
                            self.op_subtract(a, b, span)?
                        }
                    };
                    self.stack.push(result);
                }
                Op::MulLocalConst(slot, const_idx) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let local = &self.stack[base + slot as usize];
                    let frame = self.frames.last().unwrap();
                    let constant = &frame.closure.proto.chunk.constants[const_idx as usize];
                    let result = match (local, constant) {
                        (Value::Int(x), Constant::Int(y)) => match x.checked_mul(*y) {
                            Some(result) => Value::Int(result),
                            None => {
                                let span = self.current_span();
                                return Err(crate::interpreter::executor::operators::int_overflow(
                                    *x, "*", *y, span,
                                ));
                            }
                        },
                        (Value::Float(x), Constant::Float(y)) => Value::Float(x * y),
                        _ => {
                            let a = local.clone();
                            let b = constant_to_value(constant);
                            let span = self.current_span();
                            self.op_multiply(a, b, span)?
                        }
                    };
                    self.stack.push(result);
                }
                // `MulLocalConst`'s and its siblings' rules, with the result
                // stored back into the local instead of pushed. On an error the
                // local keeps its value, as `GetLocal … SetLocal` left it.
                Op::AddLocalConstInPlace(slot, const_idx)
                | Op::SubLocalConstInPlace(slot, const_idx)
                | Op::MulLocalConstInPlace(slot, const_idx) => {
                    let frame = self.frames.last().unwrap();
                    let idx = frame.stack_base + slot as usize;
                    let constant = &frame.closure.proto.chunk.constants[const_idx as usize];
                    let local = &self.stack[idx];
                    let result = match (local, constant) {
                        (Value::Int(x), Constant::Int(y)) => {
                            let (symbol, checked) = match op {
                                Op::AddLocalConstInPlace(..) => ("+", x.checked_add(*y)),
                                Op::SubLocalConstInPlace(..) => ("-", x.checked_sub(*y)),
                                _ => ("*", x.checked_mul(*y)),
                            };
                            match checked {
                                Some(result) => Value::Int(result),
                                None => {
                                    let span = self.current_span();
                                    return Err(
                                        crate::interpreter::executor::operators::int_overflow(
                                            *x, symbol, *y, span,
                                        ),
                                    );
                                }
                            }
                        }
                        (Value::Float(x), Constant::Float(y)) => Value::Float(match op {
                            Op::AddLocalConstInPlace(..) => x + y,
                            Op::SubLocalConstInPlace(..) => x - y,
                            _ => x * y,
                        }),
                        _ => {
                            let a = local.clone();
                            let b = constant_to_value(constant);
                            let span = self.current_span();
                            match op {
                                Op::AddLocalConstInPlace(..) => self.op_add(a, b, span)?,
                                Op::SubLocalConstInPlace(..) => self.op_subtract(a, b, span)?,
                                _ => self.op_multiply(a, b, span)?,
                            }
                        }
                    };
                    self.stack[idx] = result;
                }
                Op::DivLocalConst(slot, const_idx) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let local = &self.stack[base + slot as usize];
                    let frame = self.frames.last().unwrap();
                    let constant = &frame.closure.proto.chunk.constants[const_idx as usize];
                    let result = match (local, constant) {
                        (Value::Int(x), Constant::Int(y)) => {
                            if *y == 0 {
                                return Err(RuntimeError::division_by_zero(self.current_span()));
                            }
                            match x.checked_div(*y) {
                                Some(result) => Value::Int(result),
                                None => {
                                    let span = self.current_span();
                                    return Err(
                                        crate::interpreter::executor::operators::int_overflow(
                                            *x, "/", *y, span,
                                        ),
                                    );
                                }
                            }
                        }
                        (Value::Float(x), Constant::Float(y)) => {
                            if *y == 0.0 {
                                return Err(RuntimeError::division_by_zero(self.current_span()));
                            }
                            Value::Float(x / y)
                        }
                        _ => {
                            let a = local.clone();
                            let b = constant_to_value(constant);
                            let span = self.current_span();
                            self.op_divide(a, b, span)?
                        }
                    };
                    self.stack.push(result);
                }
                Op::ModLocalConst(slot, const_idx) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let local = &self.stack[base + slot as usize];
                    let frame = self.frames.last().unwrap();
                    let constant = &frame.closure.proto.chunk.constants[const_idx as usize];
                    // `Modulo`'s rules, with the operands read in place.
                    let result = match (local, constant) {
                        (Value::Int(x), Constant::Int(y)) => {
                            if *y == 0 {
                                return Err(RuntimeError::division_by_zero(self.current_span()));
                            }
                            match x.checked_rem(*y) {
                                Some(result) => Value::Int(result),
                                None => {
                                    let span = self.current_span();
                                    return Err(
                                        crate::interpreter::executor::operators::int_overflow(
                                            *x, "%", *y, span,
                                        ),
                                    );
                                }
                            }
                        }
                        _ => {
                            let a = local.clone();
                            let b = constant_to_value(constant);
                            let span = self.current_span();
                            self.op_modulo(a, b, span)?
                        }
                    };
                    self.stack.push(result);
                }
                Op::GetLocal2(slot_a, slot_b) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let a = self.stack[base + slot_a as usize].clone();
                    let b = self.stack[base + slot_b as usize].clone();
                    self.stack.push(a);
                    self.stack.push(b);
                }
                Op::LessLocalLocal(slot_a, slot_b) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let a = &self.stack[base + slot_a as usize];
                    let b = &self.stack[base + slot_b as usize];
                    let result = match (a, b) {
                        (Value::Int(x), Value::Int(y)) => *x < *y,
                        (Value::Float(x), Value::Float(y)) => *x < *y,
                        (Value::String(x), Value::String(y)) => x.as_str() < y.as_str(),
                        _ => {
                            let span = self.current_span();
                            self.op_compare_less(a, b, span)?
                        }
                    };
                    self.stack.push(Value::Bool(result));
                }
                Op::GreaterLocalLocal(slot_a, slot_b) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let a = &self.stack[base + slot_a as usize];
                    let b = &self.stack[base + slot_b as usize];
                    let result = match (a, b) {
                        (Value::Int(x), Value::Int(y)) => *x > *y,
                        (Value::Float(x), Value::Float(y)) => *x > *y,
                        (Value::String(x), Value::String(y)) => x.as_str() > y.as_str(),
                        _ => {
                            let span = self.current_span();
                            self.op_compare_less(b, a, span)?
                        }
                    };
                    self.stack.push(Value::Bool(result));
                }
                Op::NotEqualLocalConst(slot, const_idx) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let local = &self.stack[base + slot as usize];
                    let frame = self.frames.last().unwrap();
                    let constant = &frame.closure.proto.chunk.constants[const_idx as usize];
                    let result = match (local, constant) {
                        (Value::Int(x), Constant::Int(y)) => *x != *y,
                        (Value::Float(x), Constant::Float(y)) => *x != *y,
                        (Value::String(x), Constant::String(y)) => x != y,
                        _ => !values_equal(local, &constant_to_value(constant)),
                    };
                    self.stack.push(Value::Bool(result));
                }
                Op::EqualLocalConst(slot, const_idx) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let local = &self.stack[base + slot as usize];
                    let frame = self.frames.last().unwrap();
                    let constant = &frame.closure.proto.chunk.constants[const_idx as usize];
                    let result = match (local, constant) {
                        (Value::Int(x), Constant::Int(y)) => *x == *y,
                        (Value::Float(x), Constant::Float(y)) => *x == *y,
                        (Value::String(x), Constant::String(y)) => x == y,
                        _ => values_equal(local, &constant_to_value(constant)),
                    };
                    self.stack.push(Value::Bool(result));
                }
                // --- Test + Jump super-instructions ---
                Op::TestGreaterJump(offset) => {
                    let (a, b) = self.pop2();
                    let result = match (&a, &b) {
                        (Value::Int(x), Value::Int(y)) => *x > *y,
                        (Value::Float(x), Value::Float(y)) => *x > *y,
                        (Value::String(x), Value::String(y)) => x.as_str() > y.as_str(),
                        _ => {
                            let span = self.current_span();
                            // `a > b` is evaluated as `b < a` to reuse one
                            // comparator. Report the operands in source order,
                            // or the message names them backwards: `[1] > 1`
                            // said "Cannot compare int and array".
                            self.op_compare_less(&b, &a, span).map_err(|_| {
                                RuntimeError::type_error(
                                    format!(
                                        "Cannot compare {} and {}",
                                        a.type_name(),
                                        b.type_name()
                                    ),
                                    span,
                                )
                            })?
                        }
                    };
                    if !result {
                        self.frames.last_mut().unwrap().ip += offset as usize;
                    }
                }
                Op::TestGreaterEqualJump(offset) => {
                    let (a, b) = self.pop2();
                    let result = match (&a, &b) {
                        (Value::Int(x), Value::Int(y)) => *x >= *y,
                        (Value::Float(x), Value::Float(y)) => *x >= *y,
                        (Value::String(x), Value::String(y)) => x.as_str() >= y.as_str(),
                        _ => {
                            let span = self.current_span();
                            // Same swap as `>` above: report source order.
                            self.op_compare_less_equal(&b, &a, span).map_err(|_| {
                                RuntimeError::type_error(
                                    format!(
                                        "Cannot compare {} and {}",
                                        a.type_name(),
                                        b.type_name()
                                    ),
                                    span,
                                )
                            })?
                        }
                    };
                    if !result {
                        self.frames.last_mut().unwrap().ip += offset as usize;
                    }
                }
                Op::TestNotEqualJump(offset) => {
                    let (a, b) = self.pop2();
                    if values_equal(&a, &b) {
                        self.frames.last_mut().unwrap().ip += offset as usize;
                    }
                }
                Op::TestEqualJump(offset) => {
                    let (a, b) = self.pop2();
                    if !values_equal(&a, &b) {
                        self.frames.last_mut().unwrap().ip += offset as usize;
                    }
                }

                // --- Null/boolean checks ---
                Op::IsNull => {
                    let val = self.stack.last().unwrap();
                    self.stack.push(Value::Bool(matches!(val, Value::Null)));
                }
                Op::NotNull => {
                    let val = self.stack.last().unwrap();
                    self.stack.push(Value::Bool(!matches!(val, Value::Null)));
                }
                Op::JumpIfNull(offset) => {
                    let val = self.stack.last().unwrap();
                    if matches!(val, Value::Null) {
                        self.frames.last_mut().unwrap().ip += offset as usize;
                    }
                }
                Op::JumpIfNotNull(offset) => {
                    let val = self.stack.last().unwrap();
                    if !matches!(val, Value::Null) {
                        self.frames.last_mut().unwrap().ip += offset as usize;
                    }
                }
                Op::IsTruthyLocal(slot) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let val = &self.stack[base + slot as usize];
                    self.stack.push(Value::Bool(val.is_truthy()));
                }
                Op::IsFalsyLocal(slot) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let val = &self.stack[base + slot as usize];
                    self.stack.push(Value::Bool(!val.is_truthy()));
                }
                Op::JumpIfLocalFalsy(slot, offset) | Op::JumpIfLocalTruthy(slot, offset) => {
                    let frame = self.frames.last_mut().unwrap();
                    let truthy = self.stack[frame.stack_base + slot as usize].is_truthy();
                    if truthy == matches!(op, Op::JumpIfLocalTruthy(..)) {
                        frame.ip += offset as usize;
                    }
                }
                Op::AddLocalInt(slot, n) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let local = &self.stack[base + slot as usize];
                    let result = match local {
                        Value::Int(x) => match x.checked_add(n as i64) {
                            Some(sum) => Value::Int(sum),
                            None => {
                                let span = self.current_span();
                                return Err(crate::interpreter::executor::operators::int_overflow(
                                    *x, "+", n as i64, span,
                                ));
                            }
                        },
                        Value::Float(x) => Value::Float(*x + n as f64),
                        _ => {
                            let a = local.clone();
                            let span = self.current_span();
                            self.op_add(a, Value::Int(n as i64), span)?
                        }
                    };
                    self.stack.push(result);
                }
                Op::GetAndNullLocal(slot) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let val = self.stack[base + slot as usize].clone();
                    self.stack[base + slot as usize] = Value::Null;
                    self.stack.push(val);
                }
                Op::IsZeroLocal(slot) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let local = &self.stack[base + slot as usize];
                    let result = match local {
                        Value::Int(x) => *x == 0,
                        Value::Float(x) => *x == 0.0,
                        _ => false,
                    };
                    self.stack.push(Value::Bool(result));
                }
                Op::NotZeroLocal(slot) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let local = &self.stack[base + slot as usize];
                    let result = match local {
                        Value::Int(x) => *x != 0,
                        Value::Float(x) => *x != 0.0,
                        _ => true,
                    };
                    self.stack.push(Value::Bool(result));
                }
                Op::GetGlobalNullCheck(idx) => {
                    let val = {
                        let frame = self.frames.last().unwrap();
                        let name = match &frame.closure.proto.chunk.constants[idx as usize] {
                            Constant::String(s) => s.as_ref(),
                            _ => "",
                        };
                        self.globals.get(name).cloned()
                    };
                    match val {
                        Some(v) => self.stack.push(v),
                        None => self.stack.push(Value::Null),
                    };
                }
                Op::GetGlobalCall(idx, argc) => {
                    let val = {
                        let frame = self.frames.last().unwrap();
                        let name = match &frame.closure.proto.chunk.constants[idx as usize] {
                            Constant::String(s) => s.as_ref(),
                            _ => "",
                        };
                        self.globals.get(name).cloned()
                    };
                    match val {
                        Some(v) => {
                            self.stack.push(v);
                            let span = self.current_span();
                            self.call_value(argc as usize, span)?;
                        }
                        None => {
                            let name = self.read_string_constant_owned(idx);
                            return Err(self.undefined_global(name));
                        }
                    };
                }
                Op::NotLocal(slot) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let val = self.stack[base + slot as usize].clone();
                    self.stack.push(Value::Bool(!val.is_truthy()));
                }
                Op::NegateLocal(slot) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let val = self.stack[base + slot as usize].clone();
                    let result =
                        match val {
                            Value::Int(n) => match n.checked_neg() {
                                Some(negated) => Value::Int(negated),
                                None => return Err(
                                    crate::interpreter::executor::operators::int_negate_overflow(
                                        n,
                                        self.current_span(),
                                    ),
                                ),
                            },
                            Value::Float(n) => Value::Float(-n),
                            Value::Decimal(d) => {
                                Value::Decimal(crate::interpreter::value::DecimalValue(-d.0, d.1))
                            }
                            _ => {
                                return Err(RuntimeError::type_error(
                                    format!("Cannot negate {}", val.type_name()),
                                    self.current_span(),
                                ));
                            }
                        };
                    self.stack.push(result);
                }
                Op::EqualLocalLocal(slot_a, slot_b) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let equal = values_equal(
                        &self.stack[base + slot_a as usize],
                        &self.stack[base + slot_b as usize],
                    );
                    self.stack.push(Value::Bool(equal));
                }
                Op::NotEqualLocalLocal(slot_a, slot_b) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let equal = values_equal(
                        &self.stack[base + slot_a as usize],
                        &self.stack[base + slot_b as usize],
                    );
                    self.stack.push(Value::Bool(!equal));
                }
                Op::PopNull => {
                    self.stack.pop();
                    self.stack.push(Value::Null);
                }
                Op::DupN(n) => {
                    let len = self.stack.len();
                    for _ in 0..n {
                        self.stack.push(self.stack[len - 1].clone());
                    }
                }
                Op::GetLocalProperty(slot, idx, _, _) => {
                    let base = self.frames.last().unwrap().stack_base;
                    #[cfg(test)]
                    tally_general_field_op(&self.stack[base + slot as usize]);
                    // Fast path, mirroring Op::GetProperty: fields shadow
                    // methods, so a present key fully decides the access — no
                    // owned name, no span, one ahash probe. Without this the
                    // fused form would be *slower* than the pair it replaces,
                    // because the general path allocates the name as a String.
                    // `(field hit, hash key hit)`. A hash key is the value as
                    // is: no privacy rule or deferred field applies to it.
                    let (hit, hash_hit) = {
                        let frame = self.frames.last().unwrap();
                        match (
                            &self.stack[base + slot as usize],
                            &frame.closure.proto.chunk.constants[idx as usize],
                        ) {
                            (Value::Instance(inst), Constant::String(name)) => {
                                let inst_ref = inst.borrow();
                                let hit = inst_ref
                                    .fields
                                    .get(name.as_ref())
                                    .filter(|_| Self::field_hit_is_raw(&inst_ref, name.as_ref()))
                                    .cloned();
                                (hit, None)
                            }
                            // `row.title` on a hash: the key, unless a user
                            // method on Hash outranks it (as in GetProperty).
                            (Value::Hash(hash), Constant::String(name))
                                if !crate::interpreter::executor::calls::user_methods::has_user_methods(
                                    crate::interpreter::executor::calls::user_methods::PrimType::Hash,
                                ) =>
                            {
                                (None, hash.borrow().get(&StrKey(name.as_ref())).cloned())
                            }
                            _ => (None, None),
                        }
                    };
                    if let Some(val) = hash_hit {
                        self.stack.push(val);
                        continue;
                    }
                    if let Some(val) = hit {
                        // `@x` in a method: slot 0 of its frame is the `this`
                        // the check would compare the receiver with, and a
                        // member of self is never refused.
                        if crate::interpreter::value::restricted_methods_declared()
                            && (slot != 0 || !self.slot_zero_is_this())
                        {
                            self.check_private_value(
                                &self.stack[base + slot as usize],
                                self.string_constant(idx),
                            )?;
                        }
                        let val = self.force_field_hit(val)?;
                        self.stack.push(val);
                        continue;
                    }
                    let object = self.stack[base + slot as usize].clone();
                    let name = self.read_string_constant(idx);
                    self.check_private_value(&object, &name)?;
                    let span = self.current_span();
                    let result = self.op_get_property_member(&object, &name, span)?;
                    self.stack.push(result);
                }
                Op::GetLocalIndex(slot, idx_slot) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let result = match Self::try_get_index_hit(
                        &self.stack[base + slot as usize],
                        &self.stack[base + idx_slot as usize],
                    ) {
                        Some(v) => v,
                        None => {
                            let object = self.stack[base + slot as usize].clone();
                            let index_val = self.stack[base + idx_slot as usize].clone();
                            self.op_get_index(&object, &index_val, self.current_span())?
                        }
                    };
                    self.stack.push(result);
                }
                Op::GetNestedIndex(obj_slot, arr_slot, idx_slot) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let result = self.nested_index_get(base, obj_slot, arr_slot, idx_slot)?;
                    self.stack.push(result);
                }
                Op::AddNestedIndex(acc_slot, obj_slot, arr_slot, idx_slot) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let addend = self.nested_index_get(base, obj_slot, arr_slot, idx_slot)?;
                    let acc_i = base + acc_slot as usize;
                    match (&self.stack[acc_i], &addend) {
                        (&Value::Int(a), &Value::Int(b)) => match a.checked_add(b) {
                            Some(sum) => self.stack[acc_i] = Value::Int(sum),
                            None => {
                                let span = self.current_span();
                                return Err(crate::interpreter::executor::operators::int_overflow(
                                    a, "+", b, span,
                                ));
                            }
                        },
                        _ => {
                            let acc = self.stack[acc_i].clone();
                            self.stack[acc_i] = match (&acc, &addend) {
                                (Value::Float(x), Value::Float(y)) => Value::Float(x + y),
                                (Value::Int(x), Value::Float(y)) => Value::Float(*x as f64 + y),
                                (Value::Float(x), Value::Int(y)) => Value::Float(x + *y as f64),
                                _ => self.op_add(acc, addend, self.current_span())?,
                            };
                        }
                    }
                }
                Op::SetNestedIndex(obj_slot, arr_slot, idx_slot, val_slot) => {
                    let base = self.frames.last().unwrap().stack_base;
                    self.nested_index_set(base, obj_slot, arr_slot, idx_slot, val_slot)?;
                }

                // --- Templates ---
                Op::TemplateCaptureStart => {
                    let page = std::mem::take(&mut self.template_out);
                    self.template_captures.push(page);
                    self.stack.push(Value::Null);
                }
                Op::TemplateCaptureEnd => {
                    let page = self.template_captures.pop().unwrap_or_default();
                    let captured = std::mem::replace(&mut self.template_out, page);
                    self.stack.push(Value::String(captured.into()));
                }
                Op::TemplateComponent => {
                    let content = self.stack.pop().unwrap();
                    let props = self.stack.pop().unwrap();
                    let name = self.stack.pop().unwrap();
                    let span = self.current_span();
                    let html = crate::template::vm_template::render_component_block(
                        &name, &props, content,
                    )
                    .map_err(|e| RuntimeError::new(e, span))?;
                    self.template_out.push_str(&html);
                    crate::template::renderer::recycle_include_page(html);
                    self.stack.push(Value::Null);
                }
                Op::TemplatePartial => {
                    let data = self.stack.pop().unwrap();
                    let name = self.stack.pop().unwrap();
                    let span = self.current_span();
                    let html = crate::template::vm_template::render_partial_block(&name, &data)
                        .map_err(|e| RuntimeError::new(e, span))?;
                    self.template_out.push_str(&html);
                    // A page-sized include's buffer goes back to the pool for
                    // the next one.
                    crate::template::renderer::recycle_include_page(html);
                    self.stack.push(Value::Null);
                }
                // The renderer's `YieldMode::View`: a compiled template is never
                // a layout.
                Op::TemplateYield => {
                    let locals = self.stack.pop().unwrap();
                    let slot = match &locals {
                        Value::Hash(h) => h
                            .borrow()
                            .get(&crate::interpreter::value::HashKey::String(
                                "content".into(),
                            ))
                            .cloned(),
                        _ => None,
                    };
                    match slot {
                        Some(content) => crate::template::renderer::write_value_to_output(
                            &content,
                            false,
                            &mut self.template_out,
                        ),
                        None => {
                            return Err(RuntimeError::new(
                                "yield encountered outside of layout context",
                                self.current_span(),
                            ))
                        }
                    }
                    self.stack.push(Value::Null);
                }
                Op::TemplateYieldNamed => {
                    let name = self.stack.pop().unwrap();
                    let captured = match &name {
                        Value::String(n) => crate::template::content_store::get(n),
                        _ => None,
                    };
                    match captured {
                        Some(html) => self.template_out.push_str(&html),
                        None => {
                            return Err(RuntimeError::new(
                                "yield encountered outside of layout context",
                                self.current_span(),
                            ))
                        }
                    }
                    self.stack.push(Value::Null);
                }
                Op::TemplateContentFor => {
                    let content = self.stack.pop().unwrap();
                    let name = self.stack.pop().unwrap();
                    if let (Value::String(name), Value::String(content)) = (&name, &content) {
                        if self.template_content_before.is_none() {
                            self.template_content_before =
                                Some(crate::template::content_store::snapshot());
                        }
                        crate::template::content_store::append(name, content);
                    }
                    self.stack.push(Value::Null);
                }
                Op::TemplateIter => {
                    let value = self.stack.pop().unwrap();
                    let value = match value {
                        Value::Deferred(_) => value.force_deferred(),
                        other => other,
                    };
                    let value = match value {
                        Value::Hash(hash) => {
                            let pairs = hash
                                .borrow()
                                .iter()
                                .map(|(k, v)| {
                                    Value::Array(Rc::new(RefCell::new(vec![
                                        k.to_value(),
                                        v.clone(),
                                    ])))
                                })
                                .collect();
                            Value::Array(Rc::new(RefCell::new(pairs)))
                        }
                        other => other,
                    };
                    self.stack.push(value);
                }
                Op::TemplateWriteConst(idx) => {
                    let frame = self.frames.last().unwrap();
                    if let Constant::String(text) =
                        &frame.closure.proto.chunk.constants[idx as usize]
                    {
                        self.template_out.push_str(text);
                    }
                }
                Op::TemplateWrite(escaped) => {
                    let value = self.stack.pop().unwrap();
                    // A function written bare (`<%= current_user %>`) is called
                    // by the tree-walker's renderer; leave that to it.
                    if matches!(
                        value,
                        Value::Function(_)
                            | Value::NativeFunction(_)
                            | Value::VmClosure(_)
                            | Value::Method(_)
                    ) {
                        return Err(RuntimeError::EngineFallback(
                            "template wrote a function".to_string(),
                            self.current_span(),
                        ));
                    }
                    crate::template::renderer::write_value_to_output(
                        &value,
                        escaped,
                        &mut self.template_out,
                    );
                }

                // --- JSON ---
                Op::JsonParse => {
                    let json_str = match self.stack.pop().unwrap() {
                        Value::String(s) => s,
                        other => {
                            return Err(RuntimeError::new(
                                format!("JSON.parse() expects string, got {}", other.type_name()),
                                self.current_span(),
                            ))
                        }
                    };
                    match crate::interpreter::value::parse_json(&json_str) {
                        Ok(value) => self.stack.push(value),
                        Err(e) => {
                            return Err(RuntimeError::new(
                                format!("Failed to parse JSON: {}", e),
                                self.current_span(),
                            ))
                        }
                    }
                }
                Op::JsonStringify => {
                    let value = self.stack.pop().unwrap();
                    match crate::interpreter::value::stringify_to_string(&value) {
                        Ok(json_str) => {
                            self.stack.push(Value::String(json_str.into()));
                        }
                        Err(e) => {
                            return Err(RuntimeError::new(
                                format!("Cannot convert to JSON: {}", e),
                                self.current_span(),
                            ))
                        }
                    }
                }
            }
        }
    }

    // --- Stack operations ---

    #[inline(always)]
    pub fn push(&mut self, value: Value) {
        self.stack.push(value);
    }

    #[inline(always)]
    pub fn pop(&mut self) -> Value {
        // Safety: VM maintains stack invariants — pop is only called when stack is non-empty.
        // The debug_assert catches any compiler/bytecode bug that breaks that invariant in
        // debug/test builds (a controlled panic) before it becomes silent UB in release.
        debug_assert!(!self.stack.is_empty(), "VM stack underflow in pop()");
        unsafe {
            let new_len = self.stack.len() - 1;
            self.stack.set_len(new_len);
            std::ptr::read(self.stack.as_ptr().add(new_len))
        }
    }

    #[inline(always)]
    pub fn peek(&self, distance: usize) -> &Value {
        debug_assert!(
            self.stack.len() > distance,
            "VM stack underflow in peek({distance})"
        );
        unsafe { self.stack.get_unchecked(self.stack.len() - 1 - distance) }
    }

    /// `peek`, for writing the slot in place.
    #[inline(always)]
    pub(crate) fn peek_mut(&mut self, distance: usize) -> &mut Value {
        debug_assert!(
            self.stack.len() > distance,
            "VM stack underflow in peek_mut({distance})"
        );
        let index = self.stack.len() - 1 - distance;
        // SAFETY: as for `peek`: the VM's stack invariants keep `index` in
        // range, and the debug_assert checks it in debug builds.
        unsafe { self.stack.get_unchecked_mut(index) }
    }

    /// Pop two values from the stack (b first, then a).
    #[inline(always)]
    fn pop2(&mut self) -> (Value, Value) {
        debug_assert!(self.stack.len() >= 2, "VM stack underflow in pop2()");
        unsafe {
            let new_len = self.stack.len() - 2;
            let ptr = self.stack.as_ptr();
            let a = std::ptr::read(ptr.add(new_len));
            let b = std::ptr::read(ptr.add(new_len + 1));
            self.stack.set_len(new_len);
            (a, b)
        }
    }

    /// Build an array of the top `n` stack values, in order — taking an
    /// emptied array from `array_pool` when there is one.
    #[inline]
    pub(crate) fn array_from_stack(&mut self, n: usize) -> Value {
        let start = self.stack.len() - n;
        match self.array_pool.pop() {
            Some(array) => {
                {
                    let mut cells = array.borrow_mut();
                    cells.reserve(n);
                    // SAFETY: move the top `n` values into the array bitwise:
                    // `reserve` made room after its `len`, the ranges do not
                    // overlap (different allocations), and shortening the
                    // stack without dropping hands their ownership over.
                    // `Drain` + `extend` did the same through an iterator
                    // costing more than the allocation this pool saves.
                    unsafe {
                        std::ptr::copy_nonoverlapping(
                            self.stack.as_ptr().add(start),
                            cells.as_mut_ptr().add(cells.len()),
                            n,
                        );
                        let filled = cells.len() + n;
                        cells.set_len(filled);
                        self.stack.set_len(start);
                    }
                }
                Value::Array(array)
            }
            None => Value::Array(Rc::new(RefCell::new(self.stack.split_off(start)))),
        }
    }

    /// The comparison of a `<Cmp>LocalConst` op, identical to the unfused
    /// `Less` / `LessEqual` / `Greater` / `GreaterEqual` arms: the same inline
    /// int/int and float/float cases, the same comparator otherwise, and for
    /// `>` / `>=` the same operand swap with the error naming the operands in
    /// source order.
    fn compare_fused(&self, op: Op, a: &Value, b: &Value) -> Result<bool, RuntimeError> {
        if let Some(ordering) = scalar_ordering(a, b) {
            return Ok(match op {
                Op::LessLocalConst(..) => ordering.is_lt(),
                Op::LessEqualLocalConst(..) => ordering.is_le(),
                Op::GreaterLocalConst(..) => ordering.is_gt(),
                _ => ordering.is_ge(),
            });
        }
        // A NaN leaves `scalar_ordering` empty: the unfused arms compare two
        // floats inline, where every comparison with NaN is false.
        if let (Value::Float(_), Value::Float(_)) = (a, b) {
            return Ok(false);
        }
        // Two strings are ordered inline by the unfused arms too.
        if let (Value::String(x), Value::String(y)) = (a, b) {
            let ordering = x.as_str().cmp(y.as_str());
            return Ok(match op {
                Op::LessLocalConst(..) => ordering.is_lt(),
                Op::LessEqualLocalConst(..) => ordering.is_le(),
                Op::GreaterLocalConst(..) => ordering.is_gt(),
                _ => ordering.is_ge(),
            });
        }
        let span = self.current_span();
        let source_order = |_| {
            RuntimeError::type_error(
                format!("Cannot compare {} and {}", a.type_name(), b.type_name()),
                span,
            )
        };
        match op {
            Op::LessLocalConst(..) => self.op_compare_less(a, b, span),
            Op::LessEqualLocalConst(..) => self.op_compare_less_equal(a, b, span),
            Op::GreaterLocalConst(..) => self.op_compare_less(b, a, span).map_err(source_order),
            _ => self.op_compare_less_equal(b, a, span).map_err(source_order),
        }
    }

    /// Advance the current iterator, returning the next value or None if exhausted.
    fn iter_next(&mut self) -> Option<Value> {
        let state = self.iter_stack.last_mut()?;
        match state {
            // A loop callback's `for` loops push above it; the loop itself
            // is only ever on top between two callbacks, when no `ForIter` runs.
            IterState::Callback(_) => unreachable!("ForIter on an iterator-method loop"),
            IterState::Array { values, index } => {
                let arr = values.borrow();
                if *index < arr.len() {
                    let val = arr[*index].clone();
                    *index += 1;
                    Some(val)
                } else {
                    None
                }
            }
            IterState::Hash { values, index } => {
                // Live indexing into the IndexMap (insertion-ordered), matching
                // the Array variant: no upfront key-vector clone. Mutation
                // during iteration is observed live, same as arrays.
                let map = values.borrow();
                if let Some((key, _)) = map.get_index(*index) {
                    let key = key.to_value();
                    *index += 1;
                    Some(key)
                } else {
                    None
                }
            }
            IterState::Range { current, end } => {
                // Exclusive of `end`, matching the tree-walker.
                if *current < *end {
                    let val = Value::Int(*current);
                    *current += 1;
                    Some(val)
                } else {
                    None
                }
            }
            IterState::String { s, byte_offset } => {
                // Iterate by byte offset instead of pre-collecting a Vec<char>.
                if let Some(ch) = s[*byte_offset..].chars().next() {
                    *byte_offset += ch.len_utf8();
                    // Stack-buffer encode: no heap String per character.
                    Some(crate::interpreter::executor::calls::string_methods::char_to_value(ch))
                } else {
                    None
                }
            }
        }
    }

    fn create_iterator(&self, iterable: Value, span: Span) -> Result<IterState, RuntimeError> {
        match iterable {
            Value::Array(arr) => Ok(IterState::Array {
                values: arr,
                index: 0,
            }),
            Value::Hash(hash) => Ok(IterState::Hash {
                values: hash,
                index: 0,
            }),
            Value::String(s) => Ok(IterState::String { s, byte_offset: 0 }),
            // `for post in @posts` after a `grouped {}` block: the value is a
            // deferred query result, so force it and iterate the materialised
            // collection. Matches the tree-walker (executor/statements.rs's
            // `deferred @ Value::Deferred(_) => deferred.force_deferred()`).
            // Only `Deferred` is forced — a `Future` is not iterable on either
            // engine, and silently blocking on one here would diverge.
            Value::Deferred(ref cell) => {
                let resolved = crate::interpreter::builtins::model::batch::force(cell)
                    .map_err(|e| RuntimeError::General { message: e, span })?;
                self.create_iterator(resolved, span)
            }
            _ => Err(RuntimeError::type_error(
                format!("Cannot iterate over {}", iterable.type_name()),
                span,
            )),
        }
    }

    // --- Arithmetic operations ---
    //
    // The slow arm of every arithmetic and ordering op. The dispatch loop
    // computes the Int and Float cases (and string `+`) inline and hands
    // everything else here, where the rules are the tree-walker's own
    // definitions (`interpreter/executor/operators.rs`), called rather than
    // copied: a Decimal on either side of an Int or a Float, `Array - Array`,
    // string repetition, the overflow and division-by-zero errors. The copies
    // these replaced had drifted: no Decimal arm, no array difference, and a
    // zero divisor raised before the left operand was even looked at.

    fn op_add(&self, a: Value, b: Value, span: Span) -> Result<Value, RuntimeError> {
        crate::interpreter::Interpreter::eval_add(&a, &b, span)
    }

    fn op_subtract(&self, a: Value, b: Value, span: Span) -> Result<Value, RuntimeError> {
        crate::interpreter::Interpreter::eval_subtract(&a, &b, span)
    }

    fn op_multiply(&self, a: Value, b: Value, span: Span) -> Result<Value, RuntimeError> {
        crate::interpreter::Interpreter::eval_multiply(&a, &b, span)
    }

    fn op_divide(&self, a: Value, b: Value, span: Span) -> Result<Value, RuntimeError> {
        crate::interpreter::Interpreter::eval_divide(&a, &b, span)
    }

    fn op_modulo(&self, a: Value, b: Value, span: Span) -> Result<Value, RuntimeError> {
        crate::interpreter::Interpreter::eval_modulo(&a, &b, span)
    }

    /// `a < b`. The callers evaluate `a > b` as `b < a` (and `>=` as `<=`
    /// swapped), then word an error in source order.
    fn op_compare_less(&self, a: &Value, b: &Value, span: Span) -> Result<bool, RuntimeError> {
        use crate::interpreter::executor::operators::OrdOp;
        crate::interpreter::Interpreter::compare_ordered(a, OrdOp::Less, b, span)
    }

    fn op_compare_less_equal(
        &self,
        a: &Value,
        b: &Value,
        span: Span,
    ) -> Result<bool, RuntimeError> {
        use crate::interpreter::executor::operators::OrdOp;
        crate::interpreter::Interpreter::compare_ordered(a, OrdOp::LessEqual, b, span)
    }

    #[inline]
    fn two_const_str_ptrs(&self, k1: u16, k2: u16) -> (*const str, *const str) {
        let frame = self.frames.last().unwrap();
        let p1 = match &frame.closure.proto.chunk.constants[k1 as usize] {
            Constant::String(s) => s.as_ref() as *const str,
            _ => "" as *const str,
        };
        let p2 = match &frame.closure.proto.chunk.constants[k2 as usize] {
            Constant::String(s) => s.as_ref() as *const str,
            _ => "" as *const str,
        };
        (p1, p2)
    }

    /// `receiver[k1][k2]` with the same miss rules as two `HashGetConst`s:
    /// a missing first key is Null, and indexing Null raises.
    fn hash_get_nested(
        &mut self,
        receiver: &Value,
        k1: &str,
        k2: &str,
    ) -> Result<Value, RuntimeError> {
        let inner = match receiver {
            Value::Hash(hash) => hash
                .borrow()
                .get(&StrKey(k1))
                .cloned()
                .unwrap_or(Value::Null),
            other => self.hash_get_const_on(other, k1)?,
        };
        self.hash_get_const_on(&inner, k2)
    }

    fn hash_get_const_on(&mut self, receiver: &Value, key: &str) -> Result<Value, RuntimeError> {
        match receiver {
            Value::Hash(hash) => Ok(hash
                .borrow()
                .get(&StrKey(key))
                .cloned()
                .unwrap_or(Value::Null)),
            Value::Array(arr) => {
                let span = self.current_span();
                let arg = Value::String(key.into());
                self.vm_call_array_method(&arr.clone(), "get", &[arg], span)
            }
            Value::String(st) => {
                let span = self.current_span();
                let arg = Value::String(key.into());
                let st = st.clone();
                self.vm_call_string_method(&st, "get", &[arg], span)
            }
            // Un modèle se lit aussi par clé : même chemin que `GetIndex`.
            Value::Instance(_) => {
                let span = self.current_span();
                self.op_get_index(receiver, &Value::String(key.into()), span)
            }
            other => Err(RuntimeError::no_such_property(
                other.type_name(),
                "get".to_string(),
                self.current_span(),
            )),
        }
    }

    // --- Index operations ---

    /// Hit-only lookup: `Some` on a successful read (hash miss → Null),
    /// `None` when the slow path must run (wrong types, array OOB).
    #[inline(always)]
    fn try_get_index_hit(object: &Value, index: &Value) -> Option<Value> {
        match (object, index) {
            (Value::Hash(hash), Value::String(s)) => Some(
                hash.borrow()
                    .get(&StrKey(s))
                    .cloned()
                    .unwrap_or(Value::Null),
            ),
            (Value::Hash(hash), key) => Some(
                hash_get_value(&hash.borrow(), key)
                    .cloned()
                    .unwrap_or(Value::Null),
            ),
            (Value::Array(arr), Value::Int(i)) => {
                let arr = arr.borrow();
                let idx = if *i < 0 {
                    (arr.len() as i64 + i) as usize
                } else {
                    *i as usize
                };
                arr.get(idx).cloned()
            }
            // `s[i]` — a miss falls through to op_get_index for the error.
            (Value::String(s), Value::Int(i)) => {
                crate::interpreter::executor::calls::string_methods::string_char_at(s, *i)
            }
            _ => None,
        }
    }

    #[inline(always)]
    fn try_set_index_hit(object: &Value, index: &Value, value: Value) -> bool {
        match (object, index) {
            (Value::Hash(hash), key) => hash_set_value(&mut hash.borrow_mut(), key, value),
            (Value::Array(arr), Value::Int(i)) => {
                let mut arr = arr.borrow_mut();
                let idx = if *i < 0 {
                    (arr.len() as i64 + i) as usize
                } else {
                    *i as usize
                };
                if idx < arr.len() {
                    arr[idx] = value;
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    #[inline(always)]
    fn nested_index_get(
        &self,
        base: usize,
        obj_slot: u16,
        arr_slot: u16,
        idx_slot: u16,
    ) -> Result<Value, RuntimeError> {
        let key = match Self::try_get_index_hit(
            &self.stack[base + arr_slot as usize],
            &self.stack[base + idx_slot as usize],
        ) {
            Some(k) => k,
            None => {
                let object = self.stack[base + arr_slot as usize].clone();
                let index = self.stack[base + idx_slot as usize].clone();
                self.op_get_index(&object, &index, self.current_span())?
            }
        };
        match Self::try_get_index_hit(&self.stack[base + obj_slot as usize], &key) {
            Some(v) => Ok(v),
            None => {
                let object = self.stack[base + obj_slot as usize].clone();
                self.op_get_index(&object, &key, self.current_span())
            }
        }
    }

    #[inline(always)]
    fn nested_index_set(
        &self,
        base: usize,
        obj_slot: u16,
        arr_slot: u16,
        idx_slot: u16,
        val_slot: u16,
    ) -> Result<(), RuntimeError> {
        let key = match Self::try_get_index_hit(
            &self.stack[base + arr_slot as usize],
            &self.stack[base + idx_slot as usize],
        ) {
            Some(k) => k,
            None => {
                let object = self.stack[base + arr_slot as usize].clone();
                let index = self.stack[base + idx_slot as usize].clone();
                self.op_get_index(&object, &index, self.current_span())?
            }
        };
        let value = self.stack[base + val_slot as usize].clone();
        if Self::try_set_index_hit(&self.stack[base + obj_slot as usize], &key, value.clone()) {
            Ok(())
        } else {
            let object = self.stack[base + obj_slot as usize].clone();
            self.op_set_index(&object, &key, value, self.current_span())
        }
    }

    fn op_get_index(
        &self,
        object: &Value,
        index: &Value,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        // `@posts[0]` after a `grouped {}` block: force the deferred query
        // result first, the way the tree-walker's variable/member reads do.
        if let Value::Deferred(cell) = object {
            let resolved = crate::interpreter::builtins::model::batch::force(cell)
                .map_err(|e| RuntimeError::General { message: e, span })?;
            return self.op_get_index(&resolved, index, span);
        }
        match (object, index) {
            (Value::Array(arr), Value::Int(i)) => {
                let arr = arr.borrow();
                let idx = if *i < 0 {
                    (arr.len() as i64 + i) as usize
                } else {
                    *i as usize
                };
                arr.get(idx)
                    .cloned()
                    .ok_or_else(|| RuntimeError::IndexOutOfBounds {
                        index: *i,
                        length: arr.len(),
                        span,
                    })
            }
            (Value::Hash(hash), key) => {
                use crate::interpreter::value::{hash_get_value, StrKey};
                let hash = hash.borrow();
                match key {
                    Value::String(s) => Ok(hash.get(&StrKey(s)).cloned().unwrap_or(Value::Null)),
                    Value::Int(_)
                    | Value::Bool(_)
                    | Value::Null
                    | Value::Symbol(_)
                    | Value::Decimal(_) => {
                        Ok(hash_get_value(&hash, key).cloned().unwrap_or(Value::Null))
                    }
                    _ => Err(RuntimeError::type_error(
                        format!("Cannot use {} as hash key", key.type_name()),
                        span,
                    )),
                }
            }
            (Value::String(s), Value::Int(i)) => {
                crate::interpreter::executor::calls::string_methods::string_char_at(s, *i)
                    .ok_or_else(|| RuntimeError::IndexOutOfBounds {
                        index: *i,
                        // Counted only on a miss: the hit path never needs it.
                        length: s.chars().count(),
                        span,
                    })
            }
            // Une instance indexée par une chaîne : lecture dynamique du champ,
            // comme l'interpréteur (`access/index.rs`) — `user["_key"]`, ou un
            // nom de champ calculé. Sans ce cas, le VM refusait ce que
            // l'interpréteur accepte, et un gestionnaire changeait de résultat
            // selon le moteur qui le servait.
            (Value::Instance(inst), Value::String(key)) => {
                // `node["id"]` / `node.get("id")`: an XML node's attribute.
                if let Some(attr) =
                    crate::interpreter::builtins::xml::value::index_get(&inst.borrow(), key)
                {
                    return Ok(attr);
                }
                if crate::interpreter::value::restricted_methods_declared() {
                    self.check_private_value(object, key)?;
                }
                Ok(inst.borrow().get(key).unwrap_or(Value::Null))
            }
            _ => Err(RuntimeError::type_error(
                format!(
                    "Cannot index {} with {}",
                    object.type_name(),
                    index.type_name()
                ),
                span,
            )),
        }
    }

    /// `x["k"] = v` / `x.set("k", v)` on a receiver that is not a hash (the
    /// hash-set opcodes stand for both spellings). An instance takes the
    /// general index write, as a computed key does — `record["title"] = v`
    /// used to fail on the VM only. An instance whose class defines its own
    /// `set`, and anything else, keep the refusal.
    fn set_const_key_fallback(
        &self,
        receiver: &Value,
        key: &str,
        value: Value,
    ) -> Result<(), RuntimeError> {
        if let Value::Instance(inst) = receiver {
            let own_set = {
                let class = &inst.borrow().class;
                class.find_method("set").is_some() || class.find_vm_method("set").is_some()
            };
            if !own_set {
                let key = Value::String(key.to_string().into());
                return self.op_set_index(receiver, &key, value, self.current_span());
            }
        }
        Err(RuntimeError::no_such_property(
            receiver.type_name(),
            "set".to_string(),
            self.current_span(),
        ))
    }

    fn op_set_index(
        &self,
        object: &Value,
        index: &Value,
        value: Value,
        span: Span,
    ) -> Result<(), RuntimeError> {
        match (object, index) {
            (Value::Array(arr), Value::Int(i)) => {
                let mut arr = arr.borrow_mut();
                let idx = if *i < 0 {
                    (arr.len() as i64 + i) as usize
                } else {
                    *i as usize
                };
                if idx < arr.len() {
                    arr[idx] = value;
                    Ok(())
                } else {
                    Err(RuntimeError::IndexOutOfBounds {
                        index: *i,
                        length: arr.len(),
                        span,
                    })
                }
            }
            // `record[field] = value`: the write `record.<field> = value`
            // does on the tree-walker (`access/index.rs`), checks included.
            (Value::Instance(inst), Value::String(key)) => {
                // `node["id"] = v`: an XML node's attribute.
                let xml = crate::interpreter::builtins::xml::value::index_set(
                    &inst.borrow(),
                    key,
                    &value,
                );
                if let Some(result) = xml {
                    return result.map_err(|message| RuntimeError::type_error(message, span));
                }
                if crate::interpreter::value::restricted_methods_declared() {
                    self.check_private_value(object, key)?;
                }
                crate::interpreter::executor::set_instance_field(inst, key, value)
                    .map_err(|message| RuntimeError::type_error(message, span))
            }
            (Value::Hash(hash), key) => {
                if crate::interpreter::value::hash_set_value(&mut hash.borrow_mut(), key, value) {
                    Ok(())
                } else {
                    Err(RuntimeError::type_error(
                        format!("Cannot use {} as hash key", key.type_name()),
                        span,
                    ))
                }
            }
            _ => Err(RuntimeError::type_error(
                format!("Cannot set index on {}", object.type_name()),
                span,
            )),
        }
    }
}

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}

/// `MatchArrayLen`: an Array of exactly `len` elements, or of at least `len`
/// when the pattern has a `...rest`. Native length, as in the tree-walker.
#[inline]
fn array_len_matches(value: &Value, len: u16, at_least: u8) -> bool {
    match value {
        Value::Array(arr) => {
            let n = arr.borrow().len();
            if at_least != 0 {
                n >= len as usize
            } else {
                n == len as usize
            }
        }
        _ => false,
    }
}

/// Check if a class (or any of its superclasses) matches the given name.
fn class_name_matches(class: &Class, name: &str) -> bool {
    if class.name == name {
        return true;
    }
    if let Some(ref superclass) = class.superclass {
        return class_name_matches(superclass, name);
    }
    false
}

/// Convert a chunk constant to a runtime Value.
fn constant_to_value(constant: &Constant) -> Value {
    match constant {
        Constant::Int(n) => Value::Int(*n),
        Constant::Float(n) => Value::Float(*n),
        Constant::Decimal(decimal) => Value::Decimal(decimal.clone()),
        Constant::String(s) => Value::String(s.clone()),
        Constant::Bool(b) => Value::Bool(*b),
        Constant::Null => Value::Null,
        Constant::Function(proto) => {
            // A bare function proto becomes a closure with no upvalues
            Value::VmClosure(Rc::new(VmClosure::new(proto.clone(), Vec::new())))
        }
        Constant::HashKeys(_) => {
            // Never loaded as a Value — only consumed by Op::HashWithKeys.
            unreachable!("HashKeys constant should not be loaded as a Value")
        }
        Constant::ArgNames(_) => {
            // Never loaded as a Value — only consumed by Op::CallNamed/NewNamed.
            unreachable!("ArgNames constant should not be loaded as a Value")
        }
        Constant::LiteralList(_) => {
            // Never loaded as a Value — only read by Op::LiteralArray and
            // Op::LiteralListIncludes.
            unreachable!("LiteralList constant should not be loaded as a Value")
        }
    }
}

/// Order two numeric scalars for the fast tier's comparisons: `Int`/`Int`
/// and `Float`/`Float`, the two pairs the general arms compare inline.
/// Anything else — mixed numbers, strings, a NaN — is `None`, and the op
/// goes to the general arm, whose comparator owns those rules and errors.
#[inline(always)]
fn scalar_ordering(a: &Value, b: &Value) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => Some(x.cmp(y)),
        (Value::Float(x), Value::Float(y)) => x.partial_cmp(y),
        _ => None,
    }
}

/// `a == b` as the `==` and `!=` operators define it, on both engines: Ints
/// and Bools inline, everything else through `enum_aware_equal`, which the
/// tree-walker's operators call too. Every general arm that compares two
/// values for `==` or `!=` decides with this, fused or not: `Equal`,
/// `NotEqual`, `TestEqualJump`, `TestNotEqualJump`, `EqualLocalLocal`,
/// `NotEqualLocalLocal`, and `EqualLocalConst`/`NotEqualLocalConst` past
/// their inline pairs. The fast tier answers the primitive pairs itself with
/// [`primitive_equal`], which agrees with this on every pair it answers.
///
/// `PartialEq` on `Value` is not this equality. It compares two instances by
/// pointer, so two equal enum values built apart (any payload variant, or an
/// `enum_field` read rebuilt from storage) differ under it. `TestNotEqualJump`
/// used it, and `if a != b` took the branch that `a != b` as a value did not.
#[inline(always)]
fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        _ => crate::interpreter::value::enum_aware_equal(a, b),
    }
}

/// Whether `value` is one of the kinds `primitive_equal` answers for.
#[inline(always)]
fn is_primitive(value: &Value) -> bool {
    matches!(
        value,
        Value::Int(_)
            | Value::Float(_)
            | Value::Bool(_)
            | Value::Null
            | Value::String(_)
            | Value::Symbol(_)
    )
}

/// `a / b` (`divide`) or `a % b` where the fast tier may answer it: two Ints
/// with a quotient or remainder that fits (not a zero divisor, not
/// `i64::MIN / -1`), or `/` on two Floats with a non-zero divisor. `None`
/// for everything the general arms raise on or convert (a Float `%`, an
/// Int/Float mix, a Decimal).
#[inline(always)]
fn fast_quotient(a: &Value, b: &Value, divide: bool) -> Option<Value> {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => if divide {
            x.checked_div(*y)
        } else {
            x.checked_rem(*y)
        }
        .map(Value::Int),
        (Value::Float(x), Value::Float(y)) if divide && *y != 0.0 => Some(Value::Float(x / y)),
        _ => None,
    }
}

/// Whether dropping `value` is a no-op: the scalars `replace_top_two!`
/// may overwrite without running drop glue.
#[inline(always)]
fn owns_no_heap(value: &Value) -> bool {
    matches!(
        value,
        Value::Int(_) | Value::Float(_) | Value::Bool(_) | Value::Null
    )
}

/// [`values_equal`] for two primitives (Int, Float, Bool, nil, String,
/// Symbol), answered inline by the fast tier; `None` for anything else.
///
/// It is `eq_inner` restricted to those kinds: the same kind compares by
/// value (a Float NaN equals nothing), two different kinds are never equal,
/// except an Int and a Float, which compare as numbers and are left to the
/// general arm. Everything that is not a primitive declines: a `Deferred`
/// must be forced, an enum compares structurally, a class equals its name.
#[inline(always)]
fn primitive_equal(a: &Value, b: &Value) -> Option<bool> {
    Some(match (a, b) {
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Float(x), Value::Float(y)) => x == y,
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Symbol(x), Value::Symbol(y)) => x == y,
        (Value::Null, Value::Null) => true,
        (Value::Int(_), Value::Float(_)) | (Value::Float(_), Value::Int(_)) => return None,
        _ if is_primitive(a) && is_primitive(b) => false,
        _ => return None,
    })
}

/// [`primitive_equal`] against a constant-pool entry, without building a
/// `Value` for it. A `Decimal` or a function constant declines.
#[inline(always)]
fn primitive_equal_constant(value: &Value, constant: &Constant) -> Option<bool> {
    Some(match (value, constant) {
        (Value::Int(x), Constant::Int(y)) => x == y,
        (Value::Bool(x), Constant::Bool(y)) => x == y,
        (Value::Float(x), Constant::Float(y)) => x == y,
        (Value::String(x), Constant::String(y)) => x == y,
        (Value::Null, Constant::Null) => true,
        (Value::Int(_), Constant::Float(_)) | (Value::Float(_), Constant::Int(_)) => return None,
        _ if is_primitive(value)
            && matches!(
                constant,
                Constant::Int(_)
                    | Constant::Float(_)
                    | Constant::Bool(_)
                    | Constant::Null
                    | Constant::String(_)
            ) =>
        {
            false
        }
        _ => return None,
    })
}

/// Drop a value the fast tier has finished with. `Value`'s drop glue is an
/// out-of-line jump table even for a scalar, and the ops in `run_fast` —
/// `Pop`, `SetLocal`, a popped jump condition — discard a scalar far more
/// often than not.
/// `Value::clone` with the scalars copied inline: the derived impl is a
/// call through a jump table even for an integer, and the fast tier copies
/// a local on nearly every op.
#[inline(always)]
pub(crate) fn clone_scalar_fast(value: &Value) -> Value {
    match value {
        Value::Int(n) => Value::Int(*n),
        Value::Float(f) => Value::Float(*f),
        Value::Bool(b) => Value::Bool(*b),
        Value::Null => Value::Null,
        other => other.clone(),
    }
}

/// Whether reading `name` on `inst` means more than its stored field: a
/// model's declared relation or translated field, which `op_get_property`
/// hands to the tree-walker's resolver. `GetPropertyOrNull`'s field fast
/// path leaves those to it. One memoised flag for any other class.
#[inline]
fn field_needs_model_resolution(
    inst: &Rc<RefCell<crate::interpreter::value::Instance>>,
    name: &str,
) -> bool {
    let inst = inst.borrow();
    inst.class.is_model_subclass()
        && crate::interpreter::builtins::model::member_needs_model_resolution(
            &inst.class.name,
            name,
        )
}

#[inline(always)]
pub(crate) fn discard(value: Value) {
    if matches!(
        value,
        Value::Int(_) | Value::Float(_) | Value::Bool(_) | Value::Null
    ) {
        // Owns no heap data: nothing to release.
        std::mem::forget(value);
    } else {
        drop(value);
    }
}

/// Whether a bare read of `value` calls it, the tree-walker's rule for a
/// bare name (`can_auto_invoke_with_no_args`): a def that takes no arguments
/// (all defaulted counts), or a builtin that is called bare. A lambda kept in
/// a variable is left alone — `<lambda>` is the name both engines give one.
pub(crate) fn auto_invocable(value: &Value) -> bool {
    match value {
        Value::Function(func) => func.name != "<lambda>" && func.arity() == 0,
        Value::VmClosure(closure) => closure.proto.name != "<lambda>" && closure.proto.arity == 0,
        Value::NativeFunction(func) => func.is_auto_invocable || func.arity == Some(0),
        _ => false,
    }
}

#[cfg(test)]
thread_local! {
    /// Test-only: the instance `GetProperty`, `GetLocalProperty` and
    /// `SetProperty` ops this thread's general arms ran — the ones the fast
    /// tier handed back.
    static GENERAL_FIELD_OPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
thread_local! {
    /// Test-only: the `GetProperty`, `GetLocalProperty` and `SetProperty` ops
    /// this thread's general arms ran on anything but an instance — a
    /// paren-less `h.length` or `s.blank?` the fast tier handed back.
    static GENERAL_MEMBER_OPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Test-only: count a property op the general arm runs on `receiver`: as a
/// field op when that is an instance, as a member op otherwise.
#[cfg(test)]
fn tally_general_field_op(receiver: &Value) {
    if matches!(receiver, Value::Instance(_)) {
        GENERAL_FIELD_OPS.with(|ops| ops.set(ops.get() + 1));
    } else {
        GENERAL_MEMBER_OPS.with(|ops| ops.set(ops.get() + 1));
    }
}

/// Test-only: how many non-instance property ops the general arms have run
/// on this thread.
#[cfg(test)]
pub(crate) fn general_member_ops() -> usize {
    GENERAL_MEMBER_OPS.with(std::cell::Cell::get)
}

/// Test-only: how many instance field ops the general arms have run on this
/// thread, for a test to tell the fast tier's field reads and writes from
/// the general arm's.
#[cfg(test)]
pub(crate) fn general_field_ops() -> usize {
    GENERAL_FIELD_OPS.with(std::cell::Cell::get)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::value::NativeFunction;
    use crate::lexer::Scanner;
    use crate::parser::Parser;
    use crate::vm::compiler::Compiler;

    /// Infinite Soli recursion must yield a catchable RuntimeError, never a
    /// native stack overflow (which aborts without unwinding).
    #[test]
    fn unbounded_recursion_returns_error_not_crash() {
        let source = r#"
            fn loop_forever() {
                loop_forever();
            }
            loop_forever();
        "#;
        let result = compile_and_run(source);
        let err = result.expect_err("unbounded recursion must error");
        assert!(
            err.to_string().contains("too deep"),
            "unexpected error: {err}"
        );
    }

    fn compile_and_run(source: &str) -> Result<Value, crate::error::RuntimeError> {
        let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
        let program = Parser::new(tokens).parse().expect("parser error");
        let module = Compiler::compile(&program).expect("compile error");
        let mut vm = Vm::new();
        vm.globals.insert(
            "print".to_string(),
            Value::NativeFunction(NativeFunction::new("print", None, |_| Ok(Value::Null))),
        );
        vm.globals.insert(
            "puts".to_string(),
            Value::NativeFunction(NativeFunction::new("puts", None, |_| Ok(Value::Null))),
        );
        vm.execute(&module.main)
    }

    /// A method `define_method` gave a primitive type runs, or not, on the
    /// same calls in both engines.
    ///
    /// The tree-walker runs the builtin for a block-less paren call on a Hash,
    /// Array or String whose name is in that type's direct-dispatch list, and
    /// the user method for every other call. The VM used to decide by whether
    /// the name had a method id instead: `CallMethod` (no id) put the user
    /// method first for every name, so `xs.includes?(1)` ran an override the
    /// tree-walker ignores, and `CallMethodById` never looked, so
    /// `xs.count { … }`, `"ab".capitalize()` and `5.to_s()` ignored overrides
    /// the tree-walker runs.
    ///
    /// The CLI's VM cannot register a primitive method itself (a compiled
    /// closure is refused), so the tree-walker registers them here, as a serve
    /// app's tree-walked code would, and both engines then run the probe. Its
    /// own thread: the overlay tables are thread-local and die with it.
    #[test]
    fn test_vm_primitive_user_method_precedence_matches_tree_walker() {
        const SETUP: &str = r#"
Array.define_method("includes?", fn(v) { "user includes?" })
Array.define_method("select", fn(f) { "user select" })
Array.define_method("sum", fn() { "user sum" })
Array.define_method("count", fn(f) { "user count" })
String.define_method("contains?", fn(v) { "user contains?" })
String.define_method("capitalize", fn() { "user capitalize" })
Hash.define_method("map", fn(f) { "user map" })
Hash.define_method("select", fn(f) { "user select" })
Int.define_method("to_s", fn() { "user to_s" })
"#;
        const PROBE: &str = r#"
xs = [1, 2]
h = {"a": 1}
print(xs.includes?(1))
print(xs&.includes?(1))
print(xs.select(fn(x) { x > 1 }))
print(xs.select { |x| x > 1 })
print(xs.sum())
print(xs.sum)
print(xs.count { |x| x > 1 })
print("ab".contains?("a"))
print("ab".capitalize())
print("ab".upcase())
print(h.map(fn(k, v) { [k, v * 2] }))
print(h.select { |k, v| true })
print(5.to_s())
print(5.abs())
"#;
        const EXPECTED: &str = "true\ntrue\n[2]\nuser select\n3\nuser sum\nuser count\n\
true\nuser capitalize\nAB\n{a => 2}\nuser select\nuser to_s\n5\n";

        fn parse(src: &str) -> crate::ast::Program {
            let tokens = Scanner::new(src).scan_tokens().expect("lexer error");
            Parser::new(tokens).parse().expect("parser error")
        }

        std::thread::spawn(|| {
            let capture = crate::interpreter::builtins::StdoutCaptureGuard::start();
            crate::interpreter::Interpreter::new()
                .interpret(&parse(&format!("{SETUP}{PROBE}")))
                .expect("tree-walker run");
            let tree = String::from_utf8(capture.finish()).expect("utf-8");

            let module = Compiler::compile(&parse(PROBE)).expect("compile error");
            let capture = crate::interpreter::builtins::StdoutCaptureGuard::start();
            Vm::new().execute(&module.main).expect("vm run");
            let vm = String::from_utf8(capture.finish()).expect("utf-8");

            assert_eq!(tree, EXPECTED, "tree-walker");
            assert_eq!(vm, EXPECTED, "VM");
        })
        .join()
        .expect("probe thread");
    }

    /// `print`/`puts`/`println` compile to `Op::Print`, which must write
    /// through `write_captured_or_stdout` like the tree-walker's builtins:
    /// while a `StdoutCaptureGuard` is alive (test runner, graph builder) the
    /// line lands in the capture buffer, not on the process stdout. It used to
    /// `println!` straight to stdout and also keep a copy of every line in a
    /// `Vm.output` buffer nothing read, which grew a serve worker's memory
    /// without bound.
    #[test]
    fn test_vm_print_op_writes_through_stdout_capture() {
        let tokens = Scanner::new("print(\"a\", 1)\nputs(\"b\")\nprintln([2, nil])\nprint()")
            .scan_tokens()
            .expect("lexer error");
        let program = Parser::new(tokens).parse().expect("parser error");
        let module = Compiler::compile(&program).expect("compile error");
        let mut vm = Vm::new();
        let capture = crate::interpreter::builtins::StdoutCaptureGuard::start();
        vm.execute(&module.main).expect("print must run");
        let captured = String::from_utf8(capture.finish()).expect("utf-8");
        assert_eq!(captured, "a 1\nb\n[2, null]\n\n");
    }

    /// A match expression must leave the value stack balanced.
    ///
    /// A literal arm's `Equal` consumes the duplicated subject, but
    /// `compile_match` popped again unconditionally, so every literal-arm match
    /// left the stack one slot short. Nothing observed it until something read
    /// the stack by depth — a `catch` binding does, so the exception variable
    /// resolved to a stale slot (binding a fragment of an unrelated string), and
    /// one shape indexed out of bounds and panicked the VM outright, which in a
    /// release build (`panic = "abort"`) takes the worker down.
    #[test]
    fn test_vm_match_leaves_stack_balanced_for_following_catch() {
        assert_eq!(
            compile_and_get_global(
                "let x = \"unset\"\nmatch 42 { 42 => \"a\", _ => \"b\" }\ntry { throw \"boom\" } catch e { x = e }",
                "x"
            ),
            Value::String("boom".into())
        );
        // Same shape, but with the match bound to a variable first.
        assert_eq!(
            compile_and_get_global(
                "let m = match 42 { 42 => \"a\", _ => \"b\" }\nlet x = \"unset\"\ntry { throw \"boom\" } catch e { x = e }",
                "x"
            ),
            Value::String("boom".into())
        );
        // The match itself must still produce the right arm.
        assert_eq!(
            compile_and_get_global("let m = match 42 { 42 => \"a\", _ => \"b\" };", "m"),
            Value::String("a".into())
        );
        assert_eq!(
            compile_and_get_global("let m = match 7 { 1 => \"one\", _ => \"fb\" };", "m"),
            Value::String("fb".into())
        );
    }

    /// Parameter defaults must be evaluated by the callee's prologue.
    ///
    /// These previously bound `null`: the compiler counted defaults into
    /// `proto.defaults` but never compiled the expressions, and the call path
    /// pushed `Value::Null` into every omitted slot. Because that produced no
    /// error it triggered no engine fallback either, so a handler using default
    /// parameters silently computed wrong values in production (VM) while
    /// working in `--dev` (interpreter).
    #[test]
    fn test_vm_parameter_defaults_are_evaluated() {
        // Fully omitted.
        assert_eq!(
            compile_and_get_global("def f(a = 1, b = 2) { return a + b }\nlet x = f();", "x"),
            Value::Int(3)
        );
        // Partially supplied — only the omitted tail defaults.
        assert_eq!(
            compile_and_get_global("def f(a = 1, b = 2) { return a + b }\nlet x = f(10);", "x"),
            Value::Int(12)
        );
        // Required parameter followed by a defaulted one.
        assert_eq!(
            compile_and_get_global("def f(a, b = 5) { return a * b }\nlet x = f(3);", "x"),
            Value::Int(15)
        );
        // A default is an arbitrary expression, not just a literal.
        assert_eq!(
            compile_and_get_global("def f(a = 2 + 3) { return a }\nlet x = f();", "x"),
            Value::Int(5)
        );
        // Later defaults may reference earlier parameters (slots fill in order).
        assert_eq!(
            compile_and_get_global("def f(a = 2, b = a * 10) { return b }\nlet x = f();", "x"),
            Value::Int(20)
        );
        // Constructors take the same prologue.
        assert_eq!(
            compile_and_get_global(
                "class C { v: Int; new(v = 7) { this.v = v } }\nlet x = C().v;",
                "x"
            ),
            Value::Int(7)
        );
    }

    /// An explicitly passed `null` occupies its slot: it is *supplied*, so the
    /// default must not overwrite it. This is why the frame tracks a
    /// supplied-parameter bitmask rather than just an argument count.
    #[test]
    fn test_vm_explicit_null_argument_beats_default() {
        assert_eq!(
            compile_and_get_global(
                "def f(a = 1, b = 2) { return [a, b] }\nlet x = f(null)[0];",
                "x"
            ),
            Value::Null
        );
        // ...while the parameter after it still defaults.
        assert_eq!(
            compile_and_get_global(
                "def f(a = 1, b = 2) { return [a, b] }\nlet x = f(null)[1];",
                "x"
            ),
            Value::Int(2)
        );
    }

    /// Named arguments bind by parameter name at call time, and interact
    /// correctly with defaults: labelled slots are supplied, the rest default.
    #[test]
    fn test_vm_named_arguments_bind_by_name() {
        assert_eq!(
            compile_and_get_global(
                "def f(host = \"local\", port = 80) { return port }\nlet x = f(port: 3000);",
                "x"
            ),
            Value::Int(3000)
        );
        // Written order is irrelevant.
        assert_eq!(
            compile_and_get_global("def f(a, b) { return a - b }\nlet x = f(b: 1, a: 10);", "x"),
            Value::Int(9)
        );
        // Mixed positional + labelled, with a defaulted gap between them.
        assert_eq!(
            compile_and_get_global(
                "def f(a, b = 10, c = 100) { return a + b + c }\nlet x = f(1, c: 5);",
                "x"
            ),
            Value::Int(16)
        );
        // An unfilled parameter with no default is still an arity error.
        let src = "def f(a, b = 2) { return a }\nlet x = f(b: 1);";
        let tokens = Scanner::new(src).scan_tokens().expect("lexer error");
        let program = Parser::new(tokens).parse().expect("parser error");
        let module = Compiler::compile(&program).expect("compile error");
        let mut vm = Vm::new();
        assert!(
            vm.execute(&module.main).is_err(),
            "omitting a required parameter must not silently bind null"
        );
    }

    /// A field default that is not a literal runs once for each new instance,
    /// as on the tree-walker — never when the class is defined. The compiler
    /// evaluated every initializer in the class body as well (and threw the
    /// value away), so a default with a side effect ran once too often.
    #[test]
    fn test_vm_computed_field_default_runs_per_instance_only() {
        let source = r##"
let calls = 0
def tick()
  calls = calls + 1
  calls
end
class Stamped
  stamp: Int = tick()
  tag: String = "t"
end
let before = calls
let a = new Stamped()
let b = new Stamped()
let result = "#{before} #{a.stamp} #{b.stamp} #{a.tag} #{calls}"
"##;
        assert_eq!(
            compile_and_get_global(source, "result"),
            Value::String("0 1 2 t 2".into())
        );
    }

    /// The defaults that are not literals run in the activation that builds
    /// the instance, as frames above its constructor's — or above the
    /// instance when there is none — and store each field themselves. Every
    /// `new` ran them in a nested `run()` that built a hash of their values.
    /// A class that adds fields below one with such defaults still runs
    /// them class by class, nested.
    #[test]
    fn test_vm_computed_field_defaults_run_without_a_nested_run() {
        use crate::interpreter::value::FIELD_DEFAULTS_METHOD;
        let source = r##"
calls = 0
def tick()
  calls = calls + 1
  calls
end
class Stamp
  n: Int = 0
  at: Int = tick()
  twice: Int = @at * 2

  new(x)
    @n = x + @at
  end
end
class Bare
  at: Int = tick()
  tags: Array = [:a]
end
class Kid < Stamp
end
"##;
        let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
        let program = Parser::new(tokens).parse().expect("parser error");
        let module = Compiler::compile(&program).expect("compile error");
        let mut vm = Vm::new();
        vm.execute(&module.main).expect("vm error");
        let Some(Value::Class(stamp)) = vm.globals.get("Stamp").cloned() else {
            panic!("Stamp is a class");
        };
        let computed = stamp
            .field_init
            .borrow()
            .as_ref()
            .and_then(|init| init.computed.clone())
            .expect("`at` and `twice` have a method");
        assert_eq!(computed.proto.name, FIELD_DEFAULTS_METHOD);
        assert!(
            !computed
                .proto
                .chunk
                .code
                .iter()
                .any(|op| matches!(op, Op::Hash(_) | Op::HashWithKeys(..))),
            "the defaults are stored, not collected into a hash"
        );

        let use_source = r##"
a = new Stamp(10)
b = Stamp(20)
c = Stamp.new(30)
d = Bare.new()
e = new Bare()
f = Kid.new(40)
result = "#{a.n} #{a.at} #{a.twice} #{b.n} #{c.n} #{d.at} #{d.tags.length} #{e.at} #{f.n} #{f.twice} #{calls}"
"##;
        let tokens = Scanner::new(use_source).scan_tokens().expect("lexer error");
        let program = Parser::new(tokens).parse().expect("parser error");
        let module = Compiler::compile(&program).expect("compile error");
        let before = crate::vm::vm_domain::nested_runs();
        vm.execute(&module.main).expect("vm error");
        assert_eq!(
            crate::vm::vm_domain::nested_runs(),
            before,
            "no construction started a nested run"
        );
        assert_eq!(
            vm.globals.get("result"),
            Some(&Value::String("11 1 2 22 33 4 1 5 46 12 6".into()))
        );
    }

    /// The field defaults are not compiled into the constructor. `Op::Field`
    /// keeps each field's starting value (its literal default, else nil),
    /// and the defaults that are not literals compile into one method kept
    /// out of the class's methods. Baked into `init`, a default ran again
    /// whenever `super(...)` entered a parent's constructor, after the
    /// child's own, and a field without a default was never set at all.
    #[test]
    fn test_vm_field_defaults_are_not_part_of_the_constructor() {
        use crate::interpreter::value::FIELD_DEFAULTS_METHOD;
        let source = r##"
def tick()
  1
end
class Account
  balance: Int = 5
  owner: String
  opened: Int = tick()
  tags: Array = ["a"]
  floor: Int = -3

  new(owner)
    @owner = owner
  end
end
"##;
        let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
        let program = Parser::new(tokens).parse().expect("parser error");
        let module = Compiler::compile(&program).expect("compile error");
        let mut vm = Vm::new();
        vm.execute(&module.main).expect("vm error");
        let Some(Value::Class(class)) = vm.globals.get("Account").cloned() else {
            panic!("Account is a class");
        };

        let init = class
            .field_init
            .borrow()
            .clone()
            .expect("the class records its fields");
        let names: Vec<&str> = init.fields.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, ["balance", "owner", "opened", "tags", "floor"]);
        assert_eq!(init.fields[0].1, Value::Int(5));
        assert_eq!(init.fields[1].1, Value::Null);
        assert_eq!(init.fields[2].1, Value::Null, "evaluated per instance");
        assert!(matches!(&init.fields[3].1, Value::Array(items) if items.borrow().len() == 1));
        assert_eq!(
            init.fields[4].1,
            Value::Int(-3),
            "a negative number is a literal"
        );
        assert!(init.computed.is_some(), "`opened` has its own method");
        assert!(
            class
                .vm_methods
                .borrow()
                .get(FIELD_DEFAULTS_METHOD)
                .is_none(),
            "the defaults method is not a method of the class"
        );

        let ctor = class.vm_init.borrow().clone().expect("the constructor");
        let assigned: Vec<String> = ctor
            .proto
            .chunk
            .code
            .iter()
            .filter_map(|op| match op {
                Op::SetProperty(idx) => match &ctor.proto.chunk.constants[*idx as usize] {
                    Constant::String(name) => Some(name.to_string()),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        assert_eq!(
            assigned,
            ["owner"],
            "the constructor assigns what its body assigns, and no default"
        );
    }

    /// A `return` ends a constructor, and `new` is still the instance — what
    /// the tree-walker gives, which discards a constructor's value. The
    /// compiler compiled the `return` as `Null; Return`, so a guard made
    /// `new Guarded(5)` itself nil, and the next read of it raised.
    #[test]
    fn test_vm_constructor_return_yields_this() {
        let source = r##"
class Guarded
  x: Int

  new(a)
    @x = a
    return if a > 1

    @x = 0
  end
end
class Looping
  n: Int

  new(limit)
    @n = 0
    while true
      @n = @n + 1
      return if @n >= limit
    end
  end
end
let g = new Guarded(5)
let h = Guarded(0)
let l = new Looping(3)
let result = "#{g.x} #{h.x} #{l.n}"
"##;
        assert_eq!(
            compile_and_get_global(source, "result"),
            Value::String("5 0 3".into())
        );

        // Every way out of the compiled `init` returns `this` (slot 0).
        let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
        let program = Parser::new(tokens).parse().expect("parser error");
        let module = Compiler::compile(&program).expect("compile error");
        let mut vm = Vm::new();
        vm.execute(&module.main).expect("vm error");
        for name in ["Guarded", "Looping"] {
            let Some(Value::Class(class)) = vm.globals.get(name).cloned() else {
                panic!("{name} is a class");
            };
            let ctor = class.vm_init.borrow().clone().expect("the constructor");
            let code: Vec<Op> = ctor
                .proto
                .chunk
                .code
                .iter()
                .copied()
                .filter(|op| !matches!(op, Op::Nop))
                .collect();
            let returns: Vec<Op> = code
                .windows(2)
                .filter(|pair| matches!(pair[1], Op::Return))
                .map(|pair| pair[0])
                .collect();
            assert_eq!(
                returns,
                [Op::GetLocal(0), Op::GetLocal(0)],
                "{name}: the early return and the end both return `this`"
            );
        }
    }

    fn compile_and_get_global(source: &str, name: &str) -> Value {
        let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
        let program = Parser::new(tokens).parse().expect("parser error");
        let module = Compiler::compile(&program).expect("compile error");
        let mut vm = Vm::new();
        vm.globals.insert(
            "print".to_string(),
            Value::NativeFunction(NativeFunction::new("print", None, |_| Ok(Value::Null))),
        );
        vm.globals.insert(
            "puts".to_string(),
            Value::NativeFunction(NativeFunction::new("puts", None, |_| Ok(Value::Null))),
        );
        vm.execute(&module.main).expect("vm error");
        vm.globals.get(name).cloned().unwrap_or(Value::Null)
    }

    /// A name a function both bare-assigns and binds with `catch`, `for`, a
    /// comprehension or a nested `let` is a local of that function. It was
    /// left out of hoisting, so its assignment compiled to `SetGlobal`: a
    /// recursive call overwrote the caller's value, and under `soli serve`
    /// the worker VM kept it for the next request (`e = params["secret"]`
    /// in an action that also has `catch e`).
    #[test]
    fn test_vm_local_also_bound_by_an_inner_binding_is_not_a_global() {
        let source = r##"
def summary(items, depth)
  item = items[0]
  if depth > 0
    summary(["inner"], depth - 1)
  end
  names = [item.upcase for item in items]
  "#{item}:#{names.length}"
end
def guarded(secret)
  e = secret
  try
    throw "boom"
  catch e
    nil
  end
  for i in [1, 2]
    nil
  end
  i = 5
  if true
    let n = 1
  end
  n = 2
  "#{e}:#{i}:#{n}"
end
let s = summary(["outer", "b"], 1)
let g = guarded("alice-password")
"##;
        let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
        let program = Parser::new(tokens).parse().expect("parser error");
        let module = Compiler::compile(&program).expect("compile error");
        let mut vm = Vm::new();
        vm.execute(&module.main).expect("vm error");
        assert_eq!(
            vm.globals.get("s").cloned(),
            Some(Value::String("outer:2".into()))
        );
        assert_eq!(
            vm.globals.get("g").cloned(),
            Some(Value::String("alice-password:5:2".into()))
        );
        for leaked in ["item", "e", "i", "n"] {
            assert!(
                !vm.globals.contains_key(leaked),
                "`{leaked}` leaked into the VM globals"
            );
        }
    }

    #[test]
    fn test_vm_arithmetic() {
        let result = compile_and_get_global("let x = 2 + 3 * 4;", "x");
        assert_eq!(result, Value::Int(14));
    }

    /// Run hand-built bytecode as a script. Slot 0 holds the running closure,
    /// so the first value `code` pushes is local 1, the next local 2, and so on.
    fn run_bytecode(
        constants: Vec<Constant>,
        code: &[Op],
    ) -> Result<Value, crate::error::RuntimeError> {
        run_bytecode_with_globals(constants, code, &[])
    }

    /// [`run_bytecode`] with `globals` defined first: `Op::GetGlobal` pushes
    /// the values no constant can hold, such as instances.
    fn run_bytecode_with_globals(
        constants: Vec<Constant>,
        code: &[Op],
        globals: &[(&str, Value)],
    ) -> Result<Value, crate::error::RuntimeError> {
        let mut proto = FunctionProto::new(String::new());
        proto.chunk.constants = constants;
        for op in code {
            proto.chunk.emit(*op, 1);
        }
        let mut vm = Vm::new();
        for (name, value) in globals {
            vm.globals.insert(name.to_string(), value.clone());
        }
        vm.execute(&Arc::new(proto))
    }

    /// The fused local ops raise `Op::Add`'s overflow error where an Int
    /// result leaves the range. Their general arms used an unchecked `+`/`-`
    /// (`wrapping_add` in `AddLocalsInPlace`), and release builds do not check
    /// overflow, so the result wrapped: `i += 1` on the largest Int gave the
    /// smallest. Built by hand so that `IncrLocalFast` and `AddLocalInt`, which
    /// the compiler does not emit, are covered too.
    #[test]
    fn fused_int_ops_raise_on_overflow_instead_of_wrapping() {
        const MAX: u16 = 0;
        const MIN: u16 = 1;
        const ONE: u16 = 2;
        const ZERO: u16 = 3;
        const KEY: u16 = 4;
        let constants = || {
            vec![
                Constant::Int(i64::MAX),
                Constant::Int(i64::MIN),
                Constant::Int(1),
                Constant::Int(0),
                Constant::String("k".into()),
            ]
        };
        // Locals 2-4 are {"k": 1}, ["k"] and 0, so `AddNestedIndex(1, 2, 3, 4)`
        // adds {"k": 1}[["k"][0]] — that is 1 — to local 1.
        let add_one_nested = [
            Op::Constant(KEY),
            Op::Constant(ONE),
            Op::Hash(1),
            Op::Constant(KEY),
            Op::Array(1),
            Op::Constant(ZERO),
            Op::AddNestedIndex(1, 2, 3, 4),
        ];
        let max_plus_one = "integer overflow: 9223372036854775807 + 1 does not fit in an Int";
        let cases: Vec<(Vec<Op>, &str)> = vec![
            (vec![Op::Constant(MAX), Op::IncrLocal(1)], max_plus_one),
            (vec![Op::Constant(MAX), Op::IncrLocalFast(1)], max_plus_one),
            (
                vec![Op::Constant(MIN), Op::DecrLocal(1)],
                "integer overflow: -9223372036854775808 - 1 does not fit in an Int",
            ),
            (vec![Op::Constant(MAX), Op::AddLocalInt(1, 1)], max_plus_one),
            (
                vec![Op::Constant(MIN), Op::AddLocalInt(1, -1)],
                "integer overflow: -9223372036854775808 + -1 does not fit in an Int",
            ),
            (
                vec![
                    Op::Constant(MAX),
                    Op::Constant(ONE),
                    Op::AddLocalsInPlace(1, 2),
                ],
                max_plus_one,
            ),
            (
                [&[Op::Constant(MAX)][..], &add_one_nested[..]].concat(),
                max_plus_one,
            ),
        ];
        for (code, expected) in &cases {
            let err = match run_bytecode(constants(), code) {
                Ok(value) => panic!("{code:?} must raise an overflow, got {value:?}"),
                Err(err) => err,
            };
            assert!(err.to_string().contains(expected), "{code:?}: {err}");
        }

        // Up to the edge they still compute. `DecrLocal` and `AddNestedIndex`
        // have no fast arm, so every execution of them runs this code.
        let to_min = [
            Op::Constant(MIN),
            Op::IncrLocal(1),
            Op::DecrLocal(1),
            Op::GetLocal(1),
            Op::Return,
        ];
        assert_eq!(
            run_bytecode(constants(), &to_min).expect("MIN + 1 - 1 is in range"),
            Value::Int(i64::MIN)
        );
        let to_max = [
            &[Op::Constant(MAX), Op::DecrLocal(1)][..],
            &add_one_nested[..],
            &[Op::GetLocal(1), Op::Return][..],
        ]
        .concat();
        assert_eq!(
            run_bytecode(constants(), &to_max).expect("MAX - 1 + 1 is in range"),
            Value::Int(i64::MAX)
        );
    }

    /// `h["k"] = v` on a hash without `"k"` inserts a clone of the key
    /// constant, so a key longer than `EcoString`'s 15 inline bytes shares the
    /// constant's buffer. The three ops inserted `key.to_string().into()`: a
    /// heap `String`, then a second copy into a new `EcoString`.
    #[test]
    fn hash_set_const_ops_share_the_key_constant() {
        const KEY: u16 = 0;
        const ONE: u16 = 1;
        const GLOBAL: u16 = 2;
        let key: crate::interpreter::value::SoliStr = "content_type_header_value".into();
        assert!(key.len() > 15, "the key must not fit inline");
        let constants = || {
            vec![
                Constant::String(key.clone()),
                Constant::Int(1),
                Constant::String("h".into()),
            ]
        };
        let shapes: Vec<(&str, Vec<Op>)> = vec![
            (
                "HashSetConst",
                vec![
                    Op::Hash(0),
                    Op::GetLocal(1),
                    Op::Constant(ONE),
                    Op::HashSetConst(KEY),
                    Op::Pop,
                    Op::GetLocal(1),
                    Op::Return,
                ],
            ),
            (
                "HashSetLocalConst",
                vec![
                    Op::Hash(0),
                    Op::Constant(ONE),
                    Op::HashSetLocalConst(1, KEY),
                    Op::Pop,
                    Op::GetLocal(1),
                    Op::Return,
                ],
            ),
            (
                "HashSetGlobalConst",
                vec![
                    Op::Constant(ONE),
                    Op::HashSetGlobalConst(GLOBAL, KEY),
                    Op::Pop,
                    Op::GetGlobal(GLOBAL),
                    Op::Return,
                ],
            ),
        ];
        for (name, code) in &shapes {
            let globals = [("h", crate::interpreter::value::empty_hash())];
            let value = run_bytecode_with_globals(constants(), code, &globals)
                .unwrap_or_else(|err| panic!("{name} raised: {err}"));
            let Value::Hash(hash) = value else {
                panic!("{name} did not return the hash: {value:?}");
            };
            let hash = hash.borrow();
            assert_eq!(hash.len(), 1, "{name}");
            let (stored_key, stored_value) = hash.get_index(0).unwrap();
            let HashKey::String(stored) = stored_key else {
                panic!("{name} stored a non-string key: {stored_key:?}");
            };
            assert_eq!(stored.as_str(), key.as_str(), "{name}");
            assert_eq!(
                stored.as_ptr(),
                key.as_ptr(),
                "{name} copied the key instead of sharing the constant"
            );
            assert_eq!(stored_value, &Value::Int(1), "{name}");
            // A shared key still hashes and compares like a borrowed one.
            assert_eq!(
                hash.get(&StrKey(key.as_str())),
                Some(&Value::Int(1)),
                "{name}"
            );
        }
    }

    /// `for ch in str` yields one-character strings, multi-byte ones included;
    /// each is built inline from the char's UTF-8 bytes.
    #[test]
    fn string_iteration_yields_each_character() {
        let source = "let out = []\nfor ch in \"a\u{e9}\u{1F600}z\"\n  out.push(ch)\nend\nlet joined = out.join(\"|\")\nlet n = out.length";
        assert_eq!(
            compile_and_get_global(source, "joined"),
            Value::String("a|\u{e9}|\u{1F600}|z".into())
        );
        assert_eq!(compile_and_get_global(source, "n"), Value::Int(4));
    }

    /// The source shapes the peephole turns into those ops: each must really
    /// compile to the fused op, and raise the overflow the tree-walker raises.
    #[test]
    fn fused_local_arithmetic_raises_on_overflow() {
        let largest = "i = 9223372036854775807";
        let smallest = "i = -9223372036854775807 - 1";
        let with_local = "i = 9223372036854775807\n  one = 1";
        let with_keys = "i = 9223372036854775807\n  h = {\"k\": 1}\n  names = [\"k\"]\n  k = 0";
        let max_plus_one = "integer overflow: 9223372036854775807 + 1 does not fit in an Int";
        let min_minus_one = "integer overflow: -9223372036854775808 - 1 does not fit in an Int";
        let cases = [
            (largest, "i += 1", Op::IncrLocal(0), max_plus_one),
            (largest, "i = i + 1", Op::IncrLocal(0), max_plus_one),
            (largest, "i++", Op::IncrLocal(0), max_plus_one),
            (smallest, "i -= 1", Op::DecrLocal(0), min_minus_one),
            (smallest, "i = i - 1", Op::DecrLocal(0), min_minus_one),
            (smallest, "i--", Op::DecrLocal(0), min_minus_one),
            (
                largest,
                "i += 2",
                Op::AddLocalConstInPlace(0, 0),
                "integer overflow: 9223372036854775807 + 2 does not fit in an Int",
            ),
            (
                largest,
                "i = i + 2",
                Op::AddLocalConstInPlace(0, 0),
                "integer overflow: 9223372036854775807 + 2 does not fit in an Int",
            ),
            (
                smallest,
                "i -= 2",
                Op::SubLocalConstInPlace(0, 0),
                "integer overflow: -9223372036854775808 - 2 does not fit in an Int",
            ),
            (
                largest,
                "i *= 2",
                Op::MulLocalConstInPlace(0, 0),
                "integer overflow: 9223372036854775807 * 2 does not fit in an Int",
            ),
            (
                with_local,
                "i += one",
                Op::AddLocalsInPlace(0, 0),
                max_plus_one,
            ),
            (
                with_local,
                "i = i + one",
                Op::AddLocalsInPlace(0, 0),
                max_plus_one,
            ),
            (
                with_keys,
                "i += h[names[k]]",
                Op::AddNestedIndex(0, 0, 0, 0),
                max_plus_one,
            ),
            (
                with_keys,
                "i = i + h[names[k]]",
                Op::AddNestedIndex(0, 0, 0, 0),
                max_plus_one,
            ),
        ];
        for (setup, statement, fused, expected) in cases {
            // `i` is read afterwards, so the statement carries the `Pop` the
            // fusions need.
            let source = format!("def f()\n  {setup}\n  {statement}\n  i\nend\nf()\n");
            let tokens = Scanner::new(&source).scan_tokens().expect("lexer error");
            let program = Parser::new(tokens).parse().expect("parser error");
            let module = Compiler::compile(&program).expect("compile error");
            let code = module
                .main
                .chunk
                .constants
                .iter()
                .find_map(|constant| match constant {
                    Constant::Function(proto) if proto.name == "f" => {
                        Some(proto.chunk.code.clone())
                    }
                    _ => None,
                })
                .expect("the function f");
            assert!(
                code.iter()
                    .any(|op| std::mem::discriminant(op) == std::mem::discriminant(&fused)),
                "`{statement}` should compile to {fused:?}: {code:?}"
            );
            let err = match Vm::new().execute(&module.main) {
                Ok(value) => panic!("`{statement}` must raise an overflow, got {value:?}"),
                Err(err) => err,
            };
            assert!(err.to_string().contains(expected), "`{statement}`: {err}");
        }
    }

    /// A class, as far as `==` is concerned: an enum value is an instance
    /// carrying a `__variant` tag, so these tests need no enum declaration.
    fn bare_class(name: &str) -> Rc<Class> {
        Rc::new(Class {
            name: name.to_string(),
            ..Default::default()
        })
    }

    /// A unit variant read back from storage, as an `enum_field` read and
    /// `Status.parse("Active")` build it: a new instance on every call.
    fn stored_variant(class: &Rc<Class>, tag: &str) -> Value {
        crate::interpreter::value::build_enum_value(class, &Value::String(tag.into()))
    }

    /// `Shape.Circle(radius)` as `__enum_construct` builds it: a new instance
    /// on every call.
    fn circle(shape: &Rc<Class>, radius: i64) -> Value {
        let mut instance = crate::interpreter::value::Instance::new(shape.clone());
        instance.set("__variant", Value::String("Circle".into()));
        instance.set("radius", Value::Int(radius));
        Value::Instance(Rc::new(RefCell::new(instance)))
    }

    /// Every op that decides `==` or `!=` compares enum values by value, as
    /// the tree-walker does. `TestNotEqualJump` (the fused `if a != b`) and
    /// the general arms of `EqualLocalLocal` and `NotEqualLocalLocal` used
    /// `PartialEq`, which compares two instances by pointer: two equal enum
    /// values built apart were "not equal" to them, and equal to `Equal`.
    /// Built by hand, so every fused shape is covered whatever the
    /// peephole picks for a given source.
    #[test]
    fn enum_values_compare_by_value_in_every_equality_op() {
        let status = bare_class("Status");
        let shape = bare_class("Shape");
        let shared = stored_variant(&status, "Active");
        let point = || {
            let mut instance = crate::interpreter::value::Instance::new(bare_class("Point"));
            instance.set("x", Value::Int(1));
            Value::Instance(Rc::new(RefCell::new(instance)))
        };
        // (pair, a, b, whether a == b)
        let pairs = [
            (
                "two stored Status.Active",
                stored_variant(&status, "Active"),
                stored_variant(&status, "Active"),
                true,
            ),
            (
                "Status.Active and Status.Inactive",
                stored_variant(&status, "Active"),
                stored_variant(&status, "Inactive"),
                false,
            ),
            ("two Circle(1)", circle(&shape, 1), circle(&shape, 1), true),
            (
                "Circle(1) and Circle(2)",
                circle(&shape, 1),
                circle(&shape, 2),
                false,
            ),
            (
                "Status.Active and Mode.Active",
                stored_variant(&status, "Active"),
                stored_variant(&bare_class("Mode"), "Active"),
                false,
            ),
            ("one value twice", shared.clone(), shared, true),
            // Not enums: an instance is still only equal to itself.
            ("two plain instances, same fields", point(), point(), false),
            ("two Ints", Value::Int(3), Value::Int(3), true),
            (
                "two Strings",
                Value::String("a".into()),
                Value::String("b".into()),
                false,
            ),
        ];
        const A: u16 = 0;
        const B: u16 = 1;
        const YES: u16 = 2;
        const NO: u16 = 3;
        let constants = || {
            vec![
                Constant::String("a".into()),
                Constant::String("b".into()),
                Constant::Bool(true),
                Constant::Bool(false),
            ]
        };
        // (op, the code after `a` and `b` are pushed as locals 1 and 2, and
        // whether the Bool it returns answers "equal?" or "not equal?")
        let shapes: [(&str, &[Op], bool); 6] = [
            ("Equal", &[Op::Equal, Op::Return], true),
            ("NotEqual", &[Op::NotEqual, Op::Return], false),
            (
                "EqualLocalLocal",
                &[Op::EqualLocalLocal(1, 2), Op::Return],
                true,
            ),
            (
                "NotEqualLocalLocal",
                &[Op::NotEqualLocalLocal(1, 2), Op::Return],
                false,
            ),
            // `if a != b { true } else { false }`: equal operands jump over
            // the `true` to the `false`.
            (
                "TestNotEqualJump",
                &[
                    Op::TestNotEqualJump(2),
                    Op::Constant(YES),
                    Op::Return,
                    Op::Constant(NO),
                    Op::Return,
                ],
                false,
            ),
            // `if a == b { true } else { false }`.
            (
                "TestEqualJump",
                &[
                    Op::TestEqualJump(2),
                    Op::Constant(YES),
                    Op::Return,
                    Op::Constant(NO),
                    Op::Return,
                ],
                true,
            ),
        ];
        for (pair, a, b, equal) in &pairs {
            let globals = [("a", a.clone()), ("b", b.clone())];
            for &(op, tail, answers_equal) in &shapes {
                let code = [&[Op::GetGlobal(A), Op::GetGlobal(B)][..], tail].concat();
                let answer = run_bytecode_with_globals(constants(), &code, &globals)
                    .unwrap_or_else(|e| panic!("{op} on {pair}: {e}"));
                assert_eq!(
                    answer,
                    Value::Bool(*equal == answers_equal),
                    "{op} on {pair}"
                );
            }
        }

        // `EqualLocalConst` and `NotEqualLocalConst` take their right side
        // from the constant pool, which holds no instances. Past their inline
        // pairs they must still answer what `Equal` does: an enum is not its
        // tag, and the Int 3 equals the Float 3.0.
        let pool = vec![
            Constant::String("a".into()),
            Constant::String("Active".into()),
            Constant::Int(3),
            Constant::Float(3.0),
        ];
        let locals = [
            ("a stored Status.Active", stored_variant(&status, "Active")),
            ("the Int 3", Value::Int(3)),
        ];
        for (label, local) in &locals {
            let globals = [("a", local.clone())];
            for constant in 1..pool.len() as u16 {
                let run = |tail: &[Op]| {
                    let code = [&[Op::GetGlobal(A)][..], tail].concat();
                    run_bytecode_with_globals(pool.clone(), &code, &globals)
                        .unwrap_or_else(|e| panic!("{label}, constant {constant}: {e}"))
                };
                let Value::Bool(equal) = run(&[Op::Constant(constant), Op::Equal, Op::Return])
                else {
                    panic!("Equal answers a Bool: {label}, constant {constant}");
                };
                assert_eq!(
                    run(&[Op::EqualLocalConst(1, constant), Op::Return]),
                    Value::Bool(equal),
                    "EqualLocalConst: {label}, constant {constant}"
                );
                assert_eq!(
                    run(&[Op::NotEqualLocalConst(1, constant), Op::Return]),
                    Value::Bool(!equal),
                    "NotEqualLocalConst: {label}, constant {constant}"
                );
            }
        }
    }

    /// The conditions the peephole fuses into `TestNotEqualJump` (an `if`,
    /// an `elsif`, a `while`, a ternary, a postfix `if` and a guard clause on
    /// `!=`) branch the way `!=` evaluates, on equal enum values built apart.
    /// `status != WANTED` has the shape of `if order.status != Status.Paid`.
    /// The values come in as globals, so no enum declaration is needed.
    #[test]
    fn not_equal_conditions_compare_enum_values_by_value() {
        let source = r#"
def guarded(status)
  return "guard " if status != WANTED

  ""
end

def conditions(status)
  seen = ""
  if status != WANTED
    seen = seen + "if "
  end
  if status == nil
    seen = seen + "nil "
  elsif status != WANTED
    seen = seen + "elsif "
  end
  rounds = 0
  while status != WANTED
    rounds += 1
    break
  end
  seen = seen + "while " if rounds > 0
  seen = seen + (status != WANTED ? "ternary " : "")
  seen = seen + "postfix " if status != WANTED
  seen + guarded(status)
end

let result = conditions(STATUS)
"#;
        let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
        let program = Parser::new(tokens).parse().expect("parser error");
        let module = Compiler::compile(&program).expect("compile error");
        // The test is about the fused op: every `!=` condition must be one.
        for (function, expected_fused) in [("guarded", 1), ("conditions", 5)] {
            let code = module
                .main
                .chunk
                .constants
                .iter()
                .find_map(|constant| match constant {
                    Constant::Function(proto) if proto.name == function => {
                        Some(proto.chunk.code.clone())
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("the function {function}"));
            let fused = code
                .iter()
                .filter(|op| matches!(op, Op::TestNotEqualJump(_)))
                .count();
            assert_eq!(fused, expected_fused, "{function}: {code:?}");
            assert!(!code.contains(&Op::NotEqual), "{function}: {code:?}");
        }

        let status = bare_class("Status");
        let shape = bare_class("Shape");
        let cases = [
            (
                "two stored Status.Active",
                stored_variant(&status, "Active"),
                stored_variant(&status, "Active"),
                "",
            ),
            ("two Circle(1)", circle(&shape, 1), circle(&shape, 1), ""),
            (
                "Status.Active and Status.Inactive",
                stored_variant(&status, "Active"),
                stored_variant(&status, "Inactive"),
                "if elsif while ternary postfix guard ",
            ),
        ];
        for (pair, value, wanted, expected) in cases {
            let mut vm = Vm::new();
            vm.globals.insert("STATUS".to_string(), value);
            vm.globals.insert("WANTED".to_string(), wanted);
            vm.execute(&module.main)
                .unwrap_or_else(|e| panic!("{pair}: {e}"));
            assert_eq!(
                vm.globals.get("result"),
                Some(&Value::String(expected.into())),
                "{pair}"
            );
        }
    }

    // Regression: `@sdbql{ ... #{var} ... }` must lower to a resolvable
    // `__sdql_exec(query, binds)` call in the VM. Before this was wired, the
    // compiler emitted a call to an unregistered global and JIT'd code (jobs,
    // controller actions) blew up with "Undefined variable '__sdql_exec'".
    // We stub __sdql_exec to capture its arguments and assert the raw query
    // and the binds hash (var name -> value) arrive intact.
    #[test]
    fn test_vm_sdql_block_lowers_to_sdql_exec_with_binds() {
        use crate::interpreter::value::HashKey;
        use std::cell::RefCell;
        use std::rc::Rc;

        let source = "let room = 7; \
             let rows = @sdbql{ FOR c IN comments FILTER c.id == #{room} RETURN c };";
        let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
        let program = Parser::new(tokens).parse().expect("parser error");
        let module = Compiler::compile(&program).expect("compile error");

        type CapturedQuery = Rc<RefCell<Option<(String, Vec<(String, i64)>)>>>;
        let captured: CapturedQuery = Rc::new(RefCell::new(None));
        let sink = captured.clone();

        let mut vm = Vm::new();
        vm.globals.insert(
            "__sdql_exec".to_string(),
            Value::NativeFunction(NativeFunction::new("__sdql_exec", None, move |args| {
                let query = match args.first() {
                    Some(Value::String(s)) => s.as_ref().to_string(),
                    _ => String::new(),
                };
                let mut binds = Vec::new();
                if let Some(Value::Hash(hash)) = args.get(1) {
                    for (key, value) in hash.borrow().iter() {
                        if let (HashKey::String(name), Value::Int(n)) = (key, value) {
                            binds.push((name.to_string(), *n));
                        }
                    }
                }
                *sink.borrow_mut() = Some((query, binds));
                Ok(Value::Array(Rc::new(RefCell::new(vec![]))))
            })),
        );

        vm.execute(&module.main).expect("vm error");

        let captured = captured.borrow();
        let (query, binds) = captured.as_ref().expect("__sdql_exec was invoked");
        assert!(
            query.contains("FOR c IN comments") && query.contains("#{room}"),
            "raw query (placeholders intact) should reach the builtin, got: {query}"
        );
        assert_eq!(binds, &vec![("room".to_string(), 7i64)]);
    }

    #[test]
    fn test_vm_variables() {
        let result = compile_and_get_global("let x = 10; let y = x + 5;", "y");
        assert_eq!(result, Value::Int(15));
    }

    #[test]
    fn test_vm_if_else() {
        let result = compile_and_get_global(
            "let x = 10; let y = 0; if (x > 5) { y = 1; } else { y = 2; }",
            "y",
        );
        assert_eq!(result, Value::Int(1));
    }

    #[test]
    fn test_vm_while_loop() {
        let result = compile_and_get_global(
            r#"
            let i = 0;
            let sum = 0;
            while (i < 10) {
                sum = sum + i;
                i = i + 1;
            }
            "#,
            "sum",
        );
        assert_eq!(result, Value::Int(45));
    }

    #[test]
    fn test_vm_function_call() {
        let result = compile_and_get_global(
            r#"
            fn add(a: Int, b: Int) -> Int {
                return a + b;
            }
            let result = add(3, 4);
            "#,
            "result",
        );
        assert_eq!(result, Value::Int(7));
    }

    #[test]
    fn test_vm_recursive_fib() {
        let result = compile_and_get_global(
            r#"
            fn fib(n: Int) -> Int {
                if (n <= 1) {
                    return n;
                }
                return fib(n - 1) + fib(n - 2);
            }
            let result = fib(10);
            "#,
            "result",
        );
        assert_eq!(result, Value::Int(55));
    }

    #[test]
    fn test_vm_iterative_fib() {
        let result = compile_and_get_global(
            r#"
            fn fib(n: Int) -> Int {
                if (n <= 1) {
                    return n;
                }
                let a = 0;
                let b = 1;
                let i = 2;
                while (i <= n) {
                    let temp = a + b;
                    a = b;
                    b = temp;
                    i = i + 1;
                }
                return b;
            }
            let result = fib(30);
            "#,
            "result",
        );
        assert_eq!(result, Value::Int(832040));
    }

    #[test]
    fn test_vm_string_concat() {
        let result = compile_and_get_global(r#"let x = "hello" + " " + "world";"#, "x");
        assert_eq!(result, Value::String("hello world".into()));
    }

    #[test]
    fn test_vm_comparison() {
        let t = compile_and_get_global("let x = 5 > 3;", "x");
        assert_eq!(t, Value::Bool(true));
        let f = compile_and_get_global("let x = 5 < 3;", "x");
        assert_eq!(f, Value::Bool(false));
    }

    #[test]
    fn test_vm_logical_ops() {
        let result = compile_and_get_global("let x = true && false;", "x");
        assert_eq!(result, Value::Bool(false));
        let result = compile_and_get_global("let x = true || false;", "x");
        assert_eq!(result, Value::Bool(true));
    }

    #[test]
    fn test_vm_array() {
        let result = compile_and_get_global("let x = [1, 2, 3];", "x");
        if let Value::Array(arr) = result {
            assert_eq!(arr.borrow().len(), 3);
        } else {
            panic!("Expected array");
        }
    }

    #[test]
    fn test_vm_loop_sum_10000() {
        let result = compile_and_get_global(
            r#"
            fn sum_to(n: Int) -> Int {
                let total = 0;
                let i = 1;
                while (i <= n) {
                    total = total + i;
                    i = i + 1;
                }
                return total;
            }
            let result = sum_to(10000);
            "#,
            "result",
        );
        assert_eq!(result, Value::Int(50005000));
    }

    // ============================================================
    // Array-method dispatch on the VM.
    // soli serve (production mode) executes handlers through the VM.
    // If any of these fail, a controller using that array method
    // will silently fall back to the tree walker on each request.
    // ============================================================

    fn run_array_method(source: &str) -> Result<Value, crate::error::RuntimeError> {
        compile_and_run(source)
    }

    #[test]
    fn test_vm_array_length() {
        let r = run_array_method("let a = [1, 2, 3]; let x = a.length();");
        assert!(r.is_ok(), "array.length() failed on VM: {:?}", r.err());
    }

    #[test]
    fn test_vm_array_map() {
        let r = run_array_method("let a = [1, 2, 3]; let x = a.map(fn(v) v * 2);");
        assert!(r.is_ok(), "array.map() failed on VM: {:?}", r.err());
    }

    #[test]
    fn test_vm_array_filter() {
        let r = run_array_method("let a = [1, 2, 3]; let x = a.filter(fn(v) v > 1);");
        assert!(r.is_ok(), "array.filter() failed on VM: {:?}", r.err());
    }

    #[test]
    fn test_vm_array_reduce() {
        let r = run_array_method("let a = [1, 2, 3]; let x = a.reduce(fn(acc, v) acc + v, 0);");
        assert!(r.is_ok(), "array.reduce() failed on VM: {:?}", r.err());
    }

    #[test]
    fn test_vm_array_each() {
        let r = run_array_method("let a = [1, 2, 3]; a.each(fn(v) v);");
        assert!(r.is_ok(), "array.each() failed on VM: {:?}", r.err());
    }

    #[test]
    fn test_vm_array_push() {
        let r = run_array_method("let a = [1, 2]; a.push(3);");
        assert!(r.is_ok(), "array.push() failed on VM: {:?}", r.err());
    }

    #[test]
    fn test_vm_array_pop() {
        let r = run_array_method("let a = [1, 2, 3]; let x = a.pop();");
        assert!(r.is_ok(), "array.pop() failed on VM: {:?}", r.err());
    }

    #[test]
    fn test_vm_array_reverse() {
        let r = run_array_method("let a = [1, 2, 3]; let x = a.reverse();");
        assert!(r.is_ok(), "array.reverse() failed on VM: {:?}", r.err());
    }

    #[test]
    fn test_vm_array_sort() {
        let r = run_array_method("let a = [3, 1, 2]; let x = a.sort();");
        assert!(r.is_ok(), "array.sort() failed on VM: {:?}", r.err());
    }

    #[test]
    fn test_vm_array_uniq() {
        let r = run_array_method("let a = [1, 1, 2]; let x = a.uniq();");
        assert!(r.is_ok(), "array.uniq() failed on VM: {:?}", r.err());
    }

    #[test]
    fn test_vm_array_join() {
        let r = run_array_method(r#"let a = ["x", "y"]; let x = a.join(",");"#);
        assert!(r.is_ok(), "array.join() failed on VM: {:?}", r.err());
    }

    #[test]
    fn test_vm_array_first_last() {
        let first = run_array_method("let a = [1, 2, 3]; let x = a.first();");
        let last = run_array_method("let a = [1, 2, 3]; let x = a.last();");
        assert!(
            first.is_ok(),
            "array.first() failed on VM: {:?}",
            first.err()
        );
        assert!(last.is_ok(), "array.last() failed on VM: {:?}", last.err());
    }

    #[test]
    fn test_vm_array_contains() {
        let r = run_array_method("let a = [1, 2, 3]; let x = a.contains(2);");
        assert!(r.is_ok(), "array.contains() failed on VM: {:?}", r.err());
    }

    #[test]
    fn test_vm_array_flatten() {
        let r = run_array_method("let a = [[1, 2], [3, 4]]; let x = a.flatten();");
        assert!(r.is_ok(), "array.flatten() failed on VM: {:?}", r.err());
    }

    // Additional array-method coverage (vm_array_methods.rs)

    #[test]
    fn test_vm_array_each_with_index() {
        let r = compile_and_run("let a = [1, 2, 3]; a.each_with_index(fn(v, i) v + i);");
        assert!(r.is_ok());
    }

    /// A block that declares no parameter ignores the element, as on the
    /// tree-walker: `3.times do … end`, `xs.each { log("x") }`. The VM passed
    /// the element anyway, and the block's frame refused it ("Wrong number of
    /// arguments: expected 0, got 1"). A script stopped there, and a `serve`
    /// handler was demoted after its side effects had already run.
    #[test]
    fn test_vm_block_with_no_parameters_ignores_the_element() {
        // Int iterators drop the index. `times` still returns its receiver.
        assert_eq!(
            compile_and_get_global(
                r##"
let n = 0
let r = 3.times do
  n = n + 1
end
1.upto(3) { n = n + 10 }
3.downto(1) { n = n + 100 }
let x = "#{n} #{r}"
"##,
                "x"
            ),
            Value::String("333 3".into())
        );
        // Array iterators, through the in-VM loop (each, map, filter, select)
        // and through the native drivers (the rest).
        assert_eq!(
            compile_and_get_global(
                r##"
let xs = [3, 1, 2]
let n = 0
let back = xs.each { n = n + 1 }
let mapped = xs.map { 5 }
let kept = xs.filter { true }
let dropped = xs.select { false }
let rejected = xs.reject { false }
let found = xs.find { true }
let counted = xs.count { true }
let any_true = xs.any? { true }
let all_false = xs.all? { false }
let none_false = xs.none? { false }
let one_true = xs.one? { true }
let sorted = xs.sort_by { 1 }
let x = "#{n} #{back} #{mapped} #{kept} #{dropped} #{rejected} #{found} #{counted} #{any_true} #{all_false} #{none_false} #{one_true} #{sorted}"
"##,
                "x"
            ),
            Value::String(
                "3 [3, 1, 2] [5, 5, 5] [3, 1, 2] [] [3, 1, 2] 3 3 true false true false [3, 1, 2]"
                    .into()
            )
        );
        // Hash iterators get no `[key, value]` pair, and no key or value.
        assert_eq!(
            compile_and_get_global(
                r##"
let h = {"a": 1, "b": 2}
let n = 0
let back = h.each { n = n + 1 }
h.each_key { n = n + 10 }
h.each_value { n = n + 100 }
let mapped = h.map { ["k", 9] }
let kept = h.filter { true }
let dropped = h.select { false }
let rejected = h.reject { true }
let kept_if = h.keep_if { true }
let deleted_if = h.delete_if { false }
let any_true = h.any? { true }
let all_false = h.all? { false }
let revalued = h.transform_values { 0 }
let renamed = h.transform_keys { "z" }
let x = "#{n} #{back} #{mapped} #{kept} #{dropped} #{rejected} #{kept_if} #{deleted_if} #{any_true} #{all_false} #{revalued} #{renamed}"
"##,
                "x"
            ),
            Value::String(
                "222 {a => 1, b => 2} {k => 9} {a => 1, b => 2} {} {} {a => 1, b => 2} \
                 {a => 1, b => 2} true false {a => 0, b => 0} {z => 2}"
                    .into()
            )
        );
    }

    /// The in-VM callback loop takes a block with no parameter instead of
    /// handing it to the native driver. Its frame gets no argument slot, and
    /// refilling it writes none. Only a loop that ran in the dispatch loop
    /// leaves `spare_callback_loop` behind.
    #[test]
    fn test_vm_zero_parameter_block_takes_the_in_vm_loop() {
        let run = |source: &str| {
            let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
            let program = Parser::new(tokens).parse().expect("parser error");
            let module = Compiler::compile(&program).expect("compile error");
            let mut vm = Vm::new();
            vm.execute(&module.main).expect("vm error");
            let x = vm.globals.get("x").cloned().unwrap_or(Value::Null);
            (x, vm.spare_callback_loop.is_some())
        };
        // One frame per element, rerun in place.
        assert_eq!(
            run("let n = 0\n[1, 2, 3].each { n = n + 1 }\nlet x = n"),
            (Value::Int(3), true)
        );
        assert_eq!(
            run("let x = [1, 2, 3].map { 5 }.join(\",\")"),
            (Value::String("5,5,5".into()), true)
        );
        // `filter` keeps the element it did not pass.
        assert_eq!(
            run("let x = [1, 2, 3].filter { true }.join(\",\")"),
            (Value::String("1,2,3".into()), true)
        );
        // Hash loops: no pair is built and no slot is written.
        assert_eq!(
            run("let n = 0\nlet h = {\"a\": 1, \"b\": 2}\nh.each { n = n + 1 }\nlet x = n"),
            (Value::Int(2), true)
        );
        assert_eq!(
            run("let t = {\"a\": 1, \"b\": 2}.transform_values { 0 }\nlet x = \"#{t}\""),
            (Value::String("{a => 0, b => 0}".into()), true)
        );
        assert_eq!(
            run("let t = {\"a\": 1, \"b\": 2}.reject { false }\nlet x = \"#{t}\""),
            (Value::String("{a => 1, b => 2}".into()), true)
        );
        // `{ count += 1 }` on a captured local is the accumulator template:
        // no frame at all.
        assert_eq!(
            run(r##"
def tally(xs)
  let count = 0
  xs.each { count += 1 }
  count
end
let x = tally([1, 2, 3])
"##),
            (Value::Int(3), true)
        );
        // A block whose local is captured returns through the general
        // `Return`, and the next element's frame is opened afresh.
        assert_eq!(
            run(r##"
def keep(xs)
  let fns = []
  xs.each do
    let y = 7
    fns.push(fn() { y })
  end
  fns[0]() + fns[1]()
end
let x = keep([1, 2])
"##),
            (Value::Int(14), true)
        );
    }

    /// A block takes as many of two arguments as it declares:
    /// `each_with_index { |x| }` gets the element alone, and `{ }` gets
    /// neither. A one-parameter `reduce` callback gets the `[acc, item]`
    /// pair, as on the tree-walker. All of these raised "Wrong number of
    /// arguments" on the VM.
    #[test]
    fn test_vm_block_takes_a_prefix_of_two_arguments() {
        assert_eq!(
            compile_and_get_global(
                r##"
let seen = []
[10, 20].each_with_index { |x| seen.push(x) }
[10, 20].each_with_index { seen.push(0) }
let sum = [1, 2, 3].reduce(fn(pair) { pair[0] + pair[1] }, 0)
let product = [1, 2, 3].reduce(fn(pair) { pair[0] * pair[1] })
let constant = [1, 2, 3].reduce(fn() { 7 }, 0)
let both = [1, 2, 3].reduce(fn(acc, x) { acc + x }, 0)
let x = "#{seen} #{sum} #{product} #{constant} #{both}"
"##,
                "x"
            ),
            Value::String("[10, 20, 0, 0] 6 6 7 6".into())
        );
    }

    /// A destructuring callback with more defaulted than required parameters
    /// computed `arity - defaults`, but `arity` counts only the required
    /// parameters. That underflowed, and the VM panicked with "capacity
    /// overflow". Now a defaulted parameter takes its part of the element when
    /// the element has one and keeps its default otherwise. A missing required
    /// part reads as nil.
    #[test]
    fn test_vm_destructuring_callback_with_defaults_does_not_underflow() {
        assert_eq!(
            compile_and_get_global(
                r##"
def tri(a, b, c = 1, d = 2, e = 3)
  [a, b, c, d, e]
end
let x = "#{[[1, 2], [1, 2, 3], [1, 2, 3, 4, 5, 6], [1]].map(tri)}"
"##,
                "x"
            ),
            Value::String(
                "[[1, 2, 1, 2, 3], [1, 2, 3, 2, 3], [1, 2, 3, 4, 5], [1, null, 1, 2, 3]]".into()
            )
        );
    }

    /// Blocks bind loosely, but direct calls stay strict, as on the
    /// tree-walker.
    #[test]
    fn test_vm_direct_calls_stay_strict() {
        let err = compile_and_run("let f = fn() { 1 }\nf(5)")
            .expect_err("a direct call checks its arity");
        assert!(
            err.to_string().contains("Wrong number of arguments"),
            "{err}"
        );
        let err = compile_and_run("let h = {\"f\": fn() { 1 }}\nh.f(5)")
            .expect_err("a call through a hash entry checks its arity");
        assert!(
            err.to_string().contains("Wrong number of arguments"),
            "{err}"
        );
    }

    #[test]
    fn test_vm_array_index_of() {
        assert_eq!(
            compile_and_get_global("let a = [10, 20, 30]; let x = a.index_of(20);", "x"),
            Value::Int(1)
        );
        assert_eq!(
            compile_and_get_global("let a = [10, 20, 30]; let x = a.index_of(99);", "x"),
            Value::Int(-1)
        );
    }

    #[test]
    fn test_vm_array_clear_empty() {
        let r = compile_and_run("let a = [1, 2, 3]; a.clear();");
        assert!(r.is_ok());
        assert_eq!(
            compile_and_get_global("let x = [].empty?();", "x"),
            Value::Bool(true)
        );
        assert_eq!(
            compile_and_get_global("let x = [1].empty?();", "x"),
            Value::Bool(false)
        );
    }

    #[test]
    fn test_vm_array_first_last_value() {
        assert_eq!(
            compile_and_get_global("let a = [1, 2, 3]; let x = a.first();", "x"),
            Value::Int(1)
        );
        assert_eq!(
            compile_and_get_global("let a = [1, 2, 3]; let x = a.last();", "x"),
            Value::Int(3)
        );
    }

    #[test]
    fn test_vm_array_compact() {
        let r = compile_and_run("let a = [1, null, 2, null, 3]; let x = a.compact();");
        assert!(r.is_ok());
    }

    #[test]
    fn test_vm_array_sum_min_max() {
        assert_eq!(
            compile_and_get_global("let x = [1, 2, 3, 4].sum();", "x"),
            Value::Int(10)
        );
        assert_eq!(
            compile_and_get_global("let x = [3, 1, 4, 1, 5].min();", "x"),
            Value::Int(1)
        );
        assert_eq!(
            compile_and_get_global("let x = [3, 1, 4, 1, 5].max();", "x"),
            Value::Int(5)
        );
        let r = compile_and_run("let x = [1.5, 2.5, 3.5].sum();");
        assert!(r.is_ok());
    }

    #[test]
    fn test_vm_array_take_drop() {
        let r = compile_and_run("let a = [1, 2, 3, 4, 5]; let x = a.take(2);");
        assert!(r.is_ok());
        let r = compile_and_run("let a = [1, 2, 3, 4, 5]; let x = a.drop(2);");
        assert!(r.is_ok());
    }

    #[test]
    fn test_vm_array_get() {
        assert_eq!(
            compile_and_get_global("let a = [10, 20, 30]; let x = a.get(1);", "x"),
            Value::Int(20)
        );
        // Out-of-bounds get returns null
        assert_eq!(
            compile_and_get_global("let a = [10, 20, 30]; let x = a.get(99);", "x"),
            Value::Null
        );
    }

    #[test]
    fn test_vm_array_to_string_inspect() {
        let r = compile_and_run("let x = [1, 2, 3].to_s();");
        assert!(r.is_ok());
        let r = compile_and_run("let x = [1, 2, 3].inspect;");
        assert!(r.is_ok());
    }

    #[test]
    fn test_vm_int_zero_arg_methods() {
        // Bare member access resolves through int_member_access.
        assert_eq!(
            compile_and_get_global("let n = -42; let x = n.abs;", "x"),
            Value::Int(42)
        );
        assert_eq!(
            compile_and_get_global("let n = 4; let x = n.even?;", "x"),
            Value::Bool(true)
        );
        assert_eq!(
            compile_and_get_global("let n = 255; let x = n.to_s;", "x"),
            Value::String("255".into())
        );
        // `n.to_s()` with parens dispatches through call_int_method_impl.
        assert_eq!(
            compile_and_get_global("let n = 255; let x = n.to_s();", "x"),
            Value::String("255".into())
        );
        assert_eq!(
            compile_and_get_global("let n = 65; let x = n.chr;", "x"),
            Value::String("A".into())
        );
        assert_eq!(
            compile_and_get_global("let n = 42; let x = n.class;", "x"),
            Value::String("int".into())
        );
        // Parens on a direct-return zero-arg method behaves like the bare
        // form — Ruby-style, matching the tree-walker's evaluate_call.
        assert_eq!(
            compile_and_get_global("let n = -42; let x = n.abs();", "x"),
            Value::Int(42)
        );
        assert_eq!(
            compile_and_get_global("let n = 42; let x = n.to_f();", "x"),
            Value::Float(42.0)
        );
        // Unknown property on a primitive errors (tree-walker parity) —
        // bare and parens forms both.
        let r = compile_and_run("let n = 42; let x = n.frobnicate;");
        assert!(r.is_err());
        let r = compile_and_run("let n = 42; let x = n.frobnicate();");
        assert!(r.is_err());
    }

    #[test]
    fn test_vm_float_methods() {
        assert_eq!(
            compile_and_get_global("let f = -3.7; let x = f.abs;", "x"),
            Value::Float(3.7)
        );
        assert_eq!(
            compile_and_get_global("let f = 3.7; let x = f.ceil;", "x"),
            Value::Int(4)
        );
        assert_eq!(
            compile_and_get_global("let f = 3.7; let x = f.round;", "x"),
            Value::Int(4)
        );
        assert_eq!(
            compile_and_get_global("let f = 3.7; let x = f.round();", "x"),
            Value::Int(4)
        );
        assert_eq!(
            compile_and_get_global("let f = 1.23456; let x = f.round(2);", "x"),
            Value::Float(1.23)
        );
        // Ruby-style decimal rounding, not binary-float artifact (38.99).
        assert_eq!(
            compile_and_get_global("let f = 38.995; let x = f.round(2);", "x"),
            Value::Float(39.0)
        );
        assert_eq!(
            compile_and_get_global("let f = 3.5; let x = f.between?(3, 4);", "x"),
            Value::Bool(true)
        );
        assert_eq!(
            compile_and_get_global("let f = 3.5; let x = f.clamp(0, 3.0);", "x"),
            Value::Float(3.0)
        );
        assert_eq!(
            compile_and_get_global("let f = 3.5; let x = f.is_a?(\"float\");", "x"),
            Value::Bool(true)
        );
        assert_eq!(
            compile_and_get_global("let f = 3.5; let x = f.class;", "x"),
            Value::String("float".into())
        );
    }

    #[test]
    fn test_vm_bool_null_methods() {
        assert_eq!(
            compile_and_get_global("let b = true; let x = b.to_i;", "x"),
            Value::Int(1)
        );
        assert_eq!(
            compile_and_get_global("let b = false; let x = b.blank?;", "x"),
            Value::Bool(true)
        );
        assert_eq!(
            compile_and_get_global("let b = true; let x = b.is_a?(\"bool\");", "x"),
            Value::Bool(true)
        );
        assert_eq!(
            compile_and_get_global("let n = null; let x = n.nil?;", "x"),
            Value::Bool(true)
        );
        assert_eq!(
            compile_and_get_global("let n = null; let x = n.to_s;", "x"),
            Value::String("".into())
        );
        assert_eq!(
            compile_and_get_global("let n = null; let x = n.to_i;", "x"),
            Value::Int(0)
        );
        assert_eq!(
            compile_and_get_global("let n = null; let x = n.is_a?(\"null\");", "x"),
            Value::Bool(true)
        );
        // Unknown property on null still errors (tree-walker parity) — a
        // null receiver must not silently swallow typo'd member access.
        let r = compile_and_run("let n = null; let x = n.address;");
        assert!(r.is_err());
    }

    #[test]
    fn test_vm_native_instance_method_binding() {
        // Native instance methods (DateTime/Duration pattern) must receive
        // the instance as args[0]. Bare access on a zero-arg native
        // auto-invokes (op_get_property_member / bind wrapper); the parens
        // form goes through CallMethod's direct native path (no per-call
        // wrapper). Both must see the receiver.
        use crate::interpreter::value::{Class, Instance};
        use std::collections::HashMap;

        let mut native_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();
        native_methods.insert(
            "val".to_string(),
            Rc::new(NativeFunction::new("Box.val", Some(0), |args| {
                match args.first() {
                    Some(Value::Instance(inst)) => Ok(inst
                        .borrow()
                        .fields
                        .get("v")
                        .cloned()
                        .unwrap_or(Value::Null)),
                    _ => Err("Box.val called without receiver".to_string()),
                }
            })),
        );
        native_methods.insert(
            "add".to_string(),
            Rc::new(NativeFunction::new("Box.add", Some(1), |args| {
                let this = match args.first() {
                    Some(Value::Instance(inst)) => inst,
                    _ => return Err("Box.add called without receiver".to_string()),
                };
                let base = match this.borrow().fields.get("v") {
                    Some(Value::Int(n)) => *n,
                    _ => 0,
                };
                let n = match args.get(1) {
                    Some(Value::Int(n)) => *n,
                    _ => return Err("Box.add expects an int".to_string()),
                };
                Ok(Value::Int(base + n))
            })),
        );
        let class = Rc::new(Class {
            name: "Box".to_string(),
            native_methods,
            ..Default::default()
        });
        let mut instance = Instance::new(class);
        instance.fields.insert("v".into(), Value::Int(7));
        let obj = Value::Instance(Rc::new(RefCell::new(instance)));

        for source in ["let x = obj.val();", "let x = obj.val;"] {
            let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
            let program = Parser::new(tokens).parse().expect("parser error");
            let module = Compiler::compile(&program).expect("compile error");
            let mut vm = Vm::new();
            vm.globals.insert("obj".to_string(), obj.clone());
            vm.execute(&module.main).expect("vm error");
            assert_eq!(vm.globals.get("x"), Some(&Value::Int(7)), "{}", source);
        }

        // One-arg native via CallMethod direct path
        let tokens = Scanner::new("let x = obj.add(3);")
            .scan_tokens()
            .expect("lexer error");
        let program = Parser::new(tokens).parse().expect("parser error");
        let module = Compiler::compile(&program).expect("compile error");
        let mut vm = Vm::new();
        vm.globals.insert("obj".to_string(), obj.clone());
        vm.execute(&module.main).expect("vm error");
        assert_eq!(vm.globals.get("x"), Some(&Value::Int(10)));

        // Field shadows native of the same name
        let mut native_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();
        native_methods.insert(
            "score".to_string(),
            Rc::new(NativeFunction::new("Box.score", Some(0), |_args| {
                Ok(Value::Int(999))
            })),
        );
        let class = Rc::new(Class {
            name: "Box".to_string(),
            native_methods,
            ..Default::default()
        });
        let mut instance = Instance::new(class);
        instance.fields.insert("score".into(), Value::Int(42));
        let obj = Value::Instance(Rc::new(RefCell::new(instance)));
        let tokens = Scanner::new("let x = obj.score();")
            .scan_tokens()
            .expect("lexer error");
        let program = Parser::new(tokens).parse().expect("parser error");
        let module = Compiler::compile(&program).expect("compile error");
        let mut vm = Vm::new();
        vm.globals.insert("obj".to_string(), obj);
        vm.execute(&module.main).expect("vm error");
        assert_eq!(vm.globals.get("x"), Some(&Value::Int(42)));
    }

    #[test]
    fn test_vm_model_static_receives_class() {
        // Native statics on Model subclasses expect the class as args[0]
        // (collection resolution) — op_get_property binds it via
        // bind_native_static_to_model_class.
        use crate::interpreter::value::Class;
        use std::collections::HashMap;

        let model_base = Rc::new(Class {
            name: "Model".to_string(),
            ..Default::default()
        });
        let mut native_static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();
        native_static_methods.insert(
            "collection_name".to_string(),
            Rc::new(NativeFunction::new(
                "Model.collection_name",
                None,
                |args| match args.first() {
                    Some(Value::Class(class)) => Ok(Value::String(class.name.clone().into())),
                    _ => Err("collection_name called without class receiver".to_string()),
                },
            )),
        );
        let user_class = Rc::new(Class {
            name: "User".to_string(),
            superclass: Some(model_base),
            native_static_methods,
            ..Default::default()
        });

        let source = "let x = User.collection_name();";
        let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
        let program = Parser::new(tokens).parse().expect("parser error");
        let module = Compiler::compile(&program).expect("compile error");
        let mut vm = Vm::new();
        vm.globals
            .insert("User".to_string(), Value::Class(user_class));
        vm.execute(&module.main).expect("vm error");
        assert_eq!(vm.globals.get("x"), Some(&Value::String("User".into())));
    }

    #[test]
    fn test_vm_model_instance_save_runs_on_vm() {
        use crate::interpreter::value::{Class, Instance};
        use std::collections::HashMap;

        let model_base = Rc::new(Class {
            name: "Model".to_string(),
            ..Default::default()
        });
        let mut native_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();
        native_methods.insert(
            "save".to_string(),
            Rc::new(NativeFunction::new("Model.save", None, |_| {
                Ok(Value::Bool(true))
            })),
        );
        let record_class = Rc::new(Class {
            name: "User".to_string(),
            superclass: Some(model_base),
            native_methods,
            ..Default::default()
        });
        let record = Value::Instance(Rc::new(RefCell::new(Instance::new(record_class))));

        let source = "let x = record.save();";
        let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
        let program = Parser::new(tokens).parse().expect("parser error");
        let module = Compiler::compile(&program).expect("compile error");
        let mut vm = Vm::new();
        vm.globals.insert("record".to_string(), record);
        vm.execute(&module.main).expect("save should run on the VM");
        assert_eq!(vm.globals.get("x"), Some(&Value::Bool(true)));
    }

    /// `user["_key"]` sur une instance de modèle, sur le VM.
    ///
    /// Une clé chaîne constante se compile en `HashGetConst` (et ses variantes
    /// locale et globale), qui ne savaient lire qu'un hachage, un tableau ou une
    /// chaîne : sur une instance, elles répondaient « Cannot access property
    /// 'get' on User ». `organisations#choose` lit `current_user()["_key"]`, et
    /// répondait 500 dès qu'il tournait sur le VM — un échec intermittent, qui
    /// disparaissait dès qu'on instrumentait l'action (le `try` la faisait
    /// retomber sur l'interpréteur).
    #[test]
    fn test_vm_const_key_read_on_model_instance() {
        use crate::interpreter::value::{Class, Instance};

        let model_base = Rc::new(Class {
            name: "Model".to_string(),
            ..Default::default()
        });
        let record_class = Rc::new(Class {
            name: "User".to_string(),
            superclass: Some(model_base),
            ..Default::default()
        });
        let mut instance = Instance::new(record_class);
        instance
            .fields
            .insert("_key".into(), Value::String("abc".into()));
        let record = Value::Instance(Rc::new(RefCell::new(instance)));

        let source = "fn courant() { return record }\n\
                      fn locale() { let r = record\n return r[\"_key\"] }\n\
                      let par_appel = courant()[\"_key\"];\n\
                      let par_globale = record[\"_key\"];\n\
                      let par_locale = locale();";
        let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
        let program = Parser::new(tokens).parse().expect("parser error");
        let module = Compiler::compile(&program).expect("compile error");
        let mut vm = Vm::new();
        vm.globals.insert("record".to_string(), record);
        vm.execute(&module.main)
            .expect("a model instance reads by key on the VM");
        for name in ["par_appel", "par_globale", "par_locale"] {
            assert_eq!(
                vm.globals.get(name),
                Some(&Value::String("abc".into())),
                "{name}"
            );
        }
    }

    #[test]
    fn test_vm_model_before_save_veto_skips_native() {
        use crate::interpreter::builtins::model::callbacks::register_callback;
        use crate::interpreter::value::{Class, Function, Instance};
        use std::collections::HashMap;
        use std::sync::atomic::{AtomicBool, Ordering};

        static NATIVE_RAN: AtomicBool = AtomicBool::new(false);
        NATIVE_RAN.store(false, Ordering::SeqCst);

        let body_tokens = Scanner::new("return false")
            .scan_tokens()
            .expect("lex veto");
        let body = Parser::new(body_tokens)
            .parse()
            .expect("parse veto")
            .statements;

        let model_base = Rc::new(Class {
            name: "Model".to_string(),
            ..Default::default()
        });
        let mut native_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();
        native_methods.insert(
            "save".to_string(),
            Rc::new(NativeFunction::new("Model.save", None, |_| {
                NATIVE_RAN.store(true, Ordering::SeqCst);
                Ok(Value::Bool(true))
            })),
        );
        let record_class = Rc::new(Class {
            name: "VetoUser".to_string(),
            superclass: Some(model_base),
            native_methods,
            ..Default::default()
        });
        record_class.methods.borrow_mut().insert(
            "veto".to_string(),
            Rc::new(Function {
                name: "veto".to_string(),
                body: Rc::new(body.into()),
                ..Function::default()
            }),
        );
        register_callback("VetoUser", "before_save", "veto");
        let record = Value::Instance(Rc::new(RefCell::new(Instance::new(record_class))));

        let source = "let x = record.save();";
        let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
        let program = Parser::new(tokens).parse().expect("parser error");
        let module = Compiler::compile(&program).expect("compile error");
        let mut vm = Vm::new();
        vm.globals.insert("record".to_string(), record);
        vm.execute(&module.main).expect("veto save");
        assert_eq!(vm.globals.get("x"), Some(&Value::Bool(false)));
        assert!(
            !NATIVE_RAN.load(Ordering::SeqCst),
            "before_save veto must skip the native"
        );
    }

    /// A paren-less `record.save` used to auto-invoke the bound native
    /// directly, so no callback ran and a `before_save` veto could not stop
    /// the write. It now goes through the same wrapper as `record.save()`.
    #[test]
    fn test_vm_bare_model_save_runs_callbacks() {
        use crate::interpreter::builtins::model::callbacks::register_callback;
        use crate::interpreter::value::{Class, Function, Instance};
        use std::collections::HashMap;
        use std::sync::atomic::{AtomicBool, Ordering};

        static NATIVE_RAN: AtomicBool = AtomicBool::new(false);
        NATIVE_RAN.store(false, Ordering::SeqCst);

        let body_tokens = Scanner::new("return false")
            .scan_tokens()
            .expect("lex veto");
        let body = Parser::new(body_tokens)
            .parse()
            .expect("parse veto")
            .statements;

        let model_base = Rc::new(Class {
            name: "Model".to_string(),
            ..Default::default()
        });
        let mut native_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();
        native_methods.insert(
            "save".to_string(),
            Rc::new(NativeFunction::new_auto_invocable(
                "Model.save",
                None,
                |_| {
                    NATIVE_RAN.store(true, Ordering::SeqCst);
                    Ok(Value::Bool(true))
                },
            )),
        );
        let record_class = Rc::new(Class {
            name: "BareVetoUser".to_string(),
            superclass: Some(model_base),
            native_methods,
            ..Default::default()
        });
        record_class.methods.borrow_mut().insert(
            "veto".to_string(),
            Rc::new(Function {
                name: "veto".to_string(),
                body: Rc::new(body.into()),
                ..Function::default()
            }),
        );
        register_callback("BareVetoUser", "before_save", "veto");
        let record = Value::Instance(Rc::new(RefCell::new(Instance::new(record_class))));

        let source = "let x = record.save";
        let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
        let program = Parser::new(tokens).parse().expect("parser error");
        let module = Compiler::compile(&program).expect("compile error");
        let mut vm = Vm::new();
        vm.globals.insert("record".to_string(), record);
        vm.execute(&module.main).expect("veto bare save");
        assert_eq!(vm.globals.get("x"), Some(&Value::Bool(false)));
        assert!(
            !NATIVE_RAN.load(Ordering::SeqCst),
            "a bare save must honour the before_save veto"
        );
    }

    /// `def delete(force)` on a model is what `post.delete` reads: a method
    /// taking an argument, left uncalled. The paren-less path used to take the
    /// `Model#delete` native the class inherits, and delete the row.
    #[test]
    fn test_vm_bare_model_mutator_reads_the_models_own_method() {
        use crate::interpreter::value::{Class, Function, Instance};
        use std::collections::HashMap;
        use std::sync::atomic::{AtomicBool, Ordering};

        static NATIVE_RAN: AtomicBool = AtomicBool::new(false);
        NATIVE_RAN.store(false, Ordering::SeqCst);

        let mut native_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();
        native_methods.insert(
            "delete".to_string(),
            Rc::new(NativeFunction::new_auto_invocable(
                "Model.delete",
                Some(0),
                |_| {
                    NATIVE_RAN.store(true, Ordering::SeqCst);
                    Ok(Value::Bool(true))
                },
            )),
        );
        let model_base = Rc::new(Class {
            name: "Model".to_string(),
            native_methods,
            ..Default::default()
        });
        let record_class = Rc::new(Class {
            name: "OwnDeleteRecord".to_string(),
            superclass: Some(model_base),
            ..Default::default()
        });
        let program = Parser::new(
            Scanner::new("def delete(force)\n  force\nend")
                .scan_tokens()
                .expect("lex"),
        )
        .parse()
        .expect("parse");
        let crate::ast::stmt::StmtKind::Function(decl) = &program.statements[0].kind else {
            panic!("expected a def");
        };
        record_class.methods.borrow_mut().insert(
            "delete".to_string(),
            Rc::new(Function {
                name: "delete".to_string(),
                params: decl.params.clone().into(),
                body: Rc::new(decl.body.clone()),
                ..Function::default()
            }),
        );
        assert!(record_class.is_model_subclass());
        let record = Value::Instance(Rc::new(RefCell::new(Instance::new(record_class))));

        let source = "let x = record.delete";
        let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
        let program = Parser::new(tokens).parse().expect("parser error");
        let module = Compiler::compile(&program).expect("compile error");
        let mut vm = Vm::new();
        vm.globals.insert("record".to_string(), record);
        vm.execute(&module.main).expect("bare delete");
        assert!(
            !NATIVE_RAN.load(Ordering::SeqCst),
            "the model's own delete(force) must win over the inherited native"
        );
        assert!(
            !matches!(vm.globals.get("x"), Some(Value::Bool(_))),
            "a method with a parameter is read, not called: got {:?}",
            vm.globals.get("x")
        );
    }

    #[test]
    fn test_vm_model_class_create_runs_on_vm() {
        use crate::interpreter::value::Class;
        use std::collections::HashMap;

        let model_base = Rc::new(Class {
            name: "Model".to_string(),
            ..Default::default()
        });
        let mut native_static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();
        native_static_methods.insert(
            "create".to_string(),
            Rc::new(NativeFunction::new("Model.create", None, |_| {
                let mut pairs = crate::interpreter::value::HashPairs::default();
                pairs.insert(
                    crate::interpreter::value::HashKey::String("valid".into()),
                    Value::Bool(true),
                );
                Ok(Value::Hash(Rc::new(RefCell::new(pairs))))
            })),
        );
        let user_class = Rc::new(Class {
            name: "CreateUser".to_string(),
            superclass: Some(model_base),
            native_static_methods,
            ..Default::default()
        });
        let source = "let x = CreateUser.create({\"n\": 1});";
        let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
        let program = Parser::new(tokens).parse().expect("parser error");
        let module = Compiler::compile(&program).expect("compile error");
        let mut vm = Vm::new();
        vm.globals
            .insert("CreateUser".to_string(), Value::Class(user_class));
        vm.execute(&module.main).expect("create on VM");
        let x = vm.globals.get("x").expect("x");
        match x {
            Value::Hash(h) => assert_eq!(
                h.borrow()
                    .get(&crate::interpreter::value::HashKey::String("valid".into())),
                Some(&Value::Bool(true))
            ),
            other => panic!("expected hash, got {other:?}"),
        }
    }

    #[test]
    fn test_vm_model_class_create_veto_returns_instance() {
        use crate::interpreter::builtins::model::callbacks::register_callback;
        use crate::interpreter::value::{Class, Function};
        use std::collections::HashMap;
        use std::sync::atomic::{AtomicBool, Ordering};

        static NATIVE_RAN: AtomicBool = AtomicBool::new(false);
        NATIVE_RAN.store(false, Ordering::SeqCst);

        let body_tokens = Scanner::new("return false")
            .scan_tokens()
            .expect("lex veto");
        let body = Parser::new(body_tokens)
            .parse()
            .expect("parse veto")
            .statements;

        let model_base = Rc::new(Class {
            name: "Model".to_string(),
            ..Default::default()
        });
        let mut native_static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();
        native_static_methods.insert(
            "create".to_string(),
            Rc::new(NativeFunction::new("Model.create", None, |_| {
                NATIVE_RAN.store(true, Ordering::SeqCst);
                Ok(Value::Bool(true))
            })),
        );
        let user_class = Rc::new(Class {
            name: "CreateVeto".to_string(),
            superclass: Some(model_base),
            native_static_methods,
            ..Default::default()
        });
        user_class.methods.borrow_mut().insert(
            "veto".to_string(),
            Rc::new(Function {
                name: "veto".to_string(),
                body: Rc::new(body.into()),
                ..Function::default()
            }),
        );
        register_callback("CreateVeto", "before_save", "veto");

        let source = "let x = CreateVeto.create({\"n\": 1});";
        let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
        let program = Parser::new(tokens).parse().expect("parser error");
        let module = Compiler::compile(&program).expect("compile error");
        let mut vm = Vm::new();
        vm.globals
            .insert("CreateVeto".to_string(), Value::Class(user_class));
        vm.execute(&module.main).expect("veto create");
        assert!(matches!(vm.globals.get("x"), Some(Value::Instance(_))));
        assert!(!NATIVE_RAN.load(Ordering::SeqCst));
    }

    #[test]
    fn test_vm_model_delete_with_dependent_runs_on_vm() {
        use crate::interpreter::builtins::model::relations::{
            register_relation, DependentStrategy, RelationDef, RelationType,
        };
        use crate::interpreter::executor::calls::cascade;
        use crate::interpreter::value::{Class, Instance};
        use std::collections::HashMap;
        use std::sync::atomic::{AtomicBool, Ordering};

        static CHILD_DELETED: AtomicBool = AtomicBool::new(false);
        static OWNER_DELETED: AtomicBool = AtomicBool::new(false);
        CHILD_DELETED.store(false, Ordering::SeqCst);
        OWNER_DELETED.store(false, Ordering::SeqCst);

        let model_base = Rc::new(Class {
            name: "Model".to_string(),
            ..Default::default()
        });
        let mut child_natives: HashMap<String, Rc<NativeFunction>> = HashMap::new();
        child_natives.insert(
            "delete".to_string(),
            Rc::new(NativeFunction::new("Model.delete", None, |_| {
                CHILD_DELETED.store(true, Ordering::SeqCst);
                Ok(Value::Bool(true))
            })),
        );
        let child_class = Rc::new(Class {
            name: "CascadePost".to_string(),
            superclass: Some(model_base.clone()),
            native_methods: child_natives,
            ..Default::default()
        });
        let mut owner_natives: HashMap<String, Rc<NativeFunction>> = HashMap::new();
        owner_natives.insert(
            "delete".to_string(),
            Rc::new(NativeFunction::new("Model.delete", None, |_| {
                OWNER_DELETED.store(true, Ordering::SeqCst);
                Ok(Value::Bool(true))
            })),
        );
        let owner_class = Rc::new(Class {
            name: "CascadeUser".to_string(),
            superclass: Some(model_base),
            native_methods: owner_natives,
            ..Default::default()
        });
        register_relation(
            "CascadeUser",
            RelationDef {
                name: "posts".to_string(),
                relation_type: RelationType::HasMany,
                class_name: "CascadePost".to_string(),
                collection: "cascade_posts".to_string(),
                foreign_key: "user_id".to_string(),
                polymorphic_type_field: None,
                polymorphic_type_value: None,
                join_table: None,
                association_foreign_key: None,
                dependent: Some(DependentStrategy::Delete),
                through: None,
                source: None,
                counter_cache: None,
            },
        );

        let mut child_inst = Instance::new(child_class);
        child_inst.set("_key", Value::String("post-1".into()));
        let child = Value::Instance(Rc::new(RefCell::new(child_inst)));
        cascade::set_test_cascade_children("user-1", vec![child]);

        let mut owner_inst = Instance::new(owner_class);
        owner_inst.set("_key", Value::String("user-1".into()));
        let owner = Value::Instance(Rc::new(RefCell::new(owner_inst)));

        let source = "let x = record.delete();";
        let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
        let program = Parser::new(tokens).parse().expect("parser error");
        let module = Compiler::compile(&program).expect("compile error");
        let mut vm = Vm::new();
        vm.globals.insert("record".to_string(), owner);
        vm.execute(&module.main)
            .expect("dependent delete must stay on the VM");
        cascade::clear_test_cascade_children();
        assert_eq!(vm.globals.get("x"), Some(&Value::Bool(true)));
        assert!(CHILD_DELETED.load(Ordering::SeqCst), "child delete native");
        assert!(OWNER_DELETED.load(Ordering::SeqCst), "owner delete native");
    }

    #[test]
    fn test_vm_model_delete_cycle_is_noop_not_fallback() {
        use crate::interpreter::builtins::model::relations::{
            register_relation, DependentStrategy, RelationDef, RelationType,
        };
        use crate::interpreter::executor::calls::cascade;
        use crate::interpreter::value::{Class, Instance};
        use std::collections::HashMap;
        use std::sync::atomic::{AtomicUsize, Ordering};

        static OWNER_DELETES: AtomicUsize = AtomicUsize::new(0);
        OWNER_DELETES.store(0, Ordering::SeqCst);

        let model_base = Rc::new(Class {
            name: "Model".to_string(),
            ..Default::default()
        });
        let mut owner_natives: HashMap<String, Rc<NativeFunction>> = HashMap::new();
        owner_natives.insert(
            "delete".to_string(),
            Rc::new(NativeFunction::new("Model.delete", None, |_| {
                OWNER_DELETES.fetch_add(1, Ordering::SeqCst);
                Ok(Value::Bool(true))
            })),
        );
        let owner_class = Rc::new(Class {
            name: "CycleUser".to_string(),
            superclass: Some(model_base.clone()),
            native_methods: owner_natives,
            ..Default::default()
        });
        let mut child_natives: HashMap<String, Rc<NativeFunction>> = HashMap::new();
        child_natives.insert(
            "delete".to_string(),
            Rc::new(NativeFunction::new("Model.delete", None, |_| {
                Ok(Value::Bool(true))
            })),
        );
        let child_class = Rc::new(Class {
            name: "CyclePost".to_string(),
            superclass: Some(model_base),
            native_methods: child_natives,
            ..Default::default()
        });
        register_relation(
            "CycleUser",
            RelationDef {
                name: "posts".to_string(),
                relation_type: RelationType::HasMany,
                class_name: "CyclePost".to_string(),
                collection: "cycle_posts".to_string(),
                foreign_key: "user_id".to_string(),
                polymorphic_type_field: None,
                polymorphic_type_value: None,
                join_table: None,
                association_foreign_key: None,
                dependent: Some(DependentStrategy::Delete),
                through: None,
                source: None,
                counter_cache: None,
            },
        );
        register_relation(
            "CyclePost",
            RelationDef {
                name: "user".to_string(),
                relation_type: RelationType::HasMany,
                class_name: "CycleUser".to_string(),
                collection: "cycle_users".to_string(),
                foreign_key: "post_id".to_string(),
                polymorphic_type_field: None,
                polymorphic_type_value: None,
                join_table: None,
                association_foreign_key: None,
                dependent: Some(DependentStrategy::Delete),
                through: None,
                source: None,
                counter_cache: None,
            },
        );

        let mut owner_inst = Instance::new(owner_class);
        owner_inst.set("_key", Value::String("cycle-u".into()));
        let owner = Value::Instance(Rc::new(RefCell::new(owner_inst)));
        let mut child_inst = Instance::new(child_class);
        child_inst.set("_key", Value::String("cycle-p".into()));
        let child = Value::Instance(Rc::new(RefCell::new(child_inst)));
        cascade::set_test_cascade_children("cycle-u", vec![child.clone()]);
        cascade::set_test_cascade_children("cycle-p", vec![owner.clone()]);

        let source = "let x = record.delete();";
        let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
        let program = Parser::new(tokens).parse().expect("parser error");
        let module = Compiler::compile(&program).expect("compile error");
        let mut vm = Vm::new();
        vm.globals.insert("record".to_string(), owner);
        vm.execute(&module.main)
            .expect("cycle delete must stay on the VM");
        cascade::clear_test_cascade_children();
        assert_eq!(vm.globals.get("x"), Some(&Value::Bool(true)));
        assert_eq!(OWNER_DELETES.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_vm_model_delete_dependent_veto_skips_natives() {
        use crate::interpreter::builtins::model::callbacks::register_callback;
        use crate::interpreter::builtins::model::relations::{
            register_relation, DependentStrategy, RelationDef, RelationType,
        };
        use crate::interpreter::executor::calls::cascade;
        use crate::interpreter::value::{Class, Function, Instance};
        use std::collections::HashMap;
        use std::sync::atomic::{AtomicBool, Ordering};

        static CHILD_DELETED: AtomicBool = AtomicBool::new(false);
        static OWNER_DELETED: AtomicBool = AtomicBool::new(false);
        CHILD_DELETED.store(false, Ordering::SeqCst);
        OWNER_DELETED.store(false, Ordering::SeqCst);

        let body_tokens = Scanner::new("return false")
            .scan_tokens()
            .expect("lex veto");
        let body = Parser::new(body_tokens)
            .parse()
            .expect("parse veto")
            .statements;

        let model_base = Rc::new(Class {
            name: "Model".to_string(),
            ..Default::default()
        });
        let mut owner_natives: HashMap<String, Rc<NativeFunction>> = HashMap::new();
        owner_natives.insert(
            "delete".to_string(),
            Rc::new(NativeFunction::new("Model.delete", None, |_| {
                OWNER_DELETED.store(true, Ordering::SeqCst);
                Ok(Value::Bool(true))
            })),
        );
        let owner_class = Rc::new(Class {
            name: "VetoCascadeUser".to_string(),
            superclass: Some(model_base.clone()),
            native_methods: owner_natives,
            ..Default::default()
        });
        owner_class.methods.borrow_mut().insert(
            "veto".to_string(),
            Rc::new(Function {
                name: "veto".to_string(),
                body: Rc::new(body.into()),
                ..Function::default()
            }),
        );
        register_callback("VetoCascadeUser", "before_delete", "veto");
        register_relation(
            "VetoCascadeUser",
            RelationDef {
                name: "posts".to_string(),
                relation_type: RelationType::HasMany,
                class_name: "CascadePost".to_string(),
                collection: "cascade_posts".to_string(),
                foreign_key: "user_id".to_string(),
                polymorphic_type_field: None,
                polymorphic_type_value: None,
                join_table: None,
                association_foreign_key: None,
                dependent: Some(DependentStrategy::Delete),
                through: None,
                source: None,
                counter_cache: None,
            },
        );

        let mut child_natives: HashMap<String, Rc<NativeFunction>> = HashMap::new();
        child_natives.insert(
            "delete".to_string(),
            Rc::new(NativeFunction::new("Model.delete", None, |_| {
                CHILD_DELETED.store(true, Ordering::SeqCst);
                Ok(Value::Bool(true))
            })),
        );
        let child_class = Rc::new(Class {
            name: "VetoCascadePost".to_string(),
            superclass: Some(model_base),
            native_methods: child_natives,
            ..Default::default()
        });
        let mut child_inst = Instance::new(child_class);
        child_inst.set("_key", Value::String("post-v".into()));
        cascade::set_test_cascade_children(
            "user-v",
            vec![Value::Instance(Rc::new(RefCell::new(child_inst)))],
        );

        let mut owner_inst = Instance::new(owner_class);
        owner_inst.set("_key", Value::String("user-v".into()));
        let owner = Value::Instance(Rc::new(RefCell::new(owner_inst)));

        let source = "let x = record.delete();";
        let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
        let program = Parser::new(tokens).parse().expect("parser error");
        let module = Compiler::compile(&program).expect("compile error");
        let mut vm = Vm::new();
        vm.globals.insert("record".to_string(), owner);
        vm.execute(&module.main).expect("veto delete on VM");
        cascade::clear_test_cascade_children();
        assert_eq!(vm.globals.get("x"), Some(&Value::Bool(false)));
        assert!(!CHILD_DELETED.load(Ordering::SeqCst));
        assert!(!OWNER_DELETED.load(Ordering::SeqCst));
    }

    #[test]
    fn test_vm_model_delete_purges_attachments_on_vm() {
        use crate::interpreter::builtins::model::uploaders::{register_uploader, UploaderConfig};
        use crate::interpreter::value::{Class, Instance};
        use std::collections::HashMap;
        use std::sync::atomic::{AtomicBool, Ordering};

        static DETACHED: AtomicBool = AtomicBool::new(false);
        static OWNER_DELETED: AtomicBool = AtomicBool::new(false);
        DETACHED.store(false, Ordering::SeqCst);
        OWNER_DELETED.store(false, Ordering::SeqCst);

        let model_base = Rc::new(Class {
            name: "Model".to_string(),
            ..Default::default()
        });
        let mut natives: HashMap<String, Rc<NativeFunction>> = HashMap::new();
        natives.insert(
            "delete".to_string(),
            Rc::new(NativeFunction::new("Model.delete", None, |_| {
                OWNER_DELETED.store(true, Ordering::SeqCst);
                Ok(Value::Bool(true))
            })),
        );
        let owner_class = Rc::new(Class {
            name: "AttachUser".to_string(),
            superclass: Some(model_base),
            native_methods: natives,
            ..Default::default()
        });
        register_uploader(
            "AttachUser",
            UploaderConfig {
                name: "avatar".to_string(),
                multiple: false,
                content_types: vec!["image/png".to_string()],
                max_size: 1_000_000,
                collection: "attach_user_avatars".to_string(),
                format: None,
                quality: None,
                max_width: None,
                max_height: None,
                service: "disk".to_string(),
            },
        );
        let mut owner_inst = Instance::new(owner_class);
        owner_inst.set("_key", Value::String("attach-1".into()));
        let owner = Value::Instance(Rc::new(RefCell::new(owner_inst)));

        let source = "let x = record.delete();";
        let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
        let program = Parser::new(tokens).parse().expect("parser error");
        let module = Compiler::compile(&program).expect("compile error");
        let mut vm = Vm::new();
        vm.globals.insert("record".to_string(), owner);
        vm.globals.insert(
            "detach_all_uploads".to_string(),
            Value::NativeFunction(NativeFunction::new("detach_all_uploads", Some(1), |_| {
                DETACHED.store(true, Ordering::SeqCst);
                Ok(Value::Bool(true))
            })),
        );
        vm.execute(&module.main)
            .expect("attachment delete must stay on the VM");
        assert_eq!(vm.globals.get("x"), Some(&Value::Bool(true)));
        assert!(DETACHED.load(Ordering::SeqCst), "detach_all_uploads ran");
        assert!(OWNER_DELETED.load(Ordering::SeqCst));
    }

    #[test]
    fn test_vm_model_class_delete_loads_and_cascades() {
        use crate::interpreter::builtins::model::relations::{
            register_relation, DependentStrategy, RelationDef, RelationType,
        };
        use crate::interpreter::executor::calls::cascade;
        use crate::interpreter::value::{Class, Instance};
        use std::collections::HashMap;
        use std::sync::atomic::{AtomicBool, Ordering};

        static CHILD_DELETED: AtomicBool = AtomicBool::new(false);
        static OWNER_DELETED: AtomicBool = AtomicBool::new(false);
        CHILD_DELETED.store(false, Ordering::SeqCst);
        OWNER_DELETED.store(false, Ordering::SeqCst);

        let model_base = Rc::new(Class {
            name: "Model".to_string(),
            ..Default::default()
        });
        let mut child_natives: HashMap<String, Rc<NativeFunction>> = HashMap::new();
        child_natives.insert(
            "delete".to_string(),
            Rc::new(NativeFunction::new("Model.delete", None, |_| {
                CHILD_DELETED.store(true, Ordering::SeqCst);
                Ok(Value::Bool(true))
            })),
        );
        let child_class = Rc::new(Class {
            name: "ClassDelPost".to_string(),
            superclass: Some(model_base.clone()),
            native_methods: child_natives,
            ..Default::default()
        });
        let mut owner_natives: HashMap<String, Rc<NativeFunction>> = HashMap::new();
        owner_natives.insert(
            "delete".to_string(),
            Rc::new(NativeFunction::new("Model.delete", None, |_| {
                OWNER_DELETED.store(true, Ordering::SeqCst);
                Ok(Value::Bool(true))
            })),
        );
        let owner_class = Rc::new(Class {
            name: "ClassDelUser".to_string(),
            superclass: Some(model_base),
            native_methods: owner_natives,
            ..Default::default()
        });
        register_relation(
            "ClassDelUser",
            RelationDef {
                name: "posts".to_string(),
                relation_type: RelationType::HasMany,
                class_name: "ClassDelPost".to_string(),
                collection: "class_del_posts".to_string(),
                foreign_key: "user_id".to_string(),
                polymorphic_type_field: None,
                polymorphic_type_value: None,
                join_table: None,
                association_foreign_key: None,
                dependent: Some(DependentStrategy::Delete),
                through: None,
                source: None,
                counter_cache: None,
            },
        );

        let mut child_inst = Instance::new(child_class);
        child_inst.set("_key", Value::String("p-class".into()));
        cascade::set_test_cascade_children(
            "u-class",
            vec![Value::Instance(Rc::new(RefCell::new(child_inst)))],
        );

        let mut owner_inst = Instance::new(owner_class.clone());
        owner_inst.set("_key", Value::String("u-class".into()));
        cascade::set_test_loaded_instance(
            "u-class",
            Value::Instance(Rc::new(RefCell::new(owner_inst))),
        );

        let source = "let x = ClassDelUser.delete(\"u-class\");";
        let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
        let program = Parser::new(tokens).parse().expect("parser error");
        let module = Compiler::compile(&program).expect("compile error");
        let mut vm = Vm::new();
        vm.globals
            .insert("ClassDelUser".to_string(), Value::Class(owner_class));
        vm.execute(&module.main)
            .expect("class delete must stay on the VM");
        cascade::clear_test_cascade_children();
        cascade::clear_test_loaded_instances();
        assert_eq!(vm.globals.get("x"), Some(&Value::Bool(true)));
        assert!(CHILD_DELETED.load(Ordering::SeqCst));
        assert!(OWNER_DELETED.load(Ordering::SeqCst));
    }

    #[test]
    fn test_vm_model_delete_try_rescue_is_not_fallback() {
        use crate::interpreter::builtins::model::relations::{
            register_relation, DependentStrategy, RelationDef, RelationType,
        };
        use crate::interpreter::value::{Class, Instance};
        use std::collections::HashMap;

        let model_base = Rc::new(Class {
            name: "Model".to_string(),
            ..Default::default()
        });
        let mut natives: HashMap<String, Rc<NativeFunction>> = HashMap::new();
        natives.insert(
            "delete".to_string(),
            Rc::new(NativeFunction::new("Model.delete", None, |_| {
                Ok(Value::Bool(true))
            })),
        );
        let owner_class = Rc::new(Class {
            name: "RescueDelUser".to_string(),
            superclass: Some(model_base),
            native_methods: natives,
            ..Default::default()
        });
        register_relation(
            "RescueDelUser",
            RelationDef {
                name: "posts".to_string(),
                relation_type: RelationType::HasMany,
                class_name: "RescueDelPost".to_string(),
                collection: "rescue_posts".to_string(),
                foreign_key: "user_id".to_string(),
                polymorphic_type_field: None,
                polymorphic_type_value: None,
                join_table: None,
                association_foreign_key: None,
                dependent: Some(DependentStrategy::Delete),
                through: None,
                source: None,
                counter_cache: None,
            },
        );
        let mut owner_inst = Instance::new(owner_class);
        owner_inst.set("_key", Value::String("rescue-1".into()));
        let owner = Value::Instance(Rc::new(RefCell::new(owner_inst)));
        crate::interpreter::executor::calls::cascade::set_test_cascade_children("rescue-1", vec![]);

        let source =
            "let x = \"start\"\ntry { record.delete() } catch (e) { x = \"caught\" }\nlet y = record.delete() rescue \"caught\";";
        let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
        let program = Parser::new(tokens).parse().expect("parser error");
        let module = Compiler::compile(&program).expect("compile error");
        let mut vm = Vm::new();
        vm.globals.insert("record".to_string(), owner);
        vm.execute(&module.main)
            .expect("try/rescue must not see EngineFallback");
        crate::interpreter::executor::calls::cascade::clear_test_cascade_children();
        assert_eq!(vm.globals.get("x"), Some(&Value::String("start".into())));
        assert_eq!(vm.globals.get("y"), Some(&Value::Bool(true)));
    }

    #[test]
    fn test_vm_class_method_missing_runs_on_vm() {
        let source = r#"
class Mailer {
  def self.method_missing(name, args) {
    return name
  }
}
let x = Mailer.welcome()
"#;
        let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
        let program = Parser::new(tokens).parse().expect("parser error");
        let module = Compiler::compile(&program).expect("compile error");
        let mut vm = Vm::new();
        vm.execute(&module.main).expect("method_missing on VM");
        assert_eq!(vm.globals.get("x"), Some(&Value::String("welcome".into())));
    }

    #[test]
    fn test_vm_decimal_methods() {
        assert_eq!(
            compile_and_get_global("let d = 3.7D; let x = d.round;", "x"),
            Value::Int(4)
        );
        assert_eq!(
            compile_and_get_global("let d = 3.14159D; let x = d.round(2).to_s;", "x"),
            Value::String("3.14".into())
        );
        assert_eq!(
            compile_and_get_global("let d = 3.5D; let x = d.between?(3, 4);", "x"),
            Value::Bool(true)
        );
        assert_eq!(
            compile_and_get_global("let d = 3.5D; let x = d.is_a?(\"decimal\");", "x"),
            Value::Bool(true)
        );
        assert_eq!(
            compile_and_get_global("let d = -2.5D; let x = d.negative?;", "x"),
            Value::Bool(true)
        );
        assert_eq!(
            compile_and_get_global("let d = 2.5D; let x = d.class;", "x"),
            Value::String("decimal".into())
        );
    }

    /// A Decimal literal is parsed when it is compiled: its constant holds
    /// the value, with the precision the literal was written with, and the
    /// literal's text is not read again at run time.
    #[test]
    fn a_decimal_literal_compiles_to_its_value() {
        use crate::interpreter::value::DecimalValue;
        let source = "price = 19.99D\nlong = 1.12345678901234567890123456789012D";
        let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
        let program = Parser::new(tokens).parse().expect("parser error");
        let module = Compiler::compile(&program).expect("compile error");
        let decimals: Vec<&DecimalValue> = module
            .main
            .chunk
            .constants
            .iter()
            .filter_map(|constant| match constant {
                Constant::Decimal(decimal) => Some(decimal),
                _ => None,
            })
            .collect();
        let price = DecimalValue("19.99".parse().unwrap(), 2);
        // 32 digits written, 28 kept: the precision counts what was written.
        let long = DecimalValue("1.1234567890123456789012345679".parse().unwrap(), 32);
        assert_eq!(decimals, [&price, &long]);
        assert_eq!(
            compile_and_get_global(source, "price"),
            Value::Decimal(price)
        );
    }

    /// Text that is no decimal does not compile, where the VM used to load
    /// a silent 0. The tree-walker then runs the code, and raises "Invalid
    /// decimal literal" where the literal is evaluated.
    #[test]
    fn an_invalid_decimal_literal_does_not_compile() {
        for source in [
            "x = %n[abcD]",
            "big = 99999999999999999999999999999999.5D",
            "h = {99999999999999999999999999999999.5D: 1}",
        ] {
            let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
            let program = Parser::new(tokens).parse().expect("parser error");
            let err = Compiler::compile(&program).expect_err(source);
            assert!(
                err.to_string().contains("Invalid decimal literal: "),
                "{source}: {err}"
            );
        }
    }

    /// The fast tier loads a Decimal constant itself. It used to hand the op
    /// to the general arm, which parsed the literal's text on every load.
    #[test]
    fn a_decimal_constant_loads_in_the_fast_tier() {
        use crate::interpreter::value::DecimalValue;
        let decimal = DecimalValue::from_literal("19.99").unwrap();
        let mut proto = FunctionProto::new(String::new());
        let idx = proto.chunk.add_constant(Constant::Decimal(decimal.clone()));
        proto.chunk.emit(Op::Constant(idx), 1);
        let closure = Rc::new(VmClosure::new(Arc::new(proto), Vec::new()));
        let mut vm = Vm::new();
        vm.push(Value::VmClosure(closure.clone()));
        vm.frames
            .push(CallFrame::new(closure, 0, vm.iter_stack.len(), None, 0));
        // `None`: the chunk ran to its end without an op handed back.
        assert_eq!(vm.run_fast().expect("no error"), None);
        assert_eq!(vm.stack.last(), Some(&Value::Decimal(decimal)));
    }

    #[test]
    fn test_vm_int_to_s_radix() {
        assert_eq!(
            compile_and_get_global("let n = 255; let x = n.to_s(16);", "x"),
            Value::String("ff".into())
        );
        assert_eq!(
            compile_and_get_global("let n = 255; let x = n.to_string(2);", "x"),
            Value::String("11111111".into())
        );
        assert_eq!(
            compile_and_get_global("let n = -255; let x = n.to_s(16);", "x"),
            Value::String("-ff".into())
        );
        // Out-of-range base is a runtime error
        let r = compile_and_run("let n = 255; let x = n.to_s(1);");
        assert!(r.is_err());
        let r = compile_and_run("let n = 255; let x = n.to_s(37);");
        assert!(r.is_err());
    }

    #[test]
    fn test_vm_int_with_args_methods() {
        assert_eq!(
            compile_and_get_global("let n = 12; let x = n.gcd(8);", "x"),
            Value::Int(4)
        );
        assert_eq!(
            compile_and_get_global("let n = 4; let x = n.lcm(6);", "x"),
            Value::Int(12)
        );
        assert_eq!(
            compile_and_get_global("let n = 2; let x = n.pow(10);", "x"),
            Value::Int(1024)
        );
        assert_eq!(
            compile_and_get_global("let n = 5; let x = n.between?(1, 10);", "x"),
            Value::Bool(true)
        );
        assert_eq!(
            compile_and_get_global("let n = 15; let x = n.clamp(0, 10);", "x"),
            Value::Int(10)
        );
        assert_eq!(
            compile_and_get_global("let n = 42; let x = n.is_a?(\"int\");", "x"),
            Value::Bool(true)
        );
        // Unknown int method errors
        let r = compile_and_run("let n = 42; let x = n.frobnicate(1);");
        assert!(r.is_err());
    }

    #[test]
    fn test_vm_int_closure_methods() {
        assert_eq!(
            compile_and_get_global(
                "let sum = 0; let n = 5; n.times(fn(i) { sum = sum + i; }); let x = sum;",
                "x"
            ),
            Value::Int(10)
        );
        assert_eq!(
            compile_and_get_global(
                "let sum = 0; let n = 1; n.upto(4, fn(i) { sum = sum + i; }); let x = sum;",
                "x"
            ),
            Value::Int(10)
        );
        assert_eq!(
            compile_and_get_global(
                "let last = 0; let n = 3; n.downto(1, fn(i) { last = i; }); let x = last;",
                "x"
            ),
            Value::Int(1)
        );
    }

    #[test]
    fn test_vm_array_is_a() {
        assert_eq!(
            compile_and_get_global(r#"let x = [1].is_a?("array");"#, "x"),
            Value::Bool(true)
        );
        assert_eq!(
            compile_and_get_global(r#"let x = [1].is_a?("hash");"#, "x"),
            Value::Bool(false)
        );
    }

    // ========================================================================
    // VM call dispatch + closures (vm_calls.rs, upvalue.rs)
    // ========================================================================

    #[test]
    fn test_vm_closure_captures_local() {
        let result = compile_and_get_global(
            r#"
            fn make_adder(x: Int) -> Function {
                return fn(y) { return x + y; };
            }
            let add5 = make_adder(5);
            let val = add5(3);
            "#,
            "val",
        );
        assert_eq!(result, Value::Int(8));
    }

    #[test]
    fn test_vm_closure_mutates_upvalue() {
        let result = compile_and_get_global(
            r#"
            fn make_counter() -> Function {
                let n = 0;
                return fn() {
                    n = n + 1;
                    return n;
                };
            }
            let c = make_counter();
            c();
            c();
            let val = c();
            "#,
            "val",
        );
        assert_eq!(result, Value::Int(3));
    }

    #[test]
    fn test_vm_lambda_pipe_syntax() {
        let result = compile_and_get_global(
            r#"
            let double = |x| { return x * 2; };
            let val = double(7);
            "#,
            "val",
        );
        assert_eq!(result, Value::Int(14));
    }

    #[test]
    fn test_vm_implicit_return() {
        let result = compile_and_get_global(
            r#"
            fn add(a: Int, b: Int) -> Int {
                a + b
            }
            let val = add(3, 4);
            "#,
            "val",
        );
        assert_eq!(result, Value::Int(7));
    }

    /// A `try` ending a function is worth the value of the branch that ran;
    /// it was null on both engines.
    #[test]
    fn test_vm_try_in_tail_position_returns_branch_value() {
        let src = "def f(n)\n  try\n    10 / n\n  catch e\n    \"caught\"\n  end\nend\n";
        let ok = compile_and_get_global(&format!("{src}val = f(2)\n"), "val");
        assert_eq!(ok, Value::Int(5));
        let caught = compile_and_get_global(&format!("{src}val = f(0)\n"), "val");
        assert_eq!(caught.to_string(), "caught");
    }

    #[test]
    fn test_vm_try_finally_in_tail_position_runs_finally_and_keeps_value() {
        let src = "log = []\ndef f\n  try\n    1\n  finally\n    log.push(\"f\")\n  end\nend\nval = [f(), log.length]\n";
        let val = compile_and_get_global(src, "val");
        assert_eq!(val.to_string(), "[1, 1]");
    }

    /// `x = try … end`: `try` used as a value.
    #[test]
    fn test_vm_try_as_expression() {
        let src = "x = try\n  throw \"boom\"\ncatch e\n  \"got #{e}\"\nend\ny = try\n  42\ncatch e\n  0\nend\nval = [x, y]\n";
        let val = compile_and_get_global(src, "val");
        assert_eq!(val.to_string(), "[got boom, 42]");
    }

    // ========================================================================
    // VM expression compilation coverage (compiler_exprs.rs)
    // ========================================================================

    #[test]
    fn test_vm_unary_minus_not() {
        assert_eq!(compile_and_get_global("let x = -5;", "x"), Value::Int(-5));
        assert_eq!(
            compile_and_get_global("let x = !true;", "x"),
            Value::Bool(false)
        );
        assert_eq!(
            compile_and_get_global("let x = !false;", "x"),
            Value::Bool(true)
        );
    }

    #[test]
    fn test_vm_modulo_division() {
        assert_eq!(
            compile_and_get_global("let x = 17 % 5;", "x"),
            Value::Int(2)
        );
        assert_eq!(
            compile_and_get_global("let x = 20 / 4;", "x"),
            Value::Int(5)
        );
    }

    #[test]
    fn test_vm_float_arithmetic() {
        let r = compile_and_run("let x = 3.14 + 2.86;");
        assert!(r.is_ok());
        let r = compile_and_run("let x = 1.5 * 2.0;");
        assert!(r.is_ok());
    }

    #[test]
    fn test_vm_short_circuit_and_or() {
        assert_eq!(
            compile_and_get_global("let x = false && (1 / 0);", "x"),
            Value::Bool(false)
        );
        assert_eq!(
            compile_and_get_global("let x = true || (1 / 0);", "x"),
            Value::Bool(true)
        );
    }

    #[test]
    fn test_vm_ternary_via_if_else() {
        let result = compile_and_get_global(
            r#"
            let x = 0;
            if (5 > 3) { x = 1; } else { x = 2; }
            "#,
            "x",
        );
        assert_eq!(result, Value::Int(1));
    }

    #[test]
    fn test_vm_for_loop_array() {
        let result = compile_and_get_global(
            r#"
            let total = 0;
            for n in [1, 2, 3, 4, 5] {
                total = total + n;
            }
            "#,
            "total",
        );
        assert_eq!(result, Value::Int(15));
    }

    #[test]
    fn test_vm_array_spread() {
        let r = compile_and_run("let a = [1, 2]; let b = [...a, 3, 4]; let x = b.length();");
        assert!(r.is_ok());
    }

    #[test]
    fn test_vm_index_assignment() {
        let result = compile_and_get_global(
            r#"
            let a = [10, 20, 30];
            a[1] = 99;
            let x = a[1];
            "#,
            "x",
        );
        assert_eq!(result, Value::Int(99));
    }

    #[test]
    fn test_vm_hash_index_assignment() {
        let result = compile_and_get_global(
            r#"
            let h = {"a": 1};
            h["b"] = 2;
            let x = h["b"];
            "#,
            "x",
        );
        assert_eq!(result, Value::Int(2));
    }

    #[test]
    fn test_vm_pipeline_operator() {
        let result = compile_and_get_global(
            r#"
            let double = fn(x) { return x * 2; };
            let inc = fn(x) { return x + 1; };
            let val = 5 |> double() |> inc();
            "#,
            "val",
        );
        assert_eq!(result, Value::Int(11));
    }

    #[test]
    fn test_vm_nullish_coalescing() {
        assert_eq!(
            compile_and_get_global("let x = null ?? 42;", "x"),
            Value::Int(42)
        );
        assert_eq!(
            compile_and_get_global("let x = 7 ?? 42;", "x"),
            Value::Int(7)
        );
    }

    #[test]
    fn test_vm_postfix_if_unless() {
        let result = compile_and_get_global(
            r#"
            let x = 0;
            x = 42 if true;
            "#,
            "x",
        );
        assert_eq!(result, Value::Int(42));

        let result = compile_and_get_global(
            r#"
            let x = 0;
            x = 42 unless true;
            "#,
            "x",
        );
        assert_eq!(result, Value::Int(0));
    }

    // ========================================================================
    // VM string method dispatch (vm_string_methods.rs)
    // ========================================================================

    #[test]
    fn test_vm_string_upcase_downcase() {
        assert_eq!(
            compile_and_get_global(r#"let x = "hello".upcase();"#, "x"),
            Value::String("HELLO".into())
        );
        assert_eq!(
            compile_and_get_global(r#"let x = "HELLO".downcase();"#, "x"),
            Value::String("hello".into())
        );
        assert_eq!(
            compile_and_get_global(r#"let x = "abc".uppercase();"#, "x"),
            Value::String("ABC".into())
        );
        assert_eq!(
            compile_and_get_global(r#"let x = "ABC".lowercase();"#, "x"),
            Value::String("abc".into())
        );
    }

    #[test]
    fn test_vm_string_len_size() {
        assert_eq!(
            compile_and_get_global(r#"let x = "hello".len();"#, "x"),
            Value::Int(5)
        );
        assert_eq!(
            compile_and_get_global(r#"let x = "hi".length();"#, "x"),
            Value::Int(2)
        );
        assert_eq!(
            compile_and_get_global(r#"let x = "abc".size();"#, "x"),
            Value::Int(3)
        );
    }

    #[test]
    fn test_vm_string_trim_strip() {
        assert_eq!(
            compile_and_get_global(r#"let x = "  hi  ".trim();"#, "x"),
            Value::String("hi".into())
        );
        assert_eq!(
            compile_and_get_global(r#"let x = "  hi  ".strip();"#, "x"),
            Value::String("hi".into())
        );
        assert_eq!(
            compile_and_get_global(r#"let x = "  hi  ".lstrip();"#, "x"),
            Value::String("hi  ".into())
        );
        assert_eq!(
            compile_and_get_global(r#"let x = "  hi  ".rstrip();"#, "x"),
            Value::String("  hi".into())
        );
    }

    #[test]
    fn test_vm_string_capitalize_swapcase() {
        assert_eq!(
            compile_and_get_global(r#"let x = "hello world".capitalize();"#, "x"),
            Value::String("Hello world".into())
        );
        assert_eq!(
            compile_and_get_global(r#"let x = "Hello World".swapcase();"#, "x"),
            Value::String("hELLO wORLD".into())
        );
        assert_eq!(
            compile_and_get_global(r#"let x = "".capitalize();"#, "x"),
            Value::String("".into())
        );
    }

    #[test]
    fn test_vm_string_chomp() {
        assert_eq!(
            compile_and_get_global("let x = \"hello\\n\".chomp();", "x"),
            Value::String("hello".into())
        );
        assert_eq!(
            compile_and_get_global(r#"let x = "hello".chomp();"#, "x"),
            Value::String("hello".into())
        );
    }

    #[test]
    fn test_vm_string_reverse() {
        assert_eq!(
            compile_and_get_global(r#"let x = "abc".reverse();"#, "x"),
            Value::String("cba".into())
        );
    }

    #[test]
    fn test_vm_string_chars_bytes() {
        let r = compile_and_run(r#"let x = "abc".chars();"#);
        assert!(r.is_ok());
        let r = compile_and_run(r#"let x = "abc".bytes();"#);
        assert!(r.is_ok());
        let r = compile_and_run(r#"let x = "a\nb\nc".lines();"#);
        assert!(r.is_ok());
        assert_eq!(
            compile_and_get_global(r#"let x = "abc".bytesize();"#, "x"),
            Value::Int(3)
        );
    }

    #[test]
    fn test_vm_string_empty_predicate() {
        assert_eq!(
            compile_and_get_global(r#"let x = "".empty?();"#, "x"),
            Value::Bool(true)
        );
        assert_eq!(
            compile_and_get_global(r#"let x = "x".empty?();"#, "x"),
            Value::Bool(false)
        );
    }

    #[test]
    #[allow(clippy::approx_constant)] // "3.14" is test input for to_f(), not π
    fn test_vm_string_to_i_to_f() {
        assert_eq!(
            compile_and_get_global(r#"let x = "42".to_i();"#, "x"),
            Value::Int(42)
        );
        assert_eq!(
            compile_and_get_global(r#"let x = "  17  ".to_int();"#, "x"),
            Value::Int(17)
        );
        assert_eq!(
            compile_and_get_global(r#"let x = "abc".to_i();"#, "x"),
            Value::Int(0)
        );
        assert_eq!(
            compile_and_get_global(r#"let x = "3.14".to_f();"#, "x"),
            Value::Float(3.14)
        );
    }

    #[test]
    fn test_vm_string_ord() {
        assert_eq!(
            compile_and_get_global(r#"let x = "A".ord();"#, "x"),
            Value::Int(65)
        );
    }

    #[test]
    fn test_vm_string_contains_starts_ends() {
        assert_eq!(
            compile_and_get_global(r#"let x = "hello world".contains("world");"#, "x"),
            Value::Bool(true)
        );
        assert_eq!(
            compile_and_get_global(r#"let x = "hello".starts_with("he");"#, "x"),
            Value::Bool(true)
        );
        assert_eq!(
            compile_and_get_global(r#"let x = "hello".ends_with?("lo");"#, "x"),
            Value::Bool(true)
        );
        assert_eq!(
            compile_and_get_global(r#"let x = "hello".include?("ell");"#, "x"),
            Value::Bool(true)
        );
    }

    #[test]
    fn test_vm_string_split() {
        let r = compile_and_run(r#"let x = "a,b,c".split(",");"#);
        assert!(r.is_ok());
        let r = compile_and_run(r#"let x = "abc".split("");"#);
        assert!(r.is_ok());
    }

    #[test]
    fn test_vm_string_index_of_count() {
        assert_eq!(
            compile_and_get_global(r#"let x = "hello".index_of("l");"#, "x"),
            Value::Int(2)
        );
        assert_eq!(
            compile_and_get_global(r#"let x = "hello".index_of("z");"#, "x"),
            Value::Int(-1)
        );
        assert_eq!(
            compile_and_get_global(r#"let x = "banana".count("a");"#, "x"),
            Value::Int(3)
        );
    }

    #[test]
    fn test_vm_string_delete_prefix_suffix() {
        assert_eq!(
            compile_and_get_global(r#"let x = "hello".delete("l");"#, "x"),
            Value::String("heo".into())
        );
        assert_eq!(
            compile_and_get_global(r#"let x = "hello".delete_prefix("he");"#, "x"),
            Value::String("llo".into())
        );
        assert_eq!(
            compile_and_get_global(r#"let x = "hello".delete_suffix("lo");"#, "x"),
            Value::String("hel".into())
        );
    }

    #[test]
    fn test_vm_string_partition() {
        let r = compile_and_run(r#"let x = "key=value".partition("=");"#);
        assert!(r.is_ok());
        let r = compile_and_run(r#"let x = "a-b-c".rpartition("-");"#);
        assert!(r.is_ok());
    }

    #[test]
    fn test_vm_string_replace_gsub_sub() {
        assert_eq!(
            compile_and_get_global(r#"let x = "hello".replace("l", "L");"#, "x"),
            Value::String("heLLo".into())
        );
        assert_eq!(
            compile_and_get_global(r#"let x = "hello".gsub("l", "L");"#, "x"),
            Value::String("heLLo".into())
        );
        assert_eq!(
            compile_and_get_global(r#"let x = "hello".sub("l", "L");"#, "x"),
            Value::String("heLlo".into())
        );
        assert_eq!(
            compile_and_get_global(r#"let x = "abc".tr("ab", "xy");"#, "x"),
            Value::String("xyc".into())
        );
    }

    #[test]
    fn test_vm_string_substring_insert() {
        assert_eq!(
            compile_and_get_global(r#"let x = "hello".substring(1, 4);"#, "x"),
            Value::String("ell".into())
        );
        assert_eq!(
            compile_and_get_global(r#"let x = "abc".insert(1, "X");"#, "x"),
            Value::String("aXbc".into())
        );
    }

    #[test]
    fn test_vm_string_padding() {
        assert_eq!(
            compile_and_get_global(r#"let x = "ab".center(6);"#, "x"),
            Value::String("  ab  ".into())
        );
        assert_eq!(
            compile_and_get_global(r#"let x = "ab".ljust(5);"#, "x"),
            Value::String("ab   ".into())
        );
        assert_eq!(
            compile_and_get_global(r#"let x = "ab".rjust(5);"#, "x"),
            Value::String("   ab".into())
        );
        assert_eq!(
            compile_and_get_global(r#"let x = "ab".lpad(4, "0");"#, "x"),
            Value::String("00ab".into())
        );
        assert_eq!(
            compile_and_get_global(r#"let x = "ab".rpad(4, "0");"#, "x"),
            Value::String("ab00".into())
        );
    }

    #[test]
    fn test_vm_string_truncate() {
        assert_eq!(
            compile_and_get_global(r#"let x = "hello world".truncate(8);"#, "x"),
            Value::String("hello...".into())
        );
        assert_eq!(
            compile_and_get_global(r#"let x = "hi".truncate(8);"#, "x"),
            Value::String("hi".into())
        );
    }

    #[test]
    fn test_vm_string_universal_methods() {
        assert_eq!(
            compile_and_get_global(r#"let x = "abc".is_a?("string");"#, "x"),
            Value::Bool(true)
        );
        assert_eq!(
            compile_and_get_global(r#"let x = "abc".is_a?("int");"#, "x"),
            Value::Bool(false)
        );
        assert_eq!(
            compile_and_get_global(r#"let x = "abc".to_sym();"#, "x"),
            Value::Symbol("abc".into())
        );
        // Zero-arg universal methods on strings need to be invoked through
        // member access; the VM exposes them by name without auto-invocation,
        // so we just verify the call dispatches without crashing.
        let r = compile_and_run(r#"let x = "abc".class;"#);
        assert!(r.is_ok());
        let r = compile_and_run(r#"let x = "abc".nil?;"#);
        assert!(r.is_ok());
        let r = compile_and_run(r#"let x = "".blank?;"#);
        assert!(r.is_ok());
        let r = compile_and_run(r#"let x = "abc".present?;"#);
        assert!(r.is_ok());
    }

    // ========================================================================
    // VM hash method dispatch (vm_hash_methods.rs)
    // ========================================================================

    #[test]
    fn test_vm_hash_basic_ops() {
        let r = compile_and_run(r#"let h = {"a": 1, "b": 2}; h.set("c", 3);"#);
        assert!(r.is_ok());
        let r = compile_and_run(r#"let h = {"a": 1}; h.delete("a");"#);
        assert!(r.is_ok());
        let r = compile_and_run(r#"let h = {"a": 1}; h.clear();"#);
        assert!(r.is_ok());
    }

    #[test]
    fn test_vm_hash_size_empty() {
        assert_eq!(
            compile_and_get_global(r#"let x = {"a": 1, "b": 2}.length();"#, "x"),
            Value::Int(2)
        );
        assert_eq!(
            compile_and_get_global(r#"let x = {"a": 1}.len();"#, "x"),
            Value::Int(1)
        );
        assert_eq!(
            compile_and_get_global(r#"let x = {}.empty?();"#, "x"),
            Value::Bool(true)
        );
        assert_eq!(
            compile_and_get_global(r#"let x = {"a": 1}.empty?();"#, "x"),
            Value::Bool(false)
        );
    }

    #[test]
    fn test_vm_hash_keys_values_entries() {
        let r = compile_and_run(r#"let x = {"a": 1, "b": 2}.keys();"#);
        assert!(r.is_ok());
        let r = compile_and_run(r#"let x = {"a": 1, "b": 2}.values();"#);
        assert!(r.is_ok());
        let r = compile_and_run(r#"let x = {"a": 1, "b": 2}.entries();"#);
        assert!(r.is_ok());
    }

    #[test]
    fn test_vm_hash_has_key_get() {
        assert_eq!(
            compile_and_get_global(r#"let x = {"a": 1}.has_key("a");"#, "x"),
            Value::Bool(true)
        );
        assert_eq!(
            compile_and_get_global(r#"let x = {"a": 1}.has_key("b");"#, "x"),
            Value::Bool(false)
        );
        assert_eq!(
            compile_and_get_global(r#"let x = {"a": 7}.get("a");"#, "x"),
            Value::Int(7)
        );
        assert_eq!(
            compile_and_get_global(r#"let x = {"a": 7}.fetch("missing", "default");"#, "x"),
            Value::String("default".into())
        );
    }

    #[test]
    fn test_vm_hash_merge_compact_invert() {
        let r = compile_and_run(r#"let x = {"a": 1}.merge({"b": 2});"#);
        assert!(r.is_ok());
        let r = compile_and_run(r#"let x = {"a": 1, "b": null}.compact();"#);
        assert!(r.is_ok());
        let r = compile_and_run(r#"let x = {"a": "x", "b": "y"}.invert();"#);
        assert!(r.is_ok());
    }

    #[test]
    fn test_vm_hash_universal() {
        assert_eq!(
            compile_and_get_global(r#"let x = {"a": 1}.is_a?("hash");"#, "x"),
            Value::Bool(true)
        );
        // Zero-arg universal methods dispatch but aren't auto-invoked in VM mode.
        let r = compile_and_run(r#"let x = {"a": 1}.class;"#);
        assert!(r.is_ok());
        let r = compile_and_run(r#"let x = {}.blank?;"#);
        assert!(r.is_ok());
        let r = compile_and_run(r#"let x = {"a": 1}.present?;"#);
        assert!(r.is_ok());
    }

    // ========================================================================
    // VM exception dispatch (vm_exceptions.rs)
    //
    // The VM's `throw`+`catch`+global-reassignment path has open issues
    // (assignment inside the catch block does not propagate to outer globals)
    // so we only assert the throw-path compiles, the no-throw path runs, and
    // an uncaught throw surfaces as a `RuntimeError`.
    // ========================================================================

    #[test]
    fn test_vm_try_no_throw() {
        let result = compile_and_get_global(
            r#"
            let val = 0;
            try {
                val = 42;
            } catch (e) {
                val = -1;
            }
            "#,
            "val",
        );
        assert_eq!(result, Value::Int(42));
    }

    #[test]
    fn test_vm_throw_compiles_and_dispatches() {
        // The catch handler runs; we just don't assert the inner state because
        // global reassignment from inside the catch block isn't wired.
        let r = compile_and_run(
            r#"
            try {
                throw "boom";
            } catch (e) {
            }
            "#,
        );
        assert!(
            r.is_ok(),
            "try/catch with throw should not error: {:?}",
            r.err()
        );
    }

    /// The sentinel builtins must still be recognised when the compiler is
    /// seeded with the worker's globals, which is how `serve` compiles.
    ///
    /// This is not hypothetical. `next` was first written to ask
    /// `known_globals` whether the name was a user global — which is true at
    /// the CLI and FALSE-negative under `serve`, because that set is seeded
    /// with the whole global table, builtins included. The result compiled
    /// clean and was silently ignored again, exactly the bug the native
    /// compilation was meant to end, and only a request through a real server
    /// showed it. `program_globals` answers the question the code actually
    /// means, and this pins it.
    #[test]
    fn sentinel_builtins_survive_a_seeded_global_table() {
        fn compile_seeded(source: &str) -> Result<(), crate::error::CompileError> {
            let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
            let program = Parser::new(tokens).parse().expect("parser error");
            // Mimic serve: seed with a global table that contains the builtins.
            let seeded = ["next", "debug", "print", "len"].map(str::to_string);
            Compiler::compile_with_globals(&program, seeded).map(|_| ())
        }

        // `debug` is still refused even though it is in the seeded table.
        let err = compile_seeded("for i in [1, 2] { debug }")
            .expect_err("`debug` must be refused even when seeded as a global");
        assert!(
            err.to_string().contains("debug"),
            "expected the debug refusal, got: {err}"
        );

        // `next` still compiles as control flow, not as a variable read.
        compile_seeded("for i in [1, 2] { next }")
            .unwrap_or_else(|e| panic!("`next` must compile when seeded, got: {e}"));

        // And a program that declares its own `next` still gets a variable.
        compile_seeded("let next = 42\nprint(next + 1)")
            .unwrap_or_else(|e| panic!("a declared `next` must compile, got: {e}"));
    }

    /// `debug()` is refused so the handler falls back to the engine that
    /// implements it; a program's own variable of that name is not.
    ///
    /// `debug()` signals through a sentinel `Value` the compiled engine has no
    /// handling for — compiled, it was evaluated and discarded, so the
    /// breakpoint never fired. `next` was the other one and is now compiled
    /// natively as a jump, so only this one is left.
    ///
    /// The refusal must hit only the *builtin*: a program that declares its own
    /// `debug` is talking about its own variable, and demoting for that would
    /// cost speed for nothing.
    #[test]
    fn test_vm_refuses_sentinel_builtin_but_not_user_globals() {
        fn compile_src(source: &str) -> Result<(), crate::error::CompileError> {
            let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
            let program = Parser::new(tokens).parse().expect("parser error");
            Compiler::compile(&program).map(|_| ())
        }

        let err = compile_src("for i in [1, 2] { debug }")
            .expect_err("the sentinel builtin must be refused");
        assert!(
            err.to_string().contains("debug"),
            "expected the debug refusal, got: {err}"
        );

        // A user-declared global or local of the same name is not the builtin.
        compile_src("let debug = 42\nprint(debug + 1)")
            .unwrap_or_else(|e| panic!("a global named `debug` must compile, got: {e}"));
        compile_src("fn f() { let debug = 1\n  return debug }")
            .unwrap_or_else(|e| panic!("a local named `debug` must compile, got: {e}"));

        // `next` compiles now, in both spellings, and a user global of that
        // name stays an ordinary variable.
        compile_src("for i in [1, 2] { next }")
            .unwrap_or_else(|e| panic!("`next` must compile, got: {e}"));
        compile_src("for i in [1, 2] { next() }")
            .unwrap_or_else(|e| panic!("`next()` must compile, got: {e}"));
        compile_src("let next = 42\nprint(next + 1)")
            .unwrap_or_else(|e| panic!("a global named `next` must compile, got: {e}"));
    }

    /// `try`/`finally` compiles, and the block runs on the edges that used to
    /// skip it.
    ///
    /// This test used to assert the compiler *refused* `finally` — the stopgap
    /// after the compiled version was found to run the block only when control
    /// fell off the end of the try. It is compiled properly now: the block is
    /// inlined on every edge that leaves, so the refusal is gone and the
    /// handler no longer demotes for it.
    #[test]
    fn test_vm_compiles_finally_and_runs_it_on_return() {
        // Falls off the end: the block runs.
        let val = compile_and_get_global(
            r#"
            let val = "";
            try {
                val = "ok";
            } finally {
                val = val + "+final";
            }
            "#,
            "val",
        );
        assert_eq!(val, Value::String("ok+final".into()));

        // Leaves by `return`: the block still runs, and the returned value is
        // the one computed before it. This is the edge that dropped cleanup —
        // `cleaned` is appended only if the block ran, and the returned value
        // proves the block did not disturb the value on the stack.
        let out = compile_and_get_global(
            r#"
            let cleaned = "no";
            fn probe() {
                try {
                    return "returned";
                } finally {
                    cleaned = "yes";
                }
            }
            let out = probe() + "/" + cleaned;
            "#,
            "out",
        );
        assert_eq!(out, Value::String("returned/yes".into()));
    }

    #[test]
    fn test_vm_uncaught_exception_returns_error() {
        let r = compile_and_run(r#"throw "unhandled";"#);
        assert!(r.is_err(), "uncaught throw should surface as RuntimeError");
    }

    // ========================================================================
    // VM pattern matching (compiler_patterns.rs)
    //
    // Literal/wildcard/string match arms are implemented; guards and structural
    // patterns (array/hash) hit unfinished code paths in the compiler that
    // panic on the VM. We exercise only the arms that are wired end-to-end.
    // ========================================================================

    #[test]
    fn test_vm_match_literal() {
        let result = compile_and_get_global(
            r#"
            let val = match 42 {
                1 => "one",
                42 => "the answer",
                _ => "other"
            };
            "#,
            "val",
        );
        assert_eq!(result, Value::String("the answer".into()));
    }

    #[test]
    fn test_vm_match_wildcard_default() {
        let result = compile_and_get_global(
            r#"
            let val = match 99 {
                1 => "one",
                2 => "two",
                _ => "fallback"
            };
            "#,
            "val",
        );
        assert_eq!(result, Value::String("fallback".into()));
    }

    #[test]
    fn test_vm_match_string() {
        let result = compile_and_get_global(
            r#"
            let val = match "hi" {
                "bye" => 1,
                "hi" => 2,
                _ => 3
            };
            "#,
            "val",
        );
        assert_eq!(result, Value::Int(2));
    }

    // ========================================================================
    // Additional compiler/expr coverage (compiler_exprs.rs, vm_calls.rs)
    // ========================================================================

    #[test]
    fn test_vm_string_concat_operator() {
        assert_eq!(
            compile_and_get_global(r#"let x = "foo" + "bar";"#, "x"),
            Value::String("foobar".into())
        );
        assert_eq!(
            compile_and_get_global(r#"let x = "x" + "y" + "z";"#, "x"),
            Value::String("xyz".into())
        );
    }

    #[test]
    fn test_vm_negative_int_literal() {
        assert_eq!(compile_and_get_global("let x = -42;", "x"), Value::Int(-42));
    }

    #[test]
    fn test_vm_chained_comparisons() {
        assert_eq!(
            compile_and_get_global("let x = 1 < 2 && 2 < 3;", "x"),
            Value::Bool(true)
        );
        assert_eq!(
            compile_and_get_global("let x = 1 < 2 && 2 > 3;", "x"),
            Value::Bool(false)
        );
    }

    #[test]
    fn test_vm_equality_comparison() {
        assert_eq!(
            compile_and_get_global("let x = 1 == 1;", "x"),
            Value::Bool(true)
        );
        assert_eq!(
            compile_and_get_global("let x = 1 != 2;", "x"),
            Value::Bool(true)
        );
        assert_eq!(
            compile_and_get_global(r#"let x = "a" == "a";"#, "x"),
            Value::Bool(true)
        );
    }

    #[test]
    fn test_vm_array_literal_with_expressions() {
        let r = compile_and_run("let x = [1 + 1, 2 * 3, 10 / 2];");
        assert!(r.is_ok());
    }

    #[test]
    fn test_vm_hash_literal() {
        let r = compile_and_run(r#"let x = {"name": "alice", "age": 30, "active": true};"#);
        assert!(r.is_ok());
        assert_eq!(
            compile_and_get_global(r#"let x = {"a": 1, "b": 2}["a"];"#, "x"),
            Value::Int(1)
        );
    }

    #[test]
    fn test_vm_nested_array_indexing() {
        assert_eq!(
            compile_and_get_global("let a = [[1, 2], [3, 4]]; let x = a[1][0];", "x"),
            Value::Int(3)
        );
    }

    #[test]
    fn test_vm_array_destructure_in_index() {
        // Slicing-like: a[0], a[len-1] etc.
        assert_eq!(
            compile_and_get_global("let a = [10, 20, 30]; let x = a[2];", "x"),
            Value::Int(30)
        );
    }

    #[test]
    fn test_vm_function_with_multiple_returns() {
        let result = compile_and_get_global(
            r#"
            fn classify(n: Int) -> String {
                if (n > 0) {
                    return "positive";
                }
                if (n < 0) {
                    return "negative";
                }
                return "zero";
            }
            let val = classify(-5);
            "#,
            "val",
        );
        assert_eq!(result, Value::String("negative".into()));
    }

    #[test]
    fn test_vm_function_no_args() {
        let result = compile_and_get_global(
            r#"
            fn greet() -> String {
                return "hi";
            }
            let val = greet();
            "#,
            "val",
        );
        assert_eq!(result, Value::String("hi".into()));
    }

    #[test]
    fn test_vm_recursive_factorial() {
        let result = compile_and_get_global(
            r#"
            fn fact(n: Int) -> Int {
                if (n <= 1) { return 1; }
                return n * fact(n - 1);
            }
            let val = fact(6);
            "#,
            "val",
        );
        assert_eq!(result, Value::Int(720));
    }

    #[test]
    fn test_vm_higher_order_function() {
        let result = compile_and_get_global(
            r#"
            fn apply(f: Function, x: Int) -> Int {
                return f(x);
            }
            let inc = fn(n) { return n + 1; };
            let val = apply(inc, 41);
            "#,
            "val",
        );
        assert_eq!(result, Value::Int(42));
    }

    #[test]
    fn test_vm_closure_returns_closure() {
        let result = compile_and_get_global(
            r#"
            fn outer(x: Int) -> Function {
                return fn(y) {
                    return fn(z) {
                        return x + y + z;
                    };
                };
            }
            let f = outer(1);
            let g = f(2);
            let val = g(3);
            "#,
            "val",
        );
        assert_eq!(result, Value::Int(6));
    }

    #[test]
    fn test_vm_method_call_on_string_via_local() {
        let result = compile_and_get_global(
            r#"
            let s = "hello";
            let upper = s.upcase();
            "#,
            "upper",
        );
        assert_eq!(result, Value::String("HELLO".into()));
    }

    #[test]
    fn test_vm_method_chain() {
        let result = compile_and_get_global(r#"let val = "  hello  ".trim().upcase();"#, "val");
        assert_eq!(result, Value::String("HELLO".into()));
    }

    #[test]
    fn test_vm_array_method_chain() {
        let result = compile_and_get_global(
            r#"
            let val = [1, 2, 3, 4, 5]
                .filter(fn(n) n > 2)
                .map(fn(n) n * 10)
                .reduce(fn(a, b) a + b, 0);
            "#,
            "val",
        );
        // [3,4,5] → [30,40,50] → 120
        assert_eq!(result, Value::Int(120));
    }

    #[test]
    fn test_vm_const_declaration() {
        let result = compile_and_get_global("const PI = 3; let x = PI * 2;", "x");
        assert_eq!(result, Value::Int(6));
    }

    #[test]
    fn test_vm_compound_assignment() {
        // i = i + 1 inside loops compiles to incr + assign — exercise
        // both forms.
        let result = compile_and_get_global(
            r#"
            let n = 5;
            n = n + 10;
            n = n * 2;
            "#,
            "n",
        );
        assert_eq!(result, Value::Int(30));
    }

    #[test]
    fn test_vm_logical_operators_return_value_not_bool() {
        // In Soli, `||` and `&&` short-circuit and return one of the operands.
        let result = compile_and_get_global(r#"let x = null || "fallback";"#, "x");
        assert_eq!(result, Value::String("fallback".into()));

        let result = compile_and_get_global(r#"let x = "a" || "b";"#, "x");
        assert_eq!(result, Value::String("a".into()));
    }

    #[test]
    fn test_vm_complex_expression() {
        let result = compile_and_get_global("let x = (1 + 2) * 3 - 4 / 2 + 5 % 3;", "x");
        // (1+2)*3 = 9; 4/2 = 2; 5%3 = 2; 9 - 2 + 2 = 9
        assert_eq!(result, Value::Int(9));
    }

    #[test]
    fn test_vm_nested_function_calls() {
        let result = compile_and_get_global(
            r#"
            fn double(x: Int) -> Int { return x * 2; }
            fn add_one(x: Int) -> Int { return x + 1; }
            let val = double(add_one(double(3)));
            "#,
            "val",
        );
        // 3*2=6, 6+1=7, 7*2=14
        assert_eq!(result, Value::Int(14));
    }

    #[test]
    fn test_vm_array_first_last_with_method_chain() {
        assert_eq!(
            compile_and_get_global("let x = [1, 2, 3, 4, 5].filter(fn(n) n > 2).first();", "x"),
            Value::Int(3)
        );
    }

    /// Run `code` in the fast tier alone, with `locals` in slots 1.., and
    /// return the op it handed back (`None`: it ran every op itself) and
    /// what it left on the stack above the locals.
    fn run_fast_tier(
        constants: Vec<Constant>,
        locals: &[Value],
        code: &[Op],
    ) -> (Option<Op>, Vec<Value>) {
        run_fast_tier_with(constants, locals, code, 0, &[])
    }

    /// [`run_fast_tier`] in a frame whose caller supplied the parameters in
    /// the `supplied` mask, with `globals` defined.
    fn run_fast_tier_with(
        constants: Vec<Constant>,
        locals: &[Value],
        code: &[Op],
        supplied: u64,
        globals: &[(&str, Value)],
    ) -> (Option<Op>, Vec<Value>) {
        let (declined, mut vm) = run_fast_tier_vm(constants, locals, code, supplied, globals);
        let above_locals = vm.stack.split_off(1 + locals.len());
        (declined, above_locals)
    }

    /// [`run_fast_tier_with`], handing back the whole VM so a test can read
    /// the state the fast tier left beside the stack (handlers, frames).
    fn run_fast_tier_vm(
        constants: Vec<Constant>,
        locals: &[Value],
        code: &[Op],
        supplied: u64,
        globals: &[(&str, Value)],
    ) -> (Option<Op>, Vm) {
        let mut proto = FunctionProto::new(String::new());
        proto.chunk.constants = constants;
        for op in code {
            proto.chunk.emit(*op, 1);
        }
        let mut vm = Vm::new();
        for (name, value) in globals {
            vm.globals.insert(name.to_string(), value.clone());
        }
        let closure = Rc::new(VmClosure::new(Arc::new(proto), Vec::new()));
        vm.push(Value::VmClosure(closure.clone()));
        for local in locals {
            vm.push(local.clone());
        }
        vm.frames.push(CallFrame::new(
            closure,
            0,
            vm.iter_stack.len(),
            None,
            supplied,
        ));
        let declined = vm.run_fast().expect("the fast tier raised");
        (declined, vm)
    }

    /// A `try` or `expr rescue x` that does not throw stays in the fast
    /// tier: `TryBegin`, `TryEnd`, `PopHandler`, `RescueJump` and a
    /// `ForceFuture` on anything but a Future had no fast arm, so each cost a
    /// tier exit. The handler the fast arm pushes must be the one the general
    /// arm pushed, since the throw side reads nothing else.
    #[test]
    fn try_and_rescue_that_do_not_throw_stay_in_the_fast_tier() {
        const SEVEN: u16 = 0;
        const ZERO: u16 = 1;
        let constants = || vec![Constant::Int(7), Constant::Int(0)];
        let locals = [Value::Int(5)];

        // `TryBegin` records the targets relative to the next op, and the
        // stack (closure + one local), frame and iterator depths.
        let (declined, vm) = run_fast_tier_vm(constants(), &locals, &[Op::TryBegin(2, 3)], 0, &[]);
        assert_eq!(declined, None, "TryBegin left the fast tier");
        assert_eq!(vm.exception_handlers.len(), 1);
        let handler = &vm.exception_handlers[0];
        assert_eq!(
            (
                handler.catch_ip,
                handler.finally_ip,
                handler.stack_depth,
                handler.frame_depth,
                handler.iter_depth,
            ),
            (3, 4, 2, 1, 0)
        );

        // The non-throwing paths, each leaving no handler behind.
        let cases: Vec<(&[Op], Vec<Value>)> = vec![
            // try { 7 } … : the handler is pushed and popped.
            (
                &[Op::TryBegin(0, 0), Op::Constant(SEVEN), Op::TryEnd],
                vec![Value::Int(7)],
            ),
            // The outer handler of a try/finally, dropped on the normal path.
            (&[Op::TryBegin(0, 0), Op::PopHandler], vec![]),
            (&[Op::Constant(SEVEN), Op::ForceFuture], vec![Value::Int(7)]),
            (
                &[Op::RescueJump(1), Op::Constant(SEVEN), Op::Null],
                vec![Value::Null],
            ),
            // `7 rescue 0`, as the compiler lays it out: the Jump skips the
            // catch target's Pop and the fallback.
            (
                &[
                    Op::TryBegin(4, 0),
                    Op::Constant(SEVEN),
                    Op::ForceFuture,
                    Op::TryEnd,
                    Op::Jump(2),
                    Op::Pop,
                    Op::Constant(ZERO),
                ],
                vec![Value::Int(7)],
            ),
        ];
        for (code, expected) in cases {
            let (declined, mut vm) = run_fast_tier_vm(constants(), &locals, code, 0, &[]);
            assert_eq!(declined, None, "{code:?} left the fast tier");
            assert!(vm.exception_handlers.is_empty(), "{code:?} left a handler");
            assert_eq!(vm.stack.split_off(2), expected, "{code:?}");
        }

        // A Future still has to be settled, which the general arm does: the
        // op is handed back with the Future untouched under the handler.
        #[allow(clippy::arc_with_non_send_sync)]
        let future = Value::Future(Arc::new(std::sync::Mutex::new(
            crate::interpreter::value::FutureState::Resolved(Value::Int(7)),
        )));
        crate::interpreter::value::note_future_created();
        let (declined, vm) = run_fast_tier_vm(
            constants(),
            &[future],
            &[Op::TryBegin(0, 0), Op::GetLocal(1), Op::ForceFuture],
            0,
            &[],
        );
        assert_eq!(declined, Some(Op::ForceFuture));
        assert_eq!(vm.exception_handlers.len(), 1);
        assert!(vm.stack.last().unwrap().is_future());
    }

    /// `def f(a, b = 7) b`, built by hand: the prologue runs `b`'s default
    /// only when the caller left `b` out.
    fn defaulted_callee() -> Value {
        let mut proto = FunctionProto::new("f".to_string());
        proto.param_names = vec!["a".to_string(), "b".to_string()];
        proto.arity = 1;
        proto.defaults = 1;
        proto.defaults_mask = 0b10;
        proto.chunk.constants = vec![Constant::Int(7)];
        for op in [
            Op::JumpIfParamSupplied(1, 2),
            Op::Constant(0),
            Op::SetLocalPop(2),
            Op::GetLocal(2),
            Op::Return,
        ] {
            proto.chunk.emit(op, 1);
        }
        Value::VmClosure(Rc::new(VmClosure::new(Arc::new(proto), Vec::new())))
    }

    /// A parameter-default prologue and a call that leaves defaulted
    /// parameters out stay in the fast tier. `JumpIfParamSupplied` had no
    /// fast arm, so every defaulted parameter cost a tier exit per call, even
    /// a supplied one; and the fast `Call`/`CallGlobal` took only calls that
    /// supplied every parameter.
    #[test]
    fn defaulted_parameters_and_short_calls_stay_in_the_fast_tier() {
        const FIVE: u16 = 0;
        const F: u16 = 1;
        let constants = || vec![Constant::Int(5), Constant::String("f".into())];
        let five = || Value::Int(5);
        let seven = || Value::Int(7);

        // The prologue alone: a supplied parameter skips its default, an
        // omitted one runs it, and past bit 63 the mask reads as supplied.
        let prologue: Vec<(Op, u64, Vec<Value>)> = vec![
            (Op::JumpIfParamSupplied(0, 1), 0b1, vec![]),
            (Op::JumpIfParamSupplied(0, 1), 0b10, vec![five()]),
            (Op::JumpIfParamSupplied(1, 1), 0b10, vec![]),
            (Op::JumpIfParamSupplied(63, 1), 1 << 63, vec![]),
            (Op::JumpIfParamSupplied(64, 1), 0, vec![]),
        ];
        for (jump, supplied, expected) in prologue {
            let code = [jump, Op::Constant(FIVE)];
            let (declined, stack) = run_fast_tier_with(constants(), &[], &code, supplied, &[]);
            assert_eq!(declined, None, "{code:?} left the fast tier");
            assert_eq!(stack, expected, "{code:?} with {supplied:#b}");
        }

        // A call from the fast tier into `f`, whose prologue and return run
        // there too: `b` defaults to 7 when left out.
        let callee = [defaulted_callee()];
        let globals = [("f", defaulted_callee())];
        let calls: Vec<(&[Op], Vec<Value>)> = vec![
            (
                &[Op::GetLocal(1), Op::Constant(FIVE), Op::Call(1)],
                vec![seven()],
            ),
            (
                &[
                    Op::GetLocal(1),
                    Op::Constant(FIVE),
                    Op::Constant(FIVE),
                    Op::Call(2),
                ],
                vec![five()],
            ),
            (&[Op::Constant(FIVE), Op::CallGlobal(F, 1)], vec![seven()]),
            (
                &[Op::Constant(FIVE), Op::Constant(FIVE), Op::CallGlobal(F, 2)],
                vec![five()],
            ),
        ];
        for (code, expected) in calls {
            let (declined, stack) = run_fast_tier_with(constants(), &callee, code, 0, &globals);
            assert_eq!(declined, None, "{code:?} left the fast tier");
            assert_eq!(stack, expected, "{code:?}");
        }

        // Too few or too many arguments: the general arm raises the error.
        let declined: Vec<&[Op]> = vec![
            &[Op::GetLocal(1), Op::Call(0)],
            &[
                Op::GetLocal(1),
                Op::Constant(FIVE),
                Op::Constant(FIVE),
                Op::Constant(FIVE),
                Op::Call(3),
            ],
            &[Op::CallGlobal(F, 0)],
        ];
        for code in declined {
            let (handed_back, _) = run_fast_tier_with(constants(), &callee, code, 0, &globals);
            assert_eq!(handed_back, code.last().copied(), "{code:?}");
        }
    }

    /// A call to a function with a native kernel stays in the fast tier. The
    /// fast `Call`/`CallGlobal` required `kernel.is_none()`, so every such
    /// call left the tier for the general arm to try the kernel, and a
    /// kernel that declined (an argument of another type, a rebound name)
    /// cost that tier exit on every call before its bytecode ran.
    ///
    /// The callee carries the kernel of `addf(a: Float, b: Float) -> Float`
    /// but a body of one op the fast tier hands back (`Print`): a call that
    /// finishes in the tier was answered by the kernel, and one that hands
    /// `Print` back from a second frame was declined and ran the bytecode.
    #[cfg(feature = "native")]
    #[test]
    fn calls_to_a_native_kernel_stay_in_the_fast_tier() {
        let source = "def addf(a: Float, b: Float) -> Float\n  a + b\nend\n";
        let program = Parser::new(Scanner::new(source).scan_tokens().unwrap())
            .parse()
            .unwrap();
        let set =
            crate::native::analyze_and_compile(&program, &|_| false).expect("kernels compile");
        let crate::ast::StmtKind::Function(decl) = &program.statements[0].kind else {
            unreachable!()
        };
        let mut proto = FunctionProto::new("addf".to_string());
        proto.param_names = vec!["a".to_string(), "b".to_string()];
        proto.arity = 2;
        proto.kernel = Some(set.for_decl(decl).expect("addf is a kernel"));
        proto.chunk.emit(Op::Print(0), 1);
        let addf = Value::VmClosure(Rc::new(VmClosure::new(Arc::new(proto), Vec::new())));

        const A: u16 = 0;
        const B: u16 = 1;
        const INT: u16 = 2;
        const ADDF: u16 = 3;
        let constants = || {
            vec![
                Constant::Float(1.5),
                Constant::Float(2.0),
                Constant::Int(2),
                Constant::String("addf".into()),
            ]
        };
        let callee = [addf.clone()];
        let bound = [("addf", addf.clone())];
        let by_value: &[Op] = &[
            Op::GetLocal(1),
            Op::Constant(A),
            Op::Constant(B),
            Op::Call(2),
        ];
        let by_name: &[Op] = &[Op::Constant(A), Op::Constant(B), Op::CallGlobal(ADDF, 2)];

        // The kernel answers: no frame, the value where the callee was.
        for code in [by_value, by_name] {
            let (declined, mut vm) = run_fast_tier_vm(constants(), &callee, code, 0, &bound);
            assert_eq!(declined, None, "{code:?} left the fast tier");
            assert_eq!(vm.frames.len(), 1, "{code:?} opened a frame");
            assert_eq!(vm.stack.split_off(2), vec![Value::Float(3.5)], "{code:?}");
        }

        // An Int argument: declined, and the bytecode runs in the tier.
        let int_arg: Vec<&[Op]> = vec![
            &[
                Op::GetLocal(1),
                Op::Constant(A),
                Op::Constant(INT),
                Op::Call(2),
            ],
            &[Op::Constant(A), Op::Constant(INT), Op::CallGlobal(ADDF, 2)],
        ];
        for code in int_arg {
            let (declined, vm) = run_fast_tier_vm(constants(), &callee, code, 0, &bound);
            assert_eq!(declined, Some(Op::Print(0)), "{code:?}");
            assert_eq!(vm.frames.len(), 2, "{code:?} did not enter addf");
        }

        // `addf` no longer bound to its kernel: declined (Rebound) likewise.
        let (declined, vm) = run_fast_tier_vm(constants(), &callee, by_value, 0, &[]);
        assert_eq!(declined, Some(Op::Print(0)));
        assert_eq!(vm.frames.len(), 2);
    }

    /// Named arguments to a compiled function bind in place on the stack, by
    /// the tree-walker's rules: the positional value wins over a label for
    /// the same parameter, an unknown label is an undefined variable, a
    /// missing required parameter is an arity error, and a call with more
    /// labels than fit inline binds the same way. A refused call leaves a
    /// stack the next statement can use.
    #[test]
    fn named_arguments_bind_in_place_by_the_tree_walkers_rules() {
        let f = "def f(a, b = 2, c = 3) { return [a, b, c] }\n";
        let cases = [
            ("f(1, a: 9)", "[1, 2, 3]"),
            ("f(c: 30, a: 10)", "[10, 2, 30]"),
            ("f(1, c: nil)", "[1, 2, null]"),
            ("f(b: 20, c: 30, a: 10)", "[10, 20, 30]"),
            ("f(1, 2, c: 3)", "[1, 2, 3]"),
        ];
        for (call, expected) in cases {
            let source = format!("{f}let x = {call}.to_s();");
            assert_eq!(
                compile_and_get_global(&source, "x"),
                Value::String(expected.into()),
                "{call}"
            );
        }

        let errors = [
            ("f(1, zz: 1)", "Undefined variable 'zz'"),
            ("f(b: 1)", "expected 1, got 0"),
            ("f(a: 1, a: 2)", "duplicate named argument 'a'"),
            ("f(1, 2, 3, 4, c: 1)", "expected 3, got 5"),
        ];
        for (call, message) in errors {
            let err = compile_and_run(&format!("{f}{call}")).expect_err(call);
            assert!(err.to_string().contains(message), "{call}: {err}");
        }

        // Caught, the refusal leaves a stack the following calls run on.
        assert_eq!(
            compile_and_get_global(
                &format!(
                    "{f}let e = nil\ntry {{ f(1, zz: 2) }} catch err {{ e = err }}\nlet x = f(c: 5, a: 4).to_s();"
                ),
                "x"
            ),
            Value::String("[4, 2, 5]".into())
        );

        // Twenty labels, past the inline plan, in reverse order.
        let params: Vec<String> = (0..20).map(|i| format!("p{i} = 0")).collect();
        let labels: Vec<String> = (0..20).rev().map(|i| format!("p{i}: {i}")).collect();
        let source = format!(
            "def g({}) {{ return p0 + p7 * 100 + p19 * 10000 }}\nlet x = g({});",
            params.join(", "),
            labels.join(", ")
        );
        assert_eq!(compile_and_get_global(&source, "x"), Value::Int(190_700));

        // A constructor binds the same way, with the instance as `this`.
        assert_eq!(
            compile_and_get_global(
                "class P { x: Int; y: Int; new(x, y = 5) { this.x = x; this.y = y } }\nlet x = P(y: 1, x: 2).x * 10 + P(x: 3).y;",
                "x"
            ),
            Value::Int(25)
        );
    }

    /// String literals, symbols and `==`/`!=` between primitives stay in the
    /// fast tier, each comparison shape included: `Equal`/`NotEqual`, the
    /// `*LocalConst` and `*LocalLocal` fused ops and both test-and-jump ops.
    /// Each used to hand a string, a nil or a float back to `run_dispatch`.
    #[test]
    fn string_literals_and_primitive_equality_stay_in_the_fast_tier() {
        const ADMIN: u16 = 0;
        const USER: u16 = 1;
        const NINE: u16 = 2;
        const HALF: u16 = 3;
        const YES: u16 = 4;
        const NIL: u16 = 5;
        const SYM: u16 = 6;
        let constants = || {
            vec![
                Constant::String("admin".into()),
                Constant::String("user".into()),
                Constant::Int(9),
                Constant::Float(1.5),
                Constant::Bool(true),
                Constant::Null,
                Constant::String("sym".into()),
            ]
        };
        let string = |s: &str| Value::String(s.into());
        let locals = [string("admin"), string("user"), Value::Null];
        let t = Value::Bool(true);
        let f = Value::Bool(false);
        let cases: Vec<(&[Op], Vec<Value>)> = vec![
            (&[Op::Constant(ADMIN)], vec![string("admin")]),
            (&[Op::Symbol(SYM)], vec![Value::Symbol("sym".into())]),
            (
                &[Op::Constant(ADMIN), Op::Constant(ADMIN), Op::Equal],
                vec![t.clone()],
            ),
            (
                &[Op::Constant(ADMIN), Op::Constant(USER), Op::NotEqual],
                vec![t.clone()],
            ),
            (&[Op::Null, Op::Constant(ADMIN), Op::Equal], vec![f.clone()]),
            (&[Op::Null, Op::Null, Op::Equal], vec![t.clone()]),
            (
                &[Op::Null, Op::Constant(NINE), Op::NotEqual],
                vec![t.clone()],
            ),
            (
                &[Op::Constant(HALF), Op::Constant(HALF), Op::Equal],
                vec![t.clone()],
            ),
            (
                &[Op::Constant(NINE), Op::Constant(ADMIN), Op::Equal],
                vec![f.clone()],
            ),
            (
                &[Op::Symbol(SYM), Op::Constant(SYM), Op::Equal],
                vec![f.clone()],
            ),
            (
                &[Op::Symbol(SYM), Op::Symbol(SYM), Op::Equal],
                vec![t.clone()],
            ),
            (&[Op::EqualLocalConst(1, ADMIN)], vec![t.clone()]),
            (&[Op::NotEqualLocalConst(2, ADMIN)], vec![t.clone()]),
            (&[Op::EqualLocalConst(1, NIL)], vec![f.clone()]),
            (&[Op::EqualLocalConst(3, NIL)], vec![t.clone()]),
            (&[Op::EqualLocalConst(3, YES)], vec![f.clone()]),
            (&[Op::EqualLocalLocal(1, 2)], vec![f.clone()]),
            (&[Op::NotEqualLocalLocal(1, 2)], vec![t.clone()]),
            (&[Op::EqualLocalLocal(1, 1)], vec![t.clone()]),
            // `if a == b { true }`: equal falls through to the `True`,
            // different jumps over it.
            (
                &[
                    Op::Constant(ADMIN),
                    Op::Constant(ADMIN),
                    Op::TestEqualJump(1),
                    Op::True,
                ],
                vec![t.clone()],
            ),
            (
                &[
                    Op::Constant(ADMIN),
                    Op::Constant(USER),
                    Op::TestEqualJump(1),
                    Op::True,
                ],
                vec![],
            ),
            (
                &[
                    Op::Constant(ADMIN),
                    Op::Constant(ADMIN),
                    Op::TestNotEqualJump(1),
                    Op::True,
                ],
                vec![],
            ),
            (
                &[
                    Op::Null,
                    Op::Constant(USER),
                    Op::TestNotEqualJump(1),
                    Op::True,
                ],
                vec![t.clone()],
            ),
        ];
        for (code, expected) in cases {
            let (declined, stack) = run_fast_tier(constants(), &locals, code);
            assert_eq!(declined, None, "{code:?} left the fast tier");
            assert_eq!(stack, expected, "{code:?}");
        }

        // What still needs the general arm declines, untouched: an Int and a
        // Float compare as numbers there, an instance may be an enum, a
        // Decimal constant is parsed.
        let status = bare_class("Status");
        let enum_locals = [
            stored_variant(&status, "Active"),
            stored_variant(&status, "Active"),
        ];
        let declined: Vec<(&[Op], &[Value], Op)> = vec![
            (
                &[Op::Constant(NINE), Op::Constant(HALF), Op::Equal],
                &locals,
                Op::Equal,
            ),
            (
                &[Op::EqualLocalLocal(1, 2)],
                &enum_locals,
                Op::EqualLocalLocal(1, 2),
            ),
            (
                &[Op::GetLocal(1), Op::GetLocal(2), Op::TestEqualJump(0)],
                &enum_locals,
                Op::TestEqualJump(0),
            ),
            (
                &[Op::EqualLocalConst(1, ADMIN)],
                &enum_locals,
                Op::EqualLocalConst(1, ADMIN),
            ),
        ];
        for (code, locals, op) in declined {
            let (handed_back, _) = run_fast_tier(constants(), locals, code);
            assert_eq!(handed_back, Some(op), "{code:?}");
        }
    }

    /// `k -= 1`, `/` and `%` on Ints (`/` on Floats), `??`, `h["k"]` on a
    /// local hash and `xs[i]` in range stay in the fast tier, each in its
    /// stack and fused forms; every one used to cost a round trip through
    /// `run_dispatch`. What the general arms raise on or convert declines,
    /// untouched, so the error text stays theirs.
    #[test]
    fn decrement_division_nullish_and_index_reads_stay_in_the_fast_tier() {
        const SEVEN: u16 = 0;
        const ZERO: u16 = 1;
        const HALF: u16 = 2;
        const KEY_A: u16 = 3;
        const KEY_B: u16 = 4;
        const ONE: u16 = 5;
        const BIG: u16 = 6;
        let constants = || {
            vec![
                Constant::Int(7),
                Constant::Int(0),
                Constant::Float(0.5),
                Constant::String("a".into()),
                Constant::String("b".into()),
                Constant::Int(1),
                Constant::Int(9),
            ]
        };
        let mut pairs = HashPairs::default();
        pairs.insert(HashKey::String("a".into()), Value::Int(1));
        let hash = Value::Hash(Rc::new(RefCell::new(pairs)));
        let array = Value::Array(Rc::new(RefCell::new(vec![
            Value::Int(10),
            Value::Int(20),
            Value::Int(30),
        ])));
        // Slots: 1 k=23, 2 c=-4, 3 hash, 4 array, 5 nil, 6 i=-1, 7 x=3.0.
        let locals = [
            Value::Int(23),
            Value::Int(-4),
            hash,
            array,
            Value::Null,
            Value::Int(-1),
            Value::Float(3.0),
        ];
        let int = Value::Int;
        let cases: Vec<(&[Op], Vec<Value>)> = vec![
            (&[Op::DecrLocal(1), Op::GetLocal(1)], vec![int(22)]),
            (
                &[Op::GetLocal(1), Op::Constant(SEVEN), Op::Divide],
                vec![int(3)],
            ),
            (
                &[Op::GetLocal(1), Op::Constant(SEVEN), Op::Modulo],
                vec![int(2)],
            ),
            (
                &[Op::GetLocal(2), Op::Constant(SEVEN), Op::Modulo],
                vec![int(-4)],
            ),
            (
                &[Op::GetLocal(7), Op::GetLocal(7), Op::Divide],
                vec![Value::Float(1.0)],
            ),
            (&[Op::DivLocalLocal(1, 2)], vec![int(-5)]),
            (&[Op::ModLocalLocal(1, 2)], vec![int(3)]),
            (&[Op::DivLocalConst(1, SEVEN)], vec![int(3)]),
            (&[Op::ModLocalConst(1, SEVEN)], vec![int(2)]),
            (&[Op::DivLocalConst(7, HALF)], vec![Value::Float(6.0)]),
            // `nil ?? 7` evaluates the default; `23 ?? 7` jumps over it.
            (
                &[
                    Op::GetLocal(5),
                    Op::NullishJump(2),
                    Op::Pop,
                    Op::Constant(SEVEN),
                ],
                vec![int(7)],
            ),
            (
                &[
                    Op::GetLocal(1),
                    Op::NullishJump(2),
                    Op::Pop,
                    Op::Constant(SEVEN),
                ],
                vec![int(23)],
            ),
            (&[Op::HashGetLocalConst(3, KEY_A)], vec![int(1)]),
            (&[Op::HashGetLocalConst(3, KEY_B)], vec![Value::Null]),
            (
                &[Op::GetLocal(4), Op::Constant(ONE), Op::GetIndex],
                vec![int(20)],
            ),
            (
                &[Op::GetLocal(4), Op::GetLocal(6), Op::GetIndex],
                vec![int(30)],
            ),
            (
                &[Op::GetLocal(3), Op::Constant(KEY_B), Op::GetIndex],
                vec![Value::Null],
            ),
            (&[Op::GetLocalIndex(4, 6)], vec![int(30)]),
        ];
        for (code, expected) in cases {
            let (declined, stack) = run_fast_tier(constants(), &locals, code);
            assert_eq!(declined, None, "{code:?} left the fast tier");
            assert_eq!(stack, expected, "{code:?}");
        }

        let min = [Value::Int(i64::MIN), Value::Int(-1), Value::Float(2.0)];
        let declined: Vec<(&[Op], &[Value], Op)> = vec![
            (&[Op::DecrLocal(1)], &min, Op::DecrLocal(1)),
            (&[Op::DecrLocal(3)], &min, Op::DecrLocal(3)),
            (&[Op::DivLocalLocal(1, 2)], &min, Op::DivLocalLocal(1, 2)),
            (&[Op::ModLocalLocal(1, 2)], &min, Op::ModLocalLocal(1, 2)),
            (
                &[Op::DivLocalConst(1, ZERO)],
                &locals,
                Op::DivLocalConst(1, ZERO),
            ),
            (
                &[Op::ModLocalConst(1, ZERO)],
                &locals,
                Op::ModLocalConst(1, ZERO),
            ),
            // A Float `%` and an Int/Float mix are converted by the general arm.
            (
                &[Op::ModLocalConst(7, HALF)],
                &locals,
                Op::ModLocalConst(7, HALF),
            ),
            (
                &[Op::DivLocalConst(1, HALF)],
                &locals,
                Op::DivLocalConst(1, HALF),
            ),
            (
                &[Op::GetLocal(1), Op::Constant(ZERO), Op::Divide],
                &locals,
                Op::Divide,
            ),
            (
                &[Op::GetLocal(3), Op::GetLocal(3), Op::Modulo],
                &min,
                Op::Modulo,
            ),
            // An array or a string receiver of `x["k"]` has its own rules.
            (
                &[Op::HashGetLocalConst(4, KEY_A)],
                &locals,
                Op::HashGetLocalConst(4, KEY_A),
            ),
            (
                &[Op::GetLocal(4), Op::Constant(BIG), Op::GetIndex],
                &locals,
                Op::GetIndex,
            ),
            (&[Op::GetLocalIndex(4, 1)], &locals, Op::GetLocalIndex(4, 1)),
        ];
        for (code, locals, op) in declined {
            let (handed_back, _) = run_fast_tier(constants(), locals, code);
            assert_eq!(handed_back, Some(op), "{code:?}");
        }
    }

    /// `k += c`, `k -= c` and `k *= c` with an Int or Float constant run in
    /// the fast tier and store into the slot, pushing nothing. An overflow, a
    /// mix of Int and Float, and any other type are handed to the general
    /// arm, which raises or converts.
    #[test]
    fn local_const_in_place_ops_run_in_the_fast_tier() {
        const SEVEN: u16 = 0;
        const HALF: u16 = 1;
        let constants = || vec![Constant::Int(7), Constant::Float(0.5)];
        // Slots: 1 k=23, 2 x=3.0, 3 s="ab", 4 the smallest Int.
        let locals = [
            Value::Int(23),
            Value::Float(3.0),
            Value::String("ab".into()),
            Value::Int(i64::MIN),
        ];
        let cases: Vec<(Op, u16, Value)> = vec![
            (Op::AddLocalConstInPlace(1, SEVEN), 1, Value::Int(30)),
            (Op::SubLocalConstInPlace(1, SEVEN), 1, Value::Int(16)),
            (Op::MulLocalConstInPlace(1, SEVEN), 1, Value::Int(161)),
            (Op::AddLocalConstInPlace(2, HALF), 2, Value::Float(3.5)),
            (Op::SubLocalConstInPlace(2, HALF), 2, Value::Float(2.5)),
            (Op::MulLocalConstInPlace(2, HALF), 2, Value::Float(1.5)),
        ];
        for (op, slot, expected) in cases {
            let (declined, stack) = run_fast_tier(constants(), &locals, &[op, Op::GetLocal(slot)]);
            assert_eq!(declined, None, "{op:?} left the fast tier");
            assert_eq!(stack, vec![expected], "{op:?}");
        }

        for op in [
            // Overflow.
            Op::SubLocalConstInPlace(4, SEVEN),
            Op::MulLocalConstInPlace(4, SEVEN),
            // Int with Float, Float with Int.
            Op::AddLocalConstInPlace(1, HALF),
            Op::MulLocalConstInPlace(2, SEVEN),
            // A string.
            Op::MulLocalConstInPlace(3, SEVEN),
        ] {
            let (handed_back, stack) = run_fast_tier(constants(), &locals, &[op]);
            assert_eq!(handed_back, Some(op));
            assert!(stack.is_empty(), "{op:?} pushed {stack:?}");
        }
    }

    /// The fast tier reads and pops the operand stack's top unchecked
    /// (`pop`, `pop2`, `peek`, `peek_mut`, `peek_two!`). Each op that does so
    /// still runs there, leaves what the general arm would, and releases a
    /// heap operand exactly once: the array's reference count after each
    /// sequence is the slot's own plus one per copy left on the stack.
    #[test]
    fn unchecked_operand_reads_keep_values_and_reference_counts() {
        const NAME: u16 = 0;
        let constants = || vec![Constant::String("a name longer than inline".into())];
        let items = Rc::new(RefCell::new(vec![Value::Int(10), Value::Int(20)]));
        let array = Value::Array(items.clone());
        // Slots: 1 the array, 2 i=1, 3 nil.
        let locals = [array.clone(), Value::Int(1), Value::Null];
        let cases: Vec<(&[Op], Vec<Value>, usize)> = vec![
            // Truthy: no jump, `True` runs.
            (
                &[Op::GetLocal(1), Op::JumpIfFalse(1), Op::True],
                vec![Value::Bool(true)],
                0,
            ),
            (&[Op::GetLocal(1), Op::JumpIfTrue(1), Op::True], vec![], 0),
            (&[Op::GetLocal(1), Op::Not], vec![Value::Bool(false)], 0),
            (
                &[Op::GetLocal(1), Op::Dup],
                vec![array.clone(), array.clone()],
                2,
            ),
            (
                &[Op::GetLocal(1), Op::JumpIfFalseNoPop(1), Op::True],
                vec![array.clone(), Value::Bool(true)],
                1,
            ),
            (
                &[Op::GetLocal(1), Op::JumpIfTrueNoPop(1), Op::True],
                vec![array.clone()],
                1,
            ),
            (
                &[Op::GetLocal(1), Op::IsNull],
                vec![array.clone(), Value::Bool(false)],
                1,
            ),
            (
                &[Op::GetLocal(1), Op::JumpIfNotNull(1), Op::True],
                vec![array.clone()],
                1,
            ),
            (
                &[Op::GetLocal(3), Op::JumpIfNull(1), Op::True],
                vec![Value::Null],
                0,
            ),
            // `SetLocal` copies the top into slot 3 and leaves it there.
            (
                &[Op::GetLocal(1), Op::SetLocal(3), Op::Pop, Op::GetLocal(3)],
                vec![array.clone()],
                2,
            ),
            (
                &[Op::GetLocal(1), Op::GetLocal(2), Op::GetIndex],
                vec![Value::Int(20)],
                0,
            ),
            // Two strings: `primitive_equal` answers, `pop2` drops both.
            (
                &[Op::Constant(NAME), Op::Constant(NAME), Op::Equal],
                vec![Value::Bool(true)],
                0,
            ),
            (
                &[Op::Constant(NAME), Op::Constant(NAME), Op::NotEqual],
                vec![Value::Bool(false)],
                0,
            ),
        ];
        for (code, expected, extra) in cases {
            let before = Rc::strong_count(&items);
            let (declined, mut vm) = run_fast_tier_vm(constants(), &locals, code, 0, &[]);
            assert_eq!(declined, None, "{code:?} left the fast tier");
            // The VM's copy in slot 1, plus `extra` left on the stack or in
            // slot 3.
            assert_eq!(
                Rc::strong_count(&items),
                before + 1 + extra,
                "{code:?}: reference count"
            );
            let stack = vm.stack.split_off(1 + locals.len());
            assert_eq!(stack, expected, "{code:?}");
            drop(stack);
            drop(vm);
            assert_eq!(Rc::strong_count(&items), before, "{code:?}: released");
        }
    }

    /// The fast tier's inline equality is `values_equal`, restricted: on
    /// every pair it answers, it answers the same, and it declines the pairs
    /// whose rules live in the general arm (an Int and a Float, a class and
    /// its name, an enum, a Decimal).
    #[test]
    fn primitive_equal_agrees_with_values_equal() {
        let status = bare_class("Status");
        let values = [
            Value::Int(1),
            Value::Int(2),
            Value::Float(1.0),
            Value::Float(f64::NAN),
            Value::Bool(true),
            Value::Bool(false),
            Value::Null,
            Value::String("a".into()),
            Value::String("1".into()),
            Value::String("Status".into()),
            Value::Symbol("a".into()),
            Value::Class(status.clone()),
            stored_variant(&status, "Active"),
        ];
        for a in &values {
            for b in &values {
                let answered = primitive_equal(a, b);
                if let Some(equal) = answered {
                    assert_eq!(equal, values_equal(a, b), "{a:?} == {b:?}");
                }
                let numbers = matches!(
                    (a, b),
                    (Value::Int(_), Value::Float(_)) | (Value::Float(_), Value::Int(_))
                );
                let inline = is_primitive(a) && is_primitive(b) && !numbers;
                assert_eq!(answered.is_some(), inline, "{a:?} == {b:?}");
            }
        }
        let constants = [
            Constant::Int(1),
            Constant::Float(1.0),
            Constant::Float(f64::NAN),
            Constant::Bool(true),
            Constant::Null,
            Constant::String("a".into()),
            Constant::String("Status".into()),
            Constant::Decimal(
                crate::interpreter::value::DecimalValue::from_literal("1.0").unwrap(),
            ),
        ];
        for value in &values {
            for constant in &constants {
                let answered = primitive_equal_constant(value, constant);
                if let Some(equal) = answered {
                    assert_eq!(
                        equal,
                        values_equal(value, &constant_to_value(constant)),
                        "{value:?} == {constant:?}"
                    );
                }
                let numbers = matches!(
                    (value, constant),
                    (Value::Int(_), Constant::Float(_)) | (Value::Float(_), Constant::Int(_))
                );
                let inline =
                    is_primitive(value) && !matches!(constant, Constant::Decimal(_)) && !numbers;
                assert_eq!(answered.is_some(), inline, "{value:?} == {constant:?}");
            }
        }
    }

    /// A key named like a hash method is the key's value on every VM path, as
    /// on the tree-walker. The key fast paths (`GetProperty`, the fused
    /// `GetLocalProperty`) always answered it, but `op_get_property` — where a
    /// read lands once a Hash user method exists, and the read of `x.name ||=`
    /// (`GetPropertyOrNull`) — answered the builtin method: `row.size ||= 0`
    /// wrote the entry count over the field, `{"size": nil}.size ||= 7` stored
    /// 1. A call still means the method, and so does a read without the key.
    #[test]
    fn test_vm_hash_key_named_like_a_method_reads_the_key() {
        let row = "let row = {\"size\": 2048, \"key\": \"abc\", \"map\": null}\n";
        let global = |body: &str| compile_and_get_global(&format!("{row}{body}"), "x");
        assert_eq!(global("let x = row.size"), Value::Int(2048));
        assert_eq!(
            global("fn read(r) { return r.key }\nlet x = read(row)"),
            Value::String("abc".into())
        );
        // `||=` and `??=` read through GetPropertyOrNull and always write back.
        assert_eq!(
            global("row.size ||= 0\nlet x = row[\"size\"]"),
            Value::Int(2048)
        );
        assert_eq!(
            global("row.map ??= \"set\"\nlet x = row[\"map\"]"),
            Value::String("set".into())
        );
        assert_eq!(
            compile_and_get_global(
                "let counts = {\"size\": null}\ncounts.size ||= 7\nlet x = counts[\"size\"]",
                "x"
            ),
            Value::Int(7)
        );
        // Parentheses call the method; so does a bare read without the key.
        assert_eq!(global("let x = row.size()"), Value::Int(3));
        assert_eq!(
            global("let x = row.key(\"abc\")"),
            Value::String("key".into())
        );
        assert_eq!(
            compile_and_get_global("let x = {\"a\": 1, \"b\": 2}.size", "x"),
            Value::Int(2)
        );
        // The slow path itself, where a registered Hash user method sends
        // every read (registering one here would leak into other tests).
        let mut pairs = crate::interpreter::value::HashPairs::default();
        pairs.insert(
            crate::interpreter::value::HashKey::String("size".into()),
            Value::Int(2048),
        );
        let hash = Value::Hash(Rc::new(RefCell::new(pairs)));
        assert_eq!(
            Vm::new()
                .op_get_property(&hash, "size", Span::default())
                .unwrap(),
            Value::Int(2048)
        );
    }

    /// With metrics on, a VM times only its outermost `run`: the runs that
    /// paren-less method calls, a `reduce` block and a resumed `catch`
    /// re-enter are inside it, and timing them again cost a clock pair and
    /// two shared atomic adds each, and counted their time twice.
    #[test]
    fn metrics_time_only_the_outermost_run() {
        let source = r##"
class Widget
  def one
    1
  end

  def boom
    throw "nope"
  end
end

widget = Widget.new()
x = 0
i = 0
while i < 10
  x = x + widget.one
  i = i + 1
end
caught = ""
try
  widget.boom
catch error
  caught = "#{error}"
end
total = [1, 2, 3].reduce(fn(acc, v) acc + v + widget.one, 0)

def explode
  widget.boom
end
"##;
        let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
        let program = Parser::new(tokens).parse().expect("parser error");
        let module = Compiler::compile(&program).expect("compile error");
        let metrics = Metrics::global();

        let mut vm = Vm::new();
        vm.time_runs = true;
        let before = metrics.vm_execution.count();
        vm.execute(&module.main).expect("script runs");
        let after = metrics.vm_execution.count();
        assert_eq!(after - before, 1, "one outermost run, one record");
        assert!(!vm.timed_run_open);
        assert!(matches!(vm.globals.get("x"), Some(Value::Int(10))));
        assert!(matches!(vm.globals.get("total"), Some(Value::Int(9))));
        match vm.globals.get("caught") {
            Some(Value::String(s)) => assert_eq!(s.as_str(), "nope"),
            other => panic!("caught = {other:?}"),
        }

        // A call into the VM from outside is an outermost run of its own,
        // and an error escaping it still closes the timing.
        let explode = vm.globals.get("explode").cloned().expect("explode");
        let before = metrics.vm_execution.count();
        let err = vm.call_value_direct(explode, &[], Span::default());
        assert!(err.is_err(), "an uncaught throw escapes");
        let after = metrics.vm_execution.count();
        assert_eq!(after - before, 1);
        assert!(!vm.timed_run_open, "an escaping error closes the timing");

        // Metrics off: nothing is recorded.
        let mut quiet = Vm::new();
        quiet.time_runs = false;
        let before = metrics.vm_execution.count();
        quiet.execute(&module.main).expect("script runs");
        let after = metrics.vm_execution.count();
        assert_eq!(after, before);
    }
}

#[cfg(test)]
mod field_fast_tier_tests {
    //! Instance field reads and writes (`@x`, `this.x`, `post.title`,
    //! `@x = v`, `post.title = t`) complete in the fast tier whenever the
    //! general arm's field hit would complete without an error: the field is
    //! present (a write may also add it), the value is no `grouped {}`
    //! placeholder and the privacy check lets the access through. Everything
    //! else — a miss, a method, a deferred field, a refused or frozen access
    //! — still reaches the general arm, which answers as before.
    use super::general_field_ops;
    use crate::interpreter::value::{Class, DeferredCell, Instance, Value};
    use crate::vm::compiler::Compiler;
    use crate::vm::Vm;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Run `src` with `__N__` replaced by `n` and `globals` bound; return its
    /// `result` global and the instance field ops the general arms ran.
    fn run(src: &str, n: usize, globals: Vec<(&str, Value)>) -> (Value, usize) {
        let src = src.replace("__N__", &n.to_string());
        let tokens = crate::lexer::Scanner::new(&src).scan_tokens().expect("lex");
        let program = crate::parser::Parser::new(tokens).parse().expect("parse");
        let module = Compiler::compile(&program).expect("compile");
        let mut vm = Vm::new();
        for (name, value) in globals {
            vm.globals.insert(name.to_string(), value);
        }
        let start = general_field_ops();
        vm.execute(&module.main).expect("vm");
        let general = general_field_ops() - start;
        (vm.globals.get("result").cloned().expect("result"), general)
    }

    fn ints(values: &[i64]) -> Value {
        Value::Array(Rc::new(RefCell::new(
            values.iter().map(|n| Value::Int(*n)).collect(),
        )))
    }

    const POINT: &str = r#"
class Point
  x: Int
  y: Int

  new(x: Int, y: Int)
    @x = x
    @y = y
  end

  def walk(n)
    i = 0
    while i < n
      @x = @x + 1
      this.y = this.y + @x
      i = i + 1
    end
    @x * 1000 + @y
  end
end

def make
  new Point(7, 8)
end

def poke(point, n)
  i = 0
  while i < n
    point.x = point.x + point.y
    [1].each { |k| point.y = point.y + k }
    i = i + 1
  end
  point.x
end

point = new Point(1, 2)
walked = point.walk(__N__)
poked = poke(point, __N__)
point.extra = 5
point.extra = point.extra + make().x
result = [walked, poked, point.x, point.y, point.extra]
"#;

    /// Every field read and write of `POINT` — in a constructor, on `self`
    /// in a method (`@x`, `this.y`), on a local, a block's upvalue, a global
    /// and a call's result, the first write of a field no class declares —
    /// completes in the fast tier: the general arms run none of them, at
    /// any loop length. Before, each one left the tier.
    #[test]
    fn field_reads_and_writes_stay_in_the_fast_tier() {
        let (short, short_general) = run(POINT, 3, Vec::new());
        let (long, long_general) = run(POINT, 30, Vec::new());
        // walk(3): x 1 -> 4, y 2 + 2 + 3 + 4 = 11; poke(3) then adds y into
        // x and bumps y each round: x 15, 27, 40 and y 12, 13, 14.
        assert_eq!(short, ints(&[4011, 40, 40, 14, 12]));
        let Value::Array(long) = long else {
            panic!("expected an array, got {long:?}");
        };
        assert_eq!(long.borrow()[2], long.borrow()[1]);
        assert_eq!(short_general, 0);
        assert_eq!(long_general, 0);
    }

    const VAULT: &str = r#"
class Vault
  private code: Int
  protected rank: Int
  label: String

  new(code: Int)
    @code = code
    @rank = 1
    @label = "vault"
  end

  def turn(n)
    i = 0
    while i < n
      @code = @code + 1
      [1].each { |k| @code = @code + k }
      i = i + 1
    end
    @code
  end
end

def relabel(vault, n)
  i = 0
  while i < n
    vault.label = vault.label + "!"
    i = i + 1
  end
  vault.label
end

def peek(vault)
  vault.code rescue "refused"
end

def forge(vault)
  try
    vault.code = 7
    return "written"
  catch error
    return "refused"
  end
end

def rank_of(vault)
  vault.rank rescue "refused"
end

vault = new Vault(0)
turned = vault.turn(__N__)
label = relabel(vault, __N__)
result = [turned, label, peek(vault), forge(vault), rank_of(vault), vault.turn(0)]
"#;

    /// A class with a private and a protected field: a field no class of
    /// the chain restricts, read and written from a plain function, and the
    /// private one read and written by the class's own method and the block
    /// it runs, pass the check and stay in the fast tier. Only the three
    /// refused accesses from outside — the private read and write and the
    /// protected read — reach the general arm, which raises them.
    #[test]
    fn a_private_member_keeps_the_other_accesses_in_the_fast_tier() {
        let (short, short_general) = run(VAULT, 3, Vec::new());
        let (long, long_general) = run(VAULT, 30, Vec::new());
        let expected = |turned: i64, bangs: usize| {
            Value::Array(Rc::new(RefCell::new(vec![
                Value::Int(turned),
                Value::String(format!("vault{}", "!".repeat(bangs)).into()),
                Value::String("refused".into()),
                Value::String("refused".into()),
                Value::String("refused".into()),
                Value::Int(turned),
            ])))
        };
        assert_eq!(short, expected(6, 3));
        assert_eq!(long, expected(60, 30));
        assert_eq!(short_general, 3);
        assert_eq!(long_general, 3);
    }

    fn deferred_posts() -> Value {
        Value::Deferred(Rc::new(RefCell::new(DeferredCell {
            resolved: Some(Value::Array(Rc::new(RefCell::new(vec![
                Value::String("a".into()),
                Value::String("b".into()),
            ])))),
        })))
    }

    fn record() -> Value {
        let class = Rc::new(Class {
            name: "Rec".to_string(),
            ..Default::default()
        });
        let mut instance = Instance::new(class);
        instance.fields.insert("posts".into(), deferred_posts());
        instance
            .fields
            .insert("title".into(), Value::String("t".into()));
        Value::Instance(Rc::new(RefCell::new(instance)))
    }

    const DECLINED: &str = r#"
class Box
  w: Int

  new(w: Int)
    @w = w
  end

  def area
    @w * @w
  end
end

def sizes(rec)
  [rec.posts.length, rec.title]
end

box = new Box(3)
result = [box.area, box.w, sizes(rec), rec.posts.length, rec.title]
"#;

    /// What the fast tier hands back still gets the general arm's answer: a
    /// name with no field (`box.area`, the method, auto-invoked) and a field
    /// holding a `grouped {}` placeholder, from a local (`GetLocalProperty`)
    /// and from a global (`GetProperty`), which the general arm forces. The
    /// field hits beside them (`box.w`, `rec.title`) do not leave the tier.
    #[test]
    fn a_miss_and_a_deferred_field_go_to_the_general_arm() {
        let (result, general) = run(DECLINED, 0, vec![("rec", record())]);
        let pair = Value::Array(Rc::new(RefCell::new(vec![
            Value::Int(2),
            Value::String("t".into()),
        ])));
        assert_eq!(
            result,
            Value::Array(Rc::new(RefCell::new(vec![
                Value::Int(9),
                Value::Int(3),
                pair,
                Value::Int(2),
                Value::String("t".into()),
            ])))
        );
        assert_eq!(general, 3);
    }

    const AMOUNT: &str = r#"
class Amount < ValueObject
  cents: Int
end

class Email < ValueObject
  address: String

  new(address: String)
    @address = address.trim
  end
end

amount = Amount.new(5)
frozen = "written"
try
  amount.cents = 6
catch error
  frozen = "refused"
end
email = Email.new(" a@b.c ")
result = [amount.cents, frozen, email.address]
"#;

    /// A value object is written by its own constructor in the fast tier;
    /// a write once it is built is the general arm's to refuse.
    #[test]
    fn a_frozen_value_object_write_goes_to_the_general_arm() {
        let value_object = Value::Class(Rc::new(Class {
            name: "ValueObject".to_string(),
            ..Default::default()
        }));
        let (result, general) = run(AMOUNT, 0, vec![("ValueObject", value_object)]);
        assert_eq!(
            result,
            Value::Array(Rc::new(RefCell::new(vec![
                Value::Int(5),
                Value::String("refused".into()),
                Value::String("a@b.c".into()),
            ])))
        );
        assert_eq!(general, 1);
    }

    const MEMO: &str = r#"
class Tally
  def bump(n)
    i = 0
    while i < n
      @total ||= 0
      @total = @total + 1
      @seen &&= @seen + 1
      @label ??= "t"
      i = i + 1
    end
    @total
  end
end

tally = new Tally()
tally.seen = 1
total = tally.bump(__N__)
tally.total ||= 99
result = [total, tally.total, tally.seen, tally.label]
"#;

    /// `@total ||= 0` (and `&&=`, `??=`) on a set field is one fast-tier
    /// read and a jump, with no write: only the first `||=` and `??=`,
    /// which find nothing and assign, reach the general arm — two ops
    /// whatever the loop length. Before, every read left the tier.
    #[test]
    fn a_short_circuit_field_assign_stays_in_the_fast_tier() {
        let (short, short_general) = run(MEMO, 3, Vec::new());
        let (long, long_general) = run(MEMO, 40, Vec::new());
        assert_eq!(
            short,
            Value::Array(Rc::new(RefCell::new(vec![
                Value::Int(3),
                Value::Int(3),
                Value::Int(4),
                Value::String("t".into()),
            ])))
        );
        let Value::Array(long) = long else {
            panic!("expected an array, got {long:?}");
        };
        assert_eq!(long.borrow()[0], Value::Int(40));
        assert_eq!(long.borrow()[2], Value::Int(41));
        assert_eq!(short_general, 2);
        assert_eq!(long_general, 2);
    }
}

#[cfg(test)]
mod bare_builtin_fast_tier_tests {
    //! A paren-less zero-argument builtin (`h.length`, `s.blank?`, `a.first`,
    //! `n.to_s`) is answered in the fast tier by method id, as its `()` form
    //! is by `CallMethodById`, instead of leaving the tier for the general
    //! arm's by-name path. The value is the one the general arm gives; the
    //! precedences it keeps (a hash key first, `h.len` a method) stay.
    use super::general_member_ops;
    use crate::interpreter::value::Value;
    use crate::vm::compiler::Compiler;
    use crate::vm::Vm;

    /// Run `src`; return its `result` global and the non-instance property
    /// ops the general arms ran.
    fn run(src: &str) -> (Value, usize) {
        let tokens = crate::lexer::Scanner::new(src).scan_tokens().expect("lex");
        let program = crate::parser::Parser::new(tokens).parse().expect("parse");
        let module = Compiler::compile(&program).expect("compile");
        let mut vm = Vm::new();
        let start = general_member_ops();
        vm.execute(&module.main).expect("vm");
        let general = general_member_ops() - start;
        (vm.globals.get("result").cloned().expect("result"), general)
    }

    fn shown(value: &Value) -> String {
        format!("{}", value)
    }

    /// The bare reads, from a local (`GetLocalProperty`) and from a global
    /// (`GetProperty`), give what the `()` forms give and never reach the
    /// general arm. Bools are left out: another test in this binary sets the
    /// process-wide "user method on Bool" flag, which rightly sends them
    /// there.
    #[test]
    fn bare_builtins_stay_in_the_fast_tier() {
        const READS: &[&str] = &[
            "h.length",
            "h.size",
            "h.empty?",
            "h.keys",
            "h.values",
            "h.present?",
            "h.blank?",
            "h.nil?",
            "h.class",
            "s.length",
            "s.upcase",
            "s.trim",
            "s.blank?",
            "s.present?",
            "s.nil?",
            "s.to_s",
            "s.reverse",
            "s.empty?",
            "a.length",
            "a.first",
            "a.last",
            "a.empty?",
            "a.sum",
            "a.present?",
            "n.to_s",
            "n.to_i",
            "n.to_f",
            "n.nil?",
            "n.class",
            "f.to_s",
            "f.to_i",
            "d.to_s",
            "d.to_i",
            "nothing.to_s",
            "nothing.nil?",
            "nothing.blank?",
            "nothing.to_i",
        ];
        let setup = "h = {\"a\": 1, \"b\": 2}\ns = \"  Hello \"\na = [3, 1, 2]\nn = 42\n\
                     f = 2.5\nd = 1.25D\nnothing = nil\n";
        let bare = READS.join(", ");
        let paren = READS
            .iter()
            .map(|read| format!("{read}()"))
            .collect::<Vec<_>>()
            .join(", ");
        // Locals of a function: the fused `GetLocalProperty`.
        let local = format!("def go\n{setup}[{bare}]\nend\nresult = go()\n");
        // Globals: `GetGlobal` + `GetProperty`.
        let global = format!("{setup}result = [{bare}]\n");
        let (expected, _) = run(&format!("{setup}result = [{paren}]\n"));
        for src in [local, global] {
            let (value, general) = run(&src);
            assert_eq!(shown(&value), shown(&expected), "{src}");
            assert_eq!(general, 0, "a bare builtin left the fast tier:\n{src}");
        }
    }

    /// What the general arm answered before still holds: a hash key outranks
    /// the builtin on the VM, `h.len` reads as the method, and a name no
    /// table answers by id (`f.round`, `i.abs`) still goes by name.
    #[test]
    fn bare_reads_keep_their_precedence() {
        let (value, _) = run(
            "h = {\"length\": 99, \"keys\": \"k\"}\n\
             result = [h.length, h.keys, h.size, {\"a\": 1}.len, 2.5.round, (-3).abs, \"zz\".length]\n",
        );
        assert_eq!(shown(&value), "[99, k, 2, <method hash.len>, 3, 3, 2]");
    }
}

#[cfg(test)]
mod static_read_fast_tier_tests {
    //! `Conf.per_page` and `Conf.active` — a static field holding plain
    //! data, as an enum's unit variant is an instance in one — are read in
    //! the fast tier, from a global class (`GetProperty`)
    //! and from a local one (`GetLocalProperty`). A static field the general
    //! arm does more with (a callable it auto-invokes) still goes there.
    use super::general_member_ops;
    use crate::interpreter::value::Value;
    use crate::vm::compiler::Compiler;
    use crate::vm::Vm;

    fn run(src: &str) -> (String, usize) {
        let tokens = crate::lexer::Scanner::new(src).scan_tokens().expect("lex");
        let program = crate::parser::Parser::new(tokens).parse().expect("parse");
        let module = Compiler::compile(&program).expect("compile");
        let mut vm = Vm::new();
        let start = general_member_ops();
        vm.execute(&module.main).expect("vm");
        let general = general_member_ops() - start;
        let result: Value = vm.globals.get("result").cloned().expect("result");
        (format!("{}", result), general)
    }

    fn reads(n: usize) -> String {
        format!(
            "class Marker\nend\n\
             class Conf\n  static per_page: Int = 20\n  static active: Any = Marker.new()\n  static tags: Array = [\"a\"]\n\
             \x20 static label: String = \"c\"\n  static make: Any = fn() {{ 42 }}\nend\n\
             def go\n  klass = Conf\n  total = 0\n  hits = 0\n  i = 0\n  while i < {n}\n\
             \x20   total += Conf.per_page + klass.per_page + klass.tags.length\n\
             \x20   hits += 1 if Conf.active == klass.active\n\
             \x20   i += 1\n  end\n  [total, hits, Conf.label]\nend\n\
             def calls\n  total = 0\n  i = 0\n  while i < {n}\n    total += Conf.make\n\
             \x20   i += 1\n  end\n  total\nend\n\
             result = [go(), calls()]\n"
        )
    }

    /// The plain-data reads cost the general arms nothing per iteration, so
    /// 3 and 40 iterations run the same number of general member ops; the
    /// callable static field still runs one per iteration (37 more).
    #[test]
    fn static_field_reads_stay_in_the_fast_tier() {
        let (short, short_general) = run(&reads(3));
        let (long, long_general) = run(&reads(40));
        assert_eq!(short, "[[123, 3, c], 126]");
        assert_eq!(long, "[[1640, 40, c], 1680]");
        assert_eq!(
            long_general - short_general,
            37,
            "{short_general} vs {long_general}"
        );
    }
}

#[cfg(test)]
mod hash_write_tests {
    //! `h.key = v` on a hash completes in the fast tier, and every constant-key
    //! hash write (`h.key = v`, `h["key"] = v` on a local, a global or any
    //! other receiver) appends a new key as a refcount clone of the constant
    //! pool's `SoliStr`, never as a copy of its bytes.
    use super::general_member_ops;
    use crate::interpreter::value::{HashKey, Value};
    use crate::vm::chunk::{Constant, FunctionProto};
    use crate::vm::compiler::Compiler;
    use crate::vm::Vm;

    /// Run `src`; return its `result` global, the non-instance property ops
    /// the general arms ran, and the address of every heap-held string
    /// constant of the program (functions included).
    fn run(src: &str) -> (Value, usize, Vec<*const u8>) {
        let tokens = crate::lexer::Scanner::new(src).scan_tokens().expect("lex");
        let program = crate::parser::Parser::new(tokens).parse().expect("parse");
        let module = Compiler::compile(&program).expect("compile");
        let mut pool = Vec::new();
        collect_strings(&module.main, &mut pool);
        let mut vm = Vm::new();
        let start = general_member_ops();
        vm.execute(&module.main).expect("vm");
        let general = general_member_ops() - start;
        let result = vm.globals.get("result").cloned().expect("result");
        (result, general, pool)
    }

    fn collect_strings(proto: &FunctionProto, pool: &mut Vec<*const u8>) {
        for constant in &proto.chunk.constants {
            match constant {
                Constant::String(s) => pool.push(s.as_str().as_ptr()),
                Constant::Function(inner) => collect_strings(inner, pool),
                _ => {}
            }
        }
    }

    const BUILD: &str = r#"
h = {"a": 1}
h.a = 2
h.b = 3
h.a_key_longer_than_fifteen = 4
def build(n)
  out = {}
  out.first = n
  out.first = out.first + 10
  [1, 2].each { |k| out.from_block_key_long_name = k }
  i = 0
  while i < 10
    out["k#{i}"] = i
    i = i + 1
  end
  out.after_promotion = true
  out.first = 0
  out
end
o = {"inner": {"x": 1}}
o.inner.y = 2
result = [h, build(5), o]
"#;

    /// Dot writes on a hash — a global, a function's local, a block's
    /// upvalue, a nested hash, before and after the promotion past
    /// `SMALL_MAX` keys — give the general arm's result (overwrites keep
    /// their position, new keys append) and none of them leaves the tier.
    #[test]
    fn hash_dot_writes_stay_in_the_fast_tier() {
        let (value, general, _) = run(BUILD);
        assert_eq!(
            format!("{value}"),
            "[{a => 2, b => 3, a_key_longer_than_fifteen => 4}, \
             {first => 0, from_block_key_long_name => 2, k0 => 0, k1 => 1, k2 => 2, \
             k3 => 3, k4 => 4, k5 => 5, k6 => 6, k7 => 7, k8 => 8, k9 => 9, \
             after_promotion => true}, {inner => {x => 1, y => 2}}]"
        );
        assert_eq!(general, 0, "a hash dot write left the fast tier");
    }

    /// Each long constant key the writes added is the pool's own buffer:
    /// `SetProperty`, `HashSetGlobalConst`, `HashSetLocalConst` and
    /// `HashSetConst` append a refcount clone. Before, each copied the key
    /// into a `String` and then into a fresh `SoliStr`.
    #[test]
    fn constant_hash_keys_share_the_pool_buffer() {
        let src = r#"
g = {}
g.dot_key_longer_than_fifteen = 1
g["index_key_longer_than_fifteen"] = 2
def local_writes
  l = {}
  l.local_dot_key_longer_than_15 = 3
  l["local_index_key_longer_than_15"] = 4
  l
end
o = {"inner": {}}
o["inner"]["nested_index_key_longer_than_15"] = 5
result = [g, local_writes(), o["inner"]]
"#;
        let (value, _, pool) = run(src);
        let Value::Array(hashes) = value else {
            panic!("expected an array, got {value:?}");
        };
        let mut seen = 0;
        for hash in hashes.borrow().iter() {
            let Value::Hash(hash) = hash else {
                panic!("expected a hash, got {hash:?}");
            };
            for (key, _) in hash.borrow().iter() {
                let HashKey::String(key) = key else {
                    panic!("expected a string key, got {key:?}");
                };
                assert!(
                    pool.contains(&key.as_str().as_ptr()),
                    "key {key} was copied out of the constant pool"
                );
                seen += 1;
            }
        }
        assert_eq!(seen, 5);
    }
}

#[cfg(test)]
mod method_fast_tier_tests {
    //! `obj.m(args)`, `@m(args)` and `this.m(args)` on an instance complete
    //! in the fast tier whenever the general arm would open the compiled
    //! method's frame with nothing to pad or raise: the receiver's class
    //! chain has the method compiled, it takes exactly the arguments given,
    //! the depth limit is not reached and the privacy check lets the call
    //! through. The method comes from the call site's entry in the VM's
    //! method cache, which any change to some class's `vm_methods` stales.
    //! Everything else — a native or AST-only method, `send`,
    //! `method_missing`, omitted parameters, a refusal, the depth limit —
    //! still reaches the general arm, which answers as before.
    //!
    //! "General" below counts `call_method_slow_path` entries: every
    //! instance call the fast tier hands back enters it (a refused call
    //! raises before it, a `send` enters it twice).
    use crate::interpreter::value::{Class, Instance, NativeFunction, Value};
    use crate::vm::compiler::Compiler;
    use crate::vm::vm_calls::slow_path_calls;
    use crate::vm::Vm;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Run `src` with `__N__` replaced by `n` and `globals` bound; return its
    /// `result` global and the general method calls it made.
    fn run(src: &str, n: usize, globals: Vec<(String, Value)>) -> (Value, usize) {
        let src = src.replace("__N__", &n.to_string());
        let tokens = crate::lexer::Scanner::new(&src).scan_tokens().expect("lex");
        let program = crate::parser::Parser::new(tokens).parse().expect("parse");
        let module = Compiler::compile(&program).expect("compile");
        let mut vm = Vm::new();
        for (name, value) in globals {
            vm.globals.insert(name, value);
        }
        let start = slow_path_calls();
        vm.execute(&module.main).expect("vm");
        let general = slow_path_calls() - start;
        (vm.globals.get("result").cloned().expect("result"), general)
    }

    fn text(s: &str) -> Value {
        Value::String(s.into())
    }

    fn array(values: Vec<Value>) -> Value {
        Value::Array(Rc::new(RefCell::new(values)))
    }

    const SHAPES: &str = r##"
module Greets
  def hello(who)
    "#{who} #{@name}"
  end
end

class Shape
  name: String

  new(name: String)
    @name = name
  end

  include Greets

  def area
    0
  end

  def scaled(k)
    @area() * k
  end

  def label
    "shape"
  end

  def describe(k)
    "#{this.label()}:#{@scaled(k)}"
  end

  def fact(n)
    return 1 if n <= 1

    n * @fact(n - 1)
  end

  def length
    7
  end
end

class Square < Shape
  side: Int

  new(side: Int)
    super("square")
    @side = side
  end

  def area
    @side * @side
  end

  def label
    "square"
  end

  def describe(k)
    "[#{super.describe(k)}]"
  end
end

class Cube < Square
end

def total(shapes, n)
  sum = 0
  seen = []
  i = 0
  while i < n
    shapes.each do |shape|
      sum = sum + shape.scaled(i) + shape.length()
    end
    seen = [shapes[1].describe(2), shapes[2].describe(3), shapes[2].fact(5), shapes[2].hello("hi"), shapes[0]&.area()]
    i = i + 1
  end
  [sum, seen]
end

result = total([new Shape("blob"), new Square(3), new Cube(2)], __N__)
"##;

    /// Every method call of `SHAPES`'s loop completes in the fast tier, so
    /// the general calls do not grow with the loop: own, inherited and
    /// overridden methods; `@area()` in `Shape#scaled`, one site seeing three
    /// classes; `this.m()`, `@m(x)` and recursion; `length`, a name
    /// `CallMethodById` carries; a method an included module brings;
    /// `obj&.m()`; calls from a block. `cube.describe` runs
    /// `Square#describe`, whose `super` reaches `Shape#describe` only
    /// because the frame records the defining class, not the receiver's.
    #[test]
    fn method_calls_stay_in_the_fast_tier() {
        let mut general = Vec::new();
        for (n, sum) in [(3, 102), (30, 6285)] {
            let (result, spent) = run(SHAPES, n, Vec::new());
            assert_eq!(
                result,
                array(vec![
                    Value::Int(sum),
                    array(vec![
                        text("[square:18]"),
                        text("[square:12]"),
                        Value::Int(120),
                        text("hi square"),
                        Value::Int(0),
                    ]),
                ]),
                "n = {n}"
            );
            general.push(spent);
        }
        assert_eq!(
            general[0], general[1],
            "a call in the loop left the fast tier"
        );
    }

    const ACCOUNT: &str = r#"
class Account
  owner: String

  new(owner: String)
    @owner = owner
  end

  def balance(cents)
    cents / 100
  end

  def greet(who = "you")
    "hi #{who}"
  end

  def method_missing(name, args)
    "missing #{name}"
  end

  def dig(n)
    @dig(n + 1)
  end

  private

  def _secret
    "s"
  end
end

def calls(account, n)
  out = nil
  fast = 0
  i = 0
  while i < n
    fast = fast + account.balance(100 * i)
    out = __CALL__
    i = i + 1
  end
  [fast, out]
end

result = calls(new Account("ada"), __N__)
"#;

    /// What the fast tier hands back gets the general arm's answer, once per
    /// call: an omitted default, `send` on a class with none of its own
    /// (two entries: the `send`, then the call it makes), `method_missing`,
    /// too few arguments. A private method called from outside is refused
    /// by the general arm's check, before the slow path. The fast `balance`
    /// call beside each costs nothing.
    #[test]
    fn the_general_arm_answers_what_the_fast_tier_hands_back() {
        let cases = [
            ("account.greet()", text("hi you"), 1),
            ("account.greet(\"bo\")", text("hi bo"), 0),
            ("account.send(\"balance\", 500)", Value::Int(5), 2),
            ("account.whatever(1)", text("missing whatever"), 1),
            ("account.balance() rescue \"arity\"", text("arity"), 1),
            ("account._secret() rescue \"refused\"", text("refused"), 0),
        ];
        for (call, answer, per_call) in cases {
            let src = ACCOUNT.replace("__CALL__", call);
            let (short, short_general) = run(&src, 3, Vec::new());
            let (long, long_general) = run(&src, 30, Vec::new());
            assert_eq!(short, array(vec![Value::Int(3), answer.clone()]), "{call}");
            assert_eq!(long, array(vec![Value::Int(435), answer]), "{call}");
            assert_eq!(long_general - short_general, 27 * per_call, "{call}");
        }
    }

    /// The call at the depth limit goes to the general arm, which raises;
    /// every call below it stays in the fast tier.
    #[test]
    fn the_depth_limit_is_the_general_arms() {
        let src = ACCOUNT.replace("__CALL__", "1")
            + "result = new Account(\"a\").dig(0) rescue \"deep\"\n";
        let (result, general) = run(&src, 3, Vec::new());
        assert_eq!(result, text("deep"));
        assert_eq!(general, 1);
    }

    /// An instance of a class whose only `reading` is native.
    fn meter() -> Value {
        let reading = NativeFunction::new("reading", Some(1), |args: &[Value]| match args.get(1) {
            Some(Value::Int(n)) => Ok(Value::Int(n * 2)),
            _ => Err("reading expects an integer".to_string()),
        });
        let class = Rc::new(Class {
            name: "Meter".to_string(),
            native_methods: [("reading".to_string(), Rc::new(reading))]
                .into_iter()
                .collect(),
            ..Default::default()
        });
        Value::Instance(Rc::new(RefCell::new(Instance::new(class))))
    }

    const READINGS: &str = r#"
def readings(meter, n)
  total = 0
  i = 0
  while i < n
    total = total + meter.reading(i)
    i = i + 1
  end
  total
end

result = readings(meter, __N__)
"#;

    /// A native instance method has no compiled frame to open: every call
    /// goes to the general arm, which calls the native as before.
    #[test]
    fn a_native_method_is_called_by_the_general_arm() {
        for (n, total) in [(3, 6), (30, 870)] {
            let (result, general) = run(READINGS, n, vec![("meter".to_string(), meter())]);
            assert_eq!(result, Value::Int(total), "n = {n}");
            assert_eq!(general, n, "n = {n}");
        }
    }

    const PRICING_CLASS: &str = r#"
class Pricing
  rate: Int

  new(rate: Int)
    @rate = rate
  end

  def total(n)
    n * @rate
  end
end

pricing = new Pricing(2)
"#;

    const PRICES: &str = r#"
def sum(pricing, n)
  acc = 0
  i = 0
  while i < n
    acc = acc + pricing.total(i)
    i = i + 1
  end
  acc
end

result = sum(pricing, __N__)
"#;

    /// A class the tree-walker defined, as serve loads an app's classes, has
    /// its methods as AST only. The first call goes to the general arm,
    /// which compiles `total` and registers it on the class; that change
    /// stales the site's remembered miss, and every later call finds the
    /// compiled method and stays in the fast tier.
    #[test]
    fn an_ast_method_stays_in_the_fast_tier_once_compiled() {
        for (n, total) in [(3, 6), (30, 870)] {
            let tokens = crate::lexer::Scanner::new(PRICING_CLASS)
                .scan_tokens()
                .expect("lex");
            let program = crate::parser::Parser::new(tokens).parse().expect("parse");
            let mut interpreter = crate::interpreter::executor::Interpreter::new();
            interpreter.interpret(&program).expect("interpret");
            let globals = interpreter
                .environment
                .borrow()
                .get_all_bindings()
                .into_iter()
                .collect();
            let (result, general) = run(PRICES, n, globals);
            assert_eq!(result, Value::Int(total), "n = {n}");
            assert_eq!(general, 1, "n = {n}");
        }
    }

    const REDEFINED: &str = r#"
class A
  def greet
    "A"
  end

  def hi
    "hi"
  end

  def yo
    "yo"
  end
end

class B < A
end

class C < B
end

def greet_of(x)
  x.greet()
end

def hi_of(x)
  x.hi()
end

b = new B()
c = new C()
first = [greet_of(b), greet_of(c), hi_of(c)]
B.define_method("greet", fn() { "B" })
A.alias_method("hi", "yo")
result = [first, [greet_of(b), greet_of(c), hi_of(c)]]
"#;

    /// A method defined or aliased after a call site has run is what the
    /// site calls next: `B.define_method` is seen on `B` and on its subclass
    /// `C`, `A.alias_method` on a grandchild. Without the epoch the second
    /// round would answer "A", "A", "hi" again from the cache.
    #[test]
    fn a_method_defined_after_a_call_is_seen_at_the_same_site() {
        let (result, _) = run(REDEFINED, 0, Vec::new());
        assert_eq!(
            result,
            array(vec![
                array(vec![text("A"), text("A"), text("hi")]),
                array(vec![text("B"), text("B"), text("yo")]),
            ])
        );
    }
}
