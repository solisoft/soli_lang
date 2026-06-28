# Models

Models manage data and business logic in your MVC application. SoliLang provides a simple OOP-style interface for database operations.

## Defining Models

Create model files in `app/models/`. The collection name is **automatically derived** from the class name:

- `User` → `"users"`
- `BlogPost` → `"blog_posts"`
- `UserProfile` → `"user_profiles"`

**Automatic Collection Creation**: When you call a Model method (like `create()`, `all()`, `find()`, etc.) on a collection that doesn't exist yet, SoliLang will automatically create the collection for you. If the configured **database** doesn't exist yet either, it is created on that same first call before the collection. This means you can start using your models immediately without running migrations first.

- `User` → `"users"`
- `BlogPost` → `"blog_posts"`
- `UserProfile` → `"user_profiles"`

```soli
# app/models/user.sl
class User < Model
end
```

```soli
# app/models/blog_post.sl
class BlogPost < Model
end
```

That's it! No need to manually specify collection names or field definitions.

## Auto-Loading

Every `.sl` file under `app/models/` is loaded automatically at startup — by `soli serve` (in each worker) and by the REPL. Model classes are therefore available everywhere (controllers, views, other models, the REPL) without an `import` statement.

```soli
# app/controllers/users_controller.sl — no import needed
class UsersController < Controller
  fn index
    render("users/index", { "users": User.all })
  end
end
```

Loading is **recursive** — subdirectories are walked too, so `app/models/billing/invoice.sl` is auto-loaded just like a top-level file. The same recursive auto-loader also covers two sibling directories, loaded before controllers so they can be referenced freely:

- **`app/services/`** — integration / domain-service classes (Stripe, mailers, etc.)
- **`app/policies/`** — authorization policies (see [Authorization](authorization.md))

Within a directory, files load in alphabetical order and *before* their subdirectories (top-down). Soli executes files eagerly, so if one class extends another defined in a different file, keep the base class at an equal-or-shallower depth (e.g. `app/models/application_record.sl`) so it loads first.

In `--dev`, edits to nested files under these directories hot-reload without a restart.

If you run a model, service, or controller file directly with `soli run path/to/file.sl`, the auto-loader does **not** run — in that case you still need explicit imports.

## CRUD Operations

> **Auto-creation**: All Model operations automatically create the collection — and the database itself, if it's missing too — when it doesn't exist. This only happens on the first call that encounters a missing collection or database.

### Creating Records

```soli
result = User.create({
  "email": "alice@example.com",
  "name": "Alice",
  "age": 30
});
# Returns: { "valid": true, "record": { "id": "...", "email": "...", ... } }
# Or on validation failure: { "valid": false, "errors": [...] }
```

### Finding Records

```soli
# Find by ID
user = User.find("user123");

# Find all
users = User.all;

# Find with where clause — Hash form (recommended for user input)
# Each key is validated as an AQL identifier and values flow through
# bind parameters, so attacker-controlled values can never reach the
# query template. Equality semantics: every pair joins with AND.
admins = User.where({ "role": "admin", "active": true }).all;
alice  = User.where({ "email": "alice@example.com" }).first;

# Find with where clause — string form (developer-trusted only)
# Use this when you need operators (>=, IN, etc.) or boolean expressions.
# The string MUST NOT come from untrusted input — see Security note below.
adults = User.where("doc.age >= @age", { "age": 18 }).all;
results = User.where("doc.age >= @min_age AND doc.role == @role", {
  "min_age": 21,
  "role": "admin"
}).all;
```

> **Security — `where(...)` filter forms.** The Hash form
> (`where({field: value, ...})`) is safe for user input: keys are
> validated as `[A-Za-z_][A-Za-z0-9_]*` identifiers and values are
> bound, so nothing from `req["params"]` can become AQL syntax. The
> string form (`where("doc.foo == @foo", {...})`) splices the filter
> argument verbatim into the AQL FILTER clause — treat it as
> developer-trusted only, like a `format!()` template. Building a
> filter string from request data **will leak full AQL injection**.
> When the operators you need go beyond equality, prefer composing
> small string-form clauses around literal strings rather than
> concatenating user input into them.

### Updating Records

```soli
# Static method: update by ID
User.update("user123", {
  "name": "Alice Smith",
  "age": 31
});

# Instance method: modify fields and save
user = User.find("user123");
user.name = "Alice Smith";
user.age = 31;
user.save();

# Instance method with bulk-update hash (same merge-then-persist path,
# one call instead of N assignments + save)
user = User.find("user123");
user.save({ "name": "Alice Smith", "age": 31 });

# `.update(hash)` is equivalent on an existing record
user.update({ "name": "Alice Smith", "age": 31 });
```

### Deleting Records

```soli
User.delete("user123");
```

### Counting Records

```soli
total = User.count;
```

## Query Builder Chaining

Chain methods to build complex queries:

```soli
results = User
  .where("doc.age >= @age", { "age": 18 })
  .where("doc.active == @active", { "active": true })
  .order("created_at", "desc")
  .limit(10)
  .offset(20)
  .all;

# Get first result only
first = User.where("doc.email == @email", { "email": "alice@example.com" }).first;

# Count with conditions
count = User.where("doc.role == @role", { "role": "admin" }).count;
```

## Static Methods Reference

| Method | Description |
|--------|-------------|
| `Model.create(data)` | Insert a new document |
| `Model.create_many([data, ...])` | Batch insert multiple documents, returns `{ created, errors }` |
| `Model.find(id)` | Get document by ID. **Raises** `RecordNotFound` if missing (auto-mapped to a 404 HTTP response). Use `find_by` for optional lookups. |
| `Model.find_by(field, value)` | Find first record by field value. Returns `null` when missing. |
| `Model.first_by(field, value)` | Find first record by field with ordering |
| `Model.find_or_create_by(field, value, data?)` | Find by field, or create if not found |
| `Model.where(hash)` | Hash filter — safe for user input (keys validated, values bound). Returns QueryBuilder |
| `Model.where(string, bind_vars)` | SDBQL filter string — **developer-trusted only**, never feed `req[...]` into the string. Returns QueryBuilder |
| `Model.all` | Get all documents |
| `Model.update(id, data)` | Update a document |
| `Model.upsert(id, data)` | Insert or update document by ID |
| `Model.delete(id)` | Delete a document |
| `Model.delete_all` | Wipe every document in the collection (primarily for test setup/teardown). Use `Model.where(...).delete_all` for filtered bulk deletes. |
| `Model.count` | Count all documents |
| `Model.transaction do … end` | Run a block in a transaction — commit on success, roll back on throw |
| `Model.transaction("aql")` | Execute a single AQL statement transactionally |
| `Model.transaction()` | Get transaction handle for manual control |
| `Model.<scope_name>` | Invoke a named scope declared with `scope(name, fn)` (returns QueryBuilder) |
| `Model.with_deleted()` | Include soft-deleted records (QueryBuilder) |
| `Model.only_deleted()` | Query only deleted records (QueryBuilder) |
| `Model.includes(rel, ...)` | Eager load relations (returns QueryBuilder) |
| `Model.includes(rel, filter, binds)` | Eager load with filter condition (returns QueryBuilder) |
| `Model.includes({ rel: [fields] })` | Eager load with field selection (returns QueryBuilder) |
| `Model.includes_count(rel, ...)` | Eager load `<rel>_count` field per parent (HasMany/HABTM only) |
| `Model.select(field, ...)` | Select specific fields (returns QueryBuilder) |
| `Model.fields(field, ...)` | Alias for `select()` (returns QueryBuilder) |
| `Model.join(rel, filter?, binds?)` | Filter by related existence (returns QueryBuilder) |
| `Model.order(field, dir?)` | Order results (returns QueryBuilder) |
| `Model.limit(n)` | Limit results (returns QueryBuilder) |
| `Model.offset(n)` | Offset results (returns QueryBuilder) |
| `Model.paginate(hash)` | Terminal: fetch paginated results + metadata. See [Pagination](#pagination) below. |

## Relationship DSL

| Method | Description |
|--------|-------------|
| `has_many(name)` | Declare a one-to-many relationship |
| `has_one(name)` | Declare a one-to-one relationship |
| `belongs_to(name)` | Declare an inverse relationship |

## QueryBuilder Methods

| Method | Description |
|--------|-------------|
| `.where(filter, bind_vars)` | Add filter condition (ANDed with existing) |
| `.order(field, direction)` | Set sort order ("asc" or "desc") |
| `.limit(n)` | Limit results to n documents |
| `.offset(n)` | Skip first n documents |
| `.includes(rel, ...)` | Eager load relations via subqueries |
| `.includes(rel, filter, binds)` | Eager load with filter and optional `"fields"` key |
| `.includes({ rel: [fields] })` | Eager load with field projection |
| `.includes_count(rel, ...)` | Eager load count as `<rel>_count` (HasMany/HABTM only) |
| `.select(field, ...)` | Select specific fields on the main collection |
| `.fields(field, ...)` | Alias for `.select()` |
| `.join(rel, filter?, binds?)` | Filter by existence of related records |
| `.pluck(field, ...)` | Return only specified fields (single or array) |
| `.all` | Execute query, return all results |
| `.first` | Execute query, return first result |
| `.count` | Execute query, return count |
| `.exists` | Execute query, return boolean (true if records exist) |
| `.delete_all` | Execute as a bulk REMOVE — every matching row is deleted in a single statement. Hard delete (ignores soft-delete mode); order/limit/offset/select/group_by are ignored since they don't compose with REMOVE. Returns `null`. |
| `.update_all(hash)` | Execute as a bulk UPDATE — every matching row is patched with `hash` in a single statement. Skips validations and lifecycle callbacks; order/limit/offset/select/group_by are ignored since they don't compose with UPDATE. Returns `null`. |
| `.sum(field)` | Execute aggregation, return sum of field |
| `.avg(field)` | Execute aggregation, return average of field |
| `.min(field)` | Execute aggregation, return minimum of field |
| `.max(field)` | Execute aggregation, return maximum of field |
| `.group_by(field, func, agg_field)` | Execute grouping aggregation |
| `.paginate(hash)` | Terminal: fetch paginated results + metadata. See [Pagination](#pagination) below. |
| `.to_query` | Return the generated SDBQL string (for debugging) |

## Pagination

`Model.paginate(hash)` (static) and `.paginate(hash)` (chainable on a QueryBuilder) are **terminal** methods that execute the query with pagination and return a hash with both records and pagination metadata.

### Arguments

| Key | Default | Description |
|-----|---------|-------------|
| `page` | `1` | Page number (1-indexed, clamped to valid range) |
| `per` | `25` | Results per page |

### Return Value

```soli
{
  "records": [...],                // Array of model instances for this page
  "pagination": {
    "page":        1,              // Current page (clamped)
    "per":         25,             // Results per page
    "total":       100,            // Total matching records (unpaginated)
    "total_pages": 4               // Total number of pages
  }
}
```

### Usage

Chain from any QueryBuilder — all filters, includes, ordering, etc. are preserved:

```soli
let result = Contact
    .search(@q)
    .includes("organisation")
    .order("name", "asc")
    .paginate({ page: this._page_param(), per: 25 });

@contacts   = result["records"];
@pagination = result["pagination"];
```

The paginate method:
1. Runs `count` first to get the total matching records
2. Computes `total_pages = ceil(total / per)`
3. Clamps `page` to valid range (1..total_pages)
4. Sets `offset = (page - 1) * per` and `limit = per`
5. Fetches the records
6. Returns the result hash

If `total` is 0, `total_pages` is set to 1 and `page` is clamped to 1 (returning an empty records array with no error).

Direct static call also works:

```soli
let result = Contact.paginate({ page: 2, per: 10 });
```

## Mass Assignment Protection

By default, `Model.create(hash)` and `instance.update(hash)` write **every** key in the supplied hash straight to the document. If `hash` came from a request body, that includes any field a client decides to send — `role`, `is_admin`, `password_digest`, etc. Declare `attr_accessible(...)` on the model to lock down which keys mass-assign accepts.

```soli
class User < Model
  # Variadic form
  attr_accessible("name", "email", "bio")

  # …or pass a single array — equivalent
  # attr_accessible(["name", "email", "bio"])
end

User.create({
  "name":  "Alice",
  "email": "alice@example.com",
  "role":  "admin"   # silently dropped — not in the whitelist
});
```

Filtering applies to every mass-assign path: `Model.create(hash)`, `Model.update(id, hash)`, `instance.update(hash)`, `instance.save(hash)`. Non-permitted keys are dropped before validation runs and before the document is written, so they cannot be probed via validation errors either.

**Empty list = full lock-down.** `attr_accessible([])` declares that the model accepts no mass-assigned attributes; everything must be set by trusted server code via direct field assignment (`user.role = "admin"`).

**Models without a declaration keep the legacy "all keys accepted" behaviour** for backwards compatibility. New models that take request data should always declare `attr_accessible`. The CLAUDE.md security guidance recommends auditing every `Model.create`/`Model.update` call site against an explicit whitelist.

For controller-side filtering (when you'd rather hand-pick keys at the boundary), the existing `hash.slice(["a", "b"])` returns a new hash with only the listed keys — handy when you need different whitelists per action:

```soli
fn update
  let user = User.find(req["params"]["id"]);
  let safe = req["json"].slice(["name", "bio"]);
  user.update(safe);
  return redirect("/users/" + user._key);
end
```

## Validations

Define validation rules in your model class:

```soli
class User < Model
  validates("email", { "presence": true, "uniqueness": true })
  validates("name", { "presence": true, "min_length": 2, "max_length": 100 })
  validates("age", { "numericality": true, "min": 0, "max": 150 })
  validates("website", { "format": "^https?://" })
end
```

### Validation Options

| Option | Description |
|--------|-------------|
| `presence: true` | Field must be present and not empty |
| `uniqueness: true` | Field value must be unique in collection |
| `min_length: n` | String must be at least n characters |
| `max_length: n` | String must be at most n characters |
| `format: "regex"` | String must match regex pattern |
| `numericality: true` | Value must be a number |
| `min: n` | Number must be >= n |
| `max: n` | Number must be <= n |
| `custom: "method_name"` | Call custom validation method |

### Validation Results

`Model.create()` always returns an instance of the class. On validation or
database failure, the instance is **not persisted** and its `_errors` field
holds an array of error entries. On success, `_errors` is `nil`.

```soli
user = User.create({ "email": "" });

if user._errors
  for error in user._errors
    print(error["field"] + ": " + error["message"]);
  end
else
  print("Created user: " + user.id);
end
```

### Atomic uniqueness

`uniqueness: true` issues a `SELECT … LIMIT 1` before the write, but that
check is **best-effort**: two concurrent `User.create({ "email": "x" })`
calls can both pass the SELECT and both insert. To make uniqueness atomic,
declare a unique index on the column at deploy time and let the database
enforce it. Soli detects the resulting 409 from `Model.create`,
`instance.save`, `instance.update`, `Model.upsert`, and
`Model.find_or_create_by`, and turns it into the same `_errors` entry the
SELECT path produces (`field: "has already been taken"`), so callers
handle the race identically.

```soli
# Run once at deploy time (e.g. in a migration):
solidb.create_index("users", "users_email_unique", ["email"], { "unique": true });
```

Without the index, the SELECT is the only line of defense and the race is
silently lost.

## Callbacks

Define lifecycle callbacks to run code at specific points. The method name
can be passed as a string or a symbol:

```soli
class User < Model
  before_save("normalize_email")   # both strings and symbols work
  before_save(:normalize_email)    # Ruby-style symbol shorthand
  after_create("send_welcome_email")
  before_update("log_changes")
  after_delete("cleanup_related")

  fn normalize_email()        this.email = this.email.downcase;
  end

  fn send_welcome_email()        # Send email logic
  end
end
```

### Available Callbacks

| Callback | When it runs |
|----------|--------------|
| `before_save` | Before create or update |
| `after_save` | After create or update |
| `before_create` | Before inserting new record |
| `after_create` | After inserting new record |
| `before_update` | Before updating existing record |
| `after_update` | After updating existing record |
| `before_delete` | Before deleting record |
| `after_delete` | After deleting record |

### Firing order per persistence method

Both class-level methods (`Model.create`, `Model.update`) and instance-level mutators run the matching callbacks. Rails-style: the `_save` callbacks fire on every persistence path, plus the more specific event for the operation.

| Method | Before-callbacks (in order) | DB write | After-callbacks (in order) |
|--------|------------------------------|----------|-----------------------------|
| `Model.create(attrs)` | `before_save` → `before_create` | INSERT | `after_create` → `after_save` |
| `Model.update(id, attrs)` | `before_save` → `before_update` | UPDATE | `after_update` → `after_save` |
| `instance.save([attrs])` — new record (no `_key`) | `before_save` → `before_create` | INSERT | `after_create` → `after_save` |
| `instance.save([attrs])` — persisted record | `before_save` → `before_update` | UPDATE | `after_update` → `after_save` |
| `instance.update(attrs)` | `before_save` → `before_update` | UPDATE | `after_update` → `after_save` |
| `instance.restore()` | `before_save` → `before_update` | UPDATE | `after_update` → `after_save` |
| `instance.increment(field, n?)` | `before_save` → `before_update` | UPDATE | `after_update` → `after_save` |
| `instance.decrement(field, n?)` | `before_save` → `before_update` | UPDATE | `after_update` → `after_save` |
| `instance.touch()` | `before_save` → `before_update` | UPDATE | `after_update` → `after_save` |
| `instance.delete()` (soft + hard) | `before_delete` | UPDATE / DELETE | `after_delete` |

After-callbacks only fire when the persist call succeeds. If the native method returns `false` (validation or DB error) the after-callbacks are skipped and the instance carries `_errors`.

### Vetoing persistence from a `before_*` callback

Returning `false` from any `before_*` callback aborts the operation. The native DB write is skipped, after-callbacks don't run, and the instance picks up an `_errors` entry of the form `[{"message": "before_<event> callback returned false; persistence aborted"}]`. Callers receive `false` (or, for `Model.create` / `Model.update`, the instance with `_errors` populated) so they can branch on the result identically to a validation failure.

```soli
class Audited < Model
  before_save("can_save")  # symbols also work: before_save(:can_save)

  def can_save
    return false if User.current.is_nil?  # returns false → save() / update() aborts
  end
end
```

The veto fires at the first `false` — subsequent before-callbacks in the chain don't run. Use this for authorization gates, integrity checks, or any "deny by default" pattern. Mutating-only callbacks (the common case) just don't return `false` and run end-to-end as before.

## Uploaders

Declare a blob attachment with `uploader(name, options)`. Soli registers the field, validates incoming files against the rules, stores the blob in SoliDB, and auto-generates instance methods so the controller stays a one-liner.

**Single attachment:**

```soli
class Contact < Model
  uploader("photo", {
    "multiple":      false,
    "content_types": ["image/jpeg", "image/png", "image/webp"],
    "max_size":      2_000_000,
    "collection":    "contact_photos"   # optional; defaults to <snake>_<field>s
  })
end
```

Auto-generated on each instance: `attach_<field>(file)`, `detach_<field>([blob_id])`, `<field>_url()` (single), `<field>_urls()` (multiple). Failures populate `_errors` on the record.

**Multiple attachments** (array of blob ids in `<name>_blob_ids`):

```soli
class Contact < Model
  uploader("document", {
    "multiple":      true,
    "content_types": ["application/pdf", "image/jpeg", "image/png",
             "application/zip", "text/csv"],
    "max_size":      10_000_000,
    "collection":    "contact_documents"
  })
end
```

```soli
# POST /contacts/:id/documents (HTML form → redirect+flash)
def attach_document
  contact = Contact.find(params.id)
  file = find_uploaded_file(req, "document")
  if file.nil?
    flash("error", "Pick a file before submitting.")
  elsif contact.attach_document(file)
    flash("success", "Document filed.")
  else
    flash("error", (contact._errors[0] ?? { "message": "Upload failed." })["message"])
  end
  redirect("/contacts/#{contact._key}")
end

# POST /contacts/:id/document/:blob_id/delete
def detach_document
  contact = Contact.find(params.id)
  if contact.detach_document(params.blob_id)
    flash("success", "Document removed.")
  else
    flash("error", "Document not found on this record.")
  end
  redirect("/contacts/#{contact._key}")
end
```

### Uploader options

| Option | Default | Description |
|--------|---------|-------------|
| `multiple` | `false` | `true` keeps an array of blob ids (`<field>_blob_ids`); `false` keeps one (`<field>_blob_id`). |
| `content_types` | — (required) | Allow-list of MIME types. Anything else is rejected before storage. |
| `max_size` | — (required) | Hard cap in bytes. Above this → `_errors` populated, no blob stored. |
| `collection` | `<snake>_<field>s` | SoliDB collection that holds the blobs. |
| `format` | — | Convert image uploads to `"jpeg"`, `"png"`, or `"webp"` **before storage**. Non-image uploads (PDF, csv, …) are never converted. |
| `quality` | `82` | Encoder quality (1–100) for lossy formats (`jpeg`, `webp`). |
| `max_width` / `max_height` | — | Downscale the original to fit within these pixel bounds before storage, preserving aspect ratio. Never upscales. |

#### Transform the original before storage

To avoid storing large originals (e.g. heavy PNG photos), declare a `format`
and/or size caps. The framework decodes the upload, optionally downscales it,
re-encodes it to the target format, and stores the result — updating the blob's
content-type and filename extension to match. WebP is encoded **lossy** (via
libwebp) so photos shrink dramatically; JPEG quality is honoured too.

```soli
class Listing < Model
  uploader("photo", {
    "content_types": ["image/jpeg", "image/png", "image/webp"],
    "max_size":      10_000_000,   # accept up to 10 MB on the way in
    "format":        "webp",       # …but store a lossy WebP
    "quality":       80,
    "max_width":     1600,         # downscale huge originals
    "max_height":    1600
  })
end
```

A 4 MB PNG uploaded here lands in storage as a downscaled ~200 KB WebP. The
transform only runs for image content-types — a PDF in a multi-file uploader is
stored byte-for-byte. If the bytes can't be decoded as an image, the original is
stored unchanged so an upload is never blocked by a transform failure.

> The same lossy WebP/quality encoding powers the read-time URL transform
> pipeline (`photo_url(...)` with `?fmt=webp&w=...`), so you can also keep a
> larger original and convert per-request instead.

For drag-and-drop / AJAX flows that prefer JSON 204/422 over redirects, use `uploads("contacts", "document")` in `config/routes.sl` instead — that auto-mounts a generic `AttachmentsController` for upload, download, and per-blob delete.

**Cleanup on destroy** — `before_delete` callbacks aren't yet dispatched by `Model.delete(id)`; call `detach_all_uploads(record)` explicitly until that lands. The helper walks every `uploader(...)` field on the class.

```soli
def destroy
  contact = Contact.find(params.id)
  detach_all_uploads(contact) unless contact.nil?
  Contact.delete(params.id)
  redirect("/contacts")
end
```

**Introspection** — `model_uploader_config(class_or_name, field)` returns `{ name, multiple, content_types, max_size, collection }` (or `null`); `model_uploader_fields(class_or_name)` lists the declared field names.

## Relationships

Declare associations using the built-in DSL. Association names accept strings
or symbols:

```soli
class User < Model
  has_many("posts")      # strings and symbols both work
  has_many(:posts)       # Ruby-style symbol shorthand
  has_one("profile")
end

class Post < Model
  belongs_to("user")
  has_many("comments")
end
```

### Naming Conventions

The DSL applies Rails-style naming conventions automatically:

| Declaration | Related Class | Collection | Foreign Key |
|-------------|--------------|------------|-------------|
| `has_many("posts")` | `Post` | `posts` | `user_id` (owner + `_id`) |
| `has_one("profile")` | `Profile` | `profiles` | `user_id` (owner + `_id`) |
| `belongs_to("user")` | `User` | `users` | `user_id` (name + `_id`) |
| `has_and_belongs_to_many("tags")` | `Tag` | `tags` | join table `posts_tags`, FKs `post_id` / `tag_id` |

Override defaults with an options hash:

```soli
class Post < Model
  belongs_to("author", { "class_name": "User", "foreign_key": "author_id" })

  has_and_belongs_to_many("labels", {
    "class_name": "Tag",
    "join_table": "post_labels",
    "foreign_key": "post_id",
    "association_foreign_key": "label_id"
  })
end
```

### Eager Loading (includes)

Preload related records to avoid N+1 queries. Uses LET subqueries with MERGE:

```soli
# Load users with their posts and profiles in a single query
users = User.includes("posts", "profile").all

# Combine with where clauses
active = User.where("active = @a", { "a": true }).includes("posts").first

# Inspect the generated query
print(User.includes("posts").to_query)
# => FOR doc IN users LET _rel_posts = (FOR rel IN posts FILTER rel.user_id == doc._key RETURN rel) RETURN MERGE(doc, {posts: _rel_posts})
```

- `has_many` includes return an array of related documents
- `has_one` and `belongs_to` includes return a single document (via `FIRST()`)

After `.all`, the preloaded data is cached on each instance: subsequent `instance.<rel>` reads return the cached value without issuing another query. This applies to `has_and_belongs_to_many`, `belongs_to`, `has_one`, and polymorphic relations. (`has_many` accessors still return a chainable `QueryBuilder`, so they are not served from the preload cache — use `.where(...).all` if you want a materialised array.)

### Join Filtering

Filter records by the existence of related records. Unlike `includes`, `join` does **not** preload the related data — it only filters:

```soli
# Find users who have at least one post
users_with_posts = User.join("posts").all

# Find users who have published posts
count = User.join("posts", "published = @p", { "p": true }).count

# Chain with other query methods
recent = User.join("posts").order("created_at", "desc").limit(10).all
```

This is equivalent to ActiveRecord's `joins` — use `includes` when you need the related data, and `join` when you only need to filter by existence.

### Filtered Includes

Filter included relations to load only matching related records:

```soli
# Only load published posts for each user
users = User.includes("posts", "published = @p", { "p": true }).all

# Inspect the generated query
print(User.includes("posts", "published = @p", { "p": true }).to_query)
# => ... LET _rel_posts = (FOR rel IN posts FILTER rel.user_id == doc._key AND rel.published == @p RETURN rel) ...
```

Combine a filter with field projection using the `"fields"` key in the bind hash:

```soli
# Only load title and body of published posts
users = User.includes("posts", "published = @p", {
  "p": true,
  "fields": ["title", "body"]
}).all
# => ... RETURN {title: rel.title, body: rel.body} ...
```

### Includes with Field Projection

Use a hash argument to select specific fields on included relations (without filtering):

```soli
# Only load title and body from posts
users = User.includes({ "posts": ["title", "body"] }).all
# => ... LET _rel_posts = (FOR rel IN posts FILTER rel.user_id == doc._key RETURN {title: rel.title, body: rel.body}) ...
```

### Chaining Multiple Includes

Chain `.includes()` calls to eagerly load multiple relations with different options:

```soli
# Filtered posts + unfiltered profile
users = User.includes("posts", "published = @p", { "p": true })
  .includes("profile")
  .all
```

### Counting Relations (includes_count)

When you only need the *count* of a relation (not the rows), `.includes_count()` adds a single `LET _rel_<name>_count = LENGTH(...)` subquery to the parent and exposes the result as a `<name>_count` field on each instance. Cheaper than `.includes()` when you only render counts:

```soli
# Each Category gets a `products_count` integer field, in one round-trip
cats = Category.includes_count("products").all
print(cats[0].products_count)
# => 3

# Combine with .includes() and other chain steps
q = Author.where("active = @a", { "a": true })
  .includes("profile")
  .includes_count("posts")
  .order("name", "asc")
  .all
```

- Only valid for `has_many` and `has_and_belongs_to_many` relations. Calling it on `belongs_to`, `has_one`, or polymorphic relations raises an error at registration time (the count is always 0 or 1, so the API doesn't earn its keep there).
- The exposed field is always `<relation_name>_count`. Reads are O(1) — it's just an integer field on the instance, no extra query.

### Field Selection (select / fields)

Use `.select()` (or its alias `.fields()`) to return only specific fields from the main collection. `_key` is always included automatically for identity:

```soli
# Only return name and email
users = User.select("name", "email").all
# => FOR doc IN users RETURN {name: doc.name, email: doc.email, _key: doc._key}

# .fields() is an alias
users = User.fields("name", "email").all
# => same query

# Combine with other query methods
users = User.where("active = @a", { "a": true })
  .select("name", "email")
  .order("name")
  .limit(10)
  .all

# Combine with includes
users = User.select("name", "email").includes("posts").all
# => ... RETURN MERGE({name: doc.name, email: doc.email, _key: doc._key}, {posts: _rel_posts})

# Full combo: select + filtered includes with field projection
users = User.select("name")
  .includes("posts", "published = @p", { "p": true, "fields": ["title"] })
  .all
```

### Has And Belongs To Many

Many-to-many associations use a join table that stores `(<foreign_key>, <association_foreign_key>)` rows:

```soli
class Post < Model
  has_and_belongs_to_many("tags")
end

class Tag < Model
  has_and_belongs_to_many("posts")
end
```

Both classes declare the relation. The default join table is the alphabetical concatenation of the two pluralized class names — here `posts_tags`. The default foreign keys are `post_id` and `tag_id`.

**Read** — `post.tags` returns an array of related `Tag` instances:

```soli
post = Post.find(post_id)
tags = post.tags  # => [Tag, Tag, ...]
```

**Add / remove** — auto-generated mutators insert/delete join-table rows. The method name is `add_<singular>` / `remove_<singular>` derived from the relation name:

```soli
post.add_tag(tag)              # accepts a Tag instance
post.add_tag("tag_key")        # …or a raw _key
post.add_tag([tag1, tag2])     # …or an array
post.add_tag(tag1, tag2)       # …or variadic args

post.remove_tag(tag)
post.remove_tag([tag1, tag2])
```

**Shovel operator (`<<`)** — equivalent to `add_<singular>`, returns the (refreshed) relation:

```soli
post.tags << tag                # appends one tag, returns post.tags
```

**Eager loading** uses a two-stage subquery through the join table:

```soli
posts = Post.includes("tags").all
# => ... LET _rel_tags = (FOR jt IN posts_tags FILTER jt.post_id == doc._key
#                          FOR rel IN tags FILTER rel._key == jt.tag_id RETURN rel) ...
```

**Existence filtering** with `.join()`:

```soli
tagged_posts = Post.join("tags").all              # posts that have at least one tag
tutorials = Post.join("tags", "name = @n", { "n": "tutorial" }).all
```

**Overrides** — supply an options hash to customize the join:

```soli
class Article < Model
  has_and_belongs_to_many("labels", {
    "class_name": "Tag",
    "join_table": "article_labels",
    "foreign_key": "article_id",
    "association_foreign_key": "tag_id"
  })
end
```

### Manual Relationships

You can also implement relationships as custom methods for more control:

```soli
class Post < Model
  fn author()
    User.find(this.author_id)
  end
end
```

## Finder Methods

Find records by specific field values:

```soli
# Find by exact field match
user = User.find_by("email", "alice@example.com");

# Find with ordering (first by field value)
user = User.first_by("name", "Alice");

# Find or create - returns existing or creates new
user = User.find_or_create_by("email", "new@example.com");
user = User.find_or_create_by("email", "new@example.com", { "name": "New User" });
```

### Dynamic Finder Methods

Automatically generated finders for any field combination:

```soli
# Single field finder
user = User.find_by_email("alice@example.com");

# Two-field finder (AND logic)
user = User.find_by_email_and_active("alice@example.com", true);

# Three+ field combinations
post = Post.find_by_title_and_published_and_author_id("Hello", true, 123);
```

These methods return the first matching record or `null` if not found.

## Aggregations

Calculate sums, averages, min, max on query results:

```soli
# Sum
total = User.where("age > @a", { "a": 18 }).sum("balance");

# Average
avg = User.avg("score");

# Minimum
min_score = User.min("score");

# Maximum
max_score = User.max("views");

# Group by aggregation
by_country = User.group_by("country", "sum", "balance");
# Returns: [{ group: "US", result: 1000 }, { group: "FR", result: 500 }, ...]
```

## Pluck and Exists

Quick queries for specific data:

```soli
# Get array of single field values
names = User.where("active = @a", { "a": true }).pluck("name");
# Returns: ["Alice", "Bob", "Charlie"]

# Get multiple fields as objects
users = User.pluck("name", "email");
# Returns: [{ name: "Alice", email: "alice@example.com" }, ...]

# Check if records exist (returns boolean)
exists = User.where("role = @r", { "r": "admin" }).exists;
# Returns: true or false
```

## Vector / Similarity Search

Rank results by semantic relevance using `.similar()`:

```soli
# Basic semantic search (uses default "embedding" field, top 10)
results = Post
    .where("published == true")
    .similar("how to deploy a web app")
    .all

# Custom embedding field and result count
results = Product
    .where("active == true")
    .similar("red running shoes", "title_embedding", 5)
    .all

# Each result gets a _similarity_score
for product in results
    print(product.name + " (" + str(product._similarity_score) + ")")
end
```

Requires `SOLI_EMBEDDING_API_KEY` environment variable. See [database docs](/docs/database/finders#similar-simple) for configuration.

SolidDB also supports native vector search with HNSW indexes, `VECTOR_SIMILARITY()` in SDBQL, hybrid search, and scalar quantization. Create a vector index on your embedding field for production workloads. See the [SolidDB Vector Search docs](https://solidb.solisoft.net/docs/vector-search) for details.

## Instance Methods

Methods available on model instances:

```soli
user = User.find("user_id");

# Update fields and persist
user.name = "New Name";
user.update();

# Atomic increment/decrement (CAS via `_rev` with bounded retry)
user.increment("view_count");      # +1
user.increment("view_count", 5);   # +5
user.decrement("stock");           # -1

# Update timestamp only
user.touch();  # Updates _updated_at

# Refresh from database
user.reload();
```

### How `increment` / `decrement` stay atomic under concurrency

`increment` and `decrement` are **not** plain read-modify-writes on the in-memory
instance — each call drives an optimistic compare-and-swap loop against SoliDB:

1. Re-fetch the document to read the current field value and its `_rev`.
2. Compute `current + delta` (or `current - delta`).
3. PUT the new value with an `If-Match: <rev>` header.
4. If another writer modified the document in between, the DB returns
   `409 Conflict` and the loop retries (up to 10 attempts) by re-fetching.

On success the in-memory instance's field **and** `_rev` are refreshed, so any
follow-up call observes the same state the DB now holds. Concurrent
increments cannot lose updates: every successful PUT was the unique
continuation of the rev it read.

If the document is being hit by many writers at once, `increment` may return
an error like `"increment failed: Atomic update of users.view_count failed
after 10 attempts (too much contention)"` instead of silently dropping the
update. Callers can retry, queue the work, or back off as they prefer.

### Bulk attribute updates: `.save(hash)` and `.update(hash)`

Both `.save()` and `.update()` accept an optional hash of attributes that are
applied to the instance before the persist pipeline runs. This collapses the
common "set multiple fields, then save" pattern into a single call:

```soli
# Instead of:
user.name = "Alice";
user.email = "alice@example.com";
user.role = "admin";
user.save();

# Write:
user.save({
  "name": "Alice",
  "email": "alice@example.com",
  "role": "admin"
});
```

The hash is merged onto the instance — keys you don't pass keep their current
value, keys you do pass overwrite. Validations run *after* the merge, so
errors surface on `.errors` the same way as individual field assignments:

```soli
# Partial update — only `price` changes, `name` is preserved
p = Product.find(id);
p.update({ "price": 99.00 });

# Mix field assignment with hash — pre-assigned fields fall back when hash
# omits them, hash wins on conflict.
p = Product.new();
p.name = "Widget";            # will survive
p.save({ "price": 12.50 });   # name stays "Widget", price becomes 12.50
```

Framework-internal fields (`_key`, `_id`, `_rev`, `_errors`, etc.) are
silently skipped when they appear in the hash — you can't overwrite them via
bulk update. A non-hash argument raises:
`expected a Hash of attributes, got <type>`.

`.update(hash)` is effectively sugar for `.save(hash)` on an existing record
(requires `_key` to be set); the two share the exact same validation and DB
write path.

## Scopes

Define reusable query scopes. Inside the closure `this` is bound to a fresh `QueryBuilder` for the model; the closure returns a (possibly refined) `QueryBuilder`. Accessing the scope name on the class invokes the closure:

```soli
class User < Model
  scope("active", fn() { this.where("active = @a", { "a": true }) })
  scope("recent", fn() { this.order("created_at", "desc").limit(10) })
end

# Use scopes — `User.active` invokes the closure and returns a QueryBuilder
# you can chain further:
let active_users = User.active.all
let top_ten     = User.recent.where("verified = @v", { "v": true }).all
```

Scopes compose: `User.active.recent` chains both closures' refinements. See [Metaprogramming](metaprogramming.md#named-scopes) for the underlying mechanism.

## Soft Delete

Mark records as deleted without removing them:

```soli
class Post < Model
  soft_delete
end

# Delete sets deleted_at timestamp
post.delete();

# Restore clears deleted_at
post.restore();

# Query without deleted records (default behavior)
posts = Post.all;

# Include soft-deleted records
all = Post.with_deleted.all;

# Query only deleted records
deleted = Post.only_deleted.all;
```

## Encrypted Attributes

Encrypt sensitive fields at rest with `encrypts`. Values are encrypted on
create/save/update and decrypted transparently on load, using AES-256-GCM:

```soli
class User < Model
  encrypts(:ssn, :api_token)
end

u = User.create({ "ssn": "123-45-6789", "email": "a@b.com" });
# stored ciphertext in the DB; in memory it's plaintext:
User.find(u._key).ssn  # => "123-45-6789"
```

The encryption key comes from the `SOLI_ENCRYPTION_KEY` environment variable —
set it to a long, high-entropy secret (e.g. `Crypto.random_hex(32)`) and keep
it out of source control.

> **Encrypted columns can't be queried by value.** AES-GCM uses a random nonce,
> so the same plaintext encrypts to different ciphertext every time —
> `User.where("ssn = @s", { "s": "123-45-6789" })` will never match. Encrypt
> only fields you store and read, not ones you filter on.

A field written before `encrypts` was added (legacy plaintext) is returned
as-is on load rather than erroring. Low-level/transaction writes that bypass
`create`/`save` aren't auto-encrypted — use `Crypto.encrypt` directly there.

`encrypts` builds on the standalone `Crypto.encrypt(plaintext, key?)` /
`Crypto.decrypt(ciphertext, key?)` builtins, which you can also use on their
own (key defaults to `SOLI_ENCRYPTION_KEY`).

## Relationship Accessors

Access related records directly from instances:

```soli
user = User.find("user_id");

# has_many returns a chainable QueryBuilder, not an array.
# Each terminal call (.all, .count, .delete_all, iteration, ...)
# runs a query against the foreign-key filter at that moment.
posts = user.posts;

# Access has_one relation
profile = user.profile;

# Access belongs_to relation
author = post.user;
```

### has_many is Enumerable AND chainable

The `has_many` accessor behaves like an array (iteration, indexing, `len`,
`each`, `map`, `filter`, ...) **and** like a QueryBuilder
(`.where`, `.order`, `.limit`, `.count`, `.delete_all`, `.exists`, ...):

```soli
user = User.find("user_id");

# Iterate — each iteration runs the FK-filtered query once
for post in user.posts
  print(post.title);
end

# Indexing materializes the result set
first = user.posts[0];

# len() and .length / .size return the count
n = len(user.posts);
same = user.posts.length;
alt = user.posts.size;

# Array-style helpers materialize then delegate
user.posts.each(fn(p) { print(p.title) });
titles = user.posts.map(fn(p) { p.title });

# Chained query — composes onto the seed `user_id == @__rel_fk` filter
published = user.posts.where("published = @p", { "p": true }).all;
n_pub = user.posts.where("published = @p", { "p": true }).count;

# Bulk delete — one REMOVE statement, no N+1
user.posts.delete_all;
user.posts.where("draft = @d", { "d": true }).delete_all;

# Bulk update — one UPDATE statement, no N+1
user.posts.where("draft = @d", { "d": true }).update_all({ "draft": false });

# Sort / paginate before materializing
recent = user.posts.order("created_at", "desc").limit(10).all;
```

Notes:

- An owner that has not been saved yet (no `_key`) returns a QueryBuilder
  whose filter never matches — `count` is `0`, `delete_all` / `update_all`
  are no-ops, and iteration yields nothing.
- If the related model uses `soft_delete`, soft-deleted children are filtered
  out of the relation (consistent with `Related.where(...)`). Use the static
  `Related.with_deleted` / `Related.only_deleted` to query them explicitly.
- `belongs_to` and `has_one` still return a single instance (or `nil`),
  not a QueryBuilder.

## Batch Operations

Insert or update multiple records:

```soli
# Batch create
result = User.create_many([
  { "name": "Alice", "email": "alice@example.com" },
  { "name": "Bob", "email": "bob@example.com" },
  { "name": "Charlie", "email": "charlie@example.com" }
]);
# Returns: { "created": 3, "errors": [] }

# Upsert (insert or update by ID)
User.upsert("user123", { "name": "Updated Name" });
# Updates if exists, inserts with ID if not

# Batch delete via QueryBuilder — one AQL REMOVE for the whole match.
# Useful for clearing a relation without an N+1 loop:
User.where("doc.active == false").delete_all;
post.comments.delete_all;          # via has_many relation
Model.delete_all;                  # static — wipe the whole collection

# Batch update via QueryBuilder — one AQL UPDATE for the whole match.
# Patches every matching row; skips validations and callbacks:
User.where("doc.active == false").update_all({ "archived": true });
post.comments.where("draft = @d", { "d": true }).update_all({ "draft": false });
```

## Coalescing Reads (`grouped`)

A controller action that reads several unrelated things pays one network
round-trip per read:

```soli
@posts    = Post.all                          # round-trip 1
@accounts = Account.where({ active: true }).count   # round-trip 2
@tags     = Tag.all                           # round-trip 3
```

Wrap the reads in `grouped(fn() { ... })` and they are deferred and combined
into a **single** request — one `LET … RETURN […]` statement that computes every
subquery server-side and returns them together:

```soli
grouped(fn() {
  @posts    = Post.all
  @accounts = Account.where({ active: true }).count
  @tags     = Tag.all
})
# one round-trip for all three
```

Inside the block each read returns a placeholder instead of hitting the
database; the queries fire as one combined statement when the block ends. The
results are then materialised, so **after** the block `@posts`, `@accounts`, and
`@tags` are ordinary values you use exactly as before.

### What gets coalesced

Read queries are batched: `all`, `where(...).all` / `.first` / `.count` /
`.exists`, the aggregates (`sum` / `avg` / `min` / `max`), `find`, `find_by`, and
`first_by`. **Writes are not** — `create`, `save`, `update`, `delete` run
immediately even inside the block (use `transaction` for atomic writes).

### Reading a result inside the block (auto-flush)

If you read one of the deferred results *before* the block ends, the queries
collected so far fire immediately (an "auto-flush"), then collection resumes for
the rest:

```soli
grouped(fn() {
  @posts = Post.all
  log("loaded #{@posts.length} posts") if @posts.present?  # forces a flush here
  @tags = Tag.all                                          # batched separately
})
```

This always returns correct data; it just means you get more than one round-trip
when you interleave reads. For maximum coalescing, do the reads first and use the
results after the block.

### Notes

- `find` on a missing id still raises `RecordNotFound` (→ 404) — the error
  surfaces when the result is read or the block ends.
- A combined query is all-or-nothing: if it fails, every read in the batch
  fails together.
- In **`--dev`** the reads are *not* coalesced — each runs as its own query so
  the dev query log stays readable (you see the natural statements instead of
  one combined `LET … RETURN […]`). Coalescing is active in production, where
  the single round-trip is what matters. To confirm the production shape, check
  the combined query in a non-dev run.

## Transactions

Execute multiple operations atomically within a database transaction:

### Using a Block (Recommended)

`Model.transaction` runs a block inside a single database transaction. Every
document write inside the block — `create`, `save`, `update`, `delete`, and key
lookups via `find` — participates automatically. The block **commits** when it
returns normally and **rolls back** (re-raising the error) if it throws:

```soli
User.transaction do
  User.create({ "name": "Alice", "age": 30 })
  User.create({ "name": "Bob", "age": 25 })
end
# Both rows commit together. If either operation — or your own `throw` —
# raises, neither row is persisted.
```

The block's value is returned, so you can hand back what you computed:

```soli
order = Order.transaction do
  account = Account.find(account_id)   # `find` (key lookup) sees in-transaction state
  account.balance -= amount
  account.save()
  Order.create({ "account_id": account_id, "total": amount })["record"]
end
```

Nested `transaction` calls **join** the outer transaction — only the outermost
begins and commits/rolls back (SolidB has no savepoints), so a `throw` anywhere
inside undoes every write in the whole nest.

> **Cursor reads see committed state.** Queries inside the block
> (`.where(...).all()`, `find_by`, aggregations) read *committed* data — they do
> not observe the transaction's own uncommitted writes. To read a row you wrote
> earlier in the same transaction, use `find` (a key lookup).

### Using AQL String

```soli
# Execute a single AQL statement transactionally (auto-commits).
# The string runs through the cursor endpoint with bind variables, so the
# query parameter is never interpolated into server-side JavaScript.
result = User.transaction("
  FOR u IN users FILTER u.active == true UPDATE u WITH { last_seen: DATE_NOW() } IN users
")
```

### Using Transaction Object (Manual Control)

```soli
# Get transaction handle for manual control
tx = User.transaction();
tx.create({ name: "Alice" });
tx.create({ name: "Bob" });
tx.commit();
# Or tx.rollback() to undo all changes
```

All operations within the transaction either all succeed or all fail together.

class User < Model
    fn posts()
        Post.where("doc.author_id == @id", { "id": this.id })
    end
end
```

## Custom Methods

Add custom methods to your models:

```soli
class User < Model
  fn is_admin() -> Bool
    this.role == "admin"
  end

  fn full_name() -> String
    this.first_name + " " + this.last_name
  end
end

# Usage
user = User.find("user123");
if user.is_admin()
  print("Welcome, admin " + user.full_name());
end
```

## Query Generation (SDBQL)

Under the hood, Model methods generate SDBQL (SoliDB Query Language) queries:

| Method | Generated SDBQL |
|--------|-----------------|
| `User.all` | `FOR doc IN users RETURN doc` |
| `User.where("age >= @age", {"age": 18})` | `FOR doc IN users FILTER doc.age >= @age RETURN doc` |
| `.order("name", "asc")` | `... SORT doc.name ASC RETURN doc` |
| `.limit(10).offset(20)` | `... LIMIT 20, 10 RETURN doc` |
| `User.count` | `RETURN COLLECTION_COUNT("users")` |
| `User.includes("posts")` | `FOR doc IN users LET _rel_posts = (FOR rel IN posts FILTER rel.user_id == doc._key RETURN rel) RETURN MERGE(doc, {posts: _rel_posts})` |
| `User.includes("posts", "published = @p", {"p": true})` | `... FILTER rel.user_id == doc._key AND rel.published == @p RETURN rel ...` |
| `User.includes({"posts": ["title"]})` | `... RETURN {title: rel.title} ...` |
| `User.select("name", "email")` | `FOR doc IN users RETURN {name: doc.name, email: doc.email, _key: doc._key}` |
| `User.join("posts")` | `FOR doc IN users FILTER LENGTH(FOR rel IN posts FILTER rel.user_id == doc._key LIMIT 1 RETURN 1) > 0 RETURN doc` |

SDBQL uses:
- `FOR doc IN collection` instead of `SELECT * FROM`
- `FILTER expression` instead of `WHERE`
- `SORT doc.field ASC/DESC` instead of `ORDER BY`
- `@variable` syntax for bind parameters
- `LET` subqueries + `MERGE` for eager loading

## Complete Example

```soli
# app/models/user.sl
class User < Model
  has_many("posts")      # strings and symbols both work
  has_many(:posts)       # Ruby-style symbol shorthand
  has_one("profile")

  validates("email", { "presence": true, "uniqueness": true })
  validates("name", { "presence": true, "min_length": 2 })

  before_save("normalize_email")   # strings and symbols both work
  before_save(:normalize_email)    # Ruby-style symbol shorthand

  fn normalize_email()
    this.email = this.email.downcase;
  end

  fn is_adult() -> Bool
    this.age >= 18
  end
end

# app/models/post.sl
class Post < Model
  belongs_to("user")     # symbols also work
  has_many("comments")

  validates("title", { "presence": true, "min_length": 3 })
end

# app/models/profile.sl
class Profile < Model
  belongs_to("user")
end

# Usage in controller
class UsersController < Controller
  fn index
    # Eager load posts and profiles to avoid N+1 queries
    users = User.includes("posts", "profile").all;
    render("users/index", { "users": users })
  end

  fn show
    id = req["params"]["id"];
    user = User.includes("posts").find(id);
    render("users/show", { "user": user })
  end

  fn active
    # Find active users who have at least one post
    users = User.join("posts")
      .where("active = @a", { "a": true })
      .order("created_at", "desc")
      .limit(10)
      .all;
    render("users/active", { "users": users })
  end

  fn create
    user = User.create({
      "name": req["params"]["name"],
      "email": req["params"]["email"],
      "age": req["params"]["age"]
    });

    if user._errors
      render("users/new", { "errors": user._errors })
    else
      redirect("/users/" + user.id)
    end
  end
end
```

## Testing Models

See the [Testing Guide](/docs/testing) for comprehensive information on testing models.

### Mock Database Queries

For integration tests without a real database, use `Model.mock_query_result()`:

```soli
describe("User queries", fn()
  before_each(fn()
    User.clear_mocks()
  end)
  
  after_each(fn()
    User.clear_mocks()
  end)
  
  test("finds user by id", fn()
    User.mock_query_result(
      "FOR doc IN users FILTER doc._key == @key RETURN doc",
      [
        {
          "_key": "123",
          "_id": "default:users/123",
          "name": "Alice",
          "email": "alice@example.com"
        }
      ]
    )
    
    user = User.find("123")
    expect(user.name).to_equal("Alice")
  end)
  
  test("includes returns correct class for relations", fn()
    # Mock the parent query
    Contact.mock_query_result(
      "FOR doc IN contacts RETURN doc",
      [
        {
          "_key": "c1",
          "_id": "default:contacts/c1",
          "name": "Bob",
          "organisation_id": "default:organisations/o1"
        }
      ]
    )
    
    # Mock the included relation query
    Organisation.mock_query_result(
      "FOR doc IN organisations FILTER doc._key IN @keys RETURN doc",
      [
        {
          "_key": "o1",
          "_id": "default:organisations/o1",
          "name": "Acme Corp"
        }
      ]
    )
    
    contact = Contact.includes("organisation").first
    org = contact.organisation
    
    # Verify the relation has the correct class (not Contact)
    expect(org.class_name).to_equal("Organisation")
    expect(org.name).to_equal("Acme Corp")
  end)
end)
```

Key points:
- `Model.mock_query_result(query, results)` - Register mock data for an AQL query
- `Model.clear_mocks()` - Remove all registered mocks
- Include relations require mocking both the parent and related queries
- The `_id` field (e.g., `"default:organisations/o1"`) determines the correct class for included documents

```soli
describe("User model", fn()
  test("creates user with valid data", fn()
    user = User.create({
      "email": "test@example.com",
      "name": "Test User"
    });
    expect(user._errors).to_equal(null);
    expect(user.email).to_equal("test@example.com");
  end)

  test("fails validation for invalid data", fn()
    user = User.create({ "email": "" });
    expect(user._errors).not_to_equal(null);
  end)

  test("finds users with where clause", fn()
    User.create({ "name": "Alice", "age": 25 });
    User.create({ "name": "Bob", "age": 17 });

    # where() returns QueryBuilder - chain .all to get results
    adults = User.where("doc.age >= @age", { "age": 18 }).all;
    expect(len(adults)).to_equal(1);
  end)
end)
```

## Inspecting AQL Queries (Dev Tool)

When the server runs with `--dev`, every AQL query a request executes through the Model layer is captured into a per-request stack. Read it with the `dev_queries()` builtin and render it however you like — typically as a debug bar at the bottom of the page.

### `dev_queries()`

Returns an `Array<Hash>` of queries executed during the **current request**. The stack is cleared at the start of every request.

| Key | Type | Description |
|-----|------|-------------|
| `query` | `String` | The AQL sent to SoliDB |
| `bind_vars` | `Hash` or `null` | The bind variables, or `null` if the query had none |
| `duration_ms` | `Float` | Wall-clock time the query took, in milliseconds |

In production (without `--dev`), `dev_queries()` always returns an empty array — the executor skips logging entirely, so there's no overhead.

### Example: Controller

```soli
fn index
  users = User.where("doc.active == true").all;
  posts = Post.includes("author").all;

  return render("users/index", {
    "users":   users,
    "posts":   posts,
    "queries": dev_queries()
  });
end
```

### Example: Debug bar partial

```erb
<%# app/views/shared/_dev_bar.erb %>
<% if queries.length > 0 %>
  <div class="dev-bar">
    <h3><%= queries.length %> AQL queries</h3>
    <ol>
      <% for q in queries %>
        <li>
          <code><%= h(q["query"]) %></code>
          <% if q["bind_vars"] != null %>
            <small>binds: <%= h(json_stringify(q["bind_vars"])) %></small>
          <% end %>
          <span><%= q["duration_ms"] %> ms</span>
        </li>
      <% end %>
    </ol>
  </div>
<% end %>
```

### Coverage

Logged:
- All `Model` operations (`Model.all`, `.where()`, `.find()`, `.create()`, `.update()`, `.destroy()`, `.count`, eager-loaded `includes`, soft-delete scopes, etc.)
- Validation lookups (`uniqueness`)
- HABTM join-table operations

Not logged in v1:
- Direct `Solidb(host, db).query(...)` calls
- Mocked queries registered via `register_query_mock` (they short-circuit before the executor)
- Internal session storage queries that go through `SoliDBClient` directly

## Best Practices

1. **Keep models simple** - Just extend `Model`, no configuration needed
2. **Use meaningful class names** - They become collection names automatically
3. **Add validations** - Validate data before it reaches the database
4. **Use callbacks wisely** - Keep them focused and avoid heavy operations
5. **Add custom methods** - Encapsulate business logic in model methods
6. **Declare relationships** - Use `has_many`, `has_one`, `belongs_to` for associations
7. **Use `includes` for eager loading** - Avoid N+1 queries when accessing related data
8. **Use `join` for filtering** - When you only need to filter by existence, not preload
9. **Use migrations in production** - Define indexes and schema for optimal performance

## Database Migrations

> **Note**: Collections are now automatically created when you first use a Model. You can start using your models immediately without creating migrations.

However, for production applications, we recommend using migrations to:
- Define indexes for better query performance
- Set collection options (e.g., key options, sharding)
- Document your schema
- Handle schema changes over time

See the [Migrations Guide](/docs/migrations) for creating collections and indexes.
