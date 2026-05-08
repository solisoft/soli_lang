# Configuration

Soli loads environment variables from the process, then from `.env`, and finally from `.env.{APP_ENV}` when `APP_ENV` is set. Environment-specific files override `.env`, except variables listed in `SOLI_PROTECT_ENV`.

```bash
# .env
APP_ENV=development
SOLIDB_HOST=http://localhost:6745
SOLIDB_DATABASE=myapp_development
```

Keys must match `[A-Za-z_][A-Za-z0-9_]*`. Values cannot contain `\0`, `\r`, or `\n` — entries with control characters are skipped at load time with a warning on stderr. This avoids HTTP-header-split / log-injection vectors when an env value flows downstream into responses or structured logs.

## Application Environment

| Variable | Purpose | Default |
|----------|---------|---------|
| `APP_ENV` | Selects `.env.{APP_ENV}` and marks test mode for features that need it. | unset |
| `SOLI_PROTECT_ENV` | Comma-separated variable names that `.env.{APP_ENV}` must not override. Mostly used by the test runner. | unset |

## Server And Development

| Variable | Purpose | Default |
|----------|---------|---------|
| `SOLI_REQUEST_LOG` | Enables per-request logging when set to `1` or `true`. | `false` |
| `SOLI_PREFETCH` | Controls hover prefetch injection. Set `off`, `false`, `0`, or `no` to disable. | enabled |
| `SOLI_DEFAULT_URL_HOST` | Host used by `*_url` route helpers outside an active request. | unset |
| `SOLI_DEFAULT_URL_SCHEME` | Scheme used with `SOLI_DEFAULT_URL_HOST`. | `http` |
| `SOLI_DEV_REPL_ALLOW_REMOTE` | Allows the token-protected dev error-page REPL from non-loopback clients when set to `1`, `true`, or `yes`. Requires `SOLI_DEV_REPL_SECRET` (SEC-051) — the server refuses to start otherwise. | `false` |
| `SOLI_DEV_REPL_SECRET` | Pins the `/__dev/repl` token to an explicit shared secret instead of an auto-generated UUID. Required when `SOLI_DEV_REPL_ALLOW_REMOTE=1` so the credential is never embedded in dev-mode HTML error pages. | unset |
| `SOLI_TRACE_BOOT` | Prints boot timing trace when set. | unset |

## Hardening

These knobs control how the request edge handles untrusted input. See the
[Server Hardening](/docs/builtins/hardening) page for the full story.

| Variable | Purpose | Default |
|----------|---------|---------|
| `SOLI_TRUST_PROXY` | Honors `X-Forwarded-Proto` / `X-Forwarded-Host` when set to `1`, `true`, or `yes`. Only enable when the deployment terminates these headers at a trusted proxy hop. | `false` |
| `SOLI_FORCE_SECURE_COOKIES` | Set to `1`/`true`/`yes` to add `Secure` to every session cookie regardless of detected scheme. Use when the deployment is always on TLS but the proxy doesn't forward `X-Forwarded-Proto: https` (or `enable_trust_proxy()` isn't on). Equivalent runtime call: `enable_force_secure_cookies()`. | `false` |
| `SOLI_MAX_BODY_SIZE` | Maximum buffered request body, in bytes. Requests over the cap return `413 Payload Too Large`. | `8388608` (8 MiB) |
| `SOLI_DISABLE_CSRF` | Disables the same-origin CSRF check entirely when set to `true`. For API-only deployments where no cookie session is in play. Per-route opt-out via `skip_csrf("/path")` in `config/routes.sl` is preferred — see [Routing → CSRF Protection](/docs/routing#csrf-protection). | unset |
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
| `SOLI_SESSION_DRIVER` | Session backend: `in_memory`, `disk`, `solidb`, or `solikv`. | `in_memory` |
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
| `SOLI_COVERAGE_ENABLED` | Enables the server-side coverage dump endpoint for test aggregation. | unset |

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
