# config/application.sl — boot-time configuration.
#
# Loaded by `soli serve` once before `config/routes.sl`, so anything you
# set here is in effect by the time the first request is handled. Every
# knob below also has an env-var equivalent for deployment ergonomics —
# pick whichever fits your flow.

# ---------------------------------------------------------------------
# enable_trust_proxy — ON BY DEFAULT in scaffolded apps.
# ---------------------------------------------------------------------
# Makes the server honour `X-Forwarded-Host` / `X-Forwarded-Proto` /
# etc. from inbound requests for CSRF, redirects, `request.host`, and
# the cookie `Secure` flag. This is the right default for the typical
# deployment shape (app behind nginx / Caddy / an ALB / fly-proxy).
#
# SECURITY: only safe when the proxy in front of the app strips
# client-supplied `X-Forwarded-*` headers and rewrites them with the
# values it observed itself. Without that, a remote client can spoof
# the request authority and scheme — downgrading CSRF / origin checks
# and producing phishing-shaped redirects from `*_url` helpers.
#
# If you're exposing the app DIRECTLY to the internet with no proxy in
# front, comment the next line out (or set `SOLI_TRUST_PROXY=0` in the
# env) so spoofed `X-Forwarded-*` headers can't be trusted.

enable_trust_proxy

# ---------------------------------------------------------------------
# CSRF / same-origin policy.
# ---------------------------------------------------------------------
# State-changing requests (POST/PUT/PATCH/DELETE) are gated by a
# same-origin check: the `Origin` (or `Referer`) header must match the
# request authority. A failure looks like:
#
#   CSRF check failed: Origin example.test does not match request
#   authority localhost:20004
#
# This usually means the app sits behind a proxy/local hostname
# (`example.test` → `localhost:20004`) and the proxy is NOT sending
# `X-Forwarded-Host`, so Soli falls back to the raw `Host` header even
# though `enable_trust_proxy` is on. Two common fixes:
#
#   - You have a proxy → configure it to set `X-Forwarded-Host` (and
#     `X-Forwarded-Proto`) to the public-facing hostname. CSRF will
#     then compare that forwarded host to the Origin.
#
#   - The mismatch is from a specific webhook / public API endpoint →
#     opt that path out with `skip_csrf`. Pattern is exact path or
#     `/prefix/*` glob:
#
#       # skip_csrf("/webhooks/stripe")    # exact path
#       # skip_csrf("/api/*")              # everything under /api/
#
# Operator-level kill switch for API-only deployments where no
# cookie session is ever in play:  `SOLI_DISABLE_CSRF=true` in the env.
# Don't reach for this on a cookie-authenticated app — it disables the
# session-replay defence entirely.

# ---------------------------------------------------------------------
# set_max_body_size — request body cap.
# ---------------------------------------------------------------------
# Default is 8 MiB. Raise here if you have routes that accept large
# uploads, but prefer a per-action override inside the handler over a
# permanently large global cap.
#
#   # set_max_body_size(32 * 1024 * 1024)   # 32 MiB
#
# Equivalent env var: `SOLI_MAX_BODY_SIZE=33554432`.

# ---------------------------------------------------------------------
# session_configure — session storage backend.
# ---------------------------------------------------------------------
# Default is `in_memory` (fast, lost on restart). Switch to `disk`,
# `solidb`, or `solikv` for persistence across restarts and across
# worker processes.
#
#   # session_configure({
#   #     "driver": "solikv",
#   #     "solikv_host": "localhost",
#   #     "solikv_port": 6380,
#   # })
#
# Env-var equivalents: `SOLI_SESSION_DRIVER`, `SOLI_SESSION_TTL`, plus
# the per-backend `SOLI_SOLIDB_*` / `SOLI_SOLIKV_*` variables. See the
# Session Storage section in CLAUDE.md / the docs site.

# ---------------------------------------------------------------------
# Security headers (CSP, HSTS, clickjacking…).
# ---------------------------------------------------------------------
# `--dev` ships a relaxed CSP so the live-reload SSE works; production
# (`--no-dev`) ships sensible defaults (X-Frame-Options: SAMEORIGIN,
# X-Content-Type-Options: nosniff, etc.). Tighten further from here:
#
#   # set_csp("default-src 'self'; script-src 'self' 'nonce-{nonce}'")
#   # set_hsts(31536000, include_subdomains: true, preload: false)
#   # prevent_clickjacking()       # X-Frame-Options: DENY
#   # set_referrer_policy("strict-origin-when-cross-origin")
#
# `enable_security_headers` / `disable_security_headers` toggle the
# whole bundle.
