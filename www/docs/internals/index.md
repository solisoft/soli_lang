# Internals — how the Rust crate is organized

This section is for **people who will change the Soli interpreter**, not for people writing Soli apps. If you are a junior Rust engineer joining the repo, start here, then follow the sub-pages.

Soli is one Cargo crate (`solilang`) that is both:

1. a **language** (lexer → parser → AST → tree-walking interpreter *or* bytecode VM)
2. a **web framework** (Hyper server, ORM, templates, LiveView, jobs) implemented as Rust builtins that Soli code calls

There is no separate “framework crate”. `soli serve` is the same binary as `soli run`.

## Mental model in one picture

![Crate layout](/images/internals/crate-layout.jpg)

*Layers: CLI → language pipeline → two engines → framework (serve, db, templates). Shared `Value` sits under everything.*

## Where to stand when you open the repo

| You want to… | Start in |
|---|---|
| Change syntax | `src/lexer/`, `src/parser/`, `src/ast/` |
| Change what `1 + 2` does | `src/interpreter/executor/operators.rs` **and** `src/vm/vm.rs` (dual engine) |
| Add a builtin (`File.read`) | `src/interpreter/builtins/` then a VM method table if it is a type method |
| Change HTTP / CSRF / workers | `src/serve/` |
| Change SQL adapters | `src/db/` |
| Change templates | `src/template/` |
| Change `soli new` / generators | `src/scaffold/` |
| Change CLI flags | `src/cli/` (`src/main.rs` only boots) |

## Two binaries, one library

- `src/lib.rs` — the library. All language and framework code lives here (`pub mod …`).
- `src/main.rs` — the `soli` executable. Sets the allocator (`mimalloc`), a SIGTERM handler, SSRF test-runner token, then `cli::run()`.
- `src/cli/` is compiled **only into the binary** (it is `mod cli` in `main.rs`, not `pub mod cli` in `lib.rs`). Library tests cannot call CLI helpers unless they go through public lib APIs.

Entry points on the library:

| Function (`src/lib.rs`) | What it does |
|---|---|
| `run` / `run_with_options` / `run_with_path` | Lex, parse, optional type-check, **tree-walk** |
| `run_vm` / `run_file_vm` | Same front-end, then **bytecode VM** |
| `type_check_source` | `soli check` — no execution |
| `run_migration_source` | Interpreter with SQL DDL builtins |

Production `soli serve` does **not** call `run()` per request. It boots workers that already hold a compiled (or interpreted) copy of the app. See [Serve](/docs/internals/serve).

## Dual engine (the tax you will pay)

| Mode | Engine | Why |
|---|---|---|
| `soli serve --dev`, `soli test`, REPL | Tree-walking `Interpreter` | Readable stack traces, hot reload, no compile step |
| `soli serve` production | Bytecode `Vm` | Throughput |

**Every user-visible language feature must behave the same on both paths.** There is a differential test suite (`tests/differential_engines_test.rs`) that runs programs through both engines. If you add a String method, you typically touch:

1. `src/interpreter/builtins/` (or executor)
2. `src/vm/vm_string_methods.rs` (or the matching `vm_*_methods.rs`)
3. tests on both sides

Skipping the VM copy means the feature works in `--dev` and silently differs in production.

## Ownership patterns you will see everywhere

Soli values are cloned a lot (dynamic language). The crate leans on:

- **`Rc<RefCell<T>>`** — arrays, hashes, instances (single-threaded per worker)
- **`EcoString` (`SoliStr`)** — strings; short strings inline, long strings refcounted
- **`Arc`** — things that cross threads (compiled modules, HTTP futures, job payloads)
- **Thread-per-worker** — each HTTP worker has its **own** `Interpreter` or `Vm`. No shared mutable AST across workers.

If you put a `Rc` in something that must go to another thread, the compiler will stop you. That is usually a sign the value should be `Value` serialized or `Arc`.

## Request vs script

```
soli run file.sl     →  lib::run_with_path  → one Interpreter, exit
soli serve ./app     →  serve::serve_folder → load .env → boot gate →
                       worker pool → Hyper → CSRF → router → Soli action
```

Scripts have no filesystem jail. `soli serve` turns on `File` / `Image` jails under the app root.

## Sub-pages

1. [Pipeline — lexer, parser, AST](/docs/internals/pipeline)
2. [Interpreter — `Value`, environment, executor, builtins](/docs/internals/interpreter)
3. [Bytecode VM — compiler, opcodes, dispatch](/docs/internals/vm)
4. [Serve — HTTP workers, CSRF, boot](/docs/internals/serve)
5. [Database adapters](/docs/internals/database)
6. [Rust API catalog — types and methods](/docs/internals/rust-api)

## How to add something (junior checklist)

1. Find the **existing** similar feature. Copy its shape, don’t invent a third pattern.
2. If it is language semantics, implement **interpreter + VM** or document an `EngineFallback`.
3. Unit test next to the module (`#[cfg(test)]`) plus a `.sl` test in `tests/` when it is user-visible.
4. `cargo fmt` and `cargo clippy --all-targets -- -D warnings`.
5. User-facing behavior: `CHANGELOG.md` + `www/docs/*.md` + matching `.html.slv`.

## Crate module map (`src/`)

| Module | Role |
|---|---|
| `ast` | `Expr`, `Stmt`, `Program` — the tree both engines consume |
| `lexer` | `Scanner` → `Token` / `TokenKind` |
| `parser` | Pratt parser → `Program` |
| `types` | Optional static checker (`soli check`) |
| `module` | `import`, packages, deploy tarball, preview envs |
| `interpreter` | Tree-walk engine + all builtins |
| `vm` | Compiler + stack VM |
| `compiled_cache` | Cache compiled bytecode by source |
| `serve` | HTTP server, MVC load, CSRF, static/file mode |
| `template` | `.html.slv` / ERB renderer |
| `live` | LiveView sockets, morph, uploads |
| `db` | Postgres / MySQL / SQLite adapters + TLS |
| `jobs` | Background job engine |
| `scaffold` | `soli new` / `generate` |
| `lint` / `fmt` | `soli lint` / `soli fmt` |
| `lsp` | Language server |
| `graph` | Code-graph for agents |
| `bundle` / `virtual_fs` | `.soli` bundles and in-memory FS |
| `span` / `error` / `redaction` | Locations, errors, secret scrubbing |
| `platform` | OS process, locks, browser CDP |
| `desktop` / `update` | Packaged apps, OTA |
| `coverage` / `metrics` | Test coverage, Prometheus |
| `inflect` | Inflections for generators |
