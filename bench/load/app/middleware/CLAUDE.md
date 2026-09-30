# Middleware

Middleware sits between the HTTP server and your controller. Use it for
cross-cutting concerns: auth, logging, request munging, response headers,
rate limits.

Files live in `app/middleware/*.sl`. The loader scans each `.sl` file at boot,
registers every top-level `def` / `fn` declaration as a middleware function,
and orders them by per-function `# order:` directives.

## A minimal middleware

```soli
# app/middleware/auth.sl

# order: 20
# scope_only: true

def authenticate(req)
  api_key = req["headers"]["x-api-key"] ?? ""
  if api_key == ""
    return {
      "continue": false,
      "response": {
        "status": 401,
        "headers": { "Content-Type": "application/json" },
        "body": { "error": "Unauthorized" }.to_json
      }
    }
  end

  return { "continue": true, "request": req }
end
```

Two things to internalize:

1. The **return shape** decides whether the request proceeds or is
   short-circuited.
2. The **comment directives** above the `def` line determine the function's
   order and scoping.

## Return shape

Every middleware must return a hash. Two valid shapes:

| Shape                                                   | Effect                                          |
|---------------------------------------------------------|-------------------------------------------------|
| `{ "continue": true,  "request": req }`                 | Proceed to the next middleware / handler.       |
| `{ "continue": false, "response": { "status": ..., "body": ..., "headers": {...} } }` | Stop here; return that response. |

When proceeding, you can pass back a **modified copy** of `req` — that's how
middleware feeds data forward:

```soli
def attach_request_id(req)
  req["request_id"] = uuid()       # new field for downstream layers
  return { "continue": true, "request": req }
end
```

Downstream middleware and the controller see the updated `req`. Don't mutate
`req` in place — return the modified hash via the `"request"` key.

## File-top directives

Comments **immediately above** a `def` / `fn` line attach to that function.
Three are supported (see `docs/middleware.md` for the canonical reference):

| Directive            | Default  | Meaning                                                            |
|----------------------|----------|--------------------------------------------------------------------|
| `# order: N`         | `100`    | Lower numbers run first. Use `10`-`20` for auth, `90`+ for tail.   |
| `# global_only: true`| `false`  | Always runs on every request; cannot be excluded by a scope block. |
| `# scope_only: true` | `false`  | Only runs when explicitly wrapped in `middleware("name", -> {...})`.|

Both `#` and `//` are accepted as the comment marker (the loader tries
each prefix).

There is **no** `# methods:` or `# paths:` directive — restrict by `req["method"]`
or `req["path"]` inside the function body, or use route-level scoping (next
section).

### Function names != filenames

The loader registers each function by its **function name**, not the file
name. A file `app/middleware/auth.sl` containing `def authenticate(req) ...`
exposes the middleware as `authenticate`, not `auth`. Use the function name
when scoping in `config/routes.sl`:

```soli
middleware("authenticate", -> {
  get("/admin", "admin#index")
})
```

You can also put multiple middleware functions in one file — each gets its
own directives from the comments immediately above its `def` line.

```soli
# app/middleware/api.sl

# order: 10
def cors(req)
  # ...
end

# order: 50
# scope_only: true
def require_api_key(req)
  # ...
end
```

Private functions (names starting with `_`) are skipped — useful for helpers
inside the file.

## Execution order

Middleware runs in **ascending `order` value** — `order: 10` runs before
`order: 50` runs before `order: 100`. Default is `100`. Pick small numbers
for things that should see the raw request (auth, rate-limit), large numbers
for things that wrap the response (compression, logging).

Recommended ranges:

| Range       | Typical use                                       |
|-------------|---------------------------------------------------|
| `1-20`      | Logging, request ID assignment, CORS pre-flight.  |
| `20-40`     | Authentication, session bootstrap.                |
| `40-80`     | Authorization, feature flags, body parsing.       |
| `80-100`    | Catch-alls; ad-hoc per-app middleware.            |

Ties between middleware with the same `order` are broken by load order
(filesystem walk order). Be explicit with `order:` to avoid relying on that.

## Scoping in `config/routes.sl`

`scope_only` middleware run only inside `middleware("name", -> { ... })`
blocks. Routes outside the block are unaffected.

```soli
# config/routes.sl

get("/", "home#index")             # no auth
get("/health", "home#health")      # no auth

middleware("authenticate", -> {
  get("/admin", "admin#index")
  resources("admin/users")
  resources("admin/posts")
})
```

A non-`scope_only` middleware (default) always runs — `middleware("name", ...)`
blocks only opt **in** additional `scope_only` middleware; they don't opt
**out** of global ones.

Combine multiple scoped middleware by nesting:

```soli
middleware("authenticate", -> {
  middleware("require_admin", -> {
    resources("admin/users")
  })
})
```

## Common patterns

### Auth — short-circuit on failure, forward `current_user` on success

```soli
# order: 20
# scope_only: true

def authenticate(req)
  user_id = session_get("user_id")
  if user_id == nil
    return {
      "continue": false,
      "response": { "status": 302, "headers": { "Location": "/login" }, "body": "" }
    }
  end

  req["current_user"] = User.find_by("id", user_id)
  return { "continue": true, "request": req }
end
```

The handler reads `req["current_user"]` without needing to look it up again.

### Logging — runs on every request

```soli
# order: 5
# global_only: true

def request_log(req)
  print("#{req[\"method\"]} #{req[\"path\"]}")
  return { "continue": true, "request": req }
end
```

### Rate limit — short-circuit with 429

```soli
# order: 15

def rate_limit(req)
  key = req["headers"]["x-api-key"] ?? req["remote_ip"]
  if not _allow(key)
    return {
      "continue": false,
      "response": { "status": 429, "body": "Too many requests" }
    }
  end
  return { "continue": true, "request": req }
end

def _allow(key)    # private, not registered as middleware
  # ... bucket logic ...
end
```

## Testing middleware

E2E specs in `tests/` exercise middleware as a side-effect of hitting a route:

```soli
describe("authenticate middleware") do
  test("denies unauthenticated requests to /admin") do
    response = get("/admin")
    assert_eq(res_status(response), 302)
    assert_eq(res_headers(response)["Location"], "/login")
  end

  test("allows authenticated requests to /admin") do
    as_user(1)
    response = get("/admin")
    assert_eq(res_status(response), 200)
  end
end
```

There's no built-in way to call a middleware function in isolation — they're
designed to run in the request pipeline. Test them via E2E.

## Do / Don't

| Do                                                          | Don't                                                            |
|-------------------------------------------------------------|------------------------------------------------------------------|
| Use `# order:` to make the ordering explicit                | Rely on filesystem load order                                     |
| Return the modified `req` via `"request"`                   | Mutate `req` in place                                             |
| Use `# scope_only: true` for anything not globally desired  | Add per-request `req["path"]` checks to gate a global middleware  |
| Use `.to_json` to serialize the response body               | Use legacy `json_stringify(...)` — convention is the method form  |
| Use `#{...}` interpolation                                  | Use `\(...)` — the lexer rejects it                               |
| Add `_helper` private functions in the same file             | Pull single-file helpers into a separate module                   |
| Pass forward data via `req["custom_key"]`                   | Use global variables to communicate between middleware and handler|
| Test via E2E specs                                           | Try to unit-test by calling the function directly                 |
