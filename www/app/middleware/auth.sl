# ============================================================================
# Authentication Middleware
# ============================================================================
#
# This is a SCOPE-ONLY middleware.
#
# CHARACTERISTICS:
# ---------------
# - Marked with `# scope_only: true`
# - Does NOT run globally by default
# - Only runs when explicitly scoped using middleware("authenticate", -> { ... })
# - Use this for routes that need authentication
#
# CONFIGURATION:
# -------------
# - `# order: N` - Execution order (lower runs first, default: 100)
# - `# scope_only: true` - Required for scope-only middleware
#
# ============================================================================

# order: 20
# scope_only: true - This middleware only runs when explicitly scoped

let valid_api_key = "secret-key-123";

fn authenticate(req: Any)    let headers = req["headers"];
    let provided_key = "";

    if (has_key(headers, "X-Api-Key"))
        provided_key = headers["X-Api-Key"];
    end

    if (provided_key == "")
        if (has_key(headers, "x-api-key"))
            provided_key = headers["x-api-key"];
        end
    end

    if (provided_key == valid_api_key)
        print("[AUTH] User authenticated successfully");
        return {
            "continue": true,
            "request": req
        };
    end

    print("[AUTH] Authentication failed - invalid or missing API key");
    return {
        "continue": false,
        "response": {
            "status": 401,
            "headers": {"Content-Type": "application/json"},
            "body": json_stringify({"error": "Unauthorized", "message": "Valid API key required in X-Api-Key header"})
        }
    };
end
