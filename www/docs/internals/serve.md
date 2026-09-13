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

## Builtins: one registry per consumer, on purpose

A worker thread builds the builtin registry three times — for the interpreter's globals, the template engine's environment (`src/template/core_eval.rs`), and the view helpers' closure (`load_view_helpers`). Sharing one root between the three, with the others as child scopes, was tried and **reverted**. It measured as no RSS win (a registry is a few hundred KB; five alternating A/B pairs at `SOLI_WORKERS=16` put the difference at 0.4 MiB against a ±12 MiB spread) and it changed behaviour in two ways separate registries never did: a bare `logger = …` inside a function created a short-lived local instead of updating the global, because the shared root had to be sealed against writes; and `Model.define_method(…)` mutated an `Rc<Class>` that outlived the interpreter, so a hot reload kept a method the developer had deleted. Three cheap registries that die with their interpreter are the right shape.

One thing from that attempt stays: the template engine no longer registers the test-only builtins. It called `register_builtins(env, true)` unconditionally, so `visit`, `click`, `assert_eq`, the factories and the mock-HTTP helpers resolved inside every rendered view in production — names `register_builtins` refuses everywhere else in serve mode precisely because leaking them into a served app is a hazard.

## Tenants

`src/serve/tenant.rs` is the seam that per-application state moves behind, so a process can eventually serve more than one app.

A `Tenant` owns what belongs to one application; the process holds a registry of them; each thread knows which one it is serving, and worker threads are bound to one tenant for life — every spawn on the worker side (the HTTP workers in `serve/mod.rs`, the `background_jobs` pool threads, the `jobs` poller and webhook threads) captures `tenant::current_id()` and calls `bind_current` first thing, since thread-locals do not cross `spawn`. So the current tenant is fixed when the thread starts rather than looked up per request. With a single application the registry holds exactly one tenant (`TenantId::PRIMARY`) and every thread falls back to it, so nothing observable changes.

The HTTP side is different. A request future runs on whichever tokio thread polls it next and moves between them at every `.await`, so a thread-local binding is meaningless there — the thread that starts a request is not the one that finishes it. Yet everything the request path consults before handing off to a worker is keyed by tenant: the CORS rules, the CSRF exemptions, the cookie jar, the session config, the dev-bar store. So the request future is scoped with `tenant::task_scope(id, …)` — a tokio task-local that travels with the future — as soon as the `Host` header has said which application it belongs to, and `current_id()` consults that binding before the thread-local. `cors::evaluate` runs *inside* the scope for that reason, after the host is resolved rather than before.

A task-local does not cross `tokio::spawn`, so every spawn on the request path — the WebSocket, EUI and LiveView upgrades, their writer tasks, the per-application LiveView reaper — goes through `tenant::spawn`, which re-scopes the new task to the spawning one's tenant. `scoped()` sets the task-local as well as the thread-local, so a mount performed from inside a request handler lands on the tenant it names rather than the request's.

**Lock poisoning** is tolerated throughout `tenant.rs`, on purpose. `TenantValue::read` retries until the value is found, so a poisoned lock treated as "absent" would spin forever; a jail that read as "none" after a panic elsewhere would lift a security boundary; an app root that fell back to `.` would resolve views against the process's working directory. The values are plain maps — a writer that panicked mid-closure leaves an entry, not a torn lock — so `into_inner` is the safe answer everywhere.

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
| `lazy_static! { RwLock<T> }` | `TenantValue<T>` | `read(\|v\| …)` / `write(\|v\| …)` — the constructor is part of the declaration, so it replaces `lazy_static!` outright and stays `const`. **One lock per tenant**: a closure that blocks under `write` (the database JWT login does, for its network timeout) stalls that tenant alone |

`read`/`write` take a closure rather than returning a guard: the value lives inside a map inside the lock, and stable Rust has no way to hand out a guard borrowed into it (`parking_lot`'s mapped guards would, but it is not a direct dependency). In exchange the lock is always released. Anything `Rc`-based stays in a plain `thread_local!`: a bound worker thread already is per application.

### Done, and left

Converted: the app root (was `live::component::APP_ROOT`); the `File` and `Image` jails (were `OnceLock`s); `VIEWS_DIR`, `PUBLIC_DIR` and `TEMPLATE_CACHE` (`init_templates` was first-caller-wins, so a second application would have rendered the first one's views); `JAR_CACHE`; `MOUNTED_ENGINES`; `MAILER_CONFIG`; `TRUSTED_PROXIES`; `RATE_LIMIT_STORE`; `SOLIKV_CONFIG` with its `RESP_POOL`; `CONTROLLER_REGISTRY`; `SECURITY_HEADERS_CONFIG` with `SECURITY_HEADERS_ENABLED`; `MODEL_REGISTRY` with the four `COLLECTION_*` maps beside it; and the whole database layer — `db::registry`, `db::config`, `CACHED_DB_CONFIG`, `DB_CONFIG` and the JWT state; the `TRUST_PROXY_ENABLED` gate (the lenient half of the trusted-proxies check — process-wide, one app's `trust_proxy(true)` made a co-hosted app trust attacker-supplied `X-Forwarded-*`); and the i18n store with its default locale, which sit on the render path.

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

`tests/tenant_isolation_test.rs` is the acceptance criterion. It mounts two applications on one thread and asserts each sees only its own root, jail and collections; that neither can see the other's **database registry**, which is the property everything else rests on; that a worker thread pinned to a tenant reads what the mounting thread wrote (which is why this state is in the process-wide registry rather than a `thread_local!`); and that the primary tenant is untouched by the others, which is what makes the whole conversion a no-op for `soli serve`.

### Which application serves a request

`src/serve/vhost.rs`. A process serving one application answers everything with it; a process serving several picks by `Host` header, which is the only thing a client says about which site it meant.

`Router::single(runtime)` is what `soli serve` builds: a single fallback entry that answers every host, and no host at all — which is why adding the router changed nothing for a single-application server. A host serving several builds `Router::new()` and one `insert` per mounted application; a request whose host nobody claims gets **421 Misdirected Request** rather than whichever application happens to be first.

The header parsing is the part with traps, and it is tested on its own: `Host` is case-insensitive, a port is not part of the site (`example.com:8443` is `example.com`), `example.com.` is the fully-qualified spelling of the same name, and an IPv6 literal has colons *inside* its brackets — the naive `split(':').next()` turns `[::1]:8080` into `[`. A host claimed twice is reported to the caller rather than silently reassigned, because which application lost would otherwise depend on mount order.

HTTP/2 carries `:authority` rather than a `Host` header, and hyper leaves it in the URI, so the lookup falls back to the URI authority.

### The request path's tenant bundle

`handle_hyper_request` used to take seven separate per-application arguments — the worker queue, the reload channel, the public dir, the asset cache, the two realtime senders, the dev flag. They are now one `TenantRuntime`, destructured at the top of the function so its 1400-line body is unchanged.

That is the shape a host needs: picking which application serves a request becomes one lookup against one value from the `Host` header, not seven parallel maps. With a single application there is exactly one `TenantRuntime` (`TenantId::PRIMARY`) and nothing about the request path changes.

Its `tenant` field is not read on the request path yet, and that is deliberate: worker threads are pinned to their tenant, so choosing the queue already chooses the tenant. A host that ever shared one worker pool between applications would bind from that field instead.

### The realtime, policy and staging state

A second sweep, after the database layer, over what a *request* reaches rather than what boot writes. All of it was process-global and all of it is per application now:

| Global | What sharing it would mean |
|---|---|
| `cors::CORS_RULES` | one app's allowed origins apply to the other's `/api/*` — the whole thing CORS exists to decide |
| `csrf::CSRF_SKIP_PATTERNS` | `skip_csrf("/webhooks/*")` in one app disables the CSRF barrier on another's `/webhooks/*` |
| `live::upload` store and chunks | user-uploaded bytes, keyed by a server-minted id, in one shared map |
| `background_jobs::BG_SENDER` | a pool thread builds its interpreter from *its* app's models and talks to *its* database, so a second app's jobs would run against the first one's everything |
| `LV_EVENT_TX`, `PINNED_LV_TX` | an app's EUI sessions rendered by workers that never loaded its code |
| `live::view` registry | live instances keyed by session id, their sockets, and the per-instance frame locks meant to serialise *one* session's renders |
| `websocket` registry | live sockets and the rooms they joined — two apps both using a room called `"lobby"` would broadcast into each other's |
| `live_query::SUBSCRIPTIONS` | one app's views woken by another's model writes |
| `socket::LIVEVIEW_ROUTES`, `LIVEVIEW_TICK_TASKS`, `ROOM_COMPONENTS` | two apps may both name a component `counter`, and rooms are opt-in precisely so nothing is shareable by accident |
| `eui::manifest` capabilities and signature | what one app asks the viewer to grant it; a co-hosted app would inherit permissions it never requested |
| `eui::stats`, `dev_store` | one app's dev bar listing another's traffic, and its replay button re-dispatching another's request |

Several were `OnceLock`s, so the failure mode was not a race — it was deterministic and silent: whichever application booted second simply got the first one's.

The two registries are held as `Arc`s behind their `TenantValue` (`live_registry()`, `get_ws_registry()`), so their ~45 call sites keep a handle rather than each going through a closure.

One thing that had to survive the conversion: `put_chunk` assembled a finished file, dropped the chunk lock explicitly, *then* called `put`, which takes the store lock. Wrapping the body in a closure would have nested the two and fixed a lock order this file deliberately does not have. It now returns what it assembled and calls `put` after the closure.

### Deliberately left shared

Four, each for a reason worth reading before "finishing" them:

* **`mixin_registry::HOOKS`** cannot be keyed per application on its own. The only reason a cache-hit thread finds anything there is that some *other* thread registered it, and `compiled_cache::MODULE_CACHE` is keyed by source text, not by application. Key the hooks per tenant and the second app to compile an identical module gets a cache hit, registers nothing under its own id, and silently loses every `included do`. Fixing it properly means keying both by something content-derived — a change to the compile cache.
* **`server::ROUTES`** holds `Vec<Value>` for middleware, so it is `Rc`-based and can only ever live in a `thread_local!`, never in a `Tenant` field. Under the rule that worker threads serve one application for life, a thread-local already *is* per application, and keying it would put a hash lookup on the route index for every request. What the rule does not cover is boot — see the note in `builtins/server.rs`.
* **SQL connection pools** (`db/{postgres,mysql,sqlite}.rs`) are keyed by connection name **and URL**. Two applications pointing a connection called `primary` at different databases already get different pools, and when the URL matches, sharing the pool is the point.
* **`serve/eui/assets.rs`** is content-addressed and bounded: the same bytes uploaded by two applications are one entry, which is the cross-tenant sharing step 6 wants, not a leak.

### Left for the host (step 4)

Not globals, but gaps a multi-application host has to close before it is correct:

* **`.env`.** `load_env_files` uses `std::env::set_var`, which is process-wide whatever the tenant registry does. Routing it per application means every `std::env::var` read in the tree consults the tenant first — hundreds of sites, and a decision about what a library call inside an app should see. It belongs with the host that actually loads two `.env` files, not before it.

### Still process-global

Known, not yet converted. None decides which database a query goes to or which application answers a request; each is its own small pass:

| Where | What it is |
|---|---|
| `builtins/logger.rs` | log level, format and the capture buffer |
| `builtins/resilience.rs` | named circuit breakers and semaphores — `payments` in two apps would trip together |
| `builtins/mail_outbox.rs` | captured mail, test mode only |
| `builtins/solidb.rs`, `imap.rs`, `pop3.rs` | handle tables keyed by an integer id |
| `serve/files/mod.rs` | the file-mode root and extra asset roots — a process in file mode serves one directory by definition |
| `serve/openapi.rs`, `serve/otel.rs`, `serve/prod_log.rs`, `metrics.rs` | observability config, read once from the environment |
| `jobs/mod.rs`, `jobs/engine.rs` | the job engine's config, node id and in-flight list |
| `bundle.rs` | the bundle metadata of a protected build |
| `interpreter/symbol.rs` | the interner, which leaks by design — see the hibernation note under step 5 of the plan |
| `serve/mod.rs` `GLOBAL_VFS` | the protected-bundle filesystem, `OnceLock` first-caller-wins — two bundled apps in one process would read the first one's bundle |
| `jobs/store.rs` `INDEXED` | "queue indexes exist" bit keyed by connection *name* only, so two apps both calling theirs `primary` share it |
| `template.rs` `DEV_MODE`, `session.rs` `SESSION_READY` | one dev flag and one readiness flag for the process; `/up` reports every tenant ready once any store warms |

Everything else that looked like a candidate is a `thread_local!`, and a worker thread serves one application for life.

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
