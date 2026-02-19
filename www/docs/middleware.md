# Middleware

Middleware provides a way to filter HTTP requests and responses.

## Creating Middleware

Create a file in `app/middleware/`:

```soli
# app/middleware/auth.sl
fn authenticate(req)
    let token = req.headers["Authorization"];

    if token == null || token == ""
        return error(401, "Unauthorized");
    end

    # Validate token...
    req
end
```

## Built-in Middleware

### Logging

Log all incoming requests:

```soli
# app/middleware/logging.sl
fn log_request(req)
    let timestamp = datetime::now();
    println(timestamp + " " + req.method + " " + req.path);
    req
end
```

### CORS

Handle Cross-Origin Resource Sharing:

```soli
# app/middleware/cors.sl
fn cors(req)
    let response = req;
    response.headers["Access-Control-Allow-Origin"] = "*";
    response.headers["Access-Control-Allow-Methods"] = "GET, POST, PUT, DELETE, OPTIONS";
    response.headers["Access-Control-Allow-Headers"] = "Content-Type, Authorization";
    response
end
```

### Authentication

```soli
# app/middleware/auth.sl
fn auth(req)
    let session = req.cookies["session"];

    if session == null
        return redirect("/login");
    end

    # Verify session...
    req
end
```

## Applying Middleware

### In Routes

```soli
# config/routes.sl
get("/dashboard", "dashboard#index", ["auth", "logging"]);
get("/admin", "admin#panel", ["auth", "admin_only"]);
```

### Global Middleware

Apply to all routes in `config/routes.sl`:

```soli
# Apply logging to all routes
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
fn add_locale(req)
    let lang = req.query["lang"] ?? "en";
    req.locale = lang;
    req
end
```

### Modify Response

```soli
fn add_headers(req, response)
    response.headers["X-Frame-Options"] = "SAMEORIGIN";
    response.headers["X-Content-Type-Options"] = "nosniff";
    response
end
```

## Error Handling Middleware

```soli
fn handle_errors(req, error)
    error(500, "Internal Server Error")
end
```

## Best Practices

1. Keep middleware focused and single-purpose
2. Use middleware for cross-cutting concerns
3. Order middleware logically (auth before business logic)
4. Handle errors gracefully
5. Don't leak sensitive information in logs
