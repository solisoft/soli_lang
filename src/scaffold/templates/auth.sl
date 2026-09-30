# ============================================================================
# Authentication Middleware (Scope-Only)
# ============================================================================
#
# This middleware checks for authentication.
# It only runs when explicitly scoped to routes.
#
# Usage in routes.sl:
#   middleware("authenticate", -> {
#       get("/admin", "admin#index")
#       get("/admin/settings", "admin#settings")
#   })
#
# Configuration:
# - `# order: N` - Execution order (lower runs first), scoped and global
#   middleware sorted together
# - `# scope_only: true` - Only runs when explicitly scoped
#
# Return `req` to continue, or a response (`render_json(...)`,
# `redirect(...)`, `halt(status, message)`) to stop. A `def` whose name starts
# with `_` is a helper, not a middleware.
#
# ============================================================================

# order: 20
# scope_only: true

def authenticate(req)
  # TODO: Replace with your authentication logic
  # For example, verify JWT token, check session, etc.
  api_key = req["headers"]["x-api-key"]
  return render_json({"error": "Unauthorized", "message": "Authentication required"}, 401) if api_key.blank?

  req
end
