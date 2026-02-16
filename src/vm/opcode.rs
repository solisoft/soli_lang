//! Bytecode opcodes for the Soli VM.

/// A single bytecode instruction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Op {
    // --- Constants & Literals ---
    /// Push a constant from the constant pool onto the stack.
    Constant(u16),
    /// Push null.
    Null,
    /// Push true.
    True,
    /// Push false.
    False,

    // --- Stack manipulation ---
    /// Pop the top value off the stack.
    Pop,
    /// Duplicate the top of the stack.
    Dup,

    // --- Variables ---
    /// Get a local variable by stack slot index.
    GetLocal(u16),
    /// Set a local variable by stack slot index.
    SetLocal(u16),
    /// Get a global variable by name constant index.
    GetGlobal(u16),
    /// Set a global variable by name constant index.
    SetGlobal(u16),
    /// Define a global variable by name constant index.
    DefineGlobal(u16),

    // --- Upvalues (closures) ---
    /// Get an upvalue by index.
    GetUpvalue(u16),
    /// Set an upvalue by index.
    SetUpvalue(u16),
    /// Close upvalues up to the given stack slot.
    CloseUpvalue,

    // --- Arithmetic ---
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulo,
    Negate,

    // --- Comparison ---
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,

    // --- Logical ---
    Not,

    // --- Control flow ---
    /// Unconditional forward jump by offset.
    Jump(u16),
    /// Jump forward if top of stack is falsy (pops the value).
    JumpIfFalse(u16),
    /// Jump backward by offset (for loops).
    Loop(u16),
    /// Jump forward if falsy, don't pop (for &&).
    JumpIfFalseNoPop(u16),
    /// Jump forward if truthy, don't pop (for ||).
    JumpIfTrueNoPop(u16),
    /// Jump forward if null, don't pop (for ??).
    NullishJump(u16),

    // --- Functions ---
    /// Call a function with N arguments.
    Call(u8),
    /// Create a closure from a function prototype constant index.
    /// Followed by N upvalue descriptors encoded as (is_local: u8, index: u16) in the bytecode.
    Closure(u16),
    /// Return from a function.
    Return,

    // --- Collections ---
    /// Build an array from N elements on the stack.
    Array(u16),
    /// Build a hash from N key-value pairs on the stack (2*N values).
    Hash(u16),
    /// Build a range from two values on the stack.
    Range,
    /// Get element at index: stack has [obj, index].
    GetIndex,
    /// Set element at index: stack has [obj, index, value].
    SetIndex,
    /// Build a string from N parts on the stack (for interpolation).
    BuildString(u16),
    /// Spread an iterable's elements onto the stack.
    Spread,

    // --- Objects ---
    /// Get a property by name constant index.
    GetProperty(u16),
    /// Set a property by name constant index.
    SetProperty(u16),

    // --- Classes ---
    /// Create a class with the given name constant index.
    Class(u16),
    /// Set up inheritance: stack has [subclass, superclass].
    Inherit,
    /// Add a method to a class. Name from constant index.
    Method(u16),
    /// Add a static method to a class. Name from constant index.
    StaticMethod(u16),
    /// Instantiate a class with N constructor arguments.
    New(u8),
    /// Push `this` (slot 0 in methods).
    GetThis,
    /// Push a super reference.
    GetSuper(u16),
    /// Add a field with initializer to a class. Name from constant index.
    Field(u16),
    /// Add a static field with initializer to a class. Name from constant index.
    StaticField(u16),
    /// Add a const field to a class. Name from constant index.
    ConstField(u16),
    /// Add a static const field to a class. Name from constant index.
    StaticConstField(u16),

    // --- Exceptions ---
    /// Begin a try block. Operands: catch_offset, finally_offset.
    TryBegin(u16, u16),
    /// End a try block (pop exception handler).
    TryEnd,
    /// Throw the top of stack as an exception.
    Throw,

    // --- Iterators ---
    /// Pop iterable, push iterator state.
    GetIter,
    /// Advance iterator or jump to exit offset.
    ForIter(u16),

    // --- I/O ---
    /// Print N values from the stack.
    Print(u8),

    // --- Named arguments ---
    /// Push a named argument marker with name constant index.
    /// The VM uses this to reorder arguments for function calls.
    NamedArg(u16),

    // --- Import ---
    /// Import a module by path constant index.
    Import(u16),
}
