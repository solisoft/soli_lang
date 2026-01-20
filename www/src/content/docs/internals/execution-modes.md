---
title: Execution Modes
description: Understanding Soli's different execution modes
---

# Execution Modes

Soli supports three execution modes, each with different performance characteristics and trade-offs.

## Available Modes

### Tree-Walk Interpreter

The default interpreter that directly walks the Abstract Syntax Tree (AST) to execute code.

```bash
soli --tree-walk script.soli
```

**Characteristics:**
- Simplest implementation
- Good for debugging and development
- Slower execution speed
- Best compatibility with all language features

### Bytecode VM

A stack-based virtual machine that executes compiled bytecode.

```bash
soli --bytecode script.soli
```

**Characteristics:**
- 2-3.5x faster than tree-walk interpreter
- Compiles AST to bytecode instructions
- Stack-based execution model
- Good balance of speed and simplicity

### JIT Compilation

Just-In-Time compilation using Cranelift for native code generation.

```bash
# Requires building with JIT feature
cargo build --features jit
soli --jit script.soli
```

**Characteristics:**
- Compiles hot functions to native machine code
- Fastest execution for numeric-heavy code
- Requires the `jit` feature flag
- Currently supports simple numeric functions

## Performance Comparison

Based on benchmarks with common workloads:

| Benchmark | Tree-walk | Bytecode | Speedup |
|-----------|-----------|----------|---------|
| fib(20) recursive | 6.45 ms | 3.30 ms | **1.96x** |
| fib(40) iterative | 35 µs | 15 µs | **2.36x** |
| sum_to(10000) | 3.28 ms | 930 µs | **3.53x** |
| nested loops | 3.43 ms | 967 µs | **3.55x** |
| function calls | 841 µs | 352 µs | **2.39x** |

## Choosing an Execution Mode

- **Development/Debugging**: Use `--tree-walk` for the most predictable behavior
- **General use**: Use `--bytecode` (default) for good performance
- **Performance-critical**: Use `--jit` for numeric computations

## Disassembly Output

You can view the bytecode before execution with the `--disassemble` flag:

```bash
soli --disassemble script.soli
```

Example output:
```
== <script> (arity: 0) ==
0000    1 Constant 0 (42)
0003    | DefineGlobal 1 (x)
0006    0 Null
0007    | Return
```

This is useful for understanding how the compiler translates your code and for debugging performance issues.
