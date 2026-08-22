# Rust API catalog — core types and methods

This is the **map of types** a junior should memorize. It is not rustdoc for every private helper (the crate has thousands of `fn`s). For a symbol not listed here, `rg "struct Name" src` then read the `impl` block.

Signatures are simplified. See the source for lifetimes and errors.

---

## Crate root (`src/lib.rs`)

| Function | Role |
|---|---|
| `run(source)` | Lex → parse → type-check → tree-walk |
| `run_with_options(source, type_check)` | Same, toggle checker |
| `run_with_path(source, path, type_check)` | + module resolution from `path` |
| `run_file(path, type_check)` | Read file, then `run_with_path` |
| `run_vm` / `run_file_vm` | Same front-end, bytecode VM |
| `type_check_source` | `soli check`; returns warnings or errors |

---

## Lexer

### `Scanner<'a>` (`lexer/scanner.rs`)

| Method | Role |
|---|---|
| `new(source: &str)` | Bind input |
| `scan_tokens(&mut self) -> Result<Vec<Token>, LexerError>` | Whole file |
| `scan_token(&mut self) -> Result<Token, LexerError>` | One token (also used internally) |

### `Token` (`lexer/token.rs`)

| Field / method | Role |
|---|---|
| `kind: TokenKind` | What it is |
| `span: Span` | Where it is |
| `new(kind, span)` | Construct |
| `eof(position, line, column)` | Sentinel |

`TokenKind` — literals, keywords, operators, `SdqlBlock`, `InterpolatedString`, `Eof`. Add syntax here first.

---

## Parser

### `Parser` (`parser/core.rs`)

| Method | Role |
|---|---|
| `new(tokens: Vec<Token>)` | Cursor at 0 |
| `parse(&mut self) -> ParseResult<Program>` | Full program |

Other `impl Parser` blocks live in `declarations.rs`, `statements.rs`, `expressions.rs`. They are the same struct.

---

## AST

### `Expr` / `Stmt`

| Field | Role |
|---|---|
| `kind` | `ExprKind` / `StmtKind` enum |
| `span` | For errors |

### `Argument`

`Positional(Expr)` | `Named(NamedArgument)` | `Block(Expr)`

### `Program`

`statements: Vec<Stmt>`

---

## Values (`interpreter/value.rs`)

### `Value` (enum)

See [Interpreter](/docs/internals/interpreter) for variants.

| Method | Role |
|---|---|
| `method(ValueMethod) -> Value` | Box a bound method |

Equality for `==` is **not** always `PartialEq` on instances — enum instances compare structurally.

### `DecimalValue`

| Method | Role |
|---|---|
| `from_str(s, precision)` | Parse |
| `precision()` | Scale |
| `value()` | `&Decimal` |
| `to_f64()` | Lossy |

### `HashKey`

Hashable hash keys: int, decimal, string, symbol, bool, null.

### `Function`

AST closure: params, body, captured env, name.

### `NativeFunction`

Wraps `NativeFn = Rc<dyn Fn(&[Value]) -> Result<Value, String>>`.

### `Class`

Name, methods, class methods, superclass, static data (ORM flags hang off this).

### `Instance`

Class pointer + fields (hidden-class slots when possible).

---

## Environment (`interpreter/environment.rs`)

| Method | Role |
|---|---|
| `new()` | Empty scope |
| `with_builtins_capacity()` | Pre-sized global |
| `with_enclosing(parent)` | Child scope |
| `with_enclosing_and_data(...)` | Child + hash bindings |
| `define` / `define_or_update` / `define_const` | Bind |
| `get` / `get_local` / `get_const` | Read |
| `assign` / `assign_or_define` | Write |
| `is_const` | `const` guard |
| `contains_local` | This frame only |
| `enclosing()` | Parent |
| `get_all_bindings` / `get_all_variables` | Debug / error pages |
| `reset_for_reuse` / `reset_for_call` | Pool frames on the serve path |

---

## Interpreter (`executor/mod.rs`)

| Method | Role |
|---|---|
| `new()` | Scripts / tests, all builtins |
| `new_for_serve()` | No test DSL |
| `new_for_migrations()` | DDL builtins |
| `with_environment(env)` | Inject env |
| `interpret(&mut self, &Program)` | Run |
| `global_env()` | Globals |
| `get_stack_trace()` | `e.backtrace` |
| `set_source_path` / coverage setters | Tooling |

Evaluation of a single expression is `evaluate` (crate-visible on the executor), not a public `lib` API.

---

## VM

### `Compiler` (`vm/compiler.rs`)

| Method | Role |
|---|---|
| `compile(program)` | Script compile |
| `compile_with_globals(program, names)` | Serve: known globals |
| `compile_method_standalone(...)` | One method |
| `emit` / `emit_jump` / `patch_jump` / `emit_loop` | Bytecode |
| `begin_scope` / `end_scope` | Locals |
| `declare_variable` / `resolve_variable` / `resolve_local` / `resolve_upvalue` | Names |
| `add_constant` / `emit_constant` | Pool |
| `start_function` / `finish_function` | Nested functions |
| `wrap_in_lambda` | Subexpression `match` / comprehensions |
| `begin_loop` / `emit_break` / `emit_continue` / `end_loop` | Loops |

### `Vm` (`vm/vm.rs`)

| Method | Role |
|---|---|
| `new()` | Empty VM |
| `execute(proto)` | Run a function proto |
| `run()` | Dispatch loop |
| `push` / `pop` / `peek` | Stack |
| `close_upvalues` | Capture locals that outlive the frame |

### `CallFrame`

Instruction pointer, stack base, current `FunctionProto`.

### `Op` (`vm/opcode.rs`)

One variant per instruction. The `run` match must stay in sync with the compiler.

---

## Serve

### `server_constants`

| Function | Role |
|---|---|
| `is_production_env()` | `APP_ENV` |
| `check_production_boot(dev_mode)` | Hosts + 32-char secret |
| `resolve_http_workers_from_env()` | Pool size |
| `using_production_worker_default` | Banner |
| `realtime_worker_split` | HTTP vs WS threads |
| `get_mime_type` | Static |
| `generate_etag` | Cache |
| `parse_range_header` / `read_file_range` | HTTP Range |

`PRODUCTION_SESSION_SECRET_MIN_LEN` = 32.

### `finish_response(builder, body)` (`serve/mod.rs`)

Safe `.body()`.

### CSRF

`register_csrf_skip_pattern(pattern)` — from Soli `skip_csrf`.

---

## Database (`src/db/`)

| Type / fn | Role |
|---|---|
| `Adapter` / `parse_adapter` | postgres/mysql/sqlite |
| `init_from_app_path` | Load toml/env |
| `ConnectionRegistry::get/resolve/names` | Multi-DB |
| `ConnectionSpec::is_sql` / `label` | Kind |
| TLS helpers in `tls.rs` | `sslmode` |

---

## Span / errors

### `Span`

`new(start, end, line, column)` — byte offsets + display line/col.

### Errors

`LexerError`, `ParseError`, `RuntimeError`, `SolilangError` — all carry a span when they can.

---

## CLI (`src/cli/` — binary only)

`cli::run()` matches `Command::{Repl, Run, Serve, Test, Lint, Fmt, Check, New, Generate, …}`. Add a subcommand in `args.rs` + `commands/`.

---

## Where rustdoc still wins

`cargo doc --document-private-items --open` generates HTML for everything. Use it when you need a private helper. This page is the **guided** subset.

## Adding a method to this catalog

When you add a **public** type or a method on a type listed here, add a row. Don’t list every `fn` in `model/core.rs` — point at the file instead.
