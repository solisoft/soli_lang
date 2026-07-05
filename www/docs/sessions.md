# Session Management

SoliLang provides built-in session management with cookie-based session IDs and in-memory storage.

## Enabling Sessions

Sessions are automatically available in your controllers. The session cookie is set on all responses.

## Basic Operations

### Reading Session Data

```soli
def profile
  # Check if user is logged in
  if session_get("authenticated") != true
    return {"status": 401, "body": "Please log in"};
  end

  {
    "status": 200,
    "body": json_stringify({
      "user": session_get("user"),
      "email": session_get("email")
    })
  }
end
```

### Writing Session Data

```soli
def login
  data = req["json"];
  username = data["username"];

  # Store user data in session
  session_set("user", username);
  session_set("role", "admin");
  session_set("authenticated", true);

  {
    "status": 200,
    "body": json_stringify({"success": true})
  }
end
```

### Checking Session State

```soli
def is_logged_in() -> Bool
  session_has("authenticated") && session_get("authenticated") == true
end
```

### Deleting Session Data

```soli
def remove_item
  removed = session_delete("temporary_data");
  print("Removed:", removed);
  {"status": 200}
end
```

## Session Security

### Regenerate After Login

Always regenerate the session ID after successful authentication to prevent session fixation:

```soli
def login
  data = req["json"];

  if verify_credentials(data["username"], data["password"])
    # Regenerate session for security
    session_regenerate();

    # Now set auth data
    session_set("user_id", get_user_id(data["username"]));
    session_set("authenticated", true);

    return {"status": 200, "body": "Logged in"};
  end

  {"status": 401, "body": "Invalid credentials"}
end
```

### Destroy Session on Logout

```soli
def logout
  session_destroy();

  {
    "status": 200,
    "body": json_stringify({"success": true})
  }
end
```

## Session Middleware Example

Create a reusable authentication middleware:

```soli
# app/middleware/auth.sl
def require_auth
  if !session_has("authenticated") || session_get("authenticated") != true
    return {
      "status": 401,
      "body": json_stringify({"error": "Authentication required"})
    };
  end
  null  # Allow request to continue
end

def require_role(req, required_role: String)
  result = require_auth(req);
  if result != null
    return result;  # Return auth error
  end

  user_role = session_get("role");
  if user_role != required_role
    return {
      "status": 403,
      "body": json_stringify({"error": "Insufficient permissions"})
    };
  end

  null
end
```

Use in routes:

```soli
# config/routes.sl
get("/profile", "user#profile", ["auth"]);
get("/admin", "admin#dashboard", ["auth", "role:admin"]);
post("/users", "users#create", ["auth", "role:admin"]);
```

## API Reference

| Function | Description |
|----------|-------------|
| `session_id()` | Returns current session ID or `null` |
| `session_get(key)` | Get value from session |
| `session_set(key, value)` | Store value in session |
| `session_has(key)` | Check if key exists |
| `session_delete(key)` | Remove key and return value |
| `session_regenerate()` | Create new session ID |
| `session_destroy()` | Destroy entire session |

## Storage

Sessions are stored in-memory by default: fast, but lost on restart and not
shared between hosts. For production, pick a driver via `SOLI_SESSION_DRIVER`
(or `session_configure`):

| Driver | Description | Requires |
|--------|-------------|----------|
| `in_memory` | Default. Fast, lost on restart. | — |
| `cookie` | Encrypted client-side sessions — the whole payload travels in the cookie. Survives restarts, works across hosts, zero infrastructure. | `SOLI_SESSION_SECRET` |
| `disk` | JSON files on disk. | `SOLI_SESSION_PATH` |
| `solidb` | SolidB HTTP database. | `SOLI_SOLIDB_*` |
| `solikv` | SoliKV/Redis with TTL. | `SOLI_SOLIKV_*` |

### Encrypted cookie sessions

The `cookie` driver stores the session on the client, Rails-style: the whole
payload is sealed with AES-256-GCM and shipped in the session cookie. Nothing
is persisted server-side, so sessions survive restarts and work across
load-balanced hosts with no session database.

```bash
export SOLI_SESSION_DRIVER=cookie
export SOLI_SESSION_SECRET=$(openssl rand -hex 32)   # 32+ characters, keep stable
```

Or at runtime:

```soli
session_configure({"driver": "cookie", "secret": getenv("SOLI_SESSION_SECRET")})
```

How it works:

- The cookie value is `v1.<base64url(...)>` — an encrypted, authenticated
  blob. The AES key is derived (HKDF-SHA256) from `SOLI_SESSION_SECRET`.
- Clients can neither read nor forge session contents. A tampered, expired,
  or foreign-key blob is silently replaced by a fresh empty session, exactly
  like an unknown session ID on the server-side drivers.
- `session_id()` still returns a stable internal UUID (carried inside the
  payload), not the blob.
- The cookie is only re-emitted when the session actually changed, so
  read-only requests stay cacheable.

Trade-offs to be aware of:

- **~4KB ceiling.** Browsers cap cookies at 4096 bytes. An oversized session
  refuses to seal — the write is dropped with a loud log line and the
  client keeps its previous cookie. Store identifiers (`user_id`), not
  records.
- **No server-side revocation.** `session_destroy` overwrites the client's
  copy, but a stolen cookie stays valid until its TTL passes. Rotating
  `SOLI_SESSION_SECRET` is the kill switch — it invalidates every
  outstanding session at once.
- **TTL counts from the last write.** Expiry uses a timestamp sealed inside
  the payload, refreshed each time the session is written.

If you need instant logout-everywhere or sessions bigger than a cookie,
use a server-side driver (`solidb`, `solikv`, `disk`) instead.

## Readiness and zero-downtime deploys

When a network-backed session driver (`solidb` or `solikv`) is configured, a
freshly-booted process must open its first connection to the session store
before it can serve a request that touches the session. To keep that
cold-start off the request path, Soli warms the connection at boot and exposes
a built-in readiness endpoint:

| Endpoint | Behavior |
|----------|----------|
| `GET /up` | Returns `503 warming` until the session store's connection has been warmed, then `200 ready`. For in-memory/disk/cookie drivers it is ready immediately. |

The warm-up retries with backoff until the session store is reachable, so a
session DB that is briefly unavailable at boot does not leave the process
permanently un-ready.

Point your load balancer's health check at `/up` so traffic is only routed to
an instance that can actually serve session-backed requests. Under
[soli-proxy](https://www.solisoft.net), auto-detected Soli apps already use
`/up` as the blue/green promotion gate — the previous slot keeps serving until
the new slot reports ready, eliminating the post-deploy window where the first
requests would otherwise stall on a cold session connection. `/up` is a
built-in route; defining your own `/up` in `config/routes.sl` has no effect.

## Cookie Settings

Session cookies are automatically configured with:
- `HttpOnly`: Prevents JavaScript access
- `SameSite=Lax`: CSRF protection (override with `SOLI_SESSION_SAMESITE=Strict|None`). When set to `None`, Soli automatically forces `Secure` on the cookie regardless of the detected request scheme — browsers reject `SameSite=None` without `Secure`, so the pairing is non-optional.
- `Path=/`: Available on all paths
- `Max-Age`: tracks `SOLI_SESSION_TTL` (default `86400` — 24h)
- `Secure`: set when serving over HTTPS

### Hardening with `__Host-` prefix

Set `SOLI_SESSION_HOST_PREFIX=1` to emit the cookie as `__Host-session_id`. Browsers only accept `__Host-` cookies when they are `Secure`, scoped to `Path=/`, and carry no `Domain` attribute — this prevents a subdomain or stripped-down HTTP origin from setting an attacker-controlled session cookie that would otherwise be replayed to the secure origin. The prefix is applied only when `Secure` is also active; over plain HTTP the plain `session_id` name is used so dev still works.

```bash
# Production session hardening
export SOLI_SESSION_SAMESITE=Strict
export SOLI_SESSION_HOST_PREFIX=1
```
