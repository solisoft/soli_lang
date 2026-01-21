//! Bytecode chunk containing instructions and constants.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

use crate::bytecode::instruction::OpCode;
use crate::span::Span;

/// A chunk of bytecode containing instructions and metadata.
#[derive(Debug, Clone)]
pub struct Chunk {
    /// The bytecode instructions.
    pub code: Vec<u8>,
    /// The constant pool.
    pub constants: Vec<Constant>,
    /// Line information for debugging (offset -> line number).
    pub lines: Vec<u32>,
    /// Source spans for error reporting.
    pub spans: Vec<Span>,
}

impl Chunk {
    pub fn new() -> Self {
        Self {
            code: Vec::new(),
            constants: Vec::new(),
            lines: Vec::new(),
            spans: Vec::new(),
        }
    }

    /// Write an opcode to the chunk.
    pub fn write_op(&mut self, op: OpCode, line: u32) {
        self.code.push(op as u8);
        self.lines.push(line);
    }

    /// Write a raw byte to the chunk.
    pub fn write_byte(&mut self, byte: u8, line: u32) {
        self.code.push(byte);
        self.lines.push(line);
    }

    /// Write a 16-bit value to the chunk (little-endian).
    pub fn write_u16(&mut self, value: u16, line: u32) {
        self.code.push((value & 0xff) as u8);
        self.lines.push(line);
        self.code.push((value >> 8) as u8);
        self.lines.push(line);
    }

    /// Read a 16-bit value from the chunk at offset.
    pub fn read_u16(&self, offset: usize) -> u16 {
        let lo = self.code[offset] as u16;
        let hi = self.code[offset + 1] as u16;
        lo | (hi << 8)
    }

    /// Add a constant to the pool and return its index.
    pub fn add_constant(&mut self, constant: Constant) -> u16 {
        // Check if constant already exists
        for (i, c) in self.constants.iter().enumerate() {
            if c == &constant {
                return i as u16;
            }
        }
        let index = self.constants.len();
        assert!(index < 65536, "Too many constants in chunk");
        self.constants.push(constant);
        index as u16
    }

    /// Get the current offset in the code.
    pub fn current_offset(&self) -> usize {
        self.code.len()
    }

    /// Patch a jump instruction's offset at the given location.
    pub fn patch_jump(&mut self, offset: usize) {
        // offset points to the first byte of the 16-bit jump offset
        let jump_distance = self.code.len() - offset - 2;
        assert!(jump_distance < 65536, "Jump too large");

        self.code[offset] = (jump_distance & 0xff) as u8;
        self.code[offset + 1] = (jump_distance >> 8) as u8;
    }

    /// Patch a u16 value at the given offset.
    pub fn patch_u16(&mut self, offset: usize, value: u16) {
        self.code[offset] = (value & 0xff) as u8;
        self.code[offset + 1] = (value >> 8) as u8;
    }

    /// Get the line number at a given offset.
    pub fn get_line(&self, offset: usize) -> u32 {
        if offset < self.lines.len() {
            self.lines[offset]
        } else {
            0
        }
    }
}

impl Default for Chunk {
    fn default() -> Self {
        Self::new()
    }
}

/// A constant value in the constant pool.
#[derive(Debug, Clone)]
pub enum Constant {
    /// Integer constant
    Int(i64),
    /// Float constant
    Float(f64),
    /// String constant (also used for identifiers)
    String(String),
    /// Function constant
    Function(Rc<CompiledFunction>),
    /// Class constant (contains methods)
    Class(Rc<CompiledClass>),
    /// Null constant
    Null,
}

impl PartialEq for Constant {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Constant::Int(a), Constant::Int(b)) => a == b,
            (Constant::Float(a), Constant::Float(b)) => a == b,
            (Constant::String(a), Constant::String(b)) => a == b,
            (Constant::Null, Constant::Null) => true,
            // Functions and classes are never equal (each is unique)
            _ => false,
        }
    }
}

/// A compiled function (bytecode representation).
#[derive(Debug, Clone)]
pub struct CompiledFunction {
    /// Function name
    pub name: String,
    /// Number of required parameters (without defaults)
    pub arity: u8,
    /// Total number of parameters (including optional)
    pub full_arity: u8,
    /// Number of upvalues
    pub upvalue_count: usize,
    /// The bytecode chunk
    pub chunk: Chunk,
    /// Whether this is a method
    pub is_method: bool,
    /// Default parameter values as constant indices (None for required params)
    pub default_values: Vec<Option<u16>>,
}

impl CompiledFunction {
    pub fn new(name: String, arity: u8) -> Self {
        Self {
            name,
            arity,
            full_arity: arity,
            upvalue_count: 0,
            chunk: Chunk::new(),
            is_method: false,
            default_values: Vec::new(),
        }
    }

    /// Get the number of default parameters
    pub fn default_count(&self) -> u8 {
        self.full_arity.saturating_sub(self.arity)
    }
}

/// A compiled class.
#[derive(Debug, Clone)]
pub struct CompiledClass {
    /// Class name
    pub name: String,
    /// Methods (name -> function index in constants)
    pub methods: HashMap<String, u16>,
    /// Static methods
    pub static_methods: HashMap<String, u16>,
    /// Constructor function index (if any)
    pub constructor: Option<u16>,
    /// Superclass name (if any)
    pub superclass: Option<String>,
}

impl CompiledClass {
    pub fn new(name: String) -> Self {
        Self {
            name,
            methods: HashMap::new(),
            static_methods: HashMap::new(),
            constructor: None,
            superclass: None,
        }
    }
}

/// Runtime representation of a closure.
#[derive(Debug, Clone)]
pub struct Closure {
    /// The compiled function
    pub function: Rc<CompiledFunction>,
    /// Captured upvalues
    pub upvalues: Vec<Rc<RefCell<Upvalue>>>,
}

impl Closure {
    pub fn new(function: Rc<CompiledFunction>) -> Self {
        let upvalue_count = function.upvalue_count;
        Self {
            function,
            upvalues: Vec::with_capacity(upvalue_count),
        }
    }
}

/// An upvalue (captured variable).
#[derive(Debug, Clone)]
pub enum Upvalue {
    /// Open upvalue: points to a stack slot
    Open(usize),
    /// Closed upvalue: contains the value
    Closed(VMValue),
}

impl Upvalue {
    pub fn is_open(&self) -> bool {
        matches!(self, Upvalue::Open(_))
    }
}

/// Runtime value for the bytecode VM.
/// Similar to interpreter::Value but optimized for VM execution.
#[derive(Debug, Clone)]
pub enum VMValue {
    /// Integer value
    Int(i64),
    /// Floating point value
    Float(f64),
    /// String value
    String(Rc<String>),
    /// Boolean value
    Bool(bool),
    /// Null value
    Null,
    /// Array value
    Array(Rc<RefCell<Vec<VMValue>>>),
    /// Hash/Map value
    Hash(Rc<RefCell<Vec<(VMValue, VMValue)>>>),
    /// Closure (function + upvalues)
    Closure(Rc<RefCell<Closure>>),
    /// Native function reference
    NativeFunction(u16),
    /// Class definition
    Class(Rc<RefCell<VMClass>>),
    /// Class instance
    Instance(Rc<RefCell<VMInstance>>),
    /// Bound method (instance + closure)
    BoundMethod(Rc<RefCell<VMInstance>>, Rc<RefCell<Closure>>),
    /// Bound native method (instance + class name + method name)
    BoundNativeMethod(Rc<RefCell<VMInstance>>, String, String),
    /// Iterator state
    Iterator(Rc<RefCell<VMIterator>>),
}

impl VMValue {
    pub fn type_name(&self) -> &'static str {
        match self {
            VMValue::Int(_) => "Int",
            VMValue::Float(_) => "Float",
            VMValue::String(_) => "String",
            VMValue::Bool(_) => "Bool",
            VMValue::Null => "Null",
            VMValue::Array(_) => "Array",
            VMValue::Hash(_) => "Hash",
            VMValue::Closure(_) => "Function",
            VMValue::NativeFunction(_) => "Function",
            VMValue::Class(_) => "Class",
            VMValue::Instance(_) => "Instance",
            VMValue::BoundMethod(_, _) => "Method",
            VMValue::BoundNativeMethod(_, _, _) => "Method",
            VMValue::Iterator(_) => "Iterator",
        }
    }

    pub fn is_truthy(&self) -> bool {
        match self {
            VMValue::Bool(b) => *b,
            VMValue::Null => false,
            VMValue::Int(0) => false,
            VMValue::String(s) if s.is_empty() => false,
            VMValue::Array(arr) if arr.borrow().is_empty() => false,
            VMValue::Hash(hash) if hash.borrow().is_empty() => false,
            _ => true,
        }
    }

    pub fn is_hashable(&self) -> bool {
        matches!(
            self,
            VMValue::Int(_)
                | VMValue::Float(_)
                | VMValue::String(_)
                | VMValue::Bool(_)
                | VMValue::Null
        )
    }

    pub fn hash_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (VMValue::Int(a), VMValue::Int(b)) => a == b,
            (VMValue::Float(a), VMValue::Float(b)) => a == b,
            (VMValue::Int(a), VMValue::Float(b)) => (*a as f64) == *b,
            (VMValue::Float(a), VMValue::Int(b)) => *a == (*b as f64),
            (VMValue::String(a), VMValue::String(b)) => a == b,
            (VMValue::Bool(a), VMValue::Bool(b)) => a == b,
            (VMValue::Null, VMValue::Null) => true,
            _ => false,
        }
    }
}

impl PartialEq for VMValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (VMValue::Int(a), VMValue::Int(b)) => a == b,
            (VMValue::Float(a), VMValue::Float(b)) => a == b,
            (VMValue::Int(a), VMValue::Float(b)) => (*a as f64) == *b,
            (VMValue::Float(a), VMValue::Int(b)) => *a == (*b as f64),
            (VMValue::String(a), VMValue::String(b)) => a == b,
            (VMValue::Bool(a), VMValue::Bool(b)) => a == b,
            (VMValue::Null, VMValue::Null) => true,
            (VMValue::Array(a), VMValue::Array(b)) => Rc::ptr_eq(a, b),
            (VMValue::Hash(a), VMValue::Hash(b)) => Rc::ptr_eq(a, b),
            (VMValue::Instance(a), VMValue::Instance(b)) => Rc::ptr_eq(a, b),
            (VMValue::BoundNativeMethod(a, _, _), VMValue::BoundNativeMethod(b, _, _)) => {
                Rc::ptr_eq(a, b)
            }
            _ => false,
        }
    }
}

impl fmt::Display for VMValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VMValue::Int(n) => write!(f, "{}", n),
            VMValue::Float(n) => write!(f, "{}", n),
            VMValue::String(s) => write!(f, "{}", s),
            VMValue::Bool(b) => write!(f, "{}", b),
            VMValue::Null => write!(f, "null"),
            VMValue::Array(arr) => {
                write!(f, "[")?;
                let arr = arr.borrow();
                for (i, val) in arr.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", val)?;
                }
                write!(f, "]")
            }
            VMValue::Hash(hash) => {
                write!(f, "{{")?;
                let hash = hash.borrow();
                for (i, (key, val)) in hash.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{} => {}", key, val)?;
                }
                write!(f, "}}")
            }
            VMValue::Closure(closure) => {
                write!(f, "<fn {}>", closure.borrow().function.name)
            }
            VMValue::NativeFunction(idx) => write!(f, "<native fn {}>", idx),
            VMValue::Class(class) => write!(f, "<class {}>", class.borrow().name),
            VMValue::Instance(inst) => {
                write!(f, "<{} instance>", inst.borrow().class.borrow().name)
            }
            VMValue::BoundMethod(inst, closure) => {
                write!(
                    f,
                    "<bound method {} of {}>",
                    closure.borrow().function.name,
                    inst.borrow().class.borrow().name
                )
            }
            VMValue::BoundNativeMethod(inst, _, method_name) => {
                write!(
                    f,
                    "<bound native method {} of {}>",
                    method_name,
                    inst.borrow().class.borrow().name
                )
            }
            VMValue::Iterator(_) => write!(f, "<iterator>"),
        }
    }
}

/// Runtime class representation.
#[derive(Debug, Clone)]
pub struct VMClass {
    pub name: String,
    pub methods: HashMap<String, Rc<RefCell<Closure>>>,
    pub static_methods: HashMap<String, Rc<RefCell<Closure>>>,
    pub constructor: Option<Rc<RefCell<Closure>>>,
    pub superclass: Option<Rc<RefCell<VMClass>>>,
    /// Native method handlers (class name -> method name -> handler)
    pub native_methods: HashMap<String, fn(&VMInstance) -> Result<VMValue, String>>,
}

impl VMClass {
    pub fn new(name: String) -> Self {
        Self {
            name,
            methods: HashMap::new(),
            static_methods: HashMap::new(),
            constructor: None,
            superclass: None,
            native_methods: HashMap::new(),
        }
    }

    pub fn find_method(&self, name: &str) -> Option<Rc<RefCell<Closure>>> {
        if let Some(method) = self.methods.get(name) {
            return Some(method.clone());
        }
        if let Some(ref superclass) = self.superclass {
            return superclass.borrow().find_method(name);
        }
        None
    }

    pub fn find_native_method(
        &self,
        name: &str,
    ) -> Option<fn(&VMInstance) -> Result<VMValue, String>> {
        if let Some(method) = self.native_methods.get(name) {
            return Some(method.clone());
        }
        if let Some(ref superclass) = self.superclass {
            return superclass.borrow().find_native_method(name);
        }
        None
    }
}

/// Runtime instance representation.
#[derive(Debug, Clone)]
pub struct VMInstance {
    pub class: Rc<RefCell<VMClass>>,
    pub fields: HashMap<String, VMValue>,
}

impl VMInstance {
    pub fn new(class: Rc<RefCell<VMClass>>) -> Self {
        Self {
            class,
            fields: HashMap::new(),
        }
    }

    pub fn with_field(class: Rc<RefCell<VMClass>>, name: String, value: VMValue) -> Self {
        let mut fields = HashMap::new();
        fields.insert(name, value);
        Self { class, fields }
    }

    pub fn get(&self, name: &str) -> Option<VMValue> {
        self.fields.get(name).cloned()
    }

    pub fn set(&mut self, name: String, value: VMValue) {
        self.fields.insert(name, value);
    }
}

/// Iterator state for for-loops.
#[derive(Debug, Clone)]
pub enum VMIterator {
    Array {
        array: Rc<RefCell<Vec<VMValue>>>,
        index: usize,
    },
    Hash {
        pairs: Vec<(VMValue, VMValue)>,
        index: usize,
    },
    Range {
        current: i64,
        end: i64,
        step: i64,
    },
    String {
        chars: Vec<char>,
        index: usize,
    },
}

impl VMIterator {
    pub fn next(&mut self) -> Option<VMValue> {
        match self {
            VMIterator::Array { array, index } => {
                let arr = array.borrow();
                if *index < arr.len() {
                    let value = arr[*index].clone();
                    *index += 1;
                    Some(value)
                } else {
                    None
                }
            }
            VMIterator::Hash { pairs, index } => {
                if *index < pairs.len() {
                    // Return key for iteration
                    let (key, _) = &pairs[*index];
                    let value = key.clone();
                    *index += 1;
                    Some(value)
                } else {
                    None
                }
            }
            VMIterator::Range { current, end, step } => {
                if (*step > 0 && *current < *end) || (*step < 0 && *current > *end) {
                    let value = VMValue::Int(*current);
                    *current += *step;
                    Some(value)
                } else {
                    None
                }
            }
            VMIterator::String { chars, index } => {
                if *index < chars.len() {
                    let value = VMValue::String(Rc::new(chars[*index].to_string()));
                    *index += 1;
                    Some(value)
                } else {
                    None
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_basics() {
        let mut chunk = Chunk::new();
        chunk.write_op(OpCode::Constant, 1);
        chunk.write_u16(0, 1);
        chunk.write_op(OpCode::Return, 1);

        assert_eq!(chunk.code.len(), 4);
        assert_eq!(chunk.code[0], OpCode::Constant as u8);
        assert_eq!(chunk.read_u16(1), 0);
        assert_eq!(chunk.code[3], OpCode::Return as u8);
    }

    #[test]
    fn test_constant_pool() {
        let mut chunk = Chunk::new();
        let idx1 = chunk.add_constant(Constant::Int(42));
        let idx2 = chunk.add_constant(Constant::Int(42)); // Should return same index
        let idx3 = chunk.add_constant(Constant::String("hello".to_string()));

        assert_eq!(idx1, 0);
        assert_eq!(idx2, 0); // Deduplicated
        assert_eq!(idx3, 1);
    }

    #[test]
    fn test_jump_patching() {
        let mut chunk = Chunk::new();
        chunk.write_op(OpCode::JumpIfFalse, 1);
        let jump_offset = chunk.current_offset();
        chunk.write_u16(0xFFFF, 1); // Placeholder

        // Write some code
        chunk.write_op(OpCode::Pop, 1);
        chunk.write_op(OpCode::Pop, 1);

        // Patch the jump
        chunk.patch_jump(jump_offset);

        // Should jump over 2 Pop instructions (2 bytes)
        assert_eq!(chunk.read_u16(jump_offset), 2);
    }
}
