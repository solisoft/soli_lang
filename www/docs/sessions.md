# Session Management

SoliLang provides built-in session management with cookie-based session IDs and in-memory storage.

## Enabling Sessions

Sessions are automatically available in your controllers. The session cookie is set on all responses.

## Basic Operations

### Reading Session Data

```soli
fn profile
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
fn login
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
fn is_logged_in() -> Bool
  session_has("authenticated") && session_get("authenticated") == true
end
```

### Deleting Session Data

```soli
fn remove_item
  removed = session_delete("temporary_data");
  print("Removed:", removed);
  {"status": 200}
end
```

## Session Security

### Regenerate After Login

Always regenerate the session ID after successful authentication to prevent session fixation:

```soli
fn login
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
fn logout
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
fn require_auth
  if !session_has("authenticated") || session_get("authenticated") != true
    return {
      "status": 401,
      "body": json_stringify({"error": "Authentication required"})
    };
  end
  null  # Allow request to continue
end

fn require_role(req, required_role: String)
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

Sessions are stored in-memory by default. This means:
- Sessions survive server restarts (data is in memory)
- Sessions are shared across all server threads
- For production, consider persistent storage (Redis, database)

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
