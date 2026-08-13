# Multiple Databases

Soli can use **one connection for the whole app** (the default) or **several named connections** at once — including **SoliDB and Postgres/MySQL/SQLite in the same process**.

- **Default:** SoliDB via `SOLIDB_*` (full Model surface: graph, vector, timeseries, raw SDBQL).
- **Whole app on SQL:** `SOLI_DB_ADAPTER=postgres|mysql|sqlite` + `DATABASE_URL` (document tables).
- **Multi-DB:** `config/database.toml` + per-model `connection "name"`.

Design notes and the full capability matrix: repo `docs/sql-adapter-design.md`. Narrative: [Multiple Databases in One Soli App](/docs/blog/multi-database).

## Single connection (no TOML)

If `config/database.toml` is absent, Soli builds one connection named **`primary`** from the environment:

| Variable | Purpose | Default |
|----------|---------|---------|
| *(unset adapter)* | SoliDB | — |
| `SOLIDB_HOST` | SoliDB URL | `http://localhost:6745` |
| `SOLIDB_DATABASE` | SoliDB database name | `default` |
| `SOLIDB_USERNAME` / `SOLIDB_PASSWORD` | Basic auth | unset |
| `SOLIDB_API_KEY` | API key auth | unset |
| `SOLI_DB_ADAPTER` | `solidb` (default), `postgres`, `mysql`, or `sqlite` | `solidb` |
| `DATABASE_URL` | Required for every SQL adapter | unset |
| `SOLI_DB_POOL_SIZE` | SQL pool size | `10` (`5` on SQLite) |

```bash
# SoliDB (default)
SOLIDB_HOST=http://localhost:6745
SOLIDB_DATABASE=myapp

# Or: whole app on Postgres document tables
SOLI_DB_ADAPTER=postgres
DATABASE_URL=postgres://user:pass@localhost:5432/myapp

# Or: whole app on a single SQLite file — no server to run
SOLI_DB_ADAPTER=sqlite
DATABASE_URL=sqlite://db/app.sqlite3
```

See also [Database Configuration](database.md) and [Configuration](configuration.md).

## `config/database.toml`

Place the file under the app root (same level as `config/routes.sl`). It is loaded at `soli serve` / migrate / import after `.env`.

```toml
# config/database.toml
default = "primary"

[connections.primary]
adapter = "solidb"          # solidb | postgres | mysql | sqlite
host = "${SOLIDB_HOST:-http://localhost:6745}"
database = "${SOLIDB_DATABASE:-default}"
username = "${SOLIDB_USERNAME:-}"
password = "${SOLIDB_PASSWORD:-}"
# api_key = "${SOLIDB_API_KEY:-}"

[connections.legacy]
adapter = "postgres"
url = "${LEGACY_DATABASE_URL}"
pool = 10

[connections.warehouse]
adapter = "mysql"
url = "${WAREHOUSE_DATABASE_URL}"
pool = 5

[connections.analytics]
adapter = "sqlite"
url = "sqlite://db/analytics.sqlite3"   # a path, not a server
```

### Field reference

| Field | Adapters | Description |
|-------|----------|-------------|
| `default` | — | Connection name for models without `connection "…"` |
| `adapter` | all | `solidb`, `postgres`, `mysql`, or `sqlite` (aliases: `pg`, `postgresql`, `mariadb`, `sqlite3`, …) |
| `host` | solidb | SoliDB base URL |
| `database` | solidb | SoliDB database name |
| `username` / `password` | solidb | Optional basic auth |
| `api_key` | solidb | Optional API key |
| `url` | postgres, mysql, sqlite | Connection string (`postgres://…`, `mysql://…`) or a SQLite path (`sqlite://db/app.sqlite3`) — **required** for SQL |
| `pool` | postgres, mysql, sqlite | Pool size (default 10; 5 on SQLite, forced to 1 for `:memory:`) |

> **SoliDB connections and env**: SoliDB requests are always routed to the
> env-configured `SOLIDB_HOST` / `SOLIDB_DATABASE`. A `solidb` connection's
> `host`/`database` must therefore match the env values (use the
> `${SOLIDB_HOST:-…}` expansion shown above) — a mismatch is rejected at boot
> rather than silently sending traffic to the wrong server. Per-connection
> SoliDB targets (two different SoliDB servers/databases in one app) are not
> supported yet; secondary connections must use a SQL adapter.

### Environment expansion

Values support `${VAR}` and `${VAR:-default}` so secrets stay in `.env` while topology stays in git:

```toml
url = "${LEGACY_DATABASE_URL}"
host = "${SOLIDB_HOST:-http://localhost:6745}"
```

### Precedence

1. If `config/database.toml` exists → **named registry** (file is source of truth for names/adapters).
2. Else → env-only **`primary`** (today’s single-connection behaviour).
3. Do not invent a second parallel multi-DB story via env alone.

## Per-model `connection`

```soli
class User < Model
  # uses default ("primary")
end

class LegacyOrder < Model
  connection "legacy"
end

class FactSale < Model
  connection "warehouse"
end
```

- Accepts a **string** or **symbol** name.
- Validated at class load against the registry (unknown name → error listing known connections).
- Stored on model metadata and on the collection so CRUD routes correctly.
- STI subclasses **inherit** the parent connection unless they redeclare `connection`.

## SQL document backends

When a connection uses `adapter = "postgres"`, `"mysql"`, or `"sqlite"`, Model data is stored as:

```text
table <collection>
  _key  TEXT / VARCHAR PRIMARY KEY
  doc   JSONB / JSON / TEXT   -- full Soli document
```

SQLite has no JSON *type* — it stores JSON as text and reads it with the json1
functions (`->>`, `json_patch`, `json_set`), which every write goes through. The
Model surface is the same on all three.

Portable surface (hash filters, not raw SDBQL):

| Capability | Support |
|------------|---------|
| CRUD, validations, callbacks | ✓ |
| Hash `.where` / order / limit / count / exists | ✓ |
| sum / avg / min / max / count | ✓ |
| `delete_all` / `update_all` | ✓ |
| Soft-delete scope | ✓ |
| `pluck` / `select` (client projection) | ✓ |
| Batched `.includes` (belongs_to / has_many / has_one) | ✓ |
| Multi-row `group_by` + multi-agg | ✓ |
| Batched HABTM `.includes` + `includes_count` | ✓ (two queries: the join table, then the targets) |
| `index` declarations / `soli db:indexes` | ✓ expression index on the JSON field (generated column on MySQL) |
| Atomic `increment` / `decrement` / counter caches | ✓ one arithmetic `UPDATE` (no `_rev`, no retry loop) |
| Dev bar / `dev_queries()` / N+1 detection | ✓ the SQL, its binds, and its duration are logged per request |
| `soli db:create` / `soli db:drop` | ✓ (SoliDB creates its database on first use instead) |
| `through:` includes, `.having`, `.join` | ✗ SoliDB-only (`through:` on SQL planned) |
| Graph, vector, columnar, timeseries | ✗ SoliDB-only |
| `Model.transaction` | ✓ (holds one SQL pool connection for the block) |
| Raw SDBQL / string `.where("doc…")` | ✗ SoliDB-only |

Tables are auto-created on first write. Migrations can use `create_table` / `drop_table` on SQL connections (version table `_migrations`).

### Import SoliDB → SQL

```bash
# Destination = active SQL adapter (env primary or TOML default)
SOLI_DB_ADAPTER=postgres DATABASE_URL=postgres://…
SOLIDB_HOST=… SOLIDB_USERNAME=… SOLIDB_PASSWORD=…
soli db:import              # all non-_ collections
soli db:import posts users  # named only
```

Each document is upserted as `_key` + `doc`.

## SQLite specifics

SQLite is a file, not a server: there is nothing to install, start, or
credential. That makes it the shortest path from `soli new` to persistent data,
and a good fit for single-node apps, embedded/desktop builds, CI, and tests.

### URL forms

| URL | Meaning |
|-----|---------|
| `sqlite://db/app.sqlite3` | Relative path (the directory is created if missing) |
| `sqlite:///var/lib/app/app.db` | Absolute path |
| `sqlite:app.db` | Path, short form |
| `sqlite::memory:` or `:memory:` | Private in-memory database, gone at exit |
| `./app.db` | A bare path works too |

Every connection is opened in **WAL** mode with a 10-second busy timeout and
foreign keys on.

### What to know before you choose it

- **One writer at a time.** WAL lets readers run during a write, but writers
  serialize. A busy write path across many processes belongs on Postgres.
- **No exact numeric type.** A `DECIMAL`/`NUMERIC` column has NUMERIC
  *affinity*, not an exact type: SQLite converts the value to a `REAL`, so a
  stored `19.90` reads back as `19.9` and a value beyond f64's exact range loses
  precision on write, not just on read. Postgres `numeric` and MySQL `decimal`
  keep the scale. If exact decimal text matters on SQLite, declare the column
  `TEXT` and treat it as a string in Soli.
- **Backups are file copies** — use `.backup` / `VACUUM INTO`, not a `cp` of a
  live WAL database.
- **`:memory:` is one connection.** The pool is forced to a single connection,
  because a second one would be a second, empty database.

### Background jobs on SQLite

Jobs work unchanged. Postgres claims with `SKIP LOCKED` and MySQL with a claim
token; SQLite takes the database write lock (`BEGIN IMMEDIATE`) for the length of
the claim, which is exclusive by construction. Leases, retries, and cron behave
the same. See [Background Jobs](jobs.md).

## Column-aware models (existing databases)

The document backend above stores every collection as `_key` + `doc`, which means it only reads tables Soli created. To use Soli against a database that **already exists** — a legacy app's schema, a warehouse table, anything with real columns — declare the physical table on the model:

```soli
# app/models/order.sl
class Order < Model
  connection "legacy"    # a postgres, mysql, or sqlite connection
  table "orders"         # bind to an existing table -> column mode
end
```

`table "…"` is what switches the model into **column mode**. The model then reads and writes the table's real columns, and never creates or alters it.

The table can be one you already have, or one a **migration** builds: `create_table("orders", { … })` declares real columns portably across Postgres, MySQL, and SQLite, and the types it emits are the ones introspection reads back — see [Migrations → On the SQL adapters](migrations.md#on-the-sql-adapters).

### What Soli learns, and when

At boot, Soli introspects each declared table once and caches the result (`information_schema` on Postgres and MySQL, `PRAGMA table_info` on SQLite):

- every column, its type, and whether it is nullable;
- the **primary key**, detected automatically, including whether the database generates it (`BIGSERIAL` / `IDENTITY` / `AUTO_INCREMENT`), so inserts leave it to the database;
- whether `created_at` / `updated_at` exist — they are stamped only if they do.

Problems fail the boot with a message naming the connection and table: a missing table, a **composite** primary key (not supported yet), no primary key at all, or a `solidb` connection. Editing a model in `--dev` re-introspects; an `ALTER TABLE` while the server runs needs a restart.

### Type mapping

| PostgreSQL | MySQL | SQLite (declared type) | Soli |
|---|---|---|---|
| `int2` / `int4` / `int8` | `smallint` / `mediumint` / `int` / `bigint` | anything containing `INT` | Int |
| `float4` / `float8` | `float` / `double` | `REAL`, `FLOAT`, `DOUBLE` | Float |
| `numeric` | `decimal` | `DECIMAL`, `NUMERIC`, `MONEY` | Float (see caveat) |
| `bool` | `tinyint(1)`, `boolean`, `bit(1)` | `BOOLEAN` | Bool |
| `text` / `varchar` / `char` / `citext` | `char` / `varchar` / `text` / `enum` / `set` | `TEXT`, `VARCHAR`, `CHAR`, `CLOB`, no declared type | String |
| `uuid` | — | `UUID`, `GUID` | String |
| `date`, `timestamp`, `timestamptz` | `date`, `datetime`, `timestamp` | `DATE`, `DATETIME`, `TIMESTAMP` | DateTime (native) |
| `json` / `jsonb` | `json` | `JSON` | Hash / Array |
| anything else (`bytea`, arrays, geometry, unsigned `bigint`) | `blob`, geometry, unsigned `bigint` | `BLOB` | unsupported |

- **Exact numerics** (`numeric`/`decimal`) travel as text and are read as Float, so a value beyond f64's exact range loses precision on read. They are *written* as text, so a stored value keeps its scale.
- **Unsupported columns** are skipped on read (they come back absent) and error clearly if you try to filter or write them — they never silently corrupt a row.
- **MySQL has no timezone-aware timestamp**: `datetime`/`timestamp` values are interpreted as UTC.
- **SQLite enforces no types.** It applies *affinity* to the declared type, and any column can hold any value. Soli reads the declared type (`PRAGMA table_info`) to decide how to convert, and reads each value by what it actually is — so an `INTEGER` stored in a `TEXT` column still comes back as a number. A `DATETIME` column holding a unix timestamp (seconds or milliseconds) is converted to RFC 3339, like a stored text date is.
- **A SQLite `INTEGER PRIMARY KEY` is generated** — it aliases the rowid, with or without `AUTOINCREMENT` — so inserts omit it. A key of any other type must be supplied.

### Supported operations

```soli
order = Order.find(42)                       # real primary key, Int or String
Order.find_by("email", "a@b.c")              # equality on a real column
Order.where({ "status": "open" }).all
Order.where({ "assignee_id": null }).all     # compiles to IS NULL
Order.where({ "status": "open" }).order("created_at", "desc").limit(20).all
Order.where({ "status": "open" }).count
Order.where({ "status": "open" }).exists
Order.sum("total").all                       # sum/avg/min/max on numeric columns
Order.create({ "name": "Ada", "total": "19.99" })
order.status = "closed"
order.save
order.delete
Order.transaction(fn() { ... })              # real SQL transaction
```

`pluck` and `select` work too (projection happens client-side, as on the document path).

### Also supported

- **Batched eager loading** — `belongs_to`, `has_many`, `has_one`, and
  `includes_count`, one query per association whatever the parent count, over the
  real foreign-key columns. A parent with no children gets `[]`, never null.
- **`group_by`** with `sum`/`avg`/`min`/`max`/`count` over real columns.
- **`delete_all` / `update_all`** — bulk writes that skip validations and
  callbacks (as on the document path) but still stamp `updated_at` when the table
  has it, and never rewrite the primary key.
- **Atomic `increment` / `decrement` and counter caches** — one arithmetic
  `UPDATE` on the column.
- **`soft_delete`**, provided the table has a `deleted_at` column. Without one
  there is nowhere to record the deletion, so boot fails with that message rather
  than silently returning deleted rows.

### Not supported on column-aware models

Each of these raises an error naming the feature rather than returning wrong data:

| Feature | Why |
|---|---|
| Raw/string `.where("doc…")` | SDBQL has no meaning against columns; use the hash form |
| `.includes` across storage shapes | Both models must be column-aware — matching a real column against a JSON field is not a join Soli will guess at |
| `.includes` on `has_and_belongs_to_many`, `through:` | Needs the join table mapped as a column model too — planned |
| `.having`, `.join` | Planned |
| `encrypts`, STI | Assume Soli-managed document storage; declaring one alongside `table` fails at boot |
| Composite primary keys | Key handling is single-column throughout; refused at boot with the columns named |
| `grouped {}` coalescing, graph, vector, columnar, timeseries | SoliDB features |
| Auto-create / index sync / implicit `ALTER` | The schema is Soli's only where you wrote it: **models** never issue DDL in column mode. A [migration](migrations.md#on-the-sql-adapters) can create and alter column tables explicitly |

Doc-store models on the same connection keep working exactly as before; column mode is per model.

### Getting data in

If you would rather migrate a SoliDB collection into a table Soli manages, use `soli db:import` (below) and skip column mode entirely.

For a **new** schema, write a migration with a column hash: you get real columns without leaving Soli. Column mode also still does what it was built for — adopting a schema you cannot or do not want to change.

## Cross-connection rules

| Operation | Rule |
|-----------|------|
| `.includes` when models use different connections | **Error** (clear message with both names) |
| HABTM / through spanning connections | **Error** |
| Storing a foreign key string/id across DBs | Allowed; no integrity guarantees |
| `Model.transaction` | Begins on the receiving model's connection (SoliDB or SQL); covers that connection only — writes to other connections run outside the tx (SoliDB) or are refused (SQL model inside a SoliDB tx) |
| `grouped {}` coalescing | Same SoliDB connection only |

Do not expect distributed joins or two-phase commit. Place related aggregates on the **same** connection when you need eager loads.

## Hybrid example

```toml
default = "primary"

[connections.primary]
adapter = "solidb"
host = "${SOLIDB_HOST:-http://localhost:6745}"
database = "${SOLIDB_DATABASE:-myapp}"

[connections.legacy]
adapter = "postgres"
url = "${LEGACY_DATABASE_URL}"
```

```soli
# app/models/user.sl
class User < Model
  has_many "posts"
end

# app/models/legacy_order.sl
class LegacyOrder < Model
  connection "legacy"
end
```

```soli
# Controllers use the same Model API
def index
  @users = User.limit(20).all
  @orders = LegacyOrder.where({ "status": "open" }).limit(20).all
  render("home/index")
end
```

```soli
# This raises — different connections
LegacyOrder.includes("user").all
```

## Migrations and multi-DB

- Default migrate target is the **default** connection (env / `database.toml` `default`).
- Target a named connection:

```bash
soli db:migrate up --connection legacy
soli db:migrate status -c legacy
soli db:migrate down --connection legacy
```

- Prefer documenting which connection a migration belongs to in the filename/comment when multi-DB is in use.
- SQL secondaries are fully supported. SoliDB still uses process env host/credentials for the SoliDB HTTP client today.

## Not yet / limitations

- **Per-connection SoliDB hosts:** registry stores host fields; full multi-host SoliDB routing is still completing — prefer one SoliDB endpoint plus SQL secondaries for now. A mismatch with the env values is rejected at boot.
- **Column-aware models:** associations/`.includes`, `group_by`, `delete_all`/`update_all`, and composite primary keys are not implemented yet — see [Column-aware models](#column-aware-models-existing-databases).
- **Request-scoped roles** (read replica / `writing`/`reading`) — not v1.
- **pgvector** on document tables — SoliDB-only (or a later design).

## Compile-time features

Postgres, MySQL, and SQLite client code is optional at **build time** (all on by
default). A binary built without an adapter cannot open it — boot fails with a
rebuild hint if `SOLI_DB_ADAPTER` or `database.toml` selects one that was not
compiled in.

```bash
# SoliDB only (no SQL client crates)
cargo install --path . --locked --no-default-features \
  --features embedding,llm,codegraph

# Postgres only (no MySQL, no SQLite)
cargo install --path . --locked --no-default-features \
  --features embedding,llm,codegraph,postgres

# SQLite only — the client is bundled, so nothing to install on the host
cargo install --path . --locked --no-default-features \
  --features embedding,llm,codegraph,sqlite
```

Full table: [Configuration → Slim binary](configuration.md#slim-binary-cargo-features).

## Troubleshooting

| Symptom | Check |
|---------|--------|
| `Unknown database connection "…"` | Name matches `[connections.*]` / `default` |
| `url` required for postgres/mysql/sqlite | Set `url =` or expand `${…}` from `.env` |
| SQL features missing | Confirm model’s connection adapter is `postgres`/`mysql`/`sqlite`; see matrix above |
| `not compiled into this soli binary` | Rebuild with `--features postgres`, `mysql`, and/or `sqlite` (see [Slim binary](configuration.md#slim-binary-cargo-features)) |
| `database is locked` on SQLite | Another writer held the lock past the 10s busy timeout. Shorten write transactions, or move to Postgres for write-heavy multi-process work |
| SQLite file not created | The parent directory is created automatically; check the process can write there, and that the path is not read-only |
| Includes error across DBs | Expected — query each side separately or colocate models |

## Related

- [Database Configuration](database.md) — SoliDB env basics  
- [Models](models.md) — ORM surface  
- [Configuration](configuration.md) — all env vars, slim Cargo features  
- [Blog: Multiple Databases](/docs/blog/multi-database)  
- Design: `docs/sql-adapter-design.md`  
