// ============================================================================
// Authentication Middleware
// ============================================================================
//
// This is a SCOPE-ONLY middleware.
//
// CHARACTERISTICS:
// ---------------
// - Marked with `// scope_only: true`
// - Does NOT run globally by default
// - Only runs when explicitly scoped using middleware("authenticate", -> { ... })
// - Use this for routes that need authentication
//
// CONFIGURATION:
// -------------
// - `// order: N` - Execution order (lower runs first, default: 100)
// - `// scope_only: true` - Required for scope-only middleware
//
// ============================================================================

// order: 20
// scope_only: true - This middleware only runs when explicitly scoped

// The key comes from the environment, never from source.
//
// This file used to carry `let valid_api_key = "secret-key-123"`. It is example
// code, but example code is copied — and a credential in a repository is a
// credential in every fork, every container image and every git history that
// ever contained it. An unset variable disables the middleware rather than
// falling back to a default everyone knows.
let valid_api_key = getenv("API_KEY").to_s;

def authenticate(req: Any) -> Any {
    let headers = req["headers"];
    let provided_key = "";

    if (has_key(headers, "X-Api-Key")) {
        provided_key = headers["X-Api-Key"];
    }

    if (provided_key.blank?) {
        if (has_key(headers, "x-api-key")) {
            provided_key = headers["x-api-key"];
        }
    }

    // Constant-time comparison: `==` on strings returns as soon as two bytes
    // differ, which leaks the key one byte at a time to anyone who can time the
    // response.
    if (!valid_api_key.blank? && secure_compare(provided_key, valid_api_key)) {
        print("[AUTH] User authenticated successfully");
        return {
            "continue": true,
            "request": req
        };
    }

    print("[AUTH] Authentication failed - invalid or missing API key");
    return {
        "continue": false,
        "response": {
            "status": 401,
            "headers": {"Content-Type": "application/json"},
            "body": json_stringify({"error": "Unauthorized", "message": "Valid API key required in X-Api-Key header"})
        }
    };
}
