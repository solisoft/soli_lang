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

    /// Push a symbol from the constant pool onto the stack.
    Symbol(u16),

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
    /// Push value onto array at top of stack. Stack: [..., array], value on top.
    /// Pops value, leaves array on stack.
    ArrayPush,
    /// Build a hash from N key-value pairs on the stack (2*N values).
    Hash(u16),
    /// Build a hash from a precomputed HashKey list (constant pool index `keys_idx`)
    /// and N values on the stack. Used for hash literals with all-literal keys —
    /// avoids pushing/converting keys at runtime.
    HashWithKeys(u16, u16),
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
    /// Check if exception (top of stack) matches a class name. If no match, jump forward.
    /// Operands: (name_constant_idx, jump_if_no_match). Does NOT pop the value.
    CatchMatch(u16, u16),
    /// Re-throw the exception on top of stack (no typed catch matched).
    Rethrow,

    // --- Iterators ---
    /// Pop iterable, push iterator state.
    GetIter,
    /// Pop two ints (start, end), push IterState::Range directly (zero allocation).
    GetIterRange,
    /// Advance iterator or jump to exit offset.
    ForIter(u16),
    /// Specialized ForIter for ranges: inline range check + increment, no method call.
    ForIterRange(u16),

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

    // --- JSON ---
    /// Parse JSON string: pops string, pushes Value.
    JsonParse,
    /// Stringify value: pops value, pushes string.
    JsonStringify,

    // --- Super-instructions (optimized compound ops) ---
    /// Increment a local integer by 1: local[slot] += 1
    /// Replaces: GetLocal(slot), Constant(1), Add, SetLocal(slot), Pop
    IncrLocal(u16),
    /// Decrement a local integer by 1: local[slot] -= 1
    DecrLocal(u16),
    /// Add two locals and push result: push(local[a] + local[b])
    AddLocalLocal(u16, u16),
    /// Compare two locals with <=: push(local[a] <= local[b])
    LessEqualLocalLocal(u16, u16),
    /// Get local and add int constant: push(local[slot] + constant_int)
    AddLocalConst(u16, u16),
    /// Assign top of stack to local (without extra pop): local[slot] = pop()
    SetLocalPop(u16),
    /// Combined LessEqual + JumpIfFalse: pop two, compare <=, jump if false.
    TestLessEqualJump(u16),
    /// Combined Less + JumpIfFalse: pop two, compare <, jump if false.
    TestLessJump(u16),
    /// Combined GetGlobal + Call: push global function and call it.
    CallGlobal(u16, u8),
    /// No-op: does nothing (placeholder after peephole optimization)
    Nop,
    /// Combined property access + call: receiver is below args, method name from constant.
    /// Avoids allocating Value::Method intermediary.
    CallMethod(u16, u8),
    /// Like CallMethod but with a resolved method ID for direct dispatch (no string matching).
    /// Fields: (name_constant_idx, argc, method_id). Falls back to string dispatch for classes.
    CallMethodById(u16, u8, u16),
    /// Specialized hash get with a compile-time constant string key.
    HashGetConst(u16),
    /// Specialized hash has_key? with a compile-time constant string key.
    HashHasKeyConst(u16),
    /// Specialized hash delete with a compile-time constant string key.
    HashDeleteConst(u16),
    /// Specialized hash set with a compile-time constant string key.
    /// Stack: [..., hash, value]
    HashSetConst(u16),
    /// Specialized local-hash get with a compile-time constant string key.
    HashGetLocalConst(u16, u16),
    /// Specialized local-hash has_key? with a compile-time constant string key.
    HashHasKeyLocalConst(u16, u16),
    /// Specialized local-hash delete with a compile-time constant string key.
    HashDeleteLocalConst(u16, u16),
    /// Specialized local-hash set with a compile-time constant string key.
    /// Stack: [..., value]
    HashSetLocalConst(u16, u16),
    /// Specialized global-hash get with a compile-time constant string key.
    HashGetGlobalConst(u16, u16),
    /// Specialized global-hash has_key? with a compile-time constant string key.
    HashHasKeyGlobalConst(u16, u16),
    /// Specialized global-hash delete with a compile-time constant string key.
    HashDeleteGlobalConst(u16, u16),
    /// Specialized global-hash set with a compile-time constant string key.
    /// Stack: [..., value]
    HashSetGlobalConst(u16, u16),

    // --- Additional super-instructions for local arithmetic ---
    /// Subtract two locals: push(local[a] - local[b])
    SubLocalLocal(u16, u16),
    /// Multiply two locals: push(local[a] * local[b])
    MulLocalLocal(u16, u16),
    /// Divide two locals: push(local[a] / local[b])
    DivLocalLocal(u16, u16),
    /// Modulo two locals: push(local[a] % local[b])
    ModLocalLocal(u16, u16),
    /// Subtract constant from local: push(local[slot] - constant)
    SubLocalConst(u16, u16),
    /// Multiply local by constant: push(local[slot] * constant)
    MulLocalConst(u16, u16),
    /// Divide local by constant: push(local[slot] / constant)
    DivLocalConst(u16, u16),
    /// Push two locals onto stack (reduces GetLocal dispatch overhead)
    GetLocal2(u16, u16),
    /// Combined Greater + JumpIfFalse: pop two, compare >, jump if false.
    TestGreaterJump(u16),
    /// Combined GreaterEqual + JumpIfFalse: pop two, compare >=, jump if false.
    TestGreaterEqualJump(u16),
    /// Combined NotEqual + JumpIfFalse: pop two, compare !=, jump if false.
    TestNotEqualJump(u16),
    /// Compare locals: push(local[a] < local[b])
    LessLocalLocal(u16, u16),
    /// Compare locals: push(local[a] > local[b])
    GreaterLocalLocal(u16, u16),
    /// Check if local != constant, push bool
    NotEqualLocalConst(u16, u16),
    /// Check if local == constant, push bool
    EqualLocalConst(u16, u16),
    /// Check if value is null, push bool (optimized)
    IsNull,
    /// Check if value is not null, push bool (optimized)
    NotNull,
    /// Jump if value is null (for ?? operator fast path)
    JumpIfNull(u16),
    /// Jump if value is not null (for optional chaining)
    JumpIfNotNull(u16),
    /// Check if local is truthy, push bool
    IsTruthyLocal(u16),
    /// Check if local is falsy (null, false, 0, ""), push bool
    IsFalsyLocal(u16),
    /// Get local and add int (optimized for small ints)
    AddLocalInt(u16, i32),
    /// Increment local by 1 (already have IncrLocal, add this as variant)
    IncrLocalFast(u16),
    /// Get local, push it, then set it to null
    GetAndNullLocal(u16),
    /// Check if local == 0, push bool
    IsZeroLocal(u16),
    /// Check if local != 0, push bool
    NotZeroLocal(u16),
    /// Get local, push it, then increment it (common in for loops: i, i+=1)
    GetAndIncrLocal(u16),
    /// Get local, push it, then decrement it
    GetAndDecrLocal(u16),
    /// Set local to value from stack and push old value
    SwapSetLocal(u16),
    /// Get global and check if null/undefined in one op
    GetGlobalNullCheck(u16),
    /// Get global, call if exists (optimized fast path)
    GetGlobalCall(u16, u8),
    /// Get local, apply Not, push result
    NotLocal(u16),
    /// Get local, apply Negate, push result  
    NegateLocal(u16),
    /// Check if two locals are equal, push bool
    EqualLocalLocal(u16, u16),
    /// Check if two locals are not equal, push bool
    NotEqualLocalLocal(u16, u16),
    /// Pop and discard, push Null
    PopNull,
    /// Duplicate top of stack N times
    DupN(u8),
    /// Get local by index, then get property (common in loops)
    GetLocalProperty(u16, u16),
    /// Get local, get index, push result
    GetLocalIndex(u16, u16),
}
