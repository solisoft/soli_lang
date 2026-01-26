# E2E Controller Testing Guide

Rails-like end-to-end testing framework for Soli MVC applications.

## Overview

The E2E testing framework provides a comprehensive set of helpers for testing your Soli controllers with real HTTP requests. Built on a test server that runs alongside your test suite, it enables you to write integration tests that simulate actual browser requests and verify controller responses, sessions, and view data.

This framework follows conventions inspired by RSpec Rails testing patterns, making it familiar to developers coming from Ruby on Rails backgrounds while providing the safety and expressiveness of Soli's type system.

The test server automatically starts before your tests run and stops after completion, ensuring each test suite has a clean server instance. This isolation prevents state leakage between test runs and provides consistent, reproducible results.

## Getting Started

### Basic Test Structure

Every E2E test file follows the same structure using Soli's test DSL. The framework provides functions for grouping tests, setting up test data, making HTTP requests, and asserting expected outcomes. Here's a minimal example that tests a single endpoint:

```soli
describe("HomeController", fn() {
    test("GET /up returns UP status", fn() {
        let response = get("/up");
        assert_eq(res_status(response), 200);
        assert_eq(res_body(response), "UP");
    });
});
```

The `describe()` function creates a test group that organizes related tests, while `test()` defines individual test cases. Each test runs in isolation, with the test server managing request routing and response generation.

### Running Tests

Execute your E2E tests using the Soli test runner. The framework automatically starts and stops the test server, so you don't need to manage server lifecycle manually:

```bash
soli test tests/builtins/controller_integration_spec.sl
```

For running all builtins tests including E2E tests:

```bash
soli test tests/builtins
```

The test runner displays results in a clean format showing each test file, its execution time, and pass/fail status. Failed tests include error messages to help diagnose issues quickly.

## Request Helpers

Request helpers enable you to make HTTP requests to your controllers from within tests. These functions interact with the test server running on a random available port, simulating real browser or API client requests.

### HTTP Method Functions

The framework provides dedicated functions for each HTTP method. These functions accept a path and optional data, returning a response hash you can inspect:

**GET Requests**

The `get()` function retrieves resources without modifying server state. Use it for testing read-only endpoints, pages, and API endpoints that respond to GET requests:

```soli
let response = get("/posts");
assert_eq(res_status(response), 200);
let posts = res_json(response);
assert_gt(len(posts), 0);
```

**POST Requests**

The `post()` function submits data to create new resources. Pass the request path and a body (typically a hash or JSON string):

```soli
let response = post("/posts", {
    "title": "New Post",
    "content": "Hello World"
});
assert_eq(res_status(response), 201);
```

**PUT Requests**

The `put()` function replaces existing resources entirely. Provide the resource path and updated data:

```soli
let response = put("/posts/42", {
    "title": "Updated Title",
    "content": "Modified content"
});
assert_eq(res_status(response), 200);
```

**PATCH Requests**

The `patch()` function performs partial updates, modifying only specified fields:

```soli
let response = patch("/posts/42", {
    "title": "Just the Title"
});
assert_eq(res_status(response), 200);
```

**DELETE Requests**

The `delete()` function removes resources:

```soli
let response = delete("/posts/42");
assert_eq(res_status(response), 204);
```

**HEAD and OPTIONS Requests**

For specialized testing scenarios, `head()` performs a HEAD request (same as GET but without body), and `options()` checks allowed methods:

```soli
let head_response = head("/api/posts");
let options_response = options("/api/posts");
```

### Generic Request Function

The `request()` function provides flexibility for non-standard HTTP methods or when you need dynamic method selection:

```soli
let response = request("TRACE", "/api/posts");
let response = request("CONNECT", "/api/proxy");
```

### Custom Headers

Add custom headers to your requests using `set_header()` for individual headers or manage multiple headers through header management functions:

```soli
set_header("X-Request-ID", "test-123");
set_header("X-Custom-Header", "custom-value");

let response = get("/api/data");
clear_headers();
```

### Authentication Headers

The `with_token()` function sets a Bearer token for authenticated requests, simulating API clients or authenticated users:

```soli
with_token("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...");
let response = get("/api/protected");
clear_authorization();
```

### Cookie Management

Manage cookies for session-based authentication testing:

```soli
set_cookie("session_id", "abc123session");
let response = get("/dashboard");
clear_cookies();
```

## Response Helpers

Response helpers inspect HTTP responses returned by your controllers. These functions extract specific data from the response hash for assertions and further processing.

### Status Codes

**res_status(response)** extracts the HTTP status code as an integer:

```soli
let response = get("/posts");
let status = res_status(response);
assert_eq(status, 200);
```

**res_ok(response)** checks if the status code is in the 2xx range:

```soli
if (res_ok(response)) {
    let data = res_json(response);
    # Process successful response
}
```

**res_client_error(response)** checks for 4xx status codes:

```soli
assert(res_client_error(response));
```

**res_server_error(response)** checks for 5xx status codes:

```soli
assert_not(res_server_error(response));
```

Specific status check functions are available for common codes:

```soli
res_not_found(response)    # 404
res_unauthorized(response) # 401
res_forbidden(response)    # 403
res_unprocessable(response) # 422
```

### Response Body

**res_body(response)** returns the raw response body as a string:

```soli
let body = res_body(response);
assert_contains(body, "expected text");
```

**res_json(response)** parses the response body as JSON and returns a hash:

```soli
let response = post("/users", {"name": "John"});
let user = res_json(response);
assert_eq(user["name"], "John");
assert_not_null(user["id"]);
```

### Response Headers

**res_header(response, name)** extracts a specific header value:

```soli
let content_type = res_header(response, "Content-Type");
assert_contains(content_type, "application/json");
```

**res_headers(response)** returns all headers as a hash for comprehensive inspection:

```soli
let headers = res_headers(response);
assert_hash_has_key(headers, "Content-Type");
```

### Redirects

**res_redirect(response)** checks if the response is a redirect (3xx status):

```soli
assert(res_redirect(response));
```

**res_location(response)** extracts the Location header for redirect destinations:

```soli
let location = res_location(response);
assert_eq(location, "/expected/path");
```

## Session Helpers

Session helpers manage authentication state and session data during tests. These functions simulate user login/logout and authentication checks.

### Authentication State Management

**as_guest()** clears all authentication state, simulating an unauthenticated user:

```soli
before_each(fn() {
    as_guest();
});
```

**as_user(user_id)** simulates a logged-in regular user with the specified ID:

```soli
as_user(42);
let response = get("/profile");
assert_eq(res_status(response), 200);
```

**as_admin()** simulates an authenticated administrator:

```soli
as_admin();
let response = get("/admin/dashboard");
assert_eq(res_status(response), 200);
```

### Login and Logout

**login(email, password)** performs a login request and maintains session state:

```soli
login("user@example.com", "secretpassword");
let response = get("/dashboard");
assert_eq(res_status(response), 200);
```

**logout()** destroys the current session:

```soli
login("user@example.com", "password");
logout();
let response = get("/dashboard");
assert_eq(res_status(response), 302); # Redirect to login
```

### Session Inspection

**signed_in()** returns true if currently authenticated:

```soli
as_guest();
assert_not(signed_in());

as_user(1);
assert(signed_in());
```

**signed_out()** returns true if not authenticated:

```soli
assert(signed_out());
```

**current_user()** returns the currently authenticated user data:

```soli
as_user(42);
let user = current_user();
# user contains user data hash
```

### Session Creation and Destruction

**create_session(user_id)** creates a session cookie for the specified user:

```soli
let session_id = create_session(42);
assert_not_null(session_id);
```

**destroy_session()** clears the current session:

```soli
create_session(42);
destroy_session();
assert(signed_out());
```

### Custom Session Data

**with_session(data)** sets arbitrary session values:

```soli
let session = hash();
session["user_id"] = 42;
session["role"] = "editor";
with_session(session);
```

### Token Authentication

**with_token(token)** sets a Bearer authorization header:

```soli
with_token("your-jwt-token-here");
let response = get("/api/protected");
```

## Assigns Helpers

Assigns helpers inspect data passed to views during template rendering. These helpers are essential for testing that your controllers provide the correct context to views.

### Accessing Assigns

**assigns()** returns all assigns as a hash:

```soli
let response = get("/users/1");
let all_assigns = assigns();
assert_hash_has_key(all_assigns, "user");
assert_hash_has_key(all_assigns, "page_title");
```

**assign(key)** retrieves a specific assign value by key:

```soli
let user = assign("user");
assert_eq(user["name"], "John Doe");
```

### View Information

**view_path()** returns the path of the rendered template:

```soli
let path = view_path();
assert_eq(path, "users/show.html");
```

**render_template()** indicates whether a template was rendered:

```soli
if (render_template()) {
    let content = assigns();
    # Inspect rendered content
}
```

### Flash Messages

Flash message helpers access temporary session data used for one-time notifications:

```soli
let flash_data = flash();
assert_hash_has_key(flash_data, "notice");

let notice = flash("notice");
assert_eq(notice, "Operation successful");
```

## Complete Examples

### Testing a CRUD Controller

This comprehensive example demonstrates testing a typical PostsController with full CRUD operations:

```soli
describe("PostsController", fn() {
    before_each(fn() {
        as_guest();
    });

    describe("GET /posts", fn() {
        test("returns list of posts", fn() {
            let response = get("/posts");
            assert_eq(res_status(response), 200);
            let data = res_json(response);
            assert_gt(len(data["posts"]), 0);
        });

        test("includes pagination metadata", fn() {
            let response = get("/posts?page=1&per_page=10");
            let data = res_json(response);
            assert_hash_has_key(data, "pagination");
        });
    });

    describe("GET /posts/:id", fn() {
        test("shows single post", fn() {
            let response = get("/posts/1");
            assert_eq(res_status(response), 200);
            let post = res_json(response);
            assert_eq(post["title"], "First Post");
        });

        test("returns 404 for missing post", fn() {
            let response = get("/posts/99999");
            assert_eq(res_status(response), 404);
        });
    });

    describe("POST /posts", fn() {
        test("creates post with valid data", fn() {
            login("author@example.com", "password123");
            
            let response = post("/posts", {
                "title": "New Post Title",
                "body": "Post content here"
            });
            
            assert_eq(res_status(response), 201);
            let result = res_json(response);
            assert_not_null(result["id"]);
        });

        test("rejects unauthenticated request", fn() {
            let response = post("/posts", {"title": "Test"});
            assert_eq(res_status(response), 302); # Redirect to login
        });

        test("validates required fields", fn() {
            login("author@example.com", "password123");
            
            let response = post("/posts", {});
            assert_eq(res_status(response), 422);
            let errors = res_json(response)["errors"];
            assert_hash_has_key(errors, "title");
        });
    });

    describe("PUT /posts/:id", fn() {
        test("updates post with valid data", fn() {
            login("author@example.com", "password123");
            
            let response = put("/posts/1", {
                "title": "Updated Title"
            });
            
            assert_eq(res_status(response), 200);
        });

        test("prevents unauthorized updates", fn() {
            let other_user = create_user({"email": "other@example.com"});
            as_user(other_user["id"]);
            
            let response = put("/posts/1", {"title": "Hacked"});
            assert_eq(res_status(response), 403);
        });
    });

    describe("DELETE /posts/:id", fn() {
        test("deletes post", fn() {
            login("author@example.com", "password123");
            
            let response = delete("/posts/1");
            assert_eq(res_status(response), 204);
            
            let check_response = get("/posts/1");
            assert_eq(res_status(check_response), 404);
        });
    });
});
```

### Testing Authentication Flow

This example demonstrates comprehensive authentication testing:

```soli
describe("Authentication Flow", fn() {
    before_each(fn() {
        as_guest();
    });

    test("login with valid credentials succeeds", fn() {
        let response = post("/login", {
            "email": "admin@example.com",
            "password": "secret123"
        });
        
        assert_eq(res_status(response), 200);
        assert(signed_in());
    });

    test("login with invalid credentials fails", fn() {
        let response = post("/login", {
            "email": "wrong@example.com",
            "password": "wrongpassword"
        });
        
        assert_eq(res_status(response), 401);
        assert(signed_out());
    });

    test("profile requires authentication", fn() {
        let response = get("/users/profile");
        assert_eq(res_status(response), 302);
        assert_eq(res_location(response), "/login");
    });

    test("profile accessible after login", fn() {
        login("user@example.com", "password");
        
        let response = get("/users/profile");
        assert_eq(res_status(response), 200);
    });

    test("logout destroys session", fn() {
        login("user@example.com", "password");
        
        post("/logout");
        
        assert(signed_out());
        let response = get("/users/profile");
        assert_eq(res_status(response), 302);
    });

    test("JWT token authentication works", fn() {
        # Create a token
        let token_response = post("/auth/token", {
            "user_id": "123",
            "role": "admin"
        });
        let token_data = res_json(token_response);
        let token = token_data["token"];
        
        # Use token for authentication
        with_token(token);
        let response = get("/api/admin");
        assert_eq(res_status(response), 200);
    });
});
```

### Testing with Custom Headers

Examples of testing various header scenarios:

```soli
describe("Request Headers", fn() {
    test("custom headers are received by controller", fn() {
        set_header("X-Request-ID", "test-123-uuid");
        set_header("X-Custom-Header", "custom-value");
        
        let response = get("/api/headers");
        let headers = res_json(response);
        
        assert_eq(headers["X-Request-ID"], "test-123-uuid");
        assert_eq(headers["X-Custom-Header"], "custom-value");
    });

    test("authorization header is processed", fn() {
        with_token("Bearer eyJhbGciOiJIUzI1NiIs...");
        
        let response = get("/api/protected");
        let data = res_json(response);
        assert_eq(data["authenticated"], true);
    });

    test("cookies persist across requests", fn() {
        set_cookie("session_id", "session-abc-123");
        
        let response = get("/dashboard");
        let data = res_json(response);
        
        assert_eq(data["session_id"], "session-abc-123");
    });
});
```

### Testing View Rendering

Examples of testing template rendering and assigns:

```soli
describe("View Rendering", fn() {
    test("index renders with correct assigns", fn() {
        let response = get("/users");
        
        assert(render_template());
        assert_eq(view_path(), "users/index.html");
        
        let assigns_data = assigns();
        assert_hash_has_key(assigns_data, "users");
        assert_hash_has_key(assigns_data, "page_title");
    });

    test("show action passes correct user data", fn() {
        let response = get("/users/42");
        
        let user_assign = assign("user");
        assert_eq(user_assign["id"], 42);
        assert_eq(user_assign["name"], "John Doe");
    });

    test("flash messages are available", fn() {
        post("/users/1/comments", {"body": "Test comment"});
        
        let response = get("/users/1");
        let flash_data = flash();
        assert_hash_has_key(flash_data, "notice");
        assert_eq(flash("notice"), "Comment posted successfully");
    });
});
```

## Best Practices

### Test Organization

Structure your tests hierarchically using `describe()` blocks. Group tests by controller, then by action, then by concern. This organization makes tests easier to navigate and maintain:

```soli
describe("PostsController", fn() {
    describe("index action", fn() {
        describe("authentication", fn() {
            # Tests for authenticated/unauthenticated access
        });
        
        describe("response format", fn() {
            # Tests for JSON, HTML responses
        });
    });
});
```

### Before and After Hooks

Use `before_each()` and `after_each()` to set up and clean up test state. Always reset authentication state between tests to prevent leakage:

```soli
describe("UsersController", fn() {
    before_each(fn() {
        as_guest();
        clear_cookies();
    });
    
    after_each(fn() {
        # Cleanup after each test if needed
    });
});
```

### Test Isolation

Each test should be independent and not rely on the state created by other tests. Use factory functions or setup blocks to create test data within tests rather than depending on shared state:

```soli
test("can update own post", fn() {
    let post = create_post({"title": "Test", "author_id": 1});
    as_user(1);
    
    let response = put("/posts/" + post["id"], {
        "title": "Updated"
    });
    
    assert_eq(res_status(response), 200);
});
```

### Meaningful Assertions

Use specific assertions that clearly communicate what you're testing. Avoid generic assertions when specific ones provide better error messages:

```soli
# Good
assert_eq(res_status(response), 201);
assert_not_null(assign("post"));

# Avoid
assert(response != null);
```

### Response Validation

Always verify both status codes and response content. A 200 status doesn't guarantee the correct data was returned:

```soli
let response = get("/posts/1");
assert_eq(res_status(response), 200);

let post = res_json(response);
assert_eq(post["id"], 1);
assert_eq(post["title"], "Expected Title");
```

## Configuration

### Test Server

The test server automatically selects an available port and starts before tests run. No configuration is required for basic usage. The server runs on `localhost` and is isolated from any development or production servers.

### Test Database

When testing controllers that interact with a database, ensure your test environment uses a test database. Database transactions should be rolled back after each test to maintain isolation.

### Fixture Files

For complex test data, create fixture files in `tests/fixtures/` directory:

```soli
let users = yaml_load("tests/fixtures/users.yaml");
let posts = yaml_load("tests/fixtures/posts.yaml");
```

## Troubleshooting

### Test Server Issues

If tests fail with connection errors, ensure no other process is using the test port range. The test server selects ports dynamically, but conflicts can occur in constrained environments.

### Authentication Failures

When authentication tests fail unexpectedly, check that:
- The login endpoint accepts the credentials being tested
- Session cookies are being preserved between requests
- JWT tokens are properly formatted with "Bearer " prefix

### Response Parsing

If `res_json()` fails, verify the response body is valid JSON. Some responses may return HTML or plain text, which cannot be parsed as JSON:

```soli
# Check content type first
let content_type = res_header(response, "Content-Type");
if (contains(content_type, "application/json")) {
    let data = res_json(response);
}
```

## Integration with CI/CD

The test framework outputs results in a format compatible with most CI systems. Failed tests cause the test runner to exit with a non-zero status code, which CI systems interpret as a build failure.

For CI environments, run tests with coverage tracking:

```bash
soli test tests/builtins --coverage --coverage-min 80.0
```
