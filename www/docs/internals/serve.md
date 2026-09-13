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

Converted: the app root (was `live::component::APP_ROOT`); the `File` and `Image` jails (were `OnceLock`s); `VIEWS_DIR`, `PUBLIC_DIR` and `TEMPLATE_CACHE` (`init_templates` was first-caller-wins, so a second application would have rendered the first one's views); `JAR_CACHE`; `MOUNTED_ENGINES`; `MAILER_CONFIG`; `TRUSTED_PROXIES`; `RATE_LIMIT_STORE`; `SOLIKV_CONFIG` with its `RESP_POOL`; `CONTROLLER_REGISTRY`; `SECURITY_HEADERS_CONFIG` with `SECURITY_HEADERS_ENABLED`; `MODEL_REGISTRY` with the four `COLLECTION_*` maps beside it; and the whole database layer — `db::registry`, `db::config`, `CACHED_DB_CONFIG`, `DB_CONFIG` and the JWT state.

The database layer went with them, and it is the one that mattered most. `db::registry` (which connections exist and which is default), `db::config` (the default adapter and URL), `CACHED_DB_CONFIG` (SoliDB cursor URL, database name, API key, basic auth) and the JWT state were all process-global — and the first three were `OnceLock`s, so the *first* application to boot would have frozen them for every application after it. Not with an error: silently, each later application reading and writing the first one's data with the first one's credentials. Nothing else on this page comes close.

The JWT state is now one per-application struct rather than three globals, because its three parts move together: a token belongs to one application's credentials, and a backoff recorded after *its* login failed must not make another application skip its own login.

Several of the others are security properties rather than tidiness:

| Global | What sharing it would mean |
|---|---|
| `FILE_JAIL` / `IMAGE_JAIL` | either application resolves paths under the other's root |
| `JAR_CACHE` | it holds the keys that sign and encrypt cookies, derived from that app's `SOLI_SESSION_SECRET` — either app could mint a cookie the other trusts |
| `MAILER_CONFIG` | a co-hosted app sends through the other's SMTP credentials and `from` address |
| `TRUSTED_PROXIES` | an app behind a different proxy, or behind none, inherits the list and honours `X-Forwarded-*` from a client that reached it directly |
| `SECURITY_HEADERS_*` | `set_csp(...)` in one rewrites the other's policy; `disable_security_headers()` strips the baseline from both |
| `MODEL_REGISTRY` | two apps routinely declare a model of the same name — one would get the other's validations, callbacks, relations, encrypted fields and connection routing |

### Mounting a second application

Mount-time state — the app root, the jails, the views directory, the model and controller registries — is written to whichever tenant the *calling thread* is bound to. A host mounting a second application from the thread that booted the first would overwrite it, so mounting runs under `tenant::scoped(id, || …)`, which binds for the duration and restores the previous binding afterwards. It restores on panic too: a mount that fails halfway must not leave the thread pointing at a half-built tenant.

`tests/tenant_isolation_test.rs` is the acceptance criterion. It mounts two applications on one thread and asserts each sees only its own root, jail and collections; that a worker thread pinned to a tenant reads what the mounting thread wrote (which is why this state is in the process-wide registry rather than a `thread_local!`); and that the primary tenant is untouched by the others, which is what makes the whole conversion a no-op for `soli serve`.

### Which application serves a request

`src/serve/vhost.rs`. A process serving one application answers everything with it; a process serving several picks by `Host` header, which is the only thing a client says about which site it meant.

`Router::single(runtime)` is what `soli serve` builds: a single fallback entry that answers every host, and no host at all — which is why adding the router changed nothing for a single-application server. A host serving several builds `Router::new()` and one `insert` per mounted application; a request whose host nobody claims gets **421 Misdirected Request** rather than whichever application happens to be first.

The header parsing is the part with traps, and it is tested on its own: `Host` is case-insensitive, a port is not part of the site (`example.com:8443` is `example.com`), `example.com.` is the fully-qualified spelling of the same name, and an IPv6 literal has colons *inside* its brackets — the naive `split(':').next()` turns `[::1]:8080` into `[`. A host claimed twice is reported to the caller rather than silently reassigned, because which application lost would otherwise depend on mount order.

HTTP/2 carries `:authority` rather than a `Host` header, and hyper leaves it in the URI, so the lookup falls back to the URI authority.

### The request path's tenant bundle

`handle_hyper_request` used to take seven separate per-application arguments — the worker queue, the reload channel, the public dir, the asset cache, the two realtime senders, the dev flag. They are now one `TenantRuntime`, destructured at the top of the function so its 1400-line body is unchanged.

That is the shape a host needs: picking which application serves a request becomes one lookup against one value from the `Host` header, not seven parallel maps. With a single application there is exactly one `TenantRuntime` (`TenantId::PRIMARY`) and nothing about the request path changes.

Its `tenant` field is not read on the request path yet, and that is deliberate: worker threads are pinned to their tenant, so choosing the queue already chooses the tenant. A host that ever shared one worker pool between applications would bind from that field instead.

### Deliberately left alone

Three, each for a reason worth reading before "finishing" them:

* **`mixin_registry::HOOKS`** cannot be keyed per application on its own. The only reason a cache-hit thread finds anything there is that some *other* thread registered it, and `compiled_cache::MODULE_CACHE` is keyed by source text, not by application. Key the hooks per tenant and the second app to compile an identical module gets a cache hit, registers nothing under its own id, and silently loses every `included do`. Fixing it properly means keying both by something content-derived — a change to the compile cache.
* **`server::ROUTES`** holds `Vec<Value>` for middleware, so it is `Rc`-based and can only ever be a `TenantLocal`, never a `Tenant` field. Under the rule that worker threads serve one application for life, a thread-local already *is* per application, and keying it would put a hash lookup on the route index for every request. What the rule does not cover is boot — see the note in `builtins/server.rs`.
* **`.env`** is loaded with `std::env::set_var`, which is process-wide whatever the tenant registry does. Routing it per application means every `std::env::var` read in the tree consults the tenant first — hundreds of sites, and a decision about what a library call inside an app should see. It belongs with the host that actually loads two `.env` files, not before it.

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
