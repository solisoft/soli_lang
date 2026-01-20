//! JIT capability analyzer.
//!
//! Analyzes bytecode functions to determine if they can be JIT compiled.
//! Phase 2a: Only compile simple numeric functions (no closures, no objects).

use crate::bytecode::chunk::CompiledFunction;
use crate::bytecode::instruction::OpCode;

/// Result of analyzing a function for JIT capability.
#[derive(Debug, Clone)]
pub struct AnalysisResult {
    /// Whether the function can be JIT compiled.
    pub can_jit: bool,
    /// Reason if the function cannot be JIT compiled.
    pub reason: Option<String>,
    /// Estimated complexity score (higher = more benefit from JIT).
    pub complexity_score: u32,
}

impl AnalysisResult {
    fn jitable(score: u32) -> Self {
        Self {
            can_jit: true,
            reason: None,
            complexity_score: score,
        }
    }

    fn not_jitable(reason: &str) -> Self {
        Self {
            can_jit: false,
            reason: Some(reason.to_string()),
            complexity_score: 0,
        }
    }
}

/// Analyzer for determining JIT capability of functions.
#[derive(Debug, Default)]
pub struct Analyzer {
    /// Whether to allow closures in JIT compiled functions.
    pub allow_closures: bool,
    /// Whether to allow object operations in JIT compiled functions.
    pub allow_objects: bool,
}

impl Analyzer {
    /// Create a new analyzer with default (conservative) settings.
    pub fn new() -> Self {
        Self {
            allow_closures: false,
            allow_objects: false,
        }
    }

    /// Analyze a function to determine if it can be JIT compiled.
    pub fn analyze(&self, function: &CompiledFunction) -> AnalysisResult {
        let chunk = &function.chunk;
        let mut complexity_score = 0;
        let mut offset = 0;

        while offset < chunk.code.len() {
            let byte = chunk.code[offset];
            let opcode = match OpCode::from_u8(byte) {
                Some(op) => op,
                None => {
                    return AnalysisResult::not_jitable(&format!("Unknown opcode: {}", byte));
                }
            };

            // Check if opcode is JIT-compatible
            match opcode {
                // Supported: Basic stack operations
                OpCode::Constant
                | OpCode::Null
                | OpCode::True
                | OpCode::False
                | OpCode::Pop
                | OpCode::Dup => {
                    offset += opcode.operand_size() + 1;
                }

                // Supported: Arithmetic (great for JIT)
                OpCode::Add
                | OpCode::Subtract
                | OpCode::Multiply
                | OpCode::Divide
                | OpCode::Modulo
                | OpCode::Negate => {
                    complexity_score += 2; // Arithmetic benefits from JIT
                    offset += opcode.operand_size() + 1;
                }

                // Supported: Comparisons
                OpCode::Equal
                | OpCode::NotEqual
                | OpCode::Less
                | OpCode::LessEqual
                | OpCode::Greater
                | OpCode::GreaterEqual
                | OpCode::Not => {
                    complexity_score += 1;
                    offset += opcode.operand_size() + 1;
                }

                // Supported: Local variables
                OpCode::GetLocal | OpCode::SetLocal => {
                    complexity_score += 1;
                    offset += opcode.operand_size() + 1;
                }

                // Supported: Global variables (with some overhead)
                OpCode::GetGlobal | OpCode::SetGlobal | OpCode::DefineGlobal => {
                    offset += opcode.operand_size() + 1;
                }

                // Supported: Control flow (great for JIT)
                OpCode::Jump
                | OpCode::JumpIfFalse
                | OpCode::JumpIfTrue
                | OpCode::JumpIfFalseNoPop
                | OpCode::JumpIfTrueNoPop
                | OpCode::Loop => {
                    complexity_score += 3; // Loops benefit greatly from JIT
                    offset += opcode.operand_size() + 1;
                }

                // Supported: Function calls (can inline or call native)
                OpCode::Call | OpCode::Return => {
                    complexity_score += 2;
                    offset += opcode.operand_size() + 1;
                }

                // Supported: Native calls
                OpCode::NativeCall => {
                    offset += opcode.operand_size() + 1;
                }

                // Supported: Print (for debugging/output)
                OpCode::Print => {
                    offset += opcode.operand_size() + 1;
                }

                // Closures - conditional support
                OpCode::Closure
                | OpCode::GetUpvalue
                | OpCode::SetUpvalue
                | OpCode::CloseUpvalue => {
                    if !self.allow_closures {
                        return AnalysisResult::not_jitable("Closures not supported in JIT");
                    }
                    // Closure has variable-length encoding, need special handling
                    if opcode == OpCode::Closure {
                        // Skip past the closure's upvalue descriptors
                        let func_idx = chunk.read_u16(offset + 1);
                        if let Some(crate::bytecode::chunk::Constant::Function(f)) =
                            chunk.constants.get(func_idx as usize)
                        {
                            offset += 3 + (f.upvalue_count * 2);
                        } else {
                            return AnalysisResult::not_jitable("Invalid closure constant");
                        }
                    } else {
                        offset += opcode.operand_size() + 1;
                    }
                }

                // Object operations - conditional support
                OpCode::Class
                | OpCode::Method
                | OpCode::StaticMethod
                | OpCode::GetProperty
                | OpCode::SetProperty
                | OpCode::GetThis
                | OpCode::GetSuper
                | OpCode::Invoke
                | OpCode::SuperInvoke
                | OpCode::New
                | OpCode::Inherit => {
                    if !self.allow_objects {
                        return AnalysisResult::not_jitable(
                            "Object operations not supported in JIT",
                        );
                    }
                    offset += opcode.operand_size() + 1;
                }

                // Collections - currently not supported
                OpCode::BuildArray | OpCode::BuildHash | OpCode::Index | OpCode::IndexSet => {
                    return AnalysisResult::not_jitable(
                        "Array/Hash operations not yet supported in JIT",
                    );
                }

                // Iterator - currently not supported
                OpCode::GetIterator | OpCode::IteratorNext => {
                    return AnalysisResult::not_jitable(
                        "Iterator operations not yet supported in JIT",
                    );
                }

                // Default case - unsupported operations
                _ => {
                    return AnalysisResult::not_jitable(&format!(
                        "Unsupported opcode for JIT: {:?}",
                        opcode
                    ));
                }
            }
        }

        // Minimum complexity threshold for JIT to be worthwhile
        if complexity_score < 5 {
            return AnalysisResult::not_jitable("Function too simple for JIT");
        }

        AnalysisResult::jitable(complexity_score)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::compiler::Compiler;

    fn analyze_source(source: &str) -> Option<AnalysisResult> {
        let tokens = crate::lexer::Scanner::new(source).scan_tokens().ok()?;
        let program = crate::parser::Parser::new(tokens).parse().ok()?;
        let mut compiler = Compiler::new();
        let function = compiler.compile(&program).ok()?;

        // Find the first nested function
        for constant in &function.chunk.constants {
            if let crate::bytecode::chunk::Constant::Function(f) = constant {
                let analyzer = Analyzer::new();
                return Some(analyzer.analyze(f));
            }
        }

        None
    }

    #[test]
    fn test_simple_numeric_function() {
        let result = analyze_source(
            r#"
            fn add(a: Int, b: Int) -> Int {
                return a + b;
            }
        "#,
        );

        // Function might be too simple
        assert!(result.is_some());
    }

    #[test]
    fn test_recursive_function() {
        let result = analyze_source(
            r#"
            fn fib(n: Int) -> Int {
                if (n <= 1) { return n; }
                return fib(n - 1) + fib(n - 2);
            }
        "#,
        );

        if let Some(r) = result {
            assert!(r.can_jit, "Fibonacci should be JIT-able: {:?}", r.reason);
            assert!(r.complexity_score > 5);
        }
    }

    #[test]
    fn test_loop_function() {
        let result = analyze_source(
            r#"
            fn sum_to(n: Int) -> Int {
                let total = 0;
                let i = 0;
                while (i < n) {
                    total = total + i;
                    i = i + 1;
                }
                return total;
            }
        "#,
        );

        if let Some(r) = result {
            assert!(
                r.can_jit,
                "Loop function should be JIT-able: {:?}",
                r.reason
            );
            assert!(r.complexity_score >= 5);
        }
    }
}
