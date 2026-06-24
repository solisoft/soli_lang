# Controllers

Controllers handle HTTP requests and return responses. SoliLang supports OOP-style controllers with class inheritance, before/after action hooks, and automatic request context injection.

## Creating a Controller

Create a file in `app/controllers/` with a `_controller.sl` suffix:

```soli
# app/controllers/users_controller.sl
class UsersController < Controller
  def index
    @user = this._current_user
    @posts = Post.all
    render("posts/index")
  end

  def show
    # @post is set by before_action
    render("posts/show")
  end

  def show
    @user = User.find(params["id"])
    @title = "User Details"
    render("users/show")
  end
end
```

> **`req` is implicit.** The request hash is automatically available as a global `req` variable — you don't need to declare it as a parameter. When you do need to destructure the request (e.g. `req.params`), just reference it directly. The explicit `def index` form still works for backward compatibility.

> **No imports needed for models.** Files under `app/models/` are auto-loaded by `soli serve` and the REPL, so classes like `User`, `Post`, etc. are available inside controller actions without `import`. (If you run a controller file standalone via `soli run`, add the imports back.) The linter warns about redundant imports via `style/redundant-model-import`.

## OOP Controller Architecture

### Class-Based Controllers

Controllers are classes that extend the base `Controller` class. Actions take no explicit parameters — `req` is available automatically:

```soli
class PostsController < Controller
  # Actions go here
  fn index end
  fn show end
end
```

### Static Configuration Block

Configure controllers using a `static { ... }` block. The `static { ... }` block and the hook function bodies require brace syntax (the controller registry parses them textually):

```soli
class ApplicationController < Controller
  static {
    # Set the layout for all actions
    this.layout = "application";

    # Before action that runs for all actions
    this.before_action = fn(req) {
      user_id = req.session["user_id"];
      if user_id != null {
        req["current_user"] = User.find(user_id);
      }
      req
    }
  }
end
```

### Controller Actions

Each public function in a controller is an action:

```soli
class PostsController < Controller
  fn index end
  fn show end
  fn new end
  fn create end
  fn edit end
  fn update end
  fn delete end
```

**Note:** Methods starting with `_` are private and not exposed as routes.

## Controller Inheritance

Controllers support multi-level inheritance. Create base controllers to share logic, hooks, and layouts across multiple controllers.

### Base Controller Pattern

Create an `ApplicationController` with shared configuration:

```soli
# app/controllers/application_controller.sl
class ApplicationController < Controller
  static {
    this.layout = "application";

    # Run for all actions
    this.before_action = fn(req) {
      # Authentication check
      user_id = req.session["user_id"];
      if user_id == null {
        return redirect("/login");
      }
      req["current_user"] = User.find(user_id);
      req
    }
  }

  # Shared helper method available to all subclasses
  fn _current_user
    req["current_user"]
  end
end
```

Subclasses inherit the configuration and can override it:

```soli
# app/controllers/posts_controller.sl
class PostsController < ApplicationController
  static {
    # Override layout for this controller
    this.layout = "posts";

    # Run before_action only for specific actions
    this.before_action(:show, :edit, :update, :delete) = fn(req) {
      @post = Post.find(params["id"])    # raises 404 if not found
      req
    }
  }

  fn index
    @user = this._current_user
    @posts = Post.all
  end

  fn show
    # @post is set by before_action — template renders posts/show
  end
end
```

### Multi-level Inheritance

You can create deeper hierarchies. Each level inherits hooks and methods from its parent:

```soli
# app/controllers/admin_controller.sl
# Extends ApplicationController, which extends Controller
class AdminController < ApplicationController
  static {
    this.layout = "admin";

    this.before_action = fn(req) {
      # Parent's before_action already ran (authentication)
      if req["current_user"]["role"] != "admin" {
        return halt(403, "Forbidden");
      }
      req
    }
  }
end

# app/controllers/admin_users_controller.sl
class AdminUsersController < AdminController
  fn index
    # Inherits: ApplicationController's auth + AdminController's admin check
    render("admin/users/index", { "users": User.all })
  end
end
```

### Inheritance Rules

- **Methods**: Inherited and can be overridden. Use `super.method()` to call the parent version.
- **before_action / after_action**: Inherited from parent controllers. Child hooks run after parent hooks.
- **layout**: Inherited if the child doesn't set its own.
- **Fields**: Declared in parent classes are available in child instances.
- **Loading order**: Parent controllers are automatically loaded before children (files are sorted by dependency).

## Before/After Action Hooks

### Before Actions

Run code before an action executes. Can filter to specific actions:

```soli
class PostsController < Controller
  static {
    # Run for all actions
    this.before_action = fn(req) {
      println("Before any action: " + req.path);
      req
    }

    # Run only for specific actions
    this.before_action(:show, :edit, :delete) = fn(req) {
      @post = Post.find(params["id"])    # raises 404 if not found
      req
    }
  }
end
```

**Short-circuiting:** Return a response hash (with a `"status"` field) from a before action to skip the action:

```soli
this.before_action = fn(req) {
  if req.session["user_id"] == null {
    return redirect("/login");
  }
  req  # Continue to action
}
```

### After Actions

Run code after an action executes:

```soli
class PostsController < Controller
  static {
    this.after_action = fn(req, response) {
      # Log the action
      println("Completed: " + req.path);
      response  # Return modified or original response
    }
  }
end
```

Filter after actions to specific actions:

```soli
this.after_action(:create, :update) = fn(req, response) {
  # Log changes after create/update
  println("Data modified");
  response
}
```

## Request Object

Access request data through the `req` parameter:

```soli
fn create
  # Path parameters
  id = params["id"];

  # Query string parameters
  page = req.query["page"];

  # Form data
  name = req.form["name"];

  # JSON body (if Content-Type is application/json)
  data = req.json;

  # HTTP headers
  auth = req.headers["Authorization"];

  # HTTP method
  method = req.method;

  # Original path
  path = req.path;

  # Session data
  user_id = req.session["user_id"];

  # Parsed cookies (from Cookie header)
  session_id = req.cookies["session_id"];

  # Same value via the global shorthand
  session_id = cookies.session_id;

  # Actual TCP peer IP (no port). Used by `rate_limit` for buckets.
  # Honored as the trustworthy client identifier when `enable_trust_proxy()`
  # is off; otherwise the rightmost `X-Forwarded-For` entry wins.
  client_ip = req["remote_addr"];

  # Store data for after_action or views
  req["my_data"] = some_value;
end
```

### Request Context in Controllers

The request object is automatically injected into your controller:

```soli
class PostsController < Controller
  fn show
    # Access params directly
    id = params["id"];

    # Or access via this (after injection)
    post = this.get_controller_field(req, "post");

    render("posts/show", { "post": post })
  end
end
```

## Cookies

The `cookies` global gives you read access to cookies sent by the client. It is a hash parsed from the `Cookie` header, defaulting to `{}` when no cookies are present:

```soli
fn show
  # Read a cookie
  theme = cookies["theme"] or "light";

  # Dot access also works
  session_id = cookies.session_id;
end
```

### set_cookie(name, value)

Write a response cookie. The cookie is sent back to the client as a `Set-Cookie` header:

```soli
fn login
  set_cookie("session_id", "abc123");
  set_cookie("theme", "dark");

  {"status": 200, "body": "Logged in"}
end
```

> **Note:** `set_cookie` sets minimal cookie attributes (`Path=/`). For advanced options like `Max-Age`, `Secure`, `HttpOnly`, or `SameSite`, set the `Set-Cookie` header directly in your response.

Cookies set via `set_cookie` are visible in templates and subsequent reads within the same request through the `cookies` global.

## Returning Responses

### Render a Template

```soli
fn index
  render("home/index", {
    "title": "Welcome",
    "message": "Hello!"
  })
end
```

### Instance Fields Auto-Exposed to Views

Any field set on the controller instance during an action — via either `this.foo = ...` or the `@foo` shorthand — is automatically available as a bare local in the view that action renders. You can drop the data hash entirely when you just want to pass data through.

```soli
class PostsController < Controller
  fn show
    @post = Post.find(params["id"]);
    @comments = Comment.where({"post_id": @post.id}).all;
    render("posts/show")    # view sees `post` and `comments` with no data hash
  end
end
```

In the view you can reference these either as a bare local or with the **same `@` prefix you used in the controller** — in views, `@post` falls back to the `post` local, so both forms render the same value:

```erb
<%# app/views/posts/show.html.erb %>
<h1><%= @post.title %></h1>   <%# @-form, mirrors the controller %>
<%= post.body %>              <%# bare local, identical result %>

<h2>Comments (<%= @comments.length %>)</h2>
<% for c in comments %>
  <p><%= c.body %></p>
<% end %>
```

A view's `@foo` that has no matching local renders as empty (`nil`), the same as any other absent template local.

Rules:

- **Explicit render data wins.** `render("v", {"post": other})` overrides `@post`.
- **Framework fields are never re-exposed** via this path: `req`, `params`, `session`, `headers` always flow through their normal channels, so an action can't accidentally shadow them.
- **Scoped to the current action.** No cross-action, cross-controller, or cross-request leakage — a fresh controller instance is created per request.
- **Partials are not auto-exposed.** Always pass data to `render_partial(...)` / `partial(...)` explicitly. Inside the partial, read keys as bare identifiers (`<%= name %>`) or via the `locals` hash (`<%= locals["class"] %>`) — see [Views → The `locals` hash](./views.md#the-locals-hash).

> **Note on `@foo`:** `@foo` is a general language shorthand for `this.foo` inside any class method, not a controller-only feature. See [Soli Language → The `@` Sigil](./soli-language.md#the--sigil--shorthand-for-this) for the full rules.

### Request-Context Helpers in Views

These helpers read the current request directly — no need to plumb `current_path` or `current_method` through the data hash:

| Helper | Returns |
|--------|---------|
| `current_path()` | Request pathname, e.g. `"/users"`. `null` when called outside a request. |
| `current_method()` | HTTP method, e.g. `"GET"`. `null` outside a request. |
| `current_path?(p)` | `true` if the current path equals `p` exactly. Handy for active-link checks. |

```erb
<%# app/views/layouts/_nav.html.erb %>
<nav>
  <a href="/users" class="<%= current_path?("/users") ? "active" : "" %>">Users</a>
  <a href="/posts" class="<%= current_path?("/posts") ? "active" : "" %>">Posts</a>
</nav>

<p>You are viewing <%= current_path() %> (<%= current_method() %>).</p>
```

For prefix matches (e.g. any path under `/users`), compose with string methods: `current_path().starts_with("/users")`.

### Render with Custom Layout

Set the layout in your controller:

```soli
class PostsController < Controller
  static {
    this.layout = "posts";  # Uses layouts/posts.html.slv
  }

  fn show
    # @post is available from before_action
  end

  # Skip layout for specific action
  fn json_only
    render_json({ "data": "value" }, layout: false)
  end
end
```

### Per-Action Layouts

A single controller can serve different layouts to different actions — declared
once in the `static { ... }` block, so you never have to repeat `layout:` on
each `render(...)` call. `this.layout = "..."` sets the controller-wide default;
`this.layout("name", only: [...])` / `except: [...]` override it for specific
actions:

```soli
class ReportsController < Controller
  static {
    this.layout = "admin";                              # default for every action

    this.layout("print", only: [:invoice, :receipt]);  # these two use "print"
    this.layout("blank", except: [:index]);            # everything else but :index uses "blank"
  }

  def invoice
    render("reports/invoice")   # → "print" layout, no `layout:` needed
  end

  def index
    render("reports/index")     # → "admin" (excluded from "blank", not in "print")
  end
end
```

Resolution rules:

- Rules are checked **in declaration order**; the **first match wins**, then the
  controller-wide `this.layout` default, then the framework `"application"`
  layout.
- `only:` limits a rule to the listed actions; `except:` applies it to every
  action *but* those listed. Omit both and the rule applies to all actions
  (equivalent to setting the default).
- An explicit `layout:` passed to `render(...)` (including `layout: false` to
  skip layouts) always wins over any registered rule.
- Per-action rules are **inherited** by subclasses just like the default
  layout; a child's own rule for the same action overrides the inherited one.

Edits to these declarations are picked up on the next request in `--dev` mode —
no server restart required.

### Redirect

```soli
fn create
  # Process form data...

  # Redirect to another page
  redirect("/users")
end

fn update
  # After update, redirect to show page
  user_id = params["id"];
  redirect("/users/" + user_id)
end
```

`redirect()` only accepts local absolute paths such as `/login` or `/users/123`. This prevents accidentally turning user-controlled input into an open redirect.

For trusted external destinations, use `redirect_external()` explicitly:

```soli
fn oauth_start
  redirect_external("https://github.com/login/oauth/authorize")
end
```

To send the user back where they came from, pass the `:back` symbol. Soli reads the `Referer` header and only honors it when scheme + host match the current request — external referers (or a missing/malformed header) fall back to `/`.

```soli
fn destroy
  Comment.find(params["id"]).delete()
  redirect(:back)
end
```

### JSON Response

```soli
fn api_users
  render_json({
    "users": [
      {"id": 1, "name": "Alice"},
      {"id": 2, "name": "Bob"}
    ]
  })
end
```

> **Security — instance serialisation.** `render_json(instance)` (and any code path that JSON-stringifies a `Value::Instance`, including `to_json` on a Model record) **omits sensitive fields by default**. Names matching `password*`, `*_token`, `*_digest`, `*_secret`, or `*_hash` are dropped, as are `_`-prefixed framework internals (`_errors`, `_text`, `_pending_translations`, …). The standard Model metadata (`_key`, `_id`, `_rev`, `_created_at`, `_updated_at`) is still included. If you need to expose a field whose name matches one of the patterns, build the response shape explicitly: `render_json({ "id": user._key, "email": user.email, "auth_token_count": user.auth_token_count })` instead of `render_json(user)`.

For a reusable model-side shape, define an `as_json` method on the Model subclass:
>
> ```soli
> class User < Model
>   def as_json
>     return { "id": this._key, "email": this.email, "name": this.name }
>   end
> end
>
> # controller — render_json auto-dispatches through the user method:
> render_json(user)
> # equivalent to: render_json(user.as_json())
> ```
>
> Same convention as Rails' `ActiveModel::Serializers#as_json`. When `render_json` receives an `Instance` whose class declares `def as_json`, the framework calls the method first and forwards the resulting Hash to `render_json`. Models without an `as_json` method fall back to the default-deny filter described above. Defining `as_json` gives you a single declarative place to evolve a model's public API shape.

### Plain Text

```soli
fn ping
  render_text("pong")
end
```

### Content Negotiation with `respond_to`

For actions that need to serve multiple formats (HTML, JSON, CSV, PDF, XLSX, partial HTMX, XHR-only JSON, …), use `respond_to`. It picks the right branch based on the request and falls back to `406 Not Acceptable` when no registered format matches.

```soli
def show
  post = Post.find(req["params"]["id"]);
  respond_to(req, fn(format) {
    format.html(fn()  render("posts/show", {"post": post}));
    format.json(fn()  render_json(post));
    format.csv(fn()   render_csv_for(post));
    format.pdf(fn()   render_pdf_for(post));
    format.excel(fn() render_xlsx_for(post));
    format.htmx(fn()  render("posts/_show_partial", {"post": post}, {"layout": false}));
    format.xhr(fn()   render_json({"id": post.id}));
    format.any(fn()   render("posts/show", {"post": post}));  // optional catch-all
  })
}
```

A terser hash form is also supported:

```soli
respond_to(req, {
  "html": fn() render("posts/show", {"post": post}),
  "json": fn() render_json(post)
})
```

**Format detection priority** (first match wins):

1. `HX-Request: true` header → `htmx` branch.
2. `X-Requested-With: XMLHttpRequest` header → `xhr` branch.
3. URL extension: `.html`, `.json`, `.xml`, `.csv`, `.pdf`, `.xlsx`/`.xls`, `.txt`.
4. `?format=…` query parameter.
5. `Accept` header — parsed with q-values; `*/*` falls through to the first registered handler.

**Available format tokens**: `html`, `json`, `xml`, `csv`, `pdf`, `excel`, `htmx`, `xhr`, `text`, `any`. Registering `any` makes it the catch-all (no 406). Last registration wins on duplicates.

> Header keys in `req["headers"]` are lowercased — read `req["headers"]["accept"]`, not `Accept`.

### Error Response

```soli
fn show
  id = params["id"];
  if id == "" {
    return halt(400, "Missing ID");
  }
  user = find_user(id);
  if user == null {
    return halt(404, "User not found");
  }
  render("users/show", {"user": user})
end
```

## Controller Context

Controllers have access to context through `this`:

```soli
class PostsController < Controller
  static {
    this.layout = "posts";
    this.before_action = fn(req) {
      # Store data on request for later use
      req["post"] = Post.find(params["id"]);
      req
    }
  }

  fn show
    # Access the post set by before_action
    post = req["post"];

    render("posts/show", { "post": post })
  end

  # Access request parameters
  fn _get_id -> String
    params["id"]
  end
end
```

## Strong Parameters

Validate and sanitize input:

```soli
fn create
  params = req.form;
  clean_params = {
    "name": params["name"] ?? "",
    "email": params["email"] ?? "",
    "age": int(params["age"] ?? "0")
  };
end
```

## Routing to Controller Actions

Routes use `controller#action` syntax:

```soli
# config/routes.sl
get("/", "home#index");
get("/users", "users#index");
get("/users/:id", "users#show");
post("/users", "users#create");
```

The router automatically:
1. Instantiates a new controller instance per request
2. Injects the request context
3. Runs before_action hooks
4. Calls the action method
5. Runs after_action hooks
6. Returns the response

## File Naming Convention

| File | Class | Route Prefix |
|------|-------|--------------|
| `home_controller.sl` | `HomeController` | `home#` |
| `users_controller.sl` | `UsersController` | `users#` |
| `posts_controller.sl` | `PostsController` | `posts#` |
| `admin/users_controller.sl` | `AdminUsersController` | `admin/users#` |
| `admin/merchants_controller.sl` | `AdminMerchantsController` | `admin/merchants#` |

## Nested Controller Directories

Controllers can be organized into subdirectories under `app/controllers/`. The directory path becomes part of the controller key, the route base path, and the class name.

```
app/controllers/
├── home_controller.sl              # HomeController            → /
├── users_controller.sl             # UsersController           → /users
└── admin/
    ├── merchants_controller.sl     # AdminMerchantsController  → /admin/merchants
    └── user_profiles_controller.sl # AdminUserProfilesController → /admin/user_profiles
```

Both `_` and `/` act as word separators when deriving the class name, so `admin/user_profiles_controller.sl` becomes `AdminUserProfilesController` (not `Admin::UserProfilesController`).

Reference nested controllers from `config/routes.sl` using the same `controller#action` syntax with a `/`-separated key:

```soli
get("/admin/merchants", "admin/merchants#index");
get("/admin/merchants/:id", "admin/merchants#show");

# Or with resources()
resources("/admin/merchants", "admin/merchants");
```

Subdirectories are watched recursively in dev mode, so adding or editing a nested controller triggers hot reload like any top-level controller.

## Best Practices

1. **Keep controllers thin, models fat** - Business logic belongs in models
2. **Use before_action for authentication** - Common pattern for access control
3. **Validate parameters before processing** - Use strong parameters pattern
4. **Return appropriate HTTP status codes** - 200, 201, 400, 401, 404, 500
5. **Use redirects after successful POST requests** - Prevent form resubmission
6. **Use private helper methods** - Methods starting with `_` are not exposed
7. **Create ApplicationController** - Base class for shared configuration
8. **Use layouts consistently** - Set default layout in ApplicationController

## Testing Controllers

See the [Testing Guide](/docs/testing) for comprehensive information on testing controllers with both HTTP integration tests and direct action calls.
