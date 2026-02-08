//! The bytecode virtual machine — stack-based execution engine.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use indexmap::IndexMap;

use crate::error::RuntimeError;
use crate::interpreter::value::{Class, HashKey, Value};
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
        keys: Vec<HashKey>,
        values: Rc<RefCell<IndexMap<HashKey, Value>>>,
        index: usize,
    },
    Range {
        current: i64,
        end: i64,
    },
    String {
        chars: Vec<char>,
        index: usize,
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
    pub failed_handlers: std::collections::HashSet<String>,
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
            failed_handlers: std::collections::HashSet::new(),
        }
    }

    /// Execute a compiled module (top-level script).
    pub fn execute(&mut self, proto: &Rc<FunctionProto>) -> Result<Value, RuntimeError> {
        let closure = Rc::new(VmClosure::new(proto.clone(), Vec::new()));
        self.push(Value::VmClosure(closure.clone()));

        self.frames.push(CallFrame {
            closure,
            ip: 0,
            stack_base: 0,
        });

        self.run()
    }

    /// Run the dispatch loop.
    pub fn run(&mut self) -> Result<Value, RuntimeError> {
        loop {
            let frame_idx = self.frames.len() - 1;
            let frame = &self.frames[frame_idx];
            let ip = frame.ip;

            if ip >= frame.closure.proto.chunk.code.len() {
                return Ok(Value::Null);
            }

            let op = frame.closure.proto.chunk.code[ip];
            let line = frame
                .closure
                .proto
                .chunk
                .lines
                .get(ip)
                .copied()
                .unwrap_or(0);
            let span = Span::new(0, 0, line, 0);

            // Advance IP
            self.frames[frame_idx].ip += 1;

            match op {
                Op::Constant(idx) => {
                    let constant =
                        &self.frames[frame_idx].closure.proto.chunk.constants[idx as usize];
                    let value = constant_to_value(constant);
                    self.push(value);
                }
                Op::Null => self.push(Value::Null),
                Op::True => self.push(Value::Bool(true)),
                Op::False => self.push(Value::Bool(false)),

                Op::Pop => {
                    self.pop();
                }
                Op::Dup => {
                    let val = self.peek(0).clone();
                    self.push(val);
                }

                Op::GetLocal(slot) => {
                    let base = self.frames[frame_idx].stack_base;
                    let val = self.stack[base + slot as usize].clone();
                    self.push(val);
                }
                Op::SetLocal(slot) => {
                    let val = self.peek(0).clone();
                    let base = self.frames[frame_idx].stack_base;
                    self.stack[base + slot as usize] = val;
                }
                Op::GetGlobal(idx) => {
                    let name = self.read_string_constant(frame_idx, idx);
                    match self.globals.get(&name) {
                        Some(val) => self.push(val.clone()),
                        None => {
                            return Err(RuntimeError::undefined_variable(name, span));
                        }
                    }
                }
                Op::SetGlobal(idx) => {
                    let name = self.read_string_constant(frame_idx, idx);
                    let val = self.peek(0).clone();
                    match self.globals.entry(name.clone()) {
                        std::collections::hash_map::Entry::Occupied(mut e) => {
                            e.insert(val);
                        }
                        std::collections::hash_map::Entry::Vacant(_) => {
                            return Err(RuntimeError::undefined_variable(name, span));
                        }
                    }
                }
                Op::DefineGlobal(idx) => {
                    let name = self.read_string_constant(frame_idx, idx);
                    let val = self.pop();
                    self.globals.insert(name, val);
                }

                Op::GetUpvalue(idx) => {
                    let val = {
                        let upvalue = &self.frames[frame_idx].closure.upvalues[idx as usize];
                        let uv = upvalue.borrow();
                        match &*uv {
                            Upvalue::Open(slot) => self.stack[*slot].clone(),
                            Upvalue::Closed(val) => val.clone(),
                        }
                    };
                    self.push(val);
                }
                Op::SetUpvalue(idx) => {
                    let val = self.peek(0).clone();
                    let upvalue = self.frames[frame_idx].closure.upvalues[idx as usize].clone();
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
                    self.pop();
                }

                // --- Arithmetic ---
                Op::Add => {
                    let b = self.pop();
                    let a = self.pop();
                    let result = self.op_add(a, b, span)?;
                    self.push(result);
                }
                Op::Subtract => {
                    let b = self.pop();
                    let a = self.pop();
                    let result = self.op_subtract(a, b, span)?;
                    self.push(result);
                }
                Op::Multiply => {
                    let b = self.pop();
                    let a = self.pop();
                    let result = self.op_multiply(a, b, span)?;
                    self.push(result);
                }
                Op::Divide => {
                    let b = self.pop();
                    let a = self.pop();
                    let result = self.op_divide(a, b, span)?;
                    self.push(result);
                }
                Op::Modulo => {
                    let b = self.pop();
                    let a = self.pop();
                    let result = self.op_modulo(a, b, span)?;
                    self.push(result);
                }
                Op::Negate => {
                    let val = self.pop();
                    match val {
                        Value::Int(n) => self.push(Value::Int(-n)),
                        Value::Float(n) => self.push(Value::Float(-n)),
                        _ => {
                            return Err(RuntimeError::type_error(
                                format!("Cannot negate {}", val.type_name()),
                                span,
                            ));
                        }
                    }
                }

                // --- Comparison ---
                Op::Equal => {
                    let b = self.pop();
                    let a = self.pop();
                    self.push(Value::Bool(a == b));
                }
                Op::NotEqual => {
                    let b = self.pop();
                    let a = self.pop();
                    self.push(Value::Bool(a != b));
                }
                Op::Less => {
                    let b = self.pop();
                    let a = self.pop();
                    let result = self.op_compare_less(&a, &b, span)?;
                    self.push(Value::Bool(result));
                }
                Op::LessEqual => {
                    let b = self.pop();
                    let a = self.pop();
                    let result = self.op_compare_less_equal(&a, &b, span)?;
                    self.push(Value::Bool(result));
                }
                Op::Greater => {
                    let b = self.pop();
                    let a = self.pop();
                    let result = self.op_compare_less(&b, &a, span)?;
                    self.push(Value::Bool(result));
                }
                Op::GreaterEqual => {
                    let b = self.pop();
                    let a = self.pop();
                    let result = self.op_compare_less_equal(&b, &a, span)?;
                    self.push(Value::Bool(result));
                }

                Op::Not => {
                    let val = self.pop();
                    self.push(Value::Bool(!val.is_truthy()));
                }

                // --- Control flow ---
                Op::Jump(offset) => {
                    self.frames[frame_idx].ip += offset as usize;
                }
                Op::JumpIfFalse(offset) => {
                    let val = self.pop();
                    if !val.is_truthy() {
                        self.frames[frame_idx].ip += offset as usize;
                    }
                }
                Op::Loop(offset) => {
                    self.frames[frame_idx].ip -= offset as usize;
                }
                Op::JumpIfFalseNoPop(offset) => {
                    if !self.peek(0).is_truthy() {
                        self.frames[frame_idx].ip += offset as usize;
                    }
                }
                Op::JumpIfTrueNoPop(offset) => {
                    if self.peek(0).is_truthy() {
                        self.frames[frame_idx].ip += offset as usize;
                    }
                }
                Op::NullishJump(offset) => {
                    if !matches!(self.peek(0), Value::Null) {
                        self.frames[frame_idx].ip += offset as usize;
                    }
                }

                // --- Functions ---
                Op::Call(argc) => {
                    self.call_value(argc as usize, span)?;
                }
                Op::Closure(idx) => {
                    let constant =
                        &self.frames[frame_idx].closure.proto.chunk.constants[idx as usize];
                    if let Constant::Function(proto) = constant {
                        let proto = proto.clone();
                        let mut upvalues = Vec::new();

                        for desc in &proto.upvalue_descriptors {
                            if desc.is_local {
                                let slot = self.frames[frame_idx].stack_base + desc.index as usize;
                                let upvalue = self.capture_upvalue(slot);
                                upvalues.push(upvalue);
                            } else {
                                let upvalue = self.frames[frame_idx].closure.upvalues
                                    [desc.index as usize]
                                    .clone();
                                upvalues.push(upvalue);
                            }
                        }

                        let closure = VmClosure::new(proto, upvalues);
                        self.push(Value::VmClosure(Rc::new(closure)));
                    }
                }
                Op::Return => {
                    let result = self.pop();
                    let frame = self.frames.pop().unwrap();

                    // Close any open upvalues in this frame
                    self.close_upvalues(frame.stack_base);

                    // Restore the stack
                    self.stack.truncate(frame.stack_base);

                    if self.frames.is_empty() {
                        return Ok(result);
                    }

                    self.push(result);
                }

                // --- Collections ---
                Op::Array(n) => {
                    let mut elements = Vec::with_capacity(n as usize);
                    for _ in 0..n {
                        elements.push(self.pop());
                    }
                    elements.reverse();
                    self.push(Value::Array(Rc::new(RefCell::new(elements))));
                }
                Op::Hash(n) => {
                    let mut map = IndexMap::new();
                    let mut pairs = Vec::with_capacity(n as usize);
                    for _ in 0..n {
                        let value = self.pop();
                        let key = self.pop();
                        pairs.push((key, value));
                    }
                    pairs.reverse();
                    for (key, value) in pairs {
                        if let Some(hash_key) = HashKey::from_value(&key) {
                            map.insert(hash_key, value);
                        }
                    }
                    self.push(Value::Hash(Rc::new(RefCell::new(map))));
                }
                Op::Range => {
                    let end = self.pop();
                    let start = self.pop();
                    match (&start, &end) {
                        (Value::Int(a), Value::Int(b)) => {
                            let arr: Vec<Value> = (*a..=*b).map(Value::Int).collect();
                            self.push(Value::Array(Rc::new(RefCell::new(arr))));
                        }
                        _ => {
                            return Err(RuntimeError::type_error(
                                "Range requires integer operands",
                                span,
                            ));
                        }
                    }
                }
                Op::GetIndex => {
                    let index = self.pop();
                    let object = self.pop();
                    let result = self.op_get_index(&object, &index, span)?;
                    self.push(result);
                }
                Op::SetIndex => {
                    let value = self.pop();
                    let index = self.pop();
                    let object = self.pop();
                    self.op_set_index(&object, &index, value, span)?;
                    self.push(object);
                }
                Op::BuildString(n) => {
                    let mut parts = Vec::with_capacity(n as usize);
                    for _ in 0..n {
                        parts.push(self.pop());
                    }
                    parts.reverse();
                    let result: String = parts.iter().map(|v| format!("{}", v)).collect();
                    self.push(Value::String(result));
                }
                Op::Spread => {
                    // Spread is handled by the array/hash/call compilation
                    // At runtime, it's a no-op on the value itself
                }

                // --- Properties ---
                Op::GetProperty(idx) => {
                    let name = self.read_string_constant(frame_idx, idx);
                    let object = self.pop();
                    let result = self.op_get_property(&object, &name, span)?;
                    self.push(result);
                }
                Op::SetProperty(idx) => {
                    let name = self.read_string_constant(frame_idx, idx);
                    let value = self.pop();
                    let object = self.peek(0).clone();
                    self.op_set_property(&object, &name, value.clone(), span)?;
                    // Leave object on stack, but push the value as the result
                    self.pop(); // pop object
                    self.push(value);
                }

                // --- Classes ---
                Op::Class(idx) => {
                    let name = self.read_string_constant(frame_idx, idx);
                    let class = Class {
                        name,
                        ..Default::default()
                    };
                    self.push(Value::Class(Rc::new(class)));
                }
                Op::Inherit => {
                    let superclass_val = self.pop();
                    let subclass_val = self.peek(0).clone();
                    self.op_inherit(&subclass_val, &superclass_val, span)?;
                }
                Op::Method(idx) => {
                    let name = self.read_string_constant(frame_idx, idx);
                    let method = self.pop();
                    let class = self.peek(0).clone();
                    self.op_add_method(&class, &name, method, false, span)?;
                }
                Op::StaticMethod(idx) => {
                    let name = self.read_string_constant(frame_idx, idx);
                    let method = self.pop();
                    let class = self.peek(0).clone();
                    self.op_add_method(&class, &name, method, true, span)?;
                }
                Op::New(argc) => {
                    self.op_new(argc as usize, span)?;
                }
                Op::GetThis => {
                    let base = self.frames[frame_idx].stack_base;
                    let this = self.stack[base].clone();
                    self.push(this);
                }
                Op::GetSuper(idx) => {
                    let _name = self.read_string_constant(frame_idx, idx);
                    let _this = self.pop();
                    // Super method resolution happens at runtime
                    self.push(Value::Null); // placeholder
                }
                Op::Field(idx) => {
                    let name = self.read_string_constant(frame_idx, idx);
                    let init_value = self.pop();
                    // Store field initializer on the class (will be used during construction)
                    // For now, this is handled by the constructor compilation
                    let _ = (name, init_value);
                }
                Op::StaticField(idx) => {
                    let name = self.read_string_constant(frame_idx, idx);
                    let value = self.pop();
                    let class = self.peek(0).clone();
                    if let Value::Class(ref cls) = class {
                        cls.static_fields.borrow_mut().insert(name, value);
                    }
                }

                // --- Exceptions ---
                Op::TryBegin(catch_offset, finally_offset) => {
                    let handler = ExceptionHandler {
                        catch_ip: self.frames[frame_idx].ip + catch_offset as usize - 1,
                        finally_ip: self.frames[frame_idx].ip + finally_offset as usize - 1,
                        stack_depth: self.stack.len(),
                        frame_depth: self.frames.len(),
                    };
                    self.exception_handlers.push(handler);
                }
                Op::TryEnd => {
                    self.exception_handlers.pop();
                }
                Op::Throw => {
                    let value = self.pop();
                    self.throw_exception(value, span)?;
                }

                // --- Iterators ---
                Op::GetIter => {
                    let iterable = self.pop();
                    let state = self.create_iterator(iterable, span)?;
                    self.iter_stack.push(state);
                }
                Op::ForIter(exit_offset) => {
                    let next_val = self.iter_next();
                    if let Some(val) = next_val {
                        self.push(val);
                    } else {
                        self.iter_stack.pop();
                        self.frames[frame_idx].ip += exit_offset as usize;
                    }
                }

                // --- I/O ---
                Op::Print(n) => {
                    let mut parts = Vec::with_capacity(n as usize);
                    for _ in 0..n {
                        parts.push(self.pop());
                    }
                    parts.reverse();
                    let output: String = parts
                        .iter()
                        .map(|v| format!("{}", v))
                        .collect::<Vec<_>>()
                        .join(" ");
                    println!("{}", output);
                    self.output.push(output);
                    self.push(Value::Null);
                }

                Op::NamedArg(_) => {
                    // Named arg markers are handled by the Call dispatch
                    // If we encounter one here, just push a marker value
                    // This shouldn't happen in normal execution
                }

                Op::Import(idx) => {
                    let _path = self.read_string_constant(frame_idx, idx);
                    // Import is handled at the higher level (module loader)
                    // The VM just sees the imported globals after module resolution
                }
            }
        }
    }

    // --- Stack operations ---

    #[inline]
    pub fn push(&mut self, value: Value) {
        self.stack.push(value);
    }

    #[inline]
    pub fn pop(&mut self) -> Value {
        self.stack.pop().unwrap_or(Value::Null)
    }

    #[inline]
    pub fn peek(&self, distance: usize) -> &Value {
        &self.stack[self.stack.len() - 1 - distance]
    }

    // --- Helpers ---

    fn read_string_constant(&self, frame_idx: usize, idx: u16) -> String {
        match &self.frames[frame_idx].closure.proto.chunk.constants[idx as usize] {
            Constant::String(s) => s.clone(),
            _ => String::new(),
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
            IterState::Hash {
                keys,
                values: _,
                index,
            } => {
                if *index < keys.len() {
                    let key = keys[*index].to_value();
                    *index += 1;
                    Some(key)
                } else {
                    None
                }
            }
            IterState::Range { current, end } => {
                if *current <= *end {
                    let val = Value::Int(*current);
                    *current += 1;
                    Some(val)
                } else {
                    None
                }
            }
            IterState::String { chars, index } => {
                if *index < chars.len() {
                    let val = Value::String(chars[*index].to_string());
                    *index += 1;
                    Some(val)
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
            Value::Hash(hash) => {
                let keys: Vec<HashKey> = hash.borrow().keys().cloned().collect();
                Ok(IterState::Hash {
                    keys,
                    values: hash,
                    index: 0,
                })
            }
            Value::String(s) => {
                let chars: Vec<char> = s.chars().collect();
                Ok(IterState::String { chars, index: 0 })
            }
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
            (Value::String(a), Value::String(b)) => Ok(Value::String(format!("{}{}", a, b))),
            (Value::String(a), b) => Ok(Value::String(format!("{}{}", a, b))),
            (a, Value::String(b)) => Ok(Value::String(format!("{}{}", a, b))),
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
            _ => Err(RuntimeError::type_error(
                format!("Cannot compare {} and {}", a.type_name(), b.type_name()),
                span,
            )),
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
            _ => Err(RuntimeError::type_error(
                format!("Cannot compare {} and {}", a.type_name(), b.type_name()),
                span,
            )),
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
                if let Some(hash_key) = HashKey::from_value(key) {
                    let hash = hash.borrow();
                    Ok(hash.get(&hash_key).cloned().unwrap_or(Value::Null))
                } else {
                    Err(RuntimeError::type_error(
                        format!("Cannot use {} as hash key", key.type_name()),
                        span,
                    ))
                }
            }
            (Value::String(s), Value::Int(i)) => {
                let chars: Vec<char> = s.chars().collect();
                let idx = if *i < 0 {
                    (chars.len() as i64 + i) as usize
                } else {
                    *i as usize
                };
                chars.get(idx).map(|c| Value::String(c.to_string())).ok_or(
                    RuntimeError::IndexOutOfBounds {
                        index: *i,
                        length: chars.len(),
                        span,
                    },
                )
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

/// Convert a chunk constant to a runtime Value.
fn constant_to_value(constant: &Constant) -> Value {
    match constant {
        Constant::Int(n) => Value::Int(*n),
        Constant::Float(n) => Value::Float(*n),
        Constant::String(s) => Value::String(s.clone()),
        Constant::Bool(b) => Value::Bool(*b),
        Constant::Null => Value::Null,
        Constant::Function(proto) => {
            // A bare function proto becomes a closure with no upvalues
            Value::VmClosure(Rc::new(VmClosure::new(proto.clone(), Vec::new())))
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
        assert_eq!(result, Value::String("hello world".to_string()));
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
}
