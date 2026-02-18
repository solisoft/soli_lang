# Database Migrations

Migrations provide a structured way to evolve your database schema over time. Each migration is a versioned file that can be applied or rolled back.

## Overview

Migrations are stored in `db/migrations/` with the naming convention:
```
YYYYMMDDHHMMSS_name.sl
```

Each migration file contains `up()` and `down()` functions:

```soli
fn up(db: Any)    db.create_collection("users")
    db.create_index("users", "idx_email", ["email"], { "unique": true })
end

fn down(db: Any)    db.drop_index("users", "idx_email")
    db.drop_collection("users")
end
```

## CLI Commands

### Generate a Migration

```bash
soli db:migrate generate create_users_table
```

This creates a timestamped migration file:
```
db/migrations/20260122143052_create_users_table.sl
```

### Run Migrations

```bash
# Apply all pending migrations
soli db:migrate

# Or explicitly
soli db:migrate up
```

### Rollback

```bash
# Rollback the last migration
soli db:migrate down
```

### Check Status

```bash
# Show migration status
soli db:migrate status
```

Output:
```
  Database Migrations

  Version         Name                            Status
  --------------  ------------------------------  ----------
  20260122143052  create_users_table                 up
  20260122145201  add_posts_table                    up
  20260122151033  add_user_indexes                  down

  2 applied, 1 pending
```

## Collection Helpers

### create_collection

Create a new collection (table):

```soli
fn up(db: Any)    db.create_collection("users")
    db.create_collection("posts")
    db.create_collection("comments")
end
```

### drop_collection

Remove a collection:

```soli
fn down(db: Any)    db.drop_collection("comments")
    db.drop_collection("posts")
    db.drop_collection("users")
end
```

### list_collections

List all collections in the database:

```soli
fn up(db: Any)    let collections = db.list_collections()
    print(collections)
end
```

### collection_stats

Get statistics for a collection:

```soli
fn up(db: Any)    let stats = db.collection_stats("users")
    print(stats)
end
```

## Index Helpers

### create_index

Create an index on a collection:

```soli
fn up(db: Any)    # Simple index on one field
    db.create_index("users", "idx_email", ["email"], {})

    # Unique index
    db.create_index("users", "idx_username", ["username"], { "unique": true })

    # Sparse index (only indexes documents that contain the field)
    db.create_index("users", "idx_phone", ["phone"], { "sparse": true })

    # Compound index on multiple fields
    db.create_index("users", "idx_name", ["first_name", "last_name"], {})

    # Unique compound index
    db.create_index("posts", "idx_user_slug", ["user_id", "slug"], { "unique": true })
end
```

**Parameters:**
- `collection` - The collection name
- `name` - The index name (must be unique within the collection)
- `fields` - Array of field names to index
- `options` - Hash with optional settings:
  - `unique: true` - Enforce unique values
  - `sparse: true` - Only index documents containing the indexed fields

### drop_index

Remove an index:

```soli
fn down(db: Any)    db.drop_index("users", "idx_email")
    db.drop_index("users", "idx_username")
    db.drop_index("posts", "idx_user_slug")
end
```

### list_indexes

List all indexes for a collection:

```soli
fn up(db: Any)    let indexes = db.list_indexes("users")
    print(indexes)
end
```

## Raw Queries

For operations not covered by helpers, use raw SDBQL queries:

```soli
fn up(db: Any)    # Insert seed data
    db.query("INSERT { name: 'Admin', role: 'admin' } INTO users")

    # Update existing data
    db.query("FOR u IN users FILTER u.role == 'guest' UPDATE u WITH { role: 'user' } IN users")

    # Complex queries with bind variables
    db.query("FOR doc IN users FILTER doc.status == @status RETURN doc")
end
```

## Complete Example

Here's a complete migration for a blog application:

```soli
# db/migrations/20260122143052_create_blog_schema.sl
# Migration: create_blog_schema
# Created: 2026-01-22 14:30:52

fn up(db: Any)    # Create collections
    db.create_collection("users")
    db.create_collection("posts")
    db.create_collection("comments")
    db.create_collection("tags")

    # User indexes
    db.create_index("users", "idx_users_email", ["email"], { "unique": true })
    db.create_index("users", "idx_users_username", ["username"], { "unique": true })

    # Post indexes
    db.create_index("posts", "idx_posts_author", ["author_id"], {})
    db.create_index("posts", "idx_posts_slug", ["slug"], { "unique": true })
    db.create_index("posts", "idx_posts_published", ["published_at"], { "sparse": true })

    # Comment indexes
    db.create_index("comments", "idx_comments_post", ["post_id"], {})
    db.create_index("comments", "idx_comments_author", ["author_id"], {})

    # Tag indexes
    db.create_index("tags", "idx_tags_name", ["name"], { "unique": true })
end

fn down(db: Any)    # Drop indexes first
    db.drop_index("tags", "idx_tags_name")
    db.drop_index("comments", "idx_comments_author")
    db.drop_index("comments", "idx_comments_post")
    db.drop_index("posts", "idx_posts_published")
    db.drop_index("posts", "idx_posts_slug")
    db.drop_index("posts", "idx_posts_author")
    db.drop_index("users", "idx_users_username")
    db.drop_index("users", "idx_users_email")

    # Drop collections
    db.drop_collection("tags")
    db.drop_collection("comments")
    db.drop_collection("posts")
    db.drop_collection("users")
end
```

## Environment Configuration

Migrations use environment variables for database connection. Create a `.env` file in your app root:

```bash
SOLIDB_HOST=http://localhost:6745
SOLIDB_DATABASE=myapp_development
SOLIDB_USERNAME=root
SOLIDB_PASSWORD=secret
```

Or set them directly:

```bash
export SOLIDB_HOST=http://localhost:6745
export SOLIDB_DATABASE=myapp_development
soli db:migrate
```

## Migration Tracking

Applied migrations are tracked in the `_migrations` collection with:

- `version` - The timestamp portion of the filename
- `name` - The descriptive name
- `batch` - The batch number (incremented each time migrations run)
- `executed_at` - When the migration was applied

## Best Practices

1. **Keep migrations small** - One logical change per migration
2. **Always write down()** - Enable clean rollbacks
3. **Test rollbacks** - Run `down` then `up` to verify reversibility
4. **Use descriptive names** - `create_users_table` not `migration1`
5. **Order matters in down()** - Drop indexes before collections, children before parents
6. **Don't modify old migrations** - Create new ones for changes
7. **Use unique index names** - Include collection name in index name for clarity

## Helpers Reference

| Method | Description |
|--------|-------------|
| `db.create_collection(name)` | Create a new collection |
| `db.drop_collection(name)` | Drop a collection |
| `db.list_collections()` | List all collections |
| `db.collection_stats(name)` | Get collection statistics |
| `db.create_index(collection, name, fields, options)` | Create an index |
| `db.drop_index(collection, name)` | Drop an index |
| `db.list_indexes(collection)` | List indexes for a collection |
| `db.query(sdbql)` | Execute a raw SDBQL query |
