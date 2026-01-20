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

    // ============ Pattern Matching ============
    /// Check if value is of a type: TYPE_CHECK <type_index:u16>
    /// Pops value, pushes bool result
    TypeCheck,
    /// Get array length: ARRAY_LEN
    /// Pops array, pushes int length
    ArrayLen,
    /// Get property by string constant: GET_PROPERTY_STR <name_index:u16>
    /// Pops object, pushes property value (or null if not found)
    GetPropertyStr,
    /// Get instance field by string constant: GET_FIELD_STR <name_index:u16>
    /// Pops instance, pushes field value
    GetFieldStr,
    /// Create array from stack elements: BUILD_ARRAY_FROM_STACK <count:u16>
    /// Pops count elements, pushes new array
    BuildArrayFromStack,
    /// Create hash from stack key-value pairs: BUILD_HASH_FROM_STACK <pair_count:u16>
    /// Pops 2*pair_count elements, pushes new hash
    BuildHashFromStack,
    /// Store binding: STORE_BINDING <name_index:u16>
    /// Pops value, stores in bindings map
    StoreBinding,

    // ============ Debugging ============
    /// Print top of stack (for debugging)
    Print,

    // ============ Exception Handling ============
    /// Push exception handler info: TRY <catch_offset:u16> <finally_offset:u16>
    /// catch_offset and finally_offset are 0 if not present
    Try,
    /// End try block: TRY_END
    TryEnd,
    /// Throw an exception (pops value from stack): THROW
    Throw,
    /// Re-throw current exception: RETHROW
    Rethrow,
    /// Pop exception handler: POP_TRY
    PopTry,
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
            | OpCode::ArrayLen
            | OpCode::Print
            | OpCode::TryEnd
            | OpCode::Throw
            | OpCode::Rethrow
            | OpCode::PopTry => 0,

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
            | OpCode::LoadDefault
            | OpCode::TypeCheck
            | OpCode::GetPropertyStr
            | OpCode::GetFieldStr
            | OpCode::BuildArrayFromStack
            | OpCode::BuildHashFromStack
            | OpCode::StoreBinding
            | OpCode::Try => 2,

            // 3 byte operand (2 bytes + 1 byte)
            OpCode::Invoke | OpCode::SuperInvoke | OpCode::New | OpCode::NativeCall => 3,
        }
    }

    /// Convert from u8 to OpCode.
    pub fn from_u8(byte: u8) -> Option<OpCode> {
        match byte {
            0 => Some(OpCode::Constant),
            1 => Some(OpCode::Null),
            2 => Some(OpCode::True),
            3 => Some(OpCode::False),
            4 => Some(OpCode::Pop),
            5 => Some(OpCode::Dup),
            6 => Some(OpCode::GetLocal),
            7 => Some(OpCode::SetLocal),
            8 => Some(OpCode::GetGlobal),
            9 => Some(OpCode::SetGlobal),
            10 => Some(OpCode::DefineGlobal),
            11 => Some(OpCode::GetUpvalue),
            12 => Some(OpCode::SetUpvalue),
            13 => Some(OpCode::CloseUpvalue),
            14 => Some(OpCode::Add),
            15 => Some(OpCode::Subtract),
            16 => Some(OpCode::Multiply),
            17 => Some(OpCode::Divide),
            18 => Some(OpCode::Modulo),
            19 => Some(OpCode::Negate),
            20 => Some(OpCode::Equal),
            21 => Some(OpCode::NotEqual),
            22 => Some(OpCode::Less),
            23 => Some(OpCode::LessEqual),
            24 => Some(OpCode::Greater),
            25 => Some(OpCode::GreaterEqual),
            26 => Some(OpCode::Not),
            27 => Some(OpCode::Jump),
            28 => Some(OpCode::JumpIfFalse),
            29 => Some(OpCode::JumpIfTrue),
            30 => Some(OpCode::JumpIfFalseNoPop),
            31 => Some(OpCode::JumpIfTrueNoPop),
            32 => Some(OpCode::Loop),
            33 => Some(OpCode::Call),
            34 => Some(OpCode::Invoke),
            35 => Some(OpCode::SuperInvoke),
            36 => Some(OpCode::Return),
            37 => Some(OpCode::Closure),
            38 => Some(OpCode::LoadDefault),
            39 => Some(OpCode::Class),
            40 => Some(OpCode::Inherit),
            41 => Some(OpCode::Method),
            42 => Some(OpCode::StaticMethod),
            43 => Some(OpCode::GetProperty),
            44 => Some(OpCode::SetProperty),
            45 => Some(OpCode::GetThis),
            46 => Some(OpCode::GetSuper),
            47 => Some(OpCode::New),
            48 => Some(OpCode::BuildArray),
            49 => Some(OpCode::BuildHash),
            50 => Some(OpCode::Index),
            51 => Some(OpCode::IndexSet),
            52 => Some(OpCode::SpreadArray),
            53 => Some(OpCode::SpreadHash),
            54 => Some(OpCode::GetIterator),
            55 => Some(OpCode::IteratorNext),
            56 => Some(OpCode::NativeCall),
            57 => Some(OpCode::TypeCheck),
            58 => Some(OpCode::ArrayLen),
            59 => Some(OpCode::GetPropertyStr),
            60 => Some(OpCode::GetFieldStr),
            61 => Some(OpCode::BuildArrayFromStack),
            62 => Some(OpCode::BuildHashFromStack),
            63 => Some(OpCode::StoreBinding),
            64 => Some(OpCode::Print),
            65 => Some(OpCode::Try),
            66 => Some(OpCode::TryEnd),
            67 => Some(OpCode::Throw),
            68 => Some(OpCode::Rethrow),
            69 => Some(OpCode::PopTry),
            _ => None,
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
        let opcodes = [
            OpCode::Constant,
            OpCode::Null,
            OpCode::True,
            OpCode::False,
            OpCode::Pop,
            OpCode::Dup,
            OpCode::GetLocal,
            OpCode::SetLocal,
            OpCode::GetGlobal,
            OpCode::SetGlobal,
            OpCode::DefineGlobal,
            OpCode::GetUpvalue,
            OpCode::SetUpvalue,
            OpCode::CloseUpvalue,
            OpCode::Add,
            OpCode::Subtract,
            OpCode::Multiply,
            OpCode::Divide,
            OpCode::Modulo,
            OpCode::Negate,
            OpCode::Equal,
            OpCode::NotEqual,
            OpCode::Less,
            OpCode::LessEqual,
            OpCode::Greater,
            OpCode::GreaterEqual,
            OpCode::Not,
            OpCode::Jump,
            OpCode::JumpIfFalse,
            OpCode::JumpIfTrue,
            OpCode::JumpIfFalseNoPop,
            OpCode::JumpIfTrueNoPop,
            OpCode::Loop,
            OpCode::Call,
            OpCode::Invoke,
            OpCode::SuperInvoke,
            OpCode::Return,
            OpCode::Closure,
            OpCode::LoadDefault,
            OpCode::Class,
            OpCode::Inherit,
            OpCode::Method,
            OpCode::StaticMethod,
            OpCode::GetProperty,
            OpCode::SetProperty,
            OpCode::GetThis,
            OpCode::GetSuper,
            OpCode::New,
            OpCode::BuildArray,
            OpCode::BuildHash,
            OpCode::Index,
            OpCode::IndexSet,
            OpCode::SpreadArray,
            OpCode::SpreadHash,
            OpCode::GetIterator,
            OpCode::IteratorNext,
            OpCode::NativeCall,
            OpCode::TypeCheck,
            OpCode::ArrayLen,
            OpCode::GetPropertyStr,
            OpCode::GetFieldStr,
            OpCode::BuildArrayFromStack,
            OpCode::BuildHashFromStack,
            OpCode::StoreBinding,
            OpCode::Print,
            OpCode::Try,
            OpCode::TryEnd,
            OpCode::Throw,
            OpCode::Rethrow,
            OpCode::PopTry,
        ];

        for op in opcodes {
            let byte: u8 = op.into();
            let roundtrip = OpCode::from_u8(byte);
            assert_eq!(Some(op), roundtrip, "Failed for {:?}", op);
        }
    }

    #[test]
    fn test_invalid_opcode() {
        assert!(OpCode::from_u8(255).is_none());
    }
}
