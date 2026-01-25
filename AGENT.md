# Soli Language Agent Context

**Soli** is a web-oriented programming language designed with strict minimum features. It focuses on the essential capabilities needed for server-side web development while maintaining simplicity and readability.

## Table of Contents

1. [Core Philosophy](#core-philosophy)
2. [Language Features](#language-features-minimal-set)
3. [Built-in Functions](#built-in-functions-web-focused)
4. [Web MVC Architecture](#web-mvc-architecture)
5. [AI/LLM Code Generation Guide](#ai-llm-code-generation-guide)
6. [Syntax Summary](#syntax-summary)
7. [Model ORM](#model-orm)
8. [Examples](#examples)

- **Minimum Features, Maximum Utility**: Only essential language constructs
- **Web-First**: Built for server-side web development (HTTP, templates, MVC)
- **Ruby-like Expressiveness**: Clean, natural syntax for common patterns
- **Statically Typed**: Catch errors at compile time with type inference

## Language Features (Minimal Set)

### Types
- `Int` - 64-bit integers
- `Float` - 64-bit floating point
- `String` - Unicode strings
- `Bool` - true/false
- `Null` - null/void
- `Array<T>` - homogeneous arrays
- `Hash<K, V>` - key-value maps

### Control Flow
- `if/else` - conditional branching
- `while` - loop with condition
- `for (x in iterable)` - iterator loops
- `return` - function return

### Functions
```soli
fn double(x: Int) -> Int { return x * 2; }
fn greet(name: String, prefix: String = "Hello") -> String {
    return prefix + " " + name;
}
```

### Lambdas/Closures
```soli
fn(x) x * 2           // expression body (implicit return)
fn(x) { return x * 2; }  // block body (explicit return)
```

### Iteration Methods
```soli
[1, 2, 3].map(fn(x) x * 2)      // [2, 4, 6]
[1, 2, 3].filter(fn(x) x > 1)   // [2, 3]
[1, 2, 3].each(fn(x) print(x))  // side effects
{"a": 1, "b": 2}.map(fn(pair) [pair[0], pair[1] * 2])
```

### Pipeline Operator
```soli
5 |> double() |> addOne()  // 11
[1, 2, 3] |> map(fn(x) x * 2) |> filter(fn(x) x > 2)
```

### Classes (Minimal OOP)
```soli
class User {
    name: String;
    email: String;

    new(n: String, e: String) {
        this.name = n;
        this.email = e;
    }

    fn greet() -> String {
        return "Hello, " + this.name;
    }
}
```

### Hash/Array Literals
```soli
// Arrays
let nums = [1, 2, 3, 4, 5];
let typed: Int[] = [1, 2, 3];

// Hashes (JSON-style or Ruby-style)
let user = {"name": "Alice", "age": 30};
let scores = {"Alice" => 95, "Bob" => 87};
```

### String Interpolation
```soli
let name = "Alice";
let msg = "Hello \(name)!";
```

### Modules/Imports
```soli
import "./utils/math.sl";
print(math.add(1, 2));
```

### Exception Handling (try/catch/throw)
```soli
// Basic try/catch
try {
    throw "Something went wrong";
    print("After throw");
} catch (error) {
    print("Caught: " + str(error));
}

// Try/catch/finally
try {
    risky_operation();
} catch (e) {
    print("Error: " + str(e));
} finally {
    print("Always runs");
}

// Throwing custom errors
throw ValueError.new("Invalid input");
throw RuntimeError.new("Something failed");

// Built-in error types: Error, ValueError, TypeError, KeyError, IndexError, RuntimeError
```

## Built-in Functions (Web-Focused)

### Arrays
- `len(arr)` - array length
- `push(arr, value)` - append
- `pop(arr)` - remove last
- `range(start, end)` - create numeric array

### Hashes
- `len(hash)` - entry count
- `keys(hash)` - array of keys
- `values(hash)` - array of values
- `entries(hash)` - array of [key, value] pairs
- `has_key(hash, key)` - check existence
- `delete(hash, key)` - remove entry
- `merge(h1, h2)` - combine hashes
- `clear(hash)` - remove all

### HTTP/Server
- `http_get(url)` - HTTP GET (returns future)
- `http_post(url, body)` - HTTP POST
- `http_server_listen(port)` - start HTTP server
- `json_parse(str)` - parse JSON
- `json_stringify(value)` - stringify to JSON

### Templates
- `render(template_name, data)` - render template with data
- `partial(template_name, data)` - include partial

### HTML/Escaping
- `html_escape(string)` - escape `<`, `>`, `&`, `"`, `'` for safe HTML output
- `html_unescape(string)` - convert HTML entities back to characters
- `sanitize_html(string)` - remove dangerous tags/attributes (XSS prevention)

## What Soli Does NOT Have

- No async/await (uses futures with auto-resolve)
- No pattern matching
- No generics (beyond Array<T> and Hash<K, V>)
- No macros
- No decorators
- No async generators
- No enums
- No traits/interfaces
- No union types
- No custom operators

## Web MVC Architecture

```
myapp/
├── config/
│   └── routes.sl      # URL routing rules
├── controllers/         # Request handlers
│   └── users_controller.sl
├── models/              # Data models
│   └── user.sl
├── views/               # Templates
│   ├── users/
│   │   ├── index.html.sl
│   │   └── show.html.sl
│   └── layouts/
│       └── application.html.sl
├── public/              # Static assets
└── main.sl            # Entry point
```

### Routes Example
```soli
get("/", fn() {
    return render("home/index", {});
});

get("/users", fn() {
    let users = db.query("FOR doc IN users RETURN doc");
    return render("users/index", {"users": users});
});

post("/users", fn() {
    let name = request.body["name"];
    db.query("INSERT { name: @name } INTO users", { "name": name });
    redirect("/users");
});
```

## Syntax Summary

| Feature | Syntax |
|---------|--------|
| Variable | `let x: Int = 5;` |
| Function | `fn name(params) -> Type { body }` |
| Lambda | `fn(x) x * 2` |
| Class | `class Name { fields; new() { } methods }` |
| Array literal | `[1, 2, 3]` |
| Hash literal | `{"key": value}` or `{"key" => value}` |
| Index | `arr[0]`, `hash["key"]` |
| Pipeline | `value |> fn()` |
| String interp | `"Hello \(name)"` |
| Import | `import "./file.sl"` |

## Execution Modes

- **Tree-walking interpreter**: Default, simple, portable
- **Bytecode VM**: Faster execution for larger codebases

## File Extension

- `.sl` - Soli source files

## Model ORM

Models provide an OOP interface to the database. Collection names are auto-derived from class names.

```soli
class User extends Model {
    validates("email", { "presence": true, "uniqueness": true })
    validates("name", { "min_length": 2 })

    before_save("normalize_email")

    fn normalize_email() -> Any {
        this.email = this.email.downcase();
    }
}

// CRUD Operations
let result = User.create({ "name": "Alice", "email": "alice@example.com" });
let user = User.find("user_id");
let users = User.all();
User.update("user_id", { "name": "Alice Smith" });
User.delete("user_id");
let count = User.count();

// Query Builder with SDBQL filters
let adults = User.where("doc.age >= @age", { "age": 18 }).all();
let results = User
    .where("doc.age >= @age AND doc.active == @active", { "age": 18, "active": true })
    .order("created_at", "desc")
    .limit(10)
    .offset(20)
    .all();

// Relationships
class Post extends Model {
    fn author() -> Any {
        return User.find(this.author_id);
    }
}
```

## AI/LLM Code Generation Guide

This section provides guidance for AI/LLM agents generating code for the Soli MVC framework.

### Framework Structure Overview

```
project/
├── .soli/                      # AI-friendly convention files
│   ├── context.json           # Framework metadata for AI agents
│   ├── conventions/           # Machine-readable conventions
│   │   ├── controller.json
│   │   ├── middleware.json
│   │   ├── routes.json
│   │   └── views.json
│   └── examples/              # Annotated examples
│       ├── controller.sl
│       ├── middleware.sl
│       └── routes.sl
├── www/                       # MVC framework web application
│   ├── app/
│   │   ├── controllers/       # Request handlers (.sl files)
│   │   ├── middleware/        # HTTP middleware functions
│   │   ├── models/            # Data models (extends Model)
│   │   ├── views/             # Templates and layouts
│   │   └── helpers/           # View helper functions
│   ├── config/
│   │   └── routes.sl          # Route definitions
│   └── public/                # Static assets
```

### Convention Files for AI Agents

AI agents should read `.soli/context.json` for framework metadata and `.soli/conventions/*.json` for detailed patterns.

**Key convention files:**
- `.soli/context.json` - Framework metadata, naming conventions, response types
- `.soli/conventions/controller.json` - Controller patterns, method signatures
- `.soli/conventions/middleware.json` - Middleware types, execution order
- `.soli/conventions/routes.json` - Route patterns, REST conventions
- `.soli/conventions/views.json` - Template syntax, variable access

### Controller Generation Patterns

**Basic CRUD Controller Template:**
```soli
class {Resource}Controller extends Controller {
    static {
        this.layout = "application";
    }
    
    fn index(req: Any) -> Any {
        let {resources} = {Resource}.all();
        return render("{resources}/index", {
            "{resources}": {resources},
            "title": "{Resource} List"
        });
    }
    
    fn show(req: Any) -> Any {
        let id = req["params"]["id"];
        let {resource} = {Resource}.find(id);
        if ({resource} == null) {
            return {"status": 404, "body": "{Resource} not found"};
        }
        return render("{resources}/show", {"{resource}": {resource}});
    }
    
    fn new(req: Any) -> Any {
        return render("{resources}/new", {"title": "New {Resource}"}]);
    }
    
    fn create(req: Any) -> Any {
        let data = req["json"];
        let result = {Resource}.create(data);
        if (result["valid"]) {
            return {"status": 302, "headers": {"Location": "/{resources}/" + result["id"]}};
        }
        return {"status": 422, "body": json_stringify({"errors": result["errors"]})};
    }
    
    fn edit(req: Any) -> Any {
        let id = req["params"]["id"];
        let {resource} = {Resource}.find(id);
        if ({resource} == null) {
            return {"status": 404, "body": "{Resource} not found"};
        }
        return render("{resources}/edit", {"{resource}": {resource}}]);
    }
    
    fn update(req: Any) -> Any {
        let id = req["params"]["id"];
        let data = req["json"];
        let result = {Resource}.update(id, data);
        if (result["valid"]) {
            return {"status": 302, "headers": {"Location": "/{resources}/" + id}};
        }
        return {"status": 422, "body": json_stringify({"errors": result["errors"]})};
    }
    
    fn destroy(req: Any) -> Any {
        let id = req["params"]["id"];
        {Resource}.destroy(id);
        return {"status": 302, "headers": {"Location": "/{resources}"}};
    }
}
```

**Controller Naming Conventions:**
- Class name: `PascalCase` ending with `Controller` (e.g., `UsersController`)
- File name: `snake_case` ending with `_controller.sl` (e.g., `users_controller.sl`)
- Method signature: `fn method_name(req: Any) -> Any`

### Middleware Generation Patterns

**Scope-only Middleware Template:**
```soli
// order: N
// scope_only: true

fn {middleware_name}(req: Any) -> Any {
    // Authentication/authorization logic
    
    if (condition_met) {
        return {"continue": true, "request": req};
    }
    
    return {
        "continue": false,
        "response": {
            "status": 401,
            "headers": {"Content-Type": "application/json"},
            "body": json_stringify({"error": "Unauthorized"})
        }
    };
}
```

**Global-only Middleware Template:**
```soli
// order: N
// global_only: true

fn {middleware_name}(req: Any) -> Any {
    // Logic that runs for all requests
    
    return {"continue": true, "request": req};
}
```

### Route Generation Patterns

**Basic Routes:**
```soli
get("/{path}", "{controller}#{action}");
post("/{path}", "{controller}#{action}");
put("/{path}/{id}", "{controller}#{action}");
delete("/{path}/{id}", "{controller}#{action}");
```

**Scoped Middleware Routes:**
```soli
middleware("{middleware_name}", -> {
    get("/{path}", "{controller}#{action}");
    get("/{path}/:id", "{controller}#{action}");
});
```

**RESTful Resource Routes:**
```soli
resources("{resource}", null);  // Generates all CRUD routes
```

**WebSocket Routes:**
```soli
router_websocket("/ws/{path}", "{controller}#{handler}");
```

**LiveView Routes:**
```soli
router_live("/{path}", "{ComponentLive}");
```

### View Generation Patterns

**ERB Template Syntax:**
```erb
<%= expression %>     <!-- Output HTML-escaped result -->
<% code %>            <!-- Execute code without output -->
<%= yield %>          <!-- Layout content insertion -->
```

**Index View Template:**
```erb
<h1><%= @title %></h1>

<% if (@{resources}.length > 0) { %>
<table>
    <thead>
        <tr>
            <th>ID</th>
            <th>Name</th>
            <th>Actions</th>
        </tr>
    </thead>
    <tbody>
        <% for ({resource} in @{resources}) { %>
        <tr>
            <td><%= {resource}["id"] %></td>
            <td><%= h({resource}["name"]) %></td>
            <td>
                <a href="/<%= h({resource}["name"]) %>/<%= {resource}["id"] %>">View</a>
            </td>
        </tr>
        <% } %>
    </tbody>
</table>
<% } else { %>
<p>No {resources} found.</p>
<% } %>

<a href="/<%= h({resources}) %>/new">New {Resource}</a>
```

### Common Generation Tasks

**1. Generate Full Resource Scaffold:**
```
Generate a PostsController with CRUD actions (index, show, new, create, edit, update, destroy)
Add RESTful routes in config/routes.sl
Generate views: index, show, new, edit, _form
```

**2. Generate API Controller:**
```
Generate an ApiController with JSON endpoints
Use render() with data hash for HTML, return dict with status/body for JSON
```

**3. Add Authentication Middleware:**
```
Create authenticate middleware (scope_only: true, order: 20)
Protect routes with middleware("authenticate", -> { ... })
```

**4. Generate CRUD Routes:**
```
Add routes for PostsController: index, show, new, create, edit, update, destroy
Use get/post/put/delete helpers
```

### Response Types Reference

| Response Type | Return Format | Example |
|--------------|---------------|---------|
| HTML View | `render(template, data)` | `return render("posts/index", {"posts": posts});` |
| JSON | `{"status": N, "body": json_stringify(data)}` | `return {"status": 200, "body": json_stringify({"id": 1})};` |
| Redirect | `{"status": 302, "headers": {"Location": url}}` | `return {"status": 302, "headers": {"Location": "/posts"}};` |
| Raw | `{"status": N, "body": string}` | `return {"status": 404, "body": "Not found"};` |

### Request Access Patterns

| Data | Access Pattern |
|------|----------------|
| Path parameters | `req["params"]["param_name"]` |
| Query string | `req["query"]["key"]` |
| JSON body | `req["json"]["field"]` |
| Headers | `req["headers"]["Header-Name"]` |
| Session | `session_get("key")` |
| Cookies | `req["cookies"]["name"]` |

### Built-in Helpers in Views

| Helper | Usage | Description |
|--------|-------|-------------|
| `h()` | `<%= h(text) %>` | HTML escape |
| `datetime_format()` | `<%= datetime_format(date, "%Y-%m-%d") %>` | Format DateTime |
| `render_partial()` | `<%= render_partial("path", data) %>` | Include partial |

### AI Agent Quick Reference

**When generating controllers:**
1. Extend `Controller` base class
2. Use `fn method(req: Any) -> Any` signature
3. Return `render()` for HTML, dict for JSON/redirect
4. Access params via `req["params"]`, body via `req["json"]`

**When generating middleware:**
1. Mark global with `// global_only: true`
2. Mark scoped with `// scope_only: true`
3. Set order with `// order: N`
4. Return `{"continue": bool, "request"?: dict, "response"?: dict}`

**When generating routes:**
1. Use `get/post/put/delete/patch` helpers
2. Format: `HTTP_METHOD('/path', 'controller#action')`
3. Parameters: `/path/:param_name`
4. Scope middleware: `middleware("name", -> { routes })`

### File Locations Reference

| Component | Location |
|-----------|----------|
| Controllers | `app/controllers/{name}_controller.sl` |
| Middleware | `app/middleware/{name}.sl` |
| Views | `app/views/{controller}/{action}.html.erb` |
| Layouts | `app/views/layouts/{name}.html.erb` |
| Partials | `app/views/{controller}/_{name}.html.erb` |
| Routes | `config/routes.sl` |
| Conventions | `.soli/conventions/*.json` |
| Examples | `.soli/examples/*.sl` |

## Example: Simple Web Handler
```soli
// Fetch users and render template
fn get_users() -> Any {
    let users = User.where("doc.active == @active", { "active": true }).limit(10).all();
    return render("users/list", {"users": users});
}

// With pipeline processing
let data = fetch_json("/api/users")
    |> then(fn(r) r.json())
    |> then(fn(users) users.filter(fn(u) u["active"]));
```

## Example: Safe HTML Rendering (XSS Prevention)
```soli
fn render_comment(author: String, content: String) -> String {
    let safe_author = html_escape(author);
    let safe_content = sanitize_html(content);
    return "<div class=\"comment\"><strong>" + safe_author + ":</strong> " + safe_content + "</div>";
}

let comment = render_comment("<script>evil()</script>Alice", "<p>Hello!</p><script>steal()</script>");
print(comment);
// Output: <div class="comment"><strong>&lt;script&gt;evil()&lt;/script&gt;Alice:</strong> <p>Hello!</p></div>
```

## Key Design Decisions

1. **No async/await**: Futures auto-resolve when used, simpler mental model
2. **No exceptions**: Errors return Result-like values
3. **Minimal OOP**: Classes only, no inheritance complexity
4. **Ruby influences**: Hash rockets, blocks, pipelines
5. **Type inference**: `let x = 5` infers `Int`
6. **Single file execution**: Scripts run directly, no complex build
