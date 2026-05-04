# App-level startup configuration. Runs once before routes.sl, so anything
# you set here is in effect by the time the first request is handled.
#
# Most knobs also have an env-var equivalent for deployment ergonomics —
# see /docs/getting-started/configuration. Pick whichever fits your flow:
# function calls if you want the config in code, env vars if you want a
# single image to behave differently per deployment.

# Honor X-Forwarded-Proto / X-Forwarded-Host only when terminating these
# headers at a trusted proxy hop (Caddy, nginx, ALB, etc.).
# enable_trust_proxy()

# Raise the default 8 MiB body cap for routes that accept large uploads.
# Prefer per-action caps inside the handler over a high global cap.
# set_max_body_size(32 * 1024 * 1024)
