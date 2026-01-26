// ============================================================================
// Controller Integration Tests
// ============================================================================
// Rails-like E2E testing for controllers using the test server
// ============================================================================
//
// These tests use the test server infrastructure to make real HTTP requests
// to controllers and verify responses, sessions, and view assigns.
// ============================================================================

describe("Test Server Infrastructure", fn() {
    test("test server starts successfully", fn() {
        let running = test_server_running();
        assert(running);
    });

    test("test server has valid URL", fn() {
        let url = test_server_url();
        assert_not_null(url);
        assert_gt(len(url), 0);
    });
});

describe("Request Helper Functions", fn() {
    test("get() makes GET request", fn() {
        let response = get("/up");
        assert_eq(res_status(response), 200);
    });

    test("post() makes POST request", fn() {
        let response = post("/login", "{\"email\":\"test\",\"password\":\"test\"}");
        assert_not_null(response);
    });

    test("put() makes PUT request", fn() {
        let response = put("/posts/1", "{\"title\":\"Updated\"}");
        assert_not_null(response);
    });

    test("patch() makes PATCH request", fn() {
        let response = patch("/posts/1", "{\"title\":\"Patched\"}");
        assert_not_null(response);
    });

    test("delete() makes DELETE request", fn() {
        let response = delete("/posts/1");
        assert_not_null(response);
    });

    test("head() makes HEAD request", fn() {
        let response = head("/up");
        assert_eq(res_status(response), 200);
    });

    test("request() with custom method", fn() {
        let response = request("OPTIONS", "/up");
        assert_eq(res_status(response), 200);
    });

    test("set_header() adds custom header", fn() {
        set_header("X-Custom-Header", "test-value");
        assert_not_null(test_server_url());
    });

    test("clear_headers() removes custom headers", fn() {
        set_header("X-Test", "value");
        clear_headers();
        assert_not_null(test_server_url());
    });
});

describe("Response Helper Functions", fn() {
    test("res_status() extracts status code", fn() {
        let response = get("/up");
        let status = res_status(response);
        assert_eq(status, 200);
    });

    test("res_body() extracts response body", fn() {
        let response = get("/up");
        let body = res_body(response);
        assert_eq(body, "UP");
    });

    test("res_json() parses JSON response", fn() {
        let response = get("/health");
        let json = res_json(response);
        assert_not_null(json);
    });

    test("res_header() extracts header value", fn() {
        let response = get("/health");
        let content_type = res_header(response, "Content-Type");
        assert_not_null(content_type);
    });

    test("res_headers() returns all headers", fn() {
        let response = get("/health");
        let headers = res_headers(response);
        assert_not_null(headers);
    });

    test("res_redirect() detects redirect", fn() {
        let response = get("/docs");
        assert(res_redirect(response));
    });

    test("res_location() extracts Location header", fn() {
        let response = get("/docs");
        let location = res_location(response);
        assert_eq(location, "/docs.html");
    });

    test("res_ok() checks 2xx status", fn() {
        let response = get("/up");
        assert(res_ok(response));
    });

    test("res_client_error() checks 4xx status", fn() {
        let response = get("/nonexistent");
        assert(res_client_error(response));
    });

    test("res_not_found() checks 404 status", fn() {
        let response = get("/this-does-not-exist");
        assert(res_not_found(response));
    });

    test("res_unauthorized() checks 401 status", fn() {
        let response = get("/status/401");
        assert(res_unauthorized(response));
    });

    test("res_forbidden() checks 403 status", fn() {
        let response = get("/status/403");
        assert(res_forbidden(response));
    });

    test("res_unprocessable() checks 422 status", fn() {
        let response = get("/status/422");
        assert(res_unprocessable(response));
    });

    test("res_server_error() checks 5xx status", fn() {
        let response = get("/status/500");
        assert(res_server_error(response));
    });
});

describe("Session Helper Functions", fn() {
    before_each(fn() {
        as_guest();
    });

    test("as_guest() clears authentication", fn() {
        as_guest();
        assert(signed_out);
    });

    test("as_user() sets user ID", fn() {
        as_user(123);
        assert(signed_in);
    });

    test("as_admin() sets admin role", fn() {
        as_admin();
        assert(signed_in);
    });

    test("with_session() sets custom session data", fn() {
        let session = hash();
        session["user_id"] = 42;
        session["role"] = "editor";
        with_session(session);
        assert(signed_in);
    });

    test("with_token() sets Bearer token", fn() {
        with_token("test-jwt-token");
        assert_not_null(test_server_url());
    });

    test("clear_authorization() removes token", fn() {
        with_token("test-token");
        clear_authorization();
        assert_not_null(test_server_url());
    });

    test("signed_in() checks authentication", fn() {
        as_guest();
        assert(signed_out);
        as_user(1);
        assert(signed_in);
    });

    test("signed_out() checks not authenticated", fn() {
        as_guest();
        assert(signed_out);
    });

    test("create_session() creates session cookie", fn() {
        let session_id = create_session(1);
        assert_not_null(session_id);
    });

    test("destroy_session() clears session", fn() {
        create_session(1);
        destroy_session();
        assert(signed_out);
    });

    test("set_cookie() adds cookie", fn() {
        set_cookie("test_cookie", "test_value");
        assert_not_null(test_server_url());
    });

    test("clear_cookies() removes cookies", fn() {
        set_cookie("test", "value");
        clear_cookies();
        assert_not_null(test_server_url());
    });
});

describe("View Assigns Helper Functions", fn() {
    test("assigns() returns assigns hash", fn() {
        let result = assigns();
        assert_not_null(result);
    });

    test("assign() gets specific assign", fn() {
        let value = assign("nonexistent");
        assert_null(value);
    });

    test("have_assign() checks if assign exists", fn() {
        assert_not(have_assign("nonexistent"));
    });

    test("assert_assigns() passes for existing key", fn() {
        assert_assigns("nonexistent");
    });

    test("flash() returns flash messages", fn() {
        let flash_data = flash();
        assert_not_null(flash_data);
    });

    test("flash.now() returns flash.now", fn() {
        let flash_now_data = flash.now();
        assert_not_null(flash_now_data);
    });
});

describe("HomeController Integration", fn() {
    test("GET /up returns UP", fn() {
        let response = get("/up");
        assert_eq(res_status(response), 200);
        assert_eq(res_body(response), "UP");
    });

    test("GET /health returns JSON", fn() {
        let response = get("/health");
        assert_eq(res_status(response), 200);
        let json = res_json(response);
        assert_not_null(json);
    });

    test("GET / renders index", fn() {
        let response = get("/");
        assert_eq(res_status(response), 200);
    });

    test("GET /docs redirects", fn() {
        let response = get("/docs");
        assert_eq(res_status(response), 302);
        assert_eq(res_location(response), "/docs.html");
    });
});

describe("UsersController Integration", fn() {
    before_each(fn() {
        as_guest();
    });

    test("GET /users/login renders login page", fn() {
        let response = get("/users/login");
        assert_eq(res_status(response), 200);
    });

    test("GET /users/logout redirects", fn() {
        let response = get("/users/logout");
        assert_eq(res_status(response), 302);
        assert_eq(res_location(response), "/");
    });

    test("GET /users/profile redirects when not authenticated", fn() {
        let response = get("/users/profile");
        assert_eq(res_status(response), 302);
        assert_eq(res_location(response), "/users/login");
    });

    test("GET /users/register renders register page", fn() {
        let response = get("/users/register");
        assert_eq(res_status(response), 200);
    });

    test("POST /users/register validates input", fn() {
        let response = post("/users/register", "{}");
        assert_eq(res_status(response), 422);
    });

    test("POST /login with invalid credentials returns 401", fn() {
        let response = post("/login", "{\"email\":\"wrong\",\"password\":\"wrong\"}");
        assert_eq(res_status(response), 401);
    });

    test("POST /login with valid credentials returns 200", fn() {
        let response = post("/login", "{\"email\":\"admin@example.com\",\"password\":\"secret123\"}");
        assert_eq(res_status(response), 200);
    });

    test("GET /users/profile after login succeeds", fn() {
        post("/login", "{\"email\":\"admin@example.com\",\"password\":\"secret123\"}");
        let response = get("/users/profile");
        assert_eq(res_status(response), 200);
    });

    test("GET /users/validation-demo renders page", fn() {
        let response = get("/users/validation-demo");
        assert_eq(res_status(response), 200);
    });

    test("POST /users/validate-registration with valid data", fn() {
        let data = "{\"username\":\"testuser\",\"email\":\"test@example.com\",\"password\":\"password123\",\"age\":25}";
        let response = post("/users/validate-registration", data);
        assert_eq(res_status(response), 200);
    });

    test("POST /users/validate-registration with invalid data", fn() {
        let data = "{\"username\":\"ab\",\"email\":\"invalid\",\"password\":\"short\",\"age\":5}";
        let response = post("/users/validate-registration", data);
        assert_eq(res_status(response), 422);
    });
});

describe("JWT Controller Integration", fn() {
    test("POST /users/create-token creates token", fn() {
        let data = "{\"user_id\":\"user_123\",\"name\":\"Test User\",\"role\":\"admin\"}";
        let response = post("/users/create-token", data);
        assert_eq(res_status(response), 200);
        let json = res_json(response);
        assert_not_null(json["token"]);
        assert_eq(json["type"], "Bearer");
    });

    test("POST /users/verify-token with missing token returns 400", fn() {
        let response = post("/users/verify-token", "{}");
        assert_eq(res_status(response), 400);
    });

    test("POST /users/verify-token with invalid token returns 401", fn() {
        let response = post("/users/verify-token", "{\"token\":\"invalid\"}");
        assert_eq(res_status(response), 401);
    });

    test("POST /users/decode-token returns claims", fn() {
        let data = "{\"token\":\"test\"}";
        let response = post("/users/decode-token", data);
        assert_eq(res_status(response), 200);
    });
});

describe("Nested Describe Blocks", fn() {
    describe("Authentication Flow", fn() {
        before_each(fn() {
            as_guest();
        });

        test("guest cannot access protected route", fn() {
            let response = get("/users/profile");
            assert(res_redirect(response));
        });

        test("user can access protected route after login", fn() {
            post("/login", "{\"email\":\"admin@example.com\",\"password\":\"secret123\"}");
            let response = get("/users/profile");
            assert_eq(res_status(response), 200);
        });

        test("logout destroys session", fn() {
            post("/login", "{\"email\":\"admin@example.com\",\"password\":\"secret123\"}");
            post("/logout");
            let response = get("/users/profile");
            assert(res_redirect(response));
        });
    });

    describe("Registration Flow", fn() {
        test("registration page is public", fn() {
            let response = get("/users/register");
            assert_eq(res_status(response), 200);
        });

        test("registration with valid data creates account", fn() {
            let data = "{\"username\":\"newuser\",\"email\":\"new@example.com\",\"password\":\"password123\",\"age\":25}";
            let response = post("/users/register", data);
            assert_eq(res_status(response), 201);
        });

        test("registration with short username fails", fn() {
            let data = "{\"username\":\"ab\",\"email\":\"test@example.com\",\"password\":\"password123\",\"age\":25}";
            let response = post("/users/register", data);
            assert_eq(res_status(response), 422);
        });

        test("registration with invalid email fails", fn() {
            let data = "{\"username\":\"validuser\",\"email\":\"invalid-email\",\"password\":\"password123\",\"age\":25}";
            let response = post("/users/register", data);
            assert_eq(res_status(response), 422);
        });

        test("registration with short password fails", fn() {
            let data = "{\"username\":\"validuser\",\"email\":\"test@example.com\",\"password\":\"short\",\"age\":25}";
            let response = post("/users/register", data);
            assert_eq(res_status(response), 422);
        });

        test("registration with mismatched passwords fails", fn() {
            let data = "{\"username\":\"validuser\",\"email\":\"test@example.com\",\"password\":\"password123\",\"confirm_password\":\"different\",\"age\":25}";
            let response = post("/users/register", data);
            assert_eq(res_status(response), 422);
        });

        test("registration with underage fails", fn() {
            let data = "{\"username\":\"validuser\",\"email\":\"test@example.com\",\"password\":\"password123\",\"age\":10}";
            let response = post("/users/register", data);
            assert_eq(res_status(response), 422);
        });
    });
});

describe("Request Headers Integration", fn() {
    test("request with custom headers", fn() {
        set_header("X-Request-ID", "test-123");
        let response = get("/up");
        assert_eq(res_status(response), 200);
    });

    test("request with authorization header", fn() {
        with_token("Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.test");
        let response = get("/up");
        assert_eq(res_status(response), 200);
    });

    test("cookies persist across requests", fn() {
        set_cookie("session_id", "abc123");
        let response = get("/up");
        assert_eq(res_status(response), 200);
    });
});

describe("Edge Cases", fn() {
    test("handles trailing slash", fn() {
        let response = get("/up/");
        assert_eq(res_status(response), 200);
    });

    test("handles query parameters", fn() {
        let response = get("/postspage=1&per_page=10");
        assert_not_null(response);
    });

    test("handles POST with JSON content type", fn() {
        set_header("Content-Type", "application/json");
        let response = post("/users/validate-registration", "{\"username\":\"test\",\"email\":\"test@test.com\",\"password\":\"password123\",\"age\":25}");
        assert_not_null(response);
    });

    test("handles empty body", fn() {
        let response = post("/users/register", "");
        assert_not_null(response);
    });

    test("handles large request body", fn() {
        let large_body = "x" * 10000;
        let response = post("/users/register", large_body);
        assert_not_null(response);
    });
});
