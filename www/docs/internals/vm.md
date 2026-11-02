# Bytecode VM

Production `soli serve` compiles each handler to bytecode and runs it on a stack machine. `--dev` does not. Scripts (`soli script.sl`, `soli -e`, `soli build tool.sl` executables) run on it too, unless they need the tree-walker — see the engine table in [Internals](/docs/internals).

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

Before the big `match`, `run_dispatch` calls `run_fast`, which runs the simple ops with the current frame's `ip`, code, constants and stack base held in locals — `&mut self` otherwise forces a reload of all of them through `self.frames` on every op. It covers locals, upvalues, global reads, constants, jumps, integer/float arithmetic and comparisons, the fused local super-instructions, hash key reads (`row.title`), instance field reads and writes (`@x`, `post.title`, `@x = v`), and the common shape of `Call`/`CallGlobal`/`Return` (a compiled closure given exactly its arguments; a return that closes no upvalue and stays in this `run`), switching its cached state to the new frame.

The rule that keeps the two tiers from disagreeing: **a fast arm either completes its op or hands it back untouched.** Type and overflow checks peek before anything is popped; anything else — a string `+`, an overflow, a native callee, an error to raise — `break`s out with the op, `ip` is written back, and the general arm runs it exactly as if the fast tier did not exist. So a new op needs only a general arm to be correct; a fast arm is an optimisation, and must never produce an error or a result the general arm would not. `tests/differential_engines_test.rs` (`fast_tier_hands_off_to_the_general_arm`) exercises the hand-offs.

The instance field arms (`GetProperty`, `GetLocalProperty`, `SetProperty` — `Vm::fast_field_read` / `fast_field_write`) answer only what the general arms' own field fast paths answer, and only when that answer involves no error: a read needs the field present (a field shadows a method, so a present key decides the read) and not a `grouped {}` placeholder, which the general arm forces; a write overwrites the field or adds it, unless the instance is a constructed value object. Both run the privacy check below before they complete, and a refused access is handed back: the general arm checks it again and raises, so a refusal costs two checks while an access that passes costs one. A miss (a method, or a model member that no stored field answers), a hash write, a class receiver and `GetPropertyOrNull` (`@x ||= v`) stay general. A change to what the general arms' field fast paths answer must be made in these arms too. `field_fast_tier_tests` (`vm.rs`) counts the instance field ops the general arms run, so a field access that falls out of the fast tier shows up as a test failure.

A paren-less zero-argument builtin — `h.length`, `s.blank?`, `a.first`, `n.to_s`, the house style — is answered by the same arms. The compiler (`Compiler::get_property_op`) gives `GetProperty`/`GetLocalProperty` two more operands: the name's `resolve_method_id` and `method_registry::zero_arg_receiver_mask`, the receiver types on which that name is a zero-argument builtin (0 for a field name or a user method, which costs nothing more). The fast arm then calls `method_table::bare_zero_arg`, which answers from the integer tables `CallMethodById` uses for `h.length()`. The general arm's order is kept: a hash key comes first (`{"length": 99}.length` is 99 on the VM), a user method on the type outranks the builtin (`has_user_methods`), and `h.len`, which is no zero-argument builtin of a hash, stays a method. A name with no id (`f.round`, `i.abs`), a type the tables do not cover (symbols, instances, `DateTime`) and a value a table declines (`"zz".hex`, which raises) go to the general arm. The tables and the general arm must agree: `bare_zero_arg_agrees_with_the_general_path` (`method_table.rs`) compares them for every registered zero-argument name on a range of receivers, and `bare_builtin_fast_tier_tests` (`vm.rs`) counts the non-instance property ops the general arms run.

### Iterator callbacks (`vm_callback_loop.rs`)

`h.each(fn …)`, `map`, `filter`/`reject`, `any?`/`all?`, `transform_values`/`transform_keys`, `each_value`/`each_key` and array `each`/`map`/`filter` — when the callback is a compiled closure taking exactly the arguments the method passes — do not loop in Rust. `CallMethod` hands them to `try_start_callback_loop`, which leaves a `CallbackLoop` on `iter_stack` and opens the first element's frame flagged `drives_loop`. When that frame returns, `Return` gives the result to the loop, which records it and refills the **same frame's** argument slots for the next element (`rerun_loop_callback`), or pops itself and pushes the method's result. Living on `iter_stack` is what makes a `throw` out of a callback safe: `Return` and exception unwinding already truncate that stack.

A callback whose body is a **template** (`Kernel`) is not called at all: a parameter, `x + - * c`, `x < <= > >= c`, `[a, b + c]`, `[a, b]`, or `acc = acc + x` into a captured variable. The loop evaluates it per element in Rust. The same rule as the fast tier applies — a template computes only what its opcode's general arm answers inline (int with overflow checked, float with float); any other operand hands *that element* to the frame path, which runs the real opcodes. Templates are recognised from the exact bytecode shape (`Kernel::of`), so a peephole change that alters a callback's opcodes can make it stop matching: that is safe (it runs on the frame path), only slower.

Everything else — a native or bound callee, a default parameter, a `[key, value]` pair argument to a template — keeps the native drivers in `vm_hash_methods.rs` / `vm_array_methods.rs`, whose semantics the loop mirrors element for element (live iteration, length fixed at the start, `any?` short-circuit, the same error messages). `tests/differential_engines_test.rs` holds a case for each path.

### Privacy checks

`private` and `protected` members are enforced at run time by `Vm::check_private_value` (`vm_classes.rs`), which the property, call and index ops call. It costs one relaxed load (`restricted_methods_declared()`) until some class in the process declares a restricted member. After that it still has to be cheap, because every scaffolded controller declares one. So it tries the cheap answers first, and each one is the answer the full rule would give:

1. **The receiver is the running method's `this`.** A member of `self` is never refused. When the running frame is a method frame, its slot 0 *is* the `this` the rule compares with, so `@x = v` or `@helper()` in a method's own body passes on one pointer comparison. `GetLocalProperty(0, …)` in a method frame, which is what `@x` compiles to, does not call the check at all, in either tier.
2. **No class of the receiver's chain restricts the name.** `Class::restricted_method` first asks the class's `restricted_names_memo`: the set of names that some class of the chain declares private or protected. A name outside that set cannot be restricted, so the check returns without walking the chain and without looking at the call frames. This covers every field of a class with no restricted member, from any call depth, and `@x` inside a block when `x` is not restricted.
3. **The name is restricted.** Only now does the check look for `this` past the blocks a method runs. `current_this()` scans the frame stack down to the nearest method frame, so from a plain function it visits every frame. A member of that `this` passes, which is how a block reaches its method's private members.
4. **Otherwise** a protected member passes when the caller's class is or inherits the defining class. Anything else is refused with the usual message and span.

`restricted_method` decides by walking the chain until a class defines the name, so the nearest definition wins: a subclass that redefines a private method without a modifier makes it public again.

The memo is valid for one `RESTRICTED_MEMBERS_EPOCH`. **Code that adds a name to a class's `private_members` or `protected_members` after the class exists must call `note_restricted_methods_declared()`.** Where the flag must not change (a nested `private class`), call `note_restricted_members_changed()` instead. Skip both and every class below keeps a stale memo and stops refusing that name. A cloned `Class` starts with an empty memo. `restricted_member_tests` (`value.rs`) and `privacy_check_tests` (`vm_classes.rs`) count lookups, walks and frame scans, so a lost shortcut shows up as a test failure. The `privacy_*` differential cases pin the verdicts.

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

Errors carry the same `line:column` as the tree-walker's: `Chunk` keeps a `columns` vector parallel to `code` (`emit_at` records the column of the expression or statement being compiled, 0 where unknown). The call-depth error names the function and points at its declaration, worded as the tree-walker words it (`call stack too deep (256 frames) in "down" — unbounded recursion?`).

`Vm::check_return_types` checks a function's or method's declared return type when it returns (`FunctionProto` keeps it), with the tree-walker's message (`function 'f' expected to return Int, got null`). `run_compiled_on_vm` in `lib.rs` turns it on, so scripts get it; `soli serve` leaves it off, as apps have always run.

## Native methods on the VM

`vm_string_methods.rs`, `vm_array_methods.rs`, `vm_hash_methods.rs`, `vm_int_methods.rs`, `vm_primitive_methods.rs` implement **the same** methods as the interpreter builtins, but they take stack values and must not allocate an `Interpreter` for the happy path.

If interpreter and VM diverge, users see “works in tests, fails in production” (or the reverse). Add both, then a differential test.

## Seeding builtins

`run_vm` in `lib.rs` builds a throwaway `Interpreter::new()`, then copies its globals into the VM so `DateTime`, `HTTP`, `File`, … exist. Serve workers do the same once at boot (`engine_loader`).
