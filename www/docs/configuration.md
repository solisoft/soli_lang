# Configuration

Soli loads environment variables from the process, then from `.env`, and finally from `.env.{APP_ENV}` when `APP_ENV` is set. Environment-specific files override `.env`, except variables listed in `SOLI_PROTECT_ENV`.

```bash
# .env
APP_ENV=development
SOLIDB_HOST=http://localhost:6745
SOLIDB_DATABASE=myapp_development
```

Keys must match `[A-Za-z_][A-Za-z0-9_]*`. Values cannot contain `\0`, `\r`, or `\n` — entries with control characters are skipped at load time with a warning on stderr. This avoids HTTP-header-split / log-injection vectors when an env value flows downstream into responses or structured logs.

The files are read from the app folder passed to `soli serve`. When serving a bundle (`soli serve app.soli`), they are read from the directory containing the `.soli` file — dotfiles are never included in a bundle, so ship the `.env` alongside it.

## Application Environment

| Variable | Purpose | Default |
|----------|---------|---------|
| `APP_ENV` | Selects `.env.{APP_ENV}` and marks test mode for features that need it. | unset |
| `SOLI_PROTECT_ENV` | Comma-separated variable names that `.env.{APP_ENV}` must not override. Mostly used by the test runner. | unset |

## Server And Development

| Variable | Purpose | Default |
|----------|---------|---------|
| `SOLI_HOST` | IP address the server binds to. Set `127.0.0.1` to keep a dev server off the LAN (only local processes can connect); the default listens on all interfaces. An invalid value is a startup error. | `0.0.0.0` |
| `SOLI_REQUEST_LOG` | Enables per-request `[LOG] METHOD PATH - STATUS (Xms)` lines on stdout when set to `1` or `true`. Always on under `--dev`. Alias for `SOLI_LOG=access`. | `false` |
| `SOLI_LOG` | Comma-separated production log channels: `access` (the request line), `query` (AQL queries with binds + duration), `http` (outgoing `HTTP.*` calls), `timing` (middleware/view/phase breakdown), or `all`. Each detail channel prints an indented block under the access line and implies `access`. Lets you see the rich per-request diagnostics — otherwise gated to `--dev` — without paying for full dev mode. | unset |
| `SOLI_SLOW_REQUEST_MS` | Slow-request threshold in milliseconds. A request whose total time (queue wait + handler) reaches it prints a full `[SLOW]` detail block — every `SOLI_LOG` channel plus the queue-wait split — while faster requests stay silent. Composes with `SOLI_LOG`. | unset |
| `SOLI_DB_POOL_IDLE_SECS` | Idle lifetime (seconds) of pooled SoliDB connections in the internal HTTP client. A retired idle connection means the next query pays a fresh DNS + TCP (+ TLS) connect mid-request. | `90` |
| `SOLI_DB_KEEP_WARM` | Set to `0` to disable the periodic keep-warm ping that holds a live SoliDB connection in the pool between sparse requests. Only spawned when a DB is configured (`SOLIDB_HOST` or credentials set). | enabled |
| `SOLI_NAV` | Controls instant-navigation injection (link clicks fetch + swap `<body>` in place instead of a full page load). Set `off`, `false`, `0`, or `no` to disable and fall back to plain hover prefetch. | enabled |
| `SOLI_PREFETCH` | Controls hover prefetch injection (and hover warming inside instant navigation). Set `off`, `false`, `0`, or `no` to disable. | enabled |
| `SOLI_PREFETCH_TTL` | Freshness window (seconds, clamped 1–300) for a prefetched HTML response, so the click reuses it without a revalidation round-trip — keeps prefetch working behind a CDN. | `30` |
| `SOLI_DEFAULT_URL_HOST` | Host used by `*_url` route helpers outside an active request. | unset |
| `SOLI_DEFAULT_URL_SCHEME` | Scheme used with `SOLI_DEFAULT_URL_HOST`. | `http` |
| `SOLI_DEV_REPL_ALLOW_REMOTE` | Allows the token-protected dev error-page REPL from non-loopback clients when set to `1`, `true`, or `yes`. Requires `SOLI_DEV_REPL_SECRET` (SEC-051) — the server refuses to start otherwise. | `false` |
| `SOLI_DEV_REPL_SECRET` | Pins the `/__dev/repl` token to an explicit shared secret instead of an auto-generated UUID. Required when `SOLI_DEV_REPL_ALLOW_REMOTE=1` so the credential is never embedded in dev-mode HTML error pages. | unset |
| `SOLI_TRACE_BOOT` | Prints boot timing trace when set. | unset |

### Parsing And Security Limits

| Variable | Purpose | Default |
|----------|---------|---------|
| `SOLI_DEFLATE_MAX_BYTES` | Maximum decompressed output (in bytes) that `Deflate.inflate` produces before it fails closed. A few-KB highly-repetitive raw-DEFLATE stream can inflate to many GB — a decompression bomb — and the SAML HTTP-Redirect binding feeds `Deflate.inflate` unauthenticated `SAMLRequest`/`SAMLResponse` payloads. Raise it only for legitimately large payloads. | `67108864` (64 MiB) |

### Bundle protection

Used when serving an encrypted / protected `.soli` bundle (see [Encrypted & Protected Bundles](/docs/development-tools/deploy#encrypted-bundles)). These are read at both `soli build --encrypt`/`--protect` and `soli serve app.soli`, and may live in the `.env` next to the bundle. Distinct from `SOLI_ENCRYPTION_KEY`, which encrypts model fields.

| Variable | Purpose | Default |
|----------|---------|---------|
| `SOLI_BUNDLE_KEY` | The bundle AES key material itself. Simplest option; also handy for local testing. | unset |
| `SOLI_BUNDLE_AUTH_URL` | URL of a key server. Soli issues a `GET`; the response body (≤ 4 KB, trimmed) is the key material. Revoke the entry to lock the app out. Used only when `SOLI_BUNDLE_KEY` is unset. | unset |
| `SOLI_BUNDLE_API_KEY` | Sent as the `x-api-key` header on the `SOLI_BUNDLE_AUTH_URL` request — this host's identity to the key server. | unset |
| `SOLI_BUNDLE_ALLOW_DISK` | Set to `1` to allow a decrypted bundle to extract to the temp dir when `/dev/shm` (RAM-backed tmpfs) is unavailable. Without it, such a boot is refused rather than writing plaintext to persistent disk. | unset |

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

Pooled SoliDB connections idle out after `SOLI_DB_POOL_IDLE_SECS` (default
90s). On a quiet server, a request arriving after a longer gap used to pay a
fresh DNS + TCP (+ TLS for remote hosts) connect mid-request — visible as
intermittent latency spikes. When a DB is configured, `soli serve` now runs
a periodic read-only `RETURN 1` ping that keeps a live connection pooled at
all times (and pre-warms the model DB at boot). Disable it with
`SOLI_DB_KEEP_WARM=0`.

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
| `SOLI_JOBS_DATABASE` | SolidB database that stores queues and cron entries. Falls back to `SOLIDB_DATABASE`. | `default` |
| `SOLI_JOBS_DEFAULT_QUEUE` | Queue used when no queue is specified. | `default` |
| `SOLI_JOBS_CALLBACK_URL` | Base URL SolidB calls when a job fires. | `http://127.0.0.1:3000/_jobs/run` |
| `SOLI_JOBS_SECRET` | **Required.** HMAC-SHA256 key used to sign and verify job callbacks (`X-Job-Signature` header). If unset, `/_jobs/run/:name` is not registered — see [Jobs / Signed Callbacks](jobs.md#security-signed-callbacks). | unset |
| `SOLI_JOB_WORKERS` | Size of the in-process pool that runs jobs marked `static background: Bool = true`. `0` disables backgrounding (all jobs run inline) — see [Jobs / Long-Running Jobs](jobs.md#long-running-jobs-background-pool). | `2` |

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
