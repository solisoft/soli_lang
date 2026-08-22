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

## Related env

See [Configuration](/docs/getting-started/configuration) for `SOLI_CSRF_TOKENS`, `SOLI_DISABLE_CSRF`, `SOLI_FORCE_SECURE_COOKIES`, `SOLI_HTTP_MAX_RESPONSE_BYTES`, image and parallel-fan-out caps, and `SOLI_MAX_UPLOAD_FILES`.

Response-side headers: [Security Headers](/docs/builtins/security-headers).
