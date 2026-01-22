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
let user = User.create({
    "email": "alice@example.com",
    "name": "Alice",
    "age": 30
});
// Inserts into "users" collection
```

### Finding Records

```soli
// Find by ID
let user = User.find("user123");

// Find all
let users = User.all();

// Find with where clause
let adults = User.where("age", ">=", 18);
let active = User.where("status", "==", "active");
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

## Static Methods Reference

| Method | Description |
|--------|-------------|
| `Model.create(data)` | Insert a new document |
| `Model.find(id)` | Get document by ID |
| `Model.where(field, op, value)` | Query with filter |
| `Model.all()` | Get all documents |
| `Model.update(id, data)` | Update a document |
| `Model.delete(id)` | Delete a document |
| `Model.count()` | Count all documents |

## Where Operators

The `where` method supports these comparison operators:

- `"=="` - Equal
- `"!="` - Not equal
- `">"` - Greater than
- `">="` - Greater than or equal
- `"<"` - Less than
- `"<="` - Less than or equal

```soli
// Find users older than 30
let users = User.where("age", ">", 30);

// Find posts by author
let posts = BlogPost.where("author_id", "==", "user123");
```

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
        return Post.where("author_id", "==", this.id);
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

## Complete Example

```soli
// app/models/user.soli
class User extends Model {
    fn posts() -> Any {
        return Post.where("user_id", "==", this.id);
    }

    fn is_adult() -> Bool {
        return this.age >= 18;
    }
}

// app/models/post.soli
class BlogPost extends Model {
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
        let posts = user.posts();
        return this.render("users/show", {
            "user": user,
            "posts": posts
        });
    }

    fn create() -> Any {
        let user = User.create({
            "name": this.params["name"],
            "email": this.params["email"],
            "age": this.params["age"]
        });
        return this.redirect("/users/" + user.id);
    }
}
```

## Testing Models

See the [Testing Guide](/docs/testing) for comprehensive information on testing models.

```soli
describe("User model", fn() {
    test("creates user with valid data", fn() {
        let user = User.create({
            "email": "test@example.com",
            "name": "Test User"
        });
        expect(user.email).to_equal("test@example.com");
    });

    test("finds user by ID", fn() {
        let user = User.create({ "name": "Alice" });
        let found = User.find(user.id);
        expect(found.name).to_equal("Alice");
    });

    test("counts users", fn() {
        User.create({ "name": "Alice" });
        User.create({ "name": "Bob" });
        expect(User.count()).to_equal(2);
    });
});
```

## Best Practices

1. **Keep models simple** - Just extend `Model`, no configuration needed
2. **Use meaningful class names** - They become collection names automatically
3. **Add custom methods** - Encapsulate business logic in model methods
4. **Use relationships** - Create methods that return related models

## Database Migrations

To manage your database schema, see the [Migrations Guide](/docs/migrations) for creating collections and indexes.
