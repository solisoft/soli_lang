//! AST-to-bytecode compiler.
//!
//! Single-pass compilation: walks the AST once, emitting bytecode into a `Chunk`.
//! Variable resolution happens at compile time — locals become stack slot indices.

use std::rc::Rc;

use crate::ast::stmt::{Parameter, Program};
use crate::error::CompileError;
use crate::span::Span;

use super::chunk::{Chunk, CompiledModule, Constant, FunctionProto};
use super::opcode::Op;
use super::upvalue::UpvalueDescriptor;

/// Result type for compilation.
pub type CompileResult<T> = Result<T, CompileError>;

/// A local variable tracked during compilation.
#[derive(Debug, Clone)]
pub struct Local {
    pub name: String,
    pub depth: i32,
    pub is_captured: bool,
    pub is_const: bool,
}

/// Tracks what kind of function is being compiled.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FunctionType {
    Script,
    Function,
    Method,
    Constructor,
    Lambda,
}

/// The compiler: transforms AST into bytecode.
pub struct Compiler {
    /// The function prototype being built.
    pub proto: FunctionProto,
    /// Local variables in scope.
    pub locals: Vec<Local>,
    /// Current scope depth (0 = global).
    pub scope_depth: i32,
    /// Upvalue descriptors for the current function.
    pub upvalues: Vec<UpvalueDescriptor>,
    /// Enclosing compiler (for nested functions/closures).
    pub enclosing: Option<Box<Compiler>>,
    /// What kind of function we're compiling.
    pub function_type: FunctionType,
    /// Current loop context for break/continue (start_offset, break_patches).
    pub loop_context: Option<LoopContext>,
    /// Current class context for this/super.
    pub class_context: Option<ClassContext>,
}

#[derive(Debug, Clone)]
pub struct LoopContext {
    pub start: usize,
    pub break_patches: Vec<usize>,
    pub enclosing: Option<Box<LoopContext>>,
}

#[derive(Debug, Clone)]
pub struct ClassContext {
    pub has_superclass: bool,
}

impl Compiler {
    pub fn new(function_type: FunctionType, name: String) -> Self {
        let mut compiler = Self {
            proto: FunctionProto::new(name),
            locals: Vec::new(),
            scope_depth: 0,
            upvalues: Vec::new(),
            enclosing: None,
            function_type,
            loop_context: None,
            class_context: None,
        };

        // Reserve slot 0 for `this` in methods, or an empty slot otherwise
        let slot_name = if function_type == FunctionType::Method
            || function_type == FunctionType::Constructor
        {
            "this".to_string()
        } else {
            String::new()
        };
        compiler.locals.push(Local {
            name: slot_name,
            depth: 0,
            is_captured: false,
            is_const: false,
        });

        compiler
    }

    /// Compile a full program.
    pub fn compile(program: &Program) -> CompileResult<CompiledModule> {
        let mut compiler = Compiler::new(FunctionType::Script, String::new());
        for stmt in &program.statements {
            compiler.compile_stmt(stmt)?;
        }
        // Implicit return null for scripts
        compiler.emit(Op::Null, 0);
        compiler.emit(Op::Return, 0);

        let mut proto = compiler.proto;
        proto.upvalue_descriptors = compiler.upvalues;
        Ok(CompiledModule {
            main: Rc::new(proto),
        })
    }

    // --- Chunk helpers ---

    pub fn chunk(&mut self) -> &mut Chunk {
        &mut self.proto.chunk
    }

    pub fn emit(&mut self, op: Op, line: usize) -> usize {
        self.proto.chunk.emit(op, line)
    }

    pub fn emit_constant(&mut self, constant: Constant, line: usize) {
        let idx = self.proto.chunk.add_constant(constant);
        self.emit(Op::Constant(idx), line);
    }

    pub fn current_offset(&self) -> usize {
        self.proto.chunk.len()
    }

    pub fn emit_jump(&mut self, op: Op, line: usize) -> usize {
        self.emit(op, line)
    }

    pub fn patch_jump(&mut self, offset: usize) {
        self.proto.chunk.patch_jump(offset);
    }

    pub fn emit_loop(&mut self, loop_start: usize, line: usize) {
        let offset = self.proto.chunk.len() - loop_start + 1;
        self.emit(Op::Loop(offset as u16), line);
    }

    pub fn add_constant(&mut self, constant: Constant) -> u16 {
        self.proto.chunk.add_constant(constant)
    }

    pub fn add_string_constant(&mut self, s: &str) -> u16 {
        self.proto.chunk.add_constant(Constant::String(s.to_string()))
    }

    // --- Scope management ---

    pub fn begin_scope(&mut self) {
        self.scope_depth += 1;
    }

    pub fn end_scope(&mut self, line: usize) {
        self.scope_depth -= 1;
        // Pop locals that go out of scope
        while let Some(local) = self.locals.last() {
            if local.depth <= self.scope_depth {
                break;
            }
            if local.is_captured {
                self.emit(Op::CloseUpvalue, line);
            } else {
                self.emit(Op::Pop, line);
            }
            self.locals.pop();
        }
    }

    // --- Local variables ---

    pub fn add_local(&mut self, name: String, is_const: bool) {
        self.locals.push(Local {
            name,
            depth: self.scope_depth,
            is_captured: false,
            is_const,
        });
    }

    pub fn declare_variable(&mut self, name: &str, is_const: bool, span: Span) -> CompileResult<()> {
        if self.scope_depth == 0 {
            return Ok(()); // globals are handled differently
        }
        // Check for redeclaration in the same scope
        for local in self.locals.iter().rev() {
            if local.depth != -1 && local.depth < self.scope_depth {
                break;
            }
            if local.name == name {
                return Err(CompileError::new(
                    format!("Variable '{}' already declared in this scope", name),
                    span,
                ));
            }
        }
        self.add_local(name.to_string(), is_const);
        Ok(())
    }

    pub fn resolve_local(&self, name: &str) -> Option<u16> {
        for (i, local) in self.locals.iter().enumerate().rev() {
            if local.name == name && local.depth != -1 {
                return Some(i as u16);
            }
        }
        None
    }

    pub fn resolve_upvalue(&mut self, name: &str) -> Option<u16> {
        // Check local in enclosing compiler
        if let Some(ref mut enclosing) = self.enclosing {
            if let Some(local_idx) = enclosing.resolve_local(name) {
                enclosing.locals[local_idx as usize].is_captured = true;
                return Some(self.add_upvalue(local_idx, true));
            }
            // Check upvalue in enclosing compiler (recursive)
            if let Some(upvalue_idx) = enclosing.resolve_upvalue(name) {
                return Some(self.add_upvalue(upvalue_idx, false));
            }
        }
        None
    }

    fn add_upvalue(&mut self, index: u16, is_local: bool) -> u16 {
        // Check if we already have this upvalue
        for (i, uv) in self.upvalues.iter().enumerate() {
            if uv.index == index && uv.is_local == is_local {
                return i as u16;
            }
        }
        let idx = self.upvalues.len() as u16;
        self.upvalues.push(UpvalueDescriptor { is_local, index });
        idx
    }

    /// Resolve a variable name to the appropriate get/set operations.
    pub fn resolve_variable(&mut self, name: &str) -> VariableAccess {
        if let Some(slot) = self.resolve_local(name) {
            VariableAccess::Local(slot)
        } else if let Some(idx) = self.resolve_upvalue(name) {
            VariableAccess::Upvalue(idx)
        } else {
            VariableAccess::Global(name.to_string())
        }
    }

    // --- Function compilation ---

    /// Start compiling a new function. Returns the current compiler, replacing it with a fresh one.
    pub fn start_function(
        &mut self,
        function_type: FunctionType,
        name: String,
        params: &[Parameter],
    ) -> Box<Compiler> {
        let mut new_compiler = Compiler::new(function_type, name);
        new_compiler.class_context = self.class_context.clone();

        // Add parameters as locals
        for param in params {
            new_compiler.add_local(param.name.clone(), false);
        }
        new_compiler.proto.arity = params.iter().filter(|p| p.default_value.is_none()).count() as u8;
        new_compiler.proto.defaults = params.iter().filter(|p| p.default_value.is_some()).count() as u8;
        new_compiler.proto.param_names = params.iter().map(|p| p.name.clone()).collect();

        // Swap self with the new compiler, storing self as enclosing
        let old = std::mem::replace(self, new_compiler);
        self.enclosing = Some(Box::new(old));
        // Return a dummy — we'll use finish_function to unwrap
        Box::new(Compiler::new(FunctionType::Script, String::new()))
    }

    /// Finish compiling the current function, returning the proto and restoring the enclosing compiler.
    pub fn finish_function(&mut self, line: usize) -> FunctionProto {
        // Implicit return null
        self.emit(Op::Null, line);
        self.emit(Op::Return, line);

        let mut proto = std::mem::replace(&mut self.proto, FunctionProto::new(String::new()));
        proto.upvalue_descriptors = std::mem::take(&mut self.upvalues);

        // Restore enclosing compiler
        if let Some(enclosing) = self.enclosing.take() {
            *self = *enclosing;
        }

        proto
    }

    // --- Loop context ---

    pub fn begin_loop(&mut self, start: usize) {
        let enclosing = self.loop_context.take().map(Box::new);
        self.loop_context = Some(LoopContext {
            start,
            break_patches: Vec::new(),
            enclosing,
        });
    }

    pub fn end_loop(&mut self) {
        if let Some(ctx) = self.loop_context.take() {
            // Patch all break jumps
            for patch in &ctx.break_patches {
                self.patch_jump(*patch);
            }
            self.loop_context = ctx.enclosing.map(|b| *b);
        }
    }

    pub fn add_break_patch(&mut self, offset: usize) {
        if let Some(ref mut ctx) = self.loop_context {
            ctx.break_patches.push(offset);
        }
    }
}

/// How a variable is accessed at runtime.
#[derive(Debug, Clone)]
pub enum VariableAccess {
    Local(u16),
    Upvalue(u16),
    Global(String),
}
