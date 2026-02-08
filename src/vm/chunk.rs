//! Bytecode chunk and function prototype types.

use std::rc::Rc;

use super::opcode::Op;
use super::upvalue::UpvalueDescriptor;

/// A constant value stored in a chunk's constant pool.
#[derive(Debug, Clone)]
pub enum Constant {
    Int(i64),
    Float(f64),
    String(String),
    Bool(bool),
    Null,
    /// A compiled function prototype.
    Function(Rc<FunctionProto>),
}

/// A compiled function (or top-level script).
#[derive(Debug, Clone)]
pub struct FunctionProto {
    /// Function name (empty string for top-level script).
    pub name: String,
    /// Number of parameters.
    pub arity: u8,
    /// Number of parameters with default values.
    pub defaults: u8,
    /// Parameter names (for named argument resolution).
    pub param_names: Vec<String>,
    /// Default value constant indices (for parameters with defaults).
    /// Each entry is a list of opcodes that compute the default value.
    pub default_ops: Vec<Vec<Op>>,
    /// The bytecode instructions.
    pub chunk: Chunk,
    /// Upvalue descriptors for creating closures.
    pub upvalue_descriptors: Vec<UpvalueDescriptor>,
    /// Whether this is a method (has `this` in slot 0).
    pub is_method: bool,
}

impl FunctionProto {
    pub fn new(name: String) -> Self {
        Self {
            name,
            arity: 0,
            defaults: 0,
            param_names: Vec::new(),
            default_ops: Vec::new(),
            chunk: Chunk::new(),
            upvalue_descriptors: Vec::new(),
            is_method: false,
        }
    }
}

/// A chunk of bytecode: instructions + constant pool + line info.
#[derive(Debug, Clone)]
pub struct Chunk {
    /// The bytecode instructions.
    pub code: Vec<Op>,
    /// Source line numbers, parallel to `code`.
    pub lines: Vec<usize>,
    /// Constant pool.
    pub constants: Vec<Constant>,
}

impl Chunk {
    pub fn new() -> Self {
        Self {
            code: Vec::new(),
            lines: Vec::new(),
            constants: Vec::new(),
        }
    }

    /// Emit an instruction and record its source line.
    pub fn emit(&mut self, op: Op, line: usize) -> usize {
        let offset = self.code.len();
        self.code.push(op);
        self.lines.push(line);
        offset
    }

    /// Add a constant to the pool and return its index.
    pub fn add_constant(&mut self, constant: Constant) -> u16 {
        // Check for duplicate string constants
        if let Constant::String(ref s) = constant {
            for (i, c) in self.constants.iter().enumerate() {
                if let Constant::String(existing) = c {
                    if existing == s {
                        return i as u16;
                    }
                }
            }
        }
        let idx = self.constants.len();
        self.constants.push(constant);
        idx as u16
    }

    /// Get the current offset (next instruction index).
    pub fn len(&self) -> usize {
        self.code.len()
    }

    pub fn is_empty(&self) -> bool {
        self.code.is_empty()
    }

    /// Patch a jump instruction at `offset` with the actual jump distance.
    pub fn patch_jump(&mut self, offset: usize) {
        let jump = self.code.len() - offset - 1;
        let jump = jump as u16;
        match &mut self.code[offset] {
            Op::Jump(target)
            | Op::JumpIfFalse(target)
            | Op::JumpIfFalseNoPop(target)
            | Op::JumpIfTrueNoPop(target)
            | Op::NullishJump(target)
            | Op::ForIter(target) => {
                *target = jump;
            }
            _ => panic!("Tried to patch non-jump instruction at offset {}", offset),
        }
    }
}

impl Default for Chunk {
    fn default() -> Self {
        Self::new()
    }
}

/// A compiled module: the top-level script function.
#[derive(Debug, Clone)]
pub struct CompiledModule {
    pub main: Rc<FunctionProto>,
}
