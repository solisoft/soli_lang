//! The bytecode virtual machine — stack-based execution engine.

use ahash::AHashMap as HashMap;
use std::cell::RefCell;
use std::rc::Rc;

use crate::error::RuntimeError;
use crate::interpreter::value::{Class, HashKey, HashPairs, Value};
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
        values: Rc<RefCell<HashPairs>>,
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
    pub failed_handlers: ahash::AHashSet<String>,
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

    /// Get the current source span (only call on error paths).
    #[cold]
    fn current_span(&self) -> Span {
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

    /// Read a string constant as &str without cloning (for use within a scoped borrow).
    #[inline]
    fn read_string_constant_owned(&self, idx: u16) -> String {
        let frame = self.frames.last().unwrap();
        match &frame.closure.proto.chunk.constants[idx as usize] {
            Constant::String(s) => s.clone(),
            _ => String::new(),
        }
    }

    /// Run the dispatch loop.
    pub fn run(&mut self) -> Result<Value, RuntimeError> {
        loop {
            // Fetch opcode and advance IP in a scoped borrow
            let op = {
                let frame = self.frames.last_mut().unwrap();
                let ip = frame.ip;
                if ip >= frame.closure.proto.chunk.code.len() {
                    return Ok(Value::Null);
                }
                let op = frame.closure.proto.chunk.code[ip];
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
                            Constant::String(s) => s.as_str(),
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
                    let val = self.stack.last().unwrap().clone();
                    // Avoid cloning the string constant for lookup
                    let found = {
                        let frame = self.frames.last().unwrap();
                        let name = match &frame.closure.proto.chunk.constants[idx as usize] {
                            Constant::String(s) => s.as_str(),
                            _ => "",
                        };
                        if let Some(entry) = self.globals.get_mut(name) {
                            *entry = val;
                            true
                        } else {
                            false
                        }
                    };
                    if !found {
                        let name = self.read_string_constant_owned(idx);
                        return Err(RuntimeError::undefined_variable(name, self.current_span()));
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
                            let mut s = String::with_capacity(x.len() + y.len());
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
                        _ => a == b,
                    };
                    self.stack.push(Value::Bool(result));
                }
                Op::NotEqual => {
                    let (a, b) = self.pop2();
                    let result = match (&a, &b) {
                        (Value::Int(x), Value::Int(y)) => x != y,
                        (Value::Bool(x), Value::Bool(y)) => x != y,
                        _ => a != b,
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
                    let span = self.current_span();
                    self.call_value(argc as usize, span)?;
                }
                Op::CallGlobal(name_idx, argc) => {
                    // Combined GetGlobal + Call: lookup global, push, and call in one step
                    let val = {
                        let frame = self.frames.last().unwrap();
                        let name = match &frame.closure.proto.chunk.constants[name_idx as usize] {
                            Constant::String(s) => s.as_str(),
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
                            Constant::String(s) => s.as_str() as *const str,
                            _ => "" as *const str,
                        }
                    };
                    // SAFETY: name points into the constant pool which is alive for
                    // the entire execution of this frame. We never mutate constants.
                    let name: &str = unsafe { &*name };

                    // Fast path: dispatch on receiver type without cloning
                    match &self.stack[receiver_idx] {
                        Value::String(_) => {
                            // Ultra-fast path: inline common zero-arg string methods
                            let result = if argc == 0 {
                                let s: &str = match &self.stack[receiver_idx] {
                                    Value::String(s) => s.as_str(),
                                    _ => unreachable!(),
                                };
                                match name {
                                    "len" | "length" => Some(Value::Int(s.len() as i64)),
                                    "empty?" => Some(Value::Bool(s.is_empty())),
                                    "bytesize" => Some(Value::Int(s.len() as i64)),
                                    "upcase" | "uppercase" => Some(Value::String(s.to_uppercase())),
                                    "downcase" | "lowercase" => {
                                        Some(Value::String(s.to_lowercase()))
                                    }
                                    "trim" => Some(Value::String(s.trim().to_string())),
                                    "reverse" => Some(Value::String(s.chars().rev().collect())),
                                    "nil?" => Some(Value::Bool(false)),
                                    "class" => Some(Value::String("string".to_string())),
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
                                        Value::String(s) => s.as_str(),
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
                            // Slow path: use GetProperty + Call semantics
                            let span = self.current_span();
                            let object = self.stack[receiver_idx].clone();
                            let method_val = self.op_get_property(&object, name, span)?;
                            self.stack[receiver_idx] = method_val;
                            self.call_value(argc, span)?;
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
                                    "class" => Some(Value::String("array".to_string())),
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
                                let args = &self.stack[receiver_idx + 1..receiver_idx + 1 + argc];
                                let span = self.current_span();
                                let result = self.vm_call_array_method(&arr, name, args, span)?;
                                self.stack.truncate(receiver_idx);
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
                                    "class" => Some(Value::String("hash".to_string())),
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
                                let args = &self.stack[receiver_idx + 1..receiver_idx + 1 + argc];
                                let span = self.current_span();
                                let result = self.vm_call_hash_method(&hash, name, args, span)?;
                                self.stack.truncate(receiver_idx);
                                self.stack.push(result);
                            }
                        }
                        _ => {
                            let span = self.current_span();
                            let object = self.stack[receiver_idx].clone();
                            let method_val = self.op_get_property(&object, name, span)?;
                            self.stack[receiver_idx] = method_val;
                            self.call_value(argc, span)?;
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
                                super::method_table::string_method_zero_arg(s.as_str(), method_id)
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

                    // Medium path: known method ID with args — use string dispatch
                    // (still avoids the name clone since we borrow from constants)
                    let name: *const str = {
                        let frame = self.frames.last().unwrap();
                        match &frame.closure.proto.chunk.constants[name_idx as usize] {
                            Constant::String(s) => s.as_str() as *const str,
                            _ => "" as *const str,
                        }
                    };
                    let name: &str = unsafe { &*name };

                    match &self.stack[receiver_idx] {
                        Value::String(_) => {
                            let result = {
                                let s: &str = match &self.stack[receiver_idx] {
                                    Value::String(s) => s.as_str(),
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
                            let args = &self.stack[receiver_idx + 1..receiver_idx + 1 + argc];
                            let span = self.current_span();
                            let result = self.vm_call_array_method(&arr, name, args, span)?;
                            self.stack.truncate(receiver_idx);
                            self.stack.push(result);
                        }
                        Value::Hash(_) => {
                            let hash = match &self.stack[receiver_idx] {
                                Value::Hash(h) => h.clone(),
                                _ => unreachable!(),
                            };
                            let args = &self.stack[receiver_idx + 1..receiver_idx + 1 + argc];
                            let span = self.current_span();
                            let result = self.vm_call_hash_method(&hash, name, args, span)?;
                            self.stack.truncate(receiver_idx);
                            self.stack.push(result);
                        }
                        _ => {
                            // Class instances, closures: fall back to property dispatch
                            let span = self.current_span();
                            let object = self.stack[receiver_idx].clone();
                            let method_val = self.op_get_property(&object, name, span)?;
                            self.stack[receiver_idx] = method_val;
                            self.call_value(argc, span)?;
                        }
                    }
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

                    if self.frames.is_empty() {
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
                Op::Hash(n) => {
                    let mut map = HashPairs::default();
                    let mut pairs = Vec::with_capacity(n as usize);
                    for _ in 0..n {
                        let value = self.stack.pop().unwrap();
                        let key = self.stack.pop().unwrap();
                        pairs.push((key, value));
                    }
                    pairs.reverse();
                    for (key, value) in pairs {
                        if let Some(hash_key) = HashKey::from_value(&key) {
                            map.insert(hash_key, value);
                        }
                    }
                    self.stack.push(Value::Hash(Rc::new(RefCell::new(map))));
                }
                Op::Range => {
                    let (start, end) = self.pop2();
                    match (&start, &end) {
                        (Value::Int(a), Value::Int(b)) => {
                            let arr: Vec<Value> = (*a..=*b).map(Value::Int).collect();
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
                    let mut result = String::new();
                    for i in start..len {
                        match &self.stack[i] {
                            Value::String(s) => result.push_str(s),
                            Value::Int(i) => {
                                use std::fmt::Write;
                                let _ = write!(result, "{}", i);
                            }
                            other => result.push_str(&format!("{}", other)),
                        }
                    }
                    self.stack.truncate(start);
                    self.stack.push(Value::String(result));
                }
                Op::Spread => {
                    // Spread is handled by the array/hash/call compilation
                }

                // --- Properties ---
                Op::GetProperty(idx) => {
                    let name = self.read_string_constant_owned(idx);
                    let object = self.stack.pop().unwrap();
                    let span = self.current_span();
                    let result = self.op_get_property(&object, &name, span)?;
                    self.stack.push(result);
                }
                Op::SetProperty(idx) => {
                    let name = self.read_string_constant_owned(idx);
                    let value = self.stack.pop().unwrap();
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
                    let frame = self.frames.last().unwrap();
                    let handler = ExceptionHandler {
                        catch_ip: frame.ip + catch_offset as usize - 1,
                        finally_ip: frame.ip + finally_offset as usize - 1,
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
                        if *current <= *end {
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
                Op::NamedArg(_) => {}

                Op::Import(idx) => {
                    let _path = self.read_string_constant_owned(idx);
                }

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
                            self.stack.push(Value::String(json_str));
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
        // Safety: VM maintains stack invariants — pop is only called when stack is non-empty
        unsafe {
            let new_len = self.stack.len() - 1;
            self.stack.set_len(new_len);
            std::ptr::read(self.stack.as_ptr().add(new_len))
        }
    }

    #[inline(always)]
    pub fn peek(&self, distance: usize) -> &Value {
        unsafe { self.stack.get_unchecked(self.stack.len() - 1 - distance) }
    }

    /// Pop two values from the stack (b first, then a).
    #[inline(always)]
    fn pop2(&mut self) -> (Value, Value) {
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
