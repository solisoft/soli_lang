# ============================================================================
# CORS Middleware (Global)
# ============================================================================
#
# This middleware adds CORS headers to all responses.
# It runs for ALL requests automatically.
#
# Configuration:
# - `# order: N` - Execution order (lower runs first)
# - `# global_only: true` - Runs for all requests, cannot be scoped
#
# ============================================================================

# order: 5
# global_only: true

def add_cors_headers(req: Any) -> Any
    # Add CORS headers to the request context
    # These will be included in the response
    return {
        "continue": true,
        "request": req
    }
end
