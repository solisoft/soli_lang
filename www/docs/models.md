# Models

Models manage data and business logic in your MVC application. SoliLang provides a simple OOP-style interface for database operations.

## Defining Models

Create model files in `app/models/`. The collection name is **automatically derived** from the class name:

- `User` → `"users"`
- `BlogPost` → `"blog_posts"`
- `UserProfile` → `"user_profiles"`

```soli
// app/models/user.soli
class User extends Model { }
```

```soli
// app/models/blog_post.soli
class BlogPost extends Model { }
```

That's it! No need to manually specify collection names or field definitions.

## CRUD Operations

### Creating Records

```soli
let result = User.create({
    "email": "alice@example.com",
    "name": "Alice",
    "age": 30
});
// Returns: { "valid": true, "record": { "id": "...", "email": "...", ... } }
// Or on validation failure: { "valid": false, "errors": [...] }
```

### Finding Records

```soli
// Find by ID
let user = User.find("user123");

// Find all
let users = User.all();

// Find with where clause (SDBQL filter syntax)
let adults = User.where("doc.age >= @age", { "age": 18 });
let active = User.where("doc.status == @status", { "status": "active" });

// Complex conditions
let results = User.where("doc.age >= @min_age AND doc.role == @role", {
    "min_age": 21,
    "role": "admin"
});
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

// Get first result only
let first = User.where("doc.email == @email", { "email": "alice@example.com" }).first();

// Count with conditions
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
class User extends Model {
    validates("email", { "presence": true, "uniqueness": true })
    validates("name", { "presence": true, "min_length": 2, "max_length": 100 })
    validates("age", { "numericality": true, "min": 0, "max": 150 })
    validates("website", { "format": "^https?://" })
}
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

if result["valid"] {
    let user = result["record"];
    print("Created user: " + user["id"]);
} else {
    for error in result["errors"] {
        print(error["field"] + ": " + error["message"]);
    }
}
```

## Callbacks

Define lifecycle callbacks to run code at specific points:

```soli
class User extends Model {
    before_save("normalize_email")
    after_create("send_welcome_email")
    before_update("log_changes")
    after_delete("cleanup_related")

    fn normalize_email() -> Any {
        this.email = this.email.downcase();
    }

    fn send_welcome_email() -> Any {
        // Send email logic
    }
}
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
class Post extends Model {
    fn author() -> Any {
        return User.find(this.author_id);
    }
}

class User extends Model {
    fn posts() -> Any {
        return Post.where("doc.author_id == @id", { "id": this.id });
    }
}

// Usage
let post = Post.find("post123");
let author = post.author();

let user = User.find("user123");
let user_posts = user.posts();
```

## Custom Methods

Add custom methods to your models:

```soli
class User extends Model {
    fn is_admin() -> Bool {
        return this.role == "admin";
    }

    fn full_name() -> String {
        return this.first_name + " " + this.last_name;
    }
}

// Usage
let user = User.find("user123");
if user.is_admin() {
    print("Welcome, admin " + user.full_name());
}
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
// app/models/user.soli
class User extends Model {
    validates("email", { "presence": true, "uniqueness": true })
    validates("name", { "presence": true, "min_length": 2 })

    before_save("normalize_email")

    fn normalize_email() -> Any {
        this.email = this.email.downcase();
    }

    fn posts() -> Any {
        return Post.where("doc.user_id == @id", { "id": this.id });
    }

    fn is_adult() -> Bool {
        return this.age >= 18;
    }
}

// app/models/blog_post.soli
class BlogPost extends Model {
    validates("title", { "presence": true, "min_length": 3 })

    fn author() -> Any {
        return User.find(this.user_id);
    }
}

// Usage in controller
class UsersController extends Controller {
    fn index() -> Any {
        let users = User.all();
        return this.render("users/index", { "users": users });
    }

    fn show(id: String) -> Any {
        let user = User.find(id);
        let posts = user.posts().order("created_at", "desc").limit(5).all();
        return this.render("users/show", {
            "user": user,
            "posts": posts
        });
    }

    fn create() -> Any {
        let result = User.create({
            "name": this.params["name"],
            "email": this.params["email"],
            "age": this.params["age"]
        });

        if result["valid"] {
            return this.redirect("/users/" + result["record"]["id"]);
        } else {
            return this.render("users/new", { "errors": result["errors"] });
        }
    }
}
```

## Testing Models

See the [Testing Guide](/docs/testing) for comprehensive information on testing models.

```soli
describe("User model", fn() {
    test("creates user with valid data", fn() {
        let result = User.create({
            "email": "test@example.com",
            "name": "Test User"
        });
        expect(result["valid"]).to_equal(true);
        expect(result["record"]["email"]).to_equal("test@example.com");
    });

    test("fails validation for invalid data", fn() {
        let result = User.create({ "email": "" });
        expect(result["valid"]).to_equal(false);
    });

    test("finds users with where clause", fn() {
        User.create({ "name": "Alice", "age": 25 });
        User.create({ "name": "Bob", "age": 17 });

        let adults = User.where("doc.age >= @age", { "age": 18 }).all();
        expect(len(adults)).to_equal(1);
    });
});
```

## Best Practices

1. **Keep models simple** - Just extend `Model`, no configuration needed
2. **Use meaningful class names** - They become collection names automatically
3. **Add validations** - Validate data before it reaches the database
4. **Use callbacks wisely** - Keep them focused and avoid heavy operations
5. **Add custom methods** - Encapsulate business logic in model methods
6. **Use relationships** - Create methods that return related models

## Database Migrations

To manage your database schema, see the [Migrations Guide](/docs/migrations) for creating collections and indexes.
