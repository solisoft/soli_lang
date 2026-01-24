# Session Management

SoliLang provides built-in session management with cookie-based session IDs and in-memory storage.

## Enabling Sessions

Sessions are automatically available in your controllers. The session cookie is set on all responses.

## Basic Operations

### Reading Session Data

```soli
fn profile(req: Any) -> Any {
    // Check if user is logged in
    if session_get("authenticated") != true {
        return {"status": 401, "body": "Please log in"};
    }

    return {
        "status": 200,
        "body": json_stringify({
            "user": session_get("user"),
            "email": session_get("email")
        })
    };
}
```

### Writing Session Data

```soli
fn login(req: Any) -> Any {
    let data = req["json"];
    let username = data["username"];

    // Store user data in session
    session_set("user", username);
    session_set("role", "admin");
    session_set("authenticated", true);

    return {
        "status": 200,
        "body": json_stringify({"success": true})
    };
}
```

### Checking Session State

```soli
fn is_logged_in() -> Bool {
    return session_has("authenticated") && session_get("authenticated") == true;
}
```

### Deleting Session Data

```soli
fn remove_item(req: Any) -> Any {
    let removed = session_delete("temporary_data");
    print("Removed:", removed);
    return {"status": 200};
}
```

## Session Security

### Regenerate After Login

Always regenerate the session ID after successful authentication to prevent session fixation:

```soli
fn login(req: Any) -> Any {
    let data = req["json"];

    if verify_credentials(data["username"], data["password"]) {
        // Regenerate session for security
        session_regenerate();

        // Now set auth data
        session_set("user_id", get_user_id(data["username"]));
        session_set("authenticated", true);

        return {"status": 200, "body": "Logged in"};
    }

    return {"status": 401, "body": "Invalid credentials"};
}
```

### Destroy Session on Logout

```soli
fn logout(req: Any) -> Any {
    session_destroy();

    return {
        "status": 200,
        "body": json_stringify({"success": true})
    };
}
```

## Session Middleware Example

Create a reusable authentication middleware:

```soli
// app/middleware/auth.sl
fn require_auth(req: Any) -> Any {
    if !session_has("authenticated") || session_get("authenticated") != true {
        return {
            "status": 401,
            "body": json_stringify({"error": "Authentication required"})
        };
    }
    return null;  // Allow request to continue
}

fn require_role(req: Any, required_role: String) -> Any {
    let result = require_auth(req);
    if result != null {
        return result;  // Return auth error
    }

    let user_role = session_get("role");
    if user_role != required_role {
        return {
            "status": 403,
            "body": json_stringify({"error": "Insufficient permissions"})
        };
    }

    return null;
}
```

Use in routes:

```soli
// config/routes.sl
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
- `SameSite=Lax`: CSRF protection
- `Path=/`: Available on all paths
- `Max-Age=86400`: 24-hour expiration
