# Bytecode VM

Production `soli serve` compiles each handler to bytecode and runs it on a stack machine. `--dev` does not.

```
src/vm/
  opcode.rs           # Op enum
  chunk.rs            # bytecode + constants (CompiledModule, FunctionProto)
  compiler.rs         # Compiler + compile()
  compiler_exprs.rs   # expressions
  compiler_stmts.rs   # statements
  compiler_classes.rs
  compiler_patterns.rs
  compiler_hoist.rs   # locals for try/for
  vm.rs               # Vm, CallFrame, run() dispatch loop
  vm_calls.rs / vm_classes.rs / vm_exceptions.rs
  vm_*_methods.rs     # String/Array/Hash/Int/primitive methods
  upvalue.rs          # closures
  method_table.rs
  disassembler.rs
```

## Compile

```rust
impl Compiler {
    pub fn compile(program: &Program) -> CompileResult<CompiledModule>
    pub fn compile_with_globals<I>(program, global_names) -> CompileResult<CompiledModule>
    pub fn compile_method_standalone(...) -> ...
}
```

`compile_with_globals` is what serve uses: the worker already knows global names (`User`, `render`, …), so a bare assignment inside a handler becomes a local, matching the tree-walker.

Soli's optional `let` — `total = 0` with no `let` creates the binding — is compiled by hoisting: `compiler_hoist.rs` collects every name a function body assigns bare, and `hoist_locals` declares each one as a local at the top of the body unless it is a parameter, an enclosing local (it stays an upvalue) or a known global (the assignment updates it). At file scope, `SetGlobal` defines a name that does not exist yet. Up to 2.6.1 this was off unless `SOLI_VM_OPTIONAL_LET=1`: a bare assignment to a new name compiled to `SetGlobal` and raised *Undefined variable* at run time, so nearly every real action was re-run on the interpreter — and one that had already written to the database could not be re-run, and answered 500. It is now on; `SOLI_VM_OPTIONAL_LET=0` restores the old behavior as an escape hatch.

Every controller action runs on the VM, with or without a `(req)` parameter (`call_class_method` in `serve/mod.rs`); a zero-parameter action reads the request through the `req` global the worker publishes. An action that raises on the VM is re-run on the interpreter only while it has not written (`clear_durable_commit`); after a write the error is the response. So a VM gap is a 500 as soon as it follows a write, and the rule is to close gaps rather than rely on the fallback. Where the interpreter already holds the single definition of a behavior, the VM delegates to it instead of copying it:

- **Model instances** — a member the VM would not answer the same way (a relation, a preload, a translated field, an unset column, uploader and HABTM helpers) goes through `Interpreter::instance_member_access` (`Vm::model_instance_member`); a bound helper it returns is called through `call_method` with the VM's globals.
- **Query builders** — members resolve through `query_builder_member_access` and calls through `call_query_builder_method`. Methods taking a block (`each`, `map`, `filter`, …) materialize the rows and run on the VM's own array methods, so a compiled closure is always called by the VM that owns its upvalues. `find_each`/`in_batches` still demote.
- **Methods that exist only as AST** — inherited from a class the VM never compiled — are compiled as methods on first use and cached in their defining class's `vm_methods` (`compile_tree_walking_method`), so `this` stays the receiver.

On constructs the compiler cannot represent, it returns an engine fallback. The server then runs that handler on the interpreter (and `SOLI_FAIL_ON_VM_DEMOTION=1` turns that into a process exit in CI).

### What the compiler tracks

| Type | Role |
|---|---|
| `Local` | name, slot, const?, captured? |
| `FunctionType` | script / function / method / init |
| `LoopContext` | break/continue patch lists |
| `TryFrame` | exception handlers |
| `ClassContext` | compiling inside `class` |
| `VariableAccess` | local / upvalue / global |

Helpers you will call if you add a statement:

- `emit(op, line)`, `emit_jump`, `patch_jump`, `emit_loop`
- `begin_scope` / `end_scope` (pops locals, closes upvalues)
- `declare_variable` / `resolve_variable`
- `wrap_in_lambda` — used so a subexpression `match` or comprehension has a real local slot

## Module / function prototype

`CompiledModule` holds function prototypes and the constant pool. `FunctionProto` is one callable: arity, bytecode, upvalue descriptors, name.

`compiled_cache.rs` (crate root) memoizes compile by source so workers don’t recompile identical files.

## `Op`

`src/vm/opcode.rs` — each instruction is a variant (load constant, add, call, jump, …). Adding an opcode means:

1. Variant on `Op`
2. Emit site in the compiler
3. Arm in `Vm::run_dispatch` (the big `match`)
4. Disassembler string
5. If it is hot and simple, an arm in `Vm::run_fast` too (see below)

`vm.rs::run_dispatch` is a large match by design (dispatch). Don’t split it for style.

### The fast tier (`run_fast`)

Before the big `match`, `run_dispatch` calls `run_fast`, which runs the simple ops with the current frame's `ip`, code, constants and stack base held in locals — `&mut self` otherwise forces a reload of all of them through `self.frames` on every op. It covers locals, upvalues, global reads, constants, jumps, integer/float arithmetic and comparisons, the fused local super-instructions, and the common shape of `Call`/`CallGlobal`/`Return` (a compiled closure given exactly its arguments; a return that closes no upvalue and stays in this `run`), switching its cached state to the new frame.

The rule that keeps the two tiers from disagreeing: **a fast arm either completes its op or hands it back untouched.** Type and overflow checks peek before anything is popped; anything else — a string `+`, an overflow, a native callee, an error to raise — `break`s out with the op, `ip` is written back, and the general arm runs it exactly as if the fast tier did not exist. So a new op needs only a general arm to be correct; a fast arm is an optimisation, and must never produce an error or a result the general arm would not. `tests/differential_engines_test.rs` (`fast_tier_hands_off_to_the_general_arm`) exercises the hand-offs.

### Iterator callbacks (`vm_callback_loop.rs`)

`h.each(fn …)`, `map`, `filter`/`reject`, `any?`/`all?`, `transform_values`/`transform_keys`, `each_value`/`each_key` and array `each`/`map`/`filter` — when the callback is a compiled closure taking exactly the arguments the method passes — do not loop in Rust. `CallMethod` hands them to `try_start_callback_loop`, which leaves a `CallbackLoop` on `iter_stack` and opens the first element's frame flagged `drives_loop`. When that frame returns, `Return` gives the result to the loop, which records it and refills the **same frame's** argument slots for the next element (`rerun_loop_callback`), or pops itself and pushes the method's result. Living on `iter_stack` is what makes a `throw` out of a callback safe: `Return` and exception unwinding already truncate that stack.

A callback whose body is a **template** (`Kernel`) is not called at all: a parameter, `x + - * c`, `x < <= > >= c`, `[a, b + c]`, `[a, b]`, or `acc = acc + x` into a captured variable. The loop evaluates it per element in Rust. The same rule as the fast tier applies — a template computes only what its opcode's general arm answers inline (int with overflow checked, float with float); any other operand hands *that element* to the frame path, which runs the real opcodes. Templates are recognised from the exact bytecode shape (`Kernel::of`), so a peephole change that alters a callback's opcodes can make it stop matching: that is safe (it runs on the frame path), only slower.

Everything else — a native or bound callee, a default parameter, a `[key, value]` pair argument to a template — keeps the native drivers in `vm_hash_methods.rs` / `vm_array_methods.rs`, whose semantics the loop mirrors element for element (live iteration, length fixed at the start, `any?` short-circuit, the same error messages). `tests/differential_engines_test.rs` holds a case for each path.

## `Vm`

```rust
pub struct CallFrame { /* proto, ip, stack_base, … */ }

impl Vm {
    pub fn new() -> Self
    pub fn execute(&mut self, proto: &Arc<FunctionProto>) -> Result<Value, RuntimeError>
    pub fn run(&mut self) -> Result<Value, RuntimeError>
    pub fn push(&mut self, value: Value)
    pub fn pop(&mut self) -> Value
    pub fn peek(&self, distance: usize) -> &Value
    pub fn close_upvalues(&mut self, from_slot: usize)
}
```

`execute` sets up a frame and calls `run`. `run` loops: fetch `Op` at `ip`, execute, increment `ip` (or jump).

Stack slots are `Value`. Locals are slots relative to `CallFrame.stack_base`.

Closures: captured locals become `upvalue`s (`upvalue.rs`, `VmClosure`). `close_upvalues` is the Crafting Interpreters “move to heap when the stack slot dies” step.

## Native methods on the VM

`vm_string_methods.rs`, `vm_array_methods.rs`, `vm_hash_methods.rs`, `vm_int_methods.rs`, `vm_primitive_methods.rs` implement **the same** methods as the interpreter builtins, but they take stack values and must not allocate an `Interpreter` for the happy path.

If interpreter and VM diverge, users see “works in tests, fails in production” (or the reverse). Add both, then a differential test.

## Seeding builtins

`run_vm` in `lib.rs` builds a throwaway `Interpreter::new()`, then copies its globals into the VM so `DateTime`, `HTTP`, `File`, … exist. Serve workers do the same once at boot (`engine_loader`).
