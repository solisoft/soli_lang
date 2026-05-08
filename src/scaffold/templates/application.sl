# config/application.sl — boot-time configuration.
#
# Loaded by `soli serve` before `config/routes.sl`, so it's the place to
# flip framework-wide gates on or off at startup.
#
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
