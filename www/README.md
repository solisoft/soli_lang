# Solilang MVC Framework Example

A comprehensive demonstration of the Solilang MVC Framework with scoped middleware support.

## Prerequisites

- Rust and Cargo (latest stable)
- Node.js (v16 or higher)
- npm or yarn

## Installation

```bash
# Install dependencies
npm install
```

## Quick Start

```bash
# Start development server with hot reload and automatic Tailwind CSS compilation
soli serve . --dev

# Or use the shell script
./dev.sh
```

Visit [http://localhost:5011](http://localhost:5011) for the app, and [http://localhost:5011/docs](http://localhost:5011/docs) for full documentation.

## Tailwind CSS Integration

When running in dev mode (`--dev`), Soli automatically handles Tailwind CSS:

- **On startup**: Compiles Tailwind CSS automatically
- **On view changes**: Recompiles when you edit `.slv` template files
- **On source CSS changes**: Recompiles when files in `app/assets/css/` change

No need to run a separate Tailwind watcher - just use `soli serve . --dev`!

### Requirements for Tailwind integration

- `tailwind.config.js` in project root
- `package.json` with a `build:css` script
- Node.js and npm installed

### Production

For production, use without the `--dev` flag:

```bash
soli serve .
```

## Project Structure

```
mvc_app/
├── app/
│   ├── controllers/          # Route handlers
│   │   ├── home_controller.sl
│   │   ├── users_controller.sl
│   │   └── admin_controller.sl
│   ├── middleware/           # Middleware functions
│   │   ├── auth.sl         # Authentication (scope_only)
│   │   ├── cors.sl         # CORS headers (global_only)
│   │   └── logging.sl      # Request logging (global_only)
│   ├── models/               # Data models
│   └── views/                # Templates
│       ├── home/
│       └── layouts/
├── config/
│   └── routes.sl           # Route definitions
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

**Full documentation available at:** [http://localhost:5011/docs](http://localhost:5011/docs)

### Topics Covered

- **Getting Started** - Installation and quick start guide
- **Architecture** - MVC pattern explanation
- **Controllers** - Defining route handlers
- **Middleware** - Request/response processing
- **Views** - Template rendering and layouts
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

Available helpers in `routes.sl`:

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
// config/routes.sl

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

### E2E Controller Testing

Solilang provides a Rails-like E2E testing framework for testing your controllers with real HTTP requests:

```soli
describe("PostsController", fn() {
  before_each(fn() {
    as_guest();
    });

  test("creates post", fn() {
    login("user@example.com", "password");

    response = post("/posts", {
      "title": "New Post",
      "body": "Content"
      });

    assert_eq(res_status(response), 201);
    data = res_json(response);
    assert_eq(data["title"], "New Post");
    });
  });

```

#### Request Helpers

| Function | Description |
|----------|-------------|
| `get(path)` | GET request |
| `post(path, data)` | POST with body |
| `put(path, data)` | PUT replacement |
| `patch(path, data)` | PATCH partial update |
| `delete(path)` | DELETE resource |
| `head(path)`, `options(path)` | Other methods |
| `set_header(name, value)` | Custom headers |
| `set_authorization(token)` | Bearer token auth |
| `set_cookie(name, value)` | Session cookies |

#### Response Helpers

| Function | Description |
|----------|-------------|
| `res_status(response)` | HTTP status code |
| `res_body(response)` | Response body string |
| `res_json(response)` | Parsed JSON response |
| `res_header(response, name)` | Specific header |
| `res_redirect(response)` | Is redirect? |
| `res_ok(response)` | 2xx status? |
| `res_client_error(response)` | 4xx status? |
| `res_server_error(response)` | 5xx status? |

#### Session Helpers

| Function | Description |
|----------|-------------|
| `as_guest()`, `as_user(id)`, `as_admin()` | Set auth state |
| `login(email, password)`, `logout()` | Session management |
| `signed_in()`, `signed_out()` | Auth check |
| `with_token(token)` | JWT authentication |

#### Running Tests

```bash
# Run E2E tests
soli test tests/builtins/controller_integration_spec.sl

# Run all tests
soli test tests/builtins

# With coverage
soli test tests/builtins --coverage
```

**Documentation:** See [docs/testing-guide.md](docs/testing-guide.md) for comprehensive testing documentation.

### Public Routes

```bash
curl http://localhost:5011/
curl http://localhost:5011/health
curl http://localhost:5011/docs
```

### Documentation Routes

```bash
curl http://localhost:5011/docs/introduction
curl http://localhost:5011/docs/installation
curl http://localhost:5011/docs/routing
curl http://localhost:5011/docs/controllers
curl http://localhost:5011/docs/middleware
curl http://localhost:5011/docs/views
```

## 🔥 Hot Reload

The development server supports hot reload:

- Edit controllers → changes apply immediately
- Edit middleware → changes apply immediately
- Edit routes → routes are reloaded
- Edit templates → pages are refreshed + **Tailwind CSS recompiles automatically**
- Edit source CSS (`app/assets/css/`) → Tailwind CSS recompiles automatically

No restart needed!

## 📦 Middleware Reference

| Middleware | Type | Description |
|------------|------|-------------|
| `cors` | global_only | Adds CORS headers to all responses |
| `logging` | global_only | Logs all HTTP requests |
| `authenticate` | scope_only | Requires API key authentication (ready to use in routes) |

## 🏗️ Creating New Controllers

1. Create `app/controllers/name_controller.sl`:

```soli
fn index(req: Any) {
  return {"status": 200, "body": "Hello!"};
}

fn show(req: Any) {
  id = req["params"]["id"];
  return {"status": 200, "body": "User " + id};
}

```

2. Add routes in `config/routes.sl`:

```soli
get("/users", "users#index");
get("/users/:id", "users#show");

```

## 🔒 Security Notes

- The `authenticate` middleware uses a demo API key (`secret-key-123`)
- In production, use proper authentication (JWT, sessions, etc.)
- CORS is configured for development; configure properly for production

## 📖 Learn More

- **Full Docs:** [http://localhost:5011/docs](http://localhost:5011/docs)
- **GitHub:** https://github.com/solilang/solilang
- **Examples:** See `examples/mvc_app/`

---

Built with ❤️ using [Solilang](https://github.com/solilang/solilang)
