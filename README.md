<p align="center">
  <img src="www/public/images/soli-logo.svg" alt="Soli Logo" width="120" height="120" />
</p>

<h1 align="center">Soli MVC Framework</h1>

<p align="center">
  <strong>A dynamically-typed, high-performance web framework with optional type annotations.</strong>
</p>

<p align="center">
  <a href="#features">Features</a> •
  <a href="#quick-start">Quick Start</a> •
  <a href="#documentation">Documentation</a> •
  <a href="#examples">Examples</a> •
  <a href="#contributing">Contributing</a>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License" />
  <img src="https://img.shields.io/badge/rust-1.70+-orange.svg" alt="Rust Version" />
  <img src="https://img.shields.io/badge/performance-170k%20req%2Fs-green.svg" alt="Performance" />
</p>

---

## Why Soli?

Soli is a full-stack MVC framework written in Rust that brings the elegance of Ruby/Rails to the performance of systems programming. Build web applications with expressive, readable code while enjoying sub-millisecond response times.

```soli
// Define a controller
fn index(req: Any) -> Any {
    let posts = Post
        .where("doc.published == true")
        .order("created_at", "desc")
        .limit(10)
        .all();

    return render("posts/index", {
        "title": "Latest Posts",
        "posts": posts
    });
}
```

## Features

### Performance
- **170,000+ requests/second** on a single server
- **Sub-millisecond response times** for most requests
- **Zero-copy JSON parsing** and efficient memory management
- **Bytecode compilation** for fast execution

### Developer Experience
- **Hot reload** - See changes instantly without restart
- **Beautiful error pages** with variable inspection in dev mode
- **Scaffold generator** - Generate complete MVC resources in seconds
- **Convention over configuration** - Sensible defaults, less boilerplate

### Full-Stack Features
- **ERB-style templates** with layouts and partials
- **Active Record ORM** with migrations, validations, and relationships
- **WebSocket support** with Live View for reactive UIs
- **Built-in authentication** with JWT and session management
- **i18n support** for multi-language applications
- **Tailwind CSS integration** with automatic compilation

### Security
- CSRF protection
- XSS sanitization
- Secure session cookies
- Input validation helpers

## Quick Start

### Installation

```bash
# Clone the repository
git clone https://github.com/solisoft/soli_lang.git
cd soli_lang

# Build the release binary
cargo build --release

# Add to PATH (optional)
export PATH="$PATH:$(pwd)/target/release"
```

### Create a New Project

```bash
# Generate a new MVC application
soli new my_app
cd my_app

# Install frontend dependencies (for Tailwind CSS)
npm install

# Start the development server
soli serve . --dev
```

Visit [http://localhost:3000](http://localhost:3000) to see your app!

### Generate a Resource

```bash
# Generate a complete blog posts scaffold
soli generate scaffold posts title:string content:text author:string published:boolean

# Run migrations
soli migrate
```

This creates:
- Model with validations
- Controller with CRUD actions
- Views (index, show, new, edit)
- Database migration
- Test files
- Routes configuration

## Project Structure

```
my_app/
├── app/
│   ├── controllers/      # Route handlers
│   ├── models/           # Data models with ORM
│   ├── views/            # ERB templates
│   │   └── layouts/      # Layout templates
│   ├── middleware/       # Request/response middleware
│   └── helpers/          # View helper functions
├── config/
│   ├── routes.sl         # Route definitions
│   └── database.sl       # Database configuration
├── db/
│   └── migrations/       # Database migrations
├── public/               # Static assets
├── tests/                # Test files
└── package.json          # Frontend dependencies
```

## Documentation

Full documentation is available at [http://localhost:3000/docs](http://localhost:3000/docs) when running the server, covering:

- **Getting Started** - Installation and first steps
- **Core Concepts** - Routing, controllers, middleware, views
- **Database** - Configuration, models, migrations
- **Security** - Authentication, sessions, validation
- **Live View** - Real-time reactive components
- **Language Reference** - Soli language syntax and features
- **Built-in Functions** - Complete API reference

## Examples

### Routing

```soli
// config/routes.sl

// Simple routes
get("/", "home#index");
get("/about", "home#about");

// RESTful resources
resources("posts");
resources("users");

// Nested resources
resources("posts", fn() {
    resources("comments");
});

// Scoped middleware
middleware("authenticate", fn() {
    get("/admin", "admin#index");
    resources("admin/users");
});

// API namespace
namespace("api/v1", fn() {
    resources("posts");
});
```

### Models

```soli
class Post extends Model {
    validates("title", { "presence": true, "min_length": 3 });
    validates("content", { "presence": true });

    fn author() -> Any {
        return User.find(this.author_id);
    }

    fn comments() -> Any {
        return Comment.where("doc.post_id == @id", { "id": this.id });
    }
}
```

### Controllers

```soli
fn create(req: Any) -> Any {
    let params = req["params"];
    let result = Post.create(params);

    if result["valid"] {
        return redirect("/posts/" + result["record"]["id"]);
    }

    return render("posts/new", {
        "errors": result["errors"],
        "post": params
    });
}
```

### Views

```erb
<!-- app/views/posts/index.html.slv -->
<h1><%= title %></h1>

<% for post in posts %>
    <article>
        <h2><%= post["title"] %></h2>
        <p><%= post["excerpt"] %></p>
        <a href="/posts/<%= post["id"] %>">Read more</a>
    </article>
<% end %>
```

### Live View

Build reactive UIs without writing JavaScript:

```html
<!-- app/views/live/counter.sliv -->
<div class="counter">
    <h2>@count</h2>
    <button soli-click="decrement">-</button>
    <button soli-click="increment">+</button>
</div>
```

```soli
// app/controllers/live_controller.sl
fn counter(event: Any) -> Any {
    let count = event["state"]["count"] ?? 0;

    if event["event"] == "increment" {
        count = count + 1;
    } elsif event["event"] == "decrement" {
        count = count - 1;
    }

    return { "state": { "count": count } };
}
```

## Testing

Soli includes a comprehensive testing framework:

```soli
describe("PostsController", fn() {
    before_each(fn() {
        as_guest();
    });

    test("lists all posts", fn() {
        let response = get("/posts");
        assert_eq(res_status(response), 200);
    });

    test("creates post when authenticated", fn() {
        login("user@example.com", "password");

        let response = post("/posts", {
            "title": "New Post",
            "content": "Content here"
        });

        assert_eq(res_status(response), 201);
    });
});
```

Run tests:

```bash
# Run all tests
soli test tests/

# Run with coverage
soli test tests/ --coverage

# Run specific test file
soli test tests/controllers/posts_test.sl
```

## Performance

Benchmarked on a standard server (16 cores):

| Metric | Value |
|--------|-------|
| Requests/sec | 172,409 |
| Avg Latency | 0.58ms |
| Transfer/sec | 23.01 MB |

```bash
$ wrk -t12 -c400 -d30s http://localhost:3000/
Running 30s test @ http://localhost:3000/
  12 threads and 400 connections
  Thread Stats   Avg      Stdev     Max   +/- Stdev
    Latency   0.58ms    0.89ms  42.31ms   98.76%
    Req/Sec  14.52k     1.23k   18.67k    74.58%
  5172270 requests in 30.00s, 690.30MB read
Requests/sec: 172409.55
Transfer/sec:     23.01MB
```

## IDE Support

### VS Code Extension

Install the Soli language extension for VS Code:

```bash
cd editors/vscode
npm install
npm run build
code --install-extension soli-lang-*.vsix
```

Features:
- Syntax highlighting for `.sl` and `.sliv` files
- Code snippets
- Error diagnostics

## Contributing

Contributions are welcome! Please read our contributing guidelines before submitting PRs.

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Acknowledgments

- Inspired by Ruby on Rails, Phoenix Framework, and Laravel
- Built with Rust for performance and safety
- Uses ArangoDB for flexible document storage

---

<p align="center">
  Built with love by <a href="https://github.com/solisoft">solisoft</a>
</p>
# Trigger release
