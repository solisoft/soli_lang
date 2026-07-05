# Soli Lang

Soli is a dynamically-typed, high-performance web framework and programming language written in Rust. It combines Ruby-like expressiveness with excellent performance (170k+ req/sec).

## Project Structure

```
src/                    # Rust interpreter source
├── ast/               # Abstract Syntax Tree
├── lexer/             # Tokenizer
├── parser/            # Parser
├── interpreter/       # Runtime (builtins in interpreter/builtins/)
├── vm/                # Virtual machine
└── template/          # ERB template engine
tests/                 # Soli test files (.sl)
examples/              # Example Soli programs
stdlib/                # Standard library
template/              # MVC project template (used by `soli new`)
www/                   # Documentation website
```

## Naming Conventions

| Type | Convention | Example |
|------|------------|---------|
| Files | `snake_case.sl` | `home_controller.sl` |
| Classes | `PascalCase` | `UsersController` |
| Functions | `snake_case` | `get_user_by_id` |
| Constants | `SCREAMING_SNAKE_CASE` | `MAX_CONNECTIONS` |

## Key Syntax

### Variables and Types
```soli
name = "Alice"                # `let` is optional — bare assignment creates the binding
let age: Int = 30             # Use `let` when you want a type annotation or a
                              # forward declaration assigned conditionally later
const MAX_SIZE = 100          # Constants (immutable)
```

Prefer the bare `name = value` form. Reach for `let` only when it earns its keep — adding a type annotation, or hoisting a variable before an `if`/`match` that assigns it in each branch.

### Functions
```soli
def add(a: Int, b: Int) -> Int {
    return a + b;
}

// Optional parentheses for no-param functions
def greet {
    "Hello!"
}

// Default and named parameters
def configure(host: String = "localhost", port: Int = 8080) -> Void {
    print("Connecting to #{host}:#{port}");
}
configure(port: 3000, host: "example.com");  // Named params in any order
```

### Lambdas and Pipelines
```soli
// fn() syntax
let add = fn(a, b) { return a + b; };

// Pipe syntax
let double = |x| { return x * 2; };

// Pipeline for chaining
let result = 5 |> double() |> addOne();
[1, 2, 3] |> map(fn(x) x * 2) |> filter(fn(x) x > 2);
```

### Collections

**Arrays:**
```soli
let numbers = [1, 2, 3, 4, 5];
let combined = [...a, ...b];

numbers.map(fn(x) x * 2);
numbers.filter(fn(x) x > 2);
numbers.each(fn(x) print(x));
```

**Hashes:**
```soli
let person = {"name": "Alice", "age": 30};
person.name;           // Dot notation preferred
person["name"];        // Bracket notation also works
person.keys();
person.has_key("name");
```

### Classes
```soli
class Person {
    name: String;
    age: Int;
    
    new(name: String, age: Int) {
        this.name = name;
        this.age = age;
    }
    
    def greet() -> String {
        return "Hello, I'm " + this.name;
    }
}

class Employee extends Person {
    salary: Float;
    
    new(name: String, age: Int, salary: Float) {
        super(name, age);
        this.salary = salary;
    }
}
```

### Control Flow
```soli
// Postfix conditionals (idiomatic)
print("adult") if age >= 18;
print("minor") unless age >= 18;

// Pattern matching
let result = match value {
    42 => "the answer",
    n if n > 0 => "positive: " + str(n),
    [first, ...rest] => "first: " + str(first),
    {name: n, age: a} => n + " is " + str(a),
    _ => "wildcard"
};
```

### Error Handling
```soli
try {
    risky_operation();
} catch error {
    print("Error: " + str(error));
} finally {
    cleanup();
}

// Postfix rescue - returns fallback if expr throws
let result = risky_operation() rescue "default";
let data = fetch_data() rescue null;
```

## Best Practices

1. **Use type annotations** - Catch errors early, improve readability
   ```soli
   def process_user(user: Hash) -> Hash { ... }
   ```

2. **Prefer immutability** - Use `const` for values that shouldn't change

3. **Chain iteration methods** - Avoid manual loops when possible
   ```soli
   let result = data
       .filter(fn(x) x["active"])
       .map(fn(x) x["name"])
       .join(", ");
   ```

4. **Use pattern matching** - Cleaner than nested if/elsif for complex conditionals

5. **Use named parameters** - For functions with multiple optional params

6. **Implicit returns** - Last expression in a function block is automatically returned
   ```soli
   def get_greeting(name) {
       "Hello, " + name + "!"  // No return needed
   }
   ```

7. **Use pipelines** - For readable data transformation chains

8. **Use concise defaults and guards** - Prefer idiomatic short forms over verbose nil/empty checks
   ```soli
   # .blank? combines nil and empty-string into one check
   this.email = this.email.trim().downcase() unless this.email.blank?
   this.status = "up" if this.status.blank?
   this.initials = this.initials_from(this.name) if this.initials.blank?

   # ||= for falsey defaults (handles nil and false)
   this.balance ||= 0
   this.is_council ||= false

   # || for inline defaults in expressions
   let name = params["name"] || "Guest"
   let timeout = config["timeout"] || 30
   ```

9. **Use `.includes?` for membership checks** - More readable than chained `||` comparisons
   ```soli
   # Instead of: if s != "a" && s != "b" && s != "c"
   unless ["up", "late", "overdue"].includes?(this.status)
       this._errors = this._errors ?? []
       this._errors.push({"field": "status", "message": "invalid"})
       return false
   end

   # Positive check reads naturally too
   if ["admin", "moderator"].includes?(role)
       grant_access()
   end
   ```

10. **Use intelligible variable names** - No single-letter or cryptic short names. The name should make the intent obvious without having to scan back for the assignment.
   ```soli
   # Bad — what is p? r? pg? qb?
   let p = params
   let r = users_result(p["q"], p["sort"])
   let pg = r["pagination"]
   let qb = User.where(...)

   # Good — read top-to-bottom and the meaning is clear.
   let search_query   = params["q"]
   let sort_column    = params["sort"]
   let result         = users_result(search_query, sort_column)
   let pagination     = result["pagination"]
   let query_builder  = User.where(...)
   ```
   Short names are only acceptable for true conventions: loop indices (`i`, `j`), block parameters whose role is obvious from context (`fn(x) x * 2`), and well-known math symbols inside their natural domain.

## SOLID Principles

Apply these OOP design principles for maintainable code:

**Single Responsibility (S)** - Each class does one thing:
```soli
class UserValidator { /* only validation */ }
class UserRepository { /* only database operations */ }
```

**Open/Closed (O)** - Open for extension, closed for modification:
```soli
class Shape { def area() -> Float; }
class Circle extends Shape { radius: Float; def area() { 3.14 * radius * radius; } }
```

**Liskov Substitution (L)** - Subclasses can replace their parent:
```soli
# Don't override methods with incompatible behavior
```

**Interface Segregation (I)** - Many small interfaces over one large:
```soli
interface Printable { def print(); }
interface Exportable { def export(); }
```

**Dependency Inversion (D)** - Depend on abstractions:
```soli
interface Repository { def find(id: Int) -> User; }
class Service { repo: Repository; def get(id) { repo.find(id); } }
```

## Linting

Run `soli lint` to check code for issues:

```bash
soli lint                    # Lint entire project
soli lint path/to/file.sl   # Lint specific file
```

**Rules:**
- `naming/snake-case` - variables/functions use `snake_case`
- `naming/pascal-case` - classes/interfaces use `PascalCase`
- `style/empty-block` - avoid empty blocks
- `style/line-length` - max 120 chars per line
- `style/redundant-model-import` - no `import "../models/*.sl"` inside `app/controllers/` (models are auto-loaded)
- `smell/unreachable-code` - no code after return
- `smell/empty-catch` - catch blocks shouldn't be empty
- `smell/duplicate-methods` - no duplicate methods
- `smell/deep-nesting` - nesting ≤4 levels
- `smell/undefined-local` - reads of a bare name never assigned in the function scope (catches typos that bypass `let` because `let` is optional)
- `idiom/nil-comparison` - prefer `.nil?` / `.present?` over `== null` / `!= null`
- `idiom/prefer-blank` - prefer `.blank?` / `.present?` over comparing to `""`
- `idiom/prefer-includes` - replace a chain of 3+ same-value `==`/`!=` comparisons with `.includes?`
- `idiom/manual-find-guard` - drop the nil-check after `Model.find` (it raises on a miss; use `find_by`/`first_by` for "or nil")

## MVC Pattern

**Routes** (`config/routes.sl`):
```soli
get("/", "home#index");
post("/users", "users#create");
resources("posts");  // RESTful routes
```

**Controller**:
```soli
// app/controllers/posts_controller.sl
import "../models/post.sl";

def index(req: Any) -> Any {
    let posts = Post.all();
    return render("posts/index", {"posts": posts});
}

def create(req: Any) -> Any {
    let params = req["json"];
    let result = Post.create(params);
    if result["valid"] {
        return redirect("/posts/" + str(result["id"]));
    }
    return {"status": 422, "body": json_stringify(result["errors"])};
}
```

**Model**:
```soli
# app/models/post.sl
class Post < Model
  # `all`, `find`, `where`, `create`, etc. are inherited from Model
  # and use the worker's pre-configured SoliDB connection.
end
```

`Model.find(id)` raises `RecordNotFound` when the id doesn't exist, and the request handler turns that into a `404` automatically. **Don't add a manual `if record.nil? { return 404 }` guard after `.find` — it's dead code.** When you want the "or nil" shape, use `find_by(field, value)` or `first_by(...)`, both of which return `nil` on miss.

**View** (ERB templates):
```erb
<h1><%= title %></h1>

<% for post in posts %>
    <article>
        <h2><%= h(post["title"]) %></h2>
        <%= content %>
    </article>
<% end %>
```

## Testing Pattern
```soli
describe("Feature Name", fn() {
    before_each(fn() {
        // Setup
    });

    test("does something", fn() {
        assert_eq(actual, expected);
        assert(condition);
        assert_null(value);
    });
});
```

## Cookies

Soli exposes parsed cookies from the `Cookie` header as a global `cookies` hash, defaulting to `{}`.

### Cookies Functions
```soli
cookies["theme"];                // Read a cookie (bracket access)
cookies.theme;                   // Read a cookie (dot access)
set_cookie("name", "value");     // Set a response cookie (Set-Cookie header)
```

Use `set_cookie` in controllers and middleware to write response cookies. The cookie is emitted with `Path=/`.

## Session Storage

Soli provides session management with pluggable storage backends.

### Session Functions
```soli
session_set("user_id", 123);     // Store value in session
session_get("user_id");           // Retrieve value (returns null if not found)
session_has("user_id");           // Check if key exists
session_delete("user_id");        // Remove a key from session
session_destroy();                // Destroy entire session
session_regenerate();            // Create new session ID (security after login)
session_id();                    // Get current session ID
session_driver();                // Returns current driver: "in_memory", "cookie", "disk", "solidb", "solikv"
session_config();                // Returns configuration hash
session_configure({"driver": "solidb", "solidb_host": "localhost:8080"});
```

### Storage Backends

| Driver | Description | Configuration |
|--------|-------------|---------------|
| `in_memory` | Default. Fast but lost on restart | None |
| `cookie` | Encrypted client-side sessions (AES-256-GCM, whole payload in the cookie). Survives restarts, multi-host, no infra; ~4KB limit, no server-side revocation | `secret`: 32+ chars (or `SOLI_SESSION_SECRET`) |
| `disk` | File-based JSON storage | `path`: directory (default: `./sessions`) |
| `solidb` | SolidB HTTP database | `solidb_host`, `solidb_database`, `solidb_collection` |
| `solikv` | SoliKV/Redis with TTL | `solikv_host`, `solikv_port`, `solikv_token` |

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `SOLI_SESSION_DRIVER` | Storage backend | `in_memory` |
| `SOLI_SESSION_SECRET` | Secret for the `cookie` driver (32+ chars); rotating it invalidates all sessions | unset |
| `SOLI_SESSION_PATH` | Path for disk storage | `./sessions` |
| `SOLI_SOLIDB_HOST` | SolidB server | `localhost:8080` |
| `SOLI_SOLIDB_DATABASE` | SolidB database | `solidb` |
| `SOLI_SOLIKV_HOST` | SoliKV server | `localhost` |
| `SOLI_SOLIKV_PORT` | SoliKV port | `6380` |
| `SOLI_SESSION_TTL` | Session timeout (seconds) | `86400` |

### Example
```soli
def login(req) {
    let user = authenticate(req["json"]["email"], req["json"]["password"]);
    if user {
        session_regenerate();  // New session ID after login
        session_set("user_id", user["id"]);
        return redirect("/dashboard");
    }
    return {"status": 401, "body": "Invalid credentials"};
}
```

## Dev Tools

When the server runs with `--dev`, `dev_queries()` returns the AQL queries (with bindvars and duration_ms) executed during the current request. Returns `[]` in production with zero runtime overhead. Useful for building a debug bar. See [Models — Inspecting AQL Queries](www/docs/models.md#inspecting-aql-queries-dev-tool).

In `--dev`, every response also carries `X-Soli-Route: <controller#action>`, `X-Soli-Request-Id`, and `X-Soli-Render-Us` (server-side handler time in µs; the requests panel shows this as each row's duration, not the client round-trip) headers. The dev bar reads them (via a `fetch`/`XHR` patch) to list all routes a page touches — the main page plus its XHR/HTMx calls — under the clickable header URL. Clicking a listed request retargets the db/http/kv/flame panels to that request by fetching the dev-only `/__solidev/request/:id` endpoint, which re-renders the snapshot stashed in `serve::dev_store` (a bounded ring buffer). No headers or endpoint in production.

## Documentation Policy

**Every user-facing change MUST be documented in BOTH places:**

1. **`www/docs/*.md`** — the markdown source-of-truth for each topic (e.g. `models.md`, `database.md`, `controllers.md`, `live-reload.md`).
2. **`www/app/views/docs/**/*.html.slv`** — the rendered HTML pages served by `docs#*` controller actions. These are NOT auto-generated from the `.md` files — they are hand-maintained Tailwind/HTML and must be updated in parallel.

When you add, change, or remove a feature visible to Soli users (a new builtin, a new DSL helper, a config option, a CLI flag, behavior change), update both surfaces in the same change. Skipping either leaves the docs site or the markdown reference stale.

3. **`www/app/views/docs/getting-started/comparison.html.slv`** — the "How Soli Compares" page states what Soli has, what it lacks, and how it stacks up against Rails/Laravel/Phoenix/Django. Any code change that adds a capability listed there as missing, removes one listed as present, or shifts a maturity claim MUST update this page in the same change. A stale comparison page is worse than none — its credibility rests on being honest and current.

Use `#` for comments inside Soli code blocks in both `.md` and `.slv` (the `//` style was standardized away — see `www/app/views/docs/CLAUDE.md` recent activity).

## Imports
```soli
import "./math.sl";           // Relative import
import "/lib/utils.sl";       // Absolute import from stdlib
import "erb";                  // Builtin module
```

## String Interpolation
```soli
let name = "Alice";
let greeting = "Hello #{name}!";  // "Hello Alice!"  (#{...} is the only interpolation form; \( is an invalid escape)

// Multi-line / raw strings — NOTE: `@"..."` is NOT a valid form.
// Soli supports three raw / multiline string syntaxes:
let lua_raw = [[
    This is a raw multi-line string.
    Backslashes are literal: \n stays as two chars.
]];
let triple  = """
    Triple-quoted, also raw, multi-line.
""";
let single  = r"C:\Users\name";   // raw, single-line
```

## Important Notes

- **Files are executable top-to-bottom** - No separate `main()` function needed
- **Semicolons optional** - Statements end at line breaks (but `;` is allowed)
- **Truthiness** - Only `false` and `null` are falsy; `0` and `""` are truthy
- **Classes inherit from Object** - Built-in methods available on all objects
- **HTML escaping** - Use `h()` in templates to prevent XSS

## Build and Run

```bash
cargo build --release
./target/release/soli run script.sl
soli test tests/
cargo clippy -- -D warnings
cargo fmt
```

## Local Deploy

After making changes to the Rust interpreter, deploy the new `soli` binary locally so dev projects pick it up:

```bash
cargo install --path . --locked   # rebuild + install the `soli` binary into ~/.cargo/bin
pdev                              # restart the local Soli dev server with the new binary
```

Run both whenever a change in `src/` needs to be exercised through a running Soli app (dev bar, builtins, server behavior, etc.).

`--locked` is required: unlike `cargo build`/`cargo test`, `cargo install` ignores
`Cargo.lock` by default and re-resolves dependencies to their newest versions —
which can pull in a release whose MSRV is above the installed rustc (e.g. `time`
0.3.48 fails with an E0119 coherence error on rustc 1.95).

## Releasing

Use `scripts/release.sh` to create a new release. It bumps `Cargo.toml`, commits, tags, and pushes — CI handles the rest (binaries, GitHub release, Docker image).

```bash
./scripts/release.sh patch     # 0.55.1 -> 0.55.2
./scripts/release.sh minor     # 0.55.1 -> 0.56.0
./scripts/release.sh major     # 0.55.1 -> 1.0.0
./scripts/release.sh patch --dry-run   # preview without changes
```

The CI verifies that the tag version matches `Cargo.toml` before publishing. Never create version tags manually — always use the release script to keep them in sync.

## Key Files

| File | Purpose |
|------|---------|
| `src/interpreter/executor.rs` | Main execution engine |
| `src/interpreter/value.rs` | Value representations |
| `src/lexer/lexer.rs` | Tokenizer |
| `src/parser/parser.rs` | Parser |
| `src/template.rs` | ERB template engine |
| `FEATURE_SPECS.md` | Language feature specs |

## Available Skills

| Skill | Description |
|-------|-------------|
| `task-workflow` | Pick a task from `tasks/todo/`, work on it, and move it through the workflow |
| `ci` | Run the full CI pipeline: clippy, fmt, and tests |
| `doc` | Update documentation about current changes in the www/ folder |
| `release` | Complete development workflow: lint, tests, changelog, commit, release |
| `review` | Review code changes and provide feedback |
