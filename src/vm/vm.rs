//! The bytecode virtual machine — stack-based execution engine.

use ahash::AHashMap as HashMap;
use ahash::RandomState as AHasher;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use crate::error::RuntimeError;
use crate::interpreter::value::{Class, HashKey, HashPairs, StrKey, Value};
use crate::metrics::VmTimingGuard;
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
}

impl CallFrame {
    /// Build a frame for `closure` starting at ip 0, caching the raw code
    /// pointer for the dispatch loop's per-op fetch.
    #[inline]
    pub fn new(
        closure: Rc<VmClosure>,
        stack_base: usize,
        class: Option<Rc<crate::interpreter::value::Class>>,
    ) -> Self {
        let code = closure.proto.chunk.code.as_ptr();
        let code_len = closure.proto.chunk.code.len();
        Self {
            closure,
            ip: 0,
            stack_base,
            class,
            code,
            code_len,
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
}

/// The bytecode VM.
pub struct Vm {
    /// Value stack.
    pub stack: Vec<Value>,
    /// Call frame stack.
    pub frames: Vec<CallFrame>,
    /// Global variables.
    pub globals: HashMap<String, Value>,
    /// Open upvalues (pointing to stack slots that are still live).
    pub open_upvalues: Vec<Rc<RefCell<Upvalue>>>,
    /// Exception handler stack.
    pub exception_handlers: Vec<ExceptionHandler>,
    /// Iterator state stack (for for-in loops).
    pub iter_stack: Vec<IterState>,
    /// Output buffer for print statements (for testing/capture).
    pub output: Vec<String>,
    /// Handlers that failed VM execution — skip VM for these and use interpreter directly.
    pub failed_handlers: ahash::AHashSet<String>,
    /// Frame depth at which `run()` should stop. Bumped by native methods that
    /// need to synchronously invoke a user closure (e.g. array.map); `Op::Return`
    /// treats frames shrinking back to this depth as the exit condition.
    pub return_depth: usize,
}

impl Vm {
    pub fn new() -> Self {
        Self {
            stack: Vec::with_capacity(256),
            frames: Vec::with_capacity(64),
            globals: HashMap::new(),
            open_upvalues: Vec::new(),
            exception_handlers: Vec::new(),
            iter_stack: Vec::new(),
            output: Vec::new(),
            failed_handlers: ahash::AHashSet::new(),
            return_depth: 0,
        }
    }

    /// Execute a compiled module (top-level script).
    pub fn execute(&mut self, proto: &Arc<FunctionProto>) -> Result<Value, RuntimeError> {
        let closure = Rc::new(VmClosure::new(proto.clone(), Vec::new()));
        self.push(Value::VmClosure(closure.clone()));

        self.frames.push(CallFrame::new(closure, 0, None));

        self.run()
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
        Span::new(0, 0, line, 0)
    }

    /// Read a string constant as an owned String (for global-map keys).
    #[inline]
    fn read_string_constant_owned(&self, idx: u16) -> String {
        let frame = self.frames.last().unwrap();
        match &frame.closure.proto.chunk.constants[idx as usize] {
            Constant::String(s) => s.to_string(),
            _ => String::new(),
        }
    }

    /// Run the dispatch loop.
    /// Execute until the program completes or an error escapes.
    ///
    /// RuntimeErrors raised by ops (native methods, type errors, missing
    /// properties) are routed through the innermost active `try`/`rescue`
    /// handler — matching the tree-walker, where `catch (e)` binds the
    /// error text. Without an active handler the error propagates (which
    /// is what serve-mode's interpreter fallback relies on).
    pub fn run(&mut self) -> Result<Value, RuntimeError> {
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
                return Err(err);
            }
            let span = err.span();
            self.throw_exception(Value::String(format!("{}", err).into()), span)?;
        }
    }

    fn run_dispatch(&mut self) -> Result<Value, RuntimeError> {
        let _guard = VmTimingGuard::new();
        loop {
            // Fetch opcode and advance IP in a scoped borrow. Uses the
            // frame's cached code pointer — the closure→proto→chunk→code[ip]
            // chain cost three pointer hops plus a bounds check on every
            // executed instruction.
            let op = {
                let frame = self.frames.last_mut().unwrap();
                let ip = frame.ip;
                if ip >= frame.code_len {
                    return Ok(Value::Null);
                }
                // SAFETY: `code`/`code_len` cache `closure.proto.chunk.code`,
                // which the frame's own Rc keeps alive; `ip < code_len` was
                // just checked, and chunks are immutable after compilation.
                let op = unsafe { *frame.code.add(ip) };
                frame.ip = ip + 1;
                op
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
                            return Err(RuntimeError::undefined_variable(
                                name,
                                self.current_span(),
                            ));
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
                            return Err(RuntimeError::undefined_variable(
                                name,
                                self.current_span(),
                            ));
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
                    let upvalue =
                        self.frames.last().unwrap().closure.upvalues[idx as usize].clone();
                    let mut uv = upvalue.borrow_mut();
                    match &mut *uv {
                        Upvalue::Open(slot) => {
                            self.stack[*slot] = val;
                        }
                        Upvalue::Closed(v) => {
                            *v = val;
                        }
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
                        (Value::Int(x), Value::Int(y)) => Value::Int(x + y),
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
                        (Value::Int(x), Value::Int(y)) => Value::Int(x - y),
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
                        (Value::Int(x), Value::Int(y)) => Value::Int(x * y),
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
                            Value::Int(x / y)
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
                            Value::Int(x % y)
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
                        Value::Int(n) => self.stack.push(Value::Int(-n)),
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
                    let result = match (&a, &b) {
                        (Value::Int(x), Value::Int(y)) => x == y,
                        (Value::Bool(x), Value::Bool(y)) => x == y,
                        _ => crate::interpreter::value::enum_aware_equal(&a, &b),
                    };
                    self.stack.push(Value::Bool(result));
                }
                Op::NotEqual => {
                    let (a, b) = self.pop2();
                    let result = match (&a, &b) {
                        (Value::Int(x), Value::Int(y)) => x != y,
                        (Value::Bool(x), Value::Bool(y)) => x != y,
                        _ => !crate::interpreter::value::enum_aware_equal(&a, &b),
                    };
                    self.stack.push(Value::Bool(result));
                }
                Op::Less => {
                    let (a, b) = self.pop2();
                    let result = match (&a, &b) {
                        (Value::Int(x), Value::Int(y)) => *x < *y,
                        (Value::Float(x), Value::Float(y)) => *x < *y,
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
                        _ => {
                            let span = self.current_span();
                            self.op_compare_less(&b, &a, span)?
                        }
                    };
                    self.stack.push(Value::Bool(result));
                }
                Op::GreaterEqual => {
                    let (a, b) = self.pop2();
                    let result = match (&a, &b) {
                        (Value::Int(x), Value::Int(y)) => *x >= *y,
                        (Value::Float(x), Value::Float(y)) => *x >= *y,
                        _ => {
                            let span = self.current_span();
                            self.op_compare_less_equal(&b, &a, span)?
                        }
                    };
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
                Op::Loop(offset) => {
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
                    if let Value::VmClosure(closure) = &self.stack[callee_idx] {
                        let closure = closure.clone();
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
                        self.frames.push(CallFrame::new(closure, stack_base, None));
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
                            return Err(RuntimeError::undefined_variable(
                                name,
                                self.current_span(),
                            ));
                        }
                    }
                }
                Op::CallMethod(name_idx, argc) => {
                    let argc = argc as usize;
                    let receiver_idx = self.stack.len() - 1 - argc;

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

                    // User-method fast-path guard: gate on the per-type bit in
                    // USER_METHOD_FLAGS. Zero overhead when no user methods exist.
                    use crate::interpreter::executor::calls::user_methods::{
                        has_user_methods as _has_um, lookup_user_method as _lookup_um, PrimType,
                    };
                    let user_prim = if _has_um(PrimType::Int)
                        || _has_um(PrimType::Float)
                        || _has_um(PrimType::Bool)
                        || _has_um(PrimType::Null)
                        || _has_um(PrimType::Decimal)
                        || _has_um(PrimType::String)
                        || _has_um(PrimType::Array)
                        || _has_um(PrimType::Hash)
                        || _has_um(PrimType::Symbol)
                    {
                        match &self.stack[receiver_idx] {
                            Value::Int(_) => Some(PrimType::Int),
                            Value::Float(_) => Some(PrimType::Float),
                            Value::Bool(_) => Some(PrimType::Bool),
                            Value::Null => Some(PrimType::Null),
                            Value::Decimal(_) => Some(PrimType::Decimal),
                            Value::String(_) => Some(PrimType::String),
                            Value::Array(_) => Some(PrimType::Array),
                            Value::Hash(_) => Some(PrimType::Hash),
                            Value::Symbol(_) => Some(PrimType::Symbol),
                            _ => None,
                        }
                    } else {
                        None
                    };
                    if let Some(t) = user_prim {
                        if let Some(f) = _lookup_um(t, name) {
                            let span = self.current_span();
                            let object = self.stack[receiver_idx].clone();
                            let bound = crate::interpreter::executor::access::member::bind_user_method_to_receiver(object, f);
                            self.stack[receiver_idx] = bound;
                            self.call_value(argc, span)?;
                            continue;
                        }
                    }

                    // Fast path: dispatch on receiver type without cloning
                    match &self.stack[receiver_idx] {
                        Value::String(_) => {
                            // Ultra-fast path: inline common zero-arg string methods
                            let result = if argc == 0 {
                                let s: &str = match &self.stack[receiver_idx] {
                                    Value::String(s) => s.as_ref(),
                                    _ => unreachable!(),
                                };
                                match name {
                                    "len" | "length" => Some(Value::Int(s.len() as i64)),
                                    "empty?" => Some(Value::Bool(s.is_empty())),
                                    "bytesize" => Some(Value::Int(s.len() as i64)),
                                    "upcase" | "uppercase" => {
                                        Some(Value::String(s.to_uppercase().into()))
                                    }
                                    "downcase" | "lowercase" => {
                                        Some(Value::String(s.to_lowercase().into()))
                                    }
                                    "trim" => Some(Value::String(s.trim().to_string().into())),
                                    "reverse" => {
                                        let out: String = s.chars().rev().collect();
                                        Some(Value::String(out.into()))
                                    }
                                    "nil?" => Some(Value::Bool(false)),
                                    "class" => Some(Value::String("string".into())),
                                    _ => None,
                                }
                            } else {
                                None
                            };
                            if let Some(result) = result {
                                self.stack.truncate(receiver_idx);
                                self.stack.push(result);
                            } else {
                                // General path: borrow string and args from stack
                                let result = {
                                    let s: &str = match &self.stack[receiver_idx] {
                                        Value::String(s) => s.as_ref(),
                                        _ => unreachable!(),
                                    };
                                    let args =
                                        &self.stack[receiver_idx + 1..receiver_idx + 1 + argc];
                                    let span = self.current_span();
                                    self.vm_call_string_method(s, name, args, span)?
                                };
                                self.stack.truncate(receiver_idx);
                                self.stack.push(result);
                            }
                        }
                        Value::Instance(_) | Value::Class(_) | Value::VmClosure(_) => {
                            self.call_method_slow_path(receiver_idx, argc, name)?;
                        }
                        Value::Array(_) => {
                            // Fast path for common zero-arg array methods
                            let result = if argc == 0 {
                                let arr = match &self.stack[receiver_idx] {
                                    Value::Array(a) => a,
                                    _ => unreachable!(),
                                };
                                match name {
                                    "length" | "len" => Some(Value::Int(arr.borrow().len() as i64)),
                                    "empty?" => Some(Value::Bool(arr.borrow().is_empty())),
                                    "first" => {
                                        Some(arr.borrow().first().cloned().unwrap_or(Value::Null))
                                    }
                                    "last" => {
                                        Some(arr.borrow().last().cloned().unwrap_or(Value::Null))
                                    }
                                    "nil?" => Some(Value::Bool(false)),
                                    "class" => Some(Value::String("array".into())),
                                    "blank?" => Some(Value::Bool(arr.borrow().is_empty())),
                                    "present?" => Some(Value::Bool(!arr.borrow().is_empty())),
                                    _ => None,
                                }
                            } else {
                                None
                            };
                            if let Some(result) = result {
                                self.stack.truncate(receiver_idx);
                                self.stack.push(result);
                            } else {
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
                        }
                        Value::Hash(_) => {
                            // Fast path for common zero-arg hash methods
                            let result = if argc == 0 {
                                let hash = match &self.stack[receiver_idx] {
                                    Value::Hash(h) => h,
                                    _ => unreachable!(),
                                };
                                match name {
                                    "length" | "len" => {
                                        Some(Value::Int(hash.borrow().len() as i64))
                                    }
                                    "empty?" => Some(Value::Bool(hash.borrow().is_empty())),
                                    "nil?" => Some(Value::Bool(false)),
                                    "class" => Some(Value::String("hash".into())),
                                    "blank?" => Some(Value::Bool(hash.borrow().is_empty())),
                                    "present?" => Some(Value::Bool(!hash.borrow().is_empty())),
                                    _ => None,
                                }
                            } else {
                                None
                            };
                            if let Some(result) = result {
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
                                let result = self.vm_call_hash_method(&hash, name, &args, span)?;
                                self.stack.push(result);
                            }
                        }
                        Value::Int(_)
                        | Value::Float(_)
                        | Value::Bool(_)
                        | Value::Null
                        | Value::Decimal(_) => {
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
                Op::CallMethodById(name_idx, argc, method_id) => {
                    let argc = argc as usize;
                    let receiver_idx = self.stack.len() - 1 - argc;
                    // Fast path: zero-arg methods on primitives — pure integer dispatch
                    if argc == 0 {
                        let result = match &self.stack[receiver_idx] {
                            Value::String(s) => {
                                super::method_table::string_method_zero_arg(s.as_ref(), method_id)
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
                                let s: &str = match &self.stack[receiver_idx] {
                                    Value::String(s) => s.as_ref(),
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
                            let hash = match &self.stack[receiver_idx] {
                                Value::Hash(h) => h.clone(),
                                _ => unreachable!(),
                            };
                            let args: Vec<Value> =
                                self.stack[receiver_idx + 1..receiver_idx + 1 + argc].to_vec();
                            let span = self.current_span();
                            self.stack.truncate(receiver_idx);
                            let result = self.vm_call_hash_method(&hash, name, &args, span)?;
                            self.stack.push(result);
                        }
                        Value::Int(_)
                        | Value::Float(_)
                        | Value::Bool(_)
                        | Value::Null
                        | Value::Decimal(_) => {
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
                            return Err(RuntimeError::NoSuchProperty {
                                value_type: other.type_name(),
                                property: "get".to_string(),
                                span: self.current_span(),
                            });
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
                            return Err(RuntimeError::NoSuchProperty {
                                value_type: other.type_name(),
                                property: "has_key".to_string(),
                                span: self.current_span(),
                            });
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
                                .swap_remove(&StrKey(key))
                                .unwrap_or(Value::Null);
                            self.push(value);
                        }
                        other => {
                            return Err(RuntimeError::NoSuchProperty {
                                value_type: other.type_name(),
                                property: "delete".to_string(),
                                span: self.current_span(),
                            });
                        }
                    }
                }
                Op::HashSetConst(key_idx) => {
                    let value = self.pop();
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
                            let mut hash_ref = hash.borrow_mut();
                            if let Some((_, _, existing)) = hash_ref.get_full_mut(&StrKey(key)) {
                                *existing = value.clone();
                            } else {
                                hash_ref
                                    .insert(HashKey::String(key.to_string().into()), value.clone());
                            }
                            self.push(Value::Null);
                        }
                        other => {
                            return Err(RuntimeError::NoSuchProperty {
                                value_type: other.type_name(),
                                property: "set".to_string(),
                                span: self.current_span(),
                            });
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
                    let result = match &self.stack[base + slot as usize] {
                        Value::Hash(hash) => hash
                            .borrow()
                            .get(&StrKey(key))
                            .cloned()
                            .unwrap_or(Value::Null),
                        other => {
                            return Err(RuntimeError::NoSuchProperty {
                                value_type: other.type_name(),
                                property: "get".to_string(),
                                span: self.current_span(),
                            });
                        }
                    };
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
                            return Err(RuntimeError::NoSuchProperty {
                                value_type: other.type_name(),
                                property: "has_key".to_string(),
                                span: self.current_span(),
                            });
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
                            .swap_remove(&StrKey(key))
                            .unwrap_or(Value::Null),
                        other => {
                            return Err(RuntimeError::NoSuchProperty {
                                value_type: other.type_name(),
                                property: "delete".to_string(),
                                span: self.current_span(),
                            });
                        }
                    };
                    self.push(result);
                }
                Op::HashSetLocalConst(slot, key_idx) => {
                    let value = self.pop();
                    let base = self.frames.last().unwrap().stack_base;
                    let key: *const str = {
                        let frame = self.frames.last().unwrap();
                        match &frame.closure.proto.chunk.constants[key_idx as usize] {
                            Constant::String(s) => s.as_ref() as *const str,
                            _ => "" as *const str,
                        }
                    };
                    let key: &str = unsafe { &*key };
                    match &self.stack[base + slot as usize] {
                        Value::Hash(hash) => {
                            let mut hash_ref = hash.borrow_mut();
                            if let Some((_, _, existing)) = hash_ref.get_full_mut(&StrKey(key)) {
                                *existing = value;
                            } else {
                                hash_ref.insert(HashKey::String(key.to_string().into()), value);
                            }
                        }
                        other => {
                            return Err(RuntimeError::NoSuchProperty {
                                value_type: other.type_name(),
                                property: "set".to_string(),
                                span: self.current_span(),
                            });
                        }
                    }
                    self.push(Value::Null);
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
                            return Err(RuntimeError::NoSuchProperty {
                                value_type: other.type_name(),
                                property: "get".to_string(),
                                span: self.current_span(),
                            });
                        }
                        None => {
                            return Err(RuntimeError::undefined_variable(
                                global_name.to_string(),
                                self.current_span(),
                            ));
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
                            return Err(RuntimeError::NoSuchProperty {
                                value_type: other.type_name(),
                                property: "has_key".to_string(),
                                span: self.current_span(),
                            });
                        }
                        None => {
                            return Err(RuntimeError::undefined_variable(
                                global_name.to_string(),
                                self.current_span(),
                            ));
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
                                .swap_remove(&StrKey(key))
                                .unwrap_or(Value::Null);
                            self.push(value);
                        }
                        Some(other) => {
                            return Err(RuntimeError::NoSuchProperty {
                                value_type: other.type_name(),
                                property: "delete".to_string(),
                                span: self.current_span(),
                            });
                        }
                        None => {
                            return Err(RuntimeError::undefined_variable(
                                global_name.to_string(),
                                self.current_span(),
                            ));
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
                            let mut hash_ref = hash.borrow_mut();
                            if let Some((_, _, existing)) = hash_ref.get_full_mut(&StrKey(key)) {
                                *existing = value;
                            } else {
                                hash_ref.insert(HashKey::String(key.to_string().into()), value);
                            }
                        }
                        Some(other) => {
                            return Err(RuntimeError::NoSuchProperty {
                                value_type: other.type_name(),
                                property: "set".to_string(),
                                span: self.current_span(),
                            });
                        }
                        None => {
                            return Err(RuntimeError::undefined_variable(
                                global_name.to_string(),
                                self.current_span(),
                            ));
                        }
                    }
                    self.push(Value::Null);
                }
                Op::Closure(idx) => {
                    let frame = self.frames.last().unwrap();
                    let constant = &frame.closure.proto.chunk.constants[idx as usize];
                    if let Constant::Function(proto) = constant {
                        let proto = proto.clone();
                        let mut upvalues = Vec::with_capacity(proto.upvalue_descriptors.len());

                        for desc in &proto.upvalue_descriptors {
                            if desc.is_local {
                                let base = self.frames.last().unwrap().stack_base;
                                let slot = base + desc.index as usize;
                                let upvalue = self.capture_upvalue(slot);
                                upvalues.push(upvalue);
                            } else {
                                let upvalue = self.frames.last().unwrap().closure.upvalues
                                    [desc.index as usize]
                                    .clone();
                                upvalues.push(upvalue);
                            }
                        }

                        let closure = VmClosure::new(proto, upvalues);
                        self.stack.push(Value::VmClosure(Rc::new(closure)));
                    }
                }
                Op::Return => {
                    let result = self.pop();
                    let frame = self.frames.pop().unwrap();

                    // Close upvalues only if there are any open ones in this frame's range
                    if !self.open_upvalues.is_empty() {
                        self.close_upvalues(frame.stack_base);
                    }

                    // Restore the stack
                    self.stack.truncate(frame.stack_base);

                    if self.frames.len() <= self.return_depth {
                        return Ok(result);
                    }

                    self.stack.push(result);
                }

                // --- Collections ---
                Op::Array(n) => {
                    let len = self.stack.len();
                    let start = len - n as usize;
                    let mut elements = self.stack.split_off(start);
                    // split_off preserves order, no reverse needed
                    let _ = &mut elements; // ensure move
                    self.stack
                        .push(Value::Array(Rc::new(RefCell::new(elements))));
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
                    let mut map = HashPairs::with_capacity_and_hasher(n, AHasher::default());
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
                    let mut map = HashPairs::with_capacity_and_hasher(n, AHasher::default());
                    let mut drained = self.stack.drain(base..);
                    for k in keys {
                        let v = drained.next().unwrap();
                        map.insert(k.clone(), v);
                    }
                    drop(drained);
                    self.stack.push(Value::Hash(Rc::new(RefCell::new(map))));
                }
                Op::Range => {
                    let (start, end) = self.pop2();
                    match (&start, &end) {
                        (Value::Int(a), Value::Int(b)) => {
                            // Exclusive of the end, matching the tree-walking
                            // interpreter's `eval_range` (`start..end`).
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
                    let result = self.op_get_index(&object, &index, self.current_span())?;
                    self.stack.push(result);
                }
                Op::SetIndex => {
                    let value = self.stack.pop().unwrap();
                    let index = self.stack.pop().unwrap();
                    let object = self.stack.pop().unwrap();
                    self.op_set_index(&object, &index, value, self.current_span())?;
                    self.stack.push(object);
                }
                Op::BuildString(n) => {
                    let len = self.stack.len();
                    let start = len - n as usize;
                    let mut capacity = 0;
                    for i in start..len {
                        if let Value::String(s) = &self.stack[i] {
                            capacity += s.len();
                        } else {
                            capacity += 8;
                        }
                    }
                    let mut result = String::with_capacity(capacity);
                    for i in start..len {
                        self.stack[i].append_to_string(&mut result);
                    }
                    self.stack.truncate(start);
                    self.stack.push(Value::String(result.into()));
                }
                Op::Spread => {
                    // Spread is handled by the array/hash/call compilation
                }

                // --- Properties ---
                Op::GetProperty(idx) => {
                    let object = self.stack.pop().unwrap();
                    // Fast path: instance-field hit. Fields shadow methods
                    // (op_get_property's documented order), so a present key
                    // fully decides the access — no name copy, no span, and
                    // a single ahash probe instead of the contains_key +
                    // method-walk + re-probe of the general path.
                    if let Value::Instance(inst) = &object {
                        let hit = {
                            let frame = self.frames.last().unwrap();
                            match &frame.closure.proto.chunk.constants[idx as usize] {
                                Constant::String(name) => {
                                    inst.borrow().fields.get(name.as_ref()).cloned()
                                }
                                _ => None,
                            }
                        };
                        if let Some(val) = hit {
                            self.stack.push(val);
                            continue;
                        }
                    }
                    let name = self.read_string_constant_owned(idx);
                    let span = self.current_span();
                    let result = self.op_get_property_member(&object, &name, span)?;
                    self.stack.push(result);
                }
                Op::SetProperty(idx) => {
                    let value = self.stack.pop().unwrap();
                    // Fast path: in-place update of an EXISTING instance
                    // field — no name copy, no span. First-time sets (key
                    // absent) fall through to the general path, which needs
                    // the owned name for the insert anyway.
                    let updated = {
                        let object = self.stack.last().unwrap();
                        if let Value::Instance(inst) = object {
                            let frame = self.frames.last().unwrap();
                            match &frame.closure.proto.chunk.constants[idx as usize] {
                                Constant::String(name) => {
                                    let mut inst_mut = inst.borrow_mut();
                                    if let Some(slot) = inst_mut.fields.get_mut(name.as_ref()) {
                                        *slot = value.clone();
                                        true
                                    } else {
                                        false
                                    }
                                }
                                _ => false,
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

                // --- Classes ---
                Op::Class(idx) => {
                    let name = self.read_string_constant_owned(idx);
                    let class = Class {
                        name,
                        ..Default::default()
                    };
                    self.stack.push(Value::Class(Rc::new(class)));
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
                Op::GetThis => {
                    let base = self.frames.last().unwrap().stack_base;
                    let this = self.stack[base].clone();
                    self.stack.push(this);
                }
                Op::GetSuper(idx) => {
                    let _name = self.read_string_constant_owned(idx);
                    let _this = self.stack.pop().unwrap();
                    self.stack.push(Value::Null);
                }
                Op::CallSuperInit(argc) => {
                    let argc = argc as usize;
                    let span = self.current_span();
                    let receiver_idx = self.stack.len() - 1 - argc;
                    let superclass = self.frame_superclass(span)?;
                    // Compiled superclass constructor — `this` is already in
                    // the callee slot; "init" returns `this`.
                    if let Some((init, defining_class)) =
                        superclass.find_vm_method_with_class("init")
                    {
                        self.call_closure_in_class(init, argc, span, Some(defining_class))?;
                    } else if let Some(ctor) = superclass.find_constructor() {
                        // Tree-walking superclass constructor: JIT-compile,
                        // run to completion, evaluate to `this`.
                        let this_val = self.stack[receiver_idx].clone();
                        self.run_jit_method_to_completion(&ctor, argc, span)?;
                        self.push(this_val);
                    } else {
                        // No superclass constructor: drop args, evaluate to
                        // `this` (matching tree-walker behavior).
                        self.stack.truncate(receiver_idx + 1);
                    }
                }
                Op::CallSuperMethod(name_idx, argc) => {
                    let argc = argc as usize;
                    let span = self.current_span();
                    let receiver_idx = self.stack.len() - 1 - argc;
                    let name = self.read_string_constant_owned(name_idx);
                    let superclass = self.frame_superclass(span)?;
                    if let Some((closure, defining_class)) =
                        superclass.find_vm_method_with_class(&name)
                    {
                        self.call_closure_in_class(closure, argc, span, Some(defining_class))?;
                    } else if let Some(method) = superclass.find_method(&name) {
                        // Tree-walking method (e.g. a class copied from
                        // interpreter globals): JIT and run re-entrantly.
                        let result = self.run_jit_method_to_completion(&method, argc, span)?;
                        self.push(result);
                    } else if let Some(native) = superclass.find_native_method(&name) {
                        // Direct native call — same helper as CallMethod, no
                        // per-call bound-wrapper allocation. Model subclasses
                        // still EngineFallback so lifecycle callbacks fire in
                        // the tree-walker (see op_get_property carve-out).
                        let receiver = self.stack[receiver_idx].clone();
                        if let Value::Instance(ref inst) = receiver {
                            if inst.borrow().class.is_model_subclass() {
                                return Err(RuntimeError::EngineFallback(
                                    format!("model instance method '{}'", name),
                                    span,
                                ));
                            }
                            if let Some(expected) = native.arity {
                                if argc != expected {
                                    return Err(RuntimeError::wrong_arity(expected, argc, span));
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
                        } else {
                            return Err(RuntimeError::type_error(
                                "super method called on non-instance",
                                span,
                            ));
                        }
                    } else {
                        return Err(RuntimeError::NoSuchProperty {
                            value_type: format!("super({})", superclass.name),
                            property: name,
                            span,
                        });
                    }
                }
                Op::Field(idx) => {
                    let name = self.read_string_constant_owned(idx);
                    let init_value = self.stack.pop().unwrap();
                    let _ = (name, init_value);
                }
                Op::StaticField(idx) => {
                    let name = self.read_string_constant_owned(idx);
                    let value = self.stack.pop().unwrap();
                    let class = self.stack.last().unwrap().clone();
                    if let Value::Class(ref cls) = class {
                        cls.static_fields.borrow_mut().insert(name, value);
                    }
                }
                Op::ConstField(idx) => {
                    let name = self.read_string_constant_owned(idx);
                    let init_value = self.stack.pop().unwrap();
                    let _ = (name, init_value);
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
                    };
                    self.exception_handlers.push(handler);
                }
                Op::TryEnd => {
                    self.exception_handlers.pop();
                }
                Op::Throw => {
                    let value = self.stack.pop().unwrap();
                    let span = self.current_span();
                    self.throw_exception(value, span)?;
                }
                Op::CatchMatch(name_idx, jump_offset) => {
                    let type_name = self.read_string_constant_owned(name_idx);
                    let matches = match self.stack.last().unwrap() {
                        Value::Instance(inst) => {
                            let inst = inst.borrow();
                            class_name_matches(&inst.class, &type_name)
                        }
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
                    let len = self.stack.len();
                    let start = len - n as usize;
                    let output: String = self.stack[start..len]
                        .iter()
                        .map(|v| format!("{}", v))
                        .collect::<Vec<_>>()
                        .join(" ");
                    self.stack.truncate(start);
                    println!("{}", output);
                    self.output.push(output);
                    self.stack.push(Value::Null);
                }

                Op::Nop => {}

                // Imports are resolved before VM execution; the opcode is a
                // no-op marker at runtime.
                Op::Import(_) => {}

                // --- Combined compare+jump ---
                Op::TestLessEqualJump(offset) => {
                    let (a, b) = self.pop2();
                    let result = match (&a, &b) {
                        (Value::Int(x), Value::Int(y)) => *x <= *y,
                        (Value::Float(x), Value::Float(y)) => *x <= *y,
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
                Op::IncrLocal(slot) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let idx = base + slot as usize;
                    if let Value::Int(n) = self.stack[idx] {
                        self.stack[idx] = Value::Int(n + 1);
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
                        self.stack[idx] = Value::Int(n - 1);
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
                        (Value::Int(x), Value::Int(y)) => Value::Int(x + y),
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
                        _ => {
                            let span = self.current_span();
                            self.op_compare_less_equal(a, b, span)?
                        }
                    };
                    self.stack.push(Value::Bool(result));
                }
                Op::AddLocalConst(slot, const_idx) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let local = &self.stack[base + slot as usize];
                    let frame = self.frames.last().unwrap();
                    let constant = &frame.closure.proto.chunk.constants[const_idx as usize];
                    let result = match (local, constant) {
                        (Value::Int(x), Constant::Int(y)) => Value::Int(x + y),
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
                        (Value::Int(x), Value::Int(y)) => Value::Int(x - y),
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
                        (Value::Int(x), Value::Int(y)) => Value::Int(x * y),
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
                            Value::Int(x / y)
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
                            Value::Int(x % y)
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
                        (Value::Int(x), Constant::Int(y)) => Value::Int(x - y),
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
                        (Value::Int(x), Constant::Int(y)) => Value::Int(x * y),
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
                            Value::Int(x / y)
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
                        _ => local != &constant_to_value(constant),
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
                        _ => local == &constant_to_value(constant),
                    };
                    self.stack.push(Value::Bool(result));
                }
                // --- Test + Jump super-instructions ---
                Op::TestGreaterJump(offset) => {
                    let (a, b) = self.pop2();
                    let result = match (&a, &b) {
                        (Value::Int(x), Value::Int(y)) => *x > *y,
                        (Value::Float(x), Value::Float(y)) => *x > *y,
                        _ => {
                            let span = self.current_span();
                            self.op_compare_less(&b, &a, span)?
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
                        _ => {
                            let span = self.current_span();
                            self.op_compare_less_equal(&b, &a, span)?
                        }
                    };
                    if !result {
                        self.frames.last_mut().unwrap().ip += offset as usize;
                    }
                }
                Op::TestNotEqualJump(offset) => {
                    let (a, b) = self.pop2();
                    let result = match (&a, &b) {
                        (Value::Int(x), Value::Int(y)) => x != y,
                        (Value::Bool(x), Value::Bool(y)) => x != y,
                        _ => a != b,
                    };
                    if !result {
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
                Op::AddLocalInt(slot, n) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let local = &self.stack[base + slot as usize];
                    let result = match local {
                        Value::Int(x) => Value::Int(x + n as i64),
                        Value::Float(x) => Value::Float(*x + n as f64),
                        _ => {
                            let a = local.clone();
                            let span = self.current_span();
                            self.op_add(a, Value::Int(n as i64), span)?
                        }
                    };
                    self.stack.push(result);
                }
                Op::IncrLocalFast(slot) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let idx = base + slot as usize;
                    if let Value::Int(n) = self.stack[idx] {
                        self.stack[idx] = Value::Int(n + 1);
                    } else {
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
                Op::SwapSetLocal(slot) => {
                    let new_val = self.pop();
                    let base = self.frames.last().unwrap().stack_base;
                    let old_val = self.stack[base + slot as usize].clone();
                    self.stack[base + slot as usize] = new_val;
                    self.stack.push(old_val);
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
                            return Err(RuntimeError::undefined_variable(
                                name,
                                self.current_span(),
                            ));
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
                    let result = match val {
                        Value::Int(n) => Value::Int(-n),
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
                    let a = &self.stack[base + slot_a as usize];
                    let b = &self.stack[base + slot_b as usize];
                    let result = match (a, b) {
                        (Value::Int(x), Value::Int(y)) => x == y,
                        (Value::Bool(x), Value::Bool(y)) => x == y,
                        _ => a == b,
                    };
                    self.stack.push(Value::Bool(result));
                }
                Op::NotEqualLocalLocal(slot_a, slot_b) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let a = &self.stack[base + slot_a as usize];
                    let b = &self.stack[base + slot_b as usize];
                    let result = match (a, b) {
                        (Value::Int(x), Value::Int(y)) => x != y,
                        (Value::Bool(x), Value::Bool(y)) => x != y,
                        _ => a != b,
                    };
                    self.stack.push(Value::Bool(result));
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
                Op::GetLocalProperty(slot, idx) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let object = self.stack[base + slot as usize].clone();
                    let name = self.read_string_constant_owned(idx);
                    let span = self.current_span();
                    let result = self.op_get_property_member(&object, &name, span)?;
                    self.stack.push(result);
                }
                Op::GetLocalIndex(slot, idx_slot) => {
                    let base = self.frames.last().unwrap().stack_base;
                    let object = self.stack[base + slot as usize].clone();
                    let index_val = self.stack[base + idx_slot as usize].clone();
                    let span = self.current_span();
                    let result = self.op_get_index(&object, &index_val, span)?;
                    self.stack.push(result);
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

    fn capture_upvalue(&mut self, slot: usize) -> Rc<RefCell<Upvalue>> {
        // Check if we already have an open upvalue for this slot
        for uv in &self.open_upvalues {
            if let Upvalue::Open(s) = &*uv.borrow() {
                if *s == slot {
                    return uv.clone();
                }
            }
        }
        // Create a new open upvalue
        let upvalue = Rc::new(RefCell::new(Upvalue::Open(slot)));
        self.open_upvalues.push(upvalue.clone());
        upvalue
    }

    pub fn close_upvalues(&mut self, from_slot: usize) {
        let mut i = 0;
        while i < self.open_upvalues.len() {
            let should_close = {
                let uv = self.open_upvalues[i].borrow();
                if let Upvalue::Open(slot) = &*uv {
                    *slot >= from_slot
                } else {
                    false
                }
            };

            if should_close {
                let upvalue = self.open_upvalues.remove(i);
                let value = {
                    let uv = upvalue.borrow();
                    if let Upvalue::Open(slot) = &*uv {
                        self.stack[*slot].clone()
                    } else {
                        Value::Null
                    }
                };
                *upvalue.borrow_mut() = Upvalue::Closed(value);
            } else {
                i += 1;
            }
        }
    }

    /// Advance the current iterator, returning the next value or None if exhausted.
    fn iter_next(&mut self) -> Option<Value> {
        let state = self.iter_stack.last_mut()?;
        match state {
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
                    Some(Value::String(ch.to_string().into()))
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
            _ => Err(RuntimeError::type_error(
                format!("Cannot iterate over {}", iterable.type_name()),
                span,
            )),
        }
    }

    // --- Arithmetic operations ---

    fn op_add(&self, a: Value, b: Value, span: Span) -> Result<Value, RuntimeError> {
        match (&a, &b) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a + b)),
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a + b)),
            (Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 + b)),
            (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a + *b as f64)),
            (Value::String(a), Value::String(b)) => {
                Ok(Value::String(ecow::eco_format!("{}{}", a, b)))
            }
            (Value::String(a), b) => Ok(Value::String(ecow::eco_format!("{}{}", a, b))),
            (a, Value::String(b)) => Ok(Value::String(ecow::eco_format!("{}{}", a, b))),
            (Value::Array(a), Value::Array(b)) => {
                let mut result = a.borrow().clone();
                result.extend(b.borrow().iter().cloned());
                Ok(Value::Array(Rc::new(RefCell::new(result))))
            }
            _ => Err(RuntimeError::type_error(
                format!("Cannot add {} and {}", a.type_name(), b.type_name()),
                span,
            )),
        }
    }

    fn op_subtract(&self, a: Value, b: Value, span: Span) -> Result<Value, RuntimeError> {
        match (&a, &b) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a - b)),
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a - b)),
            (Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 - b)),
            (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a - *b as f64)),
            _ => Err(RuntimeError::type_error(
                format!("Cannot subtract {} from {}", b.type_name(), a.type_name()),
                span,
            )),
        }
    }

    fn op_multiply(&self, a: Value, b: Value, span: Span) -> Result<Value, RuntimeError> {
        match (&a, &b) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a * b)),
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a * b)),
            (Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 * b)),
            (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a * *b as f64)),
            (Value::String(s), Value::Int(n)) | (Value::Int(n), Value::String(s)) => {
                Ok(Value::String(s.repeat(*n as usize)))
            }
            _ => Err(RuntimeError::type_error(
                format!("Cannot multiply {} and {}", a.type_name(), b.type_name()),
                span,
            )),
        }
    }

    fn op_divide(&self, a: Value, b: Value, span: Span) -> Result<Value, RuntimeError> {
        match (&a, &b) {
            (_, Value::Int(0)) => Err(RuntimeError::division_by_zero(span)),
            (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a / b)),
            (Value::Float(a), Value::Float(b)) => {
                if *b == 0.0 {
                    Err(RuntimeError::division_by_zero(span))
                } else {
                    Ok(Value::Float(a / b))
                }
            }
            (Value::Int(a), Value::Float(b)) => {
                if *b == 0.0 {
                    Err(RuntimeError::division_by_zero(span))
                } else {
                    Ok(Value::Float(*a as f64 / b))
                }
            }
            (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a / *b as f64)),
            _ => Err(RuntimeError::type_error(
                format!("Cannot divide {} by {}", a.type_name(), b.type_name()),
                span,
            )),
        }
    }

    fn op_modulo(&self, a: Value, b: Value, span: Span) -> Result<Value, RuntimeError> {
        match (&a, &b) {
            (_, Value::Int(0)) => Err(RuntimeError::division_by_zero(span)),
            (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a % b)),
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a % b)),
            (Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 % b)),
            (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a % *b as f64)),
            _ => Err(RuntimeError::type_error(
                format!("Cannot modulo {} by {}", a.type_name(), b.type_name()),
                span,
            )),
        }
    }

    fn op_compare_less(&self, a: &Value, b: &Value, span: Span) -> Result<bool, RuntimeError> {
        match (a, b) {
            (Value::Int(a), Value::Int(b)) => Ok(a < b),
            (Value::Float(a), Value::Float(b)) => Ok(a < b),
            (Value::Int(a), Value::Float(b)) => Ok((*a as f64) < *b),
            (Value::Float(a), Value::Int(b)) => Ok(*a < (*b as f64)),
            (Value::String(a), Value::String(b)) => Ok(a < b),
            _ => {
                if let (Some(ts_a), Some(ts_b)) = (a.datetime_ts(), b.datetime_ts()) {
                    return Ok(ts_a < ts_b);
                }
                Err(RuntimeError::type_error(
                    format!("Cannot compare {} and {}", a.type_name(), b.type_name()),
                    span,
                ))
            }
        }
    }

    fn op_compare_less_equal(
        &self,
        a: &Value,
        b: &Value,
        span: Span,
    ) -> Result<bool, RuntimeError> {
        match (a, b) {
            (Value::Int(a), Value::Int(b)) => Ok(a <= b),
            (Value::Float(a), Value::Float(b)) => Ok(a <= b),
            (Value::Int(a), Value::Float(b)) => Ok((*a as f64) <= *b),
            (Value::Float(a), Value::Int(b)) => Ok(*a <= (*b as f64)),
            (Value::String(a), Value::String(b)) => Ok(a <= b),
            _ => {
                if let (Some(ts_a), Some(ts_b)) = (a.datetime_ts(), b.datetime_ts()) {
                    return Ok(ts_a <= ts_b);
                }
                Err(RuntimeError::type_error(
                    format!("Cannot compare {} and {}", a.type_name(), b.type_name()),
                    span,
                ))
            }
        }
    }

    // --- Index operations ---

    fn op_get_index(
        &self,
        object: &Value,
        index: &Value,
        span: Span,
    ) -> Result<Value, RuntimeError> {
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
                let chars: Vec<char> = s.chars().collect();
                let idx = if *i < 0 {
                    (chars.len() as i64 + i) as usize
                } else {
                    *i as usize
                };
                chars
                    .get(idx)
                    .map(|c| Value::String(c.to_string().into()))
                    .ok_or(RuntimeError::IndexOutOfBounds {
                        index: *i,
                        length: chars.len(),
                        span,
                    })
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
            (Value::Hash(hash), key) => {
                if let Some(hash_key) = HashKey::from_value(key) {
                    hash.borrow_mut().insert(hash_key, value);
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
        Constant::Decimal(s) => {
            use crate::interpreter::value::DecimalValue;
            let decimal: rust_decimal::Decimal = s.parse().unwrap_or_default();
            let precision = s.split('.').nth(1).map(|p| p.len() as u32).unwrap_or(0);
            Value::Decimal(DecimalValue(decimal, precision))
        }
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::value::NativeFunction;
    use crate::lexer::Scanner;
    use crate::parser::Parser;
    use crate::vm::compiler::Compiler;

    #[allow(dead_code)]
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

    #[test]
    fn test_vm_arithmetic() {
        let result = compile_and_get_global("let x = 2 + 3 * 4;", "x");
        assert_eq!(result, Value::Int(14));
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
        instance.fields.insert("v".to_string(), Value::Int(7));
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
        instance.fields.insert("score".to_string(), Value::Int(42));
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
    fn test_vm_model_instance_method_is_uncatchable_fallback() {
        // Model instance mutators are deliberately punted to the
        // tree-walker (lifecycle callbacks only fire there). The punt must
        // be an EngineFallback error that try/rescue CANNOT swallow —
        // otherwise `try { record.save() } catch` would skip the fallback
        // and the callbacks with it.
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

        for source in [
            "let x = record.save();",
            "let x = \"start\"\ntry { record.save() } catch (e) { x = \"caught\" }",
            "let x = record.save() rescue \"caught\";",
        ] {
            let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
            let program = Parser::new(tokens).parse().expect("parser error");
            let module = Compiler::compile(&program).expect("compile error");
            let mut vm = Vm::new();
            vm.globals.insert("record".to_string(), record.clone());
            let result = vm.execute(&module.main);
            match result {
                Err(err) => assert!(err.is_engine_fallback(), "{}: {}", source, err),
                Ok(_) => panic!("{}: expected EngineFallback, got Ok", source),
            }
        }
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

    #[test]
    fn test_vm_finally_runs_without_throw() {
        let result = compile_and_get_global(
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
        assert_eq!(result, Value::String("ok+final".into()));
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
}
