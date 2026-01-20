# Solilang MVC Framework Example

A comprehensive demonstration of the Solilang MVC Framework with scoped middleware support.

## 🚀 Quick Start

```bash
# Start the development server
cargo run -- serve examples/mvc_app
```

Visit [http://localhost:3000/docs](http://localhost:3000/docs) for full documentation.

## 📁 Project Structure

```
mvc_app/
├── app/
│   ├── controllers/          # Route handlers
│   │   ├── home_controller.soli
│   │   ├── users_controller.soli
│   │   └── admin_controller.soli
│   ├── middleware/           # Middleware functions
│   │   ├── auth.soli         # Authentication (scope_only)
│   │   ├── cors.soli         # CORS headers (global_only)
│   │   └── logging.soli      # Request logging (global_only)
│   ├── models/               # Data models
│   └── views/                # Templates
│       ├── home/
│       └── layouts/
├── config/
│   └── routes.soli           # Route definitions
├── public/                   # Static files
│   └── docs.html             # Framework documentation
└── README.md                 # This file
```

## 🎯 Key Features

### 1. Scoped Middleware

Apply middleware only to specific routes:

```soli
// Only /admin/* routes require authentication
middleware("authenticate", -> {
    get("/admin", "admin#index");
    get("/admin/users", "admin#users");
});

// Public routes - no auth needed
get("/", "home#index");
```

### 2. Middleware Types

| Type | Option | Behavior |
|------|--------|----------|
| Global-Only | `// global_only: true` | Runs for ALL routes, cannot be scoped |
| Scope-Only | `// scope_only: true` | Only runs when explicitly scoped |
| Regular | No option | Runs globally by default, can also be scoped |

### 3. RESTful Resources

Generate standard CRUD routes automatically:

```soli
resources("users", null);
```

Creates: `GET /users`, `POST /users`, `GET /users/:id`, etc.

### 4. Namespaces

Group routes under a common prefix:

```soli
namespace("api", -> {
    middleware("authenticate", -> {
        get("/api/users", "users#index");
    });
});
```

## 📚 Documentation

**Full documentation available at:** [http://localhost:3000/docs](http://localhost:3000/docs)

### Topics Covered

- **Getting Started** - Installation and quick start guide
- **Architecture** - MVC pattern explanation
- **Controllers** - Defining route handlers
- **Middleware** - Request/response processing
- **Scoped Middleware** - Fine-grained middleware control
- **Routing** - DSL helpers and patterns
- **Resources** - RESTful route generation
- **Configuration** - Middleware options

## 🔧 Configuration

### Middleware Options

Configure middleware behavior using special comments:

```soli
// order: 20          // Execution order (lower runs first)
// global_only: true  // Only runs globally, cannot be scoped
// scope_only: true   // Only runs when explicitly scoped
```

### Route DSL

Available helpers in `routes.soli`:

| Helper | Description |
|--------|-------------|
| `get(path, action)` | GET request |
| `post(path, action)` | POST request |
| `put(path, action)` | PUT request |
| `delete(path, action)` | DELETE request |
| `patch(path, action)` | PATCH request |
| `resources(name, block)` | RESTful resource routes |
| `namespace(name, block)` | Route grouping |
| `middleware(name, block)` | Scoped middleware |

## 📝 Example Routes

```soli
// config/routes.soli

// Scoped authentication middleware
middleware("authenticate", -> {
    get("/admin", "admin#index");
    get("/admin/users", "admin#users");
});

// Public routes
get("/", "home#index");
get("/about", "home#about");

// RESTful resources
resources("users", null);
```

## 🧪 Testing

### Public Routes (No Authentication)

```bash
curl http://localhost:3000/
curl http://localhost:3000/about
curl http://localhost:3000/users
```

### Protected Routes (Require Authentication)

```bash
# Without API key - returns 401
curl http://localhost:3000/admin

# With valid API key - succeeds
curl -H "X-Api-Key: secret-key-123" http://localhost:3000/admin
```

## 🔥 Hot Reload

The development server supports hot reload:

- Edit controllers → changes apply immediately
- Edit middleware → changes apply immediately
- Edit routes → routes are reloaded
- Edit templates → pages are refreshed

No restart needed!

## 📦 Middleware Reference

| Middleware | Type | Description |
|------------|------|-------------|
| `cors` | global_only | Adds CORS headers to all responses |
| `logging` | global_only | Logs all HTTP requests |
| `authenticate` | scope_only | Requires API key authentication |

## 🏗️ Creating New Controllers

1. Create `app/controllers/name_controller.soli`:

```soli
fn index(req: Any) -> Any {
    return {"status": 200, "body": "Hello!"};
}

fn show(req: Any) -> Any {
    let id = req["params"]["id"];
    return {"status": 200, "body": "User " + id};
}
```

2. Add routes in `config/routes.soli`:

```soli
get("/users", "users#index");
get("/users/:id", "users#show");
```

## 🔒 Security Notes

- The `authenticate` middleware uses a demo API key (`secret-key-123`)
- In production, use proper authentication (JWT, sessions, etc.)
- CORS is configured for development; configure properly for production

## 📖 Learn More

- **Full Docs:** [http://localhost:3000/docs](http://localhost:3000/docs)
- **GitHub:** https://github.com/solilang/solilang
- **Examples:** See `examples/mvc_app/`

---

Built with ❤️ using [Solilang](https://github.com/solilang/solilang)
