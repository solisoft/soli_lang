# Configuration

Soli loads environment variables from the process, then from `.env`, and finally from `.env.{APP_ENV}` when `APP_ENV` is set. Environment-specific files override `.env`, except variables listed in `SOLI_PROTECT_ENV`.

```bash
# .env
APP_ENV=development
SOLIDB_HOST=http://localhost:6745
SOLIDB_DATABASE=myapp_development
```

Keys must match `[A-Za-z_][A-Za-z0-9_]*`. Values cannot contain `\0`, `\r`, or `\n` — entries with control characters are skipped at load time with a warning on stderr. This avoids HTTP-header-split / log-injection vectors when an env value flows downstream into responses or structured logs.

The files are read from the app folder passed to `soli serve`. When serving a bundle (`soli serve app.soli`) or running a standalone executable (`soli build --standalone`), they are read from the directory containing the `.soli` file / the executable — dotfiles are never included in a bundle, so ship the `.env` alongside the artifact.

## Application Environment

| Variable | Purpose | Default |
|----------|---------|---------|
| `APP_ENV` | Selects `.env.{APP_ENV}` and marks test mode for features that need it. | unset |
| `SOLI_PROTECT_ENV` | Comma-separated variable names that `.env.{APP_ENV}` must not override. Mostly used by the test runner. | unset |
| `SOLI_DB_ADAPTER` | Single-connection backend when `config/database.toml` is absent: `solidb` (default), `postgres`, or `mysql`. SQL adapters are a document subset (CRUD, hash filters, aggregates, includes batching, migrations). Multi-DB apps use `config/database.toml` instead — see [Multiple Databases](multi-database.md). | `solidb` |
| `DATABASE_URL` | Connection URL for SQL adapters (e.g. `postgres://user:pass@localhost:5432/myapp`). Required when `SOLI_DB_ADAPTER` is `postgres` or `mysql`. Ignored for SoliDB. Named SQL connections in TOML use `url =` per connection. | unset |
| `SOLI_DB_POOL_SIZE` | Default SQL pool size (single-connection mode). TOML `pool = N` overrides per connection. | `10` |

## Server And Development

| Variable | Purpose | Default |
|----------|---------|---------|
| `SOLI_HOST` | IP address the server binds to. Set `127.0.0.1` to keep a dev server off the LAN (only local processes can connect); the default listens on all interfaces. An invalid value is a startup error. [File mode](static-server.md) defaults to `127.0.0.1` instead. | `0.0.0.0` |
| `SOLI_WORKERS` | Number of request-handling worker threads. Each worker is a full interpreter copy (its own parsed app + builtins), so this is the primary lever on baseline RSS. Defaults to the number of CPU cores; when `APP_ENV=production` (or `prod`) and this is unset, defaults to **2** so a many-core box does not open one interpreter per core. Set explicitly (or pass `--workers N`) to opt into more throughput. | CPU cores; **2** in production |
| `SOLI_WS_WORKERS` | Worker threads reserved exclusively for realtime (WebSocket/LiveView) events, so a burst of them can't starve HTTP and a slow handler can't delay presence/broadcasts. The reservation costs a whole HTTP worker, so by default it only applies once the pool has **4 or more** workers; below that every worker drains both channels and realtime shares the pool. Set it explicitly to force the split at any size (`1` on a 2-worker pool leaves 1 HTTP worker), or `0` to disable it entirely. Always clamped so at least one HTTP worker remains. The startup line reports the resulting layout. | `1` when workers ≥ 4, else `0` |
| `SOLI_REQUEST_LOG` | Enables per-request `[LOG] METHOD PATH - STATUS (Xms)` lines on stdout when set to `1` or `true`. Always on under `--dev`. Alias for `SOLI_LOG=access`. | `false` |
| `SOLI_LOG` | Comma-separated production log channels: `access` (the request line), `query` (AQL queries with binds + duration; bind values whose name looks like a credential are logged as `[REDACTED]`), `http` (outgoing `HTTP.*` calls; query-string values whose parameter name looks like a credential are logged as `[REDACTED]`), `timing` (middleware/view/phase breakdown), or `all`. Each detail channel prints an indented block under the access line and implies `access`. Lets you see the rich per-request diagnostics — otherwise gated to `--dev` — without paying for full dev mode. | unset |
| `SOLI_LOG_FORMAT` | Shape of production request (and error) logs: `text` (default multi-line human output) or `json` (one NDJSON object per event on stdout/stderr — ship to Loki, CloudWatch, Datadog, …). Detail channels become nested arrays on the same object; secret-bearing bind/query values stay redacted. | `text` |
| `SOLI_SLOW_REQUEST_MS` | Slow-request threshold in milliseconds. A request whose total time (queue wait + handler) reaches it prints a full `[SLOW]` detail block — every `SOLI_LOG` channel plus the queue-wait split — while faster requests stay silent. Composes with `SOLI_LOG`. | unset |
| `SOLI_OTEL` | Set to `1`/`true`/`yes` to enable OpenTelemetry tracing. Reuses the same per-request span tree the dev-bar flamegraph builds (middleware, action, views, DB, HTTP). Honours inbound W3C `traceparent`, echoes it on the response, and exports spans over OTLP/HTTP JSON. When set without an endpoint, defaults to `http://127.0.0.1:4318/v1/traces` (sidecar collector). | unset |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | OTLP collector base URL (e.g. `http://otel-collector:4318`). Enables tracing even when `SOLI_OTEL` is unset. Soli appends `/v1/traces` unless the value already ends with it. | unset |
| `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` | Full traces URL override (takes precedence over `OTEL_EXPORTER_OTLP_ENDPOINT`). | unset |
| `OTEL_SERVICE_NAME` | `service.name` resource attribute on exported spans. | `soli` |
| `OTEL_RESOURCE_ATTRIBUTES` | Extra resource attributes as comma-separated `key=value` pairs (e.g. `deployment.environment=prod,service.namespace=shop`). | unset |
| `OTEL_SDK_DISABLED` | Set to `true` to force tracing off regardless of the other OTEL vars. | unset |
| `SOLI_DB_POOL_IDLE_SECS` | Idle lifetime (seconds) of pooled SoliDB connections. A retired idle connection means the next query pays a fresh DNS + TCP (+ TLS) connect mid-request. Two defaults, because the two clients can afford different windows: the shared client holds a connection for 90s (its reactor runs continuously, so it sees the server close one and drops it), while a per-worker pool holds one for 25s — a worker's reactor only runs during a query, so between requests nothing notices the peer closing an idle connection and the pool must retire it first. SoliDB closes idle keep-alives after 30s. Setting this overrides both; keep it below the idle-close of whatever is on the other end. | `90` shared, `25` per-worker |
| `SOLI_DB_KEEP_WARM` | Set to `0` to disable the periodic keep-warm ping that holds a live SoliDB connection in the pool between sparse requests. Only spawned when a DB is configured (`SOLIDB_HOST` or credentials set). | enabled |
| `SOLI_DB_POOL_MAX_IDLE` | Max idle SoliDB connections kept per host by the shared internal HTTP client. Per-worker DB clients hold one hot connection each and are unaffected; this sizes the pool for the paths that still share a client (async contexts, keep-warm). | `8` |
| `SOLI_DB_SHARED_REACTOR` | Set to `1` to drive DB queries on the server's shared tokio runtime instead of each worker's own reactor. Escape hatch for the pre-worker-reactor behavior — the default is faster (readiness is polled by the thread that waits on it) and creates no TCP churn. | unset |
| `SOLI_NAV` | Controls instant-navigation injection (link clicks fetch + swap `<body>` in place instead of a full page load). Set `off`, `false`, `0`, or `no` to disable and fall back to plain hover prefetch. | enabled |
| `SOLI_PREFETCH` | Controls hover prefetch injection (and hover warming inside instant navigation). Set `off`, `false`, `0`, or `no` to disable. | enabled |
| `SOLI_PREFETCH_TTL` | Freshness window (seconds, clamped 1–300) for a prefetched HTML response, so the click reuses it without a revalidation round-trip — keeps prefetch working behind a CDN. | `30` |
| `SOLI_DEFAULT_URL_HOST` | Host used by `*_url` route helpers outside an active request. | unset |
| `SOLI_DEFAULT_URL_SCHEME` | Scheme used with `SOLI_DEFAULT_URL_HOST`. | `http` |
| `SOLI_DEV_REPL_ALLOW_REMOTE` | Allows the token-protected dev error-page REPL from non-loopback clients when set to `1`, `true`, or `yes`. Requires `SOLI_DEV_REPL_SECRET` (SEC-051) — the server refuses to start otherwise. | `false` |
| `SOLI_DEV_REPL_SECRET` | Pins the `/__dev/repl` token to an explicit shared secret instead of an auto-generated UUID. Required when `SOLI_DEV_REPL_ALLOW_REMOTE=1` so the credential is never embedded in dev-mode HTML error pages. | unset |
| `SOLI_OPENAPI` | Set to `1`/`true` to expose an OpenAPI 3 spec at `/openapi.json` (generated from the routes) and a Scalar API-reference UI at `/openapi`. Opt-in (404 otherwise); served in every environment once on. See [Routing → OpenAPI](routing.md#openapi-soli_openapi). | unset |
| `SOLI_OPENAPI_TITLE` | Title of the generated OpenAPI document. | `Soli API` |
| `SOLI_SHUTDOWN_GRACE_SECS` | How long a `SIGTERM`/`SIGINT` shutdown waits for in-flight requests to finish before exiting anyway. See [Health checks and graceful shutdown](#health-checks-and-graceful-shutdown). `0` exits immediately. | `25` |
| `SOLI_TRACE_BOOT` | Prints boot timing trace when set. | unset |

### Structured logs and OpenTelemetry

Production logs and distributed traces have a dedicated operator guide:

**[Observability](observability.md)** — metrics (`/_metrics`), `SOLI_LOG` channels, `SOLI_LOG_FORMAT=json`, slow-request mode, W3C `traceparent`, OTLP export, and log↔trace joins.

Quick enable:

```bash
# Machine-parseable access logs
SOLI_LOG=access SOLI_LOG_FORMAT=json soli serve

# Traces → collector (or SOLI_OTEL=1 for a local :4318 sidecar)
OTEL_EXPORTER_OTLP_ENDPOINT=http://otel-collector:4318 \
OTEL_SERVICE_NAME=myapp \
soli serve
```

The env rows above (`SOLI_LOG_FORMAT`, `SOLI_OTEL`, `OTEL_*`) are the full knobs; the guide covers fields, span kinds, and limits.

### Health checks and graceful shutdown

Two endpoints let an orchestrator or load balancer see the server's lifecycle. Both are
plain text, need no authentication, and are always available — there is nothing to enable.

| Endpoint | Meaning | Answers |
|----------|---------|---------|
| `GET /_health` | **Liveness** — is this process alive? | `200 ok` for as long as the server runs, *including while it shuts down* |
| `GET /_ready` | **Readiness** — should traffic be routed here right now? | `200 ready`, or `503 starting` before workers finish booting, or `503 draining` during shutdown |

The distinction matters. A shutting-down process is perfectly healthy — it just does not
want new work. If `/_health` failed during shutdown, an orchestrator would restart a
container that was already exiting cleanly. Point liveness probes at `/_health` and
readiness probes at `/_ready`.

#### What happens on SIGTERM

On `SIGTERM` or `SIGINT` the server drains rather than cutting requests off:

1. `/_ready` starts answering `503 draining`, so the load balancer stops routing here.
2. New requests get `503 Server shutting down` with `Connection: close`. Probes still answer.
3. Requests already in flight **run to completion** and return their real response.
4. Once the last one finishes — or `SOLI_SHUTDOWN_GRACE_SECS` elapses — the process exits `0`.

A second signal skips the wait and exits immediately.

The default 25s sits just under Kubernetes' default 30s `terminationGracePeriodSeconds`, so
the process exits on its own rather than being `SIGKILL`ed. If you raise one, raise both.

```yaml
# Kubernetes
livenessProbe:
  httpGet: { path: /_health, port: 3000 }
readinessProbe:
  httpGet: { path: /_ready, port: 3000 }
terminationGracePeriodSeconds: 30    # must exceed SOLI_SHUTDOWN_GRACE_SECS
```

### Parsing And Security Limits

| Variable | Purpose | Default |
|----------|---------|---------|
| `SOLI_DEFLATE_MAX_BYTES` | Maximum decompressed output (in bytes) that `Deflate.inflate` produces before it fails closed. A few-KB highly-repetitive raw-DEFLATE stream can inflate to many GB — a decompression bomb — and the SAML HTTP-Redirect binding feeds `Deflate.inflate` unauthenticated `SAMLRequest`/`SAMLResponse` payloads. Raise it only for legitimately large payloads. | `67108864` (64 MiB) |

### Bundle protection

Used when serving an encrypted / protected `.soli` bundle (see [Encrypted & Protected Bundles](/docs/development-tools/deploy#encrypted-bundles)). These are read at `soli build --encrypt`/`--protect` time, by `soli serve app.soli`, and by standalone executables built with `--standalone`; they may live in the `.env` next to the artifact. Distinct from `SOLI_ENCRYPTION_KEY`, which encrypts model fields.

| Variable | Purpose | Default |
|----------|---------|---------|
| `SOLI_BUNDLE_KEY` | The bundle AES key material itself. Simplest option; also handy for local testing. | unset |
| `SOLI_BUNDLE_AUTH_URL` | URL of a key server. Soli issues a `GET`; the response body (≤ 4 KB, trimmed) is the key material. Revoke the entry to lock the app out. Used only when `SOLI_BUNDLE_KEY` is unset. | unset |
| `SOLI_BUNDLE_API_KEY` | Sent as the `x-api-key` header on the `SOLI_BUNDLE_AUTH_URL` request — this host's identity to the key server. | unset |
| `SOLI_BUNDLE_ALLOW_DISK` | Set to `1` to allow a decrypted bundle to extract to the temp dir when `/dev/shm` (RAM-backed tmpfs) is unavailable. Without it, such a boot is refused rather than writing plaintext to persistent disk. | unset |
| `SOLI_RELEASE_BASE_URL` | Base URL `soli build --standalone --target <t>` downloads release runtimes from (layout: `{base}/v{version}/soli-{target}.tar.gz` + `.sha256`). For mirrors and air-gapped build machines. | GitHub releases |

### Production logging (`SOLI_LOG`)

The AQL query log, the outgoing HTTP log, and the middleware/view/phase
timing breakdown normally only feed the dev bar under `--dev`. `SOLI_LOG`
turns those same channels on in production and prints them to stdout as an
indented block under each request's access line — so you can debug a slow
or failing route on a live server without redeploying in dev mode (which
would also disable the VM, enable hot-reload, and inject the bar).

```bash
# Just the access line (same as SOLI_REQUEST_LOG=1)
SOLI_LOG=access soli serve

# Queries + outgoing HTTP for the whole app
SOLI_LOG=query,http soli serve

# Everything
SOLI_LOG=all soli serve
```

A request with `SOLI_LOG=query,http,timing` prints:

```text
[LOG] GET /posts - 200 (12.480ms)
  db: 2 queries (8.210ms)
    (5.110ms) FOR p IN posts FILTER p.published == @v0 RETURN p binds={"v0":true}
    (3.100ms) FOR c IN comments FILTER c.post_id == @v0 RETURN c binds={"v0":"abc"}
  http: 1 call (2.000ms)
    (2.000ms) GET https://api.example.com/feed -> 200
  timing:
    middleware auth (0.420ms)
    view posts/index (3.050ms)
      view posts/_card (1.200ms)
```

The whole block is written with a single `println!` so concurrent worker
threads never interleave their output. Bind variables and HTTP URLs are
scrubbed of secret-bearing values before they reach the log.

### Slow-request logging (`SOLI_SLOW_REQUEST_MS`)

`SOLI_LOG=all` prints a block for every request — too noisy to leave on in
production. `SOLI_SLOW_REQUEST_MS` instead emits the full detail block only
for requests whose total time (queue wait + handler) crosses the threshold,
and nothing at all for fast ones:

```bash
# Log a full breakdown only for requests slower than 100ms
SOLI_SLOW_REQUEST_MS=100 soli serve
```

```text
[SLOW] GET /gather/map - 200 (412.480ms + 0.320ms queue)
  db: 3 queries (398.210ms)
    (395.110ms) FOR p IN pins FILTER p.board == @v0 RETURN p binds={"v0":"x"}
    ...
  timing:
    view gather/map (10.050ms)
```

The access line shows handler time plus the time the request waited in the
worker queue before being picked up, so a request stuck behind a busy worker
is distinguishable from a genuinely slow handler. It composes with
`SOLI_LOG`: explicitly requested channels still print for every request; the
threshold adds the `[SLOW]` block on top.

### DB connection keep-warm

Pooled SoliDB connections idle out after `SOLI_DB_POOL_IDLE_SECS`. On a quiet
server, a request arriving after a longer gap used to pay a fresh DNS + TCP
(+ TLS for remote hosts) connect mid-request — visible as intermittent latency
spikes. When a DB is configured, `soli serve` now runs a periodic read-only
`RETURN 1` ping that keeps a live connection pooled at all times (and pre-warms
the model DB at boot). Disable it with `SOLI_DB_KEEP_WARM=0`.

The ping runs on its own thread, and with one connection pool per worker it can
only refresh its own — so a worker's connection is instead kept inside the 25s
idle window described above, short enough that the pool retires it before
SoliDB's 30s idle close. A worker idle longer than that pays one reconnect on
its next query, which is the fresh-connect cost the ping avoids elsewhere.

### Keeping memory low

`soli serve` runs a pool of worker **threads** in one process, and each worker
holds its own copy of the parsed app plus the full builtin surface (`Rc`-based
values can't be shared across threads). So baseline RSS scales with the worker
count, and — for apps with lots of code or large in-memory data (e.g. i18n
locale tables) — with the size of that app. The levers, cheapest first:

| Lever | Effect |
|-------|--------|
| `SOLI_WORKERS=N` | The biggest one — each worker is a full interpreter copy. With `APP_ENV=production`, the default is already **2** (not one-per-core). Raise it for throughput, or set `1` for a low-traffic service. Note the throughput floor: a worker blocks for the whole of each database round-trip, so a DB-backed route tops out near `workers × (1 / query latency)` — roughly 11k req/s per worker against a loopback SoliDB. Routes that never touch the DB are unaffected (a single worker serves >140k req/s). |
| `SOLI_JOB_WORKERS=1` (or `0`) | The job worker pool is a second set of full interpreters. It defaults to `1`; `0` disables the job engine in this process. |
| `SOLI_JOB_VIEW_HELPERS=0` | Drops view helpers (incl. i18n locale tables) from every job interpreter when jobs don't render helper-using templates. |
| Slim Cargo features | Build only the subsystems you need (see below). Omitting SQL clients and PASETO shrinks the binary and the code pages mapped into every worker. |
| `MIMALLOC_PURGE_DELAY=0` | mimalloc returns freed pages to the OS promptly instead of after its default delay — trims the RSS left over from the one-time boot-parse churn. Read by the allocator at startup, so set it in the environment before launch. Trade-off: a few more `madvise`/decommit syscalls under churny allocation. |
| Fewer/lazier locales | If most of an app's per-worker memory is i18n tables, load only the locales you serve (or move them to `config/locales/*.yml`, which the framework loads **once** process-wide into a shared store rather than per-worker). |

#### Slim binary (Cargo features)

`cargo install` / CI use the **default** feature set so published binaries match a full product build. Optional subsystems can be dropped at **compile time** when you build from source:

| Feature | Default | What it pulls in |
|---------|---------|------------------|
| `embedding` | on | Vector / embedding helpers |
| `llm` | on | `llm_generate` (OpenAI-compatible chat) |
| `codegraph` | on | `soli graph build` on non-Soli repos (tree-sitter + grammars) |
| `paseto` | on | `Paseto` class (`pasetors` crate) |
| `postgres` | on | PostgreSQL document adapter + client pool |
| `mysql` | on | MySQL / MariaDB document adapter + client pool |
| `sql` | off | Alias for `postgres` + `mysql` |
| `solidb-driver` | off | Native SoliDB TCP driver (needs `solidb-client`) |
| `full` | off | Default set + `solidb-driver` |

SoliDB (HTTP) and the rest of the runtime always stay linked. A SoliDB-only install without PASETO or SQL clients:

```bash
cargo install --path . --locked --no-default-features \
  --features embedding,llm,codegraph
```

Postgres only (no MySQL client, no PASETO):

```bash
cargo install --path . --locked --no-default-features \
  --features embedding,llm,codegraph,postgres
```

If the binary was built without an adapter and you set `SOLI_DB_ADAPTER=postgres`
(or a `database.toml` entry for it), boot fails with a rebuild hint rather than a
missing symbol. The same applies to `Paseto.*` — the class is simply not registered
when the `paseto` feature is off.

The boot process also builds one extra interpreter to register the shared
route/model/controller/template registries before workers start; it is now
reclaimed immediately after boot rather than parked for the process lifetime.

## Hardening

These knobs control how the request edge handles untrusted input. See the
[Server Hardening](/docs/builtins/hardening) page for the full story.

| Variable | Purpose | Default |
|----------|---------|---------|
| `SOLI_TRUST_PROXY` | Honors `X-Forwarded-Proto` / `X-Forwarded-Host` when set to `1`, `true`, or `yes`. Only enable when the deployment terminates these headers at a trusted proxy hop. | `false` |
| `SOLI_FORCE_SECURE_COOKIES` | Set to `1`/`true`/`yes` to add `Secure` to every session cookie regardless of detected scheme. Use when the deployment is always on TLS but the proxy doesn't forward `X-Forwarded-Proto: https` (or `enable_trust_proxy()` isn't on). Equivalent runtime call: `enable_force_secure_cookies()`. | `false` |
| `SOLI_MAX_BODY_SIZE` | Maximum buffered request body, in bytes. Requests over the cap return `413 Payload Too Large`. | `8388608` (8 MiB) |
| `SOLI_DISABLE_CSRF` | Disables the same-origin CSRF check entirely when set to `true`. For API-only deployments where no cookie session is in play. Per-route opt-out via `skip_csrf("/path")` in `config/routes.sl` is preferred — see [Routing → CSRF Protection](/docs/routing#csrf-protection). | unset |
| `SOLI_CSRF_TOKENS` | Set to `require` to make per-form CSRF tokens mandatory for browser form posts (urlencoded/multipart) — a form post without a valid token returns 403. Tokens are always *verified when present* regardless of this setting. See [Forms & CSRF](/docs/core-concepts/forms). | unset |
| `SOLI_HTTP_MAX_RESPONSE_BYTES` | Maximum bytes Soli will buffer from a single outbound HTTP response (`HTTP.*`, `SOAP.*`). A malicious or compromised upstream returning a multi-GB body would otherwise OOM the worker. | `52428800` (50 MiB) |
| `SOLI_IMAGE_MAX_ALLOC_BYTES` | Maximum bytes the image decoder will allocate for a single image (`Image.*`, plan execution). Defends against decompression bombs — a 100 KB PNG declaring 65535×65535 pixels would otherwise allocate ~16 GB of RGBA pixels. | `268435456` (256 MiB) |
| `SOLI_IMAGE_MAX_DIMENSION_PX` | Maximum pixel dimension on either axis for any decoded image. Images declaring more are rejected before allocation. | `16384` |
| `SOLI_PARALLEL_MAX_ITEMS` | Maximum input list length accepted by `HTTP.get_all`, `HTTP.get_all_json`, `HTTP.parallel`, and `Image.process_all`. Calls with longer arrays are rejected before any thread is spawned. | `256` |
| `SOLI_PARALLEL_MAX_CONCURRENCY` | Maximum OS threads alive at one time inside a parallel fan-out call. The runner consumes the input list in chunks of this size. | `16` |
| `SOLI_MAX_UPLOAD_FILES` | Maximum number of file parts accepted per multipart request. A body packed with thousands of tiny parts would otherwise allocate a per-file Soli hash for each one and OOM the worker. | `32` |

## Database

| Variable | Purpose | Default |
|----------|---------|---------|
| `SOLIDB_HOST` | SoliDB server URL. An explicit `http://` / `https://` prefix is preserved. When the scheme is omitted, the host defaults to `https://` for remote DBs and `http://` for loopback (`localhost`, `127.0.0.1`, `::1`) so the dev loop stays plaintext while remote DBs are TLS by default. | `http://localhost:6745` |
| `SOLIDB_DATABASE` | Database name used by models, migrations, uploads, and jobs fallback. | `default` |
| `SOLIDB_API_KEY` | API-key auth for SoliDB where supported. | unset |
| `SOLIDB_USERNAME` | Username for SolidB login/basic auth. | unset |
| `SOLIDB_PASSWORD` | Password paired with `SOLIDB_USERNAME`. | unset |

## Sessions

| Variable | Purpose | Default |
|----------|---------|---------|
| `SOLI_SESSION_DRIVER` | Session backend: `in_memory`, `cookie`, `disk`, `solidb`, or `solikv`. | `in_memory` |
| `SOLI_SESSION_SECRET` | Secret for the `cookie` session driver (32+ characters — e.g. `openssl rand -hex 32`). The AES-256-GCM key that seals client-side sessions is HKDF-derived from it; rotating it invalidates every outstanding session. Required when the driver is `cookie`. | unset |
| `SOLI_SESSION_PATH` | Directory for disk-backed session files. | `./sessions` |
| `SOLI_SESSION_TTL` | Session timeout in seconds. | `86400` |
| `SOLI_SESSION_SAMESITE` | `SameSite` attribute on the session cookie: `Lax`, `Strict`, or `None`. `Strict` blocks the cookie on any cross-site navigation; `None` is intended for cross-site embeds and **automatically pairs with `Secure`** — Soli forces the flag on regardless of the detected request scheme so browsers don't silently drop the cookie. Unknown values fall back to `Lax`. | `Lax` |
| `SOLI_SESSION_HOST_PREFIX` | Set to `1`/`true`/`yes` to emit the cookie under the `__Host-` prefix (`__Host-session_id`). The browser only accepts `__Host-` cookies that are `Secure`, have `Path=/`, and carry no `Domain` attribute, which prevents subdomain takeover from setting an attacker-controlled session cookie. The prefix is only applied when `Secure` is also active (i.e. behind HTTPS); otherwise the plain `session_id` name is used. | unset |
| `SOLI_SOLIDB_HOST` | SolidB host for the `solidb` session driver. Must be `https://` or a loopback (`localhost`, `127.0.0.1`, `::1`) — plaintext HTTP to a remote SolidB is rejected. | driver default |
| `SOLI_SOLIDB_DATABASE` | SolidB database for sessions. | driver default |
| `SOLI_SOLIDB_COLLECTION` | SolidB collection for sessions. | driver default |
| `SOLI_SOLIDB_API_KEY` | API key the `solidb` session driver presents to SolidB. Required for non-loopback hosts. Falls back to `SOLIDB_API_KEY` (the same key the Model layer reads) when unset. | unset |
| `SOLI_SOLIDB_USERNAME` | Basic-auth username for the `solidb` session driver (paired with `SOLI_SOLIDB_PASSWORD`). Falls back to `SOLIDB_USERNAME`. | unset |
| `SOLI_SOLIDB_PASSWORD` | Basic-auth password for the `solidb` session driver. Falls back to `SOLIDB_PASSWORD`. | unset |
| `SOLI_SESSION_ALLOW_INSECURE_HTTP` | Set to `1`/`true`/`yes` to allow plaintext HTTP and missing auth on non-loopback session hosts. Only when the network path is operator-trusted. | unset |
| `SOLI_SOLIKV_HOST` | SoliKV host for the `solikv` session driver. Must be a loopback (`localhost`, `127.0.0.1`, `::1`) — SoliKV uses plaintext RESP/TCP and the `AUTH` token transits in the clear, so non-loopback hosts are rejected. | `localhost` |
| `SOLI_SOLIKV_PORT` | SoliKV port for sessions. | `6380` |
| `SOLI_SOLIKV_TOKEN` | SoliKV auth token for sessions. Sent as a Redis-style `AUTH` command — same loopback-only constraint as the host. | unset |

## Jobs

| Variable | Purpose | Default |
|----------|---------|---------|
| `SOLI_JOBS_POLL_MS` | How often the job poller looks for due work, in milliseconds. | `1000` |
| `SOLI_JOBS_DEFAULT_QUEUE` | Queue used when no queue is specified. | `default` |
| `SOLI_JOBS_LEASE_SECS` | Lease length for a claimed job. A `running` job whose lease expires is reclaimed by another poller — raise this for long jobs. | `60` |
| `SOLI_JOBS_MAX_RETRIES` | Default retry budget per job; a job past it becomes `dead`. | `3` |
| `SOLI_JOBS_RETENTION_SECS` | How long completed job rows are kept before pruning. | `604800` |
| `SOLI_JOB_WORKERS` | Worker threads that run job code. Each worker is a full interpreter copy, so the default is conservative; raise it for higher throughput, or set `0` to disable the job engine in this process — see [Jobs](jobs.md#configuration). | `1` |
| `SOLI_JOB_VIEW_HELPERS` | Whether background-job interpreters load view helpers (which include an app's i18n locale tables — often the largest per-interpreter cost). Set `0` to skip them when no job renders a helper-using template, dropping that memory from every job interpreter. | enabled |

## Cache And KV

| Variable | Purpose | Default |
|----------|---------|---------|
| `SOLIKV_RESP_HOST` | SoliKV RESP host used by KV/cache builtins. | `localhost` |
| `SOLIKV_RESP_PORT` | SoliKV RESP port. | `6380` |
| `SOLIKV_TOKEN` | SoliKV auth token. | unset |
| `SOLI_KV_ALLOW_ADMIN` | Set to `1`/`true`/`yes` to lift the denylist on destructive/admin RESP commands (`FLUSHALL`, `FLUSHDB`, `KEYS`, `SCAN`, `CONFIG`, `DEBUG`, `SHUTDOWN`, `MONITOR`, `CLIENT`, `EVAL`, `SCRIPT`, etc.) reachable from `KV.cmd`, `KV.flushdb`, and `KV.keys`. Only set this on a trusted, non-user-facing process. | unset |

## S3

| Variable | Purpose | Default |
|----------|---------|---------|
| `AWS_ACCESS_KEY_ID` | AWS-compatible access key. Alternative: `S3_ACCESS_KEY`. | required for S3 calls |
| `AWS_SECRET_ACCESS_KEY` | AWS-compatible secret key. Alternative: `S3_SECRET_KEY`. | required for S3 calls |
| `AWS_REGION` | AWS region. Alternative: `S3_REGION`. | `us-east-1` |
| `S3_ACCESS_KEY` | S3-compatible access key fallback. | unset |
| `S3_SECRET_KEY` | S3-compatible secret key fallback. | unset |
| `S3_REGION` | S3-compatible region fallback. | `us-east-1` |
| `S3_ENDPOINT` | Custom endpoint for MinIO or another S3-compatible service. | unset |

## Deploy

| Variable | Purpose | Default |
|----------|---------|---------|
| `SOLI_DEPLOY_API_KEY` | API key required by `soli deploy` for proxy deployment. | required for deploy |

## Test And Coverage Internals

These are normally set by Soli tooling rather than by applications.

| Variable | Purpose | Default |
|----------|---------|---------|
| `SOLI_COVERAGE_ENABLED` | Enables the server-side coverage dump endpoint for test aggregation. The endpoint requires `SOLI_COVERAGE_TOKEN` to be set as well — without a matching `X-Coverage-Token` request header it returns 403. | unset |
| `SOLI_COVERAGE_TOKEN` | Per-process secret gating `/__coverage__`. The test runner mints a fresh random token per run and sends it as `X-Coverage-Token` when scraping; without this token the endpoint refuses every caller, even when `SOLI_COVERAGE_ENABLED` is set. | required when `SOLI_COVERAGE_ENABLED` is set |

## Runtime Overrides

The hardening knobs above also have function equivalents that override the
env-driven default at runtime. Useful when a single action needs a different
limit, or when test setup needs to flip the gate without re-reading the
environment.

Soli loads `config/application.sl` once at boot, before `config/routes.sl`,
which makes it the natural place for app-wide startup config:

```soli
# config/application.sl

# Trust X-Forwarded-* only behind a trusted proxy.
enable_trust_proxy()

# Always emit Secure session cookies — appropriate when the deployment
# is always on TLS but the proxy doesn't forward X-Forwarded-Proto.
enable_force_secure_cookies()

# Raise the default 8 MiB body cap when an app needs larger uploads.
set_max_body_size(32 * 1024 * 1024)
```
