# Models

This directory holds the data layer. **One file per model**: `post.sl` defines
`class Post < Model`. Filenames are `snake_case.sl`; class names are
`PascalCase`, singular.

Models are auto-loaded by `soli serve` — controllers and migrations reference
them by class name without an `import`. Adding `import "../models/*.sl"`
inside a controller trips `style/redundant-model-import`.

Models own validation, persistence, and business rules. Controllers are thin;
push every "X happens when Y is created" rule into the model layer.

## Anatomy of a model

```soli
class Post < Model
  # Associations
  belongs_to("user")
  has_many("comments")

  # Validations
  validates("title", { "presence": true, "min_length": 3, "max_length": 200 })
  validates("body",  { "presence": true })

  # Lifecycle callbacks
  before_save("normalize_title")
  after_create("notify_subscribers")

  # Named scopes
  scope("published", fn() { this.where({ "status": "published" }) })
  scope("recent",    fn() { this.order("created_at", "desc").limit(10) })

  # Instance methods (your own logic)
  def normalize_title
    this.title = this.title.trim()
  end

  def notify_subscribers
    # ...
  end
end
```

Models are **untyped** — you don't declare `title: String` at class level.
Fields are inferred from what you assign / persist, and validated by the rules
you register.

## Inherited CRUD (don't override)

These come with `< Model`. They use the worker's pre-configured SoliDB
connection.

| Class method                          | What it does                                                          |
|---------------------------------------|-----------------------------------------------------------------------|
| `Model.all`                           | All records as instances.                                              |
| `Model.find(id)`                      | Lookup by id. **Raises `RecordNotFound` on miss → 404 in controllers.**|
| `Model.find_by(field, val)`           | First match, or `nil`.                                                 |
| `Model.first_by(field, val)`          | First match with ordering, or `nil`.                                   |
| `Model.where({...})` / `where("doc.x == @a", {"a": ...})` | Filter (see Querying below).                       |
| `Model.create({...})`                 | Insert with validation. Always returns an instance (see `_errors`).    |
| `Model.find_or_create_by(field, val, defaults?)` | Look up or insert.                                          |
| `Model.upsert(key, data)`             | Insert if absent, else update.                                         |
| `Model.create_many([{...}, ...])`     | Batch insert.                                                          |
| `Model.count`                         | Row count.                                                             |
| `Model.delete_all`                    | Wipe the collection. Dangerous — use with care.                        |
| `Model.with_deleted` / `Model.only_deleted` | Include / restrict to soft-deleted records.                       |
| `Model.transaction`                   | Open a transaction (returns a Transaction with `get/create/update/delete/commit/rollback`). |
| `Model.paginate({ "page": 1, "per": 20 })` | Returns `{ "records": [...], "pagination": {...} }`.              |

| Instance method                       | What it does                                                          |
|---------------------------------------|-----------------------------------------------------------------------|
| `instance.save([attrs])`              | Insert or update. Returns `true` / `false`.                            |
| `instance.update({...})`              | Apply attrs and save.                                                  |
| `instance.delete`                     | Delete (or soft-delete if the model uses soft-deletes).                |
| `instance.restore`                    | Undo a soft-delete.                                                    |
| `instance.reload`                     | Re-fetch from the DB; refresh all fields.                              |
| `instance.increment("counter", n=1)`  | Atomic `+=`.                                                           |
| `instance.decrement("counter", n=1)`  | Atomic `-=`.                                                           |
| `instance.touch`                      | Bump `_updated_at`.                                                    |
| `instance._errors`                    | Array of `{"field": ..., "message": ...}` after a failed save/create.  |

If you find yourself shadowing one of these with a `static def all ... end` or
similar, **stop** — you're working against the framework. Add a `scope` or a
class-side helper with a different name instead.

## Querying

Three forms, in order of preference:

### 1. Hash form (safe — use this for user input)

```soli
User.where({ "role": params["role"], "active": true })
```

Keys are validated against the model's field set; values are bound as
parameters. Safe to pass `params` values straight in.

### 2. String form with binds (developer-trusted, but parameterized)

```soli
User.where("doc.age >= @min AND doc.role == @role", {
  "min":  params["min_age"],
  "role": params["role"]
})
```

The condition is your code — **never concatenate user input into the string**.
Bind everything that came from outside via `@name` placeholders. Use this when
the hash form isn't expressive enough (range, OR, function calls).

### 3. Raw `@sdbql{ ... }` (when the ORM doesn't fit)

```soli
let min_age = 18
let users = @sdbql{
  FOR u IN users
  FILTER u.age >= #{min_age}
  SORT u.name ASC
  LIMIT 50
  RETURN u
}
```

`#{expr}` inside the block is **bound as a parameter** (not string-interpolated
as text), so it's safe. Reach for this for joins, subqueries, and anything
hand-tuned. Returns raw documents, not model instances.

### Chaining

`where`, `order`, `limit`, `offset`, `select`/`fields`, `pluck`, `includes`,
`includes_count`, `join` all return a chainable `QueryBuilder`. Terminate the
chain with one of:

| Terminator         | Returns                                                  |
|--------------------|----------------------------------------------------------|
| `.all`             | Array of instances.                                       |
| `.first`           | First instance, or `nil`.                                 |
| `.count`           | Number.                                                   |
| `.exists`          | Boolean.                                                  |
| `.pluck("field")`  | Array of values.                                          |
| `.sum/avg/min/max("field")` | Numeric aggregate.                              |
| `.group_by(field, func, agg_field)` | Array of `{group, result}` hashes.       |

```soli
let recent = Post
  .where({ "status": "published" })
  .order("created_at", "desc")
  .limit(20)
  .all

let total_views = Post.where({ "user_id": user.id }).sum("views")
```

## Validations

Pass an options hash to `validates`. All keys are optional; combine freely.

| Option                 | Effect                                                                     |
|------------------------|----------------------------------------------------------------------------|
| `"presence": true`     | Required; rejects `null`, `""`, missing.                                    |
| `"uniqueness": true`   | Best-effort pre-check + relies on a unique DB index for atomicity.          |
| `"min_length": N`      | String length ≥ N.                                                          |
| `"max_length": N`      | String length ≤ N.                                                          |
| `"format": "regex"`    | String matches the pattern.                                                 |
| `"numericality": true` | Value is a number.                                                          |
| `"min": N` / `"max": N`| Numeric bounds.                                                             |
| `"custom": "method"`   | Calls `this.method()` for full custom checks; method appends to `_errors`.  |

```soli
class User < Model
  validates("email", { "presence": true, "uniqueness": true, "format": "^[^@]+@[^@]+$" })
  validates("age",   { "numericality": true, "min": 0, "max": 150 })
  validates("name",  { "custom": "validate_name" })

  def validate_name
    if this.name.blank? || this.name.length() < 2
      this._errors.push({ "field": "name", "message": "too short" })
    end
  end
end
```

### Reading `_errors`

`_errors` is an **array of hashes**, not a hash keyed by field:

```soli
@user = User.create(params)
if @user._errors
  for err in @user._errors
    print("#{err.field}: #{err.message}")
  end
end
```

On a clean save `_errors` is `nil` (not `[]`). Check `if @user._errors` —
truthiness is correct here.

## Associations

```soli
class Post < Model
  belongs_to("user")              # Post.user_id (FK), post.user (instance)
  has_many("comments")            # user.comments (QueryBuilder)
  has_one("featured_image")       # one-to-one
  has_and_belongs_to_many("tags") # M2M via join collection
end
```

Conventions:

- `belongs_to("user")` adds the `user_id` FK to **this** collection. Instance
  accessor `post.user` lazy-loads on first read.
- `has_many("comments")` adds the FK on the *other* side (comments have
  `user_id`). The accessor returns a `QueryBuilder` — chain on it:
  `user.posts.where({"status": "published"}).count`.
- `has_one` works like `has_many` but returns a single instance.
- All four accept overrides:
  `belongs_to("author", { "class_name": "User", "foreign_key": "author_id" })`.

### Eager loading

Avoid N+1 by pre-loading on the query:

```soli
let posts = Post.where({...}).includes("user", "comments").all
# posts[0].user and posts[0].comments are now materialized in memory
```

`includes_count("comments")` adds a `comments_count` integer to each
instance — handy for index pages.

## Scopes

A `scope` is a class-side query alias. The body runs with `this` bound to a
fresh `QueryBuilder`, so `this.where(...)` / `this.order(...)` chain off it.

```soli
class Post < Model
  scope("published", fn() { this.where({ "status": "published" }) })
  scope("by_user",   fn(user_id) { this.where({ "user_id": user_id }) })
end

Post.published.order("created_at", "desc").limit(20).all
Post.by_user(current_user.id).published.count
```

Both `Post.published` and `Post.published()` invoke the scope.

For class-body DSL closures — scopes, validators, callbacks — prefer the
explicit `fn() { this.method(...) }` form over implicit-self alternatives.

## Lifecycle callbacks

Eight hooks, each takes a **method-name string** (not a lambda):

| Hook              | When it fires                                  |
|-------------------|------------------------------------------------|
| `before_create`   | New record, before insert.                     |
| `after_create`    | New record, after insert succeeded.            |
| `before_update`   | Existing record, before save.                  |
| `after_update`    | Existing record, after save succeeded.         |
| `before_save`     | Either insert or update, before persist.       |
| `after_save`      | Either insert or update, after persist.        |
| `before_delete`   | Before `instance.delete`.                       |
| `after_delete`    | After delete succeeded.                         |

```soli
class Post < Model
  before_save("normalize_title")
  after_create("notify_subscribers")

  def normalize_title
    this.title = this.title.trim()
  end

  def notify_subscribers
    # send mail, enqueue job, etc.
  end
end
```

A `before_*` callback that mutates `this._errors` (or returns `false`,
depending on hook) aborts the operation.

## Other class-body helpers

- `attr_accessible(field1, field2, ...)` — whitelist fields for mass-assignment.
  When set, `Model.create(params)` silently drops any key not on the list.
  Pair with controller-side `_permit_params` for defense in depth.
- `uploader("avatar", { ... })` — declare a blob attachment field. See
  **Attachments and uploads** below for the full contract.
- `translate("title", "body")` — declare translatable fields (i18n).

## Attachments and uploads

Declare a blob attachment with `uploader("field", { ... })` in the class body.
The framework wires the validation, storage (SoliDB blob collection), and
URL/HTTP plumbing for you — controllers don't need to touch the blob store.

```soli
class Contact < Model
  uploader("photo", {
    "multiple":      false,
    "content_types": ["image/jpeg", "image/png", "image/webp"],
    "max_size":      2_000_000,        # bytes — rejects above this
    "collection":    "contact_photos"   # optional; defaults to "contact_photos"
  })

  uploader("attachments", {             # multi-file field
    "multiple":      true,
    "content_types": ["application/pdf", "image/png"],
    "max_size":      5_000_000
  })
end
```

| Option          | Meaning                                                                          |
|-----------------|----------------------------------------------------------------------------------|
| `multiple`      | `false` (default) → one blob per record. `true` → array of blob ids.              |
| `content_types` | Allow-list of MIME types. Anything else is rejected before storage.               |
| `max_size`      | Hard cap in bytes. Above this → `_errors` populated, no blob stored.              |
| `collection`    | SoliDB blob collection name. Defaults to `<class_snake>_<field>s` (`contact_photos`). |

The uploader adds a `<field>_blob_id` column (single) or `<field>_blob_ids`
array (multiple) to the document. You don't read those directly — use the
auto-generated instance methods below.

### Auto-generated instance methods

`uploader("photo", ...)` adds three methods on every instance:

| Method                        | What it does                                                                |
|-------------------------------|------------------------------------------------------------------------------|
| `contact.attach_photo(file)`  | Validate + store the file, update the `<field>_blob_id(s)` column.            |
| `contact.detach_photo(id?)`   | Delete the blob and clear the column. `id` is required when `multiple: true`. |
| `contact.photo_url(opts?)`    | Return the public URL for the stored blob (or `nil` if none stored).          |

`file` is the hash returned by `find_uploaded_file(req, "photo")` in a
controller (see **Controllers — Handling file uploads**). On a failed attach,
`contact._errors` is populated and `attach_photo` returns `false` — the same
error-rendering flow used by `Model.create` validation failures.

```soli
def create
  @contact = Contact.create(this._permit_params(params))
  if @contact._errors
    return render("contacts/new")
  end

  photo = find_uploaded_file(params, "photo")
  if !photo.nil? && !@contact.attach_photo(photo)
    return render("contacts/new")    # attach failed → _errors set
  end

  redirect(contact_path(@contact))
end
```

### Wiring the upload routes

Add `uploads("contacts", "photo")` to `config/routes.sl`. That single call
registers a GET / POST / DELETE family (plus `:blob_id`-scoped variants for
multi-file fields) backed by the framework's built-in `AttachmentsController`:

```soli
# config/routes.sl
resources("contacts")
uploads("contacts", "photo")        # for the photo field
uploads("contacts", "attachments")  # for the multi-file field
```

| Route                                                | Purpose                          |
|------------------------------------------------------|----------------------------------|
| `GET    /contacts/:id/photo`                          | Stream the blob (with transforms).|
| `POST   /contacts/:id/photo`                          | Upload a file (multipart).        |
| `DELETE /contacts/:id/photo`                          | Detach (single) or the named blob.|
| `GET    /contacts/:id/attachments/:blob_id`           | Stream one entry from a multi field. |
| `DELETE /contacts/:id/attachments/:blob_id`           | Remove that one entry.            |

The default routes target the framework's `AttachmentsController` — override
by defining your own `class AttachmentsController < Controller` if you need
auth checks, signed URLs, etc. Soli's loader processes app controllers after
the framework prelude, so a same-named class shadows the default cleanly.

### Cleanup on delete

`detach_all_uploads(record)` is available for `before_delete` hooks if you
need to wipe attached blobs alongside the record. Without it, a deleted
record leaves orphan blobs in the collection.

```soli
class Contact < Model
  uploader("photo", { ... })
  before_delete("cleanup_uploads")

  def cleanup_uploads
    detach_all_uploads(this)
  end
end
```

## Inspecting AQL queries (`--dev`)

`dev_queries()` returns the AQL stack issued for the current request when the
server runs with `--dev`. Each entry is `{ "query": String, "bind_vars":
Hash | null, "duration_ms": Float }`. Useful for building a debug bar or
spotting N+1s.

```erb
<% if dev_queries().length() > 0 %>
  <div class="dev-bar">
    <% for q in dev_queries() %>
      <pre><%= q.query %> (<%= q.duration_ms %>ms)</pre>
    <% end %>
  </div>
<% end %>
```

In production, `dev_queries()` returns `[]` (so the `length() > 0` guard
collapses to nothing) with zero overhead.

## Do / Don't

| Do                                                            | Don't                                                              |
|---------------------------------------------------------------|--------------------------------------------------------------------|
| Put validation rules on the model                             | Validate in the controller                                          |
| Push business rules into model methods                        | Spread "what happens when X is created" across controllers          |
| Use `where({...})` (hash form) for user input                 | Concatenate strings: `where("role = " + params["role"])`            |
| Use `#{expr}` bound interpolation in `@sdbql{...}`            | Use `\(expr)` — that's a docs typo; the lexer rejects it            |
| Use `find_by` / `first_by` when nil-on-miss is correct        | Wrap `find` in try/catch to convert raise → nil                     |
| Chain `.where.order.limit.all` for readability                | Build SDBQL strings in the controller                               |
| Use `includes(...)` to dodge N+1                               | Call `post.user` inside a `for post in posts` loop without eager load |
| Use callbacks for cross-cutting concerns (timestamps, slugs)  | Use callbacks for anything you'd want to disable in a test          |
| Declare `attr_accessible` on the model                        | Trust the controller to filter every caller                         |

## Spec location

Model specs live in `tests/<name>_model_spec.sl`. Example:

```soli
describe("Post") do
  test("rejects empty title") do
    @post = Post.new({ "title": "", "body": "x" })
    @post.save
    assert(@post._errors)
    assert_eq(@post._errors[0].field, "title")
  end

  test("normalize_title trims whitespace before save") do
    @post = Post.create({ "title": "  hello  ", "body": "x" })
    assert_eq(@post.title, "hello")
  end
end
```

Hit the real DB in model specs — that's where the validation and constraint
behavior actually lives. Don't mock the database.
