# config/application.sl — boot-time configuration.
#
# Loaded by `soli serve` once before `config/routes.sl`, so anything you
# set here is in effect by the time the first request is handled. Every
# knob below also has an env-var equivalent for deployment ergonomics —
# pick whichever fits your flow.

# ---------------------------------------------------------------------
# enable_trust_proxy — OFF by default. Turn it on when you deploy.
# ---------------------------------------------------------------------
# Makes the server honour `X-Forwarded-Host` / `X-Forwarded-Proto` /
# `X-Forwarded-For` from inbound requests for CSRF, redirects,
# `request.host`, per-IP rate limiting, and the cookie `Secure` flag.
#
# It is off here because an `X-Forwarded-*` header is only trustworthy
# when a proxy you control strips the client's copy and rewrites it
# with what it observed. On an app reachable directly, any client can
# send those headers, which spoofs the request authority and scheme —
# downgrading the CSRF / origin checks, flipping the cookie `Secure`
# flag, aiming `*_url` helpers at a phishing host, and handing every
# request a fresh identity so per-IP rate limits (including the login
# throttle) never trip.
#
# Behind nginx / Caddy / an ALB / fly-proxy, uncomment the line below
# (or set `SOLI_TRUST_PROXY=1`). Also name the hops you trust, so the
# headers are honoured only for requests that really came from them:
#
#   SOLI_TRUSTED_PROXIES=10.0.0.0/8,127.0.0.1,::1
#
# With that list set, a client reaching the app directly is not trusted
# even while the flag is on. Leaving it unset trusts every peer.

# enable_trust_proxy

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
# New apps also set `SOLI_CSRF_TOKENS=require` in `.env`, so a browser
# form post without `_csrf_token` / `X-CSRF-Token` is 403 even when
# Origin matches. `form_with` embeds the token. JSON bodies are not
# token-gated. Unset that env var only if you have a reason.
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
