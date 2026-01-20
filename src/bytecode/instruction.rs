//! Bytecode instruction definitions for the Solilang VM.

/// Opcodes for the bytecode virtual machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum OpCode {
    // ============ Constants & Stack ============
    /// Load a constant from the constant pool: CONSTANT <index:u16>
    Constant = 0,
    /// Push null onto the stack
    Null,
    /// Push true onto the stack
    True,
    /// Push false onto the stack
    False,
    /// Pop the top value from the stack
    Pop,
    /// Duplicate the top value on the stack
    Dup,

    // ============ Variables ============
    /// Get a local variable: GET_LOCAL <slot:u16>
    GetLocal,
    /// Set a local variable: SET_LOCAL <slot:u16>
    SetLocal,
    /// Get a global variable: GET_GLOBAL <name_index:u16>
    GetGlobal,
    /// Set a global variable: SET_GLOBAL <name_index:u16>
    SetGlobal,
    /// Define a global variable: DEFINE_GLOBAL <name_index:u16>
    DefineGlobal,
    /// Get an upvalue (captured variable): GET_UPVALUE <index:u8>
    GetUpvalue,
    /// Set an upvalue: SET_UPVALUE <index:u8>
    SetUpvalue,
    /// Close upvalue on top of stack
    CloseUpvalue,

    // ============ Arithmetic ============
    /// Add two values: a + b
    Add,
    /// Subtract two values: a - b
    Subtract,
    /// Multiply two values: a * b
    Multiply,
    /// Divide two values: a / b
    Divide,
    /// Modulo: a % b
    Modulo,
    /// Negate a value: -a
    Negate,

    // ============ Comparison ============
    /// Equal: a == b
    Equal,
    /// Not equal: a != b
    NotEqual,
    /// Less than: a < b
    Less,
    /// Less or equal: a <= b
    LessEqual,
    /// Greater than: a > b
    Greater,
    /// Greater or equal: a >= b
    GreaterEqual,

    // ============ Logic ============
    /// Logical not: !a
    Not,

    // ============ Control Flow ============
    /// Unconditional jump: JUMP <offset:i16>
    Jump,
    /// Jump if false (and pop): JUMP_IF_FALSE <offset:i16>
    JumpIfFalse,
    /// Jump if true (and pop): JUMP_IF_TRUE <offset:i16>
    JumpIfTrue,
    /// Jump if false (no pop, for short-circuit): JUMP_IF_FALSE_NO_POP <offset:i16>
    JumpIfFalseNoPop,
    /// Jump if true (no pop, for short-circuit): JUMP_IF_TRUE_NO_POP <offset:i16>
    JumpIfTrueNoPop,
    /// Loop back: LOOP <offset:u16>
    Loop,

    // ============ Functions & Calls ============
    /// Call a function: CALL <arg_count:u8>
    Call,
    /// Invoke a method: INVOKE <name_index:u16> <arg_count:u8>
    Invoke,
    /// Invoke a super method: SUPER_INVOKE <name_index:u16> <arg_count:u8>
    SuperInvoke,
    /// Return from function
    Return,
    /// Create a closure: CLOSURE <func_index:u16> [upvalue_info...]
    Closure,
    /// Load a default value onto the stack: LOAD_DEFAULT <default_index:u16>
    /// default_index points to a constant that is either:
    /// - A simple value (int, float, string, bool, null)
    /// - A nested function for closures
    LoadDefault,

    // ============ Classes & Objects ============
    /// Create a class: CLASS <name_index:u16>
    Class,
    /// Inherit from superclass
    Inherit,
    /// Define a method: METHOD <name_index:u16>
    Method,
    /// Define a static method: STATIC_METHOD <name_index:u16>
    StaticMethod,
    /// Get a property: GET_PROPERTY <name_index:u16>
    GetProperty,
    /// Set a property: SET_PROPERTY <name_index:u16>
    SetProperty,
    /// Get `this`
    GetThis,
    /// Get `super`
    GetSuper,
    /// Create new instance: NEW <class_name_index:u16> <arg_count:u8>
    New,

    // ============ Collections ============
    /// Build an array: BUILD_ARRAY <count:u16>
    BuildArray,
    /// Build a hash: BUILD_HASH <pair_count:u16>
    BuildHash,
    /// Get element by index: obj[index]
    Index,
    /// Set element by index: obj[index] = value
    IndexSet,
    /// Spread array elements onto the stack: SPREAD_ARRAY
    /// Pops an array from the stack and pushes its elements
    SpreadArray,
    /// Spread hash entries onto the stack: SPREAD_HASH
    /// Pops a hash from the stack and pushes its key-value pairs
    SpreadHash,

    // ============ Iteration ============
    /// Get iterator from iterable
    GetIterator,
    /// Iterate: pushes (value, has_more) or jumps if done
    IteratorNext,

    // ============ Native Functions ============
    /// Call a native function: NATIVE_CALL <native_index:u16> <arg_count:u8>
    NativeCall,

    // ============ Debugging ============
    /// Print top of stack (for debugging)
    Print,
}

impl OpCode {
    /// Get the number of operand bytes for this opcode.
    pub fn operand_size(self) -> usize {
        match self {
            // No operands
            OpCode::Null
            | OpCode::True
            | OpCode::False
            | OpCode::Pop
            | OpCode::Dup
            | OpCode::Add
            | OpCode::Subtract
            | OpCode::Multiply
            | OpCode::Divide
            | OpCode::Modulo
            | OpCode::Negate
            | OpCode::Equal
            | OpCode::NotEqual
            | OpCode::Less
            | OpCode::LessEqual
            | OpCode::Greater
            | OpCode::GreaterEqual
            | OpCode::Not
            | OpCode::Return
            | OpCode::CloseUpvalue
            | OpCode::Inherit
            | OpCode::GetThis
            | OpCode::GetSuper
            | OpCode::Index
            | OpCode::IndexSet
            | OpCode::GetIterator
            | OpCode::SpreadArray
            | OpCode::SpreadHash
            | OpCode::Print => 0,

            // 1 byte operand
            OpCode::Call | OpCode::GetUpvalue | OpCode::SetUpvalue => 1,

            // 2 byte operand
            OpCode::Constant
            | OpCode::GetLocal
            | OpCode::SetLocal
            | OpCode::GetGlobal
            | OpCode::SetGlobal
            | OpCode::DefineGlobal
            | OpCode::Jump
            | OpCode::JumpIfFalse
            | OpCode::JumpIfTrue
            | OpCode::JumpIfFalseNoPop
            | OpCode::JumpIfTrueNoPop
            | OpCode::Loop
            | OpCode::Closure
            | OpCode::Class
            | OpCode::Method
            | OpCode::StaticMethod
            | OpCode::GetProperty
            | OpCode::SetProperty
            | OpCode::BuildArray
            | OpCode::BuildHash
            | OpCode::IteratorNext
            | OpCode::LoadDefault => 2,

            // 3 byte operand (2 bytes + 1 byte)
            OpCode::Invoke | OpCode::SuperInvoke | OpCode::New | OpCode::NativeCall => 3,
        }
    }

    /// Convert from u8 to OpCode.
    pub fn from_u8(byte: u8) -> Option<OpCode> {
        if byte <= OpCode::Print as u8 {
            Some(unsafe { std::mem::transmute(byte) })
        } else {
            None
        }
    }
}

impl From<OpCode> for u8 {
    fn from(op: OpCode) -> u8 {
        op as u8
    }
}

/// Information about an upvalue for closure creation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpvalueInfo {
    /// True if this upvalue captures a local in the enclosing function,
    /// false if it captures an upvalue from the enclosing function.
    pub is_local: bool,
    /// The index of the local or upvalue being captured.
    pub index: u8,
}

impl UpvalueInfo {
    pub fn new(is_local: bool, index: u8) -> Self {
        Self { is_local, index }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_opcode_roundtrip() {
        for i in 0..=OpCode::Print as u8 {
            let op = OpCode::from_u8(i).expect("valid opcode");
            assert_eq!(i, op as u8);
        }
    }

    #[test]
    fn test_invalid_opcode() {
        assert!(OpCode::from_u8(255).is_none());
    }
}
