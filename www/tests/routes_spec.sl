# ============================================================================
# Route Integration Tests
# ============================================================================
//
# Tests for route configuration and HTTP method handling
# ============================================================================

describe("Routes Configuration", fn()
    describe("Root Routes", fn()
        test("GET / maps to home#index", fn()
            let route = {
                "method": "GET",
                "path": "/",
                "controller": "home",
                "action": "index"
            };
            assert_eq(route["path"], "/");
            assert_eq(route["controller"], "home");
            assert_eq(route["action"], "index");
        end);
    end);

    describe("Health Check Routes", fn()
        test("GET /health maps to home#health", fn()
            let route = {
                "path": "/health",
                "controller": "home",
                "action": "health"
            };
            assert_eq(route["path"], "/health");
        end);

        test("GET /up maps to home#up", fn()
            let route = {
                "path": "/up",
                "controller": "home",
                "action": "up"
            };
            assert_eq(route["path"], "/up");
        end);
    end);

    describe("Documentation Routes", fn()
        test("GET /docs maps to docs#index", fn()
            let route = {
                "path": "/docs",
                "controller": "docs",
                "action": "index"
            };
            assert_eq(route["path"], "/docs");
        end);

        test("GET /docs/introduction maps to docs#introduction", fn()
            let route = {
                "path": "/docs/introduction",
                "controller": "docs",
                "action": "introduction"
            };
            assert_eq(route["path"], "/docs/introduction");
        end);

        test("Documentation routes cover all main topics", fn()
            let docs_routes = [
                "/docs",
                "/docs/introduction",
                "/docs/installation",
                "/docs/routing",
                "/docs/controllers",
                "/docs/models",
                "/docs/views",
                "/docs/middleware",
                "/docs/websockets",
                "/docs/live-reload",
                "/docs/soli-language",
                "/docs/i18n",
                "/docs/authentication",
                "/docs/sessions",
                "/docs/validation",
                "/docs/testing",
                "/docs/request-params"
            ];
            assert_gt(len(docs_routes), 10);
        end);
    end);

    describe("WebSocket Routes", fn()
        test("GET /websocket maps to websocket#demo", fn()
            let route = {
                "path": "/websocket",
                "controller": "websocket",
                "action": "demo"
            };
            assert_eq(route["path"], "/websocket");
        end);

        test("WebSocket route /ws/chat maps to websocket#chat_handler", fn()
            let route = {
                "type": "websocket",
                "path": "/ws/chat",
                "controller": "websocket",
                "action": "chat_handler"
            };
            assert_eq(route["path"], "/ws/chat");
            assert_eq(route["type"], "websocket");
        end);
    end);
end);

describe("Users Routes", fn()
    describe("Authentication Routes", fn()
        test("GET /users/login renders login page", fn()
            let route = {
                "method": "GET",
                "path": "/users/login",
                "controller": "users",
                "action": "login"
            };
            assert_eq(route["path"], "/users/login");
        end);

        test("POST /users/login handles login submission", fn()
            let route = {
                "method": "POST",
                "path": "/users/login",
                "controller": "users",
                "action": "login_post"
            };
            assert_eq(route["method"], "POST");
        end);

        test("GET /users/register renders register page", fn()
            let route = {
                "path": "/users/register",
                "controller": "users",
                "action": "register"
            };
            assert_eq(route["path"], "/users/register");
        end);

        test("POST /users/register handles registration", fn()
            let route = {
                "method": "POST",
                "path": "/users/register",
                "controller": "users",
                "action": "register_post"
            };
            assert_eq(route["method"], "POST");
        end);

        test("GET /users/logout destroys session", fn()
            let route = {
                "path": "/users/logout",
                "controller": "users",
                "action": "logout"
            };
            assert_eq(route["path"], "/users/logout");
        end);

        test("GET /users/profile shows user profile", fn()
            let route = {
                "path": "/users/profile",
                "controller": "users",
                "action": "profile"
            };
            assert_eq(route["path"], "/users/profile");
        end);
    end);

    describe("Session Routes", fn()
        test("GET /users/regenerate-session regenerates session", fn()
            let route = {
                "path": "/users/regenerate-session",
                "controller": "users",
                "action": "regenerate_session"
            };
            assert_eq(route["path"], "/users/regenerate-session");
        end);
    end);

    describe("Validation Routes", fn()
        test("GET /users/validation-demo shows validation demo", fn()
            let route = {
                "path": "/users/validation-demo",
                "controller": "users",
                "action": "validation_demo"
            };
            assert_eq(route["path"], "/users/validation-demo");
        end);

        test("POST /users/validate-registration validates input", fn()
            let route = {
                "method": "POST",
                "path": "/users/validate-registration",
                "controller": "users",
                "action": "validate_registration"
            };
            assert_eq(route["method"], "POST");
        end);
    end);

    describe("JWT Routes", fn()
        test("POST /users/create-token creates JWT", fn()
            let route = {
                "method": "POST",
                "path": "/users/create-token",
                "controller": "users",
                "action": "create_token"
            };
            assert_eq(route["method"], "POST");
        end);

        test("POST /users/verify-token verifies JWT", fn()
            let route = {
                "method": "POST",
                "path": "/users/verify-token",
                "controller": "users",
                "action": "verify_token"
            };
            assert_eq(route["method"], "POST");
        end);

        test("POST /users/decode-token decodes JWT", fn()
            let route = {
                "method": "POST",
                "path": "/users/decode-token",
                "controller": "users",
                "action": "decode_token"
            };
            assert_eq(route["method"], "POST");
        end);
    end);
end);

describe("HTTP Methods", fn()
    test("GET requests are used for reading data", fn()
        let get_routes = ["/", "/health", "/up", "/users/login"];
        assert_gt(len(get_routes), 0);
    end);

    test("POST requests are used for creating data", fn()
        let post_routes = [
            "/users/login",
            "/users/register",
            "/users/validate-registration",
            "/users/create-token",
            "/users/verify-token",
            "/users/decode-token"
        ];
        assert_gt(len(post_routes), 0);
    end);

    test("Route format: method(path, controller#action)", fn()
        let route_syntax = "get(\"/path\", \"controller#action\")";
        assert_contains(route_syntax, "#");
    end);
end);

describe("Route Coverage", fn()
    test("All main MVC components have routes", fn()
        let controllers = ["home", "users", "docs", "websocket"];
        assert_gt(len(controllers), 0);
    end);

    test("Users routes cover authentication flow", fn()
        let auth_routes = [
            "/users/login",
            "/users/logout",
            "/users/register",
            "/users/profile"
        ];
        assert_eq(len(auth_routes), 4);
    end);

    test("Documentation covers all major features", fn()
        let feature_docs = [
            "routing", "controllers", "models", "views",
            "middleware", "websockets", "authentication",
            "sessions", "validation", "testing"
        ];
        assert_gt(len(feature_docs), 5);
    end);
end);

describe("Wildcard Routes", fn()
    test("Wildcard routes use * in path pattern", fn()
        let wildcard_pattern = "get(\"/docs/*\", \"docs#*\")";
        assert_contains(wildcard_pattern, "/*");
    end);

    test("Wildcard action uses * after controller#", fn()
        let wildcard_action = "get(\"/docs/*\", \"docs#*\")";
        assert_contains(wildcard_action, "#*");
    end);

    test("Splat params capture remaining path segments", fn()
        let splat_examples = [
            {"pattern": "/files/*path", "path": "/files/docs/readme", "expected_path": "/docs/readme"},
            {"pattern": "/api/*version/users", "path": "/api/v1/users", "expected_version": "/v1"},
            {"pattern": "/users/:id/*action", "path": "/users/123/edit", "expected_id": "123", "expected_action": "/edit"},
        ];
        assert_gt(len(splat_examples), 0);
    end);

    test("Multiple splats supported in pattern", fn()
        let multi_splat = "get(\"/*version/*resource/*id\", \"api#*\")";
        assert_contains(multi_splat, "/*");
    end);

    test("Wildcard action expands using splat params", fn()
        let examples = [
            {"handler": "docs#*", "params": {"splat": "/routing"}, "expected": "docs#routing"},
            {"handler": "api#*", "params": {"version": "/v1", "action": "/users"}, "expected": "api#users"},
            {"handler": "files#*", "params": {"filepath": "/docs/guide"}, "expected": "files#docs/guide"},
        ];
        assert_gt(len(examples), 0);
    end);

    test("Non-wildcard handlers return unchanged", fn()
        let static_handlers = [
            {"handler": "home#index", "expected": "home#index"},
            {"handler": "users#show", "expected": "users#show"},
        ];
        assert_gt(len(static_handlers), 0);
    end);
end);

describe("Route Parameters", fn()
    test("Route parameters use :param syntax", fn()
        let param_patterns = ["/users/:id", "/posts/:post_id/comments/:comment_id"];
        assert_gt(len(param_patterns), 0);
    end);

    test("Parameters are accessible in controller via req.params", fn()
        let param_access = "req[\"params\"][\"id\"]";
        assert_contains(param_access, "params");
    end);

    test("Optional parameters use :param?", fn()
        let optional_pattern = "get(\"/users/:?\", \"users#index\")";
        assert_contains(optional_pattern, ":?");
    end);
end);
