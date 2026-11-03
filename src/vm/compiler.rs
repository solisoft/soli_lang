//! AST-to-bytecode compiler.
//!
//! Single-pass compilation: walks the AST once, emitting bytecode into a `Chunk`.
//! Variable resolution happens at compile time — locals become stack slot indices.

use std::cell::{Cell, RefCell};
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
/// **On by default.** Without it a bare assignment to an undeclared name
/// compiled to `SetGlobal`, which raised "undefined variable" at runtime. The
/// server then re-ran the handler on the tree-walker, so nearly every real
/// action ran there — and one that had already written to the database could
/// not be re-run at all, so it answered 500. `SOLI_VM_OPTIONAL_LET=0` restores
/// the old behavior, as an escape hatch.
pub fn optional_let_enabled() -> bool {
    use std::sync::OnceLock;
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        crate::platform::env::var("SOLI_VM_OPTIONAL_LET")
            .map(|v| !(v == "0" || v.eq_ignore_ascii_case("false")))
            .unwrap_or(true)
    })
}

/// A local variable tracked during compilation.
#[derive(Debug, Clone)]
pub struct Local {
    pub name: String,
    pub depth: i32,
    pub is_captured: bool,
    pub is_const: bool,
    /// A top-level variable of the entry script kept in a frame slot instead
    /// of a global ([`Compiler::compile_entry_script`]). Nothing but the
    /// script's top-level code may reach it.
    pub promoted: bool,
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

/// A set of global names shared, read-only, between compilers — see
/// [`Compiler::base_globals`].
pub type GlobalNames = Rc<HashSet<String>>;

/// The compiler: transforms AST into bytecode.
pub struct Compiler {
    /// Emit `Op::CoverLine` markers: a coverage tracker was installed when
    /// compilation began (`soli test --coverage`). Off for `soli serve`.
    pub coverage: bool,
    /// The last line a `CoverLine` marker was emitted for, so a line spread
    /// over several expressions is marked once.
    pub last_cover_line: usize,
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
    /// Set while compiling a class nested in another (`module Billing` …
    /// `class Invoice`): it becomes a member of its parent, not a global or
    /// a local.
    pub compiling_nested_class: bool,
    /// Qualified names of the classes being compiled, outermost first: a
    /// nested class is named after them (`Billing::Invoice`).
    pub class_namespace: Vec<String>,
    /// Names known to be globals at compile time. Shared across every nested
    /// compiler of one module. Used to decide, for a bare assignment inside a
    /// function to a name that is neither a local nor an upvalue, whether to
    /// assign the existing global (name is in this set) or declare a fresh
    /// function-local (name is not). In `serve` this is seeded with the worker
    /// VM's full global table, so the decision matches the tree-walker exactly;
    /// for whole-program compiles it accumulates top-level names as they appear.
    pub known_globals: Rc<RefCell<HashSet<String>>>,
    /// A read-only set of names that count as known globals too, shared by
    /// every compiler that is handed it: a worker's whole global table, built
    /// once per worker rather than copied into `known_globals` for each of
    /// the hundreds of handlers and methods it pre-compiles. Consult both
    /// through [`Compiler::is_known_global`].
    pub base_globals: Option<GlobalNames>,
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
    /// This set only ever grows from a `let`/`const` or a bare assignment at
    /// global scope, or a function declaration, so it means what it says in
    /// both modes.
    pub program_globals: Rc<RefCell<HashSet<String>>>,
    /// When set, every name resolved as a global is recorded here, nested
    /// functions included. A compiled template reads its free names this way
    /// and turns them into parameters (`template::vm_template`).
    pub referenced_globals: Option<Rc<RefCell<HashSet<String>>>>,
    /// Compiling a view (`template::vm_template`): a view has no instance, so
    /// `@title` reads, calls or assigns the local `title` and a bare `this` is
    /// nil — what the tree-walker's lenient template mode does.
    pub template_mode: bool,
    /// Native kernels of the program (see `crate::native`). Only the
    /// top-level compiler holds them: a top-level `def` that has one gets a
    /// proto carrying it.
    pub kernels: Option<crate::native::KernelSet>,
    /// Source column of the expression or statement being compiled, recorded
    /// with each instruction for error spans.
    pub column: u32,
    /// Top-level `name = value` statements of the function being compiled
    /// that declare `name` where they stand instead of assigning a slot
    /// hoisted to function entry (`hoist_locals`). Each is taken when its
    /// statement is compiled (`take_in_place_decl`).
    pub in_place_decls: Vec<super::compiler_hoist::InPlaceDecl>,
    /// A lambda the compiler made up to run an expression at a clean frame
    /// (`wrap_in_lambda`), called once, right where it is created.
    pub expr_wrapper: bool,
    /// Set while compiling an entry script whose top-level variables live in
    /// frame slots, shared by every function compiled inside it: raised when
    /// one of those functions captures such a slot, which refuses the plan
    /// ([`Compiler::compile_entry_script`]).
    pub promotion_guard: Option<Rc<Cell<bool>>>,
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
            coverage: crate::coverage::global_coverage_tracker_installed(),
            last_cover_line: 0,
            proto: FunctionProto::new(name),
            locals: Vec::new(),
            scope_depth: 0,
            upvalues: Vec::new(),
            enclosing: None,
            function_type,
            loop_context: None,
            class_context: None,
            compiling_nested_class: false,
            class_namespace: Vec::new(),
            known_globals: Rc::new(RefCell::new(HashSet::new())),
            base_globals: None,
            stack_height: 0,
            try_stack: Vec::new(),
            program_globals: Rc::new(RefCell::new(HashSet::new())),
            referenced_globals: None,
            template_mode: false,
            kernels: None,
            column: 0,
            in_place_decls: Vec::new(),
            expr_wrapper: false,
            promotion_guard: None,
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
            promoted: false,
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
        Self::compile_with_globals_from(program, globals, None)
    }

    /// `compile_with_globals` for code from a known file: the functions it
    /// defines inherit `source_path`, which line coverage (`Op::CoverLine`)
    /// needs. Used when the VM compiles a tree-walker function — a free
    /// function or a static method, which a service is made of.
    pub fn compile_with_globals_from<I: IntoIterator<Item = String>>(
        program: &Program,
        globals: I,
        source_path: Option<Arc<std::path::PathBuf>>,
    ) -> CompileResult<CompiledModule> {
        Self::compile_full(program, globals, source_path, None)
    }

    /// `compile_with_globals_from`, recording into `seen` every name the
    /// program resolves as a global, nested functions included.
    ///
    /// A name passed in `globals` is a known global, so an assignment to it is
    /// recorded too. A read of a name that is not a local is recorded either
    /// way. The view VM uses that to keep a helper on the tree-walker when the
    /// helper's closure holds something the bytecode would not see.
    pub fn compile_with_globals_recording<I: IntoIterator<Item = String>>(
        program: &Program,
        globals: I,
        source_path: Option<Arc<std::path::PathBuf>>,
        seen: Rc<RefCell<HashSet<String>>>,
    ) -> CompileResult<CompiledModule> {
        let mut compiler = Compiler::new(FunctionType::Script, String::new());
        compiler.known_globals.borrow_mut().extend(globals);
        compiler.referenced_globals = Some(seen);
        Self::compile_script(compiler, program, source_path, None)
    }

    /// Compile a whole script whose top-level functions may carry native
    /// kernels, built by `crate::native::analyze_and_compile` from this same
    /// `program` (they are matched by declaration address).
    pub fn compile_with_kernels(
        program: &Program,
        kernels: Option<crate::native::KernelSet>,
    ) -> CompileResult<CompiledModule> {
        Self::compile_full(program, std::iter::empty(), None, kernels)
    }

    /// Compile the program `soli script.sl` runs (and `soli -e`, and a script
    /// executable): [`Self::compile_with_kernels`], with the script's own
    /// top-level variables kept in frame slots wherever nothing but its
    /// top-level code can reach them, so a loop at file scope runs on locals
    /// as it would in a `def` (`compiler_script_locals::plan`). `is_builtin`
    /// names the globals the VM starts with; a variable named like one stays
    /// a global.
    ///
    /// This only ever changes how fast the script runs. If the compile that
    /// keeps them in slots fails, or finds a function reaching one, the
    /// program is compiled again with every top-level variable a global.
    pub fn compile_entry_script(
        program: &Program,
        kernels: Option<crate::native::KernelSet>,
        is_builtin: &dyn Fn(&str) -> bool,
    ) -> CompileResult<CompiledModule> {
        if optional_let_enabled() {
            let plan = super::compiler_script_locals::plan(&program.statements, is_builtin);
            if !plan.is_empty() {
                if let Ok(module) = Self::compile_promoted(program, kernels.clone(), &plan) {
                    return Ok(module);
                }
            }
        }
        Self::compile_with_kernels(program, kernels)
    }

    /// [`Self::compile_with_kernels`], each `(index, name)` of `plan` declaring
    /// `name` in a frame slot at top-level statement `index`. An error when a
    /// function captures one of those slots, or a planned statement is not
    /// reached as planned.
    pub(crate) fn compile_promoted(
        program: &Program,
        kernels: Option<crate::native::KernelSet>,
        plan: &[(usize, String)],
    ) -> CompileResult<CompiledModule> {
        let mut compiler = Compiler::new(FunctionType::Script, String::new());
        compiler.promotion_guard = Some(Rc::new(Cell::new(false)));
        compiler.in_place_decls = plan
            .iter()
            .map(|(index, name)| super::compiler_hoist::InPlaceDecl {
                stmt: &program.statements[*index],
                name: name.clone(),
                resolved_early: false,
            })
            .collect();
        Self::compile_script(compiler, program, None, kernels)
    }

    /// `compile_with_globals_from`, with the known globals given as a shared
    /// set instead of an iterator copied into this compiler.
    pub fn compile_with_base_globals_from(
        program: &Program,
        base: &GlobalNames,
        source_path: Option<Arc<std::path::PathBuf>>,
    ) -> CompileResult<CompiledModule> {
        let mut compiler = Compiler::new(FunctionType::Script, String::new());
        compiler.base_globals = Some(base.clone());
        Self::compile_script(compiler, program, source_path, None)
    }

    /// Compile a view's program in `template_mode`, recording into `seen`
    /// (when given) every name it resolves as a global, nested functions
    /// included (see `referenced_globals`).
    pub fn compile_template(
        program: &Program,
        seen: Option<Rc<RefCell<HashSet<String>>>>,
    ) -> CompileResult<CompiledModule> {
        let mut compiler = Compiler::new(FunctionType::Script, String::new());
        compiler.referenced_globals = seen;
        compiler.template_mode = true;
        Self::compile_script(compiler, program, None, None)
    }

    fn compile_full<I: IntoIterator<Item = String>>(
        program: &Program,
        globals: I,
        source_path: Option<Arc<std::path::PathBuf>>,
        kernels: Option<crate::native::KernelSet>,
    ) -> CompileResult<CompiledModule> {
        let compiler = Compiler::new(FunctionType::Script, String::new());
        compiler.known_globals.borrow_mut().extend(globals);
        Self::compile_script(compiler, program, source_path, kernels)
    }

    fn compile_script(
        mut compiler: Compiler,
        program: &Program,
        source_path: Option<Arc<std::path::PathBuf>>,
        kernels: Option<crate::native::KernelSet>,
    ) -> CompileResult<CompiledModule> {
        compiler.kernels = kernels;
        compiler.proto.source_path = source_path;
        for stmt in &program.statements {
            compiler.compile_stmt(stmt)?;
        }
        // Top-level variables kept in frame slots (`compile_entry_script`):
        // a function captured one, or a planned declaration was never
        // reached. The plan was wrong, so this bytecode must not run.
        let breached = compiler
            .promotion_guard
            .as_ref()
            .is_some_and(|breach| breach.get());
        if breached || !compiler.in_place_decls.is_empty() {
            return Err(CompileError::new(
                "a top-level variable kept in a frame slot is reached from outside the top level",
                Span::default(),
            ));
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
        let compiler = Compiler::new(FunctionType::Method, func.name.clone());
        compiler.known_globals.borrow_mut().extend(globals);
        Self::compile_method_with(compiler, func)
    }

    /// `compile_method_standalone`, with the known globals given as a shared
    /// set instead of an iterator copied into this compiler.
    pub fn compile_method_standalone_with_base(
        func: &crate::interpreter::value::Function,
        base: &GlobalNames,
    ) -> CompileResult<FunctionProto> {
        let mut compiler = Compiler::new(FunctionType::Method, func.name.clone());
        compiler.base_globals = Some(base.clone());
        Self::compile_method_with(compiler, func)
    }

    /// Compile a tree-walking constructor (a class the app loader defined) as
    /// the `init` of a class the VM compiles is compiled: slot 0 is `this`,
    /// the body runs as statements, and the frame returns `this` — when it
    /// ends, `return`s early, or leaves through a loopless `break`/`next`.
    /// The VM enters it as an ordinary frame, so a construction is one frame
    /// push, not a nested run that throws the constructor's value away.
    pub fn compile_constructor_standalone<I: IntoIterator<Item = String>>(
        func: &crate::interpreter::value::Function,
        globals: I,
    ) -> CompileResult<FunctionProto> {
        let compiler = Compiler::new(FunctionType::Constructor, func.name.clone());
        compiler.known_globals.borrow_mut().extend(globals);
        Self::compile_method_with(compiler, func)
    }

    fn compile_method_with(
        mut compiler: Compiler,
        func: &crate::interpreter::value::Function,
    ) -> CompileResult<FunctionProto> {
        compiler.proto.namespace = namespace_of(func);
        compiler.proto.source_path = func
            .source_path
            .as_ref()
            .map(|p| std::sync::Arc::new(std::path::PathBuf::from(p)));
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
        compiler.proto.block_param = block_param_index(&func.params);
        compiler.proto.defaults_mask = defaults_mask(&func.params);

        let line = func.span.map(|s| s.line as usize).unwrap_or(0);
        let constructor = compiler.function_type == FunctionType::Constructor;
        compiler.begin_scope();
        compiler.emit_param_defaults(&func.params)?;
        if constructor {
            // Statements only, as `compile_constructor` compiles them: a
            // constructor's last expression is not its value.
            compiler.hoist_locals(&func.body, line);
            for stmt in func.body.iter() {
                compiler.compile_stmt(stmt)?;
            }
        } else {
            compiler.compile_function_body(&func.body)?;
        }
        compiler.end_scope(line);

        // `nil`, or a constructor's instance.
        compiler.emit_default_return_value(line);
        compiler.emit(Op::Return, line);

        let mut proto = compiler.proto;
        proto.upvalue_descriptors = compiler.upvalues;
        proto.is_method = true;

        peephole_optimize_proto(&mut proto);

        Ok(proto)
    }

    /// Mark `line` as executed for line coverage (`Op::CoverLine`), unless it
    /// was the line just marked. Only under a coverage tracker, and only for a
    /// function that knows its source file. Called at each statement (after a
    /// reset, so every statement is marked) and at each expression, so the
    /// continuation lines of a multi-line hash or call are counted as the
    /// tree-walker counts them.
    pub(crate) fn cover_line(&mut self, line: usize) {
        if self.coverage
            && line > 0
            && line != self.last_cover_line
            && self.proto.source_path.is_some()
        {
            self.emit(Op::CoverLine(line as u32), line);
            self.last_cover_line = line;
        }
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
        self.proto.chunk.emit_at(op, line, self.column)
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

    /// `GetProperty` of `name`, with the method id and zero-arg mask that let
    /// the fast tier answer a paren-less `h.length`, `s.blank?` or `n.to_s`
    /// by id (see `Op::GetProperty`).
    pub fn get_property_op(&mut self, name: &str) -> Op {
        let idx = self.add_string_constant(name);
        let mid = super::method_table::resolve_method_id(name);
        let mask = if mid == super::method_table::METHOD_UNKNOWN {
            0
        } else {
            crate::interpreter::executor::calls::method_registry::zero_arg_receiver_mask(name)
        };
        Op::GetProperty(idx, mid, mask)
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
            promoted: false,
        });
    }

    /// Declare `name` as a local whose value is already on top of the stack:
    /// a statement planned to declare it in place (`take_in_place_decl`). At
    /// the top level of a script that is a top-level variable kept in a frame
    /// slot ([`Compiler::compile_entry_script`]), marked as such.
    pub(crate) fn declare_in_place(&mut self, name: &str, is_const: bool) {
        self.add_local(name.to_string(), is_const);
        if self.function_type == FunctionType::Script {
            if let Some(local) = self.locals.last_mut() {
                local.promoted = true;
            }
        }
    }

    /// The slot of `name` when it is a top-level variable this script keeps
    /// in a frame slot ([`Compiler::compile_entry_script`]).
    pub(crate) fn promoted_slot(&self, name: &str) -> Option<u16> {
        self.resolve_local(name)
            .filter(|&slot| self.locals[slot as usize].promoted)
    }

    /// Whether `name`, just resolved here as an upvalue, is a top-level
    /// variable kept in a frame slot, reached through a function written in
    /// the source: the plan says none of those names it. A lambda the
    /// compiler made up around an expression (`expr_wrapper`) does not count,
    /// since it is called on the spot, while the slot is live.
    fn captures_promoted_local(&self, name: &str) -> bool {
        let mut through_function = !self.expr_wrapper;
        let mut current = self.enclosing.as_deref();
        while let Some(compiler) = current {
            if let Some(slot) = compiler.resolve_local(name) {
                return through_function && compiler.locals[slot as usize].promoted;
            }
            through_function |= !compiler.expr_wrapper;
            current = compiler.enclosing.as_deref();
        }
        false
    }

    /// Whether `name` is a global this compiler knows of: declared so far, or
    /// in the shared base set it was handed.
    pub(crate) fn is_known_global(&self, name: &str) -> bool {
        self.known_globals.borrow().contains(name)
            || self
                .base_globals
                .as_ref()
                .is_some_and(|base| base.contains(name))
    }

    /// Declare the locals introduced by bare assignment (Soli's optional-`let`)
    /// in `body`, a function body about to be compiled. Must be called inside
    /// the body scope after parameters have been added. A candidate is skipped
    /// when it is already a parameter/local, is captured from an enclosing
    /// scope (so it stays an upvalue), or is a known global (so the assignment
    /// targets the existing global) — matching the tree-walking interpreter.
    ///
    /// A candidate whose first mention is a top-level `name = value` is
    /// declared by that statement, the way `let name = value` declares it
    /// (`in_place_declarations` says when that cannot be told apart): its
    /// statement is recorded in `in_place_decls` for `compile_stmt` to find.
    /// The rest are declared here, as nils pushed at function entry.
    pub fn hoist_locals(&mut self, body: &[Stmt], line: usize) {
        if !optional_let_enabled() {
            return;
        }
        let candidates: Vec<String> = super::compiler_hoist::collect_hoisted_locals(body)
            .into_iter()
            .filter(|name| {
                self.resolve_local(name).is_none()
                    && !self.is_known_global(name)
                    && !self.enclosing_has_local(name)
            })
            .collect();
        let in_place =
            super::compiler_hoist::in_place_declarations(body, &candidates, self.template_mode);
        for &(index, name) in &in_place {
            self.in_place_decls
                .push(super::compiler_hoist::InPlaceDecl {
                    stmt: &body[index],
                    name: name.to_string(),
                    resolved_early: false,
                });
        }
        let hoisted: Vec<String> = candidates
            .into_iter()
            .filter(|name| {
                !in_place
                    .iter()
                    .any(|&(_, declared)| declared == name.as_str())
            })
            .collect();
        self.emit_nulls(hoisted.len(), line);
        for name in hoisted {
            self.add_local(name, false);
        }
    }

    /// Push `count` nils: a plain `Null` for one, one `PushNulls` for more.
    fn emit_nulls(&mut self, count: usize, line: usize) {
        let mut left = count;
        while left > 0 {
            let chunk = left.min(u16::MAX as usize);
            let op = if chunk == 1 {
                Op::Null
            } else {
                Op::PushNulls(chunk as u16)
            };
            self.emit(op, line);
            left -= chunk;
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
            // A function capturing a top-level variable the script keeps in
            // a frame slot: the plan missed a mention, so it is refused.
            if let Some(breach) = &self.promotion_guard {
                if self.captures_promoted_local(name) {
                    breach.set(true);
                }
            }
            VariableAccess::Upvalue(idx)
        } else {
            self.note_global_resolution(name);
            if let Some(seen) = &self.referenced_globals {
                seen.borrow_mut().insert(name.to_string());
            }
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
        // A closure lives in the file of the function that encloses it.
        new_compiler.proto.source_path = self.proto.source_path.clone();
        // Nested functions share the module's known-globals set so they make
        // the same local-vs-global decision for bare assignments.
        new_compiler.known_globals = self.known_globals.clone();
        new_compiler.base_globals = self.base_globals.clone();
        new_compiler.program_globals = self.program_globals.clone();
        new_compiler.referenced_globals = self.referenced_globals.clone();
        new_compiler.promotion_guard = self.promotion_guard.clone();
        new_compiler.template_mode = self.template_mode;
        // A closure reads names in the namespace of the code around it.
        new_compiler.proto.namespace = self.proto.namespace.clone();

        // Add parameters as locals
        for param in params {
            new_compiler.add_local(param.name.clone(), false);
        }
        new_compiler.proto.arity =
            params.iter().filter(|p| p.default_value.is_none()).count() as u8;
        new_compiler.proto.defaults =
            params.iter().filter(|p| p.default_value.is_some()).count() as u8;
        new_compiler.proto.param_names = params.iter().map(|p| p.name.clone()).collect();
        new_compiler.proto.block_param = block_param_index(params);
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

    /// Compile `body` inside a zero-argument lambda and immediately call it.
    ///
    /// Used when a construct needs a locals-baseline stack (comprehensions,
    /// binding `match`) but the current expression sits above anonymous
    /// temporaries. The inner `Compiler` starts at a clean frame, so
    /// `stack_height == locals.len()` holds; upvalues capture any outer
    /// locals the body reads. Net stack effect is +1 (the body's value).
    pub fn wrap_in_lambda<F>(&mut self, line: usize, body: F) -> CompileResult<()>
    where
        F: FnOnce(&mut Compiler) -> CompileResult<()>,
    {
        let _dummy = self.start_function(FunctionType::Lambda, "<expr>".to_string(), &[]);
        self.expr_wrapper = true;
        self.resync_stack_height();
        body(self)?;
        self.emit(Op::Return, line);
        let proto = self.finish_function(line);
        let idx = self.add_constant(Constant::Function(Arc::new(proto)));
        self.emit(Op::Closure(idx), line);
        self.emit(Op::Call(0), line);
        Ok(())
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
    /// tree-walker's top-level loop ignores both, so this emits nothing. A
    /// constructor ends with its instance, as on any other way out.
    fn emit_loopless_exit(&mut self, line: usize) -> bool {
        if self.function_type == FunctionType::Script {
            return true;
        }
        self.emit_default_return_value(line);
        self.emit(Op::Return, line);
        true
    }

    /// The value of a `return` that gives none: `nil`, but in a constructor
    /// its instance (`this`, slot 0). The tree-walker discards whatever a
    /// constructor returns, and `new` always evaluates to the instance.
    pub(crate) fn emit_default_return_value(&mut self, line: usize) {
        if self.function_type == FunctionType::Constructor {
            self.emit(Op::GetLocal(0), line);
        } else {
            self.emit(Op::Null, line);
        }
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

/// The index of the `&block` parameter, when `params` declares one.
fn block_param_index(params: &[Parameter]) -> Option<u16> {
    params
        .iter()
        .position(|p| p.is_block_param)
        .and_then(|i| u16::try_from(i).ok())
}

/// Net value-stack effect (pushes − pops) of an opcode, used by the compiler's
/// `stack_height` tracking for the comprehension clean-position gate.
///
/// Only opcodes the compiler emits during compilation reach the gate; the
/// peephole super-instructions run after compilation (mutating the chunk
/// directly, not via `emit`). Every value is held to its handler all the same:
/// `stack_effect_matches_what_each_handler_does` runs each opcode it can set
/// up on a real VM and compares the stack heights, so add a case there with a
/// new opcode. `ForIter`/`ForIterRange` use the continue-path effect (+1);
/// loops resync the height explicitly so the exit-path mismatch never matters.
/// The gate tolerates over-counting (an extra, safe fallback) but must never
/// under-count, so when in doubt an op's effect should not be more negative
/// than reality.
fn stack_effect(op: Op) -> i32 {
    use Op::*;
    match op {
        // Constants / literals push one value.
        Constant(_) | Null | True | False | Symbol(_) => 1,
        PushNulls(n) => n as i32,
        Dup => 1,
        Pop => -1,
        // Reads push; stores leave the value in place (net 0).
        GetLocal(_) | GetGlobal(_) | GetUpvalue(_) | GetThis => 1,
        // Pops `this` and pushes nil: a stub nothing emits.
        GetSuper(_) => 0,
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
        | JumpIfParamSupplied(_, _)
        | JumpIfLocalFalsy(_, _)
        | JumpIfLocalTruthy(_, _) => 0,
        JumpIfFalse(_) | JumpIfTrue(_) => -1,
        // Calls: pop callee/receiver + argc args, push the result.
        Call(argc) | CallMethod(_, argc) | CallMethodById(_, argc, _) => -(argc as i32),
        // Same shape as Call/New: the callee plus argc slots collapse to one result.
        CallNamed(argc, _) | NewNamed(argc, _) => -(argc as i32),
        // [this, args…] collapse to the result: net -argc.
        CallSuperInit(argc) | CallSuperMethod(_, argc) => -(argc as i32),
        CallGlobal(_, argc) | GetGlobalCall(_, argc) => 1 - argc as i32,
        SelfOrGlobal(_) | MarkPrivate(_) | MarkProtected(_) => 0,
        // `this` becomes the callee, and the resolution marker is pushed on
        // top; the call collapses `[callee, marker, args…]` to the result.
        SelfOrGlobalCallee(_) => 1,
        CallSelfOrGlobal(_, argc) | CallSelfOrGlobalNamed(_, argc, _) => -(argc as i32) - 1,
        // [receiver, args…] collapse to the result, as for CallMethod.
        CallMethodNamed(_, argc, _) => -(argc as i32),
        Closure(_) => 1,
        Return => 0,
        // Collections.
        Array(n) | BuildString(n) | HashWithKeys(_, n) => 1 - n as i32,
        Hash(n) => 1 - 2 * n as i32,
        // The array pushed; the argument replaced by the answer.
        LiteralArray(_) => 1,
        LiteralListIncludes(_, _) => 0,
        ArrayPush | Spread => -1,
        Range | GetIndex => -1,
        SetIndex => -2,
        // Objects.
        GetProperty(_, _, _) | GetPropertyOrNull(_) => 0,
        SetProperty(_) => -1,
        // Classes (class value stays on the stack; method/field defs pop one).
        Class(_) | Module(_) => 1,
        Inherit | Include | Extend | Method(_) | StaticMethod(_) | Field(_) | StaticField(_)
        | ConstField(_) | StaticConstField(_) => -1,
        New(argc) => -(argc as i32),
        // Exceptions.
        TryBegin(_, _) | TryEnd | ForceFuture | CatchMatch(_, _) | PopHandler | RescueJump(_) => 0,
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
        TemplateWrite(_) => -1,
        TemplateWriteConst(_) => 0,
        TemplateIter => 0,
        TemplateCaptureStart | TemplateCaptureEnd => 1,
        TemplateComponent => -2,
        TemplatePartial => -1,
        TemplateContentFor => -1,
        TemplateYield | TemplateYieldNamed => 0,
        // Peephole super-instructions, and the `Hash*Const` family, which the
        // compiler emits itself (`h["k"]`, `h["k"] = v`, `h.get("k")`,
        // `h.set("k", v)`) so the gate reads those values.
        HashGetConst(_) | HashHasKeyConst(_) | HashDeleteConst(_) | HashGetConst2(_, _) => 0,
        // Pops the receiver and the value, pushes the value back.
        HashSetConst(_) => -1,
        HashGetLocalConst(_, _)
        | HashHasKeyLocalConst(_, _)
        | HashDeleteLocalConst(_, _)
        | HashGetLocalConst2(_, _, _) => 1,
        AddLocalsInPlace(_, _)
        | AppendLocalConst(_, _)
        | AppendGlobalConst(_, _)
        | AddLocalConstInPlace(_, _)
        | SubLocalConstInPlace(_, _)
        | MulLocalConstInPlace(_, _) => 0,
        // Pops the variable's value and the RHS, pushes the updated variable.
        AppendLocal(_) | AppendGlobal(_) | AppendUpvalue(_) => -1,
        AddAssignLocal(_) | AddAssignUpvalue(_) | AddAssignGlobal(_) => -1,
        // Pops the value and pushes it back: it is the assignment's value. At
        // -1, `h["k"] = v` in the middle of an expression left the gate
        // seeing a clean stack, and a comprehension or binding `match` later
        // in the expression took an occupied slot.
        HashSetLocalConst(_, _) => 0,
        HashGetGlobalConst(_, _)
        | HashHasKeyGlobalConst(_, _)
        | HashDeleteGlobalConst(_, _)
        | HashGetGlobalConst2(_, _, _) => 1,
        HashSetGlobalConst(_, _) => 0,
        IncrLocal(_) | DecrLocal(_) | IncrLocalFast(_) | PopNull | Nop | CoverLine(_) => 0,
        // Peek the value and push a bool.
        IsNull | NotNull => 1,
        AddLocalLocal(_, _)
        | SubLocalLocal(_, _)
        | MulLocalLocal(_, _)
        | DivLocalLocal(_, _)
        | ModLocalLocal(_, _)
        | LessEqualLocalLocal(_, _)
        | LessLocalLocal(_, _)
        | GreaterLocalLocal(_, _)
        | LessLocalConst(_, _)
        | LessEqualLocalConst(_, _)
        | GreaterLocalConst(_, _)
        | GreaterEqualLocalConst(_, _)
        | EqualLocalLocal(_, _)
        | NotEqualLocalLocal(_, _)
        | AddLocalConst(_, _)
        | SubLocalConst(_, _)
        | MulLocalConst(_, _)
        | DivLocalConst(_, _)
        | ModLocalConst(_, _)
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
        | TestNotEqualJump(_)
        | TestEqualJump(_) => -2,
        GetLocalProperty(_, _, _, _) | GetLocalIndex(_, _) | GetNestedIndex(_, _, _) => 1,
        AddNestedIndex(_, _, _, _) | SetNestedIndex(_, _, _, _) => 0,
        // Touches `iter_stack`, not the value stack.
        PopIter => 0,
        // Peeks the subject and branches; leaves the stack alone.
        MatchType(_, _) => 0,
        // Reads a payload field out of a slot and pushes it.
        EnumPayload(_, _, _) => 1,
        // Read a slot, push a Bool / the rest Array.
        MatchArrayLen(_, _, _) | ArrayRestLocal(_, _) => 1,
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

    // Track which offsets are jump targets: a pattern may start at one, but
    // never have one in its tail (see the scan below).
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
            // is the right trade. (Part of it came back once the scan stopped
            // refusing a pattern whose head is a target: only a target in the
            // tail blocks a fusion now.)
            Op::Jump(offset)
            | Op::JumpIfFalse(offset)
            | Op::JumpIfTrue(offset)
            | Op::JumpIfFalseNoPop(offset)
            | Op::JumpIfTrueNoPop(offset)
            | Op::JumpIfNull(offset)
            | Op::JumpIfNotNull(offset)
            | Op::NullishJump(offset)
            | Op::JumpIfParamSupplied(_, offset)
            | Op::JumpIfLocalFalsy(_, offset)
            | Op::JumpIfLocalTruthy(_, offset)
            | Op::ForIter(offset)
            | Op::ForIterRange(offset)
            | Op::RescueJump(offset)
            | Op::CatchMatch(_, offset)
            | Op::MatchType(_, offset)
            | Op::TestLessEqualJump(offset)
            | Op::TestGreaterJump(offset)
            | Op::TestGreaterEqualJump(offset)
            | Op::TestNotEqualJump(offset)
            | Op::TestEqualJump(offset)
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

    // Pattern matching: scan for optimizable sequences.
    //
    // A pattern may start at a jump target. Every rule writes its result at
    // the head's own index `i` and blanks only the tail, and `compact_nops`
    // maps a branch to `i` onto that fused op, which does on its own what the
    // whole sequence did. What would be unsafe is a branch landing *inside*
    // the pattern, past the fused op: each rule refuses that with
    // `any_jump_target(&is_jump_target, i + 1, n)`. No rule reads an
    // instruction before `i`, and the fused jumps keep their absolute targets,
    // so `is_jump_target` stays valid through the scan.
    //
    // The scan used to skip a head that a branch landed on, before trying a
    // single rule. That left every `while` condition (the `Loop` target), the
    // statement after each `if`/`else`/loop/guard/`catch`, every `elsif` and
    // `else` head, match arms 2..n and the first statement of a function with
    // a defaulted parameter unfused: `i += 1` right after an `if` ran as five
    // ops instead of `IncrLocal`, about 25% slower than one line earlier.
    let mut i = 0;
    while i + 4 < len {
        // `total = total + h[keys[k]]` — acc += obj[arr[idx]]
        // GetLocal(acc), GetLocal(obj), GetLocal(arr), GetLocal(idx),
        // GetIndex, GetIndex, Add, SetLocal(acc), Pop
        if i + 7 < len {
            if let (
                Op::GetLocal(acc),
                Op::GetLocal(obj),
                Op::GetLocal(arr),
                Op::GetLocal(idx),
                Op::GetIndex,
                Op::GetIndex,
                Op::Add,
                Op::SetLocal(acc2),
            ) = (
                code[i],
                code[i + 1],
                code[i + 2],
                code[i + 3],
                code[i + 4],
                code[i + 5],
                code[i + 6],
                code[i + 7],
            ) {
                // The trailing `Pop` is part of the pattern, not an optional
                // extra. The fused opcode writes straight into the local slot
                // and pushes nothing, while the sequence it replaces nets +1
                // (`Op::SetLocal` peeks instead of popping; `Op::SetIndex`
                // re-pushes the object). Fusing without the `Pop` therefore
                // loses a stack slot, so a function whose last statement is one
                // of these — which `compile_function_body` emits with no `Pop`
                // before `Op::Return` — returns a stray local instead of its
                // value. The `IncrLocal` peephole below matches the same way.
                if acc == acc2
                    && i + 8 < len
                    && matches!(code[i + 8], Op::Pop)
                    && !any_jump_target(&is_jump_target, i + 1, 9)
                {
                    code[i] = Op::AddNestedIndex(acc, obj, arr, idx);
                    code[i + 1] = NOP;
                    code[i + 2] = NOP;
                    code[i + 3] = NOP;
                    code[i + 4] = NOP;
                    code[i + 5] = NOP;
                    code[i + 6] = NOP;
                    code[i + 7] = NOP;
                    code[i + 8] = NOP;
                    i += 9;
                    continue;
                }
            }
        }

        // Already fused get: GetLocal(acc), GetNestedIndex, Add, SetLocal(acc), Pop
        if i + 3 < len {
            if let (
                Op::GetLocal(acc),
                Op::GetNestedIndex(obj, arr, idx),
                Op::Add,
                Op::SetLocal(acc2),
            ) = (code[i], code[i + 1], code[i + 2], code[i + 3])
            {
                // The trailing `Pop` is part of the pattern, not an optional
                // extra. The fused opcode writes straight into the local slot
                // and pushes nothing, while the sequence it replaces nets +1
                // (`Op::SetLocal` peeks instead of popping; `Op::SetIndex`
                // re-pushes the object). Fusing without the `Pop` therefore
                // loses a stack slot, so a function whose last statement is one
                // of these — which `compile_function_body` emits with no `Pop`
                // before `Op::Return` — returns a stray local instead of its
                // value. The `IncrLocal` peephole below matches the same way.
                if acc == acc2
                    && i + 4 < len
                    && matches!(code[i + 4], Op::Pop)
                    && !any_jump_target(&is_jump_target, i + 1, 5)
                {
                    code[i] = Op::AddNestedIndex(acc, obj, arr, idx);
                    code[i + 1] = NOP;
                    code[i + 2] = NOP;
                    code[i + 3] = NOP;
                    code[i + 4] = NOP;
                    i += 5;
                    continue;
                }
            }
        }

        // `h[keys[k]] = v`
        // GetLocal(obj), GetLocal(arr), GetLocal(idx), GetIndex, GetLocal(val), SetIndex, Pop
        if i + 5 < len {
            if let (
                Op::GetLocal(obj),
                Op::GetLocal(arr),
                Op::GetLocal(idx),
                Op::GetIndex,
                Op::GetLocal(val),
                Op::SetIndex,
            ) = (
                code[i],
                code[i + 1],
                code[i + 2],
                code[i + 3],
                code[i + 4],
                code[i + 5],
            ) {
                // The trailing `Pop` is part of the pattern, not an optional
                // extra. The fused opcode writes straight into the local slot
                // and pushes nothing, while the sequence it replaces nets +1
                // (`Op::SetLocal` peeks instead of popping; `Op::SetIndex`
                // re-pushes the object). Fusing without the `Pop` therefore
                // loses a stack slot, so a function whose last statement is one
                // of these — which `compile_function_body` emits with no `Pop`
                // before `Op::Return` — returns a stray local instead of its
                // value. The `IncrLocal` peephole below matches the same way.
                if i + 6 < len
                    && matches!(code[i + 6], Op::Pop)
                    && !any_jump_target(&is_jump_target, i + 1, 7)
                {
                    code[i] = Op::SetNestedIndex(obj, arr, idx, val);
                    code[i + 1] = NOP;
                    code[i + 2] = NOP;
                    code[i + 3] = NOP;
                    code[i + 4] = NOP;
                    code[i + 5] = NOP;
                    code[i + 6] = NOP;
                    i += 7;
                    continue;
                }
            }
        }

        // GetLocal(obj), GetLocalIndex(arr, idx), GetLocal(val), SetIndex, Pop
        if i + 3 < len {
            if let (
                Op::GetLocal(obj),
                Op::GetLocalIndex(arr, idx),
                Op::GetLocal(val),
                Op::SetIndex,
            ) = (code[i], code[i + 1], code[i + 2], code[i + 3])
            {
                // The trailing `Pop` is part of the pattern, not an optional
                // extra. The fused opcode writes straight into the local slot
                // and pushes nothing, while the sequence it replaces nets +1
                // (`Op::SetLocal` peeks instead of popping; `Op::SetIndex`
                // re-pushes the object). Fusing without the `Pop` therefore
                // loses a stack slot, so a function whose last statement is one
                // of these — which `compile_function_body` emits with no `Pop`
                // before `Op::Return` — returns a stray local instead of its
                // value. The `IncrLocal` peephole below matches the same way.
                if i + 4 < len
                    && matches!(code[i + 4], Op::Pop)
                    && !any_jump_target(&is_jump_target, i + 1, 5)
                {
                    code[i] = Op::SetNestedIndex(obj, arr, idx, val);
                    code[i + 1] = NOP;
                    code[i + 2] = NOP;
                    code[i + 3] = NOP;
                    code[i + 4] = NOP;
                    i += 5;
                    continue;
                }
            }
        }

        // `h[keys[k]]` as a value
        // GetLocal(obj), GetLocal(arr), GetLocal(idx), GetIndex, GetIndex
        if i + 4 < len {
            if let (
                Op::GetLocal(obj),
                Op::GetLocal(arr),
                Op::GetLocal(idx),
                Op::GetIndex,
                Op::GetIndex,
            ) = (code[i], code[i + 1], code[i + 2], code[i + 3], code[i + 4])
            {
                if !any_jump_target(&is_jump_target, i + 1, 5) {
                    code[i] = Op::GetNestedIndex(obj, arr, idx);
                    code[i + 1] = NOP;
                    code[i + 2] = NOP;
                    code[i + 3] = NOP;
                    code[i + 4] = NOP;
                    i += 5;
                    continue;
                }
            }
        }

        // Pattern: GetLocal(s), Constant(c=1), Add, SetLocal(s), Pop → IncrLocal(s);
        // any other Int or Float `c` → AddLocalConstInPlace(s, c).
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
                // `buf = buf + "x"` — append into the local instead of allocating
                // a new string on every iteration.
                if let Some(Constant::String(_)) = constants.get(cidx as usize) {
                    code[i] = Op::AppendLocalConst(slot1, cidx);
                    code[i + 1] = NOP;
                    code[i + 2] = NOP;
                    code[i + 3] = NOP;
                    code[i + 4] = NOP;
                    i += 5;
                    continue;
                }
                // `k += 2`, `total += 0.5`: in place, where the three-op rule
                // below would leave `AddLocalConst, SetLocalPop`.
                if let Some(Constant::Int(_)) | Some(Constant::Float(_)) =
                    constants.get(cidx as usize)
                {
                    code[i] = Op::AddLocalConstInPlace(slot1, cidx);
                    code[i + 1] = NOP;
                    code[i + 2] = NOP;
                    code[i + 3] = NOP;
                    code[i + 4] = NOP;
                    i += 5;
                    continue;
                }
            }
        }

        // Top-level `buf = buf + "x"` — the name is a global, not a local.
        if let (
            Op::GetGlobal(name_idx),
            Op::Constant(cidx),
            Op::Add,
            Op::SetGlobal(slot2),
            Op::Pop,
        ) = (code[i], code[i + 1], code[i + 2], code[i + 3], code[i + 4])
        {
            if name_idx == slot2 && !any_jump_target(&is_jump_target, i + 1, 5) {
                if let Some(Constant::String(_)) = constants.get(cidx as usize) {
                    code[i] = Op::AppendGlobalConst(name_idx, cidx);
                    code[i + 1] = NOP;
                    code[i + 2] = NOP;
                    code[i + 3] = NOP;
                    code[i + 4] = NOP;
                    i += 5;
                    continue;
                }
            }
        }

        // Pattern: GetLocal(s), Constant(c=1), Subtract, SetLocal(s), Pop → DecrLocal(s);
        // any other Int or Float `c` → SubLocalConstInPlace(s, c).
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
                if let Some(Constant::Int(_)) | Some(Constant::Float(_)) =
                    constants.get(cidx as usize)
                {
                    code[i] = Op::SubLocalConstInPlace(slot1, cidx);
                    code[i + 1] = NOP;
                    code[i + 2] = NOP;
                    code[i + 3] = NOP;
                    code[i + 4] = NOP;
                    i += 5;
                    continue;
                }
            }
        }

        // Pattern: GetLocal(s), Constant(c), Multiply, SetLocal(s), Pop
        // → MulLocalConstInPlace(s, c), for an Int or Float `c`.
        if let (
            Op::GetLocal(slot1),
            Op::Constant(cidx),
            Op::Multiply,
            Op::SetLocal(slot2),
            Op::Pop,
        ) = (code[i], code[i + 1], code[i + 2], code[i + 3], code[i + 4])
        {
            if slot1 == slot2 && !any_jump_target(&is_jump_target, i + 1, 5) {
                if let Some(Constant::Int(_)) | Some(Constant::Float(_)) =
                    constants.get(cidx as usize)
                {
                    code[i] = Op::MulLocalConstInPlace(slot1, cidx);
                    code[i + 1] = NOP;
                    code[i + 2] = NOP;
                    code[i + 3] = NOP;
                    code[i + 4] = NOP;
                    i += 5;
                    continue;
                }
            }
        }

        // Pattern: a = a + b  (GetLocal, GetLocal, Add, SetLocal, Pop) → in place.
        // Must precede any 3-op rule on the same prefix, which would otherwise
        // consume it and leave the SetLocal/Pop behind.
        //
        // A second rule used to follow with the same pattern and guard, turning
        // the five ops into `AddLocalLocal(a, b), SetLocalPop(a)`. This one
        // always matched first, so that rule never fired; it is gone.
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

        // Pattern: GetLocal(s), GetProperty(c, m, z) → GetLocalProperty(s, c, m, z)
        //
        // `this.field` and `obj.field` are the most common shape in Soli's OO
        // code — every model attribute read, every controller `this.` — and the
        // pair costs ~27ns of which roughly 15ns is the second dispatch alone.
        // The fused opcode already existed and was already implemented; nothing
        // ever emitted it.
        if i + 1 < len {
            if let (Op::GetLocal(slot), Op::GetProperty(cidx, mid, mask)) = (code[i], code[i + 1]) {
                if !any_jump_target(&is_jump_target, i + 1, 2) {
                    code[i] = Op::GetLocalProperty(slot, cidx, mid, mask);
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

        // Pattern: Equal, JumpIfFalse(offset) → TestEqualJump(offset+1)
        //
        // `if x.status == "paid"`, `if a == LIMIT`: a condition whose left
        // side is not a bare local. Against a local, the `*LocalConst` and
        // `*LocalLocal` rules below take the comparison first.
        if i + 1 < len {
            if let (Op::Equal, Op::JumpIfFalse(offset)) = (code[i], code[i + 1]) {
                if !any_jump_target(&is_jump_target, i + 1, 2) {
                    code[i] = Op::TestEqualJump(offset + 1);
                    code[i + 1] = NOP;
                    i += 2;
                    continue;
                }
            }
        }

        // Pattern: GetLocal(a), GetLocal(b), Subtract → SubLocalLocal(a, b)
        //
        // `a = a - b` as a statement (and `* b`, `/ b` below) becomes
        // `SubLocalLocal(a, b), SetLocalPop(a)`: this rule takes the first three
        // ops and `fuse_set_local_pop` the `SetLocal, Pop` left after them.
        // Five-op rules for the same result used to sit further down. This rule
        // always matched first at their head, so they never fired; they are gone.
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

        // Pattern: GetLocal(a), Constant(c), <cmp> → <Cmp>LocalConst(a, c):
        // `x < 10`, `v > 100` — the most common comparison, and the whole body
        // of a predicate callback. `if` then pairs it with one `JumpIfFalse`
        // (2 ops) where it used to be `GetLocal, Constant, TestLessJump` (3).
        //
        // `role == "admin"`, `status != 200`, `x == 0` too, for any constant:
        // the fused op compares a string local with a string constant in
        // place, without copying either onto the stack, and its general arm
        // decides the rest with `values_equal`, as `Equal` does. This takes
        // `x == 0` / `x != 0` from `IsZeroLocal` / `NotZeroLocal`, whose
        // arms answered "not zero" for a `grouped` read without forcing it.
        if i + 2 < len {
            if let (Op::GetLocal(slot), Op::Constant(cidx)) = (code[i], code[i + 1]) {
                let fused = match code[i + 2] {
                    Op::Less => Some(Op::LessLocalConst(slot, cidx)),
                    Op::LessEqual => Some(Op::LessEqualLocalConst(slot, cidx)),
                    Op::Greater => Some(Op::GreaterLocalConst(slot, cidx)),
                    Op::GreaterEqual => Some(Op::GreaterEqualLocalConst(slot, cidx)),
                    Op::Equal => Some(Op::EqualLocalConst(slot, cidx)),
                    Op::NotEqual => Some(Op::NotEqualLocalConst(slot, cidx)),
                    _ => None,
                };
                if let Some(fused) = fused {
                    if !any_jump_target(&is_jump_target, i + 1, 3) {
                        code[i] = fused;
                        code[i + 1] = NOP;
                        code[i + 2] = NOP;
                        i += 3;
                        continue;
                    }
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

        // Pattern: GetLocal(a), GetLocal(b), Equal|NotEqual →
        // EqualLocalLocal(a, b) | NotEqualLocalLocal(a, b): `i == n`,
        // `name != previous`, compared in place.
        if i + 2 < len {
            if let (Op::GetLocal(slot_a), Op::GetLocal(slot_b)) = (code[i], code[i + 1]) {
                let fused = match code[i + 2] {
                    Op::Equal => Some(Op::EqualLocalLocal(slot_a, slot_b)),
                    Op::NotEqual => Some(Op::NotEqualLocalLocal(slot_a, slot_b)),
                    _ => None,
                };
                if let Some(fused) = fused {
                    if !any_jump_target(&is_jump_target, i + 1, 3) {
                        code[i] = fused;
                        code[i + 1] = NOP;
                        code[i + 2] = NOP;
                        i += 3;
                        continue;
                    }
                }
            }
        }

        // Pattern: GetLocal(slot), Constant(c), Add → AddLocalConst(slot, c)
        //
        // This takes every Int constant too. Two rules that turned
        // `GetLocal, Constant(Int), Add` into `AddLocalInt(slot, n)` sat further
        // down with the same guard, so they never fired; they are gone, and
        // nothing emits `AddLocalInt`. `AddLocalConst` is the form to keep: the
        // callback kernels (`Kernel::of`) read it, and `AddLocalInt` would hide
        // a `x + 1` block from them.
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

        // Pattern: GetLocal(slot), Constant(c), Modulo → ModLocalConst(slot, c)
        // (`i % 2 == 0`, `y = k % 7`): one op, answered in the fast tier.
        if i + 2 < len {
            if let (Op::GetLocal(slot), Op::Constant(cidx), Op::Modulo) =
                (code[i], code[i + 1], code[i + 2])
            {
                if !any_jump_target(&is_jump_target, i + 1, 3) {
                    if let Some(Constant::Int(_)) | Some(Constant::Float(_)) =
                        constants.get(cidx as usize)
                    {
                        code[i] = Op::ModLocalConst(slot, cidx);
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

        // Pattern: Null, JumpIfFalse → JumpIfNull (but we don't have JumpIfFalse directly, it's an operand)
        // This needs special handling since JumpIfFalse has an operand

        // Pattern: GetLocal, Not, JumpIfFalse(offset)
        //        → JumpIfLocalTruthy(slot, offset + 2)
        //
        // A `!flag` the compiler did not fold into a `JumpIfTrue` (see
        // `compile_condition_jump`): the branch is taken when the local is
        // truthy. The fused op sits two instructions before the old jump, so
        // its offset grows by two to reach the same target (`compact_nops`
        // then drops the NOPs and remaps it). This used to be
        // `IsFalsyLocal, NOP, JumpIfFalse`: two dispatches and a pushed Bool.
        if i + 2 < len {
            if let (Op::GetLocal(slot), Op::Not, Op::JumpIfFalse(offset)) =
                (code[i], code[i + 1], code[i + 2])
            {
                if !any_jump_target(&is_jump_target, i + 1, 3) {
                    code[i] = Op::JumpIfLocalTruthy(slot, offset + 2);
                    code[i + 1] = NOP;
                    code[i + 2] = NOP;
                    i += 3;
                    continue;
                }
            }
        }

        // Pattern: GetLocal, JumpIfFalse(offset) → JumpIfLocalFalsy(slot, offset + 1)
        //
        // `if flag`, `while flag`, `x = 1 if flag`: one dispatch that reads
        // the slot in place, where `IsTruthyLocal, JumpIfFalse` pushed a Bool
        // only for the next op to pop it.
        if i + 1 < len {
            if let (Op::GetLocal(slot), Op::JumpIfFalse(offset)) = (code[i], code[i + 1]) {
                if !any_jump_target(&is_jump_target, i + 1, 2) {
                    code[i] = Op::JumpIfLocalFalsy(slot, offset + 1);
                    code[i + 1] = NOP;
                    i += 2;
                    continue;
                }
            }
        }

        // Pattern: GetLocal, JumpIfTrue(offset) → JumpIfLocalTruthy(slot, offset + 1)
        //
        // `unless flag`, `x = 1 unless flag`, `if !flag`: the compiler folds
        // the `!` into a `JumpIfTrue`, and this reads the slot in place.
        if i + 1 < len {
            if let (Op::GetLocal(slot), Op::JumpIfTrue(offset)) = (code[i], code[i + 1]) {
                if !any_jump_target(&is_jump_target, i + 1, 2) {
                    code[i] = Op::JumpIfLocalTruthy(slot, offset + 1);
                    code[i + 1] = NOP;
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

        // `x == 0` and `x != 0` are `EqualLocalConst` / `NotEqualLocalConst`
        // (the `<Cmp>LocalConst` rule above); `IsZeroLocal` / `NotZeroLocal`
        // are no longer emitted.

        // NB: `GetLocal(s), SetLocal(s)` (`x = x`, a same-scope `let x = x`,
        // `@x = @x` in a view) is deliberately NOT fused. It used to become
        // `SwapSetLocal(s)`, which popped a value the pair never pushed: the
        // store took whatever sat on top of the stack, usually the last local,
        // and the statement's `Pop` then dropped one slot too many. Two locals
        // were swapped and the next read of the last one indexed past the stack
        // and panicked. The pair already leaves the slot as it was, so a fusion
        // would only save a dispatch on code nobody writes.

        i += 1;
    }

    // Last, once every longer pattern that ends in `SetLocal, Pop` has had its
    // pick: the plain store left behind.
    fuse_set_local_pop(code, &is_jump_target);

    // Remove NOPs (compact the bytecode) - adjust jump offsets accordingly
    compact_nops(chunk);
}

/// `SetLocal(s), Pop` → `SetLocalPop(s)`.
///
/// An assignment statement compiles to its value, `SetLocal(x)` and the
/// statement's `Pop`. So do a re-`let` of a name in the same scope and the end
/// of each `match` arm, which moves the arm's value into the subject's slot.
/// `SetLocal` copies the value into the slot and leaves it on the stack for the
/// `Pop` to drop. `SetLocalPop` moves it in instead, which saves a dispatch and,
/// for a string, array, hash or instance, a reference-count increment and
/// decrement. The VM audit measured `y = x` 3.4 ns faster with an Int and
/// 6.8 ns faster with a String this way.
///
/// It runs after the main scan, so `IncrLocal`, `AddLocalsInPlace`,
/// `AppendLocalConst`, `AddNestedIndex` and the other patterns that end in the
/// pair have already taken theirs. What is left is a plain store, often after a
/// fused read: `y = k + 2` is `AddLocalConst, SetLocal, Pop`.
///
/// Like every rule of the main scan, it takes a store whose `SetLocal` a
/// branch lands on. `x = a || b`,
/// `x = c ? a : b`, `x = match …`, `x = f() rescue nil` and `x ||= y` all
/// merge on the `SetLocal`. The fused op takes the `SetLocal`'s index,
/// `compact_nops` maps the branch onto it, and it does what the pair did. A
/// branch must not land on the `Pop`, though: `ok && (x = 1)` as a statement
/// short-circuits straight to it with `ok` on the stack, and that path needs
/// the `Pop` on its own.
fn fuse_set_local_pop(code: &mut [Op], is_jump_target: &[bool]) {
    let mut i = 0;
    while i + 1 < code.len() {
        if let (Op::SetLocal(slot), Op::Pop) = (code[i], code[i + 1]) {
            if !any_jump_target(is_jump_target, i + 1, 2) {
                code[i] = Op::SetLocalPop(slot);
                code[i + 1] = NOP;
                i += 2;
                continue;
            }
        }
        i += 1;
    }
}

/// Whether a branch lands inside a `count`-instruction pattern whose tail
/// starts at `start` (callers pass the head's index + 1): any of the
/// `count - 1` instructions the fusion would blank.
///
/// The range used to be `start + 1 .. start + count`, one too far on both
/// ends: it skipped the instruction right after the head — the one a branch
/// most often targets — and checked one past the pattern instead. In
/// `if a && b < c` the `&&` short-circuit jumps to the `if`'s `JumpIfFalse`,
/// which is exactly that instruction, so `Less, JumpIfFalse` was fused into
/// `TestLessJump` anyway: the short-circuit then landed on the blanked slot's
/// successor, inside the then-branch, and ran it with a stray `false` on the
/// stack. `if k > 4 && k < 7` over `0..10` summed to 0 under `soli serve`.
fn any_jump_target(targets: &[bool], start: usize, count: usize) -> bool {
    for j in start..(start + count - 1) {
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
    let mut columns = Vec::with_capacity(next);
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
            Op::JumpIfTrue(d) => Op::JumpIfTrue(fwd(&old_to_new, old, d, here)),
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
            Op::TestEqualJump(d) => Op::TestEqualJump(fwd(&old_to_new, old, d, here)),
            Op::JumpIfParamSupplied(p, d) => {
                Op::JumpIfParamSupplied(p, fwd(&old_to_new, old, d, here))
            }
            Op::JumpIfLocalFalsy(slot, d) => {
                Op::JumpIfLocalFalsy(slot, fwd(&old_to_new, old, d, here))
            }
            Op::JumpIfLocalTruthy(slot, d) => {
                Op::JumpIfLocalTruthy(slot, fwd(&old_to_new, old, d, here))
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
        columns.push(chunk.columns.get(old).copied().unwrap_or(0));
    }

    chunk.code = code;
    chunk.lines = lines;
    chunk.columns = columns;
}

/// Remap a forward jump offset through the old→new index table.
#[inline]
fn fwd(old_to_new: &[usize], old: usize, d: u16, here: usize) -> u16 {
    let target = old_to_new[old + 1 + d as usize];
    (target - (here + 1)) as u16
}

/// The namespace a tree-walker function was declared in — what its
/// environment records for a class nested in a module.
pub(crate) fn namespace_of(func: &crate::interpreter::value::Function) -> Option<Arc<str>> {
    match func
        .closure
        .borrow()
        .get(crate::interpreter::domain::NAMESPACE_BINDING)
    {
        Some(crate::interpreter::value::Value::String(ns)) => Some(Arc::from(ns.as_str())),
        _ => None,
    }
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
            Op::JumpIfTrue(_) => Op::JumpIfTrue(d),
            Op::JumpIfFalseNoPop(_) => Op::JumpIfFalseNoPop(d),
            Op::JumpIfTrueNoPop(_) => Op::JumpIfTrueNoPop(d),
            Op::JumpIfNull(_) => Op::JumpIfNull(d),
            Op::JumpIfNotNull(_) => Op::JumpIfNotNull(d),
            Op::NullishJump(_) => Op::NullishJump(d),
            Op::JumpIfParamSupplied(p, _) => Op::JumpIfParamSupplied(p, d),
            Op::JumpIfLocalFalsy(slot, _) => Op::JumpIfLocalFalsy(slot, d),
            Op::JumpIfLocalTruthy(slot, _) => Op::JumpIfLocalTruthy(slot, d),
            Op::ForIter(_) => Op::ForIter(d),
            Op::ForIterRange(_) => Op::ForIterRange(d),
            Op::RescueJump(_) => Op::RescueJump(d),
            Op::CatchMatch(n, _) => Op::CatchMatch(n, d),
            Op::TestLessJump(_) => Op::TestLessJump(d),
            Op::TestLessEqualJump(_) => Op::TestLessEqualJump(d),
            Op::TestGreaterJump(_) => Op::TestGreaterJump(d),
            Op::TestGreaterEqualJump(_) => Op::TestGreaterEqualJump(d),
            Op::TestNotEqualJump(_) => Op::TestNotEqualJump(d),
            Op::TestEqualJump(_) => Op::TestEqualJump(d),
            Op::TryBegin(_, f) => Op::TryBegin(d, f),
            Op::MatchType(n, _) => Op::MatchType(n, d),
            other => panic!("FORWARD_BRANCH_OPS holds a non-branch opcode: {other:?}"),
        }
    }

    fn first_offset(op: &Op) -> Option<u16> {
        Some(match *op {
            Op::Jump(d)
            | Op::JumpIfFalse(d)
            | Op::JumpIfTrue(d)
            | Op::JumpIfFalseNoPop(d)
            | Op::JumpIfTrueNoPop(d)
            | Op::JumpIfNull(d)
            | Op::JumpIfNotNull(d)
            | Op::NullishJump(d)
            | Op::JumpIfParamSupplied(_, d)
            | Op::JumpIfLocalFalsy(_, d)
            | Op::JumpIfLocalTruthy(_, d)
            | Op::ForIter(d)
            | Op::ForIterRange(d)
            | Op::RescueJump(d)
            | Op::CatchMatch(_, d)
            | Op::TestLessJump(d)
            | Op::TestLessEqualJump(d)
            | Op::TestGreaterJump(d)
            | Op::TestGreaterEqualJump(d)
            | Op::TestNotEqualJump(d)
            | Op::TestEqualJump(d)
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

    /// `o.x ||= v`, `&&=` and `??=`: the jump taken when the read decides
    /// the result lands after the `SetProperty`, so a short-circuit writes
    /// nothing (the tree-walker's order), and the receiver is loaded once
    /// before the read — the second load belongs to the assigning path.
    #[test]
    fn member_short_circuit_assign_jumps_over_the_write() {
        for op in ["||=", "&&=", "??="] {
            let ops = fn_ops(&format!("o = nil\no.x {op} 1"));
            let read = ops
                .iter()
                .position(|o| matches!(o, Op::GetPropertyOrNull(_)))
                .unwrap_or_else(|| panic!("{op}: no GetPropertyOrNull in {ops:?}"));
            let target = match ops[read + 1] {
                Op::JumpIfTrueNoPop(d) | Op::JumpIfFalseNoPop(d) | Op::JumpIfNotNull(d) => {
                    read + 2 + d as usize
                }
                ref other => panic!("{op}: expected a no-pop jump, got {other:?}"),
            };
            assert!(
                matches!(ops[target - 1], Op::SetProperty(_)),
                "{op}: the short-circuit must jump over SetProperty: {ops:?}"
            );
            assert_eq!(
                ops[..read]
                    .iter()
                    .filter(|o| matches!(o, Op::GetLocal(_)))
                    .count(),
                1,
                "{op}: one receiver load before the read: {ops:?}"
            );
        }
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

    #[test]
    fn hash_array_index_add_fuses() {
        // `total` is read afterwards so the assignment sits in statement
        // position and carries its `Pop` — which the fusion requires, because
        // `AddNestedIndex` has stack effect 0 where the sequence nets +1.
        let ops = fn_ops(
            r#"
            let h = {"k0": 1}
            let keys = ["k0"]
            let k = 0
            let total = 0
            total = total + h[keys[k]]
            total
            "#,
        );
        assert!(
            ops.iter()
                .any(|o| matches!(o, Op::AddNestedIndex(_, _, _, _))),
            "expected AddNestedIndex, got: {:?}",
            ops
        );
    }

    /// In tail position the compiler emits no `Pop`, so these fusions must not
    /// fire: they push nothing where the sequence they replace leaves a value,
    /// and the function would return a stray local instead of its result.
    #[test]
    fn nested_index_statements_do_not_fuse_in_tail_position() {
        let add_ops = fn_ops(
            r#"
            let h = {"k0": 1}
            let keys = ["k0"]
            let k = 0
            let total = 0
            total = total + h[keys[k]]
            "#,
        );
        assert!(
            !add_ops
                .iter()
                .any(|o| matches!(o, Op::AddNestedIndex(_, _, _, _))),
            "must not fuse without a trailing Pop, got: {:?}",
            add_ops
        );

        let set_ops = fn_ops(
            r#"
            let h = {"k0": 1}
            let keys = ["k0"]
            let k = 0
            let v = 2
            h[keys[k]] = v
            "#,
        );
        assert!(
            !set_ops
                .iter()
                .any(|o| matches!(o, Op::SetNestedIndex(_, _, _, _))),
            "must not fuse without a trailing Pop, got: {:?}",
            set_ops
        );
    }

    /// The values those tail statements produce, end to end. `+=` yields the
    /// new accumulator; an index assignment yields the assigned value (not the
    /// container, and not null).
    #[test]
    fn nested_index_tail_statements_return_the_right_value() {
        use crate::interpreter::value::Value;
        let run = |body: &str| {
            let src = format!("def f() {{\n{body}\n}}\n");
            let tokens = Scanner::new(&src).scan_tokens().expect("lex");
            let program = Parser::new(tokens).parse().expect("parse");
            let module = Compiler::compile(&program).expect("compile");
            let proto = module
                .main
                .chunk
                .constants
                .iter()
                .find_map(|c| match c {
                    Constant::Function(p) => Some(p.clone()),
                    _ => None,
                })
                .expect("function proto");
            let mut vm = crate::vm::Vm::new();
            vm.execute(&proto).expect("vm")
        };

        assert_eq!(
            run(r#"
            let sentinel = "OOPS"
            let h = {"k0": 5}
            let keys = ["k0"]
            let k = 0
            let total = 100
            total = total + h[keys[k]]
            "#),
            Value::Int(105)
        );

        assert_eq!(
            run(r#"
            let sentinel = "OOPS"
            let h = {"k0": 1}
            let keys = ["k0"]
            let k = 0
            let v = 42
            h[keys[k]] = v
            "#),
            Value::Int(42)
        );

        // The constant-key opcodes take the same rule.
        assert_eq!(
            run(r#"
            let sentinel = "OOPS"
            let h = {"k0": 1}
            h["k0"] = 7
            "#),
            Value::Int(7)
        );
    }

    #[test]
    fn req_params_id_fuses_to_const2() {
        let ops = fn_ops(
            r#"
            let req = {"params": {"id": "7"}}
            req["params"]["id"]
            "#,
        );
        assert!(
            ops.iter()
                .any(|o| matches!(o, Op::HashGetLocalConst2(_, _, _))),
            "expected HashGetLocalConst2, got: {:?}",
            ops
        );
    }

    #[test]
    fn req_params_id_evaluates_on_vm() {
        let ops_src = r#"
            let req = {"params": {"id": "7"}, "query": {}}
            return req["params"]["id"]
            "#;
        let ops = fn_ops(ops_src);
        assert!(
            ops.iter()
                .any(|o| matches!(o, Op::HashGetLocalConst2(_, _, _))),
            "expected HashGetLocalConst2 in function body, got: {:?}",
            ops
        );
        let src = format!("def f() {{\n{ops_src}\n}}\n");
        let tokens = Scanner::new(&src).scan_tokens().expect("lex");
        let program = Parser::new(tokens).parse().expect("parse");
        let module = Compiler::compile(&program).expect("compile");
        let proto = module
            .main
            .chunk
            .constants
            .iter()
            .find_map(|c| match c {
                Constant::Function(p) => Some(p.clone()),
                _ => None,
            })
            .expect("function proto");
        let mut vm = crate::vm::Vm::new();
        let value = vm.execute(&proto).expect("vm");
        assert_eq!(value, crate::interpreter::value::Value::String("7".into()));
    }

    #[test]
    fn hash_array_index_set_fuses() {
        let ops = fn_ops(
            r#"
            let h = {"k0": 1}
            let keys = ["k0"]
            let k = 0
            let v = 2
            h[keys[k]] = v
            h
            "#,
        );
        assert!(
            ops.iter()
                .any(|o| matches!(o, Op::SetNestedIndex(_, _, _, _))),
            "expected SetNestedIndex, got: {:?}",
            ops
        );
    }

    /// Run `program` on the VM and show its global `result` as `print` would.
    fn vm_result(program: &str) -> String {
        let tokens = Scanner::new(program).scan_tokens().expect("lex");
        let parsed = Parser::new(tokens).parse().expect("parse");
        let module = Compiler::compile(&parsed).expect("compile");
        let mut vm = crate::vm::Vm::new();
        if let Err(err) = vm.execute(&module.main) {
            panic!("{program}\nraised: {err}");
        }
        match vm.globals.get("result") {
            Some(value) => value.to_string(),
            None => panic!("{program}\nset no `result`"),
        }
    }

    /// The global-update ops read their name by sharing the constant rather
    /// than copying it (T33.20): a top-level `g = g + <expr>` is
    /// `AddAssignGlobal`, `g << x` is `AppendGlobal`, `g = g + "lit"` is
    /// `AppendGlobalConst`. Each still updates the global, whether the name is
    /// short enough to sit inline or long enough to be reference-counted, and
    /// so do `super.m(...)` and `send` with either kind of name.
    #[test]
    fn global_updates_super_and_send_share_their_names() {
        let program = "total = 0\nan_accumulator_with_a_long_name = 0\n\
                       for i in [1, 2, 3] {\n  total = total + i * 2\n  an_accumulator_with_a_long_name = an_accumulator_with_a_long_name + i * 3\n}\n\
                       buf = \"a\"\nbuf << \"b\"\na_long_buffer_name_for_appends = \"c\"\na_long_buffer_name_for_appends << \"d\"\n\
                       out = \"\"\nfor w in [1, 2] { out = out + \"x\" }\n\
                       class Base\n  def greet(x)\n    \"base #{x}\"\n  end\n  def a_method_name_longer_than_fifteen\n    \"long\"\n  end\nend\n\
                       class Kid < Base\n  def greet(x)\n    super.greet(x) + \"!\"\n  end\n  def a_method_name_longer_than_fifteen\n    super.a_method_name_longer_than_fifteen() + \"er\"\n  end\nend\n\
                       k = Kid.new()\n\
                       result = [total, an_accumulator_with_a_long_name, buf, a_long_buffer_name_for_appends, out, k.greet(1), k.a_method_name_longer_than_fifteen(), k.send(\"greet\", 2), k.send(\"a_method_name_longer_than_fifteen\")]";
        let tokens = Scanner::new(program).scan_tokens().expect("lex");
        let parsed = Parser::new(tokens).parse().expect("parse");
        let module = Compiler::compile(&parsed).expect("compile");
        let ops = &module.main.chunk.code;
        assert_eq!(
            ops.iter()
                .filter(|op| matches!(op, Op::AddAssignGlobal(_)))
                .count(),
            2,
            "{ops:?}"
        );
        assert_eq!(
            ops.iter()
                .filter(|op| matches!(op, Op::AppendGlobal(_)))
                .count(),
            2,
            "{ops:?}"
        );
        assert!(
            ops.iter().any(|op| matches!(op, Op::AppendGlobalConst(..))),
            "{ops:?}"
        );
        assert_eq!(
            vm_result(program),
            "[12, 18, ab, cd, xx, base 1!, longer, base 2!, longer]"
        );
    }

    /// An array pattern tests its shape in one `MatchArrayLen` (exact, or
    /// at-least with `...rest`) and builds the rest with `ArrayRestLocal`: no
    /// type-name test, no `length` member read, no `slice` call. Then the
    /// shapes still answer as the tree-walker does, nested ones included.
    #[test]
    fn array_pattern_tests_its_shape_in_one_op() {
        let ops = fn_ops(
            "v = [1, 2, 3]\nmatch v {\n  [a, b] => a + b,\n  [[x], ...rest] => x,\n  [first, ...rest] => first,\n  _ => 0\n}",
        );
        // The first arm's test names the subject's slot.
        let subject = match ops.iter().find(|op| matches!(op, Op::MatchArrayLen(..))) {
            Some(Op::MatchArrayLen(slot, 2, 0)) => *slot,
            other => panic!("first shape test is {other:?}: {ops:?}"),
        };
        assert!(ops.contains(&Op::MatchArrayLen(subject, 1, 1)), "{ops:?}");
        assert!(ops.contains(&Op::ArrayRestLocal(subject, 1)), "{ops:?}");
        // `[x]` runs on the element parked in its own slot.
        assert!(
            ops.iter()
                .any(|op| matches!(op, Op::MatchArrayLen(slot, 1, 0) if *slot > subject)),
            "{ops:?}"
        );
        assert!(
            !ops.iter().any(|op| matches!(
                op,
                Op::MatchType(..)
                    | Op::GetProperty(..)
                    | Op::GetLocalProperty(..)
                    | Op::CallMethod(..)
                    | Op::CallMethodById(..)
                    | Op::GreaterEqual
            )),
            "{ops:?}"
        );

        let program = "def m(v)\n  match v {\n    [] => 0,\n    [[x, y], z] => x * y + z,\n    [a, b] => a + b,\n    [first, ...rest] => [first, rest],\n    _ => -1\n  }\nend\n\
                       let result = [m([]), m([[2, 3], 4]), m([1, 2]), m([1, 2, 3]), m([9]), m(\"ab\"), m({\"a\": 1}), m(nil)]";
        assert_eq!(
            vm_result(program),
            "[0, 10, 3, [1, [2, 3]], [9, []], -1, -1, -1]"
        );
    }

    /// A branch on a bare local is one dispatch: `if flag` is
    /// `JumpIfLocalFalsy`, `unless flag` and `if !flag` are
    /// `JumpIfLocalTruthy`. They were `IsTruthyLocal, JumpIfFalse` and
    /// `IsFalsyLocal, JumpIfFalse`, a Bool pushed only to be popped.
    #[test]
    fn local_condition_compiles_to_one_branch() {
        // Slot 0 is the callee, so `flag` is slot 1.
        type Case = (&'static str, fn(&Op) -> bool);
        let cases: [Case; 4] = [
            ("flag = true\nn = 0\nif flag { n = 1 }\nn", |op| {
                matches!(op, Op::JumpIfLocalFalsy(1, _))
            }),
            ("flag = true\nn = 0\nn = 1 if flag\nn", |op| {
                matches!(op, Op::JumpIfLocalFalsy(1, _))
            }),
            ("flag = true\nn = 0\nunless flag { n = 1 }\nn", |op| {
                matches!(op, Op::JumpIfLocalTruthy(1, _))
            }),
            ("flag = true\nn = 0\nif !flag { n = 1 }\nn", |op| {
                matches!(op, Op::JumpIfLocalTruthy(1, _))
            }),
        ];
        for (body, fused) in cases {
            let ops = fn_ops(body);
            assert!(ops.iter().any(fused), "{body}\n{ops:?}");
            assert!(
                !ops.iter().any(|op| matches!(
                    op,
                    Op::IsTruthyLocal(_)
                        | Op::IsFalsyLocal(_)
                        | Op::Not
                        | Op::JumpIfFalse(_)
                        | Op::JumpIfTrue(_)
                )),
                "{body}\n{ops:?}"
            );
        }

        // Every truthiness class, through both fused branches, with the
        // `else` target remapped past the NOPs the fusion left.
        let program = "def t(v)\n  r = \"\"\n  if v\n    r = \"T\"\n  else\n    r = \"F\"\n  end\n  r\nend\n\
                       def u(v)\n  r = \"\"\n  unless v\n    r = \"F\"\n  else\n    r = \"T\"\n  end\n  r\nend\n\
                       def n(v)\n  r = \"\"\n  if !v\n    r = \"F\"\n  else\n    r = \"T\"\n  end\n  r\nend\n\
                       vals = [true, false, nil, 0, 1, -1, 0.0, \"\", \"a\", [], [0], {}, {\"a\": 1}, :s, fn(x) { x }]\n\
                       let result = [vals.map { |v| t(v) }.join(\"\"), vals.map { |v| u(v) }.join(\"\"), \
                       vals.map { |v| n(v) }.join(\"\")]";
        assert_eq!(
            vm_result(program),
            "[TFFFTTTFTFTFTTT, TFFFTTTFTFTFTTT, TFFFTTTFTFTFTTT]"
        );
    }

    /// `unless <expr>`, `x unless <expr>`, `return x unless <expr>` and
    /// `if !<expr>` branch with one `JumpIfTrue` on the expression's value:
    /// the `!` is folded into the jump instead of compiled as `Not`. They
    /// were `<expr>, Not, JumpIfFalse`. `if !!<expr>` and `unless !<expr>`
    /// fold back to a plain `JumpIfFalse`.
    #[test]
    fn negated_condition_folds_into_jump_if_true() {
        let inverted = [
            "i = 3\nn = 0\nunless i < 0 { n = 1 }\nn",
            "i = 3\nn = 0\nn = 1 unless i < 0\nn",
            "i = 3\nn = 0\nif !(i < 0) { n = 1 }\nn",
            "i = 3\nn = 0\nif !(i % 2 == 0) { n = 1 }\nn",
            "i = 3\nreturn 1 unless i < 0\n2",
            "i = 0\nwhile !(i >= 3) { i += 1 }\ni",
            "i = 3\nn = !(i < 0) ? 1 : 2\nn",
            "i = 3\nn = 0\nunless !!(i < 0) { n = 1 }\nn",
        ];
        for body in inverted {
            let ops = fn_ops(body);
            assert!(
                ops.iter().any(|op| matches!(op, Op::JumpIfTrue(_))),
                "{body}\n{ops:?}"
            );
            assert!(
                !ops.iter().any(|op| matches!(op, Op::Not)),
                "{body}\n{ops:?}"
            );
        }
        for body in [
            "i = 3\nn = 0\nif !!(i < 0) { n = 1 }\nn",
            "i = 3\nn = 0\nunless !(i < 0) { n = 1 }\nn",
        ] {
            let ops = fn_ops(body);
            assert!(
                !ops.iter()
                    .any(|op| matches!(op, Op::Not | Op::JumpIfTrue(_))),
                "{body}\n{ops:?}"
            );
        }

        // Every truthiness class through a non-local condition, each form
        // agreeing with `if` on the same value; the `else` target and the
        // postfix-guard return are remapped past the peephole's NOPs.
        let program = r#"def id(v)
  v
end
def unless_block(v)
  r = ""
  unless id(v)
    r = "F"
  else
    r = "T"
  end
  r
end
def bang_block(v)
  if !id(v)
    "F"
  else
    "T"
  end
end
def guard(v)
  return "F" unless id(v)

  "T"
end
def postfix(v)
  r = "T"
  r = "F" unless id(v)
  r
end
def double_bang(v)
  if !!id(v)
    "T"
  else
    "F"
  end
end
def ternary(v)
  !id(v) ? "F" : "T"
end
vals = [true, false, nil, 0, 1, -1, 0.0, "", "a", [], [0], {}, {"a": 1}, :s, fn(x) { x }]
let result = [unless_block, bang_block, guard, postfix, double_bang, ternary].map { |form| vals.map { |v| form(v) }.join("") }"#;
        let row = "TFFFTTTFTFTFTTT";
        assert_eq!(
            vm_result(program),
            format!("[{row}, {row}, {row}, {row}, {row}, {row}]")
        );
    }

    /// A self-assignment stays a plain read and store: `GetLocal(a)`, then
    /// the statement's `SetLocal(a), Pop` as one `SetLocalPop(a)`, which pops
    /// the value the `GetLocal` pushed. It used to be fused into
    /// `SwapSetLocal`, which popped a value the pair never pushed (see the
    /// note in `peephole_optimize_chunk`).
    #[test]
    fn self_assignment_is_left_as_compiled() {
        let ops = fn_ops("a = 1\nb = 2\na = a\nreturn [a, b]");
        assert!(
            ops.windows(2).any(
                |pair| matches!(pair, [Op::GetLocal(read), Op::SetLocalPop(write)] if read == write)
            ),
            "expected `a = a` to be GetLocal + SetLocalPop, got: {ops:?}"
        );
    }

    /// `x = x` in each of its spellings leaves `x` and every other local as
    /// they were. Fused, the store took the last local's value, the
    /// statement's `Pop` dropped one slot too many, and the next read of the
    /// last local indexed past the stack: a panic.
    #[test]
    fn self_assignment_keeps_every_local() {
        let cases = [
            (
                "def f()\n  a = 1\n  b = 2\n  a = a\n  [a, b]\nend\nlet result = f()",
                "[1, 2]",
            ),
            (
                "def f()\n  a = 1\n  b = 2\n  a = a\n  [b, a]\nend\nlet result = f()",
                "[2, 1]",
            ),
            (
                "def f(p, q)\n  p = p\n  [p, q]\nend\nlet result = f(3, 4)",
                "[3, 4]",
            ),
            (
                "def f()\n  x = 5\n  y = (x = x)\n  [x, y]\nend\nlet result = f()",
                "[5, 5]",
            ),
            (
                "def f(n)\n  x = 0\n  i = 0\n  while i < n\n    i += 1\n    x = i\n    x = x\n  end\n  x\nend\nlet result = f(3)",
                "3",
            ),
            // A `let` of a name already in the scope assigns it.
            (
                "def f()\n  let a = 1\n  let b = 2\n  let a = a\n  [a, b]\nend\nlet result = f()",
                "[1, 2]",
            ),
        ];
        for (program, expected) in cases {
            assert_eq!(vm_result(program), expected, "{program}");
        }
    }

    /// How many `SetLocal, Pop` pairs are left in `ops`.
    fn set_local_pops(ops: &[Op]) -> usize {
        ops.windows(2)
            .filter(|pair| matches!(pair, [Op::SetLocal(_), Op::Pop]))
            .count()
    }

    /// An assignment statement stores with `SetLocalPop`: the value moves
    /// into the slot, where `SetLocal, Pop` copied it there and then dropped
    /// the copy. So does every other `SetLocal, Pop` that no longer pattern
    /// covers: a store after a fused read (`y = k + 2`), a re-`let`, and each
    /// `match` arm moving its value into the subject's slot. A function's
    /// last statement has no `Pop` (its value is the return value), so its
    /// `SetLocal` stays. A local's first assignment declares it in place, as
    /// `let` does (`compiler_hoist`), with no store at all, so each case
    /// stores into a local that already exists.
    #[test]
    fn assignment_statements_store_with_set_local_pop() {
        let cases = [
            "s = \"on the heap\"\ny = nil\ny = s\ny",
            "k = 1\ny = 0\ny = k + 2\ny",
            "n = 0\nn = [1, 2].length\nn",
            "let a = 1\nlet a = 2\na",
            "v = 2\nmatch v {\n  1 => \"one\",\n  _ => \"other\"\n}\nv",
        ];
        for body in cases {
            let ops = fn_ops(body);
            assert_eq!(
                set_local_pops(&ops),
                0,
                "{body}\na SetLocal, Pop is left: {ops:?}"
            );
            assert!(
                ops.iter().any(|op| matches!(op, Op::SetLocalPop(_))),
                "{body}\nno SetLocalPop: {ops:?}"
            );
        }

        let tail = fn_ops("y = 1\nx = 0\nx = y");
        assert!(
            tail.windows(2)
                .any(|pair| matches!(pair, [Op::SetLocal(_), Op::Return])),
            "the last statement keeps its SetLocal: {tail:?}"
        );
        assert_eq!(
            vm_result("def f\n  y = 1\n  x = y + 4\nend\nlet result = f()"),
            "5"
        );
    }

    /// `a = a - b`, `* b` and `/ b` as statements are two ops: the three-op
    /// read takes the head and the store is `SetLocalPop`. Five-op rules that
    /// aimed at the same two ops never fired, because the three-op rules
    /// match first at the same head; this is what those rules were for.
    #[test]
    fn self_arithmetic_statements_are_two_ops() {
        // `a` and `b` are hoisted into slots 1 and 2; slot 0 is the callee.
        for (operator, fused) in [
            ("-", Op::SubLocalLocal(1, 2)),
            ("*", Op::MulLocalLocal(1, 2)),
            ("/", Op::DivLocalLocal(1, 2)),
        ] {
            let body = format!("a = 12\nb = 3\na = a {operator} b\na");
            let ops = fn_ops(&body);
            assert!(
                ops.windows(2)
                    .any(|pair| pair == [fused, Op::SetLocalPop(1)]),
                "{body}\nexpected {fused:?}, SetLocalPop(1): {ops:?}"
            );
        }
        assert_eq!(
            vm_result(
                "def f\n  a = 12\n  b = 3\n  a = a - b\n  a = a * b\n  a = a / b\n  a\nend\nlet result = f()"
            ),
            "9"
        );
    }

    /// `k += 2`, `k -= 2` and `k *= 2` with an Int or Float constant are one
    /// in-place op, as `k += 1` is `IncrLocal`: they were
    /// `AddLocalConst, SetLocalPop`, a push and a pop the slot did not need.
    /// `+= 1` and `-= 1` keep their own ops, a string constant keeps
    /// `AppendLocalConst`, a store into another local keeps the fused read,
    /// and a function's last statement, which has no `Pop`, is left alone.
    #[test]
    fn self_arithmetic_with_a_constant_is_one_in_place_op() {
        // `k` is hoisted into slot 1; slot 0 is the callee.
        let in_place = |op: &Op| match op {
            Op::AddLocalConstInPlace(slot, _) => Some(("+", *slot)),
            Op::SubLocalConstInPlace(slot, _) => Some(("-", *slot)),
            Op::MulLocalConstInPlace(slot, _) => Some(("*", *slot)),
            _ => None,
        };
        let pushes_a_read = |op: &Op| {
            matches!(
                op,
                Op::AddLocalConst(..) | Op::SubLocalConst(..) | Op::MulLocalConst(..)
            )
        };
        for (setup, statement, operator) in [
            ("k = 7", "k += 2", "+"),
            ("k = 7", "k = k + 2", "+"),
            ("k = 7.5", "k += 0.5", "+"),
            ("k = 7", "k -= 3", "-"),
            ("k = 7", "k = k - 3", "-"),
            ("k = 7.5", "k -= 1.5", "-"),
            ("k = 7", "k *= 2", "*"),
            ("k = 7", "k = k * 2", "*"),
            ("k = 7", "k *= 1", "*"),
            ("k = 7.5", "k *= 0.5", "*"),
        ] {
            let body = format!("{setup}\n{statement}\nk");
            let ops = fn_ops(&body);
            let fused: Vec<_> = ops.iter().filter_map(in_place).collect();
            assert_eq!(fused, [(operator, 1)], "{body}\n{ops:?}");
            assert!(!ops.iter().any(pushes_a_read), "{body}\n{ops:?}");
        }

        for (body, kept) in [
            ("k = 7\nk += 1\nk", Op::IncrLocal(1)),
            ("k = 7\nk -= 1\nk", Op::DecrLocal(1)),
        ] {
            let ops = fn_ops(body);
            assert!(ops.contains(&kept), "{body}\n{ops:?}");
            assert!(
                !ops.iter().any(|op| in_place(op).is_some()),
                "{body}\n{ops:?}"
            );
        }
        for body in [
            // A string constant appends in place.
            "s = \"a\"\ns += \"b\"\ns",
            // Another local: the read is fused, the store is `SetLocalPop`.
            "k = 7\ny = k + 2\ny",
            // The last statement's value is the return value: no `Pop`.
            "k = 7\nk += 2",
        ] {
            let ops = fn_ops(body);
            assert!(
                !ops.iter().any(|op| in_place(op).is_some()),
                "{body}\n{ops:?}"
            );
        }
        assert!(fn_ops("s = \"a\"\ns += \"b\"\ns")
            .iter()
            .any(|op| matches!(op, Op::AppendLocalConst(1, _))));

        // The general arm's conversions: an Int local with a Float constant,
        // a Float local with an Int one, a string repeated, and a last
        // statement that returns what it stored.
        assert_eq!(
            vm_result(
                "def f\n  k = 7\n  k += 2\n  k -= 3\n  k *= 4\n  x = 1.5\n  x += 0.25\n  x *= 2.0\n  x -= 0.25\n  y = 2\n  y += 0.5\n  z = 0.5\n  z *= 3\n  s = \"ab\"\n  s *= 2\n  [k, x, y, z, s]\nend\nlet result = f()"
            ),
            "[24, 3.25, 2.5, 1.5, abab]"
        );
        assert_eq!(
            vm_result("def f\n  k = 7\n  k += 2\nend\nlet result = f()"),
            "9"
        );
    }

    /// `k % 7` reads the local and the constant in one op, as `k / 7` does:
    /// it was `GetLocal, Constant, Modulo`, three dispatches with the last
    /// one outside the fast tier. A string or `nil` constant keeps the plain
    /// `Modulo`, whose general arm owns those rules.
    #[test]
    fn modulo_by_a_constant_is_one_op() {
        // `k` is hoisted into slot 1; slot 0 is the callee. `y` and `z` are
        // declared first, so the results are stores rather than declarations.
        let ops = fn_ops("k = 23\ny = 0\nz = 0\ny = k % 7\nz = k % 2.5\n[y, z]");
        assert_eq!(
            ops.iter()
                .filter(|op| matches!(op, Op::ModLocalConst(1, _)))
                .count(),
            2,
            "{ops:?}"
        );
        assert!(!ops.contains(&Op::Modulo), "{ops:?}");
        assert!(
            ops.windows(2)
                .any(|pair| matches!(pair, [Op::ModLocalConst(1, _), Op::SetLocalPop(_)])),
            "{ops:?}"
        );
        let string = fn_ops("k = 23\ny = k % \"x\"\ny");
        assert!(string.contains(&Op::Modulo), "{string:?}");
        assert_eq!(
            vm_result("def f\n  k = 23\n  [k % 7, -k % 7, k % 2.5]\nend\nlet result = f()"),
            "[2, -2, 0.5]"
        );
    }

    /// `==` and `!=` against a constant or between two locals compile to one
    /// fused op, as `<` does: `role == "admin"` was `GetLocal, Constant,
    /// Equal`, and its fused form, which compares a string in place, was
    /// never emitted. `x == 0` takes the same op. A condition whose left side
    /// is not a local is `TestEqualJump`, and every literal arm of a `match`
    /// compares the subject's slot with `EqualLocalConst`, the arms after the
    /// first included (the peephole skips them: each is a branch target).
    #[test]
    fn equality_compiles_to_fused_ops() {
        // `s` and `t` are hoisted into slots 1 and 2; slot 0 is the callee.
        let ops = fn_ops(
            "s = \"user\"\nt = \"x\"\na = s == \"admin\"\nb = s != \"admin\"\nc = s == t\nd = s != t\ne = s == 0\nk = s != 0\n[a, b, c, d, e, k]",
        );
        let fused_ops: [fn(&Op) -> bool; 4] = [
            |op| matches!(op, Op::EqualLocalConst(1, _)),
            |op| matches!(op, Op::NotEqualLocalConst(1, _)),
            |op| matches!(op, Op::EqualLocalLocal(1, 2)),
            |op| matches!(op, Op::NotEqualLocalLocal(1, 2)),
        ];
        for fused in fused_ops {
            assert!(ops.iter().any(fused), "{ops:?}");
        }
        assert_eq!(
            ops.iter()
                .filter(|op| matches!(op, Op::EqualLocalConst(..) | Op::NotEqualLocalConst(..)))
                .count(),
            4,
            "{ops:?}"
        );
        assert!(
            !ops.iter().any(|op| matches!(
                op,
                Op::Equal | Op::NotEqual | Op::IsZeroLocal(_) | Op::NotZeroLocal(_)
            )),
            "{ops:?}"
        );

        let condition = fn_ops("h = {\"k\": \"z\"}\nn = 0\nif h.k == \"z\" { n = 1 }\nn");
        assert!(
            condition
                .iter()
                .any(|op| matches!(op, Op::TestEqualJump(_))),
            "{condition:?}"
        );
        assert!(!condition.contains(&Op::Equal), "{condition:?}");

        let arms = fn_ops(
            "s = \"c\"\nmatch s {\n  \"a\" => 1,\n  \"b\" => 2,\n  nil => 3,\n  true => 4,\n  _ => 0\n}\ns",
        );
        assert_eq!(
            arms.iter()
                .filter(|op| matches!(op, Op::EqualLocalConst(..) | Op::TestEqualJump(_)))
                .count(),
            4,
            "{arms:?}"
        );
        assert!(!arms.contains(&Op::Equal), "{arms:?}");

        let program = "def g(s)\n  match s {\n    \"a\" => 1,\n    \"b\" => 2,\n    nil => 3,\n    true => 4,\n    1.5 => 5,\n    0 => 6,\n    _ => 0\n  }\nend\n\
                       def f(s, t)\n  [s == \"admin\", s != \"admin\", s == t, s != t, s == nil, s == 0, s != 0]\nend\n\
                       let result = [[g(\"a\"), g(\"b\"), g(nil), g(true), g(1.5), g(\"z\"), g(1), g(0), g(false)], \
                       f(\"admin\", \"admin\"), f(nil, 0), f(0.0, 0), f(:admin, \"admin\")]";
        assert_eq!(
            vm_result(program),
            "[[1, 2, 3, 4, 5, 0, 0, 6, 0], \
             [true, false, true, false, false, false, true], \
             [false, true, false, true, true, false, true], \
             [false, true, true, false, false, true, false], \
             [false, true, false, true, false, false, true]]"
        );
    }

    /// A value that branches merges on the `SetLocal`: `||`, `&&`, `??`, a
    /// ternary, a `match`, `rescue` and `||=` all jump to the instruction
    /// after the value. The store is still fused, because the fused op keeps
    /// the `SetLocal`'s index and the branch lands on it.
    #[test]
    fn a_store_a_branch_lands_on_still_fuses() {
        let cases = [
            "a = nil\nb = 2\nx = a || b\nx",
            "a = 1\nb = 2\nx = a && b\nx",
            "a = nil\nx = a ?? 3\nx",
            "c = true\nx = c ? 1 : 2\nx",
            "v = 2\nx = match v {\n  1 => 10,\n  n => n * 100\n}\nx",
            "x = boom() rescue 0\nx",
            "x = 0\nx ||= 5\nx",
        ];
        for body in cases {
            let ops = fn_ops(body);
            assert_eq!(
                set_local_pops(&ops),
                0,
                "{body}\na SetLocal, Pop is left: {ops:?}"
            );
        }

        let program = "def boom\n  throw \"boom\"\nend\n\ndef f(a, b, c, v)\n  x = a || b\n  y = a && b\n  z = a ?? 3\n  t = c ? 1 : 2\n  m = match v {\n    1 => 10,\n    n => n * 100\n  }\n  r = boom() rescue 30\n  n0 = 0\n  n0 ||= 5\n  [x, y, z, t, m, r, n0]\nend\nlet result = [f(nil, 2, true, 1), f(4, 2, false, 7)]";
        assert_eq!(
            vm_result(program),
            "[[2, null, 3, 1, 10, 30, 5], [4, 2, 4, 2, 700, 30, 5]]"
        );
    }

    /// `ok && (x = 1)` as a statement: the short-circuit jumps straight to the
    /// statement's `Pop` with `ok` on the stack, so the `x = 1` store keeps
    /// that `Pop` as an op of its own. Fused, the jump would land past it and
    /// leave `ok` behind, and the `let` after it would read `ok` as `y`. The
    /// same holds for `||` and for a ternary with a store in each branch.
    #[test]
    fn a_store_whose_pop_a_branch_lands_on_keeps_it() {
        for body in [
            "ok = false\nx = 0\nok && (x = 1)\nlet y = 5\n[x, y]",
            "ok = true\nx = 0\nok || (x = 2)\nlet y = 5\n[x, y]",
            "c = true\nx = 0\nc ? (x = 1) : (x = 2)\nlet y = 5\n[x, y]",
        ] {
            let ops = fn_ops(body);
            assert_eq!(
                set_local_pops(&ops),
                1,
                "{body}\nthe store before the branch target keeps its Pop: {ops:?}"
            );
        }

        let cases = [
            (
                "def f(ok)\n  x = 0\n  ok && (x = 1)\n  let y = 5\n  [x, y]\nend\nlet result = [f(false), f(true)]",
                "[[0, 5], [1, 5]]",
            ),
            (
                "def f(ok)\n  x = 0\n  ok || (x = 2)\n  let y = 5\n  [x, y]\nend\nlet result = [f(false), f(true)]",
                "[[2, 5], [0, 5]]",
            ),
            (
                "def f(c)\n  x = 0\n  c ? (x = 1) : (x = 2)\n  let y = 5\n  [x, y]\nend\nlet result = [f(true), f(false)]",
                "[[1, 5], [2, 5]]",
            ),
        ];
        for (program, expected) in cases {
            assert_eq!(vm_result(program), expected, "{program}");
        }
    }

    /// The rule on two hand-built chunks that differ only in where one branch
    /// lands. On the `SetLocal` the pair is fused, and the branch is remapped
    /// onto the `SetLocalPop`; on the `Pop` both ops stay.
    #[test]
    fn a_branch_may_land_on_the_store_but_not_on_its_pop() {
        let optimized = |offset: u16| {
            let mut chunk = Chunk::default();
            // The branch at 2 lands on 3 + offset: 4 is the `SetLocal`, 5 the `Pop`.
            for op in [
                Op::Null,
                Op::True,
                Op::JumpIfFalse(offset),
                Op::Null,
                Op::SetLocal(1),
                Op::Pop,
                Op::Null,
                Op::Return,
            ] {
                chunk.emit(op, 1);
            }
            peephole_optimize_chunk(&mut chunk);
            chunk.code
        };

        assert_eq!(
            optimized(1),
            [
                Op::Null,
                Op::True,
                Op::JumpIfFalse(1),
                Op::Null,
                Op::SetLocalPop(1),
                Op::Null,
                Op::Return,
            ]
        );
        assert_eq!(
            optimized(2),
            [
                Op::Null,
                Op::True,
                Op::JumpIfFalse(2),
                Op::Null,
                Op::SetLocal(1),
                Op::Pop,
                Op::Null,
                Op::Return,
            ]
        );
    }

    /// The ops of the first function `src` defines.
    fn first_fn_ops(src: &str) -> Vec<Op> {
        let tokens = Scanner::new(src).scan_tokens().expect("lex");
        let program = Parser::new(tokens).parse().expect("parse");
        let m = Compiler::compile(&program).expect("compile");
        for c in m.main.chunk.constants.iter() {
            if let Constant::Function(proto) = c {
                return proto.chunk.code.clone();
            }
        }
        panic!("no function proto found");
    }

    /// A pattern whose first instruction a branch lands on fuses like any
    /// other (C587). The scan used to skip such a head, so every `while`
    /// condition, the statement after an `if`/`elsif`/`else`/loop/guard and
    /// the first statement of a function with a defaulted parameter ran
    /// unfused. Slot 0 is the callee, so the first local is slot 1.
    #[test]
    fn a_pattern_a_branch_lands_on_still_fuses() {
        // (body, the fused op expected, how many of it)
        type Case = (&'static str, fn(&Op) -> bool, usize);
        let cases: [Case; 7] = [
            // The `Loop` target: the header compare.
            ("i = 0\nn = 5\nwhile i < n { i += 1 }\ni", |op| {
                matches!(op, Op::LessLocalLocal(1, 2))
            }, 1),
            ("i = 0\nwhile i < 10 { i += 1 }\ni", |op| {
                matches!(op, Op::LessLocalConst(1, _))
            }, 1),
            // The `if`'s merge.
            ("c = true\ni = 0\nif c { c = false }\ni += 1\ni", |op| {
                matches!(op, Op::IncrLocal(2))
            }, 1),
            // A postfix guard's merge: both counters.
            ("n = 0\ni = 0\nn += 1 if i > 3\ni += 1\ni", |op| {
                matches!(op, Op::IncrLocal(_))
            }, 2),
            // The `elsif` condition, and the `else` head.
            (
                "k = 2\nr = 0\nif k < 0 { r = 1 } elsif k < 3 { r = 2 } else { r = r + k }\nr",
                |op| matches!(op, Op::LessLocalConst(1, _)),
                2,
            ),
            (
                "k = 2\nr = 0\nif k < 0 { r = 1 } elsif k < 3 { r = 2 } else { r = r + k }\nr",
                |op| matches!(op, Op::AddLocalsInPlace(2, 1)),
                1,
            ),
            // The loop's exit, and a property read after an `if`.
            (
                "s = 0\ni = 0\nh = {\"a\": 1}\nwhile i < 3 { i += 1 }\ns = s + i\nif s { s = 0 }\ns = h.a\ns",
                |op| matches!(op, Op::AddLocalsInPlace(1, 2) | Op::GetLocalProperty(3, ..)),
                2,
            ),
        ];
        for (body, fused, count) in cases {
            let ops = fn_ops(body);
            assert_eq!(
                ops.iter().filter(|op| fused(op)).count(),
                count,
                "{body}\n{ops:?}"
            );
            assert!(
                !ops.iter()
                    .any(|op| matches!(op, Op::TestLessJump(_) | Op::Add | Op::Less)),
                "{body}\nan unfused head is left: {ops:?}"
            );
        }

        // The `JumpIfParamSupplied` target: the body's first statement.
        let ops = first_fn_ops("def f(a, b = 1) {\n  a += 1\n  a + b\n}\n");
        assert!(
            ops.iter().any(|op| matches!(op, Op::IncrLocal(1))),
            "{ops:?}"
        );

        // The same shapes run, through each target, with the right values.
        let program = "def count(n)\n  i = 0\n  hits = 0\n  while i < n\n    hits += 1 if i % 3 == 0\n    i += 1\n  end\n  hits\nend\n\
                       def grade(k)\n  r = 0\n  if k < 0\n    r = 1\n  elsif k < 3\n    r = 2\n  else\n    r = r + k\n  end\n  r\nend\n\
                       def step_from(a, b = 10)\n  a += 1\n  a + b\nend\n\
                       def after_loop(n)\n  s = 0\n  i = 0\n  while i < n\n    i += 1\n  end\n  s = s + i\n  s\nend\n\
                       let result = [count(10), grade(-1), grade(1), grade(7), step_from(1), step_from(1, 2), after_loop(4)]";
        assert_eq!(vm_result(program), "[4, 1, 2, 7, 12, 4, 4]");
    }

    /// The canonical counting loop, op for op: the header is the fused
    /// compare and its branch, and the back-edge lands on the compare.
    #[test]
    fn a_counting_while_loop_is_four_ops() {
        let ops = fn_ops("i = 0\nn = 5\nwhile i < n { i += 1 }\ni");
        let head = ops
            .windows(4)
            .position(|w| {
                matches!(
                    w,
                    [
                        Op::LessLocalLocal(1, 2),
                        Op::JumpIfFalse(_),
                        Op::IncrLocal(1),
                        Op::Loop(_)
                    ]
                )
            })
            .unwrap_or_else(|| panic!("no fused loop: {ops:?}"));
        let Op::Loop(back) = ops[head + 3] else {
            unreachable!()
        };
        assert_eq!(head + 4 - back as usize, head, "{ops:?}");
        let Op::JumpIfFalse(exit) = ops[head + 1] else {
            unreachable!()
        };
        assert_eq!(head + 2 + exit as usize, head + 4, "{ops:?}");
    }

    /// The tail check is what guards a fusion. Two hand-built chunks differ
    /// only in where one branch lands: on the head of `x = x + 1` the five
    /// ops become `IncrLocal` and the branch is remapped onto it; on the
    /// `Add` they stay (only the trailing store fuses).
    #[test]
    fn a_branch_into_the_tail_still_blocks_the_fusion() {
        let optimized = |offset: u16| {
            let mut chunk = Chunk::default();
            let one = chunk.add_constant(Constant::Int(1));
            // The branch at 1 lands on 2 + offset: 4 is the `GetLocal`, 6 the `Add`.
            for op in [
                Op::True,
                Op::JumpIfFalse(offset),
                Op::Null,
                Op::Pop,
                Op::GetLocal(1),
                Op::Constant(one),
                Op::Add,
                Op::SetLocal(1),
                Op::Pop,
                Op::Null,
                Op::Return,
            ] {
                chunk.emit(op, 1);
            }
            peephole_optimize_chunk(&mut chunk);
            chunk.code
        };

        assert_eq!(
            optimized(2),
            [
                Op::True,
                Op::JumpIfFalse(2),
                Op::Null,
                Op::Pop,
                Op::IncrLocal(1),
                Op::Null,
                Op::Return,
            ]
        );
        assert_eq!(
            optimized(4),
            [
                Op::True,
                Op::JumpIfFalse(4),
                Op::Null,
                Op::Pop,
                Op::GetLocal(1),
                Op::Constant(0),
                Op::Add,
                Op::SetLocalPop(1),
                Op::Null,
                Op::Return,
            ]
        );
    }

    /// `h["k"] = v` and `h.set("k", v)` leave `v` on the stack, so a
    /// comprehension or a binding `match` later in the same expression is not
    /// at the locals baseline and must take the clean-frame lambda. The
    /// constant-key set opcodes were counted at -1, the gate saw a clean
    /// stack, and the comprehension built its result in the slot holding `v`.
    #[test]
    fn index_assignment_mid_expression_keeps_the_next_slot() {
        let cases = [
            (
                "def f()\n  h = {}\n  xs = [1, 2]\n  [h[\"k\"] = 9, [x * 10 for x in xs]]\nend\nlet result = f()",
                "[9, [10, 20]]",
            ),
            (
                "def f()\n  h = {}\n  xs = [1, 2]\n  [h.set(\"k\", 8), [x + 1 for x in xs]]\nend\nlet result = f()",
                "[8, [2, 3]]",
            ),
            (
                "def f()\n  h = {}\n  xs = [1, 2]\n  [h[\"k\"] = 7, {x: x * 2 for x in xs}]\nend\nlet result = f()",
                "[7, {x => 4}]",
            ),
            (
                "def f()\n  h = {}\n  xs = [1, 2]\n  [h[\"k\"] = 1, match xs { [p, q] => p + q, _ => 0 }]\nend\nlet result = f()",
                "[1, 3]",
            ),
            // A global hash (`HashSetGlobalConst`), in a function and at the
            // top level.
            (
                "let g = {}\ndef f()\n  xs = [1, 2]\n  [g[\"k\"] = 6, [x * 3 for x in xs]]\nend\nlet result = f()",
                "[6, [3, 6]]",
            ),
            (
                "let g = {}\nlet result = [g[\"t\"] = 5, [x * 2 for x in [3, 4]]]",
                "[5, [6, 8]]",
            ),
        ];
        for (program, expected) in cases {
            assert_eq!(vm_result(program), expected, "{program}");
        }
    }
}

#[cfg(test)]
mod stack_effect_tests {
    use super::*;
    use crate::interpreter::value::{HashKey, HashPairs, NativeFunction, Value};

    // The constant pool every case shares (`constants`).
    const INT: u16 = 0; // 7
    const ZERO: u16 = 1; // 0
    const STR: u16 = 2; // "s"
    const KEY: u16 = 3; // "k"
    const HASH_GLOBAL: u16 = 4; // "g": the global {"k": {"k": 7}}
    const STR_GLOBAL: u16 = 5; // "t": the global "s"
    const FN_GLOBAL: u16 = 6; // "f": a global native function
    const LENGTH: u16 = 7; // "length"
    const FUNCTION: u16 = 8; // a function that captures nothing
    const KEY_LIST: u16 = 9; // the keys ["k"]
    const JSON: u16 = 10; // "[1]"

    fn constants() -> Vec<Constant> {
        vec![
            Constant::Int(7),
            Constant::Int(0),
            Constant::String("s".into()),
            Constant::String("k".into()),
            Constant::String("g".into()),
            Constant::String("t".into()),
            Constant::String("f".into()),
            Constant::String("length".into()),
            Constant::Function(Arc::new(FunctionProto::new("inner".to_string()))),
            Constant::HashKeys(Arc::new(vec![HashKey::String("k".into())])),
            Constant::String("[1]".into()),
        ]
    }

    // The locals `PRELUDE` leaves, by slot (slot 0 is the running closure).
    const FLAT: u16 = 1; // {"k": 7}
    const N: u16 = 2; // 7
    const KEYS: u16 = 3; // ["k"]
    const IDX: u16 = 4; // 0
    const TEXT: u16 = 5; // "s"
    const NESTED: u16 = 6; // {"k": {"k": 7}}

    const PRELUDE: &[Op] = &[
        Op::Constant(KEY),
        Op::Constant(INT),
        Op::Hash(1),
        Op::Constant(INT),
        Op::Constant(KEY),
        Op::Array(1),
        Op::Constant(ZERO),
        Op::Constant(STR),
        Op::Constant(KEY),
        Op::Constant(KEY),
        Op::Constant(INT),
        Op::Hash(1),
        Op::Hash(1),
    ];

    fn hash_of(key: &str, value: Value) -> Value {
        let mut map = HashPairs::default();
        map.insert(HashKey::String(key.into()), value);
        Value::Hash(Rc::new(RefCell::new(map)))
    }

    /// The stack height once `code` has run on a fresh VM. A chunk with no
    /// `Return` stops where its code ends and leaves the stack as it is.
    fn height_after(code: &[Op]) -> usize {
        let mut proto = FunctionProto::new(String::new());
        proto.chunk.constants = constants();
        for op in code {
            proto.chunk.emit(*op, 1);
        }
        let mut vm = crate::vm::Vm::new();
        vm.globals
            .insert("g".to_string(), hash_of("k", hash_of("k", Value::Int(7))));
        vm.globals
            .insert("t".to_string(), Value::String("s".into()));
        vm.globals.insert(
            "f".to_string(),
            Value::NativeFunction(NativeFunction::new("f", None, |_| Ok(Value::Null))),
        );
        if let Err(err) = vm.execute(&Arc::new(proto)) {
            panic!("{code:?} raised: {err}");
        }
        vm.stack.len()
    }

    /// `stack_effect` is the compiler's model of each handler, and the
    /// comprehension / binding-`match` gate trusts it never to under-count.
    /// This measures the handlers instead of restating them: each case runs
    /// `PRELUDE` + its setup with and without the opcode, and the difference
    /// in stack height is what the opcode really does. Branches take their
    /// fall-through path (offset 0), `ForIter` its continue path.
    ///
    /// `HashSetLocalConst` and `HashSetGlobalConst` were listed at -1, though
    /// both push the value back: after `h["k"] = v` mid-expression the gate
    /// saw a clean stack and a comprehension took an occupied slot.
    ///
    /// Not covered, because they need a callee frame, upvalues, class or
    /// module machinery or a thrown exception, set process-wide flags, or end
    /// the frame: `Return`, `Loop`, `Throw`, `Rethrow`, `GetUpvalue`,
    /// `SetUpvalue`, `AppendUpvalue`, `AddAssignUpvalue`, `New`, `NewNamed`,
    /// `CallNamed`, `CallMethodById`, `CallSuperInit`, `CallSuperMethod`,
    /// `Module`, `Inherit`, `Include`, `Extend`, `Method`, `StaticMethod`,
    /// `Field`, `StaticField`, `ConstField`, `StaticConstField`, `MarkPrivate`,
    /// `MarkProtected`, `EnumPayload`, `TemplateComponent`, `TemplatePartial`,
    /// `TemplateYield`, `TemplateYieldNamed` and `TemplateContentFor`.
    #[test]
    fn stack_effect_matches_what_each_handler_does() {
        use Op::*;
        let two_ints: &[Op] = &[Constant(INT), Constant(INT)];
        let cases: &[(&[Op], Op)] = &[
            // Literals and the stack.
            (&[], Constant(INT)),
            // The fast tier declines a string constant; the general arm runs.
            (&[], Constant(STR)),
            (&[], Null),
            (&[], True),
            (&[], False),
            (&[], Symbol(KEY)),
            (&[Constant(INT)], Pop),
            (&[Constant(INT)], Dup),
            (&[Constant(INT)], DupN(2)),
            (&[Constant(INT)], PopNull),
            (&[], Nop),
            (&[], CoverLine(1)),
            (&[], Import(KEY)),
            // Locals and globals.
            (&[], GetLocal(N)),
            (&[], GetLocal2(N, IDX)),
            (&[Constant(INT)], SetLocal(N)),
            (&[Constant(INT)], SetLocalPop(N)),
            (&[], GetAndNullLocal(N)),
            (&[], GetThis),
            (&[], GetGlobal(HASH_GLOBAL)),
            (&[], GetGlobalNullCheck(HASH_GLOBAL)),
            (&[Constant(INT)], SetGlobal(STR_GLOBAL)),
            (&[Constant(INT)], DefineGlobal(STR_GLOBAL)),
            (&[Null], SelfOrGlobal(HASH_GLOBAL)),
            (&[Null], SelfOrGlobalCallee(HASH_GLOBAL)),
            (&[Null], GetSuper(KEY)),
            (&[Constant(INT)], CloseUpvalue),
            // Arithmetic and comparison.
            (two_ints, Add),
            (&[Constant(STR), Constant(STR)], Add),
            (two_ints, Subtract),
            (two_ints, Multiply),
            (two_ints, Divide),
            (two_ints, Modulo),
            (two_ints, Equal),
            (&[Constant(STR), Constant(STR)], Equal),
            (two_ints, NotEqual),
            (two_ints, Less),
            (two_ints, LessEqual),
            (two_ints, Greater),
            (two_ints, GreaterEqual),
            (&[Constant(INT)], Negate),
            (&[Constant(INT)], Not),
            (&[Null], IsNull),
            (&[Null], NotNull),
            // Branches.
            (&[], Jump(0)),
            (&[True], JumpIfFalse(0)),
            (&[False], JumpIfTrue(0)),
            (&[True], JumpIfFalseNoPop(0)),
            (&[True], JumpIfTrueNoPop(0)),
            (&[Null], NullishJump(0)),
            (&[Null], JumpIfNull(0)),
            (&[Null], JumpIfNotNull(0)),
            (&[], JumpIfParamSupplied(0, 0)),
            (&[], JumpIfLocalFalsy(N, 0)),
            (&[], JumpIfLocalTruthy(N, 0)),
            (two_ints, TestLessJump(0)),
            (two_ints, TestLessEqualJump(0)),
            (two_ints, TestGreaterJump(0)),
            (two_ints, TestGreaterEqualJump(0)),
            (two_ints, TestNotEqualJump(0)),
            (two_ints, TestEqualJump(0)),
            (&[Constant(STR), Constant(STR)], TestEqualJump(0)),
            (&[], RescueJump(0)),
            (&[Constant(INT)], CatchMatch(KEY, 0)),
            (&[Constant(INT)], MatchType(KEY, 0)),
            (&[], MatchArrayLen(KEYS, 1, 0)),
            (&[], MatchArrayLen(KEYS, 0, 1)),
            (&[], MatchArrayLen(N, 1, 0)),
            (&[], ArrayRestLocal(KEYS, 0)),
            (&[], ArrayRestLocal(KEYS, 5)),
            // Fused reads of locals: each pushes its result.
            (&[], AddLocalLocal(N, N)),
            (&[], SubLocalLocal(N, N)),
            (&[], MulLocalLocal(N, N)),
            (&[], DivLocalLocal(N, N)),
            (&[], ModLocalLocal(N, N)),
            (&[], LessLocalLocal(N, N)),
            (&[], LessEqualLocalLocal(N, N)),
            (&[], GreaterLocalLocal(N, N)),
            (&[], EqualLocalLocal(N, N)),
            (&[], NotEqualLocalLocal(N, N)),
            (&[], LessLocalConst(N, INT)),
            (&[], LessEqualLocalConst(N, INT)),
            (&[], GreaterLocalConst(N, INT)),
            (&[], GreaterEqualLocalConst(N, INT)),
            (&[], EqualLocalConst(N, INT)),
            (&[], NotEqualLocalConst(N, INT)),
            (&[], AddLocalConst(N, INT)),
            (&[], SubLocalConst(N, INT)),
            (&[], MulLocalConst(N, INT)),
            (&[], DivLocalConst(N, INT)),
            (&[], ModLocalConst(N, INT)),
            (&[], AddLocalInt(N, 1)),
            (&[], IsTruthyLocal(N)),
            (&[], IsFalsyLocal(N)),
            (&[], IsZeroLocal(N)),
            (&[], NotZeroLocal(N)),
            (&[], NotLocal(N)),
            (&[], NegateLocal(N)),
            (&[], GetLocalProperty(FLAT, KEY, 0, 0)),
            (&[], GetLocalIndex(KEYS, IDX)),
            (&[], GetNestedIndex(FLAT, KEYS, IDX)),
            // In place: no stack traffic.
            (&[], IncrLocal(N)),
            (&[], DecrLocal(N)),
            (&[], IncrLocalFast(N)),
            (&[], AddLocalsInPlace(N, N)),
            (&[], AppendLocalConst(TEXT, STR)),
            (&[], AddLocalConstInPlace(N, INT)),
            (&[], SubLocalConstInPlace(N, INT)),
            (&[], MulLocalConstInPlace(N, INT)),
            (&[], AppendGlobalConst(STR_GLOBAL, STR)),
            (&[], AddNestedIndex(N, FLAT, KEYS, IDX)),
            (&[], SetNestedIndex(FLAT, KEYS, IDX, N)),
            // `<<` and `+=`, with the variable's value and the operand pushed.
            (&[GetLocal(TEXT), Constant(STR)], AppendLocal(TEXT)),
            (
                &[GetGlobal(STR_GLOBAL), Constant(STR)],
                AppendGlobal(STR_GLOBAL),
            ),
            (&[GetLocal(N), Constant(INT)], AddAssignLocal(N)),
            (
                &[GetGlobal(STR_GLOBAL), Constant(STR)],
                AddAssignGlobal(STR_GLOBAL),
            ),
            // Constant-key hash ops.
            (&[GetLocal(FLAT)], HashGetConst(KEY)),
            (&[GetLocal(FLAT)], HashHasKeyConst(KEY)),
            (&[GetLocal(FLAT)], HashDeleteConst(KEY)),
            (&[GetLocal(NESTED)], HashGetConst2(KEY, KEY)),
            (&[GetLocal(FLAT), Constant(INT)], HashSetConst(KEY)),
            (&[], HashGetLocalConst(FLAT, KEY)),
            (&[], HashHasKeyLocalConst(FLAT, KEY)),
            (&[], HashDeleteLocalConst(FLAT, KEY)),
            (&[], HashGetLocalConst2(NESTED, KEY, KEY)),
            (&[Constant(INT)], HashSetLocalConst(FLAT, KEY)),
            (&[], HashGetGlobalConst(HASH_GLOBAL, KEY)),
            (&[], HashHasKeyGlobalConst(HASH_GLOBAL, KEY)),
            (&[], HashDeleteGlobalConst(HASH_GLOBAL, KEY)),
            (&[], HashGetGlobalConst2(HASH_GLOBAL, KEY, KEY)),
            (&[Constant(INT)], HashSetGlobalConst(HASH_GLOBAL, KEY)),
            // Collections, indexing and properties.
            (&[], Array(0)),
            (two_ints, Array(2)),
            (&[Constant(KEY), Constant(INT)], Hash(1)),
            (&[Constant(INT)], HashWithKeys(KEY_LIST, 1)),
            (&[GetLocal(KEYS), Constant(INT)], ArrayPush),
            (&[Array(0), GetLocal(KEYS)], Spread),
            (&[Constant(ZERO), Constant(INT)], Range),
            (&[GetLocal(KEYS), Constant(ZERO)], GetIndex),
            (&[GetLocal(KEYS), Constant(ZERO), Constant(INT)], SetIndex),
            (&[Constant(STR), Constant(INT)], BuildString(2)),
            (&[Constant(STR)], BuildString(1)),
            (&[GetLocal(FLAT)], GetProperty(KEY, 0, 0)),
            (&[GetLocal(FLAT)], GetPropertyOrNull(KEY)),
            (&[GetLocal(FLAT), Constant(INT)], SetProperty(KEY)),
            // Iterators (the iterator itself lives on `iter_stack`).
            (&[GetLocal(KEYS)], GetIter),
            (&[Constant(ZERO), Constant(INT)], GetIterRange),
            (&[GetLocal(KEYS), GetIter], ForIter(0)),
            (
                &[Constant(ZERO), Constant(INT), GetIterRange],
                ForIterRange(0),
            ),
            (&[GetLocal(KEYS), GetIter], PopIter),
            // Exception handlers live on their own stack too.
            (&[], TryBegin(0, 0)),
            (&[TryBegin(0, 0)], TryEnd),
            (&[TryBegin(0, 0)], PopHandler),
            (&[Constant(INT)], ForceFuture),
            // Calls (through a native function), objects and output.
            (&[GetGlobal(FN_GLOBAL), Constant(INT)], Call(1)),
            (&[Constant(INT)], CallGlobal(FN_GLOBAL, 1)),
            (&[], GetGlobalCall(FN_GLOBAL, 0)),
            (&[GetLocal(KEYS)], CallMethod(LENGTH, 0)),
            (
                &[Null, SelfOrGlobalCallee(FN_GLOBAL), Constant(INT)],
                CallSelfOrGlobal(FN_GLOBAL, 1),
            ),
            (&[], Closure(FUNCTION)),
            (&[], Class(KEY)),
            (&[Constant(INT)], Print(1)),
            (&[Constant(JSON)], JsonParse),
            (&[Constant(INT)], JsonStringify),
            (&[], TemplateWriteConst(STR)),
            (&[Constant(INT)], TemplateWrite(true)),
            (&[GetLocal(KEYS)], TemplateIter),
            (&[], TemplateCaptureStart),
            (&[TemplateCaptureStart, Pop], TemplateCaptureEnd),
        ];

        let mut wrong = Vec::new();
        for (setup, op) in cases {
            let mut before = PRELUDE.to_vec();
            before.extend_from_slice(setup);
            let mut after = before.clone();
            after.push(*op);
            let measured = height_after(&after) as i32 - height_after(&before) as i32;
            if measured != stack_effect(*op) {
                wrong.push(format!(
                    "  {op:?}: stack_effect says {}, the handler does {measured}",
                    stack_effect(*op)
                ));
            }
        }
        assert!(
            wrong.is_empty(),
            "stack_effect disagrees with the VM:\n{}",
            wrong.join("\n")
        );
    }
}
