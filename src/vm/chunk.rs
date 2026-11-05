//! Bytecode chunk and function prototype types.

use std::sync::Arc;

use super::opcode::Op;
use super::upvalue::UpvalueDescriptor;

/// A constant value stored in a chunk's constant pool.
#[derive(Debug, Clone)]
pub enum Constant {
    Int(i64),
    Float(f64),
    /// Shares its buffer with every runtime `Value::String` loaded from it
    /// (zero-copy literal loading — the load is a refcount bump).
    String(crate::interpreter::value::SoliStr),
    Bool(bool),
    Null,
    /// A Decimal literal, parsed when it was compiled: loading it is a copy,
    /// in the fast tier, as for an `Int`.
    Decimal(crate::interpreter::value::DecimalValue),
    /// A compiled function prototype.
    Function(Arc<FunctionProto>),
    /// Pre-computed HashKey list — used for hash literals with all-literal keys
    /// so we don't push each key onto the value stack just to convert it back to
    /// a HashKey at insertion time.
    HashKeys(Arc<Vec<crate::interpreter::value::HashKey>>),
    /// Argument labels for a `CallNamed`, one entry per argument slot in source
    /// order: `Some(name)` for `f(port: 3000)`, `None` for a positional (or
    /// block) argument. `CallNamed` reads it to split the evaluated arguments
    /// back into positional and named groups before binding them to the callee.
    ArgNames(Arc<Vec<Option<crate::interpreter::value::SoliStr>>>),
    /// The elements of an array literal made only of literals (`["draft",
    /// "sent"]`), read by `Op::LiteralArray` and `Op::LiteralListIncludes`
    /// and never loaded as a value itself.
    LiteralList(Arc<super::literal_list::LiteralList>),
}

/// The `Constant::ArgNames` label of a block argument in a labelled method
/// call (`obj.m(k: v) do … end`). No identifier can be spelled `&`, so it never
/// collides with a parameter name.
pub const BLOCK_LABEL: &str = "&";

/// A compiled function (or top-level script).
#[derive(Debug, Clone)]
pub struct FunctionProto {
    /// Function name (empty string for top-level script).
    pub name: String,
    /// Number of parameters.
    pub arity: u8,
    /// Number of parameters with default values.
    pub defaults: u8,
    /// Bit `i` is set when parameter `i` declares a default. `defaults` only
    /// counts them, which is enough for a positional call (required parameters
    /// lead), but a named call can leave any slot unfilled and must be able to
    /// tell "omitted, has a default" from "omitted, required". Saturates at 64
    /// parameters — beyond that a parameter reads as defaulted, which errs
    /// toward running the prologue rather than a spurious arity error.
    pub defaults_mask: u64,
    /// Parameter names (for named argument resolution).
    pub param_names: Vec<String>,
    /// The bytecode instructions.
    pub chunk: Chunk,
    /// Upvalue descriptors for creating closures.
    pub upvalue_descriptors: Vec<UpvalueDescriptor>,
    /// Whether this is a method (has `this` in slot 0).
    pub is_method: bool,
    /// The file this function was compiled from, for `Op::CoverLine`. Set for
    /// methods compiled from the tree-walker's AST (actions, their helpers)
    /// and inherited by the closures compiled inside them.
    pub source_path: Option<std::sync::Arc<std::path::PathBuf>>,
    /// Machine code for this function, when it is a top-level def typed
    /// Int/Float/Bool end to end (see `crate::native`). Tried first on each
    /// call; a declined call runs the bytecode as usual.
    pub kernel: Option<crate::native::KernelRef>,
    /// The declared return type, checked on return when the VM runs a script
    /// (`Vm::check_return_types`), as the tree-walker checks it.
    /// Set with [`FunctionProto::set_return_type`], which lowers it into
    /// `return_check`.
    pub return_type: Option<crate::ast::TypeAnnotation>,
    /// `return_type` lowered once, so a checked return reads no type name.
    pub return_check: Option<crate::interpreter::value::TypeCheck>,
    /// Where the function is declared: the span of a return-type error.
    pub decl_span: crate::span::Span,
    /// The qualified class its code is written in (`Billing::PayInvoice`), so
    /// a name it reads unqualified can be its namespace's (`Billing::Invoice`).
    pub namespace: Option<std::sync::Arc<str>>,
    /// The index of a `&block` parameter, when the function declares one. A
    /// labelled method call binds its block there and no argument to it,
    /// as the tree-walker's `call_value_with_named` does.
    pub block_param: Option<u16>,
}

impl FunctionProto {
    pub fn new(name: String) -> Self {
        Self {
            name,
            arity: 0,
            defaults: 0,
            defaults_mask: 0,
            param_names: Vec::new(),
            chunk: Chunk::new(),
            upvalue_descriptors: Vec::new(),
            is_method: false,
            source_path: None,
            kernel: None,
            return_type: None,
            return_check: None,
            decl_span: crate::span::Span::default(),
            namespace: None,
            block_param: None,
        }
    }

    /// Declare the return type, and the check each return runs against it
    /// when the VM checks return types.
    pub fn set_return_type(&mut self, return_type: Option<crate::ast::TypeAnnotation>) {
        self.return_check = return_type
            .as_ref()
            .map(crate::interpreter::value::TypeCheck::lower);
        self.return_type = return_type;
    }
}

/// A chunk of bytecode: instructions + constant pool + line info.
#[derive(Debug, Clone)]
pub struct Chunk {
    /// The bytecode instructions.
    pub code: Vec<Op>,
    /// Source line numbers, parallel to `code`.
    pub lines: Vec<usize>,
    /// Source columns, parallel to `code` (0 where unknown).
    pub columns: Vec<u32>,
    /// Constant pool.
    pub constants: Vec<Constant>,
    /// Set when a jump, loop or handler offset did not fit its 16-bit
    /// operand ([`Chunk::jump_operand`]). The compiler refuses such a chunk
    /// (`Compiler::check_jump_offsets`), so the code runs on the tree-walker
    /// instead of jumping to a truncated target.
    pub offset_overflow: bool,
}

/// The longest forward or backward distance a branch operand may hold.
///
/// Operands are `u16`, and the peephole adds up to 2 to a forward offset when
/// it fuses the instructions before a jump into it (`JumpIfLocalTruthy(slot,
/// offset + 2)`), so the limit leaves that room. `compact_nops` only shortens
/// offsets.
pub const MAX_JUMP_OFFSET: usize = u16::MAX as usize - 2;

impl Chunk {
    pub fn new() -> Self {
        Self {
            code: Vec::new(),
            lines: Vec::new(),
            columns: Vec::new(),
            constants: Vec::new(),
            offset_overflow: false,
        }
    }

    /// `distance` as a branch operand. A distance past [`MAX_JUMP_OFFSET`]
    /// does not fit: it marks the chunk (`offset_overflow`), which the
    /// compiler then refuses, and gives 0. Truncating it instead, as every
    /// offset was up to 2.29.3, sent a body of more than 65,535 instructions
    /// to the wrong instruction, mid-expression, where the VM's unchecked
    /// stack reads (`peek_two!`) read past the stack.
    pub fn jump_operand(&mut self, distance: usize) -> u16 {
        if distance > MAX_JUMP_OFFSET {
            self.offset_overflow = true;
            return 0;
        }
        distance as u16
    }

    /// Emit an instruction and record its source line.
    pub fn emit(&mut self, op: Op, line: usize) -> usize {
        self.emit_at(op, line, 0)
    }

    /// Emit an instruction and record its source line and column.
    pub fn emit_at(&mut self, op: Op, line: usize, column: u32) -> usize {
        let offset = self.code.len();
        self.code.push(op);
        self.lines.push(line);
        self.columns.push(column);
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
        let jump = self.jump_operand(jump);
        match &mut self.code[offset] {
            // Single-target forward branches. `CatchMatch`, `RescueJump`
            // and `TryBegin` are patched by hand at their emit sites (the
            // last carries two targets), so they are deliberately absent.
            //
            // This match is the one of the three branch-op lists that fails
            // LOUDLY — an unlisted opcode panics here rather than silently
            // keeping a wrong offset. That is how the missing `JumpIfNull`
            // announced itself the moment safe navigation started emitting
            // one, instead of mis-jumping at run time.
            Op::Jump(target)
            | Op::JumpIfFalse(target)
            | Op::JumpIfTrue(target)
            | Op::JumpIfFalseNoPop(target)
            | Op::JumpIfTrueNoPop(target)
            | Op::JumpIfNull(target)
            | Op::JumpIfNotNull(target)
            | Op::NullishJump(target)
            | Op::ForIter(target)
            | Op::ForIterRange(target)
            | Op::JumpIfParamSupplied(_, target)
            | Op::MatchType(_, target) => {
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
        let a = c.add_constant(Constant::String("name".into()));
        let b = c.add_constant(Constant::String("name".into()));
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
        let a = c.add_constant(Constant::String("foo".into()));
        let b = c.add_constant(Constant::String("bar".into()));
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

    /// A forward jump over more instructions than its 16-bit operand holds
    /// marks the chunk, which the compiler refuses, instead of truncating
    /// the distance and landing inside the body it skips.
    #[test]
    fn a_jump_too_long_for_its_operand_marks_the_chunk() {
        for (body, overflows) in [(MAX_JUMP_OFFSET, false), (MAX_JUMP_OFFSET + 1, true)] {
            let mut c = Chunk::new();
            let off = c.emit(Op::JumpIfFalse(0), 1);
            for _ in 0..body {
                c.emit(Op::Nop, 1);
            }
            c.patch_jump(off);
            assert_eq!(c.offset_overflow, overflows, "{body}");
            let expected = if overflows { 0 } else { body as u16 };
            assert_eq!(c.code[off], Op::JumpIfFalse(expected), "{body}");
        }
    }

    #[test]
    fn patch_jump_works_for_each_jump_variant() {
        let variants = [
            Op::Jump(0),
            Op::JumpIfFalse(0),
            Op::JumpIfTrue(0),
            Op::JumpIfFalseNoPop(0),
            Op::JumpIfTrueNoPop(0),
            Op::JumpIfNull(0),
            Op::JumpIfNotNull(0),
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
                | Op::JumpIfTrue(t)
                | Op::JumpIfFalseNoPop(t)
                | Op::JumpIfTrueNoPop(t)
                | Op::JumpIfNull(t)
                | Op::JumpIfNotNull(t)
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
        assert_eq!(p.defaults_mask, 0);
        assert!(p.upvalue_descriptors.is_empty());
        assert!(p.chunk.is_empty());
        assert!(!p.is_method);
    }
}
