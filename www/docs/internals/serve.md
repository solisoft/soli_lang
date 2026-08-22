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
7. **Jail** — `File` and `Image` rooted at the app directory.
8. **Boot interpreter** — `Interpreter::new_for_serve()`, load models, controllers, routes, middleware. Then that interpreter is **dropped**; workers get their own copies.
9. **Worker pool** — Hyper + tokio; each worker owns an engine (VM in production, interpreter in `--dev`).
10. **Accept loop** — CSRF, static files, router, handler, `finish_response`.

## Dual engine on the server

| Flag | Engine per worker |
|---|---|
| `--dev` | Tree-walk `Interpreter` (hot reload) |
| production | Bytecode `Vm` after `warm_vm_handlers` |

`src/serve/engine_loader.rs` copies builtin globals into the VM. A handler the compiler refuses is demoted to the interpreter (`SOLI_ENGINE_LOG=1` logs it).

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
| `websocket.rs` | WS upgrade + rooms |
| `dev_bar.rs` | `--dev` overlay |
| `cors.rs` | `cors("/api/*", …)` |

## How to add a reserved route

Prefer a dedicated module (`nav.rs`, `camera.rs`) over growing `mod.rs`. Return `Response` through `finish_response`. Don’t skip CSRF unless you have a named reason (`skip_csrf` or framework-path list) and a test.
