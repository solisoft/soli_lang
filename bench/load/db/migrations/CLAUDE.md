# Migrations

Schema and data changes live here. Each file is a self-contained, reversible
unit; the runner applies them in timestamp order and tracks which have been
applied.

**Always generate, never hand-name.** The timestamp prefix is the ordering
key, and the generator gets it right:

```bash
soli generate migration create_posts
soli generate scaffold post title:string body:text   # also generates a migration
```

## File layout

```
db/migrations/
├── 20260301120000_create_posts.sl
├── 20260301140000_create_comments.sl
├── 20260315090000_add_email_index_to_users.sl
└── 20260401110000_seed_demo_data.sl
```

Filename pattern: `<timestamp>_<snake_case_description>.sl`. The generator
emits a numeric timestamp prefix (Unix seconds or `YYYYMMDDHHMMSS` — either
form sorts correctly). The description after the underscore is for human
reading; the runner only uses the prefix for ordering and the full filename
as a tracking key.

## Anatomy of a migration

A migration file is **top-level functions**, not a class. Define `up(db)`
and `down(db)`. Both receive a `db` handle exposing schema operations.

```soli
# db/migrations/20260301120000_create_posts.sl

def up(db)
  db.create_collection("posts")

  db.create_index("posts", "idx_user_id",   ["user_id"], {})
  db.create_index("posts", "idx_slug",      ["slug"],    { "unique": true })
  db.create_index("posts", "idx_status_pub", ["status", "published_at"], {})
end

def down(db)
  db.list_indexes("posts").each do |idx|
    db.drop_index("posts", idx["name"])
  end
  db.drop_collection("posts")
end
```

## Available `db.*` operations

The `db` handle exposes:

| Operation                                                   | What it does                                                |
|-------------------------------------------------------------|-------------------------------------------------------------|
| `db.create_collection(name)`                                | Create a regular document collection.                        |
| `db.create_collection(name, type)`                          | Create a typed collection — `type` is forwarded verbatim to SoliDB (`"blob"`, `"columnar"`, `"timeseries"`, ...). |
| `db.drop_collection(name)`                                  | Remove a collection (including its data).                    |
| `db.list_collections()`                                     | Array of collection names.                                   |
| `db.collection_stats(name)`                                 | Hash with row count, size, etc.                              |
| `db.create_index(coll, name, fields, options)`              | Create an index. `fields` is an array; `options` is a hash.  |
| `db.drop_index(coll, name)`                                 | Drop an index by name.                                       |
| `db.list_indexes(coll)`                                     | Array of `{ "name": "...", "fields": [...], ... }` hashes.   |
| `db.create_vector_index(coll, name, field, dim, opts?)`     | Vector index for similarity search.                           |
| `db.query("SDBQL ...")`                                     | Run a raw SDBQL query string (for data migrations).          |

Index `options`:
- `"unique": true` — enforce uniqueness across `fields`.
- `"sparse": true` — index only documents where every listed field is set.

There is **no** `add_field` / `remove_field` / `rename_field` builtin —
SoliDB is schema-less at the document layer. Field changes only need a
migration when you want to backfill or rename existing rows; do that via
`db.query("...")` with a data-mutation SDBQL.

## Common shapes

### Add a collection + indexes

```soli
def up(db)
  db.create_collection("comments")
  db.create_index("comments", "idx_post_id", ["post_id"], {})
  db.create_index("comments", "idx_user_id", ["user_id"], {})
end

def down(db)
  db.list_indexes("comments").each do |idx|
    db.drop_index("comments", idx["name"])
  end
  db.drop_collection("comments")
end
```

### Add an index to an existing collection

```soli
def up(db)
  db.create_index("users", "idx_email", ["email"], { "unique": true })
end

def down(db)
  db.drop_index("users", "idx_email")
end
```

### Backfill data — SDBQL via `db.query`

`db.query(...)` takes a **plain SDBQL string**, not an `@sdbql{...}` block.
The `@sdbql{...}` DSL is a model-side feature with parameter binding for
runtime user input; migrations are developer-written and run with full
privileges, so build the string however you like.

```soli
def up(db)
  db.query("
    FOR p IN posts
      FILTER p.slug == null
      UPDATE p WITH { slug: SUBSTITUTE(LOWER(p.title), \" \", \"-\") } IN posts
  ")
end

def down(db)
  db.query("FOR p IN posts UPDATE p WITH { slug: null } IN posts")
end
```

For multi-line queries, use one of Soli's raw multiline string forms so you
don't have to escape every `"`:

- `[[ ... ]]` — Lua-style, raw (no escape processing). Best when the query
  contains `"` and you don't want to think about escaping.
- `""" ... """` — triple-quoted, raw, multiline. Best when the query contains
  `]` characters (e.g. array literals in SDBQL).
- `r"..."` — raw, single-line only.

> **There is no `@"..."` syntax.** Don't write `db.query(@"...")` — it's a
> parse error. The `@` prefix is reserved for `@sdbql{...}` model-side blocks
> (which you should not use here, see above).

```soli
def up(db)
  db.query([[
    FOR p IN posts
      FILTER p.slug == null
      UPDATE p WITH { slug: SUBSTITUTE(LOWER(p.title), ' ', '-') } IN posts
  ]])
end
```

If you need to splice values from the migration script, use plain string
interpolation (`#{var}`). This is **text substitution, not parameter binding**,
so make sure the value can't possibly come from untrusted input — but in a
migration, every value is yours.

### Seed data

```soli
def up(db)
  db.query("INSERT { _key: 'demo-1', title: 'Welcome', body: '...' } INTO posts")
end

def down(db)
  db.query("REMOVE { _key: 'demo-1' } IN posts")
end
```

For larger seeds, build a batch in Soli and INSERT in one query:

```soli
def up(db)
  let batch = (0..100).map(fn(i) { { "name": "User #{i}", "email": "user#{i}@demo" } })
  let json = batch.to_json
  db.query("FOR doc IN #{json} INSERT doc INTO users")
end
```

Keep seed migrations idempotent if you can — production may run them on a
DB that already has the data.

## Commands

```bash
soli db:migrate up                # apply all pending migrations in order
soli db:migrate down              # roll back the most recent applied migration
soli db:migrate status            # show applied / pending status for each file
soli db:migrate generate create_X # same as `soli generate migration create_X`
```

`up` is the everyday command. `down` is for local development — production
rolls forward, never back.

## Reversibility rule

Every migration **must** provide a real `down(db)` that undoes the `up`. No
`# TODO`, no empty bodies. This isn't enforced by the runner today, but the
review check is: can someone roll this back in their dev environment without
losing the rest of their data?

When the inverse is non-trivial (e.g. you split a column), write the
inverse anyway — the constraint forces you to think about whether you
designed the change cleanly.

If something is genuinely irreversible (deleting a collection whose data
can't be reconstructed), say so explicitly and short-circuit:

```soli
def down(db)
  raise("irreversible: 20260301120000_drop_posts_archive intentionally deletes the archive")
end
```

That's better than a silent no-op.

## Ordering

Migrations apply in **timestamp order**. The runner records each applied
file by its full filename in a tracking collection; on `up`, only files
not in the tracking set are applied.

**Never edit a migration that has already been applied in any environment** —
write a new migration. The runner uses the filename as the key; changing the
contents won't trigger a re-run, and you'll end up with drifting schemas
across environments.

If you really need to undo something already applied, write a new migration
that does the inverse.

## Test database lifecycle

The test runner **does not** auto-run migrations. Each test worker gets a
fresh database (`<base>_w<i>_<suffix>`) dropped and recreated before the
suite starts. If your tests need schema, run migrations explicitly in a
`before_all` block:

```soli
before_all() do
  db_migrate("up")
end
```

(Or set this up once at the top of `tests/spec_helper.sl` and require it
from each spec — whatever your project convention is.)

## Style

- **Indent at 2 spaces** in migration files.
- One `up` / `down` pair per file. Don't pile unrelated changes into one
  migration — small migrations are easier to roll back and review.
- Name files for the change, not the table: `add_email_index_to_users` is
  better than `update_users`.
- Use `idx_<field>` / `idx_<field1>_<field2>` for index names. The drop in
  `down` needs the same name, so keep it predictable.

## Do / Don't

| Do                                                          | Don't                                                              |
|-------------------------------------------------------------|--------------------------------------------------------------------|
| Use `soli generate migration` for naming                    | Hand-pick a timestamp prefix                                        |
| Write a real `down(db)` for every migration                 | Leave `# TODO` or `pass` in `down`                                  |
| Use `db.query("...")` with a plain SDBQL string             | Use `@sdbql{...}` blocks here — that's the model-side DSL, not migrations |
| Use `[[ ... ]]` or `""" ... """` for multi-line SDBQL       | Hand-escape `\"` across long queries — or invent `@"..."` (not a thing) |
| Name indexes consistently (`idx_<field>`)                   | Use anonymous indexes or random names                                |
| Split unrelated schema changes into separate migrations     | Bundle "create posts" + "seed demo data" into one file              |
| Treat applied migrations as immutable                       | Edit a migration after it's been run anywhere                       |
| Roll forward with a new migration to fix a mistake          | Force a re-run by tweaking the tracking collection by hand          |
