# Server Hardening

Production-safe defaults for untrusted input at the request edge: trust-proxy gating for `X-Forwarded-*` headers, and a per-request body-size cap to prevent memory-exhaustion DoS.

The **checklist of production defaults vs what you still set** lives on [Production security defaults](/docs/security/defaults). This page is the function and env-var reference.

## Trust Proxy

Soli reads `X-Forwarded-Proto` and `X-Forwarded-Host` only when trust-proxy is enabled. Those headers govern:

- The `Secure` flag on the session cookie (when the request scheme is `https`)
- The host portion of `*_url` named-route helpers

**Default: OFF.** Enable only when TLS terminates at a trusted proxy that strips inbound `X-Forwarded-*` from clients before adding its own.

```soli
# config/application.sl
enable_trust_proxy

# Later, e.g. in a test harness running without a proxy:
disable_trust_proxy

if trust_proxy_enabled
  println("X-Forwarded-* headers are honored")
end
```

```bash
# .env.production
SOLI_TRUST_PROXY=1
```

Truthy values: `1`, `true`, `yes` (case-insensitive). Function calls override the env default at runtime.

## Request body limit

Every non-GET/HEAD request is capped before the body is buffered. Over the cap: `413 Payload Too Large` (from `Content-Length`, or mid-stream for chunked uploads).

**Default: 8 MiB.** Raise it for large uploads; prefer per-action checks over a high global cap.

```soli
set_max_body_size(32 * 1024 * 1024)  # 32 MiB
println("Current cap: " + str(max_body_size) + " bytes")
```

```bash
SOLI_MAX_BODY_SIZE=33554432
```

Non-numeric or negative env values are ignored.

### Aggregate in-flight limit

`SOLI_MAX_BODY_SIZE` bounds **one** request. Upload bytes are held in memory
while the request runs, so the two limits multiply: without a second bound, the
server would buffer one body per connection (`SOLI_MAX_CONNECTIONS`, 20 000 by
default) and each parsed request would then sit in the worker queue still
holding its payload.

`SOLI_MAX_INFLIGHT_BODY_BYTES` caps the **sum** of request-body bytes reserved
at once. A request is charged **as its bytes arrive**: it starts with a 64 KiB
reservation and doubles it as the body grows, up to `SOLI_MAX_BODY_SIZE`,
rather than claiming the full per-request cap up front — so a crowd of small
bodies no longer exhausts the budget. The reservation is returned when the
request ends — including on error paths — so a burst of large uploads is
refused rather than swapping the box.

**Default: 16 × the per-request cap** (128 MiB out of the box). Over the cap:
`503 Service Unavailable` with `Retry-After: 1`. Set it to `0` to disable the
budget entirely.

```bash
SOLI_MAX_INFLIGHT_BODY_BYTES=268435456   # 256 MiB across all requests
```

Raising `SOLI_MAX_BODY_SIZE` for an app that accepts big uploads raises this
default with it, so the two stay in proportion unless you set it explicitly.

### Stalled and broken bodies

`SOLI_BODY_READ_TIMEOUT_SECS` (default `60`) bounds how long a whole body may
take. `SOLI_BODY_IDLE_TIMEOUT_SECS` (default `10`) bounds the silence **between
two frames**: a body that stalls that long is answered `408 Request Timeout`
and its reservation freed. A body cut off by a transport error is a `400 Bad
Request` (it used to be reported as `413`).

## Related env

See [Configuration](/docs/getting-started/configuration) for `SOLI_CSRF_TOKENS`, `SOLI_DISABLE_CSRF`, `SOLI_FORCE_SECURE_COOKIES`, `SOLI_HTTP_MAX_RESPONSE_BYTES`, image and parallel-fan-out caps, `SOLI_MAX_UPLOAD_FILES`,
`SOLI_MAX_INFLIGHT_BODY_BYTES`, `SOLI_BODY_IDLE_TIMEOUT_SECS`, and the HTTP/2
`SOLI_H2_KEEPALIVE_SECS` / `SOLI_CONN_IDLE_TIMEOUT_SECS`.

Response-side headers: [Security Headers](/docs/builtins/security-headers).
