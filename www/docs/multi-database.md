# Multiple Databases

Soli can use **one connection for the whole app** (the default) or **several named connections** at once — including **SoliDB and Postgres/MySQL/SQLite in the same process**.

- **Default:** SoliDB via `SOLIDB_*` (full Model surface: graph, vector, timeseries, raw SDBQL).
- **Whole app on SQL:** `SOLI_DB_ADAPTER=postgres|mysql|sqlite` + `DATABASE_URL` (document tables).
- **Cloudflare Workers:** `SOLI_DB_ADAPTER=d1` puts the same document tables on D1 — see [Cloudflare Workers](edge.md#cloudflare-d1). `postgres` works there too, through Hyperdrive and the `pg` driver, with `DATABASE_URL` defaulting to `hyperdrive://HYPERDRIVE` — see [PostgreSQL](edge.md#postgresql-hyperdrive). MySQL does not run on the edge.
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
| `SOLI_DB_ADAPTER` | `solidb` (default), `postgres`, `mysql`, `sqlite`, or `d1` (edge build) | `solidb` |
| `DATABASE_URL` | Required for every SQL adapter; for `d1`, the binding (`d1://DB`, the default); on the edge build, `postgres` defaults to `hyperdrive://HYPERDRIVE` | unset |
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
| `replicas` / `replica` | postgres, mysql | Read replica URLs (an array, or one string) — see [Read replicas](#read-replicas) |
| `replica_delay` | postgres, mysql | Seconds a client that wrote keeps reading the primary (default 2) |

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

Adapter-specific notes: [PostgreSQL](postgres.md) · [MySQL](mysql.md) · [SQLite](sqlite.md).

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
| Hash `.where` (equality, comparisons, IN, LIKE, OR) / order / limit / count / exists | ✓ |
| sum / avg / min / max / count | ✓ |
| `delete_all` / `update_all` | ✓ |
| Soft-delete scope | ✓ |
| `pluck` / `select` (client projection) | ✓ |
| Batched `.includes` (belongs_to / has_many / has_one) | ✓ |
| Relation readers (`owner.posts` and its chains, `post.author`, has_one, HABTM) and `dependent:` cascades | ✓ (after 2.24.0; they raised on SQL before) |
| Multi-row `group_by` + multi-agg | ✓ |
| Batched HABTM `.includes` + `includes_count` | ✓ (two queries: the join table, then the targets) |
| `index` declarations / `soli db:indexes` | ✓ expression index on the JSON field (generated column on MySQL) |
| Atomic `increment` / `decrement` / counter caches | ✓ one arithmetic `UPDATE` (no `_rev`, no retry loop) |
| `Model.find_by_sql(sql, binds?)` | ✓ raw `SELECT` escape hatch, positional binds |
| `create_many` | ✓ one multi-row `INSERT` per 500-row chunk, in one transaction |
| `pluck` / `select` on column models | ✓ pushed into the `SELECT` list |
| Dev bar / `dev_queries()` / N+1 detection | ✓ the SQL, its binds, and its duration are logged per request |
| TLS to the server | ✓ Postgres / MySQL, opportunistic by default and verifiable on request — see [Postgres](postgres.md#tls) and [MySQL](mysql.md#tls) (SQLite is a local file, so there is no connection to encrypt) |
| `soli db:create` / `soli db:drop` | ✓ (SoliDB creates its database on first use instead) |
| `soli db:schema:dump` / `soli db:schema:load` | ✓ writes / applies `db/schema.sql` plus applied migration versions |
| `through:` includes | ✓ three batched queries via the intermediate model |
| `.join` (relation existence filter) | ✓ correlated `EXISTS`, so parents are not duplicated |
| `.having` | ✓ one comparison of a group key or aggregate alias against a number |
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

## Connection pool

Each SQL connection keeps a pool of up to `pool` connections (`SOLI_DB_POOL_SIZE`; 10, or 5 on
SQLite), opened as requests need them. The first opens when the pool is created, so a wrong URL
or an unreachable server fails at boot rather than on the first query, and a refused connection
reports the driver's error at once. When every connection is in use, a query waits up to 10 seconds
for one to come back.

A connection is handed out without a check when it was last used under half a second ago. One
that sat idle longer, or whose last query failed, is pinged first and replaced if it is dead — so
a busy app pays no validation round trip, and a database restart costs at most one failed query
per stale connection.

Two more round trips are saved per query:

- **Statements are prepared once per connection** (Postgres, SQLite), not once per query.
- **A table known to exist is read directly.** A read on a table that may not exist answers empty
  (`find_by` → `nil`, `where` → `[]`); the first read checks, later ones skip the check. A table
  dropped from outside the app still reads as empty: the error is recognised and the check comes
  back for that table. Inside a transaction the check always runs.

## SQLite specifics

The dedicated [SQLite](sqlite.md) page covers URL forms, WAL, jobs, backups,
and the numeric-affinity caveat. The short version:

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
order.save()
order.delete()
Order.transaction(fn() { ... })              # real SQL transaction
```

`pluck` and `select` are **pushed into the `SELECT` list** on column models, so a
projection reads two columns instead of fifty on a wide table; the primary key is
always included so the row stays identifiable. A field that is not a real column
(a nested path, a computed alias) falls back to the client-side projection rather
than failing.

### Raw SQL escape hatch

For a query the portable surface cannot express:

```soli
# Positional binds: $1/$2 on Postgres, ? on MySQL and SQLite.
Order.find_by_sql("SELECT * FROM orders WHERE total > ? AND status = ?", [100, "open"])

# A single `doc` column hydrates as documents (document tables);
# any other shape becomes a hash per row.
Post.find_by_sql("SELECT doc FROM posts WHERE doc->>'slug' = $1", ["hello"])
```

Values are **bound**, never interpolated, so a value that looks like SQL stays a
value. `find_by_sql` is SQL-only; on SoliDB it raises and points at `Model.query`
with SDBQL.

### Also supported

- **Batched eager loading** — `belongs_to`, `has_many`, `has_one`,
  `has_and_belongs_to_many`, `through:`, and `includes_count`, one query per
  association (or two/three hops for HABTM/`through:`) whatever the parent
  count, over the real foreign-key columns. A parent with no children gets
  `[]`, never null. A hash filter on `.includes("rel", { "visible": true })`
  (or `{ "where": { "n": { "gt": 1 } } }`) is applied to the related rows.
- **`group_by`** with `sum`/`avg`/`min`/`max`/`count` over real columns, plus
  **`.having("n > 5")`** and **`.join("comments")`** (a correlated `EXISTS`).
- **`delete_all` / `update_all`** — bulk writes that skip validations and
  callbacks (as on the document path) but still stamp `updated_at` when the table
  has it, and never rewrite the primary key.
- **Atomic `increment` / `decrement` and counter caches** — one arithmetic
  `UPDATE` on the column.
- **`soft_delete`**, provided the table has a `deleted_at` column. Without one
  there is nowhere to record the deletion, so boot fails with that message rather
  than silently returning deleted rows.
- **`encrypts`**, provided each named field is a string/text column. Values are
  AES-256-GCM ciphertext at rest (same as the document path) and plaintext on
  the instance after `create` / `save` / `find`. Boot fails if the column is
  missing or not text.
- **STI** (`class Admin < User` with `table` on the base). Subclass writes stamp
  a `type` string column; subclass queries add `type IN (class, descendants)`.
  Boot fails if an STI subclass's table has no `type` column. `find` /
  `find_by` / `first_by` on a subclass refuse a row of another type.

```soli
class Person < Model
  table "people"         # columns: id, name, ssn, type, timestamps
  encrypts(:ssn)
end

class Admin < Person
end

a = Admin.create({ "name": "Root", "ssn": "000-00-0001" })
a.ssn                    # plaintext
Admin.find_by("name", "Root").type   # "Admin"
Person.where({ "name": "Root" }).count()  # 1 (base matches every type)
Admin.where({ "name": "Guest" }).count()  # 0 if Guest is not an Admin
```

### Not supported on column-aware models

Each of these raises an error naming the feature rather than returning wrong data:

| Feature | Why |
|---|---|
| Raw/string `.where("doc…")` | SDBQL has no meaning against columns; use the hash form |
| `.includes` across storage shapes | Both models must be column-aware — matching a real column against a JSON field is not a join Soli will guess at |
| `.join` on `belongs_to` / HABTM / `through:` | Existence filter needs the child to hold the FK; use `.includes` and filter the related rows |
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
| `grouped {}` coalescing | Same SoliDB connection; on D1, one `batch()` per binding. Reads on other connections run one by one |

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

## One database per tenant

A SaaS keeps its accounts and sites in one database and each site's data in a
database of its own. In Soli that is two connections: the main one, and one
whose `url` — or, on SoliDB, `database` — holds `{tenant}`:

```toml
# config/database.toml
default = "primary"

[connections.primary]            # accounts, sites, jobs, sessions
adapter = "postgres"
url = "${DATABASE_URL}"

[connections.site]               # one database per site
adapter = "postgres"
url = "postgres://app@db.internal/site_{tenant}"
```

The same works on every adapter: `url = "mysql://…/site_{tenant}"`,
`url = "sqlite://db/sites/{tenant}.sqlite3"` (one file per site),
`database = "site_{tenant}"` on SoliDB, `url = "d1://SITE_{tenant}"` on
[Cloudflare D1](edge.md#cloudflare-d1) (one binding per site), and on the edge
`url = "do://SITES/{tenant}"` with the D1 adapter: a
[Durable Object per site](edge.md#sqlite-in-durable-objects-a-database-per-tenant),
created by naming it, with nothing declared per site.

The models of the site data name the connection; the others stay on the main
database:

```soli
# app/models/site.sl — main database
class Site < Model
end

# app/models/post.sl — the current site's database
class Post < Model
  connection "site"
end
```

A request chooses its site, usually in a middleware, from the host or a path
segment:

```soli
# app/middleware/site.sl
# order: 10
def choose_site(req)
  site = Site.find_by("host", req["headers"]["host"])
  halt(404, "Unknown site") if site.nil?

  switch_tenant(site.db_key)
  req
end
```

From then on every query on `site` goes to `site_<db_key>` — `Post.all`,
`Post.create(…)`, query builders, `grouped`, transactions — and the main
database answers the rest. The functions:

| Call | Does |
|------|------|
| `switch_tenant(key)` | The tenant for the rest of the request (or script). `switch_tenant(nil)` clears it. |
| `switch_tenant(key, {"url": u})` | A site whose data lives elsewhere — another server, another credential. `{"database": name}` on SoliDB. The URL replaces the template for that site. |
| `with_tenant(key) do … end` | A block under that tenant (an admin page reading several sites, a script, a console); the tenant before comes back afterwards, also when the block raises. Takes the same options hash. |
| `current_tenant()` | The current key, or `nil`. |

What to know:

- **The key goes into URLs, file names and database names**, so it is held to
  1–63 letters, digits, `_` and `-`, starting with a letter or digit. Anything
  else raises — a key taken from a host name cannot reach another directory or
  database. Store a key on the site row rather than using the host itself.
- **No tenant, no data.** A query on a tenant connection before
  `switch_tenant` fails with an error saying so; it never falls back to another
  site's database. Each request starts and ends with no tenant, so a worker
  that last served one site cannot answer the next request with its data —
  WebSocket and LiveView events start with none too.
- **Jobs run under the tenant they were enqueued under.** `perform_later`
  inside a request for `acme` runs against `acme`'s database, with the explicit
  URL if the site had one (it is stored in the job row, in the main database).
  Cron jobs have no tenant: loop over the sites with `with_tenant`.
- **Keep the default connection the main database.** Jobs, sessions and the
  error tracker live on the default connection; a default connection with
  `{tenant}` in it would scatter them across sites.
- **Pools.** Each site's database gets its own pool. Soli keeps at most
  `SOLI_TENANT_POOLS` (32) site pools open per adapter and closes the oldest
  when one more is needed; that site's next request reopens it. Set `pool` on
  the `site` connection to size each one.
- **SQLite**: site files rely on SQLite's automatic checkpoints, without the
  background checkpointer the main file gets (one thread per file would not
  scale to thousands of sites).
- **SoliDB** creates a site's database on its first write.
- **Column-aware models** (`table "orders"`) on a tenant connection are
  inspected on first use rather than at boot, and the schema read for the first
  site serves the others: every site runs the same migrations.
- `switch_tenant` inside a transaction raises (the transaction holds the
  current site's database), and reads a `grouped` block deferred run before the
  tenant changes.

### Creating and migrating a site's database

A site's database is created explicitly — when the site is created, not on its
first request:

```bash
soli db:create --tenant acme          # CREATE DATABASE site_acme (SQLite: the file)
soli db:migrate up --tenant acme      # the migrations that declare connection "site"
soli db:migrate status --tenant acme
soli db:drop --tenant acme
```

Migrations name their connection (`connection "site"` as their first line). A
plain `soli db:migrate up` runs the main database's and reports the site ones
it left for `--tenant`; `--tenant KEY` runs only the site ones, on that site's
database, which keeps its own `_migrations` table. Migrating every site is a
loop over the keys:

```bash
for key in $(psql "$DATABASE_URL" -tAc "SELECT doc->>'db_key' FROM sites"); do
  soli db:migrate up --tenant "$key"
done
```

`--tenant` is refused on a connection without `{tenant}`, so it cannot act on
the main database by mistake.

## Read replicas

A Postgres or MySQL connection can name read replicas. Reads go to them under
the **`reading`** role; writes always go to the primary.

```toml
[connections.primary]
adapter = "postgres"
url = "${DATABASE_URL}"
replicas = ["${REPLICA_URL:-}"]   # or replica = "…"; several are spread across requests
replica_delay = 2                 # seconds a client that wrote keeps reading the primary
```

An entry that expands to an empty string is ignored, so `${REPLICA_URL:-}`
turns replicas off in an environment that has none. `replicas` on a SQLite,
D1 or SoliDB connection is refused at boot, and so is any replica on the edge
(Cloudflare Workers) build.

### Which role a query runs under

| Where | Role |
|-------|------|
| A `GET` or `HEAD` request | `reading` |
| The same, from a client that wrote less than `replica_delay` seconds ago | `writing` |
| Any other request (`POST`, `PATCH`, `DELETE`, …) | `writing` |
| Jobs, socket events, the console, migrations, scripts | `writing` |
| Inside `with_role("reading") do … end` / `with_role("writing") do … end` | the one named |

Under `reading`, only the reads of a model or query go to a replica: `all`,
`find`, `find_by`, `where`, `count`, `exists`, the aggregates, `group_by`,
eager loads, and `find_by_sql` with a statement that starts with `SELECT`.
Everything else stays on the primary whatever the role: writes, DDL,
migrations, `db.execute`, a `find_by_sql` that starts with `WITH`, and every
read inside a transaction.

```soli
# GET /reports/monthly — a GET reads a replica already
def monthly(req)
  @orders = Order.where({ month: params["month"] }).all
end

# Reads that must see the latest data, even on a GET
with_role("writing") do
  @balance = Account.find(id).balance
end

# A slow report from a POST or a job, kept off the primary
with_role("reading") do
  @rows = Event.where({ kind: "signup" }).count
end

current_role()   # "reading" or "writing"
```

### Reading what you wrote

A replica lags the primary, so two rules keep a client from reading data older
than its own writes:

- **Within a request**, the first write sends the request's later reads to the
  primary. A `GET` that writes (a view counter, a `last_seen_at`) reads the
  primary from then on.
- **Across requests**, a request that wrote sets a `soli_db_primary` cookie
  (`HttpOnly`, `SameSite=Lax`, lasting `replica_delay` seconds), and that
  client's `GET`s run under `writing` until it expires — so the page a `POST`
  redirects to shows what was just saved. The cookie is not signed: forging it
  only sends that client's own reads to the primary.

An explicit `with_role("reading")` block reads a replica even after a write:
you asked for it. Unlike Rails, a write under `reading` is not refused — it
goes to the primary.

### What to know

- **Picking a replica.** Each request reads one replica, taken in turn across
  requests. A replica that does not answer fails its reads; it is not taken out
  of rotation on its own (put a proxy such as PgBouncer or ProxySQL in front if
  you need failover).
- **Pools.** Each replica gets its own pool, sized by the connection's `pool`.
- **One database per tenant.** Replica URLs may hold `{tenant}` too
  (`replicas = ["postgres://app@replica/site_{tenant}"]`); a tenant switched to
  with its own `url` reads its primary.
- **`grouped` blocks** keep their reads under the role they were made under:
  `with_role` runs a block's deferred reads before the role changes.
- **Seeing it.** Under `--dev`, the dev bar's DB panel shows the request's
  role in its header and tags each query `replica` or `primary`;
  `dev_queries()` entries carry `replica: true/false`.

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
- A connection with one database per tenant is migrated with `--tenant KEY` —
  see [Creating and migrating a site's database](#creating-and-migrating-a-sites-database).

## Not yet / limitations

- **Per-connection SoliDB hosts:** registry stores host fields; full multi-host SoliDB routing is still completing — prefer one SoliDB endpoint plus SQL secondaries for now. A mismatch with the env values is rejected at boot. The exception is a SoliDB connection with one database per tenant (`database = "site_{tenant}"`): its database follows the tenant, on the `SOLIDB_HOST` server.
- **Column-aware models:** composite primary keys are still refused at boot.
  `encrypts` (text columns) and STI (string `type` column) are supported —
  see [Column-aware models](#column-aware-models-existing-databases).
- **Read replicas:** no automatic failover or replication-lag measurement;
  sharding (several primaries for one connection) is not supported.
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
