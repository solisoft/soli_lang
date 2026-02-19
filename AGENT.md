# Soli Language Agent Context

**Soli** is a web-oriented programming language designed with strict minimum features. It focuses on the essential capabilities needed for server-side web development while maintaining simplicity and readability.

## Development Requirements

All code changes must pass linting and formatting checks before submission:

```bash
# Run clippy with deny warnings
cargo clippy -- -D warnings

# Format code
cargo fmt
```

**Always run these commands before committing changes.**

### Testing

Each new language feature must be tested in a `.sl` test file. Test files should be placed in `src/` or a dedicated `tests/` directory and can be run with:

```bash
soli test
```

## Table of Contents

1. [Development Requirements](#development-requirements)
2. [Core Philosophy](#core-philosophy)
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

### Array Methods (Chainable)
- `arr.map(fn(x) ...)` - transform each element
- `arr.filter(fn(x) ...)` - keep elements matching predicate
- `arr.each(fn(x) ...)` - iterate for side effects
- `arr.reduce(fn(acc, x) ..., initial?)` - aggregate to single value
- `arr.find(fn(x) ...)` - return first matching element
- `arr.any?(fn(x) ...)` - check if any element matches
- `arr.all?(fn(x) ...)` - check if all elements match
- `arr.sort(fn(a, b)?)` - sort elements (default or custom comparator)
- `arr.reverse()` - reverse array order
- `arr.uniq()` - remove duplicate elements
- `arr.compact()` - remove null values
- `arr.flatten(depth?)` - flatten nested arrays
- `arr.first()` - get first element
- `arr.last()` - get last element
- `arr.empty?()` - check if array has no elements
- `arr.include?(value)` - check if array contains value
- `arr.sample()` - get random element
- `arr.shuffle()` - randomize array order
- `arr.take(n)` - get first n elements
- `arr.drop(n)` - skip first n elements
- `arr.zip(other)` - combine two arrays into pairs
- `arr.sum()` - sum numeric elements
- `arr.min()` - find minimum value
- `arr.max()` - find maximum value

### Hashes
- `len(hash)` - entry count
- `keys(hash)` - array of keys
- `values(hash)` - array of values
- `entries(hash)` - array of [key, value] pairs
- `has_key(hash, key)` - check existence
- `delete(hash, key)` - remove entry
- `merge(h1, h2)` - combine hashes
- `clear(hash)` - remove all

### Hash Methods (Chainable)
- `hash.map(fn(k, v) ...)` - transform to new hash
- `hash.filter(fn(k, v) ...)` - keep entries matching predicate
- `hash.each(fn(k, v) ...)` - iterate for side effects
- `hash.get(key, default?)` - get value with optional default
- `hash.fetch(key, default?)` - get value, error if missing (optional default)
- `hash.invert()` - swap keys and values
- `hash.transform_values(fn(v) ...)` - transform all values
- `hash.transform_keys(fn(k) ...)` - transform all keys
- `hash.select(fn(k, v) ...)` - keep entries where function returns true
- `hash.reject(fn(k, v) ...)` - remove entries where function returns true
- `hash.slice([key1, key2, ...])` - get subset with specified keys
- `hash.except([key1, key2, ...])` - get hash without specified keys
- `hash.compact()` - remove entries with null values
- `hash.dig(key, key2, ...)` - navigate nested hashes safely

### String Methods
- `str.starts_with?(prefix)` - check if string starts with prefix
- `str.ends_with?(suffix)` - check if string ends with suffix
- `str.chomp()` - remove trailing newline
- `str.lstrip()` - remove leading whitespace
- `str.rstrip()` - remove trailing whitespace
- `str.squeeze(chars?)` - compress consecutive characters
- `str.count(substr)` - count occurrences of substring
- `str.gsub(pattern, replacement)` - global regex substitution
- `str.sub(pattern, replacement)` - single regex substitution
- `str.match(pattern)` - regex match, returns captures array
- `str.scan(pattern)` - find all regex matches
- `str.tr(from, to)` - character translation
- `str.center(width, pad?)` - center with padding
- `str.ljust(width, pad?)` - left justify with padding
- `str.rjust(width, pad?)` - right justify with padding
- `str.ord()` - get ASCII/Unicode codepoint of first char
- `str.bytes()` - get array of byte values
- `str.chars()` - get array of characters
- `str.lines()` - split into array of lines
- `str.bytesize()` - get byte length
- `str.capitalize()` - capitalize first letter, lowercase rest
- `str.swapcase()` - toggle case of all characters
- `str.insert(index, string)` - insert string at position
- `str.delete(substr)` - remove all occurrences
- `str.delete_prefix(prefix)` - remove prefix if present
- `str.delete_suffix(suffix)` - remove suffix if present
- `str.partition(sep)` - split into [before, sep, after]
- `str.rpartition(sep)` - partition from right
- `str.reverse()` - reverse string
- `str.hex()` - parse as hexadecimal number
- `str.oct()` - parse as octal number
- `str.truncate(length, suffix?)` - truncate with ellipsis

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

## Execution

Solilang uses a tree-walking interpreter for executing programs. The interpreter is simple, portable, and provides good performance for most use cases.

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
│   │   ├── views.json
│   │   └── solidb.json        # SoliDB integration patterns
│   └── examples/              # Annotated examples
│       ├── controller.sl
│       ├── middleware.sl
│       └── routes.sl
├── www/                       # MVC framework web application
│   ├── app/
│   │   ├── controllers/       # Request handlers (.sl files)
│   │   ├── middleware/        # HTTP middleware functions
│   │   ├── models/            # Data models with SolidB integration
│   │   ├── views/             # Templates and layouts
│   │   └── helpers/           # View helper functions
│   ├── config/
│   │   └── routes.sl          # Route definitions
│   └── public/                # Static assets
├── ../solidb/                  # SoliDB database (sibling directory)
```

### Convention Files for AI Agents

AI agents should read `.soli/context.json` for framework metadata and `.soli/conventions/*.json` for detailed patterns.

**Key convention files:**
- `.soli/context.json` - Framework metadata, naming conventions, response types
- `.soli/conventions/controller.json` - Controller patterns, method signatures
- `.soli/conventions/middleware.json` - Middleware types, execution order
- `.soli/conventions/routes.json` - Route patterns, REST conventions
- `.soli/conventions/views.json` - Template syntax, variable access
- `.soli/conventions/solidb.json` - SoliDB/SolidB database integration patterns

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
| JSON | `render_json(data, status?)` | `return render_json({"id": 1});` |
| Plain Text | `render_text(text, status?)` | `return render_text("pong");` |
| Redirect | `redirect(url)` | `return redirect("/posts");` |

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
3. Return `render()` for HTML, `render_json()` for JSON, `render_text()` for text, `redirect("/path")` for redirect
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

**When integrating SoliDB/SolidB:**
1. Initialize connection: `this.db = solidb_connect("localhost", 6745, "api_key")`
2. Execute query: `solidb_query(db, database, sdbql, params)`
3. CRUD operations: `solidb_insert/get/update/delete()`
4. Transactions: `solidb_transaction(db, database, fn(tx) { ... })`
5. SDBQL syntax: `FOR doc IN collection FILTER condition RETURN doc`

### File Locations Reference

| Component | Location |
|-----------|----------|
| Controllers | `app/controllers/{name}_controller.sl` |
| Middleware | `app/middleware/{name}.sl` |
| Views | `app/views/{controller}/{action}.html.slv` |
| Layouts | `app/views/layouts/{name}.html.slv` |
| Partials | `app/views/{controller}/_{name}.html.slv` |
| Routes | `config/routes.sl` |
| Conventions | `.soli/conventions/*.json` |
| Examples | `.soli/examples/*.sl` |
| SolidB Database | `../solidb/` |

## SoliDB (SolidB) Database Integration

SoliDB (SolidB) is the recommended database for Soli MVC applications. It's a lightweight, high-performance multi-document database with native Soli language support.

### Overview

| Feature | Description |
|---------|-------------|
| **Name** | SoliDB / SolidB |
| **Version** | 0.5.0 |
| **Repository** | https://github.com/solisoft/solidb |
| **Documentation** | https://solidb.solisoft.net/docs/ |
| **Default Port** | 6745 |
| **Query Language** | SDBQL (ArangoDB-inspired) |

### Key Features

- **JSON Document Storage** - Store and query JSON documents
- **SDBQL Query Language** - Powerful query syntax with FOR/FILTER/SORT/LIMIT/RETURN
- **Multi-node Replication** - Peer-to-peer replication with automatic sync
- **Sharding** - Horizontal data partitioning
- **ACID Transactions** - Atomic operations with configurable isolation
- **Lua Scripting** - Server-side scripts for custom endpoints
- **WebSocket Real-time** - LiveQuery subscriptions
- **Vector Search** - Hybrid search with vector indexes

### Built-in SolidB Functions

| Function | Description |
|----------|-------------|
| `solidb_connect(host, port, api_key)` | Connect to SoliDB server |
| `solidb_query(db, database, query, params)` | Execute SDBQL query |
| `solidb_insert(db, database, collection, document)` | Insert document, returns with _id |
| `solidb_get(db, database, collection, id)` | Get document by ID |
| `solidb_update(db, database, collection, id, data)` | Update document |
| `solidb_delete(db, database, collection, id)` | Delete document |
| `solidb_transaction(db, database, fn(tx) { ... })` | Execute atomic transaction |

### Controller Integration Pattern

```soli
class PostsController extends Controller {
    static {
        this.db = solidb_connect("localhost", 6745, "your-api-key");
        this.database = "myapp";
    }
    
    fn index(req: Any) -> Any {
        let posts = solidb_query(this.db, this.database, 
            "FOR doc IN posts SORT doc.created_at DESC RETURN doc", {});
        return render("posts/index", {"posts": posts});
    }
    
    fn show(req: Any) -> Any {
        let id = req["params"]["id"];
        let post = solidb_get(this.db, this.database, "posts", id);
        if (post == null) {
            return {"status": 404, "body": "Not found"};
        }
        return render("posts/show", {"post": post});
    }
    
    fn create(req: Any) -> Any {
        let data = req["json"];
        let result = solidb_insert(this.db, this.database, "posts", {
            "title": data["title"],
            "content": data["content"],
            "created_at": datetime_now()
        });
        return {"status": 201, "body": json_stringify(result)};
    }
}
```

### SDBQL Query Syntax

**Basic Queries:**
```sdbql
FOR doc IN posts RETURN doc                                    -- All documents
FOR doc IN posts FILTER doc.status == "published" RETURN doc   -- Filtered
FOR doc IN posts SORT doc.created_at DESC LIMIT 10 RETURN doc  -- Sorted & limited
```

**Parameterized Queries:**
```sdbql
FOR doc IN posts
FILTER doc.status == @status AND LIKE(doc.title, @search, true)
SORT doc.created_at DESC
LIMIT 20
RETURN doc
```

**Aggregations:**
```sdbql
FOR doc IN posts
COLLECT status = doc.status WITH COUNT INTO count
RETURN {status, count}
```

**Joins:**
```sdbql
FOR post IN posts
FOR author IN authors
FILTER post.author_id == author._key
RETURN {post, author}
```

### SDBQL Complete Function Reference

**String Functions:**
| Function | Syntax | Description | Example |
|----------|--------|-------------|---------|
| `UPPER` | `UPPER(string)` | Convert to uppercase | `UPPER(doc.name)` |
| `LOWER` | `LOWER(string)` | Convert to lowercase | `LOWER(doc.email)` |
| `TRIM` | `TRIM(string)` | Remove whitespace | `TRIM(doc.content)` |
| `CONCAT` | `CONCAT(a, b, ...)` | Concatenate strings | `CONCAT(doc.first, " ", doc.last)` |
| `SUBSTRING` | `SUBSTRING(str, start, [len])` | Extract substring | `SUBSTRING(doc.body, 0, 100)` |
| `REPLACE` | `REPLACE(str, search, replace)` | Replace substring | `REPLACE(doc.text, "old", "new")` |
| `CONTAINS` | `CONTAINS(haystack, needle)` | Check if contains | `CONTAINS(doc.desc, "keyword")` |
| `STARTS_WITH` | `STARTS_WITH(str, prefix)` | Check prefix | `STARTS_WITH(doc.url, "https://")` |
| `ENDS_WITH` | `ENDS_WITH(str, suffix)` | Check suffix | `ENDS_WITH(doc.file, ".pdf")` |
| `SPLIT` | `SPLIT(str, separator)` | Split into array | `SPLIT(doc.tags, ",")` |
| `LENGTH` | `LENGTH(string)` | String length | `LENGTH(doc.name)` |
| `LEFT` | `LEFT(str, len)` | Left substring | `LEFT(doc.code, 5)` |
| `RIGHT` | `RIGHT(str, len)` | Right substring | `RIGHT(doc.sku, 3)` |
| `REVERSE` | `REVERSE(str)` | Reverse string | `REVERSE(doc.palindrome)` |

**Numeric/Math Functions:**
| Function | Syntax | Description | Example |
|----------|--------|-------------|---------|
| `TO_NUMBER` | `TO_NUMBER(val)` | Convert to number | `TO_NUMBER(doc.price)` |
| `FLOOR` | `FLOOR(num)` | Round down | `FLOOR(doc.average)` |
| `CEIL` | `CEIL(num)` | Round up | `CEIL(doc.score)` |
| `ROUND` | `ROUND(num, [decimals])` | Round number | `ROUND(doc.rating, 2)` |
| `ABS` | `ABS(num)` | Absolute value | `ABS(doc.delta)` |
| `SQRT` | `SQRT(num)` | Square root | `SQRT(doc.value)` |
| `POWER` | `POWER(base, exp)` | Power | `POWER(doc.base, 2)` |
| `MOD` | `MOD(num, divisor)` | Modulo | `MOD(doc.value, 10)` |
| `MIN` | `MIN(a, b, ...)` | Minimum | `MIN(doc.a, doc.b)` |
| `MAX` | `MAX(a, b, ...)` | Maximum | `MAX(doc.values)` |
| `SUM` | `SUM(array)` | Sum of array | `SUM(doc.prices)` |
| `AVG` | `AVG(array)` | Average of array | `AVG(doc.scores)` |

**Array Functions:**
| Function | Syntax | Description | Example |
|----------|--------|-------------|---------|
| `FIRST` | `FIRST(array)` | First element | `FIRST(doc.items)` |
| `LAST` | `LAST(array)` | Last element | `LAST(doc.items)` |
| `LENGTH` | `LENGTH(array)` | Array length | `LENGTH(doc.tags)` |
| `PUSH` | `PUSH(array, val)` | Add element | `PUSH(doc.items, "new")` |
| `POP` | `POP(array)` | Remove last | `POP(doc.queue)` |
| `APPEND` | `APPEND(a, b)` | Concatenate | `APPEND(doc.a, doc.b)` |
| `UNIQUE` | `UNIQUE(array)` | Remove duplicates | `UNIQUE(doc.dups)` |
| `SORTED` | `SORTED(array)` | Sort ascending | `SORTED(doc.nums)` |
| `SORTED_DESC` | `SORTED_DESC(array)` | Sort descending | `SORTED_DESC(doc.nums)` |
| `REVERSE` | `REVERSE(array)` | Reverse array | `REVERSE(doc.list)` |
| `FLATTEN` | `FLATTEN(array, [depth])` | Flatten nested | `FLATTEN(doc.nested)` |
| `SLICE` | `SLICE(arr, start, [len])` | Extract slice | `SLICE(doc.list, 0, 10)` |
| `POSITION` | `POSITION(arr, val)` | Find index | `POSITION(doc.items, "x")` |
| `REMOVE_VALUE` | `REMOVE_VALUE(arr, val)` | Remove value | `REMOVE_VALUE(doc.t, "x")` |
| `REMOVE_NTH` | `REMOVE_NTH(arr, idx)` | Remove at index | `REMOVE_NTH(doc.l, 5)` |

**DateTime Functions:**
| Function | Syntax | Description | Example |
|----------|--------|-------------|---------|
| `DATE_FORMAT` | `DATE_FORMAT(date, fmt)` | Format date | `DATE_FORMAT(doc.d, "%Y-%m-%d")` |
| `DATE_NOW` | `DATE_NOW()` | Current time | `DATE_NOW()` |
| `DATE_ADD` | `DATE_ADD(date, n, unit)` | Add duration | `DATE_ADD(doc.d, 7, "day")` |
| `DATE_SUB` | `DATE_SUB(date, n, unit)` | Subtract duration | `DATE_SUB(doc.d, 1, "month")` |
| `DATE_DIFF` | `DATE_DIFF(a, b, unit)` | Date difference | `DATE_DIFF(doc.e, doc.s, "day")` |
| `IS_SAME_DATE` | `IS_SAME_DATE(a, b)` | Same date? | `IS_SAME_DATE(doc.a, doc.b)` |
| `IS_BEFORE` | `IS_BEFORE(a, b)` | A before B? | `IS_BEFORE(doc.a, doc.b)` |
| `IS_AFTER` | `IS_AFTER(a, b)` | A after B? | `IS_AFTER(doc.a, doc.b)` |

**Type Conversion Functions:**
| Function | Syntax | Description | Example |
|----------|--------|-------------|---------|
| `TO_STRING` | `TO_STRING(val)` | Convert to string | `TO_STRING(doc.n)` |
| `TO_NUMBER` | `TO_NUMBER(val)` | Convert to number | `TO_NUMBER(doc.s)` |
| `TO_BOOL` | `TO_BOOL(val)` | Convert to bool | `TO_BOOL(doc.s)` |
| `TO_ARRAY` | `TO_ARRAY(val)` | Convert to array | `TO_ARRAY(doc.v)` |
| `IS_NULL` | `IS_NULL(val)` | Is null? | `IS_NULL(doc.v)` |
| `IS_BOOL` | `IS_BOOL(val)` | Is boolean? | `IS_BOOL(doc.v)` |
| `IS_NUMBER` | `IS_NUMBER(val)` | Is number? | `IS_NUMBER(doc.v)` |
| `IS_STRING` | `IS_STRING(val)` | Is string? | `IS_STRING(doc.v)` |
| `IS_ARRAY` | `IS_ARRAY(val)` | Is array? | `IS_ARRAY(doc.v)` |
| `IS_OBJECT` | `IS_OBJECT(val)` | Is object? | `IS_OBJECT(doc.v)` |
| `IS_INTEGER` | `IS_INTEGER(val)` | Is integer? | `IS_INTEGER(doc.v)` |
| `IS_DATETIME` | `IS_DATETIME(val)` | Is datetime? | `IS_DATETIME(doc.v)` |

**Aggregate Functions (COLLECT):**
```sdbql
FOR doc IN orders
  COLLECT status = doc.status WITH COUNT INTO count
  RETURN {status, count}

FOR doc IN sales
  COLLECT year = DATE_FORMAT(doc.date, "%Y")
  AGGREGATE total = SUM(doc.amount), avg = AVG(doc.amount)
  RETURN {year, total, avg}
```

**Geo Functions:**
| Function | Syntax | Description |
|----------|--------|-------------|
| `DISTANCE` | `DISTANCE(geo1, geo2)` | Distance between points |
| `GEO_DISTANCE` | `GEO_DISTANCE(a, b)` | Geo distance in meters |
| `GEO_CONTAINS` | `GEO_CONTAINS(area, point)` | Contains check |
| `GEO_INTERSECTS` | `GEO_INTERSECTS(a, b)` | Intersection check |

**Vector Functions:**
| Function | Syntax | Description |
|----------|--------|-------------|
| `VECTOR_COSINE` | `VECTOR_COSINE(a, b)` | Cosine similarity |
| `VECTOR_EUCLIDEAN` | `VECTOR_EUCLIDEAN(a, b)` | Euclidean distance |
| `VECTOR_SIMILARITY` | `VECTOR_SIMILARITY(a, b)` | Similarity score |

**Phonetic Functions:**
| Function | Syntax | Description |
|----------|--------|-------------|
| `SOUNDEX` | `SOUNDEX(str)` | Soundex code |
| `METAPHONE` | `METAPHONE(str)` | Metaphone code |
| `NYSIIS` | `NYSIIS(str)` | NYSIIS code |
| `COLOGNE` | `COLOGNE(str)` | Cologne phonetic |

**JSON Functions:**
| Function | Syntax | Description |
|----------|--------|-------------|
| `JSON_PARSE` | `JSON_PARSE(str)` | Parse JSON string |
| `JSON_STRINGIFY` | `JSON_STRINGIFY(val)` | Stringify to JSON |
| `JSON_VALUE` | `JSON_VALUE(json, path)` | Extract value |
| `JSON_QUERY` | `JSON_QUERY(json, path)` | Extract sub-object |

**Advanced Query Patterns:**
```sdbql
-- UPSERT (insert or update)
UPSERT { _key: @key }
  INSERT { _key: @key, count: 1 }
  UPDATE { count: OLD.count + 1 }
  IN page_views

-- Graph traversal
FOR v, e, p IN 1..3 ANY @start GRAPH "my_graph"
  RETURN {vertex: v, path: p}

-- Subqueries
FOR user IN users
  LET posts = (FOR p IN posts FILTER p.user_id == user._key RETURN p)
  RETURN {user, posts}

-- Window functions
FOR doc IN sales
  SORT doc.date
  LET running_total = SUM(doc.amount) 
    OVER (ORDER BY doc.date ROWS UNBOUNDED PRECEDING)
  RETURN {doc, running_total}

-- Case expressions
RETURN {
  category: CASE
    WHEN doc.price < 10 THEN "budget"
    WHEN doc.price < 100 THEN "mid"
    ELSE "premium"
  END
}

-- Optional chaining
RETURN {
  city: doc.address?.city,
  zip: doc.address?.zipcode
}

-- Nullish coalescing
RETURN {
  name: doc.nickname ?? doc.first_name ?? "Unknown"
}
```

### API Endpoints

**Database & Collections:**
| Operation | Method | Endpoint |
|-----------|--------|----------|
| List databases | GET | `/_api/databases` |
| Create database | POST | `/_api/database` |
| Delete database | DELETE | `/_api/database/{name}` |
| List collections | GET | `/_api/database/{db}/collection` |
| Create collection | POST | `/_api/database/{db}/collection` |
| Delete collection | DELETE | `/_api/database/{db}/collection/{name}` |
| Truncate collection | PUT | `/_api/database/{db}/collection/{name}/truncate` |
| Collection stats | GET | `/_api/database/{db}/collection/{name}/stats` |
| Collection count | GET | `/_api/database/{db}/collection/{name}/count` |

**Documents:**
| Operation | Method | Endpoint |
|-----------|--------|----------|
| Insert document | POST | `/_api/database/{db}/document/{collection}` |
| Batch insert | POST | `/_api/database/{db}/document/{collection}/_batch` |
| Get document | GET | `/_api/database/{db}/document/{collection}/{id}` |
| Update document | PUT | `/_api/database/{db}/document/{collection}/{id}` |
| Delete document | DELETE | `/_api/database/{db}/document/{collection}/{id}` |

**Queries:**
| Operation | Method | Endpoint |
|-----------|--------|----------|
| Execute query | POST | `/_api/database/{db}/query` |
| Explain query | POST | `/_api/database/{db}/explain` |
| Create cursor | POST | `/_api/database/{db}/cursor` |
| Next batch | PUT | `/_api/database/{db}/cursor/{id}` |
| Delete cursor | DELETE | `/_api/database/{db}/cursor/{id}` |
| NL query | POST | `/_api/database/{db}/nl` |
| NL feedback | POST | `/_api/database/{db}/nl/feedback` |

**Indexes:**
| Operation | Method | Endpoint |
|-----------|--------|----------|
| Create index | POST | `/_api/database/{db}/index/{collection}` |
| List indexes | GET | `/_api/database/{db}/index/{collection}` |
| Delete index | DELETE | `/_api/database/{db}/index/{collection}/{name}` |
| Rebuild index | PUT | `/_api/database/{db}/index/{collection}/rebuild` |
| Create geo | POST | `/_api/database/{db}/geo/{collection}` |
| Geo near | POST | `/_api/database/{db}/geo/{collection}/{field}/near` |
| Geo within | POST | `/_api/database/{db}/geo/{collection}/{field}/within` |
| Create vector | POST | `/_api/database/{db}/vector/{collection}` |
| Vector search | POST | `/_api/database/{db}/vector/{collection}/{index}/search` |
| Hybrid search | POST | `/_api/database/{db}/hybrid/{collection}/search` |
| Create TTL | POST | `/_api/database/{db}/ttl/{collection}` |

**Transactions:**
| Operation | Method | Endpoint |
|-----------|--------|----------|
| Begin transaction | POST | `/_api/database/{db}/transaction/begin` |
| Commit transaction | POST | `/_api/database/{db}/transaction/{id}/commit` |
| Rollback transaction | POST | `/_api/database/{db}/transaction/{id}/rollback` |
| Insert in tx | POST | `/_api/database/{db}/transaction/{id}/document/{collection}` |
| Update in tx | PUT | `/_api/database/{db}/transaction/{id}/document/{collection}/{key}` |
| Delete in tx | DELETE | `/_api/database/{db}/transaction/{id}/document/{collection}/{key}` |
| Query in tx | POST | `/_api/database/{db}/transaction/{id}/query` |

**Cluster:**
| Operation | Method | Endpoint |
|-----------|--------|----------|
| Cluster status | GET | `/_api/cluster/status` |
| Cluster info | GET | `/_api/cluster/info` |
| Remove node | POST | `/_api/cluster/remove-node` |
| Rebalance | POST | `/_api/cluster/rebalance` |

**Auth (RBAC):**
| Operation | Method | Endpoint |
|-----------|--------|----------|
| List roles | GET | `/_api/auth/roles` |
| Create role | POST | `/_api/auth/roles` |
| Get role | GET | `/_api/auth/roles/{name}` |
| Update role | PUT | `/_api/auth/roles/{name}` |
| Delete role | DELETE | `/_api/auth/roles/{name}` |
| List users | GET | `/_api/auth/users` |
| Create user | POST | `/_api/auth/users` |
| Assign role | POST | `/_api/auth/users/{username}/roles` |
| Revoke role | DELETE | `/_api/auth/users/{username}/roles/{role}` |
| Get current user | GET | `/_api/auth/me` |
| Change password | PUT | `/_api/auth/password` |
| Create API key | POST | `/_api/auth/api-keys` |
| List API keys | GET | `/_api/auth/api-keys` |

**Queues & Cron:**
| Operation | Method | Endpoint |
|-----------|--------|----------|
| List queues | GET | `/_api/database/{db}/queues` |
| List jobs | GET | `/_api/database/{db}/queues/{name}/jobs` |
| Enqueue job | POST | `/_api/database/{db}/queues/{name}/enqueue` |
| Cancel job | DELETE | `/_api/database/{db}/queues/jobs/{id}` |
| List cron jobs | GET | `/_api/database/{db}/cron` |
| Create cron job | POST | `/_api/database/{db}/cron` |
| Update cron job | PUT | `/_api/database/{db}/cron/{id}` |
| Delete cron job | DELETE | `/_api/database/{db}/cron/{id}` |

**Scripts & Triggers:**
| Operation | Method | Endpoint |
|-----------|--------|----------|
| List scripts | GET | `/_api/database/{db}/scripts` |
| Create script | POST | `/_api/database/{db}/scripts` |
| Execute script | POST | `/_api/database/{db}/scripts/{id}/execute` |
| List triggers | GET | `/_api/database/{db}/triggers` |
| Create trigger | POST | `/_api/database/{db}/triggers` |

**Blobs:**
| Operation | Method | Endpoint |
|-----------|--------|----------|
| Upload blob | POST | `/_api/blob/{db}/{collection}` |
| Download blob | GET | `/_api/blob/{db}/{collection}/{key}` |

**Columnar:**
| Operation | Method | Endpoint |
|-----------|--------|----------|
| Create columnar | POST | `/_api/database/{db}/columnar` |
| List columnars | GET | `/_api/database/{db}/columnar` |
| Insert columnar | POST | `/_api/database/{db}/columnar/{collection}/insert` |
| Aggregate | POST | `/_api/database/{db}/columnar/{collection}/aggregate` |
| Query columnar | POST | `/_api/database/{db}/columnar/{collection}/query` |

**Environment:**
| Operation | Method | Endpoint |
|-----------|--------|----------|
| List env vars | GET | `/_api/database/{db}/env` |
| Set env var | PUT | `/_api/database/{db}/env/{key}` |
| Delete env var | DELETE | `/_api/database/{db}/env/{key}` |

**System:**
| Operation | Method | Endpoint |
|-----------|--------|----------|
| System stats | GET | `/_api/system/stats` |
| Health check | GET | `/_api/system/health` |
| Version info | GET | `/_api/system/version` |

### AI Agent Integration

SolidB includes native AI agent support:

| Endpoint | Description |
|----------|-------------|
| `GET/POST /_api/ai/agents` | Register, list, update agents |
| `GET/POST /_api/ai/tasks` | Task claim/complete/fail operations |
| `GET/POST /_api/ai/contributions` | Agent contribution management |
| `POST /_api/ai/generate` | Content generation with LLM |

### Conventions Files

| File | Description |
|------|-------------|
| `.soli/conventions/solidb.json` | SoliDB integration patterns and API specs |
| `.soli/context.json` | Database configuration and connection info |

### Example: Simple Web Handler
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
