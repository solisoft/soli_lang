# config/application.sl — boot-time configuration.
#
# Loaded by `soli serve` once before `config/routes.sl`, so anything you
# set here is in effect by the time the first request is handled. Every
# knob below also has an env-var equivalent for deployment ergonomics —
# pick whichever fits your flow.

# ---------------------------------------------------------------------
# enable_trust_proxy — opt-in, INSECURE BY DEFAULT.
# ---------------------------------------------------------------------
# When enabled, the server honours `X-Forwarded-Host` /
# `X-Forwarded-Proto` / etc. from inbound requests for CSRF, redirects,
# `request.host`, and the cookie `Secure` flag. ONLY enable this when:
#
#   1. The app sits behind a reverse proxy (nginx, Caddy, AWS ALB, ...)
#      that you control, AND
#   2. That proxy strips client-supplied `X-Forwarded-*` headers and
#      rewrites them with the values it observed itself.
#
# Without (2), an attacker can spoof request authority and scheme,
# downgrading CSRF / origin checks and triggering phishing-shaped
# redirects from `*_url` helpers. If the app is exposed directly to the
# internet (no proxy), leave this commented out.
#
# Uncomment after confirming your proxy strips inbound `X-Forwarded-*`:
#
#   enable_trust_proxy
#
# Equivalent operator-level toggle: `SOLI_TRUST_PROXY=1` in the env.

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
# (`example.test` → `localhost:20004`) but Soli is reading the raw
# `Host: localhost:20004` because `enable_trust_proxy` is off. Two
# common fixes:
#
#   - You DO have a trusted proxy that rewrites X-Forwarded-Host →
#     uncomment `enable_trust_proxy` above. CSRF will then compare the
#     forwarded host to the Origin.
#
#   - You DON'T have a proxy and the mismatch is from a specific
#     webhook / public API endpoint → opt that path out with
#     `skip_csrf`. Pattern is exact path or `/prefix/*` glob:
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

# ---------------------------------------------------------------------
# Mailer — outbound email (SMTP).
# ---------------------------------------------------------------------
# Define mailers under app/mailers/ (subclass Mailer) and send with
# `UserMailer.welcome(user).deliver_later`. Configure delivery here:
#
#   # Mailer.configure({
#   #   "delivery_method": "smtp",      # "smtp" | "test" | "logger"
#   #   "host": getenv("SMTP_HOST"),
#   #   "port": 587,                     # 465 = implicit TLS, 587 = STARTTLS
#   #   "user": getenv("SMTP_USER"),
#   #   "pass": getenv("SMTP_PASS"),
#   #   "tls": "auto",                   # "auto" | "starttls" | "tls" | "none"
#   #   "from": "Acme <noreply@example.com>"
#   # })
#
# In tests use `"delivery_method": "test"` and assert on
# `Mailer.deliveries()`. Env-var equivalents: SOLI_SMTP_HOST / _PORT /
# _USER / _PASS / _TLS / _FROM and SOLI_MAIL_DELIVERY_METHOD.
