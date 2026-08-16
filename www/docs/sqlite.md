# SQLite

SQLite is a file, not a server: there is nothing to install, start, or
credential. That makes it the shortest path from `soli new` to persistent data,
and a good fit for single-node apps, embedded/desktop builds, CI, and tests.

The Model surface is the **same** as [Postgres](postgres.md) and
[MySQL](mysql.md) — hash `.where`, associations, aggregates, migrations, jobs —
stored either as `_key` + JSON `doc` tables or as a
[column-aware](multi-database.md#column-aware-models-existing-databases)
mapping onto a real schema.

The client is **bundled** into the binary (no system `libsqlite3`). The pin is
new enough for the `->>` JSON operator (3.38+) and `RETURNING` (3.35+).

## Connect

Whole app on a file:

```bash
SOLI_DB_ADAPTER=sqlite
DATABASE_URL=sqlite://db/app.sqlite3
# optional; default 5, forced to 1 for :memory:
SOLI_DB_POOL_SIZE=5
```

Aliases: `sqlite`, `sqlite3`.

A named connection:

```toml
# config/database.toml
[connections.analytics]
adapter = "sqlite"
url = "sqlite://db/analytics.sqlite3"
```

```soli
class Event < Model
  connection "analytics"
end
```

The `sqlite` Cargo feature is on by default.

## URL forms

| URL | Meaning |
|-----|---------|
| `sqlite://db/app.sqlite3` | Relative path (the directory is created if missing) |
| `sqlite:///var/lib/app/app.db` | Absolute path |
| `sqlite:app.db` | Path, short form |
| `sqlite::memory:` or `:memory:` | Private in-memory database, gone at exit |
| `./app.db` | A bare path works too |

A query suffix (`?mode=rwc`) is stripped from the path.

Every connection opens in **WAL** mode with a 10-second busy timeout and
foreign keys on. A transaction is `BEGIN IMMEDIATE` so it cannot fail to
upgrade a read into a write.

`:memory:` pins the pool to **one** connection — a second one would be a
second, empty database.

## Create and drop

```bash
soli db:create              # create the file and its parent directory
soli db:migrate up
soli db:schema:dump
soli db:drop                # delete the file plus -wal / -shm sidecars
```

`db:drop` removes the `-wal` and `-shm` files too: leaving them would
resurrect committed data into the next file of the same name. `:memory:`
needs no file — create/drop is a no-op. Both accept `--connection NAME`.

## Document tables

Each collection is:

```text
table <collection>
  _key  TEXT PRIMARY KEY
  doc   TEXT NOT NULL        -- JSON, read with json1
```

SQLite has no JSON *type*. Soli stores JSON as text and reads it with json1
(`->>`, `json_patch`, `json_set`), which every write goes through. String
equality in `.where({ "status": "open" })` compares on the text extract,
which is what a document index holds.

Placeholders in raw SQL are `?`.

## Indexes

`index "status"` (or `soli db:indexes`) creates an expression index:

```sql
CREATE INDEX IF NOT EXISTS idx_posts_status ON posts ((doc ->> '$.status'))
```

The planner uses that index when the predicate is the same expression.
Multi-field and `unique:` work the same way. Numbers and booleans keep JSON
comparison and are not covered by that index.

## Background jobs

The queue lives in `_jobs` in the same file. SQLite has no `SKIP LOCKED`, so
a claim takes the database write lock (`BEGIN IMMEDIATE`) for the length of
the claim — exclusive by construction. Leases, retries, backoff, and
single-winner cron firing behave like the other adapters. See [Jobs](jobs.md).

A busy write path across **many processes** belongs on Postgres: WAL lets
readers run during a write, but writers serialize.

## Schema dump

```bash
soli db:schema:dump         # writes db/schema.sql
soli db:schema:load         # recreate a fresh database from that file
```

The dump is `sqlite_master` (`CREATE TABLE` / `CREATE INDEX` as originally
executed), plus a `-- versions:` header so load records applied migrations
without replaying every file.

## Column-aware models

```soli
class Order < Model
  table "orders"
end
```

Introspection uses `PRAGMA table_info`. An `INTEGER PRIMARY KEY` is generated
(it aliases the rowid, with or without `AUTOINCREMENT`); a key of any other
type must be supplied.

SQLite **enforces no types**. It applies *affinity* to the declared type, and
any column can hold any value. Soli reads the declared type to decide how to
convert, and reads each value by what it actually is — so an `INTEGER` stored
in a `TEXT` column still comes back as a number. A `DATETIME` column holding a
unix timestamp (seconds or milliseconds) is converted to RFC 3339.

| Declared type | Soli |
|---------------|------|
| anything containing `INT` | Int |
| `REAL`, `FLOAT`, `DOUBLE` | Float |
| `DECIMAL`, `NUMERIC`, `MONEY` | Float (see caveat) |
| `BOOLEAN` | Bool |
| `TEXT`, `VARCHAR`, `CHAR`, `CLOB`, no type | String |
| `UUID`, `GUID` | String |
| `DATE`, `DATETIME`, `TIMESTAMP` | DateTime |
| `JSON` | Hash / Array |
| `BLOB` | unsupported |

**No exact numeric type.** A `DECIMAL` column has NUMERIC *affinity*, not an
exact type: SQLite stores the value as a `REAL`, so `19.90` reads back as
`19.9` and a value beyond f64's exact range loses precision on write. Postgres
`numeric` and MySQL `decimal` keep the scale. If exact decimal text matters,
declare the column `TEXT`.

There is no native `ILIKE`. Hash `.where({ "email": { "ilike": "%@x.com" } })`
compiles to `LOWER("email") LIKE LOWER(?)`.

Migrations cannot add a `UNIQUE` or `NOT NULL`-without-default column to an
existing table — SQLite says so, with the way around it.

## Backups

Copy a **live** WAL database with `.backup` or `VACUUM INTO`, not `cp`. A
plain copy can miss frames still in the WAL.

## Honest limits

- **One writer at a time.** Fine for a single node; not a substitute for
  Postgres under multi-process write load.
- **No exact numeric type.** See above.
- **No graph, vector, fulltext, geo, columnar, timeseries, or raw SDBQL.**
  Those stay on SoliDB. Use `Model.find_by_sql` for SQL the portable surface
  cannot express.
- **`grouped {}` read-coalescing is SoliDB-only.**
- **Composite primary keys** are refused at boot on column-aware models.
- **`encrypts` and STI** work in column mode: encrypted fields must be text
  columns; STI subclasses need a string `type` column.

The shared portable surface (hash filters, `.join`, `.having`, includes,
HABTM / `through:`) is documented under [Multiple Databases](multi-database.md#sql-document-backends).

## See also

- [PostgreSQL](postgres.md) / [MySQL](mysql.md)
- [Multiple Databases](multi-database.md) — named connections and the capability matrix
- [Migrations](migrations.md#on-the-sql-adapters) — portable column DDL
- [Models](models.md) — the ORM surface
