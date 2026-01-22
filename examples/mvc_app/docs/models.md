# Models

Models manage data and business logic in your MVC application. SoliLang provides an ORM-style interface for database operations.

## Defining Models

Create model files in `app/models/`:

```soli
// app/models/user.soli
class User extends Model {
    static {
        this.collection = "users";
        this.fields = [
            "email",
            "name",
            "password_hash",
            "created_at",
            "updated_at"
        ];
        this.soft_deletes = true;
    }
}
```

## Field Types

Models support various field types:

```soli
class User extends Model {
    static {
        this.collection = "users";
        this.fields = [
            "email",           # String
            "age",             # Integer
            "balance",         # Float
            "is_active",       # Boolean
            "metadata",        # Hash
            "tags",            # Array
            "created_at"       # DateTime
        ];
    }
}
```

## CRUD Operations

### Creating Records

```soli
let user = User.create(hash(
    "email": "test@example.com",
    "name": "Test User"
));
```

### Finding Records

```soli
# Find by ID
let user = User.find("user123");

# Find first matching
let user = User.find_by("email", "test@example.com");

# Find all
let users = User.all();

# Find with where clause
let users = User.where("age", ">", 18);
let users = User.where("status", "==", "active");
```

### Updating Records

```soli
let user = User.find("user123");
user.name = "New Name";
user.save();
```

### Deleting Records

```soli
let user = User.find("user123");
user.delete();

# Soft delete (if enabled)
user.soft_delete();
```

## Model Relationships

### Has Many

```soli
class User extends Model {
    static {
        this.collection = "users";
        this.fields = ["email", "name"];
    }
}

class Post extends Model {
    static {
        this.collection = "posts";
        this.fields = ["title", "content", "user_id"];
    }
    
    fn user() -> Any {
        return User.find(this.user_id);
    }
}

# Usage
let user = User.find("user123");
let posts = Post.where("user_id", "==", user.id);
```

### Has One

```soli
class Profile extends Model {
    static {
        this.collection = "profiles";
        this.fields = ["user_id", "bio", "avatar"];
    }
    
    fn user() -> Any {
        return User.find(this.user_id);
    }
}
```

## Model Hooks

```soli
class User extends Model {
    static {
        this.collection = "users";
        this.fields = ["email", "name"];
    }
    
    fn before_save() -> Any {
        if this.email == null {
            return Error.new("Email is required");
        }
        return this;
    }
    
    fn after_create() -> Any {
        # Send welcome email
        return this;
    }
}
```

## Model Methods

Add custom methods to models:

```soli
class User extends Model {
    static {
        this.collection = "users";
        this.fields = ["email", "name", "role"];
    }
    
    fn is_admin() -> Bool {
        return this.role == "admin";
    }
    
    fn full_name() -> String {
        return this.name;
    }
    
    fn update_login_time() -> Any {
        this.last_login = DateTime.now();
        this.save();
        return this;
    }
}
```

## Database Connection

Configure database connection in `config/routes.soli`:

```soli
# Connect to SoliDB
model_connect("localhost", "myapp_db");

# With authentication
model_connect("localhost", "myapp_db", username: "admin", password: "secret");
```

## Migrations

Create database schema:

```soli
migration("20240101_create_users", "Create users table", fn() {
    create_collection("users");
    add_field("users", "email", "string", required: true);
    add_field("users", "name", "string", required: true);
    add_field("users", "password_hash", "string");
    add_field("users", "created_at", "datetime");
    add_field("users", "updated_at", "datetime");
    add_index("users", "email", unique: true);
});
```

## Testing Models

See the [Testing Guide](/docs/testing) for comprehensive information on testing models with factories and database isolation.

```soli
describe("User model", fn() {
    test("creates user with valid data", fn() {
        with_transaction(fn() {
            let user = Factory.create("user");
            expect(User.count()).to_equal(1);
            expect(user.email).to_be_valid_email();
        });
    });
    
    test("finds user by email", fn() {
        with_transaction(fn() {
            let user = Factory.create("user", hash("email": "test@example.com"));
            let found = User.find_by("email", "test@example.com");
            expect(found.id).to_equal(user.id);
        });
    });
});
```

## Best Practices

1. **Keep models focused** - Single responsibility per model
2. **Use migrations** - Version control your schema
3. **Add indexes** - Optimize frequent queries
4. **Use soft deletes** - Preserve data history
5. **Validate input** - Use model-level validation
6. **Use relationships** - Leverage associations
7. **Add timestamps** - Track created_at/updated_at
