//! Bytecode chunk and function prototype types.

use std::sync::Arc;

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
    /// Decimal value stored as string (parsed at runtime)
    Decimal(String),
    /// A compiled function prototype.
    Function(Arc<FunctionProto>),
    /// Pre-computed HashKey list — used for hash literals with all-literal keys
    /// so we don't push each key onto the value stack just to convert it back to
    /// a HashKey at insertion time.
    HashKeys(Arc<Vec<crate::interpreter::value::HashKey>>),
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
            | Op::ForIter(target)
            | Op::ForIterRange(target) => {
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
    pub main: Arc<FunctionProto>,
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- Chunk basics ----------

    #[test]
    fn chunk_new_is_empty() {
        let c = Chunk::new();
        assert!(c.is_empty());
        assert_eq!(c.len(), 0);
        assert!(c.code.is_empty());
        assert!(c.lines.is_empty());
        assert!(c.constants.is_empty());
    }

    #[test]
    fn chunk_default_matches_new() {
        assert!(Chunk::default().is_empty());
    }

    #[test]
    fn emit_returns_offset_and_records_line() {
        let mut c = Chunk::new();
        let off0 = c.emit(Op::Null, 5);
        let off1 = c.emit(Op::Pop, 6);
        assert_eq!(off0, 0);
        assert_eq!(off1, 1);
        assert_eq!(c.code, vec![Op::Null, Op::Pop]);
        assert_eq!(c.lines, vec![5, 6]);
        assert_eq!(c.len(), 2);
        assert!(!c.is_empty());
    }

    // ---------- add_constant ----------

    #[test]
    fn add_constant_returns_index_and_pushes() {
        let mut c = Chunk::new();
        let idx = c.add_constant(Constant::Int(42));
        assert_eq!(idx, 0);
        assert_eq!(c.constants.len(), 1);
        let idx2 = c.add_constant(Constant::Int(7));
        assert_eq!(idx2, 1);
        assert_eq!(c.constants.len(), 2);
    }

    #[test]
    fn add_constant_dedupes_strings() {
        // Identical String constants share a slot — saves space and
        // makes `Op::GetGlobal` lookup hit the same index.
        let mut c = Chunk::new();
        let a = c.add_constant(Constant::String("name".to_string()));
        let b = c.add_constant(Constant::String("name".to_string()));
        assert_eq!(a, b);
        assert_eq!(c.constants.len(), 1);
    }

    #[test]
    fn add_constant_does_not_dedupe_non_string_kinds() {
        // Pin: only String dedup is implemented. Two equal Int constants
        // each get their own slot. (Changing this is a deliberate perf
        // trade-off, so the test will flag it if/when the policy changes.)
        let mut c = Chunk::new();
        let a = c.add_constant(Constant::Int(42));
        let b = c.add_constant(Constant::Int(42));
        assert_ne!(a, b);
        assert_eq!(c.constants.len(), 2);
    }

    #[test]
    fn add_constant_distinct_strings_get_distinct_slots() {
        let mut c = Chunk::new();
        let a = c.add_constant(Constant::String("foo".to_string()));
        let b = c.add_constant(Constant::String("bar".to_string()));
        assert_ne!(a, b);
        assert_eq!(c.constants.len(), 2);
    }

    // ---------- patch_jump ----------

    #[test]
    fn patch_jump_writes_distance_to_jump_target() {
        // emit Jump at offset 0, then 3 more ops, then patch.
        // distance = code.len() (4) - offset (0) - 1 = 3.
        let mut c = Chunk::new();
        let off = c.emit(Op::Jump(0xFFFF), 1);
        c.emit(Op::Null, 1);
        c.emit(Op::Null, 1);
        c.emit(Op::Null, 1);
        c.patch_jump(off);
        assert_eq!(c.code[off], Op::Jump(3));
    }

    #[test]
    fn patch_jump_works_for_each_jump_variant() {
        let variants = [
            Op::Jump(0),
            Op::JumpIfFalse(0),
            Op::JumpIfFalseNoPop(0),
            Op::JumpIfTrueNoPop(0),
            Op::NullishJump(0),
            Op::ForIter(0),
            Op::ForIterRange(0),
        ];
        for variant in variants {
            let mut c = Chunk::new();
            let off = c.emit(variant, 1);
            c.emit(Op::Null, 1);
            c.emit(Op::Null, 1);
            // distance = 3 - 0 - 1 = 2
            c.patch_jump(off);
            // Re-extract the inner u16 — every variant carries a target.
            let target = match c.code[off] {
                Op::Jump(t)
                | Op::JumpIfFalse(t)
                | Op::JumpIfFalseNoPop(t)
                | Op::JumpIfTrueNoPop(t)
                | Op::NullishJump(t)
                | Op::ForIter(t)
                | Op::ForIterRange(t) => t,
                other => panic!("unexpected variant after patch: {other:?}"),
            };
            assert_eq!(target, 2, "wrong target for {variant:?}");
        }
    }

    #[test]
    #[should_panic(expected = "Tried to patch non-jump instruction")]
    fn patch_jump_panics_on_non_jump_op() {
        let mut c = Chunk::new();
        let off = c.emit(Op::Null, 1);
        c.emit(Op::Pop, 1);
        c.patch_jump(off);
    }

    // ---------- FunctionProto ----------

    #[test]
    fn function_proto_new_initialises_empty() {
        let p = FunctionProto::new("hello".to_string());
        assert_eq!(p.name, "hello");
        assert_eq!(p.arity, 0);
        assert_eq!(p.defaults, 0);
        assert!(p.param_names.is_empty());
        assert!(p.default_ops.is_empty());
        assert!(p.upvalue_descriptors.is_empty());
        assert!(p.chunk.is_empty());
        assert!(!p.is_method);
    }
}
