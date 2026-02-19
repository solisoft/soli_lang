# ============================================================================
# CORS (Cross-Origin Resource Sharing) Middleware
# ============================================================================
#
# This is a GLOBAL-ONLY middleware.
#
# CHARACTERISTICS:
# ---------------
# - Marked with `# global_only: true`
# - Cannot be scoped to specific routes
# - Always runs for ALL requests
# - Adds CORS headers to responses
#
# CONFIGURATION:
# -------------
# - `# order: N` - Execution order (lower runs first, default: 100)
# - `# global_only: true` - Required for global-only middleware
#
# ============================================================================

# order: 5
# global_only: false - This middleware cannot be scoped

fn add_cors_headers(req: Any)    # For OPTIONS preflight requests, return immediately with CORS headers
    if (req["method"] == "OPTIONS") {
        return {
            "continue": false,
            "response": {
                "status": 204,
                "headers": {
                    "Access-Control-Allow-Origin": "*",
                    "Access-Control-Allow-Methods": "GET, POST, PUT, DELETE, OPTIONS",
                    "Access-Control-Allow-Headers": "Content-Type, X-Api-Key",
                    "Access-Control-Max-Age": "86400"
                },
                "body": ""
            }
        };
    }

    # For other requests, just continue (CORS headers will be added by the response)
    return {
        "continue": true,
        "request": req
    };
end
