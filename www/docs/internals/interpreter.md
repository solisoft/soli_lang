# Interpreter — tree-walking engine

Used by **`--dev`**, **`soli test`**, the **REPL**, and as a **fallback** when the VM refuses a handler.

The type you actually hold is `executor::Interpreter`, re-exported as `interpreter::Interpreter`.

```
src/interpreter/
  value.rs           # Value, Class, Instance, Function, NativeFunction
  environment.rs     # lexical scopes
  executor/          # interpret() + evaluate()
    statements.rs
    expressions.rs
    operators.rs
    calls/           # function, method, native, cascade
    access/          # member, index, qualified
    objects/         # arrays, hashes, classes
  builtins/          # File, HTTP, Model, session, …
  hidden_class.rs    # shape-based instance layouts
  inline_cache.rs    # property/method IC
```

## `Value` — the runtime

Every Soli value is this enum (`src/interpreter/value.rs`). The VM uses the **same** `Value`.

| Variant | Meaning | Sharing |
|---|---|---|
| `Int(i64)` | Integer | copy |
| `Float(f64)` | Float | copy |
| `Decimal(DecimalValue)` | Exact decimal | clone of rust_decimal |
| `String(SoliStr)` | String (`EcoString`) | cheap clone |
| `Symbol(SoliStr)` | `:name` | cheap clone |
| `Bool` / `Null` | | copy |
| `Array(Rc<RefCell<Vec<Value>>>)` | Array | shared, mutable |
| `Hash(Rc<RefCell<HashPairs>>)` | Ordered hash | shared, mutable |
| `Function(Rc<Function>)` | Soli closure (AST) | |
| `NativeFunction` | Rust `fn(&[Value]) -> Result<Value, String>` | |
| `Class(Rc<Class>)` | Class object | |
| `Instance(Rc<RefCell<Instance>>)` | Object | |
| `DateTime(i64)` | ns since epoch, no allocation | |
| `Future` | HTTP etc., auto-resolves | `Arc<Mutex<…>>` |
| `QueryBuilder` | `User.where…` chain | |
| `VmClosure` | Bytecode closure living in a `Value` | |
| `Deferred` | `grouped()` batched read | |
| `Image` / `ImagePlan` | Image pipeline | |
| `Method` | Bound method `arr.map` | |
| `Super` | `super.foo` | |
| `Breakpoint` / `Continue` | Control-flow sentinels | |

**Truthiness:** only `false` and `null` are falsy. `0` and `""` are truthy. Don’t write Rust `if value` without going through the engine’s truth helper.

Hashes use `HashKey` (int, string, symbol, bool, null, decimal) — not arbitrary `Value`s as keys.

### `Class` / `Instance`

- `Class` holds methods, class methods, superclass, field names, ORM metadata attached by builtins.
- `Instance` holds a class pointer + field storage (often a hidden class / shape for fast slots).

### `NativeFunction`

```rust
pub type NativeFn = Rc<dyn Fn(&[Value]) -> Result<Value, String>>;
```

Builtins register these into the global environment. They receive already-evaluated arguments. Returning `Err(String)` becomes a Soli exception.

---

## `Environment`

Lexical scope chain (`Rc<RefCell<Environment>>`).

| Method | Role |
|---|---|
| `new()` / `with_builtins_capacity()` | Global / large global |
| `with_enclosing(parent)` | Nested block / call frame |
| `define(name, value)` | `let` / first assignment |
| `define_const` | `const` |
| `get` / `get_local` | Read, walk enclosing |
| `assign` | `name = value` if already bound |
| `assign_or_define` | Bare assignment (idiomatic Soli) |
| `is_const` | Reject mutation of `const` |
| `reset_for_call` / `reset_for_reuse` | Recycle frames (serve hot path) |

Workers reuse environments to avoid allocating a HashMap per request.

---

## `Interpreter`

```rust
impl Interpreter {
    pub fn new() -> Self                 // full builtins (scripts, tests)
    pub fn new_for_serve() -> Self       // no test DSL
    pub fn new_for_migrations() -> Self  // SQL DDL helpers
    pub fn with_environment(env) -> Self
    pub fn interpret(&mut self, program: &Program) -> RuntimeResult<()>
    pub fn global_env(&self) -> &Rc<RefCell<Environment>>
    pub fn get_stack_trace(&self) -> Vec<String>
    // coverage hooks…
}
```

`interpret` walks `program.statements`. Expressions go through `evaluate` in `executor/expressions.rs`, which `match`es `ExprKind`.

### Call path (`executor/calls/`)

| File | Handles |
|---|---|
| `function.rs` | Soli `Function` calls |
| native / builtin dispatch | `NativeFunction` |
| `cascade.rs` | Rails-style `obj.foo.bar` where missing methods might cascade |
| method lookup | instance + class + mixins |

If a call looks “magic” (`User.find_by_email`), it is either a builtin on `Class` or `method_missing`.

---

## Builtins (`src/interpreter/builtins/`)

This directory **is** the standard library and most of the framework.

| Area | Examples |
|---|---|
| Collections | `collections/array.rs`, `hash.rs` |
| HTTP | `http_class.rs` (SSRF-hardened) |
| ORM | `model/` (huge — CRUD, query, callbacks, associations) |
| Web | `router.rs`, `controller/`, `session.rs`, `cookie_jar.rs`, `permit.rs` |
| Mail / jobs | `mailer.rs`, `jobs.rs` |
| Files | `file.rs` (jailed), Trusted in `system.rs` / file |
| Security | `security_headers.rs`, `rate_limit.rs` |

**How a builtin is registered:** a `register_*` function inserts `NativeFunction`s into the global env or onto a `Class`. Model methods are registered in `model/core.rs` (`register_model_class`) — that function is thousands of lines on purpose; split only when a change is unreviewable.

When adding a method on Array/Hash/String, also add the VM twin (`src/vm/vm_*_methods.rs`) or production will miss it.

---

## Hidden classes and inline caches

`hidden_class.rs` + `inline_cache.rs` make `obj.field` and `obj.method()` faster when many instances share a shape (same field insertion order). You rarely touch these unless you change instance layout.

---

## Evaluating vs executing

- **Statements** (`interpret` / `execute_stmt`) — `if`, `while`, `return`, declarations. Return `RuntimeResult<()>`.
- **Expressions** (`evaluate`) — produce a `Value`.

Assignment is an **expression** (`ExprKind::Assign`) and a statement (expression statement). Bare `name = 1` goes through `assign_or_define`.
