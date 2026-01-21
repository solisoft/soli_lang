# Routing

SoliLang MVC uses a simple, declarative routing system.

## Basic Routes

Define routes in `config/routes.soli`:

```soli
// HTTP method, path, controller#action
get("/", "home#index");
post("/submit", "home#submit");
put("/update/:id", "home#update");
delete("/delete/:id", "home#delete");
```

## Route Methods

| Method | Description |
|--------|-------------|
| `get(path, handler)` | GET request |
| `post(path, handler)` | POST request |
| `put(path, handler)` | PUT request |
| `delete(path, handler)` | DELETE request |
| `patch(path, handler)` | PATCH request |

## Route Parameters

Capture dynamic segments with `:param`:

```soli
get("/users/:id", "users#show");
get("/posts/:post_id/comments/:comment_id", "comments#show");
```

Access parameters in your controller:

```soli
fn show(req: Any) -> Any {
    let user_id = req.params["id"];
    // ...
}
```

## Query Strings

Query parameters are automatically parsed:

```soli
// URL: /search?q=solilang&page=1
fn search(req: Any) -> Any {
    let query = req.query["q"];  // "solilang"
    let page = req.query["page"]; // "1"
    // ...
}
```

## Route Helpers

### route_match

Check if a path matches a route pattern:

```soli
if route_match("/users/:id", "/users/123") {
    println("Matches!");
}
```

### named_routes

Generate URLs from route names:

```soli
let url = named_routes["user_show"].("/users/:id", {"id": 123});
// Returns: "/users/123"
```

## RESTful Routes Example

```soli
// Resources
resources("/users", "users");

// Generates:
// GET    /users           -> users#index
// GET    /users/new       -> users#new
// POST   /users           -> users#create
// GET    /users/:id       -> users#show
// GET    /users/:id/edit  -> users#edit
// PUT    /users/:id       -> users#update
// DELETE /users/:id       -> users#destroy
```

## Middleware on Routes

Apply middleware to specific routes:

```soli
get("/admin", "admin#dashboard", ["auth"]);
get("/profile", "user#profile", ["auth", "verified"]);
```

## Route Groups

Group routes with common prefixes:

```soli
group("/api", [], {
    get("/users", "api#users");
    get("/posts", "api#posts");
});
```

## Best Practices

1. Use RESTful conventions for CRUD operations
2. Keep routes simple and predictable
3. Use route parameters for resource identifiers
4. Apply authentication/authorization middleware where needed
