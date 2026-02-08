//! Bytecode VM for Soli — compiles AST to bytecode and executes via a stack-based VM.
//!
//! Used as the default execution engine in `soli serve` production mode.
//! Tree-walking interpreter remains for `--dev` mode and REPL.

pub mod chunk;
pub mod compiler;
pub mod compiler_classes;
pub mod compiler_exprs;
pub mod compiler_patterns;
pub mod compiler_stmts;
pub mod disassembler;
pub mod opcode;
pub mod upvalue;
#[allow(clippy::module_inception)]
pub mod vm;
pub mod vm_calls;
pub mod vm_classes;
pub mod vm_exceptions;

pub use chunk::{CompiledModule, FunctionProto};
pub use compiler::Compiler;
pub use disassembler::disassemble;
pub use opcode::Op;
pub use upvalue::VmClosure;
pub use vm::Vm;
