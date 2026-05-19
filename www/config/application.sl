# config/application.sl — boot-time configuration for the soli.solisoft.net
# docs site.
#
# Loaded by `soli serve` before `config/routes.sl`, so it's the place to
# flip framework-wide gates on or off at startup.
#
# ---------------------------------------------------------------------
# enable_trust_proxy
# ---------------------------------------------------------------------
# The docs site sits behind a reverse proxy that terminates TLS for
# https://soli.solisoft.net (and the local https://soli.solisoft.test
# dev variant). Without trust_proxy, Soli sees `Host: soli.solisoft.net`
# but doesn't know the original request was HTTPS, so:
#
#   • request.scheme reports "http", *_url helpers emit http://…
#   • Set-Cookie's `Secure` flag is dropped
#   • CSRF Origin checks compare against the wrong authority and
#     reject every HTMx-driven POST/PATCH/DELETE with 403 Forbidden
#
# Enabling trust_proxy makes Soli honour the `X-Forwarded-Host` /
# `X-Forwarded-Proto` headers our proxy sets. The proxy MUST strip
# client-supplied `X-Forwarded-*` headers before injecting its own;
# otherwise a malicious client can spoof the request authority.
enable_trust_proxy
