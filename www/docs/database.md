# Database Configuration

SoliLang uses SoliDB as its database backend. This guide covers how to configure and connect to your database.

## Environment Variables

Database configuration is done through environment variables, typically stored in a `.env` file in your project root.

### Required Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `SOLIDB_HOST` | SoliDB server URL | `http://localhost:6745` |
| `SOLIDB_DATABASE` | Database name | `default` |

### Optional Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `SOLIDB_USERNAME` | Authentication username | None |
| `SOLIDB_PASSWORD` | Authentication password | None |

## .env File

When you create a new project with `soli new myapp`, a `.env` file is automatically generated:

```bash
# Database Configuration
SOLIDB_HOST=http://localhost:6745
SOLIDB_DATABASE=default
SOLIDB_USERNAME=admin
SOLIDB_PASSWORD=admin

# Application Settings
# APP_ENV=development
# APP_SECRET=your-secret-key-here
```

## Multiple Environments

Create environment-specific `.env` files for different environments:

```
myapp/
├── .env              # Default/development
├── .env.test         # Test environment
├── .env.production   # Production environment
```

### Development

```bash
# .env
SOLIDB_HOST=http://localhost:6745
SOLIDB_DATABASE=myapp_development
```

### Test

```bash
# .env.test
SOLIDB_HOST=http://localhost:6745
SOLIDB_DATABASE=myapp_test
```

### Production

```bash
# .env.production
SOLIDB_HOST=https://db.example.com
SOLIDB_DATABASE=myapp_production
SOLIDB_USERNAME=prod_user
SOLIDB_PASSWORD=secure_password_here
```

## Setting APP_ENV

The `APP_ENV` environment variable controls which environment-specific `.env` file is loaded. When set, the system automatically:

1. Loads the base `.env` file first
2. Then loads `.env.{APP_ENV}` to override values

This matches the convention used by Rails, Node.js, and other frameworks.

### Option 1: Shell Environment (Recommended for CI/Production)

Set `APP_ENV` when running commands:

```bash
# Run migrations in production
APP_ENV=production soli db:migrate

# Start server in test mode
APP_ENV=test soli serve

# Run with development settings (default)
soli serve
```

### Option 2: In .env File (For Local Development)

Set `APP_ENV` in your base `.env` file:

```bash
# .env
APP_ENV=development
SOLIDB_DATABASE=myapp_development
```

### File Loading Order

```
1. .env              (base configuration)
2. .env.{APP_ENV}    (environment overrides)
```

Environment-specific values override base values. For example:

```bash
# .env
SOLIDB_DATABASE=myapp_development

# .env.production
SOLIDB_DATABASE=myapp_production
```

Running `APP_ENV=production soli serve` will use `myapp_production` as the database.

## Starting SoliDB

Before running your application, start the SoliDB server:

```bash
# Start SoliDB (default port 6745)
solidb

# Or specify a custom port
solidb --port 6745

# With data directory
solidb --data ./data
```

## Verifying Connection

Test your database connection by running migrations:

```bash
cd myapp
soli db:migrate
```

If successful, you'll see:
```
Running migrations for database: myapp_development
All migrations are up to date.
```

## Using Models

Once configured, Models automatically connect to the database:

```soli
class User extends Model { }

// These all use the configured database
let users = User.all();
let user = User.create({ "name": "Alice" });
let found = User.find(user["id"]);
```

## Raw Queries

For raw database queries, use the `db` object or the `@sdql{}` query block syntax.

### Using db.query()

```soli
// SDBQL query with named parameters
let results = db.query("FOR doc IN users FILTER doc.age >= @age RETURN doc", {
    "age": 18
});

// Insert
db.query("INSERT { name: @name, email: @email } INTO users", {
    "name": "Bob",
    "email": "bob@example.com"
});
```

### Using @sdql{} Query Block

The `@sdql{}` syntax provides a more readable way to write database queries with interpolation:

```soli
// Simple query with interpolation
let users = @sdql{
    FOR u IN users
    FILTER u.age >= #{age}
    RETURN u
};

// Query with multiple interpolations
let results = @sdql{
    FOR u IN users
    FILTER u.age >= #{min_age} AND u.city == #{city}
    SORT u.name ASC
    LIMIT #{limit}
    RETURN u
};

// Insert with interpolation
@sdql{
    INSERT {
        name: #{name},
        email: #{email},
        created_at: NOW()
    } INTO users
};

// Update with interpolation
@sdql{
    UPDATE #{user_id} IN users
    SET {
        last_login: NOW()
    }
};

// Delete with interpolation
@sdql{
    REMOVE #{user_id} IN users
};
```

The `@sdql{}` block supports:
- **String interpolation** using `#{expression}` - expressions are evaluated at runtime
- **Multi-line queries** for better readability
- **All SDBQL operations**: FOR, FILTER, SORT, LIMIT, RETURN, INSERT, UPDATE, REMOVE

#### When to Use Each Syntax

| Approach | Use Case |
|----------|----------|
| `db.query()` with `@param` | When parameters should be passed separately (traditional parameterized queries) |
| `@sdql{}` with `#{expr}` | When you want inline interpolation and more readable multi-line queries |

## Connection Pooling

SoliLang automatically manages database connections. Each worker thread maintains its own connection to ensure optimal performance.

## Security Best Practices

1. **Never commit `.env` files** - Add `.env*` to `.gitignore`
2. **Use strong passwords** - Especially in production
3. **Use HTTPS in production** - Set `SOLIDB_HOST=https://...`
4. **Rotate credentials** - Change passwords periodically
5. **Limit network access** - Use firewalls to restrict database access

### Example .gitignore

```gitignore
# Environment files (contain secrets)
.env
.env.*
!.env.example
```

### Example .env.example

Create a template for other developers:

```bash
# .env.example (commit this file)
SOLIDB_HOST=http://localhost:6745
SOLIDB_DATABASE=myapp_development
SOLIDB_USERNAME=
SOLIDB_PASSWORD=
```

## Troubleshooting

### Connection Refused

```
Error: Failed to connect: Connection refused
```

**Solution**: Make sure SoliDB is running:
```bash
solidb
```

### Authentication Failed

```
Error: Authentication failed
```

**Solution**: Check your `SOLIDB_USERNAME` and `SOLIDB_PASSWORD` in `.env`.

### Database Not Found

```
Error: Database 'mydb' not found
```

**Solution**: The database is created automatically on first use. Check your `SOLIDB_DATABASE` value.

### Environment Variables Not Loading

**Solution**: Ensure `.env` is in your project root (same directory as `main.sl`).

## Next Steps

- [Models & ORM](/docs/models) - Learn how to work with data
- [Migrations](/docs/migrations) - Manage your database schema
- [Testing](/docs/testing) - Test with database isolation
