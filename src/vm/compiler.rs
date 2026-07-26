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
    /// `finally` blocks whose `try` statement encloses the current emit point,
    /// outermost first.
    ///
    /// `finally` has no runtime support — `ExceptionHandler::finally_ip` is
    /// stored and never read — so it is compiled by *inlining* the block on
    /// every edge that leaves the `try`. A `return` is the edge that has to
    /// consult this: it leaves the frame without passing through the code that
    /// follows, so the block is emitted immediately before `Op::Return`.
    ///
    /// Per-`Compiler`, and `start_function` swaps in a fresh one, so a `return`
    /// inside a lambda nested in a `try` correctly sees an empty stack rather
    /// than the enclosing function's `finally`.
    pub try_stack: Vec<TryFrame>,
    /// Globals this *program* declares, as opposed to ones that merely exist.
    ///
    /// `known_globals` cannot answer that question: in `serve` it is seeded
    /// with the worker's entire global table, builtins included, so asking it
    /// whether `next` is a user global says yes and the builtin stops being
    /// recognised — compiled fine at the CLI, silently wrong under the server.
    /// This set only ever grows from a `let`/`const` at global scope or a
    /// function declaration, so it means what it says in both modes.
    pub program_globals: Rc<RefCell<HashSet<String>>>,
}

/// One enclosing `try`, as the compiler sees it at the current emit point.
///
/// `break`, `next` and `return` all have to leave a `try` without passing
/// through the code that would normally tidy it up, and each needs both facts
/// together: how many exception handlers that `try` registered (so they can be
/// popped) and whether it has a `finally` (so it can be run). Tracking those in
/// two parallel stacks made per-level unwinding impossible to express, which is
/// why `break` inside a `try` was refused rather than compiled.
#[derive(Debug, Clone)]
pub struct TryFrame {
    /// Handlers this `try` pushed and that are still live here. Two while the
    /// try body runs under a `finally` (the outer finally pad plus the inner
    /// catch), one otherwise; a catch clause runs with its own already popped.
    pub handlers: usize,
    /// Its `finally` block, if any.
    pub finally: Option<crate::ast::Stmt>,
}

/// What `declare_variable` did with the name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclareOutcome {
    /// A fresh local: the initializer's value is already sitting in its slot.
    Declared,
    /// The name already exists in this scope, so this `let` is an assignment —
    /// the value is on top of the stack and belongs in the given slot.
    Reassigns(u16),
}

#[derive(Debug, Clone)]
pub struct LoopContext {
    pub start: usize,
    pub break_patches: Vec<usize>,
    pub enclosing: Option<Box<LoopContext>>,
    /// `locals.len()` on entry. `break` pops everything the body pushed above
    /// this before jumping, the way falling out of the loop would have.
    pub locals_base: usize,
    /// A `for` loop parked an iterator on `iter_stack`; `break` must discard it
    /// (`ForIter` only does so when the sequence runs out).
    pub has_iterator: bool,
    /// `try_stack.len()` on entry. Anything above this is a `try` the loop
    /// encloses, which `break`/`next` unwind on their way out.
    pub try_base: usize,
    /// Jumps emitted by `next`, patched to the loop's continue point.
    pub continue_patches: Vec<usize>,
    /// `locals.len()` at the point a `next` should unwind *to*.
    ///
    /// One deeper than `locals_base` in a `for`: the loop variable is torn down
    /// by the end-of-body code that `next` jumps to, so `next` must leave it in
    /// place. In a `while` there is no loop variable and the two coincide.
    pub continue_locals_base: usize,
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
            try_stack: Vec::new(),
            program_globals: Rc::new(RefCell::new(HashSet::new())),
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

        for param in func.params.iter() {
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
        compiler.proto.defaults_mask = defaults_mask(&func.params);

        let line = func.span.map(|s| s.line as usize).unwrap_or(0);
        compiler.begin_scope();
        compiler.emit_param_defaults(&func.params)?;
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
            .add_constant(Constant::String(s.to_string().into()))
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
    ) -> CompileResult<DeclareOutcome> {
        if self.scope_depth == 0 {
            return Ok(DeclareOutcome::Declared); // globals are handled differently
        }
        // Re-`let` of the same name in the same scope. The tree-walker allows
        // it — `define_or_update` writes the existing binding — so refusing
        // here demoted a whole handler for code that runs perfectly well, and
        // two of this repo's own spec files do it.
        //
        // Only the unambiguous case is taken over: `let` on top of a
        // non-`const` local, which means "assign". Anything involving `const`
        // still refuses and falls back, because the tree-walker's behaviour
        // there is not consistent enough to reproduce confidently — `const x =
        // 1; let x = 2` keeps 1, while `const x = 1; const x = 2` gives 2. The
        // engine that defines those semantics should keep defining them.
        for (idx, local) in self.locals.iter().enumerate().rev() {
            if local.depth != -1 && local.depth < self.scope_depth {
                break;
            }
            if local.name == name {
                if local.is_const || is_const {
                    return Err(CompileError::new(
                        format!("Variable '{}' already declared in this scope", name),
                        span,
                    ));
                }
                return Ok(DeclareOutcome::Reassigns(idx as u16));
            }
        }
        self.add_local(name.to_string(), is_const);
        Ok(DeclareOutcome::Declared)
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
        new_compiler.program_globals = self.program_globals.clone();

        // Add parameters as locals
        for param in params {
            new_compiler.add_local(param.name.clone(), false);
        }
        new_compiler.proto.arity =
            params.iter().filter(|p| p.default_value.is_none()).count() as u8;
        new_compiler.proto.defaults =
            params.iter().filter(|p| p.default_value.is_some()).count() as u8;
        new_compiler.proto.param_names = params.iter().map(|p| p.name.clone()).collect();
        new_compiler.proto.defaults_mask = defaults_mask(params);

        // Swap self with the new compiler, storing self as enclosing
        let old = std::mem::replace(self, new_compiler);
        self.enclosing = Some(Box::new(old));
        // Return a dummy — we'll use finish_function to unwrap
        Box::new(Compiler::new(FunctionType::Script, String::new()))
    }

    /// Emit the parameter-default prologue at the top of the function body.
    ///
    /// For each parameter that declares a default, emit
    ///
    /// ```text
    ///     JumpIfParamSupplied(i, past)   ; caller passed it — keep their value
    ///     <default expression>
    ///     SetLocalPop(i + 1)             ; slot 0 is `this`/callee, params follow
    ///   past:
    /// ```
    ///
    /// The frame's `supplied` bitmask decides, so a caller that explicitly
    /// passes `null` keeps the `null` (it occupies the slot) while an omitted
    /// argument gets the default — the same distinction the tree-walking
    /// interpreter draws by only filling parameters past the positional count.
    ///
    /// Emitting in parameter order means a later default may reference an
    /// earlier parameter (`def f(a, b = a + 1)`): slot `a` is already bound by
    /// the time `b`'s default runs.
    pub fn emit_param_defaults(&mut self, params: &[Parameter]) -> CompileResult<()> {
        for (i, param) in params.iter().enumerate() {
            let Some(default) = &param.default_value else {
                continue;
            };
            // Slot 0 is reserved (`this` for methods, callee otherwise), so the
            // parameter at index `i` lives in local slot `i + 1`.
            let Ok(slot) = u16::try_from(i + 1) else {
                return Err(CompileError::new(
                    "too many parameters for default-value prologue",
                    param.span,
                ));
            };
            let line = param.span.line as usize;
            let jump = self.emit_jump(Op::JumpIfParamSupplied(i as u16, u16::MAX), line);
            self.compile_expr(default)?;
            self.emit(Op::SetLocalPop(slot), line);
            self.patch_jump(jump);
        }
        Ok(())
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

    pub fn begin_loop(&mut self, start: usize, has_iterator: bool) {
        let enclosing = self.loop_context.take().map(Box::new);
        self.loop_context = Some(LoopContext {
            start,
            break_patches: Vec::new(),
            enclosing,
            locals_base: self.locals.len(),
            has_iterator,
            try_base: self.try_stack.len(),
            continue_patches: Vec::new(),
            // Overwritten by `for` once its loop variable is declared.
            continue_locals_base: self.locals.len(),
        });
    }

    /// Record where `next` should land, and what it must unwind to.
    pub fn set_continue_target(&mut self) {
        let base = self.locals.len();
        if let Some(ref mut ctx) = self.loop_context {
            ctx.continue_locals_base = base;
        }
    }

    /// Emit the teardown a `next` needs, then the jump to the continue point.
    ///
    /// Unlike `break` the iterator stays — the same loop is about to take its
    /// next element — but the body's own locals still have to come off, or each
    /// skipped iteration would leave its locals behind and the stack would grow
    /// for the life of the loop.
    ///
    /// Returns `false` for a `next` inside a `try` the loop does not enclose,
    /// for the same reason `break` refuses one.
    pub fn emit_continue(&mut self, line: usize) -> bool {
        if self.loop_context.is_none() {
            return self.emit_loopless_exit(line);
        }
        let Some(ctx) = self.loop_context.as_ref() else {
            return false;
        };
        let (base, try_base) = (ctx.continue_locals_base, ctx.try_base);
        if self.unwind_trys_to(try_base, line).is_err() {
            return false;
        }

        for idx in (base..self.locals.len()).rev() {
            if self.locals[idx].is_captured {
                self.emit(Op::CloseUpvalue, line);
            } else {
                self.emit(Op::Pop, line);
            }
        }
        let patch = self.emit_jump(Op::Jump(0), line);
        if let Some(ref mut ctx) = self.loop_context {
            ctx.continue_patches.push(patch);
        }
        true
    }

    /// Patch every `next` in the current loop to land here.
    pub fn patch_continues(&mut self) {
        let patches = match self.loop_context {
            Some(ref mut ctx) => std::mem::take(&mut ctx.continue_patches),
            None => return,
        };
        for patch in patches {
            self.patch_jump(patch);
        }
    }

    /// `break` or `next` with no enclosing loop *in this function*.
    ///
    /// The tree-walker absorbs both at the function boundary — `call_function`
    /// maps `ControlFlow::Break`/`Continue` to a null return — so inside a
    /// lambda they stop the lambda body without touching the loop that is
    /// running outside it:
    ///
    /// ```soli
    /// for n in [1, 2] {
    ///     [10, 20, 30].each(fn(x) { break })   // stops each callback…
    ///     seen.push(n)                          // …the for loop keeps going
    /// }
    /// ```
    ///
    /// A lambda gets its own `Compiler`, so its `loop_context` is empty even
    /// when a loop encloses the lambda *lexically* — which is exactly the
    /// distinction that makes this correct rather than a fallback.
    ///
    /// At script top level there is no frame to return from, and the
    /// tree-walker's top-level loop ignores both, so this emits nothing.
    fn emit_loopless_exit(&mut self, line: usize) -> bool {
        if self.function_type == FunctionType::Script {
            return true;
        }
        self.emit(Op::Null, line);
        self.emit(Op::Return, line);
        true
    }

    /// Leave every `try` the loop encloses, innermost first.
    ///
    /// Per level: drop that `try`'s handlers, then run its `finally`. In that
    /// order, so an exception raised inside the `finally` reaches the handler
    /// *outside* the `try` being left rather than the one it belongs to — the
    /// same order the normal fall-through path emits.
    ///
    /// The stack is left as it was found: this emits an *exit* path, and the
    /// code that follows the `break` still compiles inside those same `try`
    /// blocks. While a level's `finally` is emitted, only the levels outside it
    /// are active, so a `return` in there runs the outer ones and not itself.
    fn unwind_trys_to(&mut self, base: usize, line: usize) -> CompileResult<()> {
        let frames: Vec<TryFrame> = self.try_stack[base..].to_vec();
        for (i, frame) in frames.iter().enumerate().rev() {
            for _ in 0..frame.handlers {
                self.emit(Op::PopHandler, line);
            }
            if let Some(body) = frame.finally.clone() {
                let outer = self.try_stack[..base + i].to_vec();
                let saved = std::mem::replace(&mut self.try_stack, outer);
                let result = self.compile_stmt(&body);
                self.try_stack = saved;
                result?;
            }
        }
        Ok(())
    }

    /// Emit the teardown a `break` needs, then the jump out of the loop.
    ///
    /// Everything the body pushed has to come off exactly as it would when the
    /// loop ends normally: body locals (closing upvalues that a closure
    /// captured, so per-iteration bindings stay distinct) and, for a `for`
    /// loop, the iterator. `self.locals` is left alone — the statements after
    /// the `break` still refer to those slots on the fall-through path.
    ///
    /// Returns `false` when the `break` sits inside a `try` the loop does not
    /// enclose; the caller refuses compilation so the handler falls back rather
    /// than leaving a live handler pointing into an abandoned loop.
    pub fn emit_break(&mut self, line: usize) -> bool {
        if self.loop_context.is_none() {
            return self.emit_loopless_exit(line);
        }
        let Some(ctx) = self.loop_context.as_ref() else {
            return false;
        };
        let (locals_base, has_iterator, try_base) =
            (ctx.locals_base, ctx.has_iterator, ctx.try_base);
        if self.unwind_trys_to(try_base, line).is_err() {
            return false;
        }

        for idx in (locals_base..self.locals.len()).rev() {
            if self.locals[idx].is_captured {
                self.emit(Op::CloseUpvalue, line);
            } else {
                self.emit(Op::Pop, line);
            }
        }
        if has_iterator {
            self.emit(Op::PopIter, line);
        }
        let patch = self.emit_jump(Op::Jump(0), line);
        self.add_break_patch(patch);
        true
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
/// Bitmask of which parameters declare a default value (bit `i` = parameter
/// `i`). Parameters past bit 63 are reported as defaulted — see
/// [`FunctionProto::defaults_mask`].
fn defaults_mask(params: &[Parameter]) -> u64 {
    let mut mask = 0u64;
    for (i, param) in params.iter().enumerate() {
        if i >= 64 {
            mask |= !0u64 << 63;
            break;
        }
        if param.default_value.is_some() {
            mask |= 1u64 << i;
        }
    }
    mask
}

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
        Jump(_)
        | Loop(_)
        | JumpIfFalseNoPop(_)
        | JumpIfTrueNoPop(_)
        | NullishJump(_)
        | JumpIfNull(_)
        | JumpIfNotNull(_)
        | JumpIfParamSupplied(_, _) => 0,
        JumpIfFalse(_) => -1,
        // Calls: pop callee/receiver + argc args, push the result.
        Call(argc) | CallMethod(_, argc) | CallMethodById(_, argc, _) => -(argc as i32),
        // Same shape as Call/New: the callee plus argc slots collapse to one result.
        CallNamed(argc, _) | NewNamed(argc, _) => -(argc as i32),
        // [this, args…] collapse to the result: net -argc.
        CallSuperInit(argc) | CallSuperMethod(_, argc) => -(argc as i32),
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
        Import(_) => 0,
        JsonParse | JsonStringify => 0,
        // Peephole super-instructions (not emitted during the tracked pass; values
        // for completeness). Hash*Const directly-emitted variants are exact.
        HashGetConst(_) | HashHasKeyConst(_) | HashDeleteConst(_) => 0,
        HashSetConst(_) => -1,
        HashGetLocalConst(_, _) | HashHasKeyLocalConst(_, _) | HashDeleteLocalConst(_, _) => 1,
        AddLocalsInPlace(_, _) => 0,
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
        // Touches `iter_stack`, not the value stack.
        PopIter => 0,
        // Peeks the subject and branches; leaves the stack alone.
        MatchType(_, _) => 0,
        // Reads a payload field out of a slot and pushes it.
        EnumPayload(_, _, _) => 1,
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
            // Keep in step with `FORWARD_BRANCH_OPS`. An opcode missing here
            // lets the peephole fuse the instruction a branch lands on, so the
            // branch arrives in the middle of a fused pair and runs the head it
            // was meant to skip. Several were absent — the null/nullish jumps,
            // the greater/not-equal test-jumps, `RescueJump`, `CatchMatch` and
            // `TryBegin` — which is the same drift that left `TryBegin`
            // unremapped in `compact_nops`.
            //
            // Completing this list costs a little speed, because it withdraws
            // fusion opportunities: measured at +4.8% on `String|replace_all`
            // and +0.1% on `String|chars` (medians of 8 interleaved runs of
            // bench/cross-language), with the median across all 67 cases
            // unmoved. That is the price of not mis-compiling a branch, so it
            // is the right trade — but if a future pass wants those cycles
            // back, the way to get them is a peephole that can fuse *around* a
            // target rather than a shorter list here.
            Op::Jump(offset)
            | Op::JumpIfFalse(offset)
            | Op::JumpIfFalseNoPop(offset)
            | Op::JumpIfTrueNoPop(offset)
            | Op::JumpIfNull(offset)
            | Op::JumpIfNotNull(offset)
            | Op::NullishJump(offset)
            | Op::JumpIfParamSupplied(_, offset)
            | Op::ForIter(offset)
            | Op::ForIterRange(offset)
            | Op::RescueJump(offset)
            | Op::CatchMatch(_, offset)
            | Op::MatchType(_, offset)
            | Op::TestLessEqualJump(offset)
            | Op::TestGreaterJump(offset)
            | Op::TestGreaterEqualJump(offset)
            | Op::TestNotEqualJump(offset)
            | Op::TestLessJump(offset) => {
                let target = i + 1 + *offset as usize;
                if target < len {
                    is_jump_target[target] = true;
                }
            }
            // Two targets, so it does not fit the single-operand arm above.
            Op::TryBegin(catch_offset, finally_offset) => {
                for offset in [catch_offset, finally_offset] {
                    let target = i + 1 + *offset as usize;
                    if target < len {
                        is_jump_target[target] = true;
                    }
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
        // Pattern: a = a + b  (GetLocal, GetLocal, Add, SetLocal, Pop) → in place.
        // Must precede the 3-op `AddLocalConst` rule below, which would otherwise
        // consume the prefix and leave the SetLocal/Pop behind.
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
                    code[i] = Op::AddLocalsInPlace(slot_a, slot_b);
                    code[i + 1] = NOP;
                    code[i + 2] = NOP;
                    code[i + 3] = NOP;
                    code[i + 4] = NOP;
                    i += 5;
                    continue;
                }
            }
        }

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

        // Pattern: GetLocal(s), GetProperty(c) → GetLocalProperty(s, c)
        //
        // `this.field` and `obj.field` are the most common shape in Soli's OO
        // code — every model attribute read, every controller `this.` — and the
        // pair costs ~27ns of which roughly 15ns is the second dispatch alone.
        // The fused opcode already existed and was already implemented; nothing
        // ever emitted it.
        if i + 1 < len {
            if let (Op::GetLocal(slot), Op::GetProperty(cidx)) = (code[i], code[i + 1]) {
                if !any_jump_target(&is_jump_target, i + 1, 2) {
                    code[i] = Op::GetLocalProperty(slot, cidx);
                    code[i + 1] = NOP;
                    i += 2;
                    continue;
                }
            }
        }

        // Pattern: GetLocal(a), GetLocal(b), GetIndex → GetLocalIndex(a, b)
        //
        // `arr[i]` with both operands in locals — the shape of every indexed
        // loop. Like GetLocalProperty above, the fused opcode was already
        // implemented and simply never emitted; this collapses three dispatches
        // into one.
        if i + 2 < len {
            if let (Op::GetLocal(obj_slot), Op::GetLocal(idx_slot), Op::GetIndex) =
                (code[i], code[i + 1], code[i + 2])
            {
                if !any_jump_target(&is_jump_target, i + 1, 3) {
                    code[i] = Op::GetLocalIndex(obj_slot, idx_slot);
                    code[i + 1] = NOP;
                    code[i + 2] = NOP;
                    i += 3;
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

/// Remove the `Op::Nop` placeholders the peephole leaves behind, rewriting every
/// jump offset to match.
///
/// The peephole fuses instruction sequences by overwriting the head and blanking
/// the tail with `NOP` rather than resizing the vector, which would invalidate
/// every jump offset in the chunk. Those NOPs were then *left in the emitted
/// code* and dispatched at runtime — and because fusing is most aggressive
/// exactly where code is hottest, they concentrated in tight loops. A counter
/// loop compiled to ten instructions per iteration of which five were NOPs:
/// half the dispatches did nothing.
///
/// (The previous stub declined to do this, reasoning that peephole NOPs could
/// not be told apart from real `Pop`s. That is no longer true — `NOP` is
/// `Op::Nop`, its own variant, and the compiler emits it nowhere else.)
///
/// Offsets are relative to the instruction *after* the jump, so a forward jump
/// at `i` with offset `d` targets `i + 1 + d`, and `Loop` targets `i + 1 - d`.
/// Both are remapped through the old→new index table.
///
/// **Invariant:** every opcode that advances `ip` by one of its operands must be
/// listed in the match below. Grepping for `Jump` is not sufficient — `ForIter`,
/// `ForIterRange`, `RescueJump` and `CatchMatch` all do it under other names.
/// `tests/differential_engines_test.rs` is what catches a miss.
fn compact_nops(chunk: &mut Chunk) {
    if !chunk.code.iter().any(|op| matches!(op, Op::Nop)) {
        return;
    }
    let len = chunk.code.len();

    // old index -> new index. Entry `len` is the one-past-the-end position, which
    // a jump to the very end of the chunk legitimately targets.
    let mut old_to_new = vec![0usize; len + 1];
    let mut next = 0usize;
    for (old, op) in chunk.code.iter().enumerate() {
        old_to_new[old] = next;
        if !matches!(op, Op::Nop) {
            next += 1;
        }
    }
    old_to_new[len] = next;

    let mut code = Vec::with_capacity(next);
    let mut lines = Vec::with_capacity(next);
    for (old, op) in chunk.code.iter().enumerate() {
        if matches!(op, Op::Nop) {
            continue;
        }
        let here = old_to_new[old];
        // `here + 1` is the ip the VM has already advanced to when the jump runs.
        let fixed = match *op {
            Op::Loop(d) => {
                let target = old_to_new[old + 1 - d as usize];
                Op::Loop((here + 1 - target) as u16)
            }
            Op::Jump(d) => Op::Jump(fwd(&old_to_new, old, d, here)),
            Op::JumpIfFalse(d) => Op::JumpIfFalse(fwd(&old_to_new, old, d, here)),
            Op::JumpIfFalseNoPop(d) => Op::JumpIfFalseNoPop(fwd(&old_to_new, old, d, here)),
            Op::JumpIfTrueNoPop(d) => Op::JumpIfTrueNoPop(fwd(&old_to_new, old, d, here)),
            Op::JumpIfNull(d) => Op::JumpIfNull(fwd(&old_to_new, old, d, here)),
            Op::JumpIfNotNull(d) => Op::JumpIfNotNull(fwd(&old_to_new, old, d, here)),
            // `??`. Missed for the same reason as `TryBegin` — the name does
            // not end in "Jump" in the places people grep.
            Op::NullishJump(d) => Op::NullishJump(fwd(&old_to_new, old, d, here)),
            Op::TestLessJump(d) => Op::TestLessJump(fwd(&old_to_new, old, d, here)),
            Op::TestLessEqualJump(d) => Op::TestLessEqualJump(fwd(&old_to_new, old, d, here)),
            Op::TestGreaterJump(d) => Op::TestGreaterJump(fwd(&old_to_new, old, d, here)),
            Op::TestGreaterEqualJump(d) => Op::TestGreaterEqualJump(fwd(&old_to_new, old, d, here)),
            Op::TestNotEqualJump(d) => Op::TestNotEqualJump(fwd(&old_to_new, old, d, here)),
            Op::JumpIfParamSupplied(p, d) => {
                Op::JumpIfParamSupplied(p, fwd(&old_to_new, old, d, here))
            }
            // These four do not have "jump" in their names and were missed on the
            // first pass; the differential harness caught it immediately. Any new
            // opcode that advances `ip` by an operand MUST be added here.
            Op::ForIter(d) => Op::ForIter(fwd(&old_to_new, old, d, here)),
            Op::ForIterRange(d) => Op::ForIterRange(fwd(&old_to_new, old, d, here)),
            Op::RescueJump(d) => Op::RescueJump(fwd(&old_to_new, old, d, here)),
            Op::CatchMatch(name_idx, d) => Op::CatchMatch(name_idx, fwd(&old_to_new, old, d, here)),
            Op::MatchType(name_idx, d) => Op::MatchType(name_idx, fwd(&old_to_new, old, d, here)),
            // `TryBegin` carries TWO targets and neither has "jump" in its
            // name, so it was missed the same way the four above were — but
            // silently, because no test put a fusable instruction inside a
            // `try` block. Fusing one shifts every later offset, and the
            // unremapped catch target then landed one instruction *into* the
            // catch body, skipping its first instruction: a `let` there kept
            // whatever was in its slot, so the block ran with garbage locals
            // and no error. Both operands are relative to the instruction
            // after `TryBegin`, exactly like a forward jump.
            Op::TryBegin(c, f) => Op::TryBegin(
                fwd(&old_to_new, old, c, here),
                fwd(&old_to_new, old, f, here),
            ),
            other => other,
        };
        code.push(fixed);
        lines.push(chunk.lines.get(old).copied().unwrap_or(0));
    }

    chunk.code = code;
    chunk.lines = lines;
}

/// Remap a forward jump offset through the old→new index table.
#[inline]
fn fwd(old_to_new: &[usize], old: usize, d: u16, here: usize) -> u16 {
    let target = old_to_new[old + 1 + d as usize];
    (target - (here + 1)) as u16
}

#[cfg(test)]
mod inplace_peephole_tests {
    use super::*;
    use crate::lexer::Scanner;
    use crate::parser::Parser;

    /// Every forward branch opcode must have its target remapped by
    /// compaction.
    ///
    /// `compact_nops` matches on opcode variants, and an unlisted one falls
    /// through `other => other` keeping a *stale* offset — silently, which is
    /// how `TryBegin` stayed broken. This walks `FORWARD_BRANCH_OPS` and puts
    /// a removable NOP between each branch and its target, so a variant that
    /// is missing from that match fails here instead of in somebody's `catch`
    /// block.
    #[test]
    fn every_branch_op_is_remapped_by_compaction() {
        use crate::vm::opcode::FORWARD_BRANCH_OPS;

        for proto in FORWARD_BRANCH_OPS {
            // branch at 0 -> target at 3; NOP at 1 disappears, so after
            // compaction the target sits at 2 and the offset must drop 2 -> 1.
            let branch = set_first_offset(*proto, 2);
            let mut chunk = Chunk::default();
            for op in [branch, Op::Nop, Op::Null, Op::Null] {
                chunk.code.push(op);
                chunk.lines.push(1);
            }
            compact_nops(&mut chunk);

            let got = first_offset(&chunk.code[0]);
            assert_eq!(
                got,
                Some(1),
                "{:?} kept a stale target through compaction — add it to the \
                 match in compact_nops (see FORWARD_BRANCH_OPS)",
                proto
            );
        }
    }

    /// Read/write the ip-relative operand, which is the last field on every
    /// branch opcode (`CatchMatch`/`JumpIfParamSupplied` carry an unrelated
    /// index first, `TryBegin` carries a second target after it).
    fn set_first_offset(op: Op, d: u16) -> Op {
        match op {
            Op::Jump(_) => Op::Jump(d),
            Op::JumpIfFalse(_) => Op::JumpIfFalse(d),
            Op::JumpIfFalseNoPop(_) => Op::JumpIfFalseNoPop(d),
            Op::JumpIfTrueNoPop(_) => Op::JumpIfTrueNoPop(d),
            Op::JumpIfNull(_) => Op::JumpIfNull(d),
            Op::JumpIfNotNull(_) => Op::JumpIfNotNull(d),
            Op::NullishJump(_) => Op::NullishJump(d),
            Op::JumpIfParamSupplied(p, _) => Op::JumpIfParamSupplied(p, d),
            Op::ForIter(_) => Op::ForIter(d),
            Op::ForIterRange(_) => Op::ForIterRange(d),
            Op::RescueJump(_) => Op::RescueJump(d),
            Op::CatchMatch(n, _) => Op::CatchMatch(n, d),
            Op::TestLessJump(_) => Op::TestLessJump(d),
            Op::TestLessEqualJump(_) => Op::TestLessEqualJump(d),
            Op::TestGreaterJump(_) => Op::TestGreaterJump(d),
            Op::TestGreaterEqualJump(_) => Op::TestGreaterEqualJump(d),
            Op::TestNotEqualJump(_) => Op::TestNotEqualJump(d),
            Op::TryBegin(_, f) => Op::TryBegin(d, f),
            Op::MatchType(n, _) => Op::MatchType(n, d),
            other => panic!("FORWARD_BRANCH_OPS holds a non-branch opcode: {other:?}"),
        }
    }

    fn first_offset(op: &Op) -> Option<u16> {
        Some(match *op {
            Op::Jump(d)
            | Op::JumpIfFalse(d)
            | Op::JumpIfFalseNoPop(d)
            | Op::JumpIfTrueNoPop(d)
            | Op::JumpIfNull(d)
            | Op::JumpIfNotNull(d)
            | Op::NullishJump(d)
            | Op::JumpIfParamSupplied(_, d)
            | Op::ForIter(d)
            | Op::ForIterRange(d)
            | Op::RescueJump(d)
            | Op::CatchMatch(_, d)
            | Op::TestLessJump(d)
            | Op::TestLessEqualJump(d)
            | Op::TestGreaterJump(d)
            | Op::TestGreaterEqualJump(d)
            | Op::TestNotEqualJump(d)
            | Op::MatchType(_, d)
            | Op::TryBegin(d, _) => d,
            _ => return None,
        })
    }

    /// `compact_nops` must remap **both** of `TryBegin`'s targets.
    ///
    /// Neither has "jump" in its name, so it was missed like `ForIter` and
    /// friends before it — but silently, because nothing put a fusable
    /// instruction inside a `try` block. This checks the remapping directly
    /// rather than through a program, so a future opcode with a hidden target
    /// has a pattern to copy.
    #[test]
    fn compact_nops_remaps_try_begin_targets() {
        // TryBegin at 0 targeting catch at 4 and finally at 6, with NOPs at 2
        // and 3 that compaction will remove. Offsets are relative to the
        // instruction after TryBegin (index 1), so catch = 4 - 1 = 3.
        let mut chunk = Chunk::default();
        for op in [
            Op::TryBegin(3, 5), // 0 -> catch 4, finally 6
            Op::Null,           // 1
            Op::Nop,            // 2 (removed)
            Op::Nop,            // 3 (removed)
            Op::Null,           // 4  catch target
            Op::Null,           // 5
            Op::Null,           // 6  finally target
        ] {
            chunk.code.push(op);
            chunk.lines.push(1);
        }
        compact_nops(&mut chunk);

        // Two NOPs gone: old 4 -> new 2, old 6 -> new 4.
        assert_eq!(chunk.code.len(), 5);
        match chunk.code[0] {
            Op::TryBegin(c, f) => {
                assert_eq!(c, 1, "catch target must remap to new index 2 (1 + 1)");
                assert_eq!(f, 3, "finally target must remap to new index 4 (1 + 3)");
            }
            ref other => panic!("expected TryBegin, got {other:?}"),
        }
    }

    /// Compile `body` inside a function, because that is where locals live:
    /// at top level `let x = 0` becomes a *global* (`DefineGlobal`), so a
    /// script-scope loop never exercises the local peepholes at all.
    fn fn_ops(body: &str) -> Vec<Op> {
        let src = format!("def f() {{\n{}\n}}\n", body);
        let tokens = Scanner::new(&src).scan_tokens().expect("lex");
        let program = Parser::new(tokens).parse().expect("parse");
        let m = Compiler::compile(&program).expect("compile");
        for c in m.main.chunk.constants.iter() {
            if let Constant::Function(proto) = c {
                return proto.chunk.code.clone();
            }
        }
        panic!("no function proto found");
    }

    /// The whole point of the in-place opcodes is that the canonical loop body
    /// actually compiles to them. Without this, the peephole can silently stop
    /// matching and the only symptom is a benchmark that quietly gets slower.
    /// `i = i + 1` is already collapsed to `IncrLocal` by an earlier rule, and
    /// `d = d - 1` to `DecrLocal`. Pinning that here so nobody re-adds a
    /// redundant const/subtract in-place opcode: the seam is taken.
    #[test]
    fn counter_increment_uses_existing_incr_local() {
        let ops = fn_ops("let i = 0\nwhile i < 5 { i = i + 1 }");
        assert!(
            ops.iter().any(|o| matches!(o, Op::IncrLocal(_))),
            "expected the pre-existing IncrLocal, got: {:?}",
            ops
        );
    }

    #[test]
    fn accumulator_compiles_in_place() {
        let ops = fn_ops("let s = 0\nlet j = 0\nwhile j < 3 { s = s + j\n j = j + 1 }");
        assert!(
            ops.iter().any(|o| matches!(o, Op::AddLocalsInPlace(_, _))),
            "expected AddLocalsInPlace, got: {:?}",
            ops
        );
    }
}
