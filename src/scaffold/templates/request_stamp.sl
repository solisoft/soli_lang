# ============================================================================
# Request Stamp Middleware (Global)
# ============================================================================
#
# This middleware runs for ALL requests, before any scoped middleware with a
# higher order. It stamps the request with the time it arrived; the
# controller reads it as `req["started_at"]`.
#
# CORS headers are declared in config/routes.sl with `cors("/api/*", {...})`,
# not in a middleware.
#
# Configuration:
# - `# order: N` - Execution order (lower runs first)
# - `# global_only: true` - Runs for all requests, cannot be scoped
#
# ============================================================================

# order: 5
# global_only: true

def stamp_request(req)
  req["started_at"] = DateTime.utc.to_unix
  req
end
