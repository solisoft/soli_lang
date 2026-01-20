//! JIT compilation module for Solilang.
//!
//! Provides selective JIT compilation for hot functions using Cranelift.
//!
//! # Architecture
//!
//! The JIT system works as follows:
//! 1. Functions run initially on the bytecode VM
//! 2. The profiler tracks call counts
//! 3. When a function becomes "hot" (crosses threshold), it's analyzed
//! 4. If JIT-able, it's compiled to native code
//! 5. Subsequent calls use the native version
//!
//! # Phase 2a Limitations
//!
//! Currently only supports simple numeric functions:
//! - No closures
//! - No object operations
//! - No array/hash operations
//! - Only Int/Float parameters and returns

mod analyzer;
mod codegen;
mod profiler;

pub use analyzer::{AnalysisResult, Analyzer};
pub use codegen::{CodeGenError, CodeGenerator, CompiledNativeFunction};
pub use profiler::{Profiler, JIT_THRESHOLD};

use std::collections::HashMap;
use std::rc::Rc;

use crate::bytecode::chunk::CompiledFunction;
use crate::bytecode::vm::VM;
use crate::error::RuntimeError;

/// JIT-enabled virtual machine.
///
/// Wraps the bytecode VM and adds JIT compilation capabilities.
pub struct JitVM {
    /// The underlying bytecode VM.
    vm: VM,
    /// Profiler for hot path detection.
    profiler: Profiler,
    /// Analyzer for JIT capability.
    analyzer: Analyzer,
    /// Code generator.
    codegen: Option<CodeGenerator>,
    /// Compiled native functions.
    native_functions: HashMap<String, Rc<CompiledNativeFunction>>,
}

impl JitVM {
    /// Create a new JIT-enabled VM.
    pub fn new() -> Self {
        Self {
            vm: VM::new(),
            profiler: Profiler::new(),
            analyzer: Analyzer::new(),
            codegen: CodeGenerator::new().ok(),
            native_functions: HashMap::new(),
        }
    }

    /// Run a compiled function with JIT support.
    pub fn run(&mut self, function: CompiledFunction) -> Result<(), RuntimeError> {
        // For now, just run on the bytecode VM
        // In a full implementation, we would:
        // 1. Hook into function calls to record profiling data
        // 2. Check for hot functions and JIT compile them
        // 3. Redirect hot function calls to native code

        // The integration would require modifying the VM to call back
        // into the JitVM for function dispatch. For now, we just run
        // the bytecode VM directly.
        self.vm.run(function)
    }

    /// Try to JIT compile a function.
    pub fn try_jit_compile(
        &mut self,
        function: &CompiledFunction,
    ) -> Option<Rc<CompiledNativeFunction>> {
        // Check if already compiled
        if let Some(native) = self.native_functions.get(&function.name) {
            return Some(native.clone());
        }

        // Analyze if JIT-able
        let analysis = self.analyzer.analyze(function);
        if !analysis.can_jit {
            return None;
        }

        // Compile
        let codegen = self.codegen.as_mut()?;
        match codegen.compile(function) {
            Ok(native) => {
                let native = Rc::new(native);
                self.native_functions
                    .insert(function.name.clone(), native.clone());
                self.profiler.mark_jit_compiled(&function.name);
                Some(native)
            }
            Err(e) => {
                eprintln!("JIT compilation failed for {}: {}", function.name, e);
                None
            }
        }
    }

    /// Record a function call for profiling.
    /// Returns true if the function just became hot.
    pub fn record_call(&mut self, function_name: &str) -> bool {
        self.profiler.record_call(function_name)
    }

    /// Check if a function has been JIT compiled.
    pub fn is_jit_compiled(&self, function_name: &str) -> bool {
        self.native_functions.contains_key(function_name)
    }

    /// Get JIT statistics.
    pub fn stats(&self) -> JitStats {
        JitStats {
            functions_compiled: self.native_functions.len(),
            hot_functions: self.profiler.get_hot_functions().len(),
        }
    }
}

impl Default for JitVM {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics about JIT compilation.
#[derive(Debug, Clone)]
pub struct JitStats {
    /// Number of functions compiled to native code.
    pub functions_compiled: usize,
    /// Number of hot functions detected.
    pub hot_functions: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::compiler::Compiler;

    #[test]
    fn test_jit_vm_creation() {
        let vm = JitVM::new();
        assert!(!vm.is_jit_compiled("test"));
    }

    #[test]
    fn test_profiler_integration() {
        let mut vm = JitVM::new();

        // Record calls below threshold
        for _ in 0..50 {
            assert!(!vm.record_call("test_func"));
        }

        // Record calls up to threshold
        for i in 50..JIT_THRESHOLD {
            let became_hot = vm.record_call("test_func");
            if i == JIT_THRESHOLD - 1 {
                assert!(became_hot);
            }
        }
    }

    fn compile_to_bytecode(source: &str) -> Option<CompiledFunction> {
        let tokens = crate::lexer::Scanner::new(source).scan_tokens().ok()?;
        let program = crate::parser::Parser::new(tokens).parse().ok()?;
        let mut compiler = Compiler::new();
        compiler.compile(&program).ok()
    }

    #[test]
    fn test_simple_execution() {
        let source = r#"
            let x = 5 + 3;
            print(x);
        "#;

        if let Some(function) = compile_to_bytecode(source) {
            let mut vm = JitVM::new();
            assert!(vm.run(function).is_ok());
        }
    }
}
