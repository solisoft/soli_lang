# Soli Lang

Soli is a dynamically-typed, high-performance web framework written in Rust. This file orients an AI assistant (and future you) to how *this* application is laid out and what the language syntax actually looks like.

## For AI agents — read this first

You are working in a Soli MVC app. Soli looks like Ruby/JS but has its own quirks; skim the **Footgun cheatsheet** below before generating code. Per-directory `CLAUDE.md` files in `app/controllers/`, `app/models/`, `app/views/`, `app/middleware/`, `tests/`, and `db/migrations/` give you the local rules — Claude Code loads them automatically when you work in those directories.

### Verification loop (mandatory before reporting done)

1. `soli lint <files-you-changed>` — naming, smells, undefined-locals.
2. `soli test tests/<the-relevant-spec>.sl` — narrow, fast feedback.
3. `soli test --coverage --coverage-min 90.0` — full sweep before handing off.
4. `soli serve . --dev`, then hit the route in a browser if you changed a UI/route — confirm 200 and that the page renders.

If a step fails, fix the root cause. Don't weaken assertions, lower the coverage bar, or `--no-verify` past hooks. The `/soli-verify` slash command bundles steps 1-3.

### Reach for generators first

| Task                                     | Command                                       |
|------------------------------------------|-----------------------------------------------|
| New controller (+ views + spec)          | `soli generate controller posts`              |
| New model                                | `soli generate model post`                    |
| New migration                            | `soli generate migration create_posts`        |
| Full RESTful resource end-to-end         | `/soli-resource posts` (slash command)        |

Generators encode the naming, location, and boilerplate the framework expects. Hand-writing diverges and triggers lint failures.

## Footgun cheatsheet (Soli ≠ Ruby ≠ JS)

| You'd type…                                | In Soli it's…                              | Why                                                                          |
|--------------------------------------------|--------------------------------------------|------------------------------------------------------------------------------|
| `// comment`                               | `# comment`                                | `//` was standardized away — lint flags it.                                  |
| `${name}` / `#{name}` in a string          | `\(name)`                                  | Backslash-paren is the only interpolation form.                              |
| `@"multi\nline"` raw string                | `[[multi\nline]]` or `""" ... """`         | `@"..."` doesn't exist; `@` is only for `@sdbql{...}` query blocks.          |
| `if (x) { … }`                             | `if x … end`                               | C-style parses, but Ruby-style is the convention here.                       |
| `xs.forEach(…)`                            | `xs.each do \|x\| … end` or `for x in xs`  | No `forEach`.                                                                |
| `x \|\| default`                           | `x ?? default`                             | `\|\|` returns the wrong side when `x` is `0` or `""` (those are TRUTHY).    |
| `if (xs.length)`                           | `if xs.length() > 0`                       | `0` and `""` are truthy in Soli — only `false` and `null` are falsy.         |
| `import "../models/post.sl"` in controller | nothing — already auto-loaded              | Triggers `style/redundant-model-import` lint.                                |
| Building URLs by hand                      | `posts_path()`, `post_path(post)`          | Named helpers come from `resources(...)` in `config/routes.sl`.              |
| Overriding `Model.all` / `Model.find`      | don't                                      | Inherited from `Model`; the framework relies on it.                          |
| `if x == nil \|\| x == ""`                 | `if x.blank?`                              | `.blank?` covers both nil and empty string in one call.                      |
| `if x == nil` / `if x != nil`              | `if x.nil?` / `unless x.nil?`              | `.nil?` reads as the question; reserve `==`/`!=` for value comparisons.      |
| `user == nil ? nil : user._key`            | `user&._key`                               | Safe navigation short-circuits to `nil` if the receiver is `nil`.            |
| `if s != "a" && s != "b" && s != "c"`      | `unless ["a", "b", "c"].includes?(s)`      | Intent is membership check, not a pile of `&&`.                              |
| `x = x \|\| default`                       | `x \|\|= default`                          | `\|\|=` is a single operator for "set if nil/false".                         |

## Recipes

### Add a RESTful resource end-to-end

1. `soli generate model post` → fill fields, validations, associations.
2. `soli generate migration create_posts` → fill `up`/`down`, then `soli db:migrate up`.
3. `soli generate controller posts` → fill `index`/`show`/`create`/etc.
4. In `config/routes.sl` add `resources("posts")`.
5. Edit `app/views/posts/*.html.slv`.
6. Add specs in `tests/posts_controller_spec.sl`.
7. Run the verification loop.

(Or just: `/soli-resource post` — bundles steps 1-4 and stubs 5-6.)

### Add an authenticated route

Wrap the routes in a `middleware("authenticate", -> { … })` block in `config/routes.sl`. The `authenticate` middleware in `app/middleware/auth.sl` is `scope_only`, so unscoped routes are unaffected.

### Debug a request live

Run `soli serve . --dev`. The dev bar shows the AQL queries (`dev_queries()`) issued for the request, with bind vars and durations.

### Add a partial

- File: `app/views/<dir>/_name.html.slv` (leading underscore is mandatory).
- Render: `<%- partial("dir/name", { "key": value }) %>` — use `<%-` (raw output), not `<%=`, since the partial returns HTML that must not be re-escaped.
- Inside the partial: read via `key` (or `locals["key"]` if it collides with a builtin/helper).

## Project Structure

```
app/
├── controllers/     # Request handlers (one class per resource, < Controller)
├── helpers/         # View helper functions
├── middleware/      # Request/response filters (per-file `# order:` directives)
├── models/          # Data models (< Model — ORM is inherited)
└── views/           # ERB-style templates with .html.slv extension
config/
└── routes.sl        # URL routing
db/
└── migrations/      # Database migrations
public/              # Static assets (CSS/JS compiled into here)
tests/               # *_spec.sl test files
```

## Naming Conventions

| Type      | Convention             | Example                |
|-----------|------------------------|------------------------|
| Files     | `snake_case.sl`        | `posts_controller.sl`  |
| Classes   | `PascalCase`           | `PostsController`      |
| Functions | `snake_case`           | `get_user_by_id`       |
| Constants | `SCREAMING_SNAKE_CASE` | `MAX_SIZE`             |

**Use intelligible variable names** — no single-letter or cryptic short names. The name should make the intent obvious without scanning back for the assignment.

```soli
# Bad — what is p? r? pg? qb?
let p = params
let r = users_result(p["q"], p["sort"])
let pg = r["pagination"]
let qb = User.where(...)

# Good — read top-to-bottom and the meaning is clear.
let search_query  = params["q"]
let sort_column   = params["sort"]
let result        = users_result(search_query, sort_column)
let pagination    = result["pagination"]
let query_builder = User.where(...)
```

Short names are only acceptable for true conventions: loop indices (`i`, `j`), block parameters whose role is obvious from context (`fn(x) x * 2`), and well-known math symbols inside their natural domain.

## Syntax basics

Soli supports both Ruby-style (`def`/`end`, `class X < Y ... end`, `if cond ... end`) and C-style (`fn`/`{ }`, `class X extends Y { ... }`, `if cond { ... }`); they parse to the same AST. **The convention in this project is Ruby-style** for class declarations and control flow (`class Demo < Test ... end`, `if cond ... end`). Reserve `fn { ... }` for free-standing functions and lambdas. Match this style when writing new code.

```soli
# Variables
name = "Alice"            # `let` is optional — bare assignment creates the binding
let age: Int = 30         # Use `let` when you want a type annotation, or to
                          # forward-declare before a branch that assigns it
const MAX = 100           # Immutable

# Prefer the bare `name = value` form. Reach for `let` only when it earns
# its keep: a type annotation, or a hoisted declaration before `if`/`match`.

# Free-standing functions
fn add(a: Int, b: Int) -> Int {
    return a + b;
}

# Implicit return: the last expression in a block is returned
fn greet(name) {
    "Hello, " + name + "!"
}

# Lambdas
let double = fn(x) { return x * 2; };
let halve  = |x| { return x / 2; };

# String interpolation
let msg = "Hi \(name), age \(age)"

# Multiline / raw strings (NO @"..." — that form does not exist)
let lua_raw = [[
    Raw text. No escape processing.
    Good for queries with embedded "double quotes".
]]
let triple = """
    Raw, multiline. Closes on """.
    Good for content with ] or ]] inside.
"""
let single_raw = r"C:\Users\name"   # raw, single-line only

# Collection iteration — Ruby-style block, no parens before `do`
[1, 2, 3].map do |x| x * 2 end
[1, 2, 3].filter do |x| x > 2 end

# Pipelines (when chaining multiple stages)
[1, 2, 3] |> map(fn(x) x * 2) |> filter(fn(x) x > 2)

# Pattern matching
let label = match value {
    42 => "the answer",
    n if n > 0 => "positive",
    [first, ...rest] => "head: " + str(first),
    _ => "other"
};

# Postfix conditionals (idiomatic)
print("adult") if age >= 18
let data = fetch() rescue null     # returns null if fetch() throws

# Concise defaults and guards
this.balance ||= 0                 # ||= sets when nil/false
this.email = this.email.trim().downcase() unless this.email.blank?  # .blank? covers nil + ""
unless ["up", "late", "overdue"].includes?(this.status)              # membership check
    add_error("invalid status")
end
```

## Routes (`config/routes.sl`)

```soli
# Basic routes
get("/", "home#index", name: "root")
get("/about", "pages#about", name: "about")
post("/users", "users#create")

# RESTful resources — registers index/show/new/create/edit/update/destroy
# plus path/url helpers: posts_path(), post_path(post), new_post_path(),
# edit_post_path(post), and *_url variants.
resources("posts")

# Scoped middleware — only runs for routes inside the block
middleware("authenticate", -> {
    get("/admin", "admin#index")
    resources("admin/users")
})
```

Use the named helpers (`posts_path`, `root_path`, etc.) in controllers and views — never concatenate URLs by hand.

## Controllers

Controllers are classes that inherit from `Controller`. Action methods take a request hash and return a response.

```soli
# app/controllers/posts_controller.sl
class PostsController < Controller
    static
        this.layout = "application"
    end

    # GET /posts
    def index(req)
        let posts = Post.all()
        return render("posts/index", { "posts": posts, "title": "Posts" })
    end

    # GET /posts/:id — Model.find raises on miss; framework maps to 404
    def show(req)
        let post = Post.find(req.params["id"])
        return render("posts/show", { "post": post })
    end

    # POST /posts
    def create(req)
        let permitted = this._permit_params(req.params)
        let post = Post.create(permitted)
        if post._errors
            return render("posts/new", { "post": post })
        end
        return redirect(post_path(post))
    end

    # Mass-assignment protection — whitelist allowed fields
    def _permit_params(params)
        return { "title": params["title"], "body": params["body"] }
    end
end
```

### Request access

- `req.params["id"]` — route + query + body params merged
- `req["json"]` — parsed JSON body
- `req["headers"]`, `req["cookies"]`, `req["method"]`
- Bare `params` is also available globally inside actions (= `req.params`)

### Response shapes

- `render("view/name", {...})` — render `app/views/view/name.html.slv` with the given locals
- `redirect("/path")` or `redirect(post_path(post))` — HTTP redirect
- `{"status": 422, "headers": {...}, "body": "..."}` — raw response

## Models

Models inherit from `Model`; CRUD methods come with the inheritance — don't redefine them.

```soli
# app/models/post.sl
class Post < Model
    # Inherited from Model:
    #   Post.all()              Post.find(id)        Post.find_by(field, val)
    #   Post.where({...})       Post.create({...})   post.save()  post.delete()
    #
    # `Post.find(id)` RAISES RecordNotFound on miss — the framework converts
    # that to a 404 automatically. Don't add `if post.nil? { 404 }` after it;
    # that branch is unreachable. Use `find_by` / `first_by` when you want
    # the "or nil" shape instead.
    #
    # Add associations and validations declaratively:
    belongs_to("user")
    has_many("comments")

    validates("title", { "presence": true, "min_length": 3 })
    validates("body",  { "presence": true })

    before_save("normalize_title")

    def normalize_title
        this.title = this.title.trim()
    end
end
```

`Model.create(...)` always returns an instance. On validation/database failure, the instance has `_errors` populated — check `if post._errors` and re-render the form. Don't write fake `static` shims around the inherited CRUD.

### Raw queries (SDBQL)

Drop down to raw SDBQL only when the ORM doesn't cover the case. **Always parameterize** — never concatenate user input.

```soli
# `@sdbql{}` block — preferred for multi-line queries.
# `#{expr}` is bound as a parameter, not interpolated as text.
let min_age = 18
let users = @sdbql{
    FOR u IN users
    FILTER u.age >= #{min_age}
    SORT u.name ASC
    LIMIT 50
    RETURN u
}
```

## Views (`.html.slv`)

```erb
<h1><%= title %></h1>

<% for post in posts %>
    <article>
        <h2><%= h(post.title) %></h2>
        <%= post.body %>
    </article>
<% end %>

<%= link_to("New post", new_post_path()) %>
```

Always use `h()` to escape user-supplied content — XSS is the default risk.

## Middleware

A middleware file declares one function. Per-file directive comments at the top configure how the framework wires it up:

```soli
# app/middleware/auth.sl

# order: 20
# scope_only: true   — only runs when wrapped in `middleware("authenticate", -> { ... })`

def authenticate(req)
    let key = req["headers"]["X-Api-Key"] ?? ""
    if key == ""
        return {
            "continue": false,
            "response": { "status": 401, "body": "Unauthorized" }
        }
    end
    return { "continue": true, "request": req }
end
```

| Directive            | Meaning                                                |
|----------------------|--------------------------------------------------------|
| `# order: N`         | Lower runs first. Default 100.                         |
| `# global_only: true` | Always runs; cannot be scoped.                        |
| `# scope_only: true`  | Only runs when explicitly scoped via `middleware(...)`. |

Returning `{"continue": false, "response": {...}}` short-circuits with that response. Returning `{"continue": true, "request": req}` proceeds to the next middleware / handler.

## Testing

Specs live in `tests/` and run with `soli test`. Use the BDD DSL with `describe` / `test` / `before_each`. Controller tests get an E2E client (`get`, `post`, `put`, `delete`, `assigns()`, `view_path()`, `as_guest()`).

```soli
# tests/posts_controller_spec.sl
describe("PostsController", fn() {
    before_each(fn() {
        as_guest();
    });

    describe("GET /posts", fn() {
        test("returns list of posts", fn() {
            let response = get("/posts");
            assert_eq(res_status(response), 200);
            assert_hash_has_key(assigns(), "posts");
        });
    });

    describe("POST /posts", fn() {
        test("creates with valid data", fn() {
            let response = post("/posts", { "title": "Hello", "body": "World" });
            assert_eq(res_status(response), 302);
        });

        test("rejects invalid data", fn() {
            let response = post("/posts", {});
            assert_eq(res_status(response), 422);
        });
    });
});
```

### Test coverage requirement

**Every new feature must ship with tests achieving >90% coverage of the changed code.** Run coverage locally before opening a PR:

```bash
soli test --coverage                      # generate report
soli test --coverage --coverage-min 90.0  # fail if under 90%
```

This applies to controllers, models, middleware, helpers, and any new library code. Don't merge a feature whose coverage report is missing or below the threshold — write the tests first if it helps you design the API.

## SOLID Principles

Apply these for maintainable code.

```soli
# Single Responsibility — one reason to change per class
class UserValidator
    def validate(user) end
end

class UserRepository
    def save(user) end
end

# Open/Closed — extend via subclasses, don't edit the base
class Shape
    def area -> Float
        0.0
    end
end

class Circle < Shape
    radius: Float

    def area -> Float
        3.14159 * this.radius * this.radius
    end
end

# Liskov — subclasses must honor the parent's contract.
#   Don't override a method to throw where the parent returns.

# Interface Segregation — many small interfaces beat one fat one
interface Printable
    def print()
end

interface Exportable
    def export()
end

# Dependency Inversion — depend on abstractions
interface UserRepository
    def find(id: Int) -> User
end

class UserService
    repo: UserRepository

    def get(id)
        this.repo.find(id)
    end
end
```

## Linting

```bash
soli lint                       # lint entire project
soli lint app/controllers/      # lint a directory
soli lint path/to/file.sl       # lint a single file
```

Key rules:

- `naming/snake-case`, `naming/pascal-case`
- `style/empty-block`, `style/line-length` (≤120 chars)
- `style/redundant-model-import` — don't `import "../models/*.sl"` inside `app/controllers/`; models are auto-loaded
- `smell/unreachable-code`, `smell/empty-catch`, `smell/duplicate-methods`, `smell/dangerous-server-builtin` (flags `db_query_raw` / `Trusted.*` / `System.shell` / backticks in `app/controllers/`, `app/middleware/`, `app/views/`)
- `smell/deep-nesting` (≤4 levels)
- `smell/undefined-local` — reads of a name never assigned in scope (catches typos)
- `idiom/nil-comparison`, `idiom/prefer-blank` — prefer `.nil?`/`.present?`/`.blank?` over `== null` / `== ""`
- `idiom/prefer-includes` — replace 3+ same-value `==`/`!=` comparisons with `.includes?`
- `idiom/manual-find-guard` — drop the nil-check after `Model.find` (it raises; use `find_by`/`first_by` for "or nil")

## Common commands

```bash
soli serve . --dev                    # dev server, hot reload, dev bar enabled
soli serve . --port 5011              # run without --dev (still single-process)

soli generate controller posts        # scaffold controller + spec + views
soli generate model post              # scaffold model
soli generate migration create_posts  # scaffold migration

soli db:migrate up                    # run pending migrations
soli db:migrate down                  # roll back last migration
soli db:migrate status                # show migration state

soli test                             # run all tests in tests/
soli test --coverage --coverage-min 90.0
soli lint                             # static analysis
```

## Conventions to follow

1. **Prefer Ruby-style** for classes and control flow — `class Demo < Test ... end`, `def name(args) ... end`, `if cond ... end`. Reserve `fn { }` for free-standing functions and lambdas.
2. **Use type annotations** on public function signatures — they catch errors and document intent.
3. **Prefer immutability** — `const` for values that never change.
4. **Chain collection methods** instead of writing manual loops.
5. **Use named parameters** when a function has multiple optional args.
6. **Use named route helpers** (`posts_path`, `root_path`) — never hand-built URL strings.
7. **Validate at the model**, not in the controller — keep controllers thin.
8. **Return errors early** — don't pile `if`s; bail with a 422/redirect at the first invalid branch.
9. **Use `.blank?` for nil/empty checks** — replaces `x == nil || x == ""`.
10. **Use `.nil?` over `== nil`** — `if x.nil?` / `unless x.nil?` reads as a question; keep `==`/`!=` for value comparisons.
11. **Use `&.` to short-circuit on nil** — `user&._key` replaces `user == nil ? nil : user._key`; chain it (`user&.address&.city`) instead of nested guards.
12. **Use `||=` for falsey defaults** — `this.balance ||= 0` instead of `if this.balance == nil`.
13. **Use `.includes?` for membership checks** — replaces chained `||` comparisons.
14. **Test new features to >90% coverage** — non-negotiable, see above.
15. **Put a blank line after a `return`** — unless the next line is another `return` or an `end`. This makes guard clauses (early returns) stand out from the code that follows.

    ```soli
    def update(req)
        let post = Post.find(req.params["id"])
        return forbidden() unless can_edit?(post)   # guard clause

        post.update(this._permit_params(req.params))
        return redirect(post_path(post))
    end

    # Back-to-back returns and a return right before `end` need no blank line:
    def status_label(code)
        return "ok"    if code == 200
        return "moved" if code == 301
        return "error"
    end
    ```
