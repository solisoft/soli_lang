//! Cranelift code generator for JIT compilation.
//!
//! Compiles bytecode to native machine code using Cranelift.

use std::collections::HashMap;

use cranelift::prelude::*;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{DataDescription, Linkage, Module};

use crate::bytecode::chunk::{CompiledFunction, Constant};
use crate::bytecode::instruction::OpCode;

/// Error type for JIT code generation.
#[derive(Debug)]
pub struct CodeGenError {
    pub message: String,
}

impl CodeGenError {
    pub fn new(msg: impl Into<String>) -> Self {
        Self {
            message: msg.into(),
        }
    }
}

impl std::fmt::Display for CodeGenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "JIT CodeGen Error: {}", self.message)
    }
}

impl std::error::Error for CodeGenError {}

/// Type alias for code generation results.
pub type CodeGenResult<T> = Result<T, CodeGenError>;

/// Compiled native function that can be called directly.
pub struct CompiledNativeFunction {
    /// Pointer to the native code.
    code_ptr: *const u8,
    /// The JIT module that owns the code (must be kept alive).
    #[allow(dead_code)]
    module: JITModule,
}

impl CompiledNativeFunction {
    /// Call the native function with integer arguments and return an integer result.
    /// This is for simple numeric functions (Phase 2a).
    ///
    /// # Safety
    /// The caller must ensure the function was compiled with matching signature.
    pub unsafe fn call_i64_i64(&self, arg: i64) -> i64 {
        let func: extern "C" fn(i64) -> i64 = std::mem::transmute(self.code_ptr);
        func(arg)
    }

    /// Call with two i64 arguments.
    pub unsafe fn call_i64_i64_i64(&self, a: i64, b: i64) -> i64 {
        let func: extern "C" fn(i64, i64) -> i64 = std::mem::transmute(self.code_ptr);
        func(a, b)
    }
}

/// JIT code generator using Cranelift.
pub struct CodeGenerator {
    /// Cranelift function builder context.
    builder_context: FunctionBuilderContext,
    /// Module context.
    ctx: codegen::Context,
    /// Data description for constants.
    #[allow(dead_code)]
    data_description: DataDescription,
}

impl CodeGenerator {
    /// Create a new code generator.
    pub fn new() -> CodeGenResult<Self> {
        Ok(Self {
            builder_context: FunctionBuilderContext::new(),
            ctx: codegen::Context::new(),
            data_description: DataDescription::new(),
        })
    }

    /// Compile a bytecode function to native code.
    pub fn compile(
        &mut self,
        function: &CompiledFunction,
    ) -> CodeGenResult<CompiledNativeFunction> {
        // Create JIT module with native ISA
        let mut flag_builder = settings::builder();
        flag_builder.set("use_colocated_libcalls", "false").unwrap();
        flag_builder.set("is_pic", "false").unwrap();
        let isa_builder =
            cranelift_native::builder().map_err(|e| CodeGenError::new(e.to_string()))?;
        let isa = isa_builder
            .finish(settings::Flags::new(flag_builder))
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        let builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
        let mut module = JITModule::new(builder);

        // Create function signature
        let mut sig = module.make_signature();
        for _ in 0..function.arity {
            sig.params.push(AbiParam::new(types::I64));
        }
        sig.returns.push(AbiParam::new(types::I64));

        // Declare the function
        let func_id = module
            .declare_function(&function.name, Linkage::Export, &sig)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        // Define the function
        self.ctx.func.signature = sig;
        self.ctx.func.name = cranelift_codegen::ir::UserFuncName::user(0, func_id.as_u32());

        // Build the function body
        self.translate_function(function, &mut module)?;

        // Compile the function
        module
            .define_function(func_id, &mut self.ctx)
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        // Clear the context for reuse
        module.clear_context(&mut self.ctx);

        // Finalize the module
        module
            .finalize_definitions()
            .map_err(|e| CodeGenError::new(e.to_string()))?;

        // Get the function pointer
        let code_ptr = module.get_finalized_function(func_id);

        Ok(CompiledNativeFunction { code_ptr, module })
    }

    /// Translate bytecode to Cranelift IR.
    fn translate_function(
        &mut self,
        function: &CompiledFunction,
        _module: &mut JITModule,
    ) -> CodeGenResult<()> {
        let mut builder = FunctionBuilder::new(&mut self.ctx.func, &mut self.builder_context);

        // Create entry block
        let entry_block = builder.create_block();
        builder.append_block_params_for_function_params(entry_block);
        builder.switch_to_block(entry_block);
        builder.seal_block(entry_block);

        // Get function parameters as variables
        let params: Vec<Value> = builder.block_params(entry_block).to_vec();

        // Translation state
        let mut value_stack: Vec<Value> = Vec::new();
        let mut locals: Vec<Variable> = Vec::new();
        let mut current_block_terminated = false;

        // Initialize local variables (parameters first)
        for i in 0..function.arity as usize {
            let var = Variable::new(i);
            builder.declare_var(var, types::I64);
            builder.def_var(var, params[i]);
            locals.push(var);
        }

        // Pre-scan for jump targets to create blocks
        let chunk = &function.chunk;
        let mut block_map: HashMap<usize, Block> = HashMap::new();

        let mut offset = 0;
        while offset < chunk.code.len() {
            let byte = chunk.code[offset];
            if let Some(opcode) = OpCode::from_u8(byte) {
                match opcode {
                    OpCode::Jump
                    | OpCode::JumpIfFalse
                    | OpCode::JumpIfTrue
                    | OpCode::JumpIfFalseNoPop
                    | OpCode::JumpIfTrueNoPop => {
                        let jump_offset = chunk.read_u16(offset + 1) as usize;
                        let target = offset + 3 + jump_offset;
                        block_map
                            .entry(target)
                            .or_insert_with(|| builder.create_block());
                        // Fall-through block
                        let next = offset + 3;
                        block_map
                            .entry(next)
                            .or_insert_with(|| builder.create_block());
                    }
                    OpCode::Loop => {
                        let jump_offset = chunk.read_u16(offset + 1) as usize;
                        let target = offset + 3 - jump_offset;
                        block_map
                            .entry(target)
                            .or_insert_with(|| builder.create_block());
                    }
                    _ => {}
                }
                offset += 1 + opcode.operand_size();
            } else {
                offset += 1;
            }
        }

        // Translate bytecode instructions
        offset = 0;
        while offset < chunk.code.len() {
            // Check if we need to start a new block
            if let Some(&block) = block_map.get(&offset) {
                // Jump to the block if current block isn't terminated
                if !current_block_terminated {
                    builder.ins().jump(block, &[]);
                }
                builder.switch_to_block(block);
                builder.seal_block(block);
                current_block_terminated = false;
            }

            // Skip if current block is terminated (unreachable code)
            if current_block_terminated {
                let byte = chunk.code[offset];
                if let Some(opcode) = OpCode::from_u8(byte) {
                    offset += 1 + opcode.operand_size();
                } else {
                    offset += 1;
                }
                continue;
            }

            let byte = chunk.code[offset];
            let opcode = OpCode::from_u8(byte)
                .ok_or_else(|| CodeGenError::new(format!("Invalid opcode: {}", byte)))?;

            match opcode {
                OpCode::Constant => {
                    let idx = chunk.read_u16(offset + 1);
                    let constant = &chunk.constants[idx as usize];
                    let value = match constant {
                        Constant::Int(n) => builder.ins().iconst(types::I64, *n),
                        Constant::Float(f) => {
                            // Convert float to int bits for now (simplified)
                            builder.ins().iconst(types::I64, f.to_bits() as i64)
                        }
                        _ => return Err(CodeGenError::new("Non-numeric constant in JIT")),
                    };
                    value_stack.push(value);
                    offset += 3;
                }

                OpCode::Null | OpCode::False => {
                    value_stack.push(builder.ins().iconst(types::I64, 0));
                    offset += 1;
                }

                OpCode::True => {
                    value_stack.push(builder.ins().iconst(types::I64, 1));
                    offset += 1;
                }

                OpCode::Pop => {
                    value_stack.pop();
                    offset += 1;
                }

                OpCode::Dup => {
                    if let Some(&val) = value_stack.last() {
                        value_stack.push(val);
                    }
                    offset += 1;
                }

                OpCode::GetLocal => {
                    let slot = chunk.read_u16(offset + 1) as usize;
                    if slot < locals.len() {
                        let val = builder.use_var(locals[slot]);
                        value_stack.push(val);
                    } else {
                        return Err(CodeGenError::new("Local variable out of range"));
                    }
                    offset += 3;
                }

                OpCode::SetLocal => {
                    let slot = chunk.read_u16(offset + 1) as usize;
                    if let Some(&val) = value_stack.last() {
                        // Extend locals if needed
                        while locals.len() <= slot {
                            let var = Variable::new(locals.len());
                            builder.declare_var(var, types::I64);
                            let zero = builder.ins().iconst(types::I64, 0);
                            builder.def_var(var, zero);
                            locals.push(var);
                        }
                        builder.def_var(locals[slot], val);
                    }
                    offset += 3;
                }

                OpCode::Add => {
                    let b = value_stack
                        .pop()
                        .ok_or_else(|| CodeGenError::new("Stack underflow"))?;
                    let a = value_stack
                        .pop()
                        .ok_or_else(|| CodeGenError::new("Stack underflow"))?;
                    value_stack.push(builder.ins().iadd(a, b));
                    offset += 1;
                }

                OpCode::Subtract => {
                    let b = value_stack
                        .pop()
                        .ok_or_else(|| CodeGenError::new("Stack underflow"))?;
                    let a = value_stack
                        .pop()
                        .ok_or_else(|| CodeGenError::new("Stack underflow"))?;
                    value_stack.push(builder.ins().isub(a, b));
                    offset += 1;
                }

                OpCode::Multiply => {
                    let b = value_stack
                        .pop()
                        .ok_or_else(|| CodeGenError::new("Stack underflow"))?;
                    let a = value_stack
                        .pop()
                        .ok_or_else(|| CodeGenError::new("Stack underflow"))?;
                    value_stack.push(builder.ins().imul(a, b));
                    offset += 1;
                }

                OpCode::Divide => {
                    let b = value_stack
                        .pop()
                        .ok_or_else(|| CodeGenError::new("Stack underflow"))?;
                    let a = value_stack
                        .pop()
                        .ok_or_else(|| CodeGenError::new("Stack underflow"))?;
                    value_stack.push(builder.ins().sdiv(a, b));
                    offset += 1;
                }

                OpCode::Modulo => {
                    let b = value_stack
                        .pop()
                        .ok_or_else(|| CodeGenError::new("Stack underflow"))?;
                    let a = value_stack
                        .pop()
                        .ok_or_else(|| CodeGenError::new("Stack underflow"))?;
                    value_stack.push(builder.ins().srem(a, b));
                    offset += 1;
                }

                OpCode::Negate => {
                    let a = value_stack
                        .pop()
                        .ok_or_else(|| CodeGenError::new("Stack underflow"))?;
                    value_stack.push(builder.ins().ineg(a));
                    offset += 1;
                }

                OpCode::Equal => {
                    let b = value_stack
                        .pop()
                        .ok_or_else(|| CodeGenError::new("Stack underflow"))?;
                    let a = value_stack
                        .pop()
                        .ok_or_else(|| CodeGenError::new("Stack underflow"))?;
                    let cmp = builder.ins().icmp(IntCC::Equal, a, b);
                    let result = builder.ins().uextend(types::I64, cmp);
                    value_stack.push(result);
                    offset += 1;
                }

                OpCode::NotEqual => {
                    let b = value_stack
                        .pop()
                        .ok_or_else(|| CodeGenError::new("Stack underflow"))?;
                    let a = value_stack
                        .pop()
                        .ok_or_else(|| CodeGenError::new("Stack underflow"))?;
                    let cmp = builder.ins().icmp(IntCC::NotEqual, a, b);
                    let result = builder.ins().uextend(types::I64, cmp);
                    value_stack.push(result);
                    offset += 1;
                }

                OpCode::Less => {
                    let b = value_stack
                        .pop()
                        .ok_or_else(|| CodeGenError::new("Stack underflow"))?;
                    let a = value_stack
                        .pop()
                        .ok_or_else(|| CodeGenError::new("Stack underflow"))?;
                    let cmp = builder.ins().icmp(IntCC::SignedLessThan, a, b);
                    let result = builder.ins().uextend(types::I64, cmp);
                    value_stack.push(result);
                    offset += 1;
                }

                OpCode::LessEqual => {
                    let b = value_stack
                        .pop()
                        .ok_or_else(|| CodeGenError::new("Stack underflow"))?;
                    let a = value_stack
                        .pop()
                        .ok_or_else(|| CodeGenError::new("Stack underflow"))?;
                    let cmp = builder.ins().icmp(IntCC::SignedLessThanOrEqual, a, b);
                    let result = builder.ins().uextend(types::I64, cmp);
                    value_stack.push(result);
                    offset += 1;
                }

                OpCode::Greater => {
                    let b = value_stack
                        .pop()
                        .ok_or_else(|| CodeGenError::new("Stack underflow"))?;
                    let a = value_stack
                        .pop()
                        .ok_or_else(|| CodeGenError::new("Stack underflow"))?;
                    let cmp = builder.ins().icmp(IntCC::SignedGreaterThan, a, b);
                    let result = builder.ins().uextend(types::I64, cmp);
                    value_stack.push(result);
                    offset += 1;
                }

                OpCode::GreaterEqual => {
                    let b = value_stack
                        .pop()
                        .ok_or_else(|| CodeGenError::new("Stack underflow"))?;
                    let a = value_stack
                        .pop()
                        .ok_or_else(|| CodeGenError::new("Stack underflow"))?;
                    let cmp = builder.ins().icmp(IntCC::SignedGreaterThanOrEqual, a, b);
                    let result = builder.ins().uextend(types::I64, cmp);
                    value_stack.push(result);
                    offset += 1;
                }

                OpCode::Not => {
                    let a = value_stack
                        .pop()
                        .ok_or_else(|| CodeGenError::new("Stack underflow"))?;
                    let zero = builder.ins().iconst(types::I64, 0);
                    let cmp = builder.ins().icmp(IntCC::Equal, a, zero);
                    let result = builder.ins().uextend(types::I64, cmp);
                    value_stack.push(result);
                    offset += 1;
                }

                OpCode::Jump => {
                    let jump_offset = chunk.read_u16(offset + 1) as usize;
                    let target = offset + 3 + jump_offset;
                    let target_block = block_map
                        .get(&target)
                        .ok_or_else(|| CodeGenError::new("Jump target not found"))?;
                    builder.ins().jump(*target_block, &[]);
                    current_block_terminated = true;
                    offset += 3;
                }

                OpCode::JumpIfFalse => {
                    let jump_offset = chunk.read_u16(offset + 1) as usize;
                    let target = offset + 3 + jump_offset;
                    let next = offset + 3;

                    let condition = value_stack
                        .last()
                        .ok_or_else(|| CodeGenError::new("Stack underflow"))?;
                    let zero = builder.ins().iconst(types::I64, 0);
                    let is_false = builder.ins().icmp(IntCC::Equal, *condition, zero);

                    let target_block = *block_map
                        .get(&target)
                        .ok_or_else(|| CodeGenError::new("Jump target not found"))?;
                    let next_block = *block_map
                        .get(&next)
                        .ok_or_else(|| CodeGenError::new("Fall-through block not found"))?;

                    builder
                        .ins()
                        .brif(is_false, target_block, &[], next_block, &[]);
                    current_block_terminated = true;
                    offset += 3;
                }

                OpCode::JumpIfTrue => {
                    let jump_offset = chunk.read_u16(offset + 1) as usize;
                    let target = offset + 3 + jump_offset;
                    let next = offset + 3;

                    let condition = value_stack
                        .last()
                        .ok_or_else(|| CodeGenError::new("Stack underflow"))?;
                    let zero = builder.ins().iconst(types::I64, 0);
                    let is_true = builder.ins().icmp(IntCC::NotEqual, *condition, zero);

                    let target_block = *block_map
                        .get(&target)
                        .ok_or_else(|| CodeGenError::new("Jump target not found"))?;
                    let next_block = *block_map
                        .get(&next)
                        .ok_or_else(|| CodeGenError::new("Fall-through block not found"))?;

                    builder
                        .ins()
                        .brif(is_true, target_block, &[], next_block, &[]);
                    current_block_terminated = true;
                    offset += 3;
                }

                OpCode::Loop => {
                    let jump_offset = chunk.read_u16(offset + 1) as usize;
                    let target = offset + 3 - jump_offset;
                    let target_block = block_map
                        .get(&target)
                        .ok_or_else(|| CodeGenError::new("Loop target not found"))?;
                    builder.ins().jump(*target_block, &[]);
                    current_block_terminated = true;
                    offset += 3;
                }

                OpCode::Return => {
                    let result = value_stack
                        .pop()
                        .unwrap_or_else(|| builder.ins().iconst(types::I64, 0));
                    builder.ins().return_(&[result]);
                    current_block_terminated = true;
                    offset += 1;
                }

                _ => {
                    return Err(CodeGenError::new(format!(
                        "Unsupported opcode in JIT: {:?}",
                        opcode
                    )));
                }
            }
        }

        // Add a return if the function doesn't end with one
        if !current_block_terminated {
            let zero = builder.ins().iconst(types::I64, 0);
            builder.ins().return_(&[zero]);
        }

        // Finalize the function
        builder.finalize();

        Ok(())
    }
}

impl Default for CodeGenerator {
    fn default() -> Self {
        Self::new().expect("Failed to create CodeGenerator")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::compiler::Compiler;

    fn compile_function(source: &str) -> Option<CompiledFunction> {
        let tokens = crate::lexer::Scanner::new(source).scan_tokens().ok()?;
        let program = crate::parser::Parser::new(tokens).parse().ok()?;
        let mut compiler = Compiler::new();
        let function = compiler.compile(&program).ok()?;

        // Find the first nested function
        for constant in function.chunk.constants {
            if let Constant::Function(f) = constant {
                return Some((*f).clone());
            }
        }

        None
    }

    #[test]
    fn test_compile_simple_function() {
        let function = compile_function(
            r#"
            fn double(n: Int) -> Int {
                return n + n;
            }
        "#,
        );

        if let Some(func) = function {
            let mut codegen = CodeGenerator::new().unwrap();
            let result = codegen.compile(&func);
            assert!(result.is_ok(), "Should compile: {:?}", result.err());

            unsafe {
                let native = result.unwrap();
                assert_eq!(native.call_i64_i64(5), 10);
                assert_eq!(native.call_i64_i64(21), 42);
            }
        }
    }

    #[test]
    fn test_compile_arithmetic() {
        let function = compile_function(
            r#"
            fn calc(a: Int, b: Int) -> Int {
                return (a + b) * (a - b);
            }
        "#,
        );

        if let Some(func) = function {
            let mut codegen = CodeGenerator::new().unwrap();
            let result = codegen.compile(&func);
            assert!(result.is_ok(), "Should compile: {:?}", result.err());

            unsafe {
                let native = result.unwrap();
                // (5+3) * (5-3) = 8 * 2 = 16
                assert_eq!(native.call_i64_i64_i64(5, 3), 16);
            }
        }
    }
}
