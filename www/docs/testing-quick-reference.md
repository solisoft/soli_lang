# E2E Testing Quick Reference

Quick lookup for E2E controller testing helpers.

## Request Functions

| Function | Description | Example |
|----------|-------------|---------|
| `get(path)` | GET request | `get("/posts")` |
| `post(path, data)` | POST with body | `post("/posts", {"title": "Test"})` |
| `put(path, data)` | PUT replacement | `put("/posts/1", {...})` |
| `patch(path, data)` | PATCH partial | `patch("/posts/1", {"title": "New"})` |
| `delete(path)` | DELETE resource | `delete("/posts/1")` |
| `head(path)` | HEAD request | `head("/api/status")` |
| `options(path)` | OPTIONS request | `options("/api/methods")` |
| `request(method, path, body?)` | Custom method | `request("TRACE", "/path")` |

## Header Functions

| Function | Description | Example |
|----------|-------------|---------|
| `set_header(name, value)` | Add header | `set_header("X-Id", "123")` |
| `clear_headers()` | Remove headers | `clear_headers()` |
| `set_authorization(token)` | Set Bearer token | `set_authorization("jwt-token")` |
| `clear_authorization()` | Clear auth header | `clear_authorization()` |
| `set_cookie(name, value)` | Set cookie | `set_cookie("sid", "abc")` |
| `clear_cookies()` | Remove cookies | `clear_cookies()` |

## Response Functions

| Function | Description | Returns |
|----------|-------------|---------|
| `res_status(response)` | Status code | `200` |
| `res_body(response)` | Raw body string | `"hello"` |
| `res_json(response)` | Parsed JSON | `{...}` |
| `res_header(response, name)` | Header value | `"application/json"` |
| `res_headers(response)` | All headers | `{...}` |
| `res_redirect(response)` | Is redirect? | `true/false` |
| `res_location(response)` | Location header | `"/login"` |
| `res_ok(response)` | 2xx status? | `true/false` |
| `res_client_error(response)` | 4xx status? | `true/false` |
| `res_server_error(response)` | 5xx status? | `true/false` |
| `res_not_found(response)` | 404? | `true/false` |
| `res_unauthorized(response)` | 401? | `true/false` |
| `res_forbidden(response)` | 403? | `true/false` |
| `res_unprocessable(response)` | 422? | `true/false` |

## Session Functions

| Function | Description | Example |
|----------|-------------|---------|
| `as_guest()` | Clear auth | `as_guest()` |
| `as_user(id)` | Simulate user | `as_user(42)` |
| `as_admin()` | Simulate admin | `as_admin()` |
| `with_session(hash)` | Set session data | `with_session({...})` |
| `with_token(token)` | Set JWT auth | `with_token("jwt...")` |
| `login(email, password)` | Perform login | `login("user@test.com", "pass")` |
| `logout()` | End session | `logout()` |
| `current_user()` | Get user data | `current_user()` |
| `signed_in()` | Is authenticated? | `signed_in()` |
| `signed_out()` | Not authenticated? | `signed_out()` |
| `create_session(id)` | Create session | `create_session(1)` |
| `destroy_session()` | End session | `destroy_session()` |

## Assigns Functions

| Function | Description | Returns |
|----------|-------------|---------|
| `assigns()` | All assigns | `{...}` |
| `assign(key)` | Specific assign | value or `null` |
| `view_path()` | Template path | `"users/show.html"` |
| `render_template()` | Template rendered? | `true/false` |
| `flash()` | Flash messages | `{...}` |
| `flash(key)` | Flash message | value or `null` |

## Complete Example

```soli
describe("PostsController", fn()
    before_each(fn()
        as_guest()
    end)
    
    test("creates post", fn()
        login("user@example.com", "password")
        
        let response = post("/posts", {
            "title": "New Post",
            "body": "Content"
        })
        
        assert_eq(res_status(response), 201)
        let data = res_json(response)
        assert_eq(data["title"], "New Post")
    end)
end)
```

## Running Tests

```bash
# Run single test file
soli test tests/builtins/controller_integration_spec.sl

# Run all builtins tests
soli test tests/builtins

# Run with coverage
soli test tests/builtins --coverage
```
