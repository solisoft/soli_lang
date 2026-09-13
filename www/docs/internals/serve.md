# Serve — HTTP, MVC boot, workers

![Request path](/images/internals/request-path.jpg)

`src/serve/` is the process that makes `soli serve` a web server. It is **not** written in Soli. Soli code (routes, controllers, views) runs **inside** workers that this module starts.

## Boot sequence (`serve_folder_with_options_and_hooks`)

File: `src/serve/mod.rs`. Read it top-to-bottom once; it is the map.

1. **Dev REPL remote check** — `--dev` + `SOLI_DEV_REPL_ALLOW_REMOTE` requires `SOLI_DEV_REPL_SECRET`.
2. **Security headers** — off in `--dev`, on in production.
3. **File vs app** — no `app/controllers` and no `config/routes.sl` → [file mode](#file-mode) (`src/serve/files/`).
4. **`load_env_files`** — `.env` then `.env.{APP_ENV}`.
5. **`check_production_boot(dev_mode)`** — production refuses to start without `SOLI_APP_HOSTS` and a 32+ char `SOLI_SESSION_SECRET`. `--dev` skips this. (`server_constants.rs`)
6. **DB** — `db::init_from_app_path`, `ensure_runtime_ready`.
7. **Jail** — `File` and `Image` rooted at the app directory, on the [tenant](#tenants) rather than in a process-wide `OnceLock`.
8. **Boot interpreter** — `Interpreter::new_for_serve()`, load models, controllers, routes, middleware. Then that interpreter is **dropped**; workers get their own copies.
9. **Worker pool** — Hyper + tokio; each worker owns an engine (VM in production, interpreter in `--dev`).
10. **Accept loop** — CSRF, static files, router, handler, `finish_response`.

## Dual engine on the server

| Flag | Engine per worker |
|---|---|
| `--dev` | Tree-walk `Interpreter` (hot reload) |
| production | Bytecode `Vm` after `warm_vm_handlers` |

`src/serve/engine_loader.rs` copies builtin globals into the VM. A handler the compiler refuses is demoted to the interpreter (`SOLI_ENGINE_LOG=1` logs it).

## Builtins, once per thread

`register_builtins` defines ~500 bindings, ~100 of them classes, and none of it depends on the application. A worker thread used to build that registry three times over and keep all three alive: the interpreter's globals, the template engine's environment (`src/template/core_eval.rs`), and the view helpers' closure (`load_view_helpers`).

`builtins::serve_builtins_root()` now holds one registry per thread, and `child_of_serve_builtins()` hands out a fresh scope enclosing it. Each consumer keeps its own child `Environment` for what it adds on top — template helpers, named routes, the form builder — and resolution falls through. A binding the application defines lands in the child and shadows the shared scope, which is the ordering the separate registries had.

Two things follow. The registry holds `Rc` values, so it is per *thread*, not per process: nothing is shared across workers. And test-only builtins are deliberately absent — they belong to `Interpreter::new`, the `soli run` / `soli test` path, which owns its environment outright. The template engine used to register them unconditionally, so `visit` and `assert_eq` resolved inside a served view.

Don't expect this to move RSS; a registry is a few hundred KB. See [Measuring memory](#measuring-memory).

## Tenants

`src/serve/tenant.rs` is the seam that per-application state moves behind, so a process can eventually serve more than one app.

A `Tenant` owns what belongs to one application; the process holds a registry of them; each thread knows which one it is serving, and worker threads are pinned, so the current tenant is fixed when the thread starts rather than looked up per request. With a single application the registry holds exactly one tenant (`TenantId::PRIMARY`) and every thread falls back to it, so nothing observable changes.

Which state goes where is not a matter of taste:

| Kind | Where | Examples |
|---|---|---|
| `Send + Sync` | the process registry, via the helpers below | app root, jails, template cache, mounted engines |
| `Rc`-based, thread-confined | a `thread_local!` **keyed by `TenantId`** | interpreters, `Rc<Class>` model registries, view helpers, the parsed handler cache |

### Converting a global

Three helpers, so a singleton keeps its shape and changes only what it is. Each addresses the tenant the calling thread is serving, so with one application the map holds a single entry and behaviour is unchanged.

| Was | Becomes | API |
|---|---|---|
| `Mutex<Option<T>>`, `OnceLock<T>` | `TenantCell<T>` | `get` / `set` / `set_once` / `get_or_init` / `clear` |
| `lazy_static! { RwLock<T> }` | `TenantValue<T>` | `read(\|v\| …)` / `write(\|v\| …)` / `reset` — the constructor is part of the declaration, so it replaces `lazy_static!` outright and stays `const` |
| `thread_local! { RefCell<T> }` holding `Rc` values | `TenantLocal<T>` | `with(init, \|v\| …)` / `clear` |

`read`/`write` take a closure rather than returning a guard: the value lives inside a map inside the lock, and stable Rust has no way to hand out a guard borrowed into it (`parking_lot`'s mapped guards would, but it is not a direct dependency). In exchange the lock is always released.

### Done, and left

Converted: the app root (was `live::component::APP_ROOT`); the `File` and `Image` jails (were `OnceLock`s); `VIEWS_DIR`, `PUBLIC_DIR` and `TEMPLATE_CACHE` (`init_templates` was first-caller-wins, so a second application would have rendered the first one's views); `JAR_CACHE`; `MOUNTED_ENGINES`; and `MAILER_CONFIG`.

Three of those are security properties rather than tidiness. A shared jail in a two-application process would let either resolve paths under the other's root. `JAR_CACHE` holds the keys that sign and encrypt cookies, derived from that application's `SOLI_SESSION_SECRET` — shared, either application could mint a cookie the other trusts. `MAILER_CONFIG` holds one app's SMTP credentials and `from` address; shared, a co-hosted app would send through them, and `Mailer.configure` in one would reconfigure the other.

Still process-global, and each one a collision if a second application were added: `MODEL_REGISTRY` and the `COLLECTION_*` maps beside it, `CONTROLLER_REGISTRY`, `SOLIKV_CONFIG`, `TRUSTED_PROXIES`, `RATE_LIMIT_STORE`, `SECURITY_HEADERS_CONFIG`, the mixin hooks, and the `ROUTES` thread-local. `.env` is loaded with `std::env::set_var`, which is process-wide too.

Shareable as-is, because they are read-only or content-addressed: `REGEX_CACHE`, `SYMBOL_TABLE`, `HIDDEN_CLASS_REGISTRY`, `INLINE_CACHE`, `MODULE_CACHE`, and the `&'static [MethodDef]` tables. One caveat: `SYMBOL_TABLE` interns with `Box::leak`, so a process that loads and unloads applications would grow without bound.

## Measuring memory

`scripts/mem-probe.sh`. Four things will otherwise give you a number that is confidently wrong:

* `ps -o rss=` cannot separate a process's own heap from the file-backed pages every `soli` process shares.
* `Pss_Anon` can, but counts only **resident** pages — on a swapping machine a process reads as *smaller* the more pressure the box is under. Add `SwapPss`.
* mimalloc's default purge delay leaves the one-time boot-parse churn mapped. Sample with `MIMALLOC_PURGE_DELAY=0`.
* Transparent huge pages get collapsed and split on khugepaged's schedule, which moved anonymous RSS by tens of MiB between otherwise identical runs. Take a median of several, and read the range.

And compare two binaries by **alternating them in one session** (`--ab OLD NEW <app> [workers]`), never by lining up two sweeps taken minutes apart: machine drift lands entirely on whichever ran later. That mistake is what first made the shared-builtins change look like a 30% win; a proper A/B put it at 0.4 MiB against a ±12 MiB spread.

## Request path (happy path)

1. Hyper accepts TCP, hands the request to a worker channel (bounded; `503` if full).
2. Worker: parse method/path/headers/body (body cap `SOLI_MAX_BODY_SIZE`).
3. **CSRF** (`csrf.rs`) — Origin/Referer gate; optional token (`SOLI_CSRF_TOKENS=require`).
4. Static / reserved `/__soli/` / `/_health` short-circuit.
5. Router match → middleware → Soli controller action.
6. Action returns HTML string, redirect hash, or `{status, body, headers}`.
7. **`finish_response(builder, body)`** — never `.body().unwrap()`; a poisoned builder (bad header) becomes 500 instead of a worker panic.

Panics in the handler are caught (`catch_unwind`) → 500, worker stays up. `panic = "abort"` is a **compile_error** so that net cannot be silently disabled.

## Important types / functions

### `server_constants.rs`

| Item | Role |
|---|---|
| `is_production_env()` | `APP_ENV` is `production` or `prod` |
| `check_production_boot(dev_mode)` | Fail closed on hosts + session secret |
| `resolve_http_workers_from_env()` | `SOLI_WORKERS` / production default 2 / CPU count |
| `realtime_worker_split` | Reserve WS workers without starving HTTP |
| `get_mime_type` / `parse_range_header` / `generate_etag` | Static files |

### CSRF (`csrf.rs`)

| Item | Role |
|---|---|
| `register_csrf_skip_pattern` | `skip_csrf("/webhooks")` from routes |
| `origin_matches_declared_host` | `SOLI_APP_HOSTS` allowlist (not `X-Forwarded-Host`) |
| Jobs dashboard path | Origin gate only; no session token |

### `finish_response`

```rust
pub(crate) fn finish_response(builder: Builder, body: Bytes) -> Response<ResponseBody>
```

Use this for every response you build in `src/serve/`. File-mode already does.

## File mode

`src/serve/files/` — `soli serve ./notes` when the folder is not an MVC app.

- Disk files + MIME + Range + ETag
- `.md` → HTML (`files/markdown.rs`)
- `.slv` / `.erb` via the template engine
- Generated folder indexes

No `.env`, no DB, no controllers. Templates in that folder **are** code — only serve trees you trust.

## Other files you will open

| File | Why |
|---|---|
| `router.rs` | Path matching, `resources`, named routes |
| `middleware.rs` | Global vs scoped |
| `worker_pool.rs` | Channels, 504 timeout |
| `shutdown.rs` | SIGTERM drain; compile_error on panic=abort |
| `env_loader.rs` | dotenv |
| `tenant.rs` | Per-application state; app root and the `File`/`Image` jails |
| `websocket.rs` | WS upgrade + rooms |
| `dev_bar.rs` | `--dev` overlay |
| `cors.rs` | `cors("/api/*", …)` |

## How to add a reserved route

Prefer a dedicated module (`nav.rs`, `camera.rs`) over growing `mod.rs`. Return `Response` through `finish_response`. Don’t skip CSRF unless you have a named reason (`skip_csrf` or framework-path list) and a test.
