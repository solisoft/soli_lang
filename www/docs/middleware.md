# Middleware

Middleware runs before your controller action on every request it applies to. It
can read or modify the request, or stop the request with its own response — the
place for cross-cutting concerns like authentication, origin checks, rate limits
or tagging requests.

## Creating Middleware

Every `def` (or `fn`) in `app/middleware/*.sl` is registered as a middleware,
named after the function. The files are loaded automatically — no import, no
registration call. A function whose name starts with `_` is a helper and is
never registered.

```soli
# app/middleware/auth.sl

# order: 20
# scope_only: true
def authenticate
  return halt(401, "Unauthorized") if req["headers"]["authorization"].blank?

  req
end
```

A middleware declared without a parameter list reads the current request
through the `req` global, as an action does. Declare a parameter to receive it
as an argument instead — both forms work:

```soli
def authenticate(req)
  # ...
  req
end
```

The request hash carries `method`, `path`, `headers` (lower-cased names),
`query`, `params`, `cookies` and friends.

## Directives

Comments directly above the `def` configure it (`//` is also accepted; prefer `#`):

| Directive | Effect |
|-----------|--------|
| `# order: N` | Execution order, lower runs first. Default `100`. |
| `# global_only: true` | Runs on every request and cannot be scoped to routes. |
| `# scope_only: true` | Never runs globally — only on routes wrapped with `middleware(...)`. |

That gives three kinds:

- **Regular** (no `global_only`/`scope_only`) — runs on every request. It may
  also be named in a route's `middleware(...)`; it then still runs once, not twice.
- **`global_only`** — runs on every request, including routes that have scoped
  middleware. Naming it in `middleware(...)` prints a warning and is ignored.
- **`scope_only`** — runs only on the routes that name it.

## Scoping Middleware to Routes

Wrap routes in `middleware(...)` in `config/routes.sl`, with one name or a list:

```soli
# config/routes.sl
get("/", "home#index")
get("/login", "sessions#new")

middleware("authenticate", -> {
  get("/dashboard", "dashboard#index")
  post("/settings", "users#update_settings")
})

middleware(["authenticate", "audit"], -> {
  get("/admin", "admin#index")
})
```

## Order

For each request, the route's scoped middleware and the global middleware are
merged into one list and sorted by `# order:`. At equal order a scoped
middleware runs before a global one; scoped middleware keep their declaration
order among themselves.

```soli
# app/middleware/origin.sl

ALLOWED_ORIGINS = ["https://app.example.com"]

# order: 5
# global_only: true
def check_origin
  origin = req["headers"]["origin"]
  if origin.present? && !ALLOWED_ORIGINS.includes?(origin)
    return {"status": 403, "body": "Origin not allowed"}
  end

  req["origin"] = origin || "same-origin"
  req
end
```

```soli
# app/middleware/stamp.sl

# order: 10
def stamp
  req["started_at"] = DateTime.utc.to_unix
  req
end
```

With `check_origin` (order 5, `global_only`), `stamp` (order 10, regular) and
`authenticate` (order 20, `scope_only`):

| Route | Runs |
|-------|------|
| `/dashboard` (inside `middleware("authenticate", ...)`) | `check_origin` → `stamp` → `authenticate` → action |
| `/` | `check_origin` → `stamp` → action |

Keys a middleware sets on the request (`req["origin"]`, `req["started_at"]`)
are visible to later middleware and to the controller action.

## Returning and Stopping

What a middleware returns decides what happens next:

| Return value | Effect |
|--------------|--------|
| The request hash (possibly modified) — `req` | Continue to the next middleware, then the action. |
| A response hash — anything with a `status` key: `redirect("/login")`, `{"status": 401, "body": "..."}` | Stop; this is the response. |
| `{"continue": true, "request": req}` | Continue (explicit form). |
| `{"continue": false, "response": {"status": 401, "body": "..."}}` | Stop with that response (explicit form). |
| Anything that is not a hash | `500`. |

```soli
# order: 30
# scope_only: true
def require_session
  return redirect("/login") if session_get("user_id").nil?

  req
end
```

`render_json` and `render_text` answer from a middleware too — whatever it
renders is the response:

```soli
# order: 15
# scope_only: true
def require_api_key
  return render_json({"error": "Unauthorized"}, 401) if req["headers"]["x-api-key"].blank?

  req
end
```

Raising also stops the chain, with the matching response:

| Raised by | Response |
|-----------|----------|
| `halt(status, message)` | `status` with `message` as a plain-text body |
| `forbidden(message?)` | `403` error page |
| `Model.find` with an unknown id | `404` error page |
| any other error | `500` — the full error page under `--dev`, a request id only in production |

`halt` raises, so a bare `halt(429, "Slow down")` ends the middleware on its
own; `return halt(...)` reads the same.

## Request Logging

Request logging is built into the server — always on under `--dev`, opt-in in
production with `SOLI_REQUEST_LOG=1`:

```
[LOG] GET /users - 200 (1.234ms)
[LOG] POST /login - 302 (12.876ms)
```

## CORS

Middleware runs before the action and returns a request or a response — it
does not add headers to the action's response. For CORS headers and preflights,
use the built-in `cors("/api/*", {...})` route helper (see
[Routing → CORS](routing.md#cors)).

## Best Practices

1. Keep each middleware to one job; put shared code in `_`-prefixed helpers.
2. Mark authentication `scope_only: true` so it never applies globally by accident.
3. Use `global_only: true` for checks that must run on every request.
4. Keep global middleware cheap — it runs on every request.
