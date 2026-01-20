---
title: Installation
description: How to install Soli on your system
---

# Installation

Soli is distributed as a single binary called `soli`. Here's how to get it set up on your system.

## From Source (Recommended)

The easiest way to install Soli is to build from source using Cargo (Rust's package manager).

### Prerequisites

- [Rust](https://rustup.rs/) 1.70 or later

### Build Steps

```bash
# Clone the repository
git clone https://github.com/solilang/solilang.git
cd solilang

# Build the release binary
cargo build --release

# The binary is now at target/release/soli
```

### Build with JIT Support (Optional)

For maximum performance, you can build with JIT compilation support:

```bash
# Build with JIT feature (requires more dependencies)
cargo build --release --features jit
```

This enables the `--jit` execution mode which compiles hot functions to native machine code using Cranelift.

### Add to PATH

To use `soli` from anywhere, add it to your PATH:

```bash
# Linux/macOS
sudo cp target/release/soli /usr/local/bin/

# Or add to your shell config
echo 'export PATH="$PATH:/path/to/solilang/target/release"' >> ~/.bashrc
```

## Verify Installation

Check that Soli is installed correctly:

```bash
soli --help
```

Or start the REPL:

```bash
soli
```

You should see:

```
Soli v0.1.0 - Soli Interpreter
Type 'exit' or Ctrl+D to quit.

>>>
```

## Running Programs

### Execute a File

```bash
soli program.soli
```

### Execution Modes

Soli supports multiple execution modes:

```bash
# Default: Bytecode VM (recommended)
soli program.soli

# Tree-walk interpreter
soli --tree-walk program.soli

# Bytecode VM (explicit)
soli --bytecode program.soli

# JIT compilation (if built with --features jit)
soli --jit program.soli

# Show bytecode disassembly
soli --disassemble program.soli
```

See [Execution Modes](/internals/execution-modes/) for details on performance characteristics.

### Interactive REPL

Start the REPL by running `soli` without arguments:

```bash
$ soli
Soli v0.1.0 - Soli Interpreter
Type 'exit' or Ctrl+D to quit.

>>> 2 + 2
4
>>> let x = 10
>>> x * 5
50
>>> exit
```

## File Extension

Soli source files use the `.soli` extension by convention.

## Editor Support

While there's no official editor plugin yet, Soli syntax is similar enough to Rust and TypeScript that those highlighters work reasonably well.

## Next Steps

Now that you have Soli installed, check out the [Quick Start](/guides/quickstart/) guide to write your first program!
