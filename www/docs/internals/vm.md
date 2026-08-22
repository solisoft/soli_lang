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
3. Arm in `Vm::run` (the big `match`)
4. Disassembler string

`vm.rs::run` is a large match by design (dispatch). Don’t split it for style.

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
