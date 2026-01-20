# Soli Language Agent Context

**Soli** is a web-oriented programming language designed with strict minimum features. It focuses on the essential capabilities needed for server-side web development while maintaining simplicity and readability.

## Core Philosophy

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
import "./utils/math.soli";
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
│   └── routes.soli      # URL routing rules
├── controllers/         # Request handlers
│   └── users_controller.soli
├── models/              # Data models
│   └── user.soli
├── views/               # Templates
│   ├── users/
│   │   ├── index.html.soli
│   │   └── show.html.soli
│   └── layouts/
│       └── application.html.soli
├── public/              # Static assets
└── main.soli            # Entry point
```

### Routes Example
```soli
get("/", fn() {
    return render("home/index", {});
});

get("/users", fn() {
    let users = db.query("SELECT * FROM users");
    return render("users/index", {"users": users});
});

post("/users", fn() {
    let name = request.body["name"];
    db.execute("INSERT INTO users (name) VALUES (?)", [name]);
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
| Import | `import "./file.soli"` |

## Execution Modes

- **Tree-walking interpreter**: Default, simple, portable
- **Bytecode VM**: Faster execution for larger codebases

## File Extension

- `.soli` - Soli source files

## Example: Simple Web Handler
```soli
// Fetch users and render template
fn get_users() -> Any {
    let users = db.query("SELECT * FROM users LIMIT 10");
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
