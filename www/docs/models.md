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
User.update("user123", {
    "name": "Alice Smith",
    "age": 31
});
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
| `Model.find(id)` | Get document by ID |
| `Model.where(filter, bind_vars)` | Query with SDBQL filter |
| `Model.all()` | Get all documents |
| `Model.update(id, data)` | Update a document |
| `Model.delete(id)` | Delete a document |
| `Model.count()` | Count all documents |

## QueryBuilder Methods

| Method | Description |
|--------|-------------|
| `.where(filter, bind_vars)` | Add filter condition (ANDed with existing) |
| `.order(field, direction)` | Set sort order ("asc" or "desc") |
| `.limit(n)` | Limit results to n documents |
| `.offset(n)` | Skip first n documents |
| `.all()` | Execute query, return all results |
| `.first()` | Execute query, return first result |
| `.count()` | Execute query, return count |

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

Implement relationships using model methods:

```soli
class Post extends Model
    fn author()        User.find(this.author_id)
    end
end

class User extends Model
    # Returns a QueryBuilder for chaining
    fn posts()        Post.where("doc.author_id == @id", { "id": this.id })
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
| `User.where("doc.age >= @age", {"age": 18})` | `FOR doc IN users FILTER doc.age >= @age RETURN doc` |
| `.order("name", "asc")` | `... SORT doc.name ASC RETURN doc` |
| `.limit(10).offset(20)` | `... LIMIT 20, 10 RETURN doc` |
| `User.count()` | `FOR doc IN users COLLECT WITH COUNT INTO count RETURN count` |

SDBQL uses:
- `FOR doc IN collection` instead of `SELECT * FROM`
- `FILTER expression` instead of `WHERE`
- `SORT doc.field ASC/DESC` instead of `ORDER BY`
- `@variable` syntax for bind parameters

## Complete Example

```soli
# app/models/user.sl
class User extends Model
    validates("email", { "presence": true, "uniqueness": true })
    validates("name", { "presence": true, "min_length": 2 })

    before_save("normalize_email")

    fn normalize_email()        this.email = this.email.downcase();
    end

    fn posts()        Post.where("doc.user_id == @id", { "id": this.id })
    end

    fn is_adult() -> Bool
        this.age >= 18
    end
end

# app/models/blog_post.sl
class BlogPost extends Model
    validates("title", { "presence": true, "min_length": 3 })

    fn author()        User.find(this.user_id)
    end
end

# Usage in controller
class UsersController extends Controller
    fn index(req)
        let users = User.all();
        render("users/index", { "users": users })
    end

    fn show(req)
        let id = req["params"]["id"];
        let user = User.find(id);
        let posts = user.posts().order("created_at", "desc").limit(5).all();
        render("users/show", {
            "user": user,
            "posts": posts
        })
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
6. **Use relationships** - Create methods that return related models
7. **Use migrations in production** - Define indexes and schema for optimal performance

## Database Migrations

> **Note**: Collections are now automatically created when you first use a Model. You can start using your models immediately without creating migrations.

However, for production applications, we recommend using migrations to:
- Define indexes for better query performance
- Set collection options (e.g., key options, sharding)
- Document your schema
- Handle schema changes over time

See the [Migrations Guide](/docs/migrations) for creating collections and indexes.
