//! Bytecode module for the Solilang VM.
//!
//! This module provides a bytecode compiler and virtual machine for executing
//! Solilang programs. The bytecode VM offers significant performance improvements
//! over the tree-walking interpreter.
//!
//! # Architecture
//!
//! - `instruction`: OpCode definitions for the bytecode instruction set
//! - `chunk`: Bytecode chunks containing instructions and constant pools
//! - `compiler`: Transforms AST into bytecode
//! - `vm`: Stack-based virtual machine for executing bytecode
//! - `disassembler`: Debug output for bytecode inspection

pub mod chunk;
pub mod compiler;
pub mod disassembler;
pub mod instruction;
pub mod vm;

pub use chunk::{Chunk, CompiledFunction, Constant, VMValue};
pub use compiler::Compiler;
pub use disassembler::{disassemble_function, print_disassembly};
pub use instruction::OpCode;
pub use vm::VM;
