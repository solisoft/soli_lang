# Soli Lang

Soli is a dynamically-typed, high-performance web framework written in Rust. This file orients an AI assistant (and future you) to how *this* application is laid out and what the language syntax actually looks like.

## For AI agents — read this first

You are working in a Soli MVC app. Soli looks like Ruby/JS but has its own quirks; skim the **Footgun cheatsheet** below before generating code. Per-directory `CLAUDE.md` files in `app/controllers/`, `app/models/`,
`app/models/concerns/`, `app/views/`, `app/middleware/`, `tests/`, and
`db/migrations/` give you the local rules — Claude Code loads them
automatically when you work in those directories. Shared model mixins
(`module` / `include`) live in `app/models/concerns/`.

### Verification loop (mandatory before reporting done)

1. `soli fmt <files-you-changed>` — canonical layout, before you read your own diff.
2. `soli lint <files-you-changed>` — naming, smells, undefined-locals.
3. `soli test tests/<the-relevant-spec>.sl` — narrow, fast feedback.
4. `soli test --coverage --coverage-min 90.0` — full sweep before handing off.
5. `soli serve . --dev`, then hit the route in a browser if you changed a UI/route — confirm 200 and that the page renders.

Run `fmt` before `lint`: the formatter fixes indentation, spacing and line
breaks on its own, so several `style/*` rules stop firing once it has run —
lint's remaining output is then the part that needs your judgement.

If a step fails, fix the root cause. Don't weaken assertions, lower the coverage bar, or `--no-verify` past hooks. The `/soli-verify` slash command bundles steps 1-4.

### Generators: what actually exists

There are **no** standalone `controller` / `model` / `migration` generators.
Run `soli generate` with no argument to see the real list; as of 1.25 it is
`scaffold`, `auth`, `oidc_provider`, `mailer`, `component`, `devices`,
`client`, `app_links`, `offline`. Reaching for the three missing ones is the
most common way an agent wastes its first five minutes in a new app.

| Task                          | Command                                  |
|-------------------------------|------------------------------------------|
| Full resource (model+ctrl+views+migration+routes) | `soli generate scaffold post title:string` |
| New migration only            | `soli db:migrate generate create_posts`  |
| New seed                      | `soli db:seed generate demo_posts`       |
| New view component            | `soli generate component post_card`      |
| Model / controller / spec alone | write by hand, next to the existing ones |

`soli generate scaffold` writes a full resource and a controller E2E spec at
`tests/controllers/*_controller_spec.sl`. There is no model test file — add
one under `tests/` yourself if you want model-level coverage.

Two more things about generators, both learned the hard way:

- `soli generate` writes into the **nearest project, not the cwd**. Run from a
  sandbox inside another app's tree, `generate auth` dropped a whole User stack
  — model, policies, controllers, public `/login` + `/signup` routes, a `users`
  migration — into that app. Never probe generators from inside another tree.
- Two migrations generated in the same second **share a timestamp**. The runner
  treats them as one, plays one, and reports the other as applied. Rename one.

## Footgun cheatsheet (Soli ≠ Ruby ≠ JS)

| You'd type…                                | In Soli it's…                              | Why                                                                          |
|--------------------------------------------|--------------------------------------------|------------------------------------------------------------------------------|
| `// comment`                               | `# comment`                                | `//` was standardized away — lint flags it.                                  |
| `${name}` / `\(name)` in a string          | `#{name}`                                  | Hash-brace is the only interpolation form; `\(` is an invalid escape.        |
| `@"multi\nline"` raw string                | `[[multi\nline]]` or `""" ... """`         | `@"..."` doesn't exist; `@` is only for `@sdbql{...}` query blocks.          |
| `if (x) { … }`                             | `if x … end`                               | C-style parses, but Ruby-style is the convention here.                       |
| `xs.forEach(…)`                            | `xs.each do \|x\| … end` or `for x in xs`  | No `forEach`.                                                                |
| `x \|\| default`                           | `x ?? default`                             | `\|\|` returns the wrong side when `x` is `0` or `""` (those are TRUTHY).    |
| `if (xs.length)`                           | `if xs.length() > 0`                       | `0` and `""` are truthy in Soli — only `false` and `null` are falsy.         |
| `import "../models/post.sl"` in controller | nothing — already auto-loaded              | Triggers `style/redundant-model-import` lint.                                |
| Building URLs by hand                      | `posts_path()`, `post_path(post)`          | Named helpers come from `resources(...)` in `config/routes.sl`.              |
| Overriding `Model.all` / `Model.find`      | don't                                      | Inherited from `Model`; the framework relies on it.                          |
| `if x == nil \|\| x == ""`                 | `if x.blank?`                              | `.blank?` covers both nil and empty string in one call.                      |
| `x ?? ""` / `str(x ?? "")`                 | `x.to_s`                                   | `.to_s` already maps `nil` → `""`, so the `?? ""` fallback is redundant.      |
| `if x == nil` / `if x != nil`              | `if x.nil?` / `unless x.nil?`              | `.nil?` reads as the question; reserve `==`/`!=` for value comparisons.      |
| `user == nil ? nil : user._key`            | `user&._key`                               | Safe navigation short-circuits to `nil` if the receiver is `nil`.            |
| `if s != "a" && s != "b" && s != "c"`      | `unless ["a", "b", "c"].includes?(s)`      | Intent is membership check, not a pile of `&&`.                              |
| `x = x \|\| default`                       | `x \|\|= default`                          | `\|\|=` is a single operator for "set if nil/false".                         |
| `for key, value in a_hash`                 | `for key in a_hash.keys()`                 | There is no two-variable hash iteration; the pair form silently fails.       |
| `redirect(req["headers"]["referer"])`      | extract the path, then `redirect(path)`    | `redirect()` takes **local absolute paths only** and raises on a full URL.   |
| several unrelated reads, one per line      | `grouped(fn() { … })` around them          | Each read is a round-trip; `grouped` combines them into one query. Don't read a result inside the block (auto-flush). |
| `Model.all.slice(0, n)`                    | `Model.limit(n).all`                       | `slice` loads the whole collection into memory just to keep `n` rows.        |

## Framework behaviours found the hard way

Each of these cost a debugging session somewhere. None of them is visible from
the code you are writing — they are properties of the runtime.

| Behaviour | Consequence | What to do |
|---|---|---|
| A `before_*` callback returning `false` **aborts persistence** | `this.flag \|\|= false` as the last line silently rejects every record | End every callback with an explicit `return true` |
| `validates(..., { "custom": "method" })` **never fires** | A business rule declared that way is dead code that reads as protection | Put the rule in a method the controller calls, and render the 422 yourself |
| `update(attrs)` / `save(attrs)` **skip callbacks** | Normalisation is lost on every update | Assign fields explicitly, then `save()` |
| `_errors` is `nil` after a successful `create` but `[]` after a successful `save` | `if record._errors` is true after an update (`[]` is truthy) | Test `_errors.length() > 0` |
| `Model.delete_all` **without a scope** does not empty the collection | A spec that relies on it tests the wrong state and still passes | Loop and `delete()`, or scope it: `Model.where(...).delete_all` |
| Free functions of the **same name in two files shadow each other** | Four copies of one helper, only one ever runs, chosen by load order | Put shared helpers on a base controller or a model, not in free functions |
| Model classes are **not resolvable from `.slv` templates** | `Cart.total_of(...)` in a view raises and the page 500s | Compute in the controller, hand the view a plain value |
| `setenv` was removed (SEC-033) | A spec cannot flip a mode read from the environment | Split the env read from the work — a named method the spec can call directly |
| `.pluck(field)` returns a QueryBuilder, not an array | It lands in a bind variable and the query fails | `.all.map(fn(row) row.field)` |
| An invalid SDBQL query **returns an error string instead of raising** | A typo silently yields garbage | Prefer proven forms: `CONTAINS(LOWER(doc.x), @needle)` |
| No scoped uniqueness | `"uniqueness": true` is global and best-effort | Composite unique index in the migration + a lookup for a readable message |
| `permit` drops containers declared with `true` | Nested hashes and arrays vanish from the body | Describe the shape fully: `{"days": [{"hour": true}]}` |
| Multiple checkboxes need `name="field[]"` | Without brackets only **one** value arrives — a multi-select never persists | Always bracket a repeated field name |
| `<%= attr(url) %>` **escapes twice** — `<%=` already applies `h()` | `&` becomes `&amp;amp;` and the URL breaks | `<%= url %>` for framework-built URLs; keep `attr()` for `<%- %>` |
| A `<form>` directly inside a `<tr>` is invalid HTML | The browser hoists it out; the form **never submits** — yet specs pass, because they POST to URLs | Lay editable lists out as rows of cards |
| `find_uploaded_file` wants `req`, not `params` | Returns nil with `params`, despite the bundled docs' example | `find_uploaded_file(req, "field")` |
| `String.index_of` takes **no start offset** | `index_of("/", 1)` raises `Wrong number of arguments` | Slice first, then search |
| `before_action` hooks are wired by a **startup scan** | `--dev` reloads the action but keeps the old guard — a changed auth rule silently doesn't apply | Restart the process, not just the file |
| `HTTP.post_json` / `patch_json` **silently drop** `options.headers` | The request goes out unauthenticated; the cause is invisible | `HTTP.request(method, url, headers, body)` — headers are the 3rd positional arg |
| `HTTP.*_json` **raise** on a non-2xx status | `try/catch` gets the whole error page as a string; the status branch is dead | `HTTP.request` returns the response — read `response["status"]` |
| A `.md` view runs through the **template engine first** | Writing a template tag in prose *executes* it; `<%%` does not escape it | Name the tags instead of quoting them |

## Recipes

### Add a RESTful resource end-to-end

**Fast path:** `soli generate scaffold post title:string body:text` — model,
controller, views, migration, routes, and
`tests/controllers/post_controller_spec.sl`. Then `soli db:migrate up`.
Add a model unit spec by hand if you need one. Run the verification loop.

**Step-by-step** (when you want each piece by hand):

1. Write `app/models/post.sl` — fields, validations, associations.
2. `soli db:migrate generate create_posts` → fill `up`/`down`, then
   `soli db:migrate up`.
3. Write `app/controllers/posts_controller.sl` — `index`/`show`/`create`/etc.
4. In `config/routes.sl` add `resources("posts")` (and CORS if the API is
   browser-facing:

   ```soli
   # Built-in CORS: preflights, allow headers, origin-checked CSRF opt-in.
   cors("/api/*", {"origins": ["https://app.example.com"], "credentials": true})
   ```

5. Edit `app/views/posts/*.html.slv`.
6. Add specs in `tests/posts_controller_spec.sl`.
7. Run the verification loop.

(Or: `/soli-resource post` — prefers scaffold, then stubs remaining specs.)

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
let msg = "Hi #{name}, age #{age}"

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

# `&:name` — the shorthand that replaces an accumulator loop. It reads a hash
# key, reads a model field, OR calls a method; `.sum` terminates and returns 0
# on an empty array, so no guard is needed.
[{ "total": 2 }, { "total": 3 }].map(&:total).sum   # 5   — hash key
lines.map(&:qty).sum                                #     — model field
lines.map(&:total).sum                              #     — method, it is called

# So this is not written any more:
#   sum = 0
#   for line in lines
#     sum = sum + line.total()
#   end

# Hashes have no two-variable iteration — go through the keys.
for key in totals.keys()
    print(totals[key])
end

# Array concatenation: `+` and `.concat` both work.
first + rest

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

For the **complete SolidB request & method surface in one place** — ORM,
QueryBuilder, associations, the raw `Solidb` client, SDBQL, transactions,
search, analytics, and migrations — see `docs/solidb-reference.md`.

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

### Forms

Use the built-in form builder — it derives the URL and verb from the record
(new → `POST /posts`, persisted → `PATCH /posts/<key>` via a hidden `_method`
field the server honors) and embeds the CSRF token. Builder calls return
HTML, so output them with `<%-` (raw), never `<%=`:

```erb
<%- form_with(post) do |f| -%>
  <%- f.error_summary() %>
  <%- f.label("title") %>
  <%- f.text_field("title", {"placeholder": "Title"}) %>
  <%- f.errors_for("title") %>
  <%- f.submit("Save") %>
<%- end -%>
```

The `do |f|` block binds the builder and wraps the body in `<form>` +
`_method` + CSRF token (a bare `do` gives an implicit `f`); `-%>` swallows
the newline after a tag.

- Field helpers: `text_field`, `email_field`, `password_field`, `number_field`,
  `date_field`, `datetime_field`, `hidden_field`, `file_field` (+
  `"multipart": true` on the form), `text_area`, `check_box`, `radio_button`,
  `select`, `label`, `submit`. Options become HTML attributes; values prefill
  from the record and are escaped; errored fields get a `field-error` class.
- Top-level names are flat (`name="title"` → `params["title"]`); bracket
  names **nest** (`author[name]` → `params["author"]["name"]`, `tags[]` →
  array). `f.fields_for("author") do |author| ... end` renders them and
  prefills from the nested document. An unchecked `check_box` submits
  nothing; read it as `params["published"] == "true"`.
- Always filter mass-assignment through `permit(params, {"title": true,
  "author": {"name": true}, "tags": []})` — SoliDB is schemaless, so an
  unfiltered `Model.create(params)` persists anything a client posts.
- Delete/logout links: `button_to("Delete", "/posts/" + post["_key"].to_s,
  {"method": "delete", "confirm": "Are you sure?"})` — never a bare `<a>`.
- Hand-written `<form method="POST">`? Add `<%- csrf_field() %>` inside it.
- Partials render in a fresh scope — pass the builder along:
  `<%- partial("posts/form", { "post": post, "f": f }) %>`.

## Middleware

A middleware file declares one function. Per-file directive comments at the top configure how the framework wires it up:

```soli
# app/middleware/auth.sl

# order: 20
# scope_only: true   — only runs when wrapped in `middleware("authenticate", -> { ... })`

def authenticate(req)
    let key = req["headers"]["X-Api-Key"].to_s
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

## SOLID, as it actually applies here

The textbook version (interfaces, injected repositories) does not survive
contact with a dynamically-typed MVC framework. What follows is the version
that holds, plus the three traps this framework sets for it.

### Where a rule belongs

**The model owns rules; the controller owns HTTP.** A controller reads params,
asks a model, and returns a response. If you can describe a method without
mentioning a request, it belongs on a model or a service.

Signals that a controller has taken on a second job — check them, they are
cheap:

```bash
# How many models does one controller touch? Past ~6, it is doing two things.
grep -oE "\b[A-Z][A-Za-z]+\b" app/controllers/x_controller.sl | sort -u

# Actions over ~25 lines are usually an action plus a helper that never left.
```

**When two interfaces need the same rule with different wording, the model
returns the fact and each interface writes the sentence.** A back-office in
French and a JSON API in English do not share a message; they share a rule.

```soli
# Model — the rule, and nothing about how it will be said
static def conflicting(opening, point_of_sale_id)   # -> the clashing record, or null

# Back-office                            # API
return "cette plage en heurte une autre" # return "conflicts with an existing opening"
  unless Opening.conflicting(o, p).nil?  #   unless Opening.conflicting(o, p).nil?
```

Returning a *message* from the model is the version that rots: the second
caller copies the method instead of calling it, and a later fix reaches one
of them.

### Trap 1 — free functions share ONE global namespace

A top-level `def` is not file-scoped. Two files declaring `_first_error` do not
get one each: **one silently wins**, chosen by load order, and the other file's
copy is dead code that reads like protection. Worse, a file can call a free
function defined in a *different* file and work — until that file moves.

```bash
# Must print nothing. If it prints a name, one of the copies is already dead.
grep -rn "^def [a-z_]" app/ --include=*.sl \
  | sed 's/:def /|/' | awk -F'|' '{split($2,a,"("); print a[1]}' | sort | uniq -d
```

So **shared logic lives on a class** — a base controller, a model, or a
service — never in a free function. Keep free functions for what is genuinely
private to one file, and give them names no one else would pick.

### Trap 2 — three loading scopes that do not overlap

| Defined in        | Visible from                       | NOT visible from |
|-------------------|------------------------------------|------------------|
| `app/services/`   | models, controllers, other services | **views**       |
| `app/helpers/`    | views                              | **models, controllers** |
| `app/models/`     | models, controllers, services       | **views** (a class name in a `.slv` raises) |

This decides *where* shared code can live, and sometimes makes one
implementation impossible: logic needed by both a model and a view has to
exist twice. That is a constraint, not a failure — write it down in both
copies, pointing at each other, so the next person fixes both.

Corollary: a template must not compute. Hand the view a plain value from the
controller; `Model.helper(...)` inside a `.slv` raises and 500s the page.

### Trap 3 — no DI, so isolate what reads the environment

There is no container to inject a double into, and `setenv` was removed
(SEC-033), so a spec cannot flip a mode read from `getenv`. **Split the
decision from the work**: the branch that reads the environment stays a
one-liner, and everything below it becomes a named method a spec can call.

```soli
static def open_session(cart, email, phone)
  return PaymentGateway.stub_session(cart) unless PaymentGateway.remote?()

  return PaymentGateway.remote_session(cart, email, phone)   # <- testable directly
end
```

Same shape for anything reading an HTTP response: keep the call in one
function, the interpretation in another. `_call_remote` does the request;
`_remote_answer(response, cart)` decides — and that one is provable without a
network.

### Two responsibilities that share machinery → a base with no actions

When a second, distinct job needs the same setup (same layout, same lookup,
same guard), do not grow the first controller. Extract a base that holds only
the shared parts and carries **no actions**, then have both extend it.

```
XBaseController      the layout, the lookup, the guard — no actions
├── XController      the first job
└── XOtherController the second
```

The test is not file length, it is whether the two jobs change for different
reasons — one writing to the session, the other only reading, for instance.

### The two that still read like the textbook

- **Liskov** — a subclass must honour the parent's contract. Do not override an
  inherited `Model` method to raise where the parent returns; the framework
  calls it.
- **Interface segregation** — a base controller that every screen inherits
  should hold what every screen needs. When only two of nine screens use a
  helper, it belongs on those two, not on the base.

## Formatting

`soli fmt` is the canonical layout — run it on every file you touch, before
lint and before you review your own diff. There are no options to argue with.

```bash
soli fmt                        # format the whole project in place
soli fmt app/controllers/       # format a directory
soli fmt path/to/file.sl        # format a single file
soli fmt --check                # exit 1 and list files that would change (CI)
```

What it decides for you: 2-space indent, Ruby-style `class X < Y … end` /
`if cond … end`, `//` comments rewritten to `#`, operator spacing, `#{…}`
interpolation, lines kept under 120 chars, a single-statement `if cond return …
end` collapsed to postfix `return … if cond`, and a blank line after an early
`return` (see convention 15) unless the next line is another `return` or an
`end`. Already-formatted files are left untouched, and the output is a fixed
point — running it twice changes nothing.

It only walks `.sl` files. Templates (`.html.slv`) are formatted by hand.

Generated code arrives formatted: `soli new` and every `soli generate` run their
`.sl` output through the formatter, so `soli fmt` on a fresh app is a no-op. A
diff from `fmt` means it is *your* code it reformatted.

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
- `security/unfiltered-mass-assignment` — `Model.create(params)` in controllers/services; whitelist with `permit` / `_permit_params`
- `component/props` — a component's `props(...)` declaration must use string-literal names with no duplicates

## Common commands

```bash
soli serve . --dev                    # dev server, hot reload, dev bar enabled
soli serve . --port 5011              # run without --dev (still single-process)

soli generate                         # list the generators that exist
soli generate scaffold post           # full resource (see the caveat above)
soli db:migrate generate create_posts  # scaffold migration
soli db:seed generate demo_posts      # scaffold seed
soli update docs                      # refresh CLAUDE.md / docs/ from this soli

soli db:migrate up                    # run pending migrations
soli db:migrate down                  # roll back last migration
soli db:migrate status                # show migration state

soli fmt                              # canonical layout, in place
soli fmt --check                      # CI gate: exit 1 on any unformatted file
soli lint                             # static analysis
soli test                             # run all tests in tests/
soli test --coverage --coverage-min 90.0

soli routes                           # print the expanded route table
soli routes -g posts                  # only routes matching "posts"
soli routes --json                    # machine-readable (for scripts/agents)
```

## Conventions to follow

1. **Prefer Ruby-style** for classes and control flow — `class Demo < Test ... end`, `def name(args) ... end`, `if cond ... end`. Reserve `fn { }` for free-standing functions and lambdas.
2. **Use type annotations** on public function signatures — they catch errors and document intent.
3. **Prefer immutability** — `const` for values that never change.
4. **Chain collection methods** instead of writing manual loops. For a sum,
   `lines.map(&:total).sum` replaces the accumulator loop — `&:name` reads a
   field as readily as it calls a method. Keep the loop when the body has a
   side effect (`save`, `delete`): a `filter` that writes reads badly.
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
15. **Put a blank line after a `return`** — unless the next line is another `return` or an `end`. This makes guard clauses (early returns) stand out from the code that follows. `soli fmt` inserts it for you, so you never have to hand-place it.

    ```soli
    def update(req)
      let post = Post.find(req.params["id"])
      return forbidden() unless can_edit?(post)  # guard clause

      post.update(this._permit_params(req.params))
      return redirect(post_path(post))
    end

    # Back-to-back returns and a return right before `end` need no blank line:
    def status_label(code)
      return "ok" if code == 200
      return "moved" if code == 301
      return "error"
    end
    ```
