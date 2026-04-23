# Models

Models manage data and business logic in your MVC application. SoliLang provides a simple OOP-style interface for database operations.

## Defining Models

Create model files in `app/models/`. The collection name is **automatically derived** from the class name:

- `User` → `"users"`
- `BlogPost` → `"blog_posts"`
- `UserProfile` → `"user_profiles"`

**Automatic Collection Creation**: When you call a Model method (like `create()`, `all()`, `find()`, etc.) on a collection that doesn't exist yet, SoliLang will automatically create the collection for you. This means you can start using your models immediately without running migrations first.

- `User` → `"users"`
- `BlogPost` → `"blog_posts"`
- `UserProfile` → `"user_profiles"`

```soli
# app/models/user.sl
class User extends Model
end
```

```soli
# app/models/blog_post.sl
class BlogPost extends Model
end
```

That's it! No need to manually specify collection names or field definitions.

## Auto-Loading

Every `.sl` file under `app/models/` is loaded automatically at startup — by `soli serve` (in each worker) and by the REPL. Model classes are therefore available everywhere (controllers, views, other models, the REPL) without an `import` statement.

```soli
# app/controllers/users_controller.sl — no import needed
class UsersController extends Controller
    fn index(req)
        render("users/index", { "users": User.all() })
    end
end
```

If you run a model or controller file directly with `soli run path/to/file.sl`, the auto-loader does **not** run — in that case you still need explicit imports.

## CRUD Operations

> **Auto-creation**: All Model operations automatically create the collection if it doesn't exist. This only happens on the first call that encounters a missing collection.

### Creating Records

```soli
let result = User.create({
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
let user = User.find("user123");

# Find all
let users = User.all();

# Find with where clause (SDBQL filter syntax)
# Note: where() returns a QueryBuilder - call .all() to get results
let adults = User.where("doc.age >= @age", { "age": 18 }).all();
let active = User.where("doc.status == @status", { "status": "active" }).all();

# Complex conditions
let results = User.where("doc.age >= @min_age AND doc.role == @role", {
    "min_age": 21,
    "role": "admin"
}).all();
```

### Updating Records

```soli
# Static method: update by ID
User.update("user123", {
    "name": "Alice Smith",
    "age": 31
});

# Instance method: modify fields and save
let user = User.find("user123");
user.name = "Alice Smith";
user.age = 31;
user.save();

# Instance method with bulk-update hash (same merge-then-persist path,
# one call instead of N assignments + save)
let user = User.find("user123");
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
let total = User.count();
```

## Query Builder Chaining

Chain methods to build complex queries:

```soli
let results = User
    .where("doc.age >= @age", { "age": 18 })
    .where("doc.active == @active", { "active": true })
    .order("created_at", "desc")
    .limit(10)
    .offset(20)
    .all();

# Get first result only
let first = User.where("doc.email == @email", { "email": "alice@example.com" }).first();

# Count with conditions
let count = User.where("doc.role == @role", { "role": "admin" }).count();
```

## Static Methods Reference

| Method | Description |
|--------|-------------|
| `Model.create(data)` | Insert a new document |
| `Model.create_many([data, ...])` | Batch insert multiple documents, returns `{ created, errors }` |
| `Model.find(id)` | Get document by ID |
| `Model.find_by(field, value)` | Find first record by field value |
| `Model.first_by(field, value)` | Find first record by field with ordering |
| `Model.find_or_create_by(field, value, data?)` | Find by field, or create if not found |
| `Model.where(filter, bind_vars)` | Query with SDBQL filter (returns QueryBuilder) |
| `Model.all()` | Get all documents |
| `Model.update(id, data)` | Update a document |
| `Model.upsert(id, data)` | Insert or update document by ID |
| `Model.delete(id)` | Delete a document |
| `Model.count()` | Count all documents |
| `Model.transaction { }` | Execute block in a database transaction |
| `Model.transaction("aql")` | Execute AQL in a database transaction |
| `Model.transaction()` | Get transaction handle for manual control |
| `Model.scope(name)` | Execute a named scope (returns QueryBuilder) |
| `Model.with_deleted()` | Include soft-deleted records (QueryBuilder) |
| `Model.only_deleted()` | Query only deleted records (QueryBuilder) |
| `Model.includes(rel, ...)` | Eager load relations (returns QueryBuilder) |
| `Model.includes(rel, filter, binds)` | Eager load with filter condition (returns QueryBuilder) |
| `Model.includes({ rel: [fields] })` | Eager load with field selection (returns QueryBuilder) |
| `Model.select(field, ...)` | Select specific fields (returns QueryBuilder) |
| `Model.fields(field, ...)` | Alias for `select()` (returns QueryBuilder) |
| `Model.join(rel, filter?, binds?)` | Filter by related existence (returns QueryBuilder) |
| `Model.order(field, dir?)` | Order results (returns QueryBuilder) |
| `Model.limit(n)` | Limit results (returns QueryBuilder) |
| `Model.offset(n)` | Offset results (returns QueryBuilder) |

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
| `.select(field, ...)` | Select specific fields on the main collection |
| `.fields(field, ...)` | Alias for `.select()` |
| `.join(rel, filter?, binds?)` | Filter by existence of related records |
| `.pluck(field, ...)` | Return only specified fields (single or array) |
| `.all()` | Execute query, return all results |
| `.first()` | Execute query, return first result |
| `.count()` | Execute query, return count |
| `.exists()` | Execute query, return boolean (true if records exist) |
| `.sum(field)` | Execute aggregation, return sum of field |
| `.avg(field)` | Execute aggregation, return average of field |
| `.min(field)` | Execute aggregation, return minimum of field |
| `.max(field)` | Execute aggregation, return maximum of field |
| `.group_by(field, func, agg_field)` | Execute grouping aggregation |
| `.to_query` | Return the generated SDBQL string (for debugging) |

## Validations

Define validation rules in your model class:

```soli
class User extends Model
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

`Model.create()` returns a validation result hash:

```soli
let result = User.create({ "email": "" });

if result["valid"]
    let user = result["record"];
    print("Created user: " + user["id"]);
else
    for error in result["errors"]
        print(error["field"] + ": " + error["message"]);
    end
end
```

## Callbacks

Define lifecycle callbacks to run code at specific points:

```soli
class User extends Model
    before_save("normalize_email")
    after_create("send_welcome_email")
    before_update("log_changes")
    after_delete("cleanup_related")

    fn normalize_email()        this.email = this.email.downcase();
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

## Relationships

Declare associations using the built-in DSL:

```soli
class User extends Model
    has_many("posts")
    has_one("profile")
end

class Post extends Model
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

Override defaults with an options hash:

```soli
class Post extends Model
    belongs_to("author", { "class_name": "User", "foreign_key": "author_id" })
end
```

### Eager Loading (includes)

Preload related records to avoid N+1 queries. Uses LET subqueries with MERGE:

```soli
# Load users with their posts and profiles in a single query
let users = User.includes("posts", "profile").all()

# Combine with where clauses
let active = User.where("active = @a", { "a": true }).includes("posts").first()

# Inspect the generated query
print(User.includes("posts").to_query)
# => FOR doc IN users LET _rel_posts = (FOR rel IN posts FILTER rel.user_id == doc._key RETURN rel) RETURN MERGE(doc, {posts: _rel_posts})
```

- `has_many` includes return an array of related documents
- `has_one` and `belongs_to` includes return a single document (via `FIRST()`)

### Join Filtering

Filter records by the existence of related records. Unlike `includes`, `join` does **not** preload the related data — it only filters:

```soli
# Find users who have at least one post
let users_with_posts = User.join("posts").all()

# Find users who have published posts
let count = User.join("posts", "published = @p", { "p": true }).count()

# Chain with other query methods
let recent = User.join("posts").order("created_at", "desc").limit(10).all()
```

This is equivalent to ActiveRecord's `joins` — use `includes` when you need the related data, and `join` when you only need to filter by existence.

### Filtered Includes

Filter included relations to load only matching related records:

```soli
# Only load published posts for each user
let users = User.includes("posts", "published = @p", { "p": true }).all()

# Inspect the generated query
print(User.includes("posts", "published = @p", { "p": true }).to_query)
# => ... LET _rel_posts = (FOR rel IN posts FILTER rel.user_id == doc._key AND rel.published == @p RETURN rel) ...
```

Combine a filter with field projection using the `"fields"` key in the bind hash:

```soli
# Only load title and body of published posts
let users = User.includes("posts", "published = @p", {
    "p": true,
    "fields": ["title", "body"]
}).all()
# => ... RETURN {title: rel.title, body: rel.body} ...
```

### Includes with Field Projection

Use a hash argument to select specific fields on included relations (without filtering):

```soli
# Only load title and body from posts
let users = User.includes({ "posts": ["title", "body"] }).all()
# => ... LET _rel_posts = (FOR rel IN posts FILTER rel.user_id == doc._key RETURN {title: rel.title, body: rel.body}) ...
```

### Chaining Multiple Includes

Chain `.includes()` calls to eagerly load multiple relations with different options:

```soli
# Filtered posts + unfiltered profile
let users = User.includes("posts", "published = @p", { "p": true })
    .includes("profile")
    .all()
```

### Field Selection (select / fields)

Use `.select()` (or its alias `.fields()`) to return only specific fields from the main collection. `_key` is always included automatically for identity:

```soli
# Only return name and email
let users = User.select("name", "email").all()
# => FOR doc IN users RETURN {name: doc.name, email: doc.email, _key: doc._key}

# .fields() is an alias
let users = User.fields("name", "email").all()
# => same query

# Combine with other query methods
let users = User.where("active = @a", { "a": true })
    .select("name", "email")
    .order("name")
    .limit(10)
    .all()

# Combine with includes
let users = User.select("name", "email").includes("posts").all()
# => ... RETURN MERGE({name: doc.name, email: doc.email, _key: doc._key}, {posts: _rel_posts})

# Full combo: select + filtered includes with field projection
let users = User.select("name")
    .includes("posts", "published = @p", { "p": true, "fields": ["title"] })
    .all()
```

### Manual Relationships

You can also implement relationships as custom methods for more control:

```soli
class Post extends Model
    fn author()
        User.find(this.author_id)
    end
end
```

## Finder Methods

Find records by specific field values:

```soli
# Find by exact field match
let user = User.find_by("email", "alice@example.com");

# Find with ordering (first by field value)
let user = User.first_by("name", "Alice");

# Find or create - returns existing or creates new
let user = User.find_or_create_by("email", "new@example.com");
let user = User.find_or_create_by("email", "new@example.com", { "name": "New User" });
```

### Dynamic Finder Methods

Automatically generated finders for any field combination:

```soli
# Single field finder
let user = User.find_by_email("alice@example.com");

# Two-field finder (AND logic)
let user = User.find_by_email_and_active("alice@example.com", true);

# Three+ field combinations
let post = Post.find_by_title_and_published_and_author_id("Hello", true, 123);
```

These methods return the first matching record or `null` if not found.

## Aggregations

Calculate sums, averages, min, max on query results:

```soli
# Sum
let total = User.where("age > @a", { "a": 18 }).sum("balance");

# Average
let avg = User.avg("score");

# Minimum
let min_score = User.min("score");

# Maximum
let max_score = User.max("views");

# Group by aggregation
let by_country = User.group_by("country", "sum", "balance");
# Returns: [{ group: "US", result: 1000 }, { group: "FR", result: 500 }, ...]
```

## Pluck and Exists

Quick queries for specific data:

```soli
# Get array of single field values
let names = User.where("active = @a", { "a": true }).pluck("name");
# Returns: ["Alice", "Bob", "Charlie"]

# Get multiple fields as objects
let users = User.pluck("name", "email");
# Returns: [{ name: "Alice", email: "alice@example.com" }, ...]

# Check if records exist (returns boolean)
let exists = User.where("role = @r", { "r": "admin" }).exists();
# Returns: true or false
```

## Instance Methods

Methods available on model instances:

```soli
let user = User.find("user_id");

# Update fields and persist
user.name = "New Name";
user.update();

# Atomic increment/decrement
user.increment("view_count");      # +1
user.increment("view_count", 5);  # +5
user.decrement("stock");           # -1

# Update timestamp only
user.touch();  # Updates _updated_at

# Refresh from database
user.reload();
```

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
let p = Product.find(id);
p.update({ "price": 99.00 });

# Mix field assignment with hash — pre-assigned fields fall back when hash
# omits them, hash wins on conflict.
let p = Product.new();
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

Define reusable query scopes:

```soli
class User extends Model
    scope("active", "active = @a", { "a": true })
    scope("recent", "1 = 1", {})  # no filter, just for chaining
end

# Use scopes
let active = User.scope("active").all();
let recent = User.scope("active").limit(10).all();
```

## Soft Delete

Mark records as deleted without removing them:

```soli
class Post extends Model
    soft_delete
end

# Delete sets deleted_at timestamp
post.delete();

# Restore clears deleted_at
post.restore();

# Query without deleted records (default behavior)
let posts = Post.all();

# Include soft-deleted records
let all = Post.with_deleted.all();

# Query only deleted records
let deleted = Post.only_deleted.all();
```

## Relationship Accessors

Access related records directly from instances:

```soli
let user = User.find("user_id");

# Access has_many relation
let posts = user.posts;

# Access has_one relation
let profile = user.profile;

# Access belongs_to relation
let author = post.user;

# Chain query builder methods on relations
let published = user.posts.where("published = @p", { "p": true }).all();
```

## Batch Operations

Insert or update multiple records:

```soli
# Batch create
let result = User.create_many([
    { "name": "Alice", "email": "alice@example.com" },
    { "name": "Bob", "email": "bob@example.com" },
    { "name": "Charlie", "email": "charlie@example.com" }
]);
# Returns: { "created": 3, "errors": [] }

# Upsert (insert or update by ID)
User.upsert("user123", { "name": "Updated Name" });
# Updates if exists, inserts with ID if not
```

## Transactions

Execute multiple operations atomically within a database transaction:

### Using a Block (Recommended)

```soli
# Execute block in a transaction
User.transaction {
    User.create({ name: "Alice", age: 30 });
    User.create({ name: "Bob", age: 25 });
}
# All operations commit together, or rollback on error
```

### Using AQL String

```soli
# Execute AQL in a transaction
let result = User.transaction("
    INSERT { name: 'Alice', age: 30 } INTO users;
    INSERT { name: 'Bob', age: 25 } INTO users;
    RETURN users
");
```

### Using Transaction Object (Manual Control)

```soli
# Get transaction handle for manual control
let tx = User.transaction();
tx.create({ name: "Alice" });
tx.create({ name: "Bob" });
tx.commit();
# Or tx.rollback() to undo all changes
```

All operations within the transaction either all succeed or all fail together.

class User extends Model
    fn posts()
        Post.where("doc.author_id == @id", { "id": this.id })
    end
end
```

## Custom Methods

Add custom methods to your models:

```soli
class User extends Model
    fn is_admin() -> Bool
        this.role == "admin"
    end

    fn full_name() -> String
        this.first_name + " " + this.last_name
    end
end

# Usage
let user = User.find("user123");
if user.is_admin()
    print("Welcome, admin " + user.full_name());
end
```

## Query Generation (SDBQL)

Under the hood, Model methods generate SDBQL (SoliDB Query Language) queries:

| Method | Generated SDBQL |
|--------|-----------------|
| `User.all()` | `FOR doc IN users RETURN doc` |
| `User.where("age >= @age", {"age": 18})` | `FOR doc IN users FILTER doc.age >= @age RETURN doc` |
| `.order("name", "asc")` | `... SORT doc.name ASC RETURN doc` |
| `.limit(10).offset(20)` | `... LIMIT 20, 10 RETURN doc` |
| `User.count()` | `RETURN COLLECTION_COUNT("users")` |
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
class User extends Model
    has_many("posts")
    has_one("profile")

    validates("email", { "presence": true, "uniqueness": true })
    validates("name", { "presence": true, "min_length": 2 })

    before_save("normalize_email")

    fn normalize_email()
        this.email = this.email.downcase();
    end

    fn is_adult() -> Bool
        this.age >= 18
    end
end

# app/models/post.sl
class Post extends Model
    belongs_to("user")
    has_many("comments")

    validates("title", { "presence": true, "min_length": 3 })
end

# app/models/profile.sl
class Profile extends Model
    belongs_to("user")
end

# Usage in controller
class UsersController extends Controller
    fn index(req)
        # Eager load posts and profiles to avoid N+1 queries
        let users = User.includes("posts", "profile").all();
        render("users/index", { "users": users })
    end

    fn show(req)
        let id = req["params"]["id"];
        let user = User.includes("posts").find(id);
        render("users/show", { "user": user })
    end

    fn active(req)
        # Find active users who have at least one post
        let users = User.join("posts")
            .where("active = @a", { "a": true })
            .order("created_at", "desc")
            .limit(10)
            .all();
        render("users/active", { "users": users })
    end

    fn create(req)
        let result = User.create({
            "name": req["params"]["name"],
            "email": req["params"]["email"],
            "age": req["params"]["age"]
        });

        if result["valid"]
            redirect("/users/" + result["record"]["id"])
        else
            render("users/new", { "errors": result["errors"] })
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
        
        let user = User.find("123")
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
        
        let contact = Contact.includes("organisation").first
        let org = contact.organisation
        
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
        let result = User.create({
            "email": "test@example.com",
            "name": "Test User"
        });
        expect(result["valid"]).to_equal(true);
        expect(result["record"]["email"]).to_equal("test@example.com");
    end)

    test("fails validation for invalid data", fn()
        let result = User.create({ "email": "" });
        expect(result["valid"]).to_equal(false);
    end)

    test("finds users with where clause", fn()
        User.create({ "name": "Alice", "age": 25 });
        User.create({ "name": "Bob", "age": 17 });

        # where() returns QueryBuilder - chain .all() to get results
        let adults = User.where("doc.age >= @age", { "age": 18 }).all();
        expect(len(adults)).to_equal(1);
    end)
end)
```

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
