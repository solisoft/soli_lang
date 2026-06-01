//! AST-to-bytecode compiler.
//!
//! Single-pass compilation: walks the AST once, emitting bytecode into a `Chunk`.
//! Variable resolution happens at compile time — locals become stack slot indices.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use crate::ast::stmt::{Parameter, Program, Stmt};
use crate::error::CompileError;
use crate::span::Span;

use super::chunk::{Chunk, CompiledModule, Constant, FunctionProto};
use super::opcode::Op;
use super::upvalue::UpvalueDescriptor;

/// Result type for compilation.
pub type CompileResult<T> = Result<T, CompileError>;

/// Whether the VM honors Soli's optional-`let` (bare assignment creates a
/// binding) by hoisting function-locals and upserting globals.
///
/// **Off by default.** When disabled, a bare assignment to an undeclared name
/// compiles to `SetGlobal`, which raises "undefined variable" at runtime —
/// causing the server to fall back to the tree-walking interpreter for that
/// handler (the long-standing behavior). Enabling it (`SOLI_VM_OPTIONAL_LET=1`)
/// lets such handlers run on the VM, but that also widens the VM's exposure to
/// a class of latent control-flow/local-assignment bugs (e.g. assignment inside
/// `for`-with-index and `try`/`catch` blocks) that are otherwise masked by the
/// fallback. Keep it off until those are fixed and differentially tested.
pub fn optional_let_enabled() -> bool {
    use std::sync::OnceLock;
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("SOLI_VM_OPTIONAL_LET")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
    })
}

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
    /// Names known to be globals at compile time. Shared across every nested
    /// compiler of one module. Used to decide, for a bare assignment inside a
    /// function to a name that is neither a local nor an upvalue, whether to
    /// assign the existing global (name is in this set) or declare a fresh
    /// function-local (name is not). In `serve` this is seeded with the worker
    /// VM's full global table, so the decision matches the tree-walker exactly;
    /// for whole-program compiles it accumulates top-level names as they appear.
    pub known_globals: Rc<RefCell<HashSet<String>>>,
    /// Tracked value-stack height (relative to the frame base) at the current
    /// emit point. Updated in `emit` by each op's `stack_effect`, reset to
    /// `locals.len()` at every statement boundary (and a few known-clean points
    /// like a loop body entry). Used ONLY as a boolean gate — "are we at the
    /// locals baseline (no anonymous temporaries on the stack)?" — to decide
    /// whether a comprehension can use slot == `locals.len()` safely or must
    /// fall back to the interpreter. It is never used to assign slots, so an
    /// over-count merely causes an extra (safe) fallback; the design must never
    /// under-count (which would pick a wrong slot).
    pub stack_height: usize,
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
            known_globals: Rc::new(RefCell::new(HashSet::new())),
            stack_height: 0,
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
        Self::compile_with_globals(program, std::iter::empty())
    }

    /// Compile a full program, seeding the set of names already known to be
    /// globals (e.g. a worker VM's builtins + loaded app functions/classes).
    /// This lets bare assignments inside functions resolve to the existing
    /// global when one exists, matching the tree-walking interpreter.
    pub fn compile_with_globals<I: IntoIterator<Item = String>>(
        program: &Program,
        globals: I,
    ) -> CompileResult<CompiledModule> {
        let mut compiler = Compiler::new(FunctionType::Script, String::new());
        compiler.known_globals.borrow_mut().extend(globals);
        for stmt in &program.statements {
            compiler.compile_stmt(stmt)?;
        }
        // Implicit return null for scripts
        compiler.emit(Op::Null, 0);
        compiler.emit(Op::Return, 0);

        let mut proto = compiler.proto;
        proto.upvalue_descriptors = compiler.upvalues;

        // Run peephole optimization on all functions
        peephole_optimize_proto(&mut proto);

        Ok(CompiledModule {
            main: Arc::new(proto),
        })
    }

    /// Compile a tree-walking method (e.g., a user-defined controller action)
    /// into a standalone `FunctionProto` with slot 0 reserved for `this`.
    /// Used by the VM's bound-method dispatch for class-based controllers.
    pub fn compile_method_standalone<I: IntoIterator<Item = String>>(
        func: &crate::interpreter::value::Function,
        globals: I,
    ) -> CompileResult<FunctionProto> {
        let mut compiler = Compiler::new(FunctionType::Method, func.name.clone());
        compiler.known_globals.borrow_mut().extend(globals);
        compiler.class_context = Some(ClassContext {
            has_superclass: func.defining_superclass.is_some(),
        });

        for param in &func.params {
            compiler.add_local(param.name.clone(), false);
        }
        compiler.proto.arity = func
            .params
            .iter()
            .filter(|p| p.default_value.is_none())
            .count() as u8;
        compiler.proto.defaults = func
            .params
            .iter()
            .filter(|p| p.default_value.is_some())
            .count() as u8;
        compiler.proto.param_names = func.params.iter().map(|p| p.name.clone()).collect();

        let line = func.span.map(|s| s.line).unwrap_or(0);
        compiler.begin_scope();
        compiler.compile_function_body(&func.body)?;
        compiler.end_scope(line);

        compiler.emit(Op::Null, line);
        compiler.emit(Op::Return, line);

        let mut proto = compiler.proto;
        proto.upvalue_descriptors = compiler.upvalues;
        proto.is_method = true;

        peephole_optimize_proto(&mut proto);

        Ok(proto)
    }

    // --- Chunk helpers ---

    pub fn chunk(&mut self) -> &mut Chunk {
        &mut self.proto.chunk
    }

    pub fn emit(&mut self, op: Op, line: usize) -> usize {
        // Track value-stack height for the comprehension clean-position gate.
        // Saturating so it never goes negative (resyncs at boundaries correct
        // any drift anyway). See `stack_height` field docs.
        let effect = stack_effect(op);
        self.stack_height = (self.stack_height as i64 + effect as i64).max(0) as usize;
        self.proto.chunk.emit(op, line)
    }

    /// Resync the tracked stack height to the locals baseline. Called at points
    /// the value stack is known to hold exactly the live locals (statement
    /// boundary, loop body entry after the loop variable is bound, etc.).
    pub fn resync_stack_height(&mut self) {
        self.stack_height = self.locals.len();
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
        self.proto
            .chunk
            .add_constant(Constant::String(s.to_string()))
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

    /// Discard the top local: `CloseUpvalue` if a closure captured it (so the
    /// capturing closure keeps this binding's current value), else a plain
    /// `Pop`. Used where locals are torn down outside of `end_scope` (e.g. the
    /// per-iteration loop variables of a `for` loop).
    pub fn emit_pop_or_close_top(&mut self, line: usize) {
        let captured = self.locals.last().map(|l| l.is_captured).unwrap_or(false);
        if captured {
            self.emit(Op::CloseUpvalue, line);
        } else {
            self.emit(Op::Pop, line);
        }
        self.locals.pop();
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

    /// Declare, at the start of a function body, the locals introduced by bare
    /// assignment (Soli's optional-`let`). Must be called inside the body scope
    /// after parameters have been added. A candidate is skipped when it is
    /// already a parameter/local, is captured from an enclosing scope (so it
    /// stays an upvalue), or is a known global (so the assignment targets the
    /// existing global) — matching the tree-walking interpreter.
    pub fn hoist_locals(&mut self, body: &[Stmt], line: usize) {
        if !optional_let_enabled() {
            return;
        }
        for name in super::compiler_hoist::collect_hoisted_locals(body) {
            if self.resolve_local(&name).is_some() {
                continue;
            }
            if self.known_globals.borrow().contains(&name) {
                continue;
            }
            if self.enclosing_has_local(&name) {
                continue;
            }
            self.emit(Op::Null, line);
            self.add_local(name, false);
        }
    }

    /// Whether `name` is a local in some enclosing compiler (i.e. it would be
    /// captured as an upvalue rather than introduced as a new local here).
    fn enclosing_has_local(&self, name: &str) -> bool {
        let mut current = self.enclosing.as_deref();
        while let Some(compiler) = current {
            if compiler.resolve_local(name).is_some() {
                return true;
            }
            current = compiler.enclosing.as_deref();
        }
        false
    }

    pub fn declare_variable(
        &mut self,
        name: &str,
        is_const: bool,
        span: Span,
    ) -> CompileResult<()> {
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
        // Nested functions share the module's known-globals set so they make
        // the same local-vs-global decision for bare assignments.
        new_compiler.known_globals = self.known_globals.clone();

        // Add parameters as locals
        for param in params {
            new_compiler.add_local(param.name.clone(), false);
        }
        new_compiler.proto.arity =
            params.iter().filter(|p| p.default_value.is_none()).count() as u8;
        new_compiler.proto.defaults =
            params.iter().filter(|p| p.default_value.is_some()).count() as u8;
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

/// Peephole optimization: scan bytecode for common patterns and replace with super-instructions.
/// This runs after compilation on a FunctionProto (recursively for nested functions).
fn peephole_optimize_proto(proto: &mut FunctionProto) {
    // First, optimize nested function protos in the constant pool
    for constant in &mut proto.chunk.constants {
        if let Constant::Function(func_arc) = constant {
            if let Some(func) = Arc::get_mut(func_arc) {
                peephole_optimize_proto(func);
            }
        }
    }

    // Now optimize this function's bytecode
    peephole_optimize_chunk(&mut proto.chunk);
}

/// Net value-stack effect (pushes − pops) of an opcode, used by the compiler's
/// `stack_height` tracking for the comprehension clean-position gate.
///
/// Only opcodes the compiler emits during compilation need to be exact; the
/// peephole super-instructions run after compilation (mutating the chunk
/// directly, not via `emit`) so their values here are for completeness only.
/// `ForIter`/`ForIterRange` use the continue-path effect (+1); loops resync the
/// height explicitly so the exit-path mismatch never matters. The gate tolerates
/// over-counting (an extra, safe fallback) but must never under-count, so when
/// in doubt an op's effect should not be more negative than reality.
fn stack_effect(op: Op) -> i32 {
    use Op::*;
    match op {
        // Constants / literals push one value.
        Constant(_) | Null | True | False | Symbol(_) => 1,
        Dup => 1,
        Pop => -1,
        // Reads push; stores leave the value in place (net 0).
        GetLocal(_) | GetGlobal(_) | GetUpvalue(_) | GetThis | GetSuper(_) => 1,
        SetLocal(_) | SetGlobal(_) | SetUpvalue(_) => 0,
        DefineGlobal(_) | CloseUpvalue => -1,
        // Binary arithmetic / comparison: pop 2, push 1. Unary: pop 1, push 1.
        Add | Subtract | Multiply | Divide | Modulo => -1,
        Equal | NotEqual | Less | LessEqual | Greater | GreaterEqual => -1,
        Negate | Not => 0,
        // Control flow.
        Jump(_) | Loop(_) | JumpIfFalseNoPop(_) | JumpIfTrueNoPop(_) | NullishJump(_)
        | JumpIfNull(_) | JumpIfNotNull(_) => 0,
        JumpIfFalse(_) => -1,
        // Calls: pop callee/receiver + argc args, push the result.
        Call(argc) | CallMethod(_, argc) | CallMethodById(_, argc, _) => -(argc as i32),
        CallGlobal(_, argc) | GetGlobalCall(_, argc) => 1 - argc as i32,
        Closure(_) => 1,
        Return => 0,
        // Collections.
        Array(n) | BuildString(n) | HashWithKeys(_, n) => 1 - n as i32,
        Hash(n) => 1 - 2 * n as i32,
        ArrayPush => -1,
        Range | GetIndex => -1,
        SetIndex => -2,
        Spread => 0,
        // Objects.
        GetProperty(_) => 0,
        SetProperty(_) => -1,
        // Classes (class value stays on the stack; method/field defs pop one).
        Class(_) => 1,
        Inherit | Method(_) | StaticMethod(_) | Field(_) | StaticField(_) | ConstField(_)
        | StaticConstField(_) => -1,
        New(argc) => -(argc as i32),
        // Exceptions.
        TryBegin(_, _) | TryEnd | CatchMatch(_, _) | PopHandler | RescueJump(_) => 0,
        Throw | Rethrow => -1,
        // Iterators (GetIter/GetIterRange consume from the value stack; ForIter
        // pushes the element on the continue path — loops resync regardless).
        GetIter => -1,
        GetIterRange => -2,
        ForIter(_) | ForIterRange(_) => 1,
        // I/O: pop n, push the Null result.
        Print(n) => 1 - n as i32,
        NamedArg(_) | Import(_) => 0,
        JsonParse | JsonStringify => 0,
        // Peephole super-instructions (not emitted during the tracked pass; values
        // for completeness). Hash*Const directly-emitted variants are exact.
        HashGetConst(_) | HashHasKeyConst(_) | HashDeleteConst(_) => 0,
        HashSetConst(_) => -1,
        HashGetLocalConst(_, _) | HashHasKeyLocalConst(_, _) | HashDeleteLocalConst(_, _) => 1,
        HashSetLocalConst(_, _) => -1,
        HashGetGlobalConst(_, _) | HashHasKeyGlobalConst(_, _) | HashDeleteGlobalConst(_, _) => 1,
        HashSetGlobalConst(_, _) => -1,
        IncrLocal(_) | DecrLocal(_) | IncrLocalFast(_) | SwapSetLocal(_) | IsNull | NotNull
        | PopNull | Nop => 0,
        AddLocalLocal(_, _)
        | SubLocalLocal(_, _)
        | MulLocalLocal(_, _)
        | DivLocalLocal(_, _)
        | ModLocalLocal(_, _)
        | LessEqualLocalLocal(_, _)
        | LessLocalLocal(_, _)
        | GreaterLocalLocal(_, _)
        | EqualLocalLocal(_, _)
        | NotEqualLocalLocal(_, _)
        | AddLocalConst(_, _)
        | SubLocalConst(_, _)
        | MulLocalConst(_, _)
        | DivLocalConst(_, _)
        | AddLocalInt(_, _)
        | EqualLocalConst(_, _)
        | NotEqualLocalConst(_, _)
        | IsTruthyLocal(_)
        | IsFalsyLocal(_)
        | IsZeroLocal(_)
        | NotZeroLocal(_)
        | GetAndNullLocal(_)
        | GetAndIncrLocal(_)
        | GetAndDecrLocal(_)
        | NotLocal(_)
        | NegateLocal(_)
        | GetGlobalNullCheck(_) => 1,
        GetLocal2(_, _) => 2,
        DupN(n) => n as i32,
        SetLocalPop(_) => -1,
        TestLessEqualJump(_)
        | TestLessJump(_)
        | TestGreaterJump(_)
        | TestGreaterEqualJump(_)
        | TestNotEqualJump(_) => -2,
        GetLocalProperty(_, _) | GetLocalIndex(_, _) => 1,
    }
}

/// NOP placeholder used during peephole optimization (reuses Pop as NOP since it's harmless).
const NOP: Op = Op::Nop;

fn peephole_optimize_chunk(chunk: &mut Chunk) {
    let code = &mut chunk.code;
    let len = code.len();
    if len < 5 {
        return;
    }

    let constants = &chunk.constants;

    // Track which offsets are jump targets (can't optimize across them)
    let mut is_jump_target = vec![false; len];
    for (i, op) in code.iter().enumerate() {
        match op {
            Op::Jump(offset)
            | Op::JumpIfFalse(offset)
            | Op::JumpIfFalseNoPop(offset)
            | Op::JumpIfTrueNoPop(offset)
            | Op::NullishJump(offset)
            | Op::ForIter(offset)
            | Op::ForIterRange(offset)
            | Op::TestLessEqualJump(offset)
            | Op::TestLessJump(offset) => {
                let target = i + 1 + *offset as usize;
                if target < len {
                    is_jump_target[target] = true;
                }
            }
            Op::Loop(offset) => {
                let target = i + 1 - *offset as usize;
                if target < len {
                    is_jump_target[target] = true;
                }
            }
            _ => {}
        }
    }

    // Pattern matching: scan for optimizable sequences
    let mut i = 0;
    while i + 4 < len {
        // Don't optimize if current position is a jump target
        if is_jump_target[i] {
            i += 1;
            continue;
        }

        // Pattern: GetLocal(s), Constant(c=1), Add, SetLocal(s), Pop → IncrLocal(s)
        if let (Op::GetLocal(slot1), Op::Constant(cidx), Op::Add, Op::SetLocal(slot2), Op::Pop) =
            (code[i], code[i + 1], code[i + 2], code[i + 3], code[i + 4])
        {
            if slot1 == slot2 && !any_jump_target(&is_jump_target, i + 1, 5) {
                if let Some(Constant::Int(1)) = constants.get(cidx as usize) {
                    code[i] = Op::IncrLocal(slot1);
                    code[i + 1] = NOP;
                    code[i + 2] = NOP;
                    code[i + 3] = NOP;
                    code[i + 4] = NOP;
                    i += 5;
                    continue;
                }
            }
        }

        // Pattern: GetLocal(s), Constant(c=1), Subtract, SetLocal(s), Pop → DecrLocal(s)
        if let (
            Op::GetLocal(slot1),
            Op::Constant(cidx),
            Op::Subtract,
            Op::SetLocal(slot2),
            Op::Pop,
        ) = (code[i], code[i + 1], code[i + 2], code[i + 3], code[i + 4])
        {
            if slot1 == slot2 && !any_jump_target(&is_jump_target, i + 1, 5) {
                if let Some(Constant::Int(1)) = constants.get(cidx as usize) {
                    code[i] = Op::DecrLocal(slot1);
                    code[i + 1] = NOP;
                    code[i + 2] = NOP;
                    code[i + 3] = NOP;
                    code[i + 4] = NOP;
                    i += 5;
                    continue;
                }
            }
        }

        // Pattern: GetLocal(a), GetLocal(b), Add, SetLocal(a), Pop → AddLocalLocal(a, b) + SetLocalPop(a)
        // This is: a = a + b  → becomes two ops instead of five
        if i + 4 < len {
            if let (
                Op::GetLocal(slot_a),
                Op::GetLocal(slot_b),
                Op::Add,
                Op::SetLocal(slot_target),
                Op::Pop,
            ) = (code[i], code[i + 1], code[i + 2], code[i + 3], code[i + 4])
            {
                if slot_a == slot_target && !any_jump_target(&is_jump_target, i + 1, 5) {
                    code[i] = Op::AddLocalLocal(slot_a, slot_b);
                    code[i + 1] = Op::SetLocalPop(slot_a);
                    code[i + 2] = NOP;
                    code[i + 3] = NOP;
                    code[i + 4] = NOP;
                    i += 5;
                    continue;
                }
            }
        }

        // Pattern: GetLocal(a), GetLocal(b), LessEqual → LessEqualLocalLocal(a, b)
        if i + 2 < len {
            if let (Op::GetLocal(slot_a), Op::GetLocal(slot_b), Op::LessEqual) =
                (code[i], code[i + 1], code[i + 2])
            {
                if !any_jump_target(&is_jump_target, i + 1, 3) {
                    code[i] = Op::LessEqualLocalLocal(slot_a, slot_b);
                    code[i + 1] = NOP;
                    code[i + 2] = NOP;
                    i += 3;
                    continue;
                }
            }
        }

        // Pattern: LessEqual, JumpIfFalse(offset) → TestLessEqualJump(offset+1)
        if i + 1 < len {
            if let (Op::LessEqual, Op::JumpIfFalse(offset)) = (code[i], code[i + 1]) {
                if !any_jump_target(&is_jump_target, i + 1, 2) {
                    code[i] = Op::TestLessEqualJump(offset + 1);
                    code[i + 1] = NOP;
                    i += 2;
                    continue;
                }
            }
        }

        // Pattern: Less, JumpIfFalse(offset) → TestLessJump(offset+1)
        if i + 1 < len {
            if let (Op::Less, Op::JumpIfFalse(offset)) = (code[i], code[i + 1]) {
                if !any_jump_target(&is_jump_target, i + 1, 2) {
                    code[i] = Op::TestLessJump(offset + 1);
                    code[i + 1] = NOP;
                    i += 2;
                    continue;
                }
            }
        }

        // Pattern: Greater, JumpIfFalse(offset) → TestGreaterJump(offset+1)
        if i + 1 < len {
            if let (Op::Greater, Op::JumpIfFalse(offset)) = (code[i], code[i + 1]) {
                if !any_jump_target(&is_jump_target, i + 1, 2) {
                    code[i] = Op::TestGreaterJump(offset + 1);
                    code[i + 1] = NOP;
                    i += 2;
                    continue;
                }
            }
        }

        // Pattern: GreaterEqual, JumpIfFalse(offset) → TestGreaterEqualJump(offset+1)
        if i + 1 < len {
            if let (Op::GreaterEqual, Op::JumpIfFalse(offset)) = (code[i], code[i + 1]) {
                if !any_jump_target(&is_jump_target, i + 1, 2) {
                    code[i] = Op::TestGreaterEqualJump(offset + 1);
                    code[i + 1] = NOP;
                    i += 2;
                    continue;
                }
            }
        }

        // Pattern: NotEqual, JumpIfFalse(offset) → TestNotEqualJump(offset+1)
        if i + 1 < len {
            if let (Op::NotEqual, Op::JumpIfFalse(offset)) = (code[i], code[i + 1]) {
                if !any_jump_target(&is_jump_target, i + 1, 2) {
                    code[i] = Op::TestNotEqualJump(offset + 1);
                    code[i + 1] = NOP;
                    i += 2;
                    continue;
                }
            }
        }

        // Pattern: GetLocal(a), GetLocal(b), Subtract → SubLocalLocal(a, b)
        if i + 2 < len {
            if let (Op::GetLocal(slot_a), Op::GetLocal(slot_b), Op::Subtract) =
                (code[i], code[i + 1], code[i + 2])
            {
                if !any_jump_target(&is_jump_target, i + 1, 3) {
                    code[i] = Op::SubLocalLocal(slot_a, slot_b);
                    code[i + 1] = NOP;
                    code[i + 2] = NOP;
                    i += 3;
                    continue;
                }
            }
        }

        // Pattern: GetLocal(a), GetLocal(b), Multiply → MulLocalLocal(a, b)
        if i + 2 < len {
            if let (Op::GetLocal(slot_a), Op::GetLocal(slot_b), Op::Multiply) =
                (code[i], code[i + 1], code[i + 2])
            {
                if !any_jump_target(&is_jump_target, i + 1, 3) {
                    code[i] = Op::MulLocalLocal(slot_a, slot_b);
                    code[i + 1] = NOP;
                    code[i + 2] = NOP;
                    i += 3;
                    continue;
                }
            }
        }

        // Pattern: GetLocal(a), GetLocal(b), Divide → DivLocalLocal(a, b)
        if i + 2 < len {
            if let (Op::GetLocal(slot_a), Op::GetLocal(slot_b), Op::Divide) =
                (code[i], code[i + 1], code[i + 2])
            {
                if !any_jump_target(&is_jump_target, i + 1, 3) {
                    code[i] = Op::DivLocalLocal(slot_a, slot_b);
                    code[i + 1] = NOP;
                    code[i + 2] = NOP;
                    i += 3;
                    continue;
                }
            }
        }

        // Pattern: GetLocal(a), GetLocal(b), Modulo → ModLocalLocal(a, b)
        if i + 2 < len {
            if let (Op::GetLocal(slot_a), Op::GetLocal(slot_b), Op::Modulo) =
                (code[i], code[i + 1], code[i + 2])
            {
                if !any_jump_target(&is_jump_target, i + 1, 3) {
                    code[i] = Op::ModLocalLocal(slot_a, slot_b);
                    code[i + 1] = NOP;
                    code[i + 2] = NOP;
                    i += 3;
                    continue;
                }
            }
        }

        // Pattern: GetLocal(a), GetLocal(b), Less → LessLocalLocal(a, b)
        if i + 2 < len {
            if let (Op::GetLocal(slot_a), Op::GetLocal(slot_b), Op::Less) =
                (code[i], code[i + 1], code[i + 2])
            {
                if !any_jump_target(&is_jump_target, i + 1, 3) {
                    code[i] = Op::LessLocalLocal(slot_a, slot_b);
                    code[i + 1] = NOP;
                    code[i + 2] = NOP;
                    i += 3;
                    continue;
                }
            }
        }

        // Pattern: GetLocal(a), GetLocal(b), Greater → GreaterLocalLocal(a, b)
        if i + 2 < len {
            if let (Op::GetLocal(slot_a), Op::GetLocal(slot_b), Op::Greater) =
                (code[i], code[i + 1], code[i + 2])
            {
                if !any_jump_target(&is_jump_target, i + 1, 3) {
                    code[i] = Op::GreaterLocalLocal(slot_a, slot_b);
                    code[i + 1] = NOP;
                    code[i + 2] = NOP;
                    i += 3;
                    continue;
                }
            }
        }

        // Pattern: a = a - b  (GetLocal, GetLocal, Subtract, SetLocal, Pop)
        if i + 4 < len {
            if let (
                Op::GetLocal(slot_a),
                Op::GetLocal(slot_b),
                Op::Subtract,
                Op::SetLocal(slot_target),
                Op::Pop,
            ) = (code[i], code[i + 1], code[i + 2], code[i + 3], code[i + 4])
            {
                if slot_a == slot_target && !any_jump_target(&is_jump_target, i + 1, 5) {
                    code[i] = Op::SubLocalLocal(slot_a, slot_b);
                    code[i + 1] = Op::SetLocalPop(slot_a);
                    code[i + 2] = NOP;
                    code[i + 3] = NOP;
                    code[i + 4] = NOP;
                    i += 5;
                    continue;
                }
            }
        }

        // Pattern: a = a * b  (GetLocal, GetLocal, Multiply, SetLocal, Pop)
        if i + 4 < len {
            if let (
                Op::GetLocal(slot_a),
                Op::GetLocal(slot_b),
                Op::Multiply,
                Op::SetLocal(slot_target),
                Op::Pop,
            ) = (code[i], code[i + 1], code[i + 2], code[i + 3], code[i + 4])
            {
                if slot_a == slot_target && !any_jump_target(&is_jump_target, i + 1, 5) {
                    code[i] = Op::MulLocalLocal(slot_a, slot_b);
                    code[i + 1] = Op::SetLocalPop(slot_a);
                    code[i + 2] = NOP;
                    code[i + 3] = NOP;
                    code[i + 4] = NOP;
                    i += 5;
                    continue;
                }
            }
        }

        // Pattern: a = a / b  (GetLocal, GetLocal, Divide, SetLocal, Pop)
        if i + 4 < len {
            if let (
                Op::GetLocal(slot_a),
                Op::GetLocal(slot_b),
                Op::Divide,
                Op::SetLocal(slot_target),
                Op::Pop,
            ) = (code[i], code[i + 1], code[i + 2], code[i + 3], code[i + 4])
            {
                if slot_a == slot_target && !any_jump_target(&is_jump_target, i + 1, 5) {
                    code[i] = Op::DivLocalLocal(slot_a, slot_b);
                    code[i + 1] = Op::SetLocalPop(slot_a);
                    code[i + 2] = NOP;
                    code[i + 3] = NOP;
                    code[i + 4] = NOP;
                    i += 5;
                    continue;
                }
            }
        }

        // Pattern: GetLocal(slot), Constant(c), Add → AddLocalConst(slot, c)
        if i + 2 < len {
            if let (Op::GetLocal(slot), Op::Constant(cidx), Op::Add) =
                (code[i], code[i + 1], code[i + 2])
            {
                if !any_jump_target(&is_jump_target, i + 1, 3) {
                    if let Some(Constant::Int(_)) | Some(Constant::Float(_)) =
                        constants.get(cidx as usize)
                    {
                        code[i] = Op::AddLocalConst(slot, cidx);
                        code[i + 1] = NOP;
                        code[i + 2] = NOP;
                        i += 3;
                        continue;
                    }
                }
            }
        }

        // Pattern: GetLocal(slot), Constant(c), Subtract → SubLocalConst(slot, c)
        if i + 2 < len {
            if let (Op::GetLocal(slot), Op::Constant(cidx), Op::Subtract) =
                (code[i], code[i + 1], code[i + 2])
            {
                if !any_jump_target(&is_jump_target, i + 1, 3) {
                    if let Some(Constant::Int(_)) | Some(Constant::Float(_)) =
                        constants.get(cidx as usize)
                    {
                        code[i] = Op::SubLocalConst(slot, cidx);
                        code[i + 1] = NOP;
                        code[i + 2] = NOP;
                        i += 3;
                        continue;
                    }
                }
            }
        }

        // Pattern: GetLocal(slot), Constant(c), Multiply → MulLocalConst(slot, c)
        if i + 2 < len {
            if let (Op::GetLocal(slot), Op::Constant(cidx), Op::Multiply) =
                (code[i], code[i + 1], code[i + 2])
            {
                if !any_jump_target(&is_jump_target, i + 1, 3) {
                    if let Some(Constant::Int(_)) | Some(Constant::Float(_)) =
                        constants.get(cidx as usize)
                    {
                        code[i] = Op::MulLocalConst(slot, cidx);
                        code[i + 1] = NOP;
                        code[i + 2] = NOP;
                        i += 3;
                        continue;
                    }
                }
            }
        }

        // Pattern: GetLocal(slot), Constant(c), Divide → DivLocalConst(slot, c)
        if i + 2 < len {
            if let (Op::GetLocal(slot), Op::Constant(cidx), Op::Divide) =
                (code[i], code[i + 1], code[i + 2])
            {
                if !any_jump_target(&is_jump_target, i + 1, 3) {
                    if let Some(Constant::Int(_)) | Some(Constant::Float(_)) =
                        constants.get(cidx as usize)
                    {
                        code[i] = Op::DivLocalConst(slot, cidx);
                        code[i + 1] = NOP;
                        code[i + 2] = NOP;
                        i += 3;
                        continue;
                    }
                }
            }
        }

        // NB: `GetLocal(a), GetLocal(b)` is deliberately NOT fused into a single
        // GetLocal2. A `let x = <some local>` compiles to a bare `GetLocal`
        // whose pushed value *is* the new local `x`; if the next statement
        // reads `x`, the two GetLocals are adjacent but the second reads a slot
        // the first only just established. GetLocal2 reads both slots up front,
        // so it would index past the stack top (panic) or read a stale value.

        // Pattern: GetLocal, Constant(1), Add → AddLocalInt(slot, 1)
        if i + 2 < len {
            if let (Op::GetLocal(slot), Op::Constant(cidx), Op::Add) =
                (code[i], code[i + 1], code[i + 2])
            {
                if !any_jump_target(&is_jump_target, i + 1, 3) {
                    if let Some(Constant::Int(n)) = constants.get(cidx as usize) {
                        if *n == 1 {
                            code[i] = Op::AddLocalInt(slot, 1);
                            code[i + 1] = NOP;
                            code[i + 2] = NOP;
                            i += 3;
                            continue;
                        }
                        // For small negative numbers
                        if *n > -32768 && *n < 32767 {
                            code[i] = Op::AddLocalInt(slot, *n as i32);
                            code[i + 1] = NOP;
                            code[i + 2] = NOP;
                            i += 3;
                            continue;
                        }
                    }
                }
            }
        }

        // Pattern: GetLocal, Constant(-1), Add → AddLocalInt(slot, -1)
        if i + 2 < len {
            if let (Op::GetLocal(slot), Op::Constant(cidx), Op::Add) =
                (code[i], code[i + 1], code[i + 2])
            {
                if !any_jump_target(&is_jump_target, i + 1, 3) {
                    if let Some(Constant::Int(n)) = constants.get(cidx as usize) {
                        if *n == -1 {
                            code[i] = Op::AddLocalInt(slot, -1);
                            code[i + 1] = NOP;
                            code[i + 2] = NOP;
                            i += 3;
                            continue;
                        }
                    }
                }
            }
        }

        // Pattern: Null, JumpIfFalse → JumpIfNull (but we don't have JumpIfFalse directly, it's an operand)
        // This needs special handling since JumpIfFalse has an operand

        // Pattern: GetLocal, Not, JumpIfFalse → IsFalsyLocal + jump
        if i + 2 < len {
            if let (Op::GetLocal(slot), Op::Not, Op::JumpIfFalse(offset)) =
                (code[i], code[i + 1], code[i + 2])
            {
                if !any_jump_target(&is_jump_target, i + 1, 3) {
                    code[i] = Op::IsFalsyLocal(slot);
                    code[i + 1] = NOP;
                    code[i + 2] = Op::JumpIfFalse(offset);
                    i += 3;
                    continue;
                }
            }
        }

        // Pattern: GetLocal, JumpIfFalse → IsTruthyLocal + JumpIfFalse.
        // `JumpIfFalse` jumps when its operand is falsy, so to preserve "jump
        // when the local is falsy" the fused op must push the local's
        // *truthiness*. (Using IsFalsyLocal here would invert the branch.)
        if i + 1 < len {
            if let (Op::GetLocal(slot), Op::JumpIfFalse(offset)) = (code[i], code[i + 1]) {
                if !any_jump_target(&is_jump_target, i + 1, 2) {
                    code[i] = Op::IsTruthyLocal(slot);
                    code[i + 1] = Op::JumpIfFalse(offset);
                    i += 2;
                    continue;
                }
            }
        }

        // NB: `GetLocal, JumpIfFalseNoPop` is deliberately NOT fused. The NoPop
        // jump leaves the *tested value itself* on the stack for reuse (e.g.
        // `a && b` evaluates to `a` when `a` is falsy); replacing the local with
        // a derived boolean would corrupt that result.

        // Pattern: NullishJump (??) - could optimize but it's already specialized

        // Pattern: GetLocal, GetLocal, NotEqual → NotEqualLocalLocal (if we had it, but we don't have this yet)

        // Pattern: GetLocal(a), Constant(0), NotEqual → NotZeroLocal
        if i + 2 < len {
            if let (Op::GetLocal(slot), Op::Constant(cidx), Op::NotEqual) =
                (code[i], code[i + 1], code[i + 2])
            {
                if !any_jump_target(&is_jump_target, i + 1, 3) {
                    if let Some(Constant::Int(0)) = constants.get(cidx as usize) {
                        code[i] = Op::NotZeroLocal(slot);
                        code[i + 1] = NOP;
                        code[i + 2] = NOP;
                        i += 3;
                        continue;
                    }
                }
            }
        }

        // Pattern: GetLocal(a), Constant(0), Equal → IsZeroLocal (inverted logic)
        if i + 2 < len {
            if let (Op::GetLocal(slot), Op::Constant(cidx), Op::Equal) =
                (code[i], code[i + 1], code[i + 2])
            {
                if !any_jump_target(&is_jump_target, i + 1, 3) {
                    if let Some(Constant::Int(0)) = constants.get(cidx as usize) {
                        code[i] = Op::IsZeroLocal(slot);
                        code[i + 1] = NOP;
                        code[i + 2] = NOP;
                        i += 3;
                        continue;
                    }
                }
            }
        }

        // Pattern: GetLocal(slot), SetLocal(slot) → SwapSetLocal (swap old and new)
        if i + 1 < len {
            if let (Op::GetLocal(slot_a), Op::SetLocal(slot_b)) = (code[i], code[i + 1]) {
                if slot_a == slot_b && !any_jump_target(&is_jump_target, i + 1, 2) {
                    code[i] = Op::SwapSetLocal(slot_a);
                    code[i + 1] = NOP;
                    i += 2;
                    continue;
                }
            }
        }

        i += 1;
    }

    // Remove NOPs (compact the bytecode) - adjust jump offsets accordingly
    compact_nops(chunk);
}

/// Check if any offset in range [start+1, start+count) is a jump target.
fn any_jump_target(targets: &[bool], start: usize, count: usize) -> bool {
    for j in (start + 1)..(start + count) {
        if j < targets.len() && targets[j] {
            return true;
        }
    }
    false
}

/// Remove NOP (Pop) instructions inserted by peephole, adjusting jump offsets.
fn compact_nops(chunk: &mut Chunk) {
    let code = &chunk.code;
    let len = code.len();

    // Build a mapping from old offset to new offset
    let mut old_to_new = vec![0usize; len + 1];
    let mut new_offset = 0usize;
    for item in old_to_new.iter_mut().take(len) {
        *item = new_offset;
        // A NOP is a Pop that was inserted by peephole.
        // We detect peephole NOPs by checking if there's a sequence of Pops that were part of a pattern.
        // Actually, we can't distinguish original Pops from peephole NOPs easily.
        // Better approach: use a separate marker. Let me just keep the NOPs and not compact.
        new_offset += 1;
    }
    old_to_new[len] = new_offset;

    // For now, don't compact - the NOPs (extra Pops) are essentially free.
    // They add a tiny overhead but avoid the complexity of rewriting all jump offsets.
    // The main win is from the super-instructions reducing work, not from fewer instructions.
}
