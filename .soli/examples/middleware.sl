// ============================================================================
// Middleware Examples for AI/LLM Code Generation
// ============================================================================
//
// AI AGENT GUIDE:
// ---------------
// This file documents middleware conventions for the Soli MVC framework.
//
// MIDDLEWARE TYPES:
// 1. GLOBAL ONLY - Runs for ALL routes automatically
//    - Marked with: // global_only: true
//    - Example: CORS, logging, security headers
//
// 2. SCOPE ONLY - Runs only when explicitly scoped
//    - Marked with: // scope_only: true
//    - Used with: middleware("name", -> { routes })
//    - Example: Authentication, authorization
//
// 3. REGULAR - Can be global OR scoped
//    - No special marker
//    - Runs globally by default, can be scoped
//
// MIDDLEWARE EXECUTION:
// - Order determined by // order: N comment (lower = runs first)
// - Each middleware receives req: Any
// - Return {"continue": boolean, "request"?: dict, "response"?: dict}
// - If continue=false, chain stops (or returns early response)
// - If request is provided, modified request passed to next handler
// - If response is provided, immediately returns that response
//
// TEMPLATE FOR AI GENERATION:
// ----------------------------
// // order: N
// // global_only: true (or // scope_only: true)
//
// fn middleware_name(req: Any) -> Any {
//     // Your logic here
//     
//     return {
//         "continue": true,
//         "request": req  // optional modification
//     };
// }
//
// ============================================================================

// ============================================================================
// EXAMPLE 1: Global-only Middleware (CORS)
// ============================================================================
//
// CHARACTERISTICS:
// - Marked with // global_only: true
// - Automatically runs for ALL requests
// - No need to reference in routes.sl
//
// USAGE:
// - Set CORS headers on all responses
// - Handle preflight OPTIONS requests
//
// ============================================================================

// order: 10 - Runs early in the chain
// global_only: true - Automatically applies to all routes

fn cors(req: Any) -> Any {
    let headers = req["headers"];
    let method = req["method"];
    
    // Handle preflight OPTIONS request
    if (method == "OPTIONS") {
        let origin = "*";
        if (has_key(headers, "Origin")) {
            origin = headers["Origin"];
        }
        
        return {
            "continue": false,
            "response": {
                "status": 204,
                "headers": {
                    "Access-Control-Allow-Origin": origin,
                    "Access-Control-Allow-Methods": "GET, POST, PUT, DELETE, PATCH, OPTIONS",
                    "Access-Control-Allow-Headers": "Content-Type, Authorization, X-Api-Key",
                    "Access-Control-Allow-Credentials": "true",
                    "Access-Control-Max-Age": "86400"
                },
                "body": ""
            }
        };
    }
    
    // Add CORS headers to regular requests
    let origin = "*";
    if (has_key(headers, "Origin")) {
        origin = headers["Origin"];
    }
    
    // Modify request to include CORS headers for downstream handlers
    let modified_req = req;
    modified_req["cors_headers"] = {
        "Access-Control-Allow-Origin": origin,
        "Access-Control-Allow-Credentials": "true"
    };
    
    return {
        "continue": true,
        "request": modified_req
    };
}

// ============================================================================
// EXAMPLE 2: Scope-only Middleware (Authentication)
// ============================================================================
//
// CHARACTERISTICS:
// - Marked with // scope_only: true
// - Does NOT run automatically
// - Must be explicitly scoped in routes.sl:
//
//   middleware("authenticate", -> {
//       get("/admin", "admin#index");
//       get("/dashboard", "dashboard#index");
//   });
//
// USAGE:
// - Protect sensitive routes
// - Require API key or session authentication
//
// ============================================================================

// order: 20 - Runs after CORS (higher order number)
// scope_only: true - Only runs when explicitly scoped

fn authenticate(req: Any) -> Any {
    let headers = req["headers"];
    let path = req["path"];
    
    // Check for API key in header
    let api_key = "";
    if (has_key(headers, "X-Api-Key")) {
        api_key = headers["X-Api-Key"];
    } else if (has_key(headers, "x-api-key")) {
        api_key = headers["x-api-key"];
    }
    
    // Validate API key (in real app, check database)
    let valid_key = "secret-api-key-123";
    
    if (api_key == valid_key) {
        print("[AUTH] Request authenticated for path: ", path);
        
        // Add user info to request for downstream handlers
        let modified_req = req;
        modified_req["user"] = {"id": "user_001", "role": "admin"};
        
        return {
            "continue": true,
            "request": modified_req
        };
    }
    
    // Authentication failed - return 401
    print("[AUTH] Authentication failed for path: ", path);
    
    return {
        "continue": false,
        "response": {
            "status": 401,
            "headers": {"Content-Type": "application/json"},
            "body": json_stringify({
                "error": "Unauthorized",
                "message": "Valid API key required in X-Api-Key header"
            })
        }
    };
}

// ============================================================================
// EXAMPLE 3: Scope-only Middleware (Admin Authorization)
// ============================================================================
//
// USAGE:
// - Check if authenticated user has admin role
// - Scoped within authenticated routes
//
// ============================================================================

// order: 30
// scope_only: true

fn require_admin(req: Any) -> Any {
    let user = req["user"];
    
    if (user == null || user["role"] != "admin") {
        return {
            "continue": false,
            "response": {
                "status": 403,
                "headers": {"Content-Type": "application/json"},
                "body": json_stringify({
                    "error": "Forbidden",
                    "message": "Admin access required"
                })
            }
        };
    }
    
    return {"continue": true, "request": req};
}

// ============================================================================
// EXAMPLE 4: Regular Middleware (Request Logging)
// ============================================================================
//
// CHARACTERISTICS:
// - No special markers
// - Runs globally by default
// - Can also be scoped if needed
//
// USAGE:
// - Log all requests (or scoped requests)
// - Add request timing
// - Add request ID for tracing
//
// ============================================================================

// order: 100 - Run last (after authentication, etc.)
// No global_only or scope_only = runs globally by default

fn logger(req: Any) -> Any {
    let method = req["method"];
    let path = req["path"];
    let start_time = clock();
    
    // Log request
    print("[LOG] ", method, " ", path);
    
    // Add request ID for tracing
    let modified_req = req;
    modified_req["request_id"] = generate_uuid();
    modified_req["start_time"] = start_time;
    
    return {
        "continue": true,
        "request": modified_req
    };
}

// ============================================================================
// EXAMPLE 5: Response Modifier Middleware
// ============================================================================
//
// This middleware modifies responses after the handler completes.
// Note: This requires response modification support in the framework.
//
// ============================================================================

// order: 50
// global_only: true

fn response_minifier(req: Any) -> Any {
    // In framework with response modification:
    // - Could minify HTML responses
    // - Could add compression headers
    // - Could track response times
    
    return {"continue": true, "request": req};
}

// ============================================================================
// HOW TO USE SCOPE-ONLY MIDDLEWARE IN routes.sl:
// ============================================================================
//
// // Load middleware (already auto-loaded from app/middleware/)
// // Reference by function name (without .sl extension)
// 
// // Scoped routes with authentication
// middleware("authenticate", -> {
//     get("/api/users", "api#users");
//     get("/api/settings", "api#settings");
// });
// 
// // Nested scoping - auth + admin required
// middleware("authenticate", -> {
//     middleware("require_admin", -> {
//         get("/admin", "admin#index");
//         post("/admin/users", "admin#create_user");
//     });
// });
//
// ============================================================================
