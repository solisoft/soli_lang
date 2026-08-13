# Database Migrations

Migrations provide a structured way to evolve your database schema over time. Each migration is a versioned file that can be applied or rolled back.

## Overview

Migrations are stored in `db/migrations/` with the naming convention:
```
YYYYMMDDHHMMSS_name.sl
```

Each migration file contains `up()` and `down()` functions:

```soli
def up(db: Any)    db.create_collection("users")
  db.create_index("users", "idx_email", ["email"], { "unique": true })
end

def down(db: Any)    db.drop_index("users", "idx_email")
  db.drop_collection("users")
end
```

## Which backends run migrations

Everything below is written for **SoliDB**, the default backend. Migrations also
run on the SQL adapters (`postgres`, `mysql`, `sqlite`), where they manage
document tables *and* tables with **real columns** — see
[On the SQL adapters](#on-the-sql-adapters).

## Targeting a connection

A migration can name the database it belongs to, so `soli db:migrate up` places
it correctly with no CLI flag:

```soli
# db/migrations/20260813120000_create_events.sl
connection "analytics"

def up(db)
  db.create_table("events", { "id": "pk", "name": "string" })
end

def down(db)
  db.drop_table("events")
end
```

- The declaration must be the **first non-comment statement** in the file
  (blank lines and `#` / `//` comments may precede it). A `connection "…"`
  line inside a string, or after `def up`, is ignored — it cannot pick the
  target. A second declaration is an error, not a silent first-wins.
- The runner reads it before running anything, and it never executes as a
  statement.
- **Each connection tracks its own versions.** A migration applied to
  `analytics` is not marked applied on the default connection.
- Without a declaration, a migration runs on `--connection` if given, else the
  default connection.
- `--connection NAME` acts as a **filter**: a migration that declares a
  different database is held back (reported as skipped) rather than applied to
  the wrong schema.
- `soli db:migrate status` shows a Connection column as soon as one migration
  declares one.

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
# Apply all pending migrations (action is required)
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

Create a new collection. The optional second argument selects the collection type — the string is forwarded verbatim to SolidB; the server decides what's valid. Default is a regular document collection.

| `type` | Use for |
|--------|---------|
| (omitted) | Standard JSON document collection (the common case). |
| `"blob"` | Binary attachments; required for `solidb_store_blob` and the [uploader DSL](models#uploaders). |
| `"edge"` | Graph edges (`_from`/`_to` documents); backs the [`edge` model DSL](models#graph-models-edges-and-traversal) and traversal queries. |
| `"timeseries"` | Append-only time-indexed events (metrics, logs, telemetry); SolidB optimizes range queries on the timestamp. Backs the [`timeseries` model DSL](models#timeseries-models). |

> `db.create_collection(name, "columnar")` **raises** — columnar stores are
> not document collections (it used to silently create a mislabeled document
> collection). Use [`db.create_columnar`](#create_columnar) instead.

```soli
def up(db: Any)
  db.create_collection("users")                          # document
  db.create_collection("posts")
  db.create_collection("contact_documents", "blob")      # blob
  db.create_collection("follows", "edge")                # edge (graph)
  db.create_collection("metrics", "timeseries")          # timeseries
end
```

For an edge collection, add hash indexes on the endpoint fields so traversals
stay fast (dev auto-create does both steps for you):

```soli
def up(db: Any)
  db.create_collection("follows", "edge")
  db.create_index("follows", "idx_follows_from", ["_from"], {})
  db.create_index("follows", "idx_follows_to", ["_to"], {})
end
```

### create_columnar

Create a [columnar store](analytics.md#columnar-models) — a separate
column-oriented engine, not a document collection. `columns` is an array of
`{ "name": ..., "type": ..., "nullable"?: bool, "indexed"?: bool }` hashes;
the optional `options` hash accepts `{ "compression": "lz4" | "none" }`
(default `lz4`):

```soli
def up(db: Any)
  db.create_columnar("page_views", [
    { "name": "url", "type": "string" },
    { "name": "visited_at", "type": "timestamp" },
    { "name": "duration_ms", "type": "int", "nullable": true },
    { "name": "country", "type": "string", "indexed": true }
  ], { "compression": "lz4" })
end
```

### drop_columnar

Remove a columnar store:

```soli
def down(db: Any)
  db.drop_columnar("page_views")
end
```

### prune_collection

Delete documents older than an RFC3339 cutoff from a `timeseries` collection —
the migration-side counterpart of [`Model.prune`](models#timeseries-models):

```soli
def up(db: Any)
  db.prune_collection("metrics", "2026-01-01T00:00:00Z")
end
```

### drop_collection

Remove a collection:

```soli
def down(db: Any)    db.drop_collection("comments")
  db.drop_collection("posts")
  db.drop_collection("users")
end
```

### list_collections

List all collections in the database:

```soli
def up(db: Any)    collections = db.list_collections()
  print(collections)
end
```

### collection_stats

Get statistics for a collection:

```soli
def up(db: Any)    stats = db.collection_stats("users")
  print(stats)
end
```

## Index Helpers

### create_index

Create an index on a collection:

```soli
def up(db: Any)    # Simple index on one field
  db.create_index("users", "idx_email", ["email"], {})

  # Unique index
  db.create_index("users", "idx_username", ["username"], { "unique": true })

  # Typed index — "hash" is the default; "persistent" for sorted/range lookups
  db.create_index("users", "idx_age", ["age"], { "type": "persistent" })

  # Fulltext index
  db.create_index("articles", "idx_articles_ft", ["title", "body"], { "type": "fulltext" })

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
  - `type: "..."` - Index kind: `"hash"` (default), `"persistent"`,
    `"fulltext"`, `"bloom"`, or `"cuckoo"` (`"skiplist"` / `"btree"` are
    accepted as aliases for `"persistent"`)

> The old `sparse` option was dropped — the server never read it. Remove it
> from existing migrations at your leisure; it changed nothing.

### create_vector_index / drop_vector_index

Create an HNSW vector index for [ANN search](search.md#vector-search-similar).
The last argument is a metric string (`"cosine"`, the default) or a hash with
`metric` and `quantization`:

```soli
def up(db: Any)
  db.create_vector_index("articles", "idx_articles_embedding", "embedding", 1536, "cosine")
end

def down(db: Any)
  db.drop_vector_index("articles", "idx_articles_embedding")
end
```

### Model-declared indexes and `soli db:indexes`

Models can declare indexes in the class body (`index`, `vector_index`,
`fulltext_index`, `geo_index` — see [Search](search.md#index-dsl)). Those
declarations are metadata-only: dev creates them at server boot, and in
production you either mirror them in migrations (the recommended DDL path) or
run the reconciler:

```bash
soli db:indexes [folder]   # create any missing declared indexes
```

Geo indexes currently have no migration helper — `soli db:indexes` (or dev
boot) is the way to create them.

### drop_index

Remove an index:

```soli
def down(db: Any)    db.drop_index("users", "idx_email")
  db.drop_index("users", "idx_username")
  db.drop_index("posts", "idx_user_slug")
end
```

### list_indexes

List all indexes for a collection:

```soli
def up(db: Any)    indexes = db.list_indexes("users")
  print(indexes)
end
```

## Raw Queries

For operations not covered by helpers, use raw SDBQL queries:

```soli
def up(db: Any)    # Insert seed data
  db.query("INSERT { name: 'Admin', role: 'admin' } INTO users")

  # Update existing data
  db.query("FOR u IN users FILTER u.role == 'guest' UPDATE u WITH { role: 'user' } IN users")

  # Bind variables (preferred for user data — avoids escaping issues)
  digest = bcrypt_hash("changeme")
  db.query(
    "INSERT { email: @e, name: @n, role: @r, password_digest: @d } INTO users",
    { "e": "admin@example.com", "n": "Admin", "r": "admin", "d": digest }
  )

  # Bind variables work in FILTER / RETURN too
  db.query(
    "FOR doc IN users FILTER doc.status == @status RETURN doc",
    { "status": "active" }
  )
end
```

Raw queries are also the escape hatch for SolidB features the helpers don't
wrap — e.g. continuous aggregates over a timeseries collection via
`db.query("CREATE STREAM ...")`.

## Complete Example

Here's a complete migration for a blog application:

```soli
# db/migrations/20260122143052_create_blog_schema.sl
# Migration: create_blog_schema
# Created: 2026-01-22 14:30:52

def up(db: Any)    # Create collections
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
  db.create_index("posts", "idx_posts_published", ["published_at"], { "type": "persistent" })

  # Comment indexes
  db.create_index("comments", "idx_comments_post", ["post_id"], {})
  db.create_index("comments", "idx_comments_author", ["author_id"], {})

  # Tag indexes
  db.create_index("tags", "idx_tags_name", ["name"], { "unique": true })
end

def down(db: Any)    # Drop indexes first
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

## On the SQL adapters

On a `postgres`, `mysql`, or `sqlite` connection a migration can build either
kind of table:

- a **document table** (`_key` + `doc`) — `create_table("posts")` with no column
  hash. Document tables are also auto-created on first write, so this is only
  needed when you want the table up front or want an explicit rollback path.
- a **column table** — `create_table("orders", { … })` with a column hash. These
  are the tables [column-aware models](multi-database.md#column-aware-models-existing-databases)
  map onto.

Anything SoliDB-specific raises rather than being silently skipped.

### Column tables

```soli
def up(db)
  db.create_table("orders", {
    "id":         "pk",
    "code":       { "type": "string", "limit": 32, "null": false },
    "amount":     "decimal(10,2)",
    "qty":        "integer",
    "paid":       { "type": "boolean", "default": false },
    "meta":       "json",
    "user_id":    { "type": "bigint", "references": "users" },
    "timestamps": true
  })
  db.add_index("orders", ["code"], { "unique": true })
end

def down(db)
  db.drop_table("orders")
end
```

One migration, three backends: the types below are Soli's, and each adapter
renders its own SQL. The rendered names are chosen so introspection reads the
table back as the same Soli types — a table created this way is always one a
column-aware model can map.

| Soli type | Postgres | MySQL | SQLite |
|---|---|---|---|
| `pk` | `BIGSERIAL PRIMARY KEY` | `BIGINT AUTO_INCREMENT PRIMARY KEY` | `INTEGER PRIMARY KEY AUTOINCREMENT` |
| `uuid_pk` | `UUID PRIMARY KEY` | `CHAR(36) PRIMARY KEY` | `UUID PRIMARY KEY` |
| `string` / `string(n)` | `VARCHAR(255)` / `VARCHAR(n)` | same | same |
| `text` | `TEXT` | `TEXT` | `TEXT` |
| `integer` | `INTEGER` | `INT` | `INTEGER` |
| `bigint` | `BIGINT` | `BIGINT` | `BIGINT` |
| `float` | `DOUBLE PRECISION` | `DOUBLE` | `REAL` |
| `decimal(p,s)` | `NUMERIC(p,s)` | `DECIMAL(p,s)` | `DECIMAL(p,s)` |
| `boolean` | `BOOLEAN` | `TINYINT(1)` | `BOOLEAN` |
| `date` | `DATE` | `DATE` | `DATE` |
| `datetime` | `TIMESTAMPTZ` | `DATETIME` | `DATETIME` |
| `json` | `JSONB` | `JSON` | `JSON` |
| `uuid` | `UUID` | `CHAR(36)` | `UUID` |
| `binary` | `BYTEA` | `BLOB` | `BLOB` |

Column options (the hash form): `type`, `limit` (string only), `null`, `unique`,
`primary_key`, `default`, `references`. Plus the table-level `"timestamps": true`,
which adds `created_at` / `updated_at` as `NOT NULL DEFAULT CURRENT_TIMESTAMP`.

`"references": "users"` points at that table's `id`; write `"users(uuid)"` to
name another column. On MySQL the constraint is emitted at table level, because
MySQL parses an inline `REFERENCES` and then ignores it.

### Schema helpers

| Helper | Notes |
|--------|-------|
| `db.create_table(name)` | Document table (`_key` + `doc`) |
| `db.create_table(name, columns)` | Column table, as above |
| `db.drop_table(name)` | Either kind |
| `db.add_column(table, name, type)` | Type string or options hash |
| `db.drop_column(table, name)` | |
| `db.rename_column(table, old, new)` | MySQL 8+ |
| `db.rename_table(old, new)` | |
| `db.add_index(table, columns, options?)` | `{ "unique": true, "name": "…" }`; the name defaults to `idx_<table>_<columns>` |
| `db.add_index(table, ["doc.status"])` | A `doc.` prefix indexes a **JSON field of a document table** (expression index; a generated column on MySQL) |
| `db.drop_index(table, name)` | |
| `db.create_index(table, name, fields, options?)` | The SoliDB-shaped call, so a shared migration keeps working |
| `db.execute(sql)` | Escape hatch — engine-specific by definition. Migration-only (not callable from controllers, jobs, or templates). Runs on a dedicated connection so `SET` / `ATTACH` / `PRAGMA` cannot leak into the request pool. |
| `db.create_collection(name)` / `db.drop_collection(name)` | Aliases for document tables |
| `db.create_collection(name, "edge"/"timeseries"/…)` | ✗ raises — typed collections are SoliDB-only |
| `db.query(sdbql)` | ✗ raises — SDBQL has no meaning on SQL |
| `db.create_columnar`, `db.create_vector_index` | ✗ raise — SoliDB-only |

Two limits worth knowing before you hit them:

- **SQLite's `ALTER TABLE` is narrow.** It cannot add a `UNIQUE` or primary-key
  column to an existing table (add the column, then `add_index`), and it cannot
  add a `NOT NULL` column without a `default`. Both are reported as errors that
  name the way around them.
- **Changing a column's type is not portable**, so there is no
  `change_column`. Use `db.execute` for that, or add-copy-drop.

**Models never issue DDL.** A column-aware model maps to a table it does not
own: no auto-create, no index sync, no implicit `ALTER`. Migrations are the one
place Soli changes a column table, and only where you wrote it.

`_migrations`, `_jobs`, and `_cron_jobs` are reserved. A migration that
creates, drops, or renames them is refused.

## Environment Configuration

Migrations use environment variables for database connection. Create a `.env` file in your app root:

```bash
SOLIDB_HOST=http://localhost:6745
SOLIDB_DATABASE=myapp_development
SOLIDB_USERNAME=root
SOLIDB_PASSWORD=secret
```

On a SQL adapter the target comes from `SOLI_DB_ADAPTER` + `DATABASE_URL`
instead (or from the named connection in `config/database.toml`):

```bash
SOLI_DB_ADAPTER=sqlite
DATABASE_URL=sqlite://db/app.sqlite3
```

**The database itself** is not created by a migration. SoliDB makes its database
on first use, but a SQL server does not, so a fresh target needs one command
first:

```bash
soli db:create              # CREATE DATABASE (or the SQLite file + its directory)
soli db:create -c legacy    # a named connection
soli db:migrate up
soli db:drop                # removes it — WAL sidecars included on SQLite
```

Or set them directly:

```bash
export SOLIDB_HOST=http://localhost:6745
export SOLIDB_DATABASE=myapp_development
soli db:migrate up
```

## Migration Tracking

Applied migrations are tracked in the `_migrations` collection — a real
`_migrations` table on the SQL adapters — with:

- `version` - The timestamp portion of the filename
- `name` - The descriptive name
- `batch` - The batch number (incremented each time migrations run)
- `executed_at` - When the migration was applied

## Seeding the Database

Migrations build the schema; **seeds** populate it with data — demo accounts, lookup
tables, an initial admin user. Run them with `soli db:seed`.

Unlike migrations, seeds are **not tracked** — every file runs on every invocation. Make
your seeds idempotent (guard with `first_by` / `find_by`) so re-running them doesn't create
duplicates.

### Where seeds live

```
db/seeds.sl          # runs first
db/seeds/*.sl        # then every file here, sorted by name
```

A new app ships with a `db/seeds.sl` starter. For larger or ordered datasets, generate
additional files:

```bash
soli db:seed generate demo_users
# -> db/seeds/20260623161240_demo_users.sl
```

### Running seeds

```bash
# Run db/seeds.sl, then db/seeds/*.sl (in the current project)
soli db:seed

# Run a single seed file (path resolved relative to the project folder)
soli db:seed db/seeds/20260623161240_demo_users.sl

# Point at a different project folder
soli db:seed ./myapp
```

Seeds run with your `app/models` (and `app/services`) auto-loaded, so they can use the
Model API directly — no imports needed:

```soli
# db/seeds.sl
3.times do |i|
  let email = "user#{i}@example.com"
  User.create({ "name": "User #{i}", "email": email }) if User.first_by("email", email).nil?
end

print("Seeded users")
```

The same `.env` / `SOLIDB_*` configuration used by migrations (see below) supplies the
database connection. A seed that throws stops the run and exits non-zero.

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
| `db.create_collection(name, type?)` | Create a collection. `type` is optional — `"blob"`, `"edge"`, `"timeseries"`, etc.; default is a document collection. `"columnar"` raises — use `create_columnar`. |
| `db.create_columnar(name, columns, options?)` | Create a columnar store. `columns` is an array of `{name, type, nullable?, indexed?}` hashes; `options` accepts `{"compression": "lz4"\|"none"}`. |
| `db.drop_columnar(name)` | Drop a columnar store |
| `db.prune_collection(name, cutoff)` | Delete documents older than an RFC3339 cutoff from a timeseries collection |
| `db.drop_collection(name)` | Drop a collection |
| `db.list_collections()` | List all collections |
| `db.collection_stats(name)` | Get collection statistics |
| `db.create_index(collection, name, fields, options)` | Create an index. `options`: `unique:` and `type:` (`"hash"` default, `"persistent"`, `"fulltext"`, `"bloom"`, `"cuckoo"`) |
| `db.create_vector_index(collection, name, field, dimension, options?)` | Create an HNSW vector index. `options`: metric string or `{metric, quantization}` hash |
| `db.drop_vector_index(collection, name)` | Drop a vector index |
| `db.drop_index(collection, name)` | Drop an index |
| `db.list_indexes(collection)` | List indexes for a collection |
| `db.query(sdbql, bind_vars?)` | Execute a raw SDBQL query, optionally with a hash of bind variables |
