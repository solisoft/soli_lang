# E2E Controller Testing Guide

Rails-like end-to-end testing framework for Soli MVC applications.

## Overview

The E2E testing framework provides a comprehensive set of helpers for testing your Soli controllers with real HTTP requests. Built on a test server that runs alongside your test suite, it enables you to write integration tests that exercise your routes, controllers, sessions, and view data at the HTTP level — fast, and with no browser involved.

When you need the page itself — JavaScript, clicks, forms as the user submits them — Soli drives a real headless Chrome as well. See [Browser Testing](testing-browser.md); this guide covers the HTTP layer, which is the right default for most controller and flow testing.

This framework follows conventions inspired by RSpec Rails testing patterns, making it familiar to developers coming from Ruby on Rails backgrounds while providing the safety and expressiveness of Soli's type system.

The test server automatically starts before your tests run and stops after completion, ensuring each test suite has a clean server instance. This isolation prevents state leakage between test runs and provides consistent, reproducible results.

## Getting Started

### Basic Test Structure

Every E2E test file follows the same structure using Soli's test DSL. The framework provides functions for grouping tests, setting up test data, making HTTP requests, and asserting expected outcomes. Here's a minimal example that tests a single endpoint:

```soli
describe("HomeController", fn()
  test("GET /up returns UP status", fn()
    response = get("/up")
    assert_eq(res_status(response), 200)
    assert_eq(res_body(response), "UP")
  end)
end)
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
response = get("/posts");
assert_eq(res_status(response), 200);
posts = res_json(response);
assert_gt(len(posts), 0);
```

**POST Requests**

The `post()` function submits data to create new resources. Pass the request path and a body (typically a hash or JSON string):

```soli
response = post("/posts", {
  "title": "New Post",
  "content": "Hello World"
});
assert_eq(res_status(response), 201);
```

**PUT Requests**

The `put()` function replaces existing resources entirely. Provide the resource path and updated data:

```soli
response = put("/posts/42", {
  "title": "Updated Title",
  "content": "Modified content"
});
assert_eq(res_status(response), 200);
```

**PATCH Requests**

The `patch()` function performs partial updates, modifying only specified fields:

```soli
response = patch("/posts/42", {
  "title": "Just the Title"
});
assert_eq(res_status(response), 200);
```

**DELETE Requests**

The `delete()` function removes resources:

```soli
response = delete("/posts/42");
assert_eq(res_status(response), 204);
```

**HEAD and OPTIONS Requests**

For specialized testing scenarios, `head()` performs a HEAD request (same as GET but without body), and `options()` checks allowed methods:

```soli
head_response = head("/api/posts");
options_response = options("/api/posts");
```

### Generic Request Function

The `request()` function provides flexibility for non-standard HTTP methods or when you need dynamic method selection:

```soli
response = request("TRACE", "/api/posts");
response = request("CONNECT", "/api/proxy");
```

### Custom Headers

Add custom headers to your requests using `set_header()` for individual headers or manage multiple headers through header management functions:

```soli
set_header("X-Request-ID", "test-123");
set_header("X-Custom-Header", "custom-value");

response = get("/api/data");
clear_headers();
```

### Authentication Headers

The `with_token()` function sets a Bearer token for authenticated requests, simulating API clients or authenticated users:

```soli
with_token("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...");
response = get("/api/protected");
clear_authorization();
```

### Cookie Management

Manage cookies for session-based authentication testing:

```soli
set_request_cookie("session_id", "abc123session");
response = get("/dashboard");
clear_cookies();
```

## Response Helpers

Response helpers inspect HTTP responses returned by your controllers. These functions extract specific data from the response hash for assertions and further processing.

### Status Codes

**res_status(response)** extracts the HTTP status code as an integer:

```soli
response = get("/posts");
status = res_status(response);
assert_eq(status, 200);
```

**res_ok(response)** checks if the status code is in the 2xx range:

```soli
if (res_ok(response))
  data = res_json(response)
  # Process successful response
end
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
body = res_body(response);
assert_contains(body, "expected text");
```

**res_json(response)** parses the response body as JSON and returns a hash:

```soli
response = post("/users", {"name": "John"});
user = res_json(response);
assert_eq(user["name"], "John");
assert_not_null(user["id"]);
```

### Response Headers

**res_header(response, name)** extracts a specific header value:

```soli
content_type = res_header(response, "Content-Type");
assert_contains(content_type, "application/json");
```

**res_headers(response)** returns all headers as a hash for comprehensive inspection:

```soli
headers = res_headers(response);
assert_hash_has_key(headers, "Content-Type");
```

### Redirects

**res_redirect(response)** checks if the response is a redirect (3xx status):

```soli
assert(res_redirect(response));
```

**res_location(response)** extracts the Location header for redirect destinations:

```soli
location = res_location(response);
assert_eq(location, "/expected/path");
```

## Session Helpers

Session helpers manage authentication state and session data during tests. These functions simulate user login/logout and authentication checks.

### Authentication State Management

**as_guest()** clears all authentication state, simulating an unauthenticated user:

```soli
before_each(fn()
  as_guest()
end)
```

**as_user(user_id)** simulates a logged-in regular user with the specified ID:

```soli
as_user(42);
response = get("/profile");
assert_eq(res_status(response), 200);
```

**as_user(user_id, options)** writes both the user id and an options hash into the server-side session store. Use this when middleware reads more than `user_id` (e.g. a `role` field on the single-table users collection):

```soli
as_user(42, {"role": "admin"});
response = get("/admin/dashboard");

# Any keys work — extend as your auth needs grow:
as_user(42, {"role": "admin", "tenant": "acme"});
```

**as_role(role)** looks up the first user with `role == <value>` in the `users` collection and signs in as that record. The role is also stored in the session so middleware reading `req.session["role"]` sees it on the next request:

```soli
as_role("admin");
response = get("/admin/dashboard");
assert_eq(res_status(response), 200);
```

Errors if no user with that role exists — seed one in `before_each`, or pass an explicit id with `as_user(id, {"role": "admin"})`.

**sign_in(resource_name, id?)** is for **separate-collection** auth (Devise-style: distinct `User` / `Admin` models with their own session keys). It writes `session.{resource_name}_id`:

```soli
sign_in("admin", 5);         # session.admin_id = 5
sign_in("admin");            # session.admin_id = Admin.first.id
sign_in("user", 42);         # session.user_id = 42
sign_in("staff", 7);         # session.staff_id = 7
```

Without an explicit id, `sign_in` looks up the first record of the matching model (`"admin"` → `Admin`, `"blog_post"` → `BlogPost`). Apps with non-conventional session keys (e.g. `current_admin_id`) should keep using `with_session({"current_admin_id": 5})`.

**as_admin()** is a zero-arg convenience equivalent to `as_user(1)` — it logs in as whichever record has `id = 1` in your `users` table. There is no built-in role or permission system: by convention, scaffolded apps seed user_id 1 as the administrator, but your own controllers and middleware are still responsible for enforcing admin authorization.

```soli
as_admin();   # same as as_user(1)
response = get("/admin/dashboard");
assert_eq(res_status(response), 200);
```

#### Which helper to use?

- **Single-table role-column auth** (one `users` collection, role string field): use `as_role("admin")` or `as_user(id, {"role": "admin"})`.
- **Separate-collection auth** (distinct `User`, `Admin`, ... models): use `sign_in("admin", id)` / `sign_in("admin")`.
- **Just need a logged-in user** (no role checks): `as_user(id)` is the lightest path.
- **Anything else** (non-conventional session keys, arbitrary session seeding): fall back to `with_session({...})`.

### Login and Logout

**login(email, password)** performs a login request and maintains session state:

```soli
login("user@example.com", "secretpassword");
response = get("/dashboard");
assert_eq(res_status(response), 200);
```

**logout()** destroys the current session:

```soli
login("user@example.com", "password");
logout();
response = get("/dashboard");
assert_eq(res_status(response), 302); # Redirect to login
```

### Session Inspection

**signed_in()** returns true if currently authenticated:

```soli
as_guest()
assert_not(signed_in())

as_user(1)
assert(signed_in())
```

**signed_out()** returns true if not authenticated:

```soli
assert(signed_out())
```

**current_user()** returns the currently authenticated user data:

```soli
as_user(42);
user = current_user();
# user contains user data hash
```

### Session Creation and Destruction

**create_session(user_id)** creates a session cookie for the specified user:

```soli
session_id = create_session(42);
assert_not_null(session_id);
```

**destroy_session()** clears the current session:

```soli
create_session(42);
destroy_session();
assert(signed_out());
```

### Custom Session Data

**with_session(data)** writes arbitrary key/value pairs into the server-side session and sets a matching `session_id` cookie. Subsequent requests in the same test see the data via `session_get(...)` on the server.

```soli
with_session({
  "user_id": 42,
  "role": "editor"
})
response = get("/dashboard")
assert_eq(res_status(response), 200)
```

> **Test-runner only.** `with_session` writes to the live session store and is gated to processes started by `soli test` (or test-server children spawned by it). Calling it from `soli run`, the REPL, a job, or a `soli serve --dev` script raises `with_session is a test-only helper; ...` so an attacker who can inject Soli code into one of those contexts cannot forge an authenticated session.

### Token Authentication

**with_token(token)** sets a Bearer authorization header:

```soli
with_token("your-jwt-token-here");
response = get("/api/protected");
```

## Assigns Helpers

Assigns helpers inspect data passed to views during template rendering. These helpers are essential for testing that your controllers provide the correct context to views.

### Accessing Assigns

**assigns()** returns all assigns as a hash:

```soli
response = get("/users/1");
all_assigns = assigns();
assert_hash_has_key(all_assigns, "user");
assert_hash_has_key(all_assigns, "page_title");
```

**assign(key)** retrieves a specific assign value by key:

```soli
user = assign("user");
assert_eq(user["name"], "John Doe");
```

### View Information

**view_path()** returns the path of the rendered template, with the
conventional `.html` extension (it matches the name you passed to `render()`):

```soli
path = view_path();
assert_eq(path, "users/show.html");
```

For a response that rendered no template — a `redirect(...)`, a `render_json(...)`,
or a non-2xx halt — `view_path()` returns `""`.

**render_template()** reports whether the response rendered a view template.
`render_template?()` is an alias (the predicate spelling):

```soli
response = get("/users/1");
assert(render_template());        # a view was rendered

response = get("/users/1/delete"); # this action redirects
assert_not(render_template());
```

> **Large locals.** The view locals are shipped from the test server to the
> test process as a response header. When they are very large (≈48 KB+ of
> serialized JSON — e.g. a big collection), `assigns()` degrades to a
> keys-only view: every top-level key is still present (so `assert_hash_has_key`
> keeps working) but its value is `null`. Assert on the keys, or render a
> smaller slice, in that case.

## Browser Testing

Everything above tests what the server *sent*. When the behaviour you care
about only exists after the browser has run the page's JavaScript, add a browser
spec instead:

```soli
test("the coupon is applied without a reload", fn() {
  visit("/cart")
  fill_in("Coupon", "SAVE10")
  click_button("Apply")

  assert_text("Discount applied")
  assert_no_page_errors()
})
```

Browser specs run in a real headless Chrome, opt in with `soli test --browser`,
and share the cookie jar with the request helpers — so a `login()` in
`before_each` carries straight into `visit()`.

Full reference: [Browser Testing](testing-browser.md).

## Query Assertions (N+1 detection)

Every response from `get()` / `post()` / `request()` carries how many AQL
queries the request executed and which — if any — templates fired in a loop.
The test server records this from the same query log that powers the dev bar's
N+1 badge, so a spec catches the exact regressions you'd see in `--dev`.

Two keys are exposed on the response hash:

- `response["query_count"]` — the number of AQL queries the request ran.
- `response["n_plus_one"]` — an array of `{"query": <template>, "count": <n>}`
  for every query template that fired **2 or more times** (empty when clean).

### assert_no_n_plus_one

Fails the test if any query template repeated — the signature of a query issued
inside a loop that should have been batched. Preload the association (or batch
with `FILTER doc.field IN @ids`) and the assertion passes.

```soli
test("posts index does not N+1", fn() {
  response = get("/posts");
  assert_eq(res_status(response), 200);
  assert_no_n_plus_one(response);
});
```

When it fails, the message names each offending template and how many times it
ran, so you can grep for it:

```
N+1 detected: 1 template(s) fired in a loop (batch with `FILTER doc.field IN @ids`):
  25x  FOR doc IN authors FILTER doc._key == @key RETURN doc
```

### assert_query_count / assert_max_queries

Assert an exact or upper-bound query count for an endpoint. `assert_query_count`
pins the number precisely; `assert_max_queries` is friendlier for endpoints
whose baseline can shift as the schema grows.

```soli
test("dashboard stays within its query budget", fn() {
  response = get("/dashboard");
  assert_query_count(response, 3);   # exactly three queries
  assert_max_queries(response, 5);   # or: no more than five
});
```

Both accept the response hash. `assert_query_count` also accepts a bare `Int`
if you've already pulled the count out yourself.

> **Detection threshold.** A template counts as N+1 at **2** repetitions —
> matching the dev bar — because HABTM/`through` lookups start at two (one per
> parent). If an endpoint legitimately issues a repeated query you don't want to
> batch, use `assert_max_queries` to bound the total instead of
> `assert_no_n_plus_one`.

> **Instrumentation.** These keys are populated by the `--dev` test server that
> `soli test` runs, so they're always present in request specs. Calling the
> assertions on a value that isn't an instrumented response raises a clear
> error rather than passing silently.

### Guarding the whole suite: `--fail-on-n1`

`assert_no_n_plus_one` catches an N+1 only where you remember to call it. To
turn the whole suite into an N+1 tripwire — without editing a single spec — run:

```bash
soli test --fail-on-n1
```

Every `get()` / `post()` / `request()` that triggers an N+1 fails its test
automatically, using the exact same detection (and error message) as
`assert_no_n_plus_one`:

```
✗ posts index responds 200: N+1 detected: 1 template(s) fired in a loop
  (batch with `FILTER doc.field IN @ids`):
  12x  FOR c IN comments FILTER c.post == @key RETURN c
```

Clean and uninstrumented responses are untouched, so the flag never fails a
spec spuriously. It composes with everything else (`--jobs`, `--coverage`, …) —
wire it into CI to catch a query regression the moment it lands, even in specs
that predate the check.

## Complete Examples

### Testing a CRUD Controller

This comprehensive example demonstrates testing a typical PostsController with full CRUD operations:

```soli
describe("PostsController", fn()
  before_each(fn()
    as_guest()
  end)

  describe("GET /posts", fn()
    test("returns list of posts", fn()
      response = get("/posts")
      assert_eq(res_status(response), 200)
      data = res_json(response)
      assert_gt(len(data["posts"]), 0)
    end)

    test("includes pagination metadata", fn()
      response = get("/posts?page=1&per_page=10")
      data = res_json(response)
      assert_hash_has_key(data, "pagination")
    end)
  end)

  describe("GET /posts/:id", fn()
    test("shows single post", fn()
      response = get("/posts/1")
      assert_eq(res_status(response), 200)
      post = res_json(response)
      assert_eq(post["title"], "First Post")
    end)

    test("returns 404 for missing post", fn()
      response = get("/posts/99999")
      assert_eq(res_status(response), 404)
    end)
  end)

  describe("POST /posts", fn()
    test("creates post with valid data", fn()
      login("author@example.com", "password123")
      
      response = post("/posts", {
        "title": "New Post Title",
        "body": "Post content here"
      })
      
      assert_eq(res_status(response), 201)
      result = res_json(response)
      assert_not_null(result["id"])
    end)

    test("rejects unauthenticated request", fn()
      response = post("/posts", {"title": "Test"})
      assert_eq(res_status(response), 302) # Redirect to login
    end)

    test("validates required fields", fn()
      login("author@example.com", "password123")
      
      response = post("/posts", {})
      assert_eq(res_status(response), 422)
      errors = res_json(response)["errors"]
      assert_hash_has_key(errors, "title")
    end)
  end)

  describe("PUT /posts/:id", fn()
    test("updates post with valid data", fn()
      login("author@example.com", "password123")
      
      response = put("/posts/1", {
        "title": "Updated Title"
      })
      
      assert_eq(res_status(response), 200)
    end)

    test("prevents unauthorized updates", fn()
      other_user = create_user({"email": "other@example.com"})
      as_user(other_user["id"])
      
      response = put("/posts/1", {"title": "Hacked"})
      assert_eq(res_status(response), 403)
    end)
  end)

  describe("DELETE /posts/:id", fn()
    test("deletes post", fn()
      login("author@example.com", "password123")
      
      response = delete("/posts/1")
      assert_eq(res_status(response), 204)
      
      check_response = get("/posts/1")
      assert_eq(res_status(check_response), 404)
    end)
  end)
end)
```

### Testing Authentication Flow

This example demonstrates comprehensive authentication testing:

```soli
describe("Authentication Flow", fn()
  before_each(fn()
    as_guest()
  end)

  test("login with valid credentials succeeds", fn()
    response = post("/login", {
      "email": "admin@example.com",
      "password": "secret123"
    })
    
    assert_eq(res_status(response), 200)
    assert(signed_in())
  end)

  test("login with invalid credentials fails", fn()
    response = post("/login", {
      "email": "wrong@example.com",
      "password": "wrongpassword"
    })
    
    assert_eq(res_status(response), 401)
    assert(signed_out())
  end)

  test("profile requires authentication", fn()
    response = get("/users/profile")
    assert_eq(res_status(response), 302)
    assert_eq(res_location(response), "/login")
  end)

  test("profile accessible after login", fn()
    login("user@example.com", "password")
    
    response = get("/users/profile")
    assert_eq(res_status(response), 200)
  end)

  test("logout destroys session", fn()
    login("user@example.com", "password")
    
    post("/logout")
    
    assert(signed_out())
    response = get("/users/profile")
    assert_eq(res_status(response), 302)
  end)

  test("JWT token authentication works", fn()
    # Create a token
    token_response = post("/auth/token", {
      "user_id": "123",
      "role": "admin"
    })
    token_data = res_json(token_response)
    token = token_data["token"]
    
    # Use token for authentication
    with_token(token)
    response = get("/api/admin")
    assert_eq(res_status(response), 200)
  end)
end)
```

### Testing with Custom Headers

Examples of testing various header scenarios:

```soli
describe("Request Headers", fn()
  test("custom headers are received by controller", fn()
    set_header("X-Request-ID", "test-123-uuid")
    set_header("X-Custom-Header", "custom-value")
    
    response = get("/api/headers")
    headers = res_json(response)
    
    assert_eq(headers["X-Request-ID"], "test-123-uuid")
    assert_eq(headers["X-Custom-Header"], "custom-value")
  end)

  test("authorization header is processed", fn()
    with_token("Bearer eyJhbGciOiJIUzI1NiIs...")
    
    response = get("/api/protected")
    data = res_json(response)
    assert_eq(data["authenticated"], true)
  end)

  test("cookies persist across requests", fn()
    set_request_cookie("session_id", "session-abc-123")
    
    response = get("/dashboard")
    data = res_json(response)
    
    assert_eq(data["session_id"], "session-abc-123")
  end)
end)
```

### Testing View Rendering

Examples of testing template rendering and assigns:

```soli
describe("View Rendering", fn()
  test("index renders with correct assigns", fn()
    response = get("/users")
    
    assert(render_template())
    assert_eq(view_path(), "users/index.html")
    
    assigns_data = assigns()
    assert_hash_has_key(assigns_data, "users")
    assert_hash_has_key(assigns_data, "page_title")
  end)

  test("show action passes correct user data", fn()
    response = get("/users/42")
    
    user_assign = assign("user")
    assert_eq(user_assign["id"], 42)
    assert_eq(user_assign["name"], "John Doe")
  end)

  test("a redirect renders no template", fn()
    # create redirects on success, so no view is rendered.
    response = post("/users", {"name": "New User"})

    assert(res_redirect?(response))
    assert_not(render_template())
    assert_eq(view_path(), "")
  end)
end)
```

## Best Practices

### Test Organization

Structure your tests hierarchically using `describe()` blocks. Group tests by controller, then by action, then by concern. This organization makes tests easier to navigate and maintain:

```soli
describe("PostsController", fn()
  describe("index action", fn()
    describe("authentication", fn()
      # Tests for authenticated/unauthenticated access
    end)
    
    describe("response format", fn()
      # Tests for JSON, HTML responses
    end)
  end)
end)
```

### Before and After Hooks

Use `before_each()` and `after_each()` to set up and clean up test state. Always reset authentication state between tests to prevent leakage:

```soli
describe("UsersController", fn()
  before_each(fn()
    as_guest()
    clear_cookies()
  end)
  
  after_each(fn()
    # Cleanup after each test if needed
  end)
end)
```

### Test Isolation

Each test should be independent and not rely on the state created by other tests. Use factory functions or setup blocks to create test data within tests rather than depending on shared state:

```soli
test("can update own post", fn()
  post = create_post({"title": "Test", "author_id": 1})
  as_user(1)
  
  response = put("/posts/" + post["id"], {
    "title": "Updated"
  })
  
  assert_eq(res_status(response), 200)
end)
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
response = get("/posts/1")
assert_eq(res_status(response), 200)

post = res_json(response)
assert_eq(post["id"], 1)
assert_eq(post["title"], "Expected Title")
```

## Configuration

### Test Server

The test server automatically selects an available port and starts before tests run. No configuration is required for basic usage. The server runs on `localhost` and is isolated from any development or production servers.

### Test Database

When testing controllers that interact with a database, ensure your test environment uses a test database. Database transactions should be rolled back after each test to maintain isolation.

### Fixture Files

For complex test data, create fixture files in `tests/fixtures/` directory:

```soli
users = yaml_load("tests/fixtures/users.yaml");
posts = yaml_load("tests/fixtures/posts.yaml");
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
content_type = res_header(response, "Content-Type")
if (contains(content_type, "application/json"))
  data = res_json(response)
end
```

## Integration with CI/CD

The test framework outputs results in a format compatible with most CI systems. Failed tests cause the test runner to exit with a non-zero status code, which CI systems interpret as a build failure.

For CI environments, run tests with coverage tracking:

```bash
soli test tests/builtins --coverage --coverage-min 80.0
```
