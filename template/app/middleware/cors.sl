// ============================================================================
// CORS (Cross-Origin Resource Sharing) Middleware
// ============================================================================
//
// This is a GLOBAL-ONLY middleware.
//
// CHARACTERISTICS:
// ---------------
// - Marked with `// global_only: true`
// - Cannot be scoped to specific routes
// - Always runs for ALL requests
// - Adds CORS headers to responses
//
// CONFIGURATION:
// -------------
// - `// order: N` - Execution order (lower runs first, default: 100)
// - `// global_only: true` - Required for global-only middleware
//
// ============================================================================

// order: 5
// global_only: false - This middleware cannot be scoped

def add_cors_headers(req: Any) -> Any {
    // For OPTIONS preflight requests, return immediately with CORS headers
    if (req["method"] == "OPTIONS") {
        // Name your own origin here, or in CORS_ORIGIN.
        //
        // This used to be `*`, meaning every website could read the responses
        // of every route. That is only tenable for an API that is genuinely
        // public and carries no credentials, and example code gets copied into
        // apps that are neither. `null` matches no origin, so an unconfigured
        // copy allows nothing rather than everything. The framework's own
        // `cors()` helper in config/routes.sl is the better answer: it checks
        // the origin against a list you declare.
        let allowed_origin = "null";
        allowed_origin = getenv("CORS_ORIGIN").to_s unless getenv("CORS_ORIGIN").to_s.blank?

        return {
            "continue": false,
            "response": {
                "status": 204,
                "headers": {
                    "Access-Control-Allow-Origin": allowed_origin,
                    "Access-Control-Allow-Methods": "GET, POST, PUT, DELETE, OPTIONS",
                    "Access-Control-Allow-Headers": "Content-Type, X-Api-Key",
                    "Access-Control-Max-Age": "86400"
                },
                "body": ""
            }
        };
    }

    // For other requests, just continue (CORS headers will be added by the response)
    return {
        "continue": true,
        "request": req
    };
}
