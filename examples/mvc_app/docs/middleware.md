# Middleware

Middleware provides a way to filter HTTP requests and responses.

## Creating Middleware

Create a file in `app/middleware/`:

```soli
// app/middleware/auth.soli
fn authenticate(req: Any) -> Any {
    let token = req.headers["Authorization"];

    if token == null || token == "" {
        return error(401, "Unauthorized");
    }

    // Validate token...
    return req;
}
```

## Built-in Middleware

### Logging

Log all incoming requests:

```soli
// app/middleware/logging.soli
fn log_request(req: Any) -> Any {
    let timestamp = datetime::now();
    println(timestamp + " " + req.method + " " + req.path);
    return req;
}
```

### CORS

Handle Cross-Origin Resource Sharing:

```soli
// app/middleware/cors.soli
fn cors(req: Any) -> Any {
    let response = req;
    response.headers["Access-Control-Allow-Origin"] = "*";
    response.headers["Access-Control-Allow-Methods"] = "GET, POST, PUT, DELETE, OPTIONS";
    response.headers["Access-Control-Allow-Headers"] = "Content-Type, Authorization";
    return response;
}
```

### Authentication

```soli
// app/middleware/auth.soli
fn auth(req: Any) -> Any {
    let session = req.cookies["session"];

    if session == null {
        return redirect("/login");
    }

    // Verify session...
    return req;
}
```

## Applying Middleware

### In Routes

```soli
// config/routes.soli
get("/dashboard", "dashboard#index", ["auth", "logging"]);
get("/admin", "admin#panel", ["auth", "admin_only"]);
```

### Global Middleware

Apply to all routes in `config/routes.soli`:

```soli
// Apply logging to all routes
use("middleware/logging");

get("/", "home#index");
get("/about", "home#about");
```

## Middleware Stack Order

Middleware executes in the order it's applied:

```
Request -> Logging -> CORS -> Auth -> Controller -> Auth -> CORS -> Response
```

## Request/Response Modification

### Modify Request

```soli
fn add_locale(req: Any) -> Any {
    let lang = req.query["lang"] ?? "en";
    req.locale = lang;
    return req;
}
```

### Modify Response

```soli
fn add_headers(req: Any, response: Any) -> Any {
    response.headers["X-Frame-Options"] = "SAMEORIGIN";
    response.headers["X-Content-Type-Options"] = "nosniff";
    return response;
}
```

## Error Handling Middleware

```soli
fn handle_errors(req: Any, error: Any) -> Any {
    return error(500, "Internal Server Error");
}
```

## Best Practices

1. Keep middleware focused and single-purpose
2. Use middleware for cross-cutting concerns
3. Order middleware logically (auth before business logic)
4. Handle errors gracefully
5. Don't leak sensitive information in logs
