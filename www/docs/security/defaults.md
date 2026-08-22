# Production security defaults

Soli’s MVC surface is wide. This page is the **honest list of what production does today** versus **what a public-internet app should still set**. The long-term hardening program is to collapse that gap so `soli new` + `soli serve` is safe without a wiki.

For request-edge knobs (`SOLI_TRUST_PROXY`, body size, CSRF tokens) see [Server Hardening](/docs/builtins/hardening). For response headers see [Security Headers](/docs/builtins/security-headers).

## Already on by default (production)

| Control | What it does |
|---------|----------------|
| Auto-escaped templates | `<%= %>` HTML-escapes, including non-String values |
| CSRF Origin/Referer gate | Cross-site state-changing requests without a matching origin fail |
| Security headers | Standard preset in production (nosniff, frame options, HSTS, CSP where configured) |
| SSRF on `HTTP.*` | Loopback and private ranges refused; exceptions are literal `host:port` in `SOLI_HTTP_ALLOW_HOSTS` |
| Request body cap | 8 MiB (`SOLI_MAX_BODY_SIZE`); 413 when exceeded |
| Attachment types | Default allowlist excludes `text/html`, SVG, XML; blob route sends `nosniff` + `Content-Disposition: attachment` for non-images |
| SQL TLS | Postgres/MySQL `sslmode` / `ssl-mode` via rustls; default `prefer` |
| Panic containment | A panicking handler is a 500; the worker stays up (`catch_unwind`) |
| Log redaction | Credential-looking params, binds, locals, and HTTP URLs are `[REDACTED]` |
| Jobs dashboard | 404 in production unless `SOLI_JOBS_USER`/`PASSWORD` or `SOLI_JOBS_TOKEN` is set |
| Production boot gate | `APP_ENV=production` (or `prod`) **refuses to start** without `SOLI_APP_HOSTS` (at least one hostname) and `SOLI_SESSION_SECRET` of 32+ characters. `--dev` and non-production env skip the gate |

## You still set (today)

These are **not** implied by `soli serve` without env. Treat them as required for a public host:

| Control | Why |
|---------|-----|
| `SOLI_APP_HOSTS` | Required at production boot. CSRF origin checks use this list, not a forgeable `Host` / `X-Forwarded-Host` |
| `SOLI_SESSION_SECRET` | Required at production boot (32+ chars); sealed cookies and the cookie session driver derive keys from it |
| `SOLI_CSRF_TOKENS=require` | Tokens are *verified when present*; this makes a missing token a 403 for browser form posts. **`soli new` writes this into `.env`.** Existing apps stay optional until they set it. |
| `permit(...)` / `attr_accessible` | Mass assignment is not blocked on `Model.create(params)` unless you whitelist |
| `sslmode=require` (or `verify-full`) | Default `prefer` still allows a cleartext fallback if the server offers none |
| `SOLI_TRUST_PROXY=1` | Only behind a proxy that **strips** inbound `X-Forwarded-*` then sets its own |
| Reverse-proxy TLS | `soli serve` is HTTP. Terminate TLS at Caddy/nginx/ALB |
| Rate limits on auth | `soli generate auth` includes per-IP throttling; other endpoints need `rate_limit` |

## Target (hardening program)

Not shipped as defaults yet — do not assume they already fail closed:

- ~~`SOLI_CSRF_TOKENS=require` for new apps~~ **shipped** (`soli new` `.env`)
- ~~Production boot **fails** if `SOLI_APP_HOSTS` or a short `SOLI_SESSION_SECRET` is missing~~ **shipped**
- ~~Lint on unfiltered `Model.create(params)`~~ **shipped** (`security/unfiltered-mass-assignment`)
- Stricter CSP that matches vendored htmx + Alpine
- `DATABASE_URL` examples using `sslmode=require`

## Related

- [Server Hardening](/docs/builtins/hardening)
- [Forms & CSRF](/docs/core-concepts/forms)
- [Sessions](/docs/security/sessions)
- [Authentication](/docs/security/authentication)
- [Authorization](/docs/security/authorization)
