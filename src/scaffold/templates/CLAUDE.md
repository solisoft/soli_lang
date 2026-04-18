# Soli Lang

Soli is a dynamically-typed, high-performance web framework written in Rust.

## Project Structure

```
app/
├── controllers/     # Request handlers (parse request, return response)
├── helpers/         # View helper functions
├── middleware/      # Request/response filters
├── models/         # Data models and business logic
└── views/          # ERB templates
config/
└── routes.sl       # URL routing
db/
└── migrations/     # Database migrations
public/             # Static assets
tests/              # Test files (.sl)
```

## Naming Conventions

| Type | Convention | Example |
|------|------------|---------|
| Files | `snake_case.sl` | `home_controller.sl` |
| Classes | `PascalCase` | `UsersController` |
| Functions | `snake_case` | `get_user_by_id` |
| Constants | `SCREAMING_SNAKE_CASE` | `MAX_SIZE` |

## Routes

Routes are defined in `config/routes.sl`:

```soli
get("/", "home#index");
post("/users", "users#create");
resources("posts");
```

## Controllers

Controllers handle HTTP requests and return responses:

```soli
import "../models/post.sl";

def index(req: Any) do
    let posts = Post.all();
    return render("posts/index", {"posts": posts});
end

def create(req: Any) do
    let params = req["json"];
    let result = Post.create(params);
    if result["valid"] do
        return redirect("/posts/" + str(result["id"]));
    end
    return {"status": 422, "body": json_stringify(result["errors"])};
end
```

### Response Types

- `render("view/name", {data})` - Render an ERB template
- `redirect("/path")` - HTTP redirect
- `{"status": 200, "body": "text"}` - Raw response

## Models

```soli
class Post do
    static def all() do
        return Post.where({}).all;
    end

    static def create(params: Hash) do
        let validation = validate(params);
        if !validation["valid"] do
            return {"valid": false, "errors": validation["errors"]};
        end
        let post = Post.create(params);
        return {"valid": true, "id": post.id};
    end
end
```

## Raw Database Queries (SDBQL)

Use the ORM (`Model.where`, `Model.find`, `Model.create`, ...) first. When you need something the ORM doesn't cover, drop down to raw SDBQL. **Always parameterize** — never concatenate user input into the query string.

### `db.query(sdbql, bind_vars?)`

Pass the SDBQL string with `@name` placeholders and a hash of bind values:

```soli
# Read
let adults = db.query(
    "FOR u IN users FILTER u.age >= @min RETURN u",
    { "min": 18 }
);

# Insert
db.query(
    "INSERT { name: @name, email: @email } INTO users",
    { "name": "Bob", "email": "bob@example.com" }
);

# Update
db.query("UPDATE @key WITH { name: @name } IN users", {
    "key": user_id,
    "name": "Alice Smith"
});

# Delete
db.query("REMOVE @key IN users", { "key": user_id });
```

`@name` is a bind placeholder — the value comes from the hash and is bound safely at query time.

### `@sdql{}` block

Preferred for multi-line queries. Inline expressions with `#{expr}` — they are evaluated and bound as parameters, never inlined as text:

```soli
let min_age = 18;
let city = params.city;

let users = @sdql{
    FOR u IN users
    FILTER u.age >= #{min_age} AND u.city == #{city}
    SORT u.name ASC
    LIMIT 50
    RETURN u
};

@sdql{
    UPDATE #{user_id} IN users
    SET { last_login: NOW() }
};

@sdql{ REMOVE #{user_id} IN users };
```

Supports all SDBQL operations: `FOR`, `FILTER`, `SORT`, `LIMIT`, `RETURN`, `INSERT`, `UPDATE`, `REMOVE`.

### When to use which

| Approach | Use it for |
|----------|------------|
| `Model.where(...)` / ORM | Standard CRUD — your first choice |
| `db.query(sdbql, {...})` | Parameterized query where bind values are already a hash |
| `@sdql{ ... }` | Multi-line query, inline `#{expr}` interpolation, readability |

## Views (ERB Templates)

```erb
<h1><%= title %></h1>

<% for post in posts %>
    <article>
        <h2><%= h(post["title"]) %></h2>
        <%= content %>
    </article>
<% end %>
```

Always use `h()` to escape HTML and prevent XSS.

## Middleware

Middleware filters requests before they reach controllers:

```soli
def authenticate(req: Any) do
    let session = req["cookies"]["session"];
    if !session do
        return redirect("/login");
    end
    return next(req);
end
```

## Testing

```soli
describe("PostsController") do
    before_each do
        # Setup test data
    end
    
    test("index returns all posts") do
        let result = Post.all();
        assert_eq(len(result), 2);
    end
end
```

## Key Syntax

### Variables
```soli
let name = "Alice";           # Type inference
let age: Int = 30;            # Explicit type
const MAX = 100;              # Immutable
```

### Functions
```soli
def add(a: Int, b: Int) do
    return a + b;
end

# Implicit return
def greet(name) do
    "Hello, " + name + "!"
end
```

### Collections
```soli
# Arrays
[1, 2, 3].map() do |x| x * 2 end;

# Hashes
{"name": "Alice"}.name;  # "Alice"
```

### Control Flow
```soli
# Pattern matching
let msg = match value {
    42 => "answer",
    n if n > 0 => "positive",
    _ => "other"
};

# Postfix conditionals
print("adult") if age >= 18;
```

### Pipelines
```soli
[1, 2, 3] |> map() do |x| x * 2 end |> filter() do |x| x > 2 end;
```

## Running the App

```bash
soli serve . --dev     # Development with hot reload
soli serve . --port 5011  # Production
```

## SOLID Principles

Apply these object-oriented design principles for maintainable code:

**Single Responsibility (S)** - Each class does one thing:
```soli
class UserValidator do /* only validation */ end
class UserRepository do /* only database operations */ end
```

**Open/Closed (O)** - Open for extension, closed for modification:
```soli
class Shape { def area() -> Float; }
class Circle extends Shape { radius: Float; def area() do 3.14 * radius * radius; end }
class Rectangle extends Shape { width: Float; height: Float; def area() do width * height; end }
```

**Liskov Substitution (L)** - Subclasses can replace their parent:
```soli
class Bird { def fly() do end }
class Penguin extends Bird { def fly() do throw "Can't fly"; end }  // Violation!
```

**Interface Segregation (I)** - Many small interfaces over one large:
```soli
interface Printable do def print(); end
interface Exportable do def export(); end
class Report implements Printable, Exportable do /* ... */ end
```

**Dependency Inversion (D)** - Depend on abstractions:
```soli
interface UserRepository do def find(id: Int) -> User; end
class InMemoryRepo implements UserRepository do def find(id) do ... end end
class Service do
    repo: UserRepository;
    def get_user(id) do repo.find(id); end
end
```

## Common Patterns

1. **Chain collection operations** instead of loops
2. **Use named parameters** for functions with many optional args
3. **Prefer dot notation** for hash access: `user.name` not `user["name"]`
4. **Use `const`** for values that shouldn't change
5. **Validate early** - return errors immediately when invalid

## Available Commands

```bash
soli serve . --dev        # Start dev server
soli generate controller   # Generate controller
soli generate model        # Generate model
soli generate migration    # Generate migration
soli db:migrate up         # Run migrations
soli db:migrate down       # Rollback migration
soli test tests/           # Run tests
soli lint                  # Lint code for style/smell issues
```

## Linting

Run `soli lint` to check your code for issues:

```bash
soli lint                    # Lint entire project
soli lint app/controllers/   # Lint specific directory
soli lint file.sl           # Lint single file
```

**Naming rules:**
- `naming/snake-case` - variables/functions should use `snake_case`
- `naming/pascal-case` - classes/interfaces should use `PascalCase`

**Style rules:**
- `style/empty-block` - avoid empty blocks
- `style/line-length` - lines should be under 120 characters

**Smell rules:**
- `smell/unreachable-code` - no code after return
- `smell/empty-catch` - catch blocks shouldn't be empty
- `smell/duplicate-methods` - no duplicate method names
- `smell/deep-nesting` - nesting should be ≤4 levels deep
