# Native kernels

A script's top-level functions that are typed `Int`/`Float`/`Bool` end to end, and only do
arithmetic, comparisons, branches, loops and calls to each other, are compiled to machine code with
Cranelift when the script loads. The user-facing rules are in
[Native Kernels](/docs/language/native-kernels); this page is how the tier is built.

The one idea to keep in mind: **a kernel never reproduces an error, it only notices one.** It
returns a value or *declines*, and a declined call is run by the engine as if there were no kernel.
Kernels are pure, so running the call a second time changes nothing, and every error message, span
and exit code stays the engine's.

```
src/native/
  mod.rs          # public API: enabled(), log_level(), analyze_and_compile(),
                  #   KernelSet, KernelRef::try_call, Decline, KernelId
  eligibility.rs  # which functions qualify; its own sound typer → typed IR
  ir.rs           # the typed IR (KFunc, KStmt, KExpr, Ty)
  lower.rs        # typed IR → Cranelift IR; the semantics table lives here
  jit.rs          # JITModule, trampolines, KernelCtx — all of the tier's unsafe
  tests.rs        # eligibility tests + seeded differential fuzz vs the tree-walker
```

## Pipeline

```
Program ──eligibility::analyze──▶ Vec<KFunc> + refusals
        ──jit::compile (lower.rs per kernel)──▶ JITModule + one trampoline per kernel
        ──▶ KernelSet { kernels, by_decl: FunctionDecl address → index }
```

`analyze_and_compile(program, is_builtin)` (in `mod.rs`) does all three and logs under
`SOLI_NATIVE_LOG`. It returns `None` when the tier is off, nothing qualifies, or the host refuses
(`cranelift_native::builder` fails, or finalizing cannot map executable memory — SELinux
`deny_execmem`, hardened macOS). In every `None` case the program runs as before.

`src/lib.rs` calls it (`script_kernels`) from `execute_program_tree`, `execute_program_vm` and the
`Engine::Auto` arm of `execute_program`, which `soli script.sl`, `--tree`, `--vm`, `soli -e` and
script executables (`soli build tool.sl`) all go through. `is_builtin` asks the fresh interpreter's
environment whether a name is already bound. Scripts compile their bytecode afresh: kernels are
keyed by AST addresses, which `compiled_cache` does not keep (`run_vm` skips the cache when the tier
is on).

## Eligibility (`eligibility.rs`)

This is a separate type checker on purpose. `src/types/` is lenient (`Any`, `Unknown` let a program
through); here a wrong guess is a wrong machine instruction, so anything that cannot be typed
exactly is refused, with a reason for the log.

- **Signature.** Every parameter `Int`/`Float`/`Bool` (case-insensitive, as `value_matches_type`
  reads it), no default, no block parameter, 1 to `MAX_ARGS` (16); return type one of the three.
  An unannotated parameter means "not a candidate" (no log line); a numeric signature with a bad
  detail is a refusal.
- **Names.** `analyze` collects every name the top level can bind (`collect_bound_names`: `let`,
  `const`, classes, imports, `for` variables, `catch` variables, bare assignments outside
  lambdas). A function defined twice, or whose name something else also binds, is refused. A bare
  `x = …` in a body whose `x` is a global or a builtin is refused, because both engines would write
  the global.
- **Scoping mirrors the tree-walker**, the stricter engine: a block opens a scope, a binding made
  inside ends with it, and a bare `x = …` updates the nearest visible `x` or creates one. The VM
  hoists function locals more loosely, so the typer refuses every program where the two readings
  differ — a name is only read after an assignment in the same or an enclosing scope.
- **Fixpoint.** A candidate whose body cannot be typed is out, and so is any candidate that calls
  it. `analyze` repeats until nothing changes; the survivors are numbered in the order the lowering
  defines them, and a `KExpr::Call` holds that index.

## Typed IR (`ir.rs`)

Every name is resolved to a slot and every expression carries its type, so the lowering only picks
instructions. `KStmt::If` keeps `has_else` because a missing `else` as the last statement is a
fall-through (no value), not an empty branch. `KExpr::And`/`Or` carry `Option<Ty>`: `None` when the
operands differ, which is only allowed where the value is read for its truthiness.

## Lowering (`lower.rs`)

Each kernel is `k(ctx: *mut KernelCtx, depth: i64, params…) -> ret`. The semantics are the engines',
operator by operator (`src/interpreter/executor/operators.rs`, `src/vm/vm.rs`):

| Source | Engines | Kernel |
|---|---|---|
| Int `+ - *` | checked, overflow raises | `sadd/ssub/smul_overflow` → decline `Overflow` |
| Int `/ %` | truncating; zero divisor and `i64::MIN / -1` raise | both checked *before* `sdiv`/`srem` (which would trap) → decline |
| unary `-` on Int | `i64::MIN` raises | checked → decline `Overflow` |
| Float `/` | a zero divisor raises (`0.0` and `-0.0`, not NaN) | ordered `fcmp Equal` → decline `DivisionByZero` |
| Float `%` | Rust `f64 %` (NaN for a zero divisor) | call to `soli_native_fmod` |
| Int meets Float | Int converted to f64 | `fcvt_from_sint` first |
| Int comparisons | exact `i64` | `icmp` |
| truthiness | `0` falsy, every Float truthy (`0.0`, NaN) | `truthy()` |
| <code>&amp;&amp;</code> / <code>&#124;&#124;</code> | yield an operand | branch + merge on the operand's value |
| call depth | `MAX_CALL_DEPTH` frames | `depth >= max_depth` → decline `Depth`; each kernel call passes `depth + 1` |
| falls off the end | no value | decline `FellThrough` |

A decline stores `status = 1` and a `Decline` code in `KernelCtx` (cold block), and returns a dummy
value. After every kernel-to-kernel call the caller reloads `ctx.status` and, if set, returns at
once (`propagate_block`), so the decline unwinds to the entry, where `try_call` reads it.

**Inlining.** A `KExpr::Call` to a kernel whose body is at most `INLINE_BUDGET` (48) IR nodes is
emitted by `inline_call` as a copy of the callee's body rather than a machine call (`fib` is 17
nodes; a loop with a few statements about 40). One level only: `inline_ret` is set while a copy is
emitted, and a call met inside it is a real call, so recursion cannot unroll. The copy gets fresh
Cranelift variables for the callee's slots, starts with the depth check the callee would have made
(`depth + 1 >= max_depth` → decline `Depth`), and runs at that depth, so its own calls pass
`depth + 2` as the real call would have. `emit_return` turns a `return` in the copy into a jump to
the continuation block, with the value in a variable; falling off its end declines `FellThrough`.
The bail and propagate blocks return from the *machine* function, so they produce a dummy of
`out_ret` — the outer kernel's return type, which may differ from the inlined callee's. A bail
inside the copy therefore leaves the whole kernel, which is where a real callee's decline would
have propagated to anyway. Nothing observable changes.

What it buys is the call itself. Removing the overflow, depth and status checks altogether changed
`fib(42)` by noise; the prologue and epilogue of each call were the cost, and copying `fib` into
itself took `fib(42)` from 1453 ms to 890 ms.

**Any change to an operator in either engine must be mirrored here.** The guards are below.

## JIT (`jit.rs`)

All of the tier's `unsafe` is in this file: turning a finalized code pointer into an `Entry`, and
handing raw pointers to generated code. `compile` builds one `JITModule` per program
(`opt_level=speed`, the two flags `JITBuilder::with_flags` would set, without its panic on an
unsupported host), declares every kernel first so they can call each other directly, and then one
**trampoline** per kernel:

```rust
type Entry = unsafe extern "C" fn(*mut KernelCtx, i64, *const u64, *mut u64);
//                                ctx              depth args (8-byte slots) out
```

so Rust calls every kernel through one function type whatever its parameters. `KernelCtx` is
`#[repr(C)] { status: u32, reason: u32 }`; `lower::CTX_STATUS`/`CTX_REASON` are its offsets. `Jit`
frees the module on drop; the `KernelSet` owns it through an `Arc`, and every `KernelRef` holds that
`Arc`.

## Calling a kernel: `KernelRef::try_call`

```rust
pub fn try_call(&self, args: &[Value], depth: usize,
                bound: impl Fn(&str) -> Option<KernelId>) -> Option<Value>
```

`None` means declined. In order, it:

1. checks the argument count and each argument's type against the annotation (the engines do not
   check parameter types, so `half(3)` on a `Float` parameter must decline: `Arguments`);
2. runs the **arm check**: every kernel this one can reach (`deps`, the transitive closure of
   `callees`, itself included) must still be bound, by name, to that same kernel. `bound` asks the
   engine what a global name holds right now. A callee not defined yet (`def` statements bind as
   they execute) or rebound since declines with `Rebound` — the machine code calls it directly and
   would otherwise ignore the rebinding;
3. calls the trampoline with the caller's depth, counted the way its engine counts it;
4. converts the result back to a `Value`, or logs the first decline of that kernel and returns `None`.

## Engine hooks

**Tree-walker.**

- `StmtKind::Function` (`executor/statements.rs`): when a top-level `def` binds its `Function`,
  `func.kernel = kernels.for_decl(decl)`. The lookup is by `FunctionDecl` address, so a nested
  `def` or a method of the same name never matches.
- `call_function_with_this` (`executor/mod.rs`): first thing, if the function has a kernel, there
  is no `this` and no coverage tracker, `try_call` with `call_stack.len()` and a `bound` that reads
  the function's closure environment. On `Some(v)` it returns; otherwise the body runs as usual.
  Under coverage it is skipped, or lines a kernel ran would never be counted.

**VM.**

- `compile_function_decl` (`compiler_stmts.rs`): at `scope_depth == 0`, `proto.kernel =
  kernels.for_decl(decl)`. Only `Compiler::compile_with_kernels` installs a `KernelSet`.
- `Vm::try_kernel` (`vm_calls.rs`) runs it on the `argc` values at the top of the stack, with
  `frames.len()` as the depth and a `bound` that reads `vm.globals`; on success it replaces callee
  and arguments with the value without opening a frame. It is tried in `call_closure_in_class`
  (when `class` is `None`) and in the inline `Op::Call` arm of `run`.
- The fast tier (`run_fast`'s inline call paths) skips any closure whose proto has a kernel, so
  those calls fall to the paths above.

## Feature flag

Cargo feature `native`, on by default (and in `full`). Off, `mod.rs` compiles `disabled_impl`:
`KernelSet`/`KernelRef` are uninhabited stub types (`Infallible`), Cranelift is not a dependency,
and `enabled()` is `false`. The hooks compile unchanged in both builds. At run time `SOLI_NATIVE=0`
(`off`/`false`/`no`) or `--no-native` (`native::set_enabled(false)`) turns the tier off.

## Tests

- **`src/native/tests.rs`** — eligibility (what qualifies, the refusal reasons, global names,
  branch-scoped variables, names bound twice), declines (overflow, division, depth, unbound
  callee), and `generated_code_agrees_with_the_tree_walker`: a seeded generator writes random typed
  functions, and each is run as a kernel and on the tree-walker; values and declines-versus-errors
  must agree. Run it after touching `lower.rs` or an operator.
- **`tests/native_kernels_test.rs`** — each script in `tests/fixtures/_native/` runs on both
  engines, with kernels and with `SOLI_NATIVE=0`, and each engine's pair must match exactly
  (stdout, stderr, exit code). The two engines are not compared with each other: they already
  differ in places, and a kernel must reproduce whichever engine called it. A comparison passes
  trivially when nothing compiled, so every fixture starts with a `# kernels: a, b, c` line and the
  test checks that `SOLI_NATIVE_LOG=1` reports exactly those. A new fixture must have one.

## Not done yet

- `soli serve`, `soli test`, the REPL and coverage runs do not compile kernels.
- Methods, nested functions and lambdas are never kernels.
- Script executables carry the serialized AST and compile kernels at start-up on the target
  machine; there is no ahead-of-time object file.
