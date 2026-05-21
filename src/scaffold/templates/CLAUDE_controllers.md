# Controllers

This directory holds request handlers. **One file per resource**:
`posts_controller.sl` defines `class PostsController < Controller`. Filenames
are `snake_case.sl`; class names are `PascalCase` ending in `Controller`.

Controllers stay thin: they pull params off the request, ask a model to do
the work, and return a response. Validation, persistence, and business rules
belong on the model — not here.

## The Controller contract

Inherit from `Controller`. The class body holds action methods (one per route)
plus a `static { ... }` block for layout and lifecycle hooks.

```soli
class PostsController < Controller
  static {
    this.layout = "application"
  }

  def index
    @posts = Post.all
    @title = "Posts"
  end
end
```

The class itself uses Ruby-style `class X < Y ... end`, and methods use
`def name ... end` — but the `static` block **requires braces** (`static { ... }`).
The Ruby-style `static ... end` form does not parse.

Free-function actions (no class wrapper) also work — see
`www/app/controllers/docs_controller.sl` for a real example — but the class
form is recommended for anything stateful or with hooks.

## The `static { }` block

Set the layout, register `before_action` / `after_action` hooks. The runtime
calls to `before_action` / `after_action` are no-ops — the controller registry
**textually scans the class body** at `soli serve` startup to wire hooks up,
so the syntax has to match what the scanner expects.

```soli
class PostsController < Controller
  static {
    this.layout = "application"

    # Runs on every action.
    this.before_action = fn(req) {
      @current_user = session_get("user_id")
      req
    }

    # Runs only on the listed actions.
    this.before_action(:show, :edit, :update, :delete) = fn(req) {
      @post = Post.find(params["id"])
      req
    }

    # after_action receives the response too.
    this.after_action = fn(req, response) {
      response
    }
  }
end
```

A `before_action` returning `req` proceeds; returning a response hash (one
with a `"status"` key) short-circuits and that response is returned to the
client.

## Reading the request

Inside an action, `req` is the request hash. The framework also exposes a few
globals so you don't have to dig:

| Read                  | What it gives you                                              |
|-----------------------|----------------------------------------------------------------|
| `params`              | Merged route + query + JSON body (= `req["all"]`). Most common.|
| `params["id"]`        | Path segment from `/posts/:id`.                                |
| `req["json"]`         | Parsed JSON body (when the request had one).                   |
| `req["query"]`        | Just the URL query string params.                              |
| `req["headers"]`      | Lowercased header hash. `req["headers"]["user-agent"]`.        |
| `req["method"]`       | `"GET"`, `"POST"`, ...                                          |
| `req["path"]`         | Request path.                                                   |
| `req["files"]`        | Array of uploaded files (multipart only). See **Handling file uploads**. |
| `cookies`             | **Global** read-only hash of parsed cookies. `cookies.theme`.   |

`params` reads route params, query params, and parsed body fields with the
same key — write `params["id"]` whether the value came from `/posts/:id`,
`?id=42`, or `{"id": 42}` in the JSON body.

## Response shapes

Soli supports an **implicit render** that covers 80% of cases. Only reach for
explicit response builders when you need something special.

### 1. Implicit render (preferred for the common case)

If an action returns anything that is **not** a response hash (i.e. doesn't
have a `"status"` key), the framework auto-renders the default template at
`app/views/<controller>/<action>.html.slv`. `PostsController#show` →
`posts/show`. So an action that just sets up view state can be a one-liner:

```soli
def show
  @post = Post.find(params["id"])
end
```

That's it. No `render(...)` call, no return statement. Every `@field` on the
instance is auto-injected as a view local (see "@-variables" below).

### 2. Explicit render — non-default template or extra locals

```soli
def create
  permitted = this._permit_params(params)
  @post = Post.create(permitted)
  if @post._errors
    return render("posts/new", { "title": "New post" })  # re-render form
  end
  redirect(post_path(@post))
end
```

### 3. JSON

```soli
def show
  @post = Post.find(params["id"])
  render_json({ "id": @post.id, "title": @post.title })
end
```

`render_json` sets `Content-Type: application/json` and serializes the hash
for you.

### 4. Plain text

```soli
def health
  render_text("OK")
end
```

### 5. Redirect

```soli
redirect("/posts")            # 302 to a path
redirect(post_path(post))     # use named-route helpers, not hand-built URLs
redirect(:back)               # back to the Referer if safe
redirect_external(url)        # opt-in to redirect to a different host
```

### 6. Short-circuit with `halt`

```soli
def admin
  halt(403, "Forbidden") unless current_user.admin
  @users = User.all
end
```

`halt(status, body)` immediately returns that response and skips the rest of
the action.

### 7. Raw hash — when you need full control

```soli
def webhook
  return {
    "status": 202,
    "headers": { "Content-Type": "application/json", "X-Request-Id": req["id"] },
    "body": "{\"ok\":true}"
  }
end
```

Any hash with a `"status"` key is treated as a final response and bypasses
auto-render.

### 8. Content negotiation with `respond_to`

```soli
def show
  @post = Post.find(params["id"])
  respond_to(req, {
    "html": fn() { render("posts/show") },
    "json": fn() { render_json({ "id": @post.id, "title": @post.title }) }
  })
end
```

## `@`-variables are injected into views

Every non-underscore-prefixed instance field you set on the controller is
auto-exposed as a top-level view local. `@post = Post.find(...)` makes `post`
available in the template.

```soli
def index
  @posts = Post.all
  @title = "Posts"
  @filter = params["filter"] ?? "all"
end
```

In `app/views/posts/index.html.slv`:

```erb
<h1><%= @title %></h1>
<p>Showing: <%= @filter %></p>
<% for post in @posts %>
  <li><%= h(post.title) %></li>
<% end %>
```

(Both `@title` and bare `title` resolve to the same value — `@` is the
canonical form.)

**Underscore-prefixed fields are private.** `@_internal_state = ...` is *not*
exposed to the view — useful for state shared between hooks and actions that
shouldn't leak into templates.

Because of this, you usually don't pass a data hash to `render` at all — set
`@fields` and let the framework do the rest. Reach for `render(view, {...})`
only when you need to render a *different* view than the default, or when you
want to override a field's name for the template.

## Full CRUD sample

```soli
# app/controllers/posts_controller.sl

class PostsController < Controller
  static {
    this.layout = "application"

    # Look up @post once for every action that needs it.
    this.before_action(:show, :edit, :update, :delete) = fn(req) {
      @post = Post.find(params["id"])
      req
    }
  }

  # GET /posts — implicit render of posts/index
  def index
    @posts = Post.all
    @title = "All posts"
  end

  # GET /posts/:id — @post set by before_action, implicit render of posts/show
  def show
    @title = "Post: #{@post.title}"
  end

  # GET /posts/new — implicit render of posts/new
  def new
    @post = Post.new
    @title = "New post"
  end

  # POST /posts
  def create
    permitted = this._permit_params(params)
    @post = Post.create(permitted)
    if @post._errors
      @title = "New post"
      return render("posts/new")     # explicit: re-render the form view
    end
    redirect(post_path(@post))
  end

  # GET /posts/:id/edit — implicit render of posts/edit
  def edit
    @title = "Edit #{@post.title}"
  end

  # PATCH/PUT /posts/:id
  def update
    permitted = this._permit_params(params)
    @post.update(permitted)
    if @post._errors
      return render("posts/edit")
    end
    redirect(post_path(@post))
  end

  # DELETE /posts/:id
  def delete
    @post.delete
    redirect(posts_path())
  end

  # Mass-assignment guard — whitelist the fields users can write.
  def _permit_params(params)
    {
      "title": params["title"],
      "body":  params["body"]
    }
  end
end
```

Notes on the sample:

- `before_action(:show, ...)` does the `Post.find` once instead of repeating
  it in four actions.
- `_permit_params` is a private helper (the leading `_` makes it
  non-routable). Only its return value is passed to `Model.create` / `update`.
- `index` / `show` / `new` / `edit` rely on **implicit render** — they just
  set `@fields` and exit.
- `create` and `update` use **explicit render** for the validation-failure
  re-render, because they need to render a *different* template than the
  default for the action.
- All redirects use **named helpers** (`post_path(post)`, `posts_path()`) —
  never hand-built URL strings.

## Validation re-render flow

`Model.create(attrs)` and `instance.save()` always return; on failure they
populate `_errors` on the returned instance. The controller checks `_errors`,
re-renders the form view passing the invalid instance, and the view displays
the errors.

```soli
@post = Post.create(permitted)
if @post._errors
  return render("posts/new")    # view reads @post._errors to show messages
end
redirect(post_path(@post))
```

**Don't wrap `Model.find` in nil-checks or `try/catch`.** On miss it raises
`RecordNotFound`, which the framework converts to a 404 automatically — so a
manual `if post.nil? ... end` branch is unreachable. Use `find_by(field, val)`
or `first_by(...)` when you want the "or nil" shape:

```soli
@post  = Post.find(params["id"])             # raises → 404
@draft = Post.find_by("slug", params["slug"]) # nil on miss
```

## Handling file uploads

For `multipart/form-data` requests, the framework parses every file part into
`req["files"]` — an array of hashes. Use the `find_uploaded_file(req, "field")`
helper to pull one by form field name; it returns `nil` if no file was
attached under that name or the request wasn't multipart.

```soli
photo = find_uploaded_file(params, "photo")
# nil, or:
# {
#   "name":         "photo",                # form field name
#   "filename":     "vacation.jpg",          # client-supplied filename
#   "content_type": "image/jpeg",
#   "size":         184_213,                 # bytes
#   "data":         "<base64 body>"
# }
```

**Don't read the bytes yourself.** When the field is declared with
`uploader(...)` on the model, hand the file straight to the auto-generated
`attach_<field>` method — it runs the configured MIME/size validations and
stores the blob in SoliDB for you:

```soli
def create
  @contact = Contact.create(this._permit_params(params))
  if @contact._errors
    return render("contacts/new")
  end

  photo = find_uploaded_file(params, "photo")
  if !photo.nil? && !@contact.attach_photo(photo)
    # attach_<field> populates @contact._errors on failure (bad MIME,
    # too large, or storage error). Re-render with the same flow you
    # use for validation errors.
    return render("contacts/new")
  end

  redirect(contact_path(@contact))
end
```

For multi-file fields (`uploader("attachments", { "multiple": true, ... })`),
iterate `req["files"]` directly and attach one by one:

```soli
def upload_batch
  @document = Document.find(params["id"])
  for file in (req["files"] ?? [])
    next unless file["name"] == "attachments"
    @document.attach_attachments(file)    # array column; each call pushes one blob
  end
  redirect(document_path(@document))
end
```

The whole upload contract (declarations, options, routes, cleanup) is in
`app/models/CLAUDE.md` → **Attachments and uploads**. Don't re-implement
blob storage in the controller.

### Form markup

The HTML form needs `enctype="multipart/form-data"` and one `<input type="file">`
per uploader field. Anything posted under a name that doesn't match an
uploader is just ignored.

```erb
<form action="<%= contacts_path() %>" method="post" enctype="multipart/form-data">
  <input type="text" name="name">
  <input type="file" name="photo" accept="image/*">
  <button type="submit">Create</button>
</form>
```

The cap on `req["files"]` array length is `SOLI_MAX_UPLOAD_FILES` (default 32
per request); excess files are dropped before the action runs.

## Cookies and sessions

Cookies are a read-only global; write them with `set_cookie`:

```soli
@theme = cookies["theme"] ?? "light"     # read
set_cookie("theme", "dark")              # write (Path=/)
```

Sessions are read/write via builtins (storage backend configured in
`config/application.sl`):

```soli
session_set("user_id", user.id)
let uid = session_get("user_id")    # nil if not set
session_has("user_id")              # bool
session_delete("user_id")
session_regenerate                  # after a successful login (security)
session_destroy                     # on logout
```

## Named route helpers

`resources("posts")` in `config/routes.sl` auto-registers a family of helpers
as globals. Use them — never concatenate URLs by hand.

| Route                | Path helper             | URL helper              |
|----------------------|-------------------------|-------------------------|
| `GET    /posts`      | `posts_path()`          | `posts_url()`           |
| `GET    /posts/new`  | `new_post_path()`       | `new_post_url()`        |
| `GET    /posts/:id`  | `post_path(post)`       | `post_url(post)`        |
| `GET    /posts/:id/edit` | `edit_post_path(post)` | `edit_post_url(post)` |

Custom routes named with `name: "..."` get the same treatment:
`get("/about", "pages#about", name: "about")` → `about_path()` / `about_url()`.

`*_path` returns a relative path; `*_url` is the absolute form (and respects
`enable_trust_proxy` if set in `config/application.sl`).

## Spec location

Every controller has a sibling spec at `tests/<name>_controller_spec.sl`.
`soli generate controller posts` scaffolds it for you. Use the E2E client:

```soli
describe("PostsController") do
  before_each() do
    as_guest()
  end

  test("GET /posts returns 200") do
    response = get("/posts")
    assert_eq(res_status(response), 200)
    assert_hash_has_key(assigns(), "posts")
  end

  test("POST /posts with invalid params re-renders new") do
    response = post("/posts", {})
    assert_eq(res_status(response), 200)
    assert_eq(view_path(), "posts/new.html")
  end
end
```

E2E helpers: `get` / `post` / `put` / `delete` to make requests; `res_status`,
`assigns()` (the `@field` hash exposed to the view), `view_path()`,
`render_template()`, `as_guest()`.

## Do / Don't

| Do                                                       | Don't                                                            |
|----------------------------------------------------------|------------------------------------------------------------------|
| Use named route helpers — `post_path(post)`              | Hand-build URLs — `"/posts/" + str(post.id)`                     |
| Let `Model.find` raise → 404                             | Wrap `Model.find` in `try/catch` or `if record.nil?`             |
| Whitelist via `_permit_params` before `Model.create`     | Pass `params` (or `req["json"]`) straight to `Model.create`      |
| Keep actions thin; push rules to the model               | Stuff validation / business logic into controller actions        |
| Set `@fields` and let the framework auto-render          | Repeat `@field` in `render(...)`'s data hash                     |
| Use `_`-prefixed methods for non-routable helpers        | Expose helper methods as public actions                          |
| Use `find_by` / `first_by` when you want nil-on-miss     | Add `if record.nil?` guards after `find` — they're unreachable   |
|                                                          | `import "../models/*.sl"` — models are auto-loaded               |
|                                                          | Use `db_query_raw` / backticks here — push raw SQL to the model  |

## Lints that fire here

- `style/redundant-model-import` — models in `app/models/*.sl` are auto-loaded;
  importing them from a controller triggers this.
- `smell/dangerous-server-builtin` — `db_query_raw`, `Trusted.*`, `System.shell`,
  and backtick commands are flagged inside controllers. Use the model layer
  or a dedicated service object instead.
- `smell/deep-nesting` — keep actions ≤4 levels of nesting. If you're past
  that, the action is doing too much.
- `smell/unreachable-code` — typically catches dead branches after an early
  `return` or after a `Model.find` nil-check that can never fire.
- `smell/undefined-local` — flags reads of a name that's never assigned in
  the action's scope (catches typos that bypass `let`).
- `naming/pascal-case` — class name must be `PascalCase`.
- `naming/snake-case` — action and helper names must be `snake_case`.

Run on the directory:

```bash
soli lint app/controllers/
soli lint app/controllers/posts_controller.sl
```
