---
title: JIT Compilation
description: Just-In-Time compilation with Cranelift
---

# JIT Compilation

Soli includes an optional JIT (Just-In-Time) compiler that uses [Cranelift](https://cranelift.dev/) to generate native machine code for hot functions.

## Enabling JIT

JIT compilation requires building with the `jit` feature:

```bash
# Build with JIT support
cargo build --features jit

# Run with JIT
soli --jit script.soli
```

## How It Works

The JIT system has three main components:

### 1. Profiler

Tracks function call counts to identify "hot" functions:

```rust
const JIT_THRESHOLD: u32 = 100;

// Function becomes "hot" after 100 calls
profiler.record_call("fib");
```

### 2. Analyzer

Determines if a function can be JIT compiled:

```rust
let analysis = analyzer.analyze(&function);
if analysis.can_jit {
    // Function is safe to compile
}
```

**JIT-Compatible Features:**
- Arithmetic operations (+, -, *, /, %)
- Comparisons (==, !=, <, <=, >, >=)
- Local variables
- Control flow (if/else, while loops)
- Function calls
- Returns

**Not Yet Supported:**
- Closures (capturing variables)
- Object operations (classes, methods)
- Arrays and hashes
- Iterators

### 3. Code Generator

Compiles bytecode to native code using Cranelift:

```
┌─────────────┐     ┌──────────────┐     ┌─────────────┐
│  Bytecode   │────▶│  Cranelift   │────▶│   Native    │
│  Function   │     │  Code Gen    │     │    Code     │
└─────────────┘     └──────────────┘     └─────────────┘
```

## Example

Consider this function:

```solilang
fn fib(n: Int) -> Int {
    if (n <= 1) { return n; }
    return fib(n - 1) + fib(n - 2);
}
```

### Bytecode (before JIT)
```
== fib (arity: 1) ==
0000  GetLocal 0
0003  Constant 0 (1)
0006  LessEqual
0007  JumpIfFalse 5 -> 15
0010  Pop
0011  GetLocal 0
0014  Return
0015  Pop
0016  GetGlobal 1 (fib)
...
```

### Native Code (after JIT)

The JIT generates x86-64 assembly that directly computes the result:
- Parameters passed in registers
- Stack operations eliminated
- Native CPU instructions for arithmetic

## Performance Benefits

JIT compilation provides the best speedup for:

1. **Tight loops** - Eliminates interpreter overhead
2. **Recursive functions** - Fast function calls
3. **Numeric computations** - Uses native CPU instructions
4. **Hot paths** - Only compiles frequently-used code

## Architecture

```
┌────────────────────────────────────────────────┐
│                   JitVM                        │
├────────────────────────────────────────────────┤
│  ┌──────────┐  ┌──────────┐  ┌──────────────┐ │
│  │ Profiler │  │ Analyzer │  │ CodeGenerator│ │
│  └──────────┘  └──────────┘  └──────────────┘ │
│        │             │              │          │
│        ▼             ▼              ▼          │
│  ┌─────────────────────────────────────────┐  │
│  │           Bytecode VM                   │  │
│  │    (fallback for non-JIT functions)     │  │
│  └─────────────────────────────────────────┘  │
└────────────────────────────────────────────────┘
```

## Cranelift Integration

We use these Cranelift crates:

- `cranelift` - Main compilation framework
- `cranelift-jit` - JIT memory management
- `cranelift-module` - Module/function management
- `cranelift-native` - Native target detection
- `cranelift-frontend` - IR builder API

### IR Generation Example

```rust
// Cranelift IR for: a + b
let a = builder.use_var(locals[0]);
let b = builder.use_var(locals[1]);
let result = builder.ins().iadd(a, b);
value_stack.push(result);
```

## Limitations

Current JIT limitations (Phase 2a):

1. **No closures** - Functions that capture variables fallback to bytecode
2. **No objects** - Class methods and property access not JIT-compiled
3. **No collections** - Array/hash operations use bytecode
4. **Integer only** - Float support is simplified

These will be addressed in future phases.

## Debugging JIT

To see if a function is being JIT compiled:

```rust
let vm = JitVM::new();
if vm.is_jit_compiled("fib") {
    println!("fib is using native code!");
}
```

## Building Without JIT

JIT is optional. Without the feature flag, Soli uses only the bytecode VM:

```bash
# Default build (no JIT)
cargo build

# Only bytecode execution available
soli --bytecode script.soli
```

This keeps the binary smaller and avoids the Cranelift dependency (~10MB).
