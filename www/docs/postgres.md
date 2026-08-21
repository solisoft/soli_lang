# PostgreSQL

Postgres is the SQL adapter for a multi-process, write-heavy, or already-Postgres
app. The Model surface is the same as [MySQL](mysql.md) and [SQLite](sqlite.md)
— hash `.where`, associations, aggregates, migrations, jobs — stored either as
`_key` + JSONB `doc` tables or as a [column-aware](multi-database.md#column-aware-models-existing-databases)
mapping onto a real schema.

SoliDB remains the full stack (graph, vector, timeseries, raw SDBQL). Use
Postgres when you want SQL, an existing cluster, or to sit beside SoliDB via
[`config/database.toml`](multi-database.md).

## Connect

Whole app on Postgres:

```bash
SOLI_DB_ADAPTER=postgres
DATABASE_URL=postgres://user:pass@localhost:5432/myapp
# optional
SOLI_DB_POOL_SIZE=10
```

Aliases: `postgres`, `postgresql`, `pg`.

A named connection:

```toml
# config/database.toml
default = "primary"

[connections.primary]
adapter = "postgres"
url = "${DATABASE_URL}"
pool = 10
```

```soli
class Order < Model
  connection "primary"
end
```

The `postgres` Cargo feature is on by default. A binary built without it refuses
to open a Postgres connection at boot and tells you to rebuild.

## URL

```text
postgres://USER:PASSWORD@HOST:5432/DATABASE
postgres://USER:PASSWORD@HOST:5432/DATABASE?sslmode=require
```

The query string is kept on the URL (so `sslmode` and friends survive
`db:create`, which rewrites only the database name).

Placeholders in raw SQL are `$1`, `$2`, … — see [`Model.find_by_sql`](multi-database.md#raw-sql-escape-hatch).

## TLS

The client speaks TLS — rustls with the `ring` provider, compiled in, so there
is no system OpenSSL to install and a cross-compiled binary keeps it. The modes
are libpq's, spelled the libpq way:

| `sslmode` | Encrypts | Verifies the chain | Verifies the hostname |
|-----------|----------|--------------------|-----------------------|
| `disable` | no | — | — |
| `prefer` **(default)** | when the server offers it | no | no |
| `require` | yes | no | no |
| `verify-ca` | yes | yes | no |
| `verify-full` | yes | yes | yes |

```bash
# managed Postgres — the mode to use
DATABASE_URL=postgres://user:pass@db.example.com:5432/myapp?sslmode=verify-full

# a private CA instead of the built-in Mozilla roots
DATABASE_URL=postgres://user:pass@db.internal:5432/myapp?sslmode=verify-full&sslrootcert=/etc/ssl/rds-ca.pem
```

- **`prefer` is the default**, as in libpq: a server that offers TLS gets an
  encrypted connection with no configuration at all, and a server that does not
  still connects. `sslmode=disable` asks for cleartext explicitly.
- **Encryption and identity are separate rungs**, also as in libpq: `require`
  encrypts but never checks *who* answered. For a managed database use
  `verify-full` — that is the mode an impostor cannot satisfy.
- `sslrootcert` **replaces** the built-in roots rather than adding to them, and
  a CA file supplied with a mode that would not consult it is refused, not
  silently ignored.
- A mandatory mode fails at boot naming the reason, e.g. `connection "primary"
  asked for sslmode=require: error performing TLS handshake: server does not
  support TLS`.
- Postgres never negotiates TLS over a Unix socket, so `require` and up fail on
  a socket URL rather than pretending the connection is encrypted.
- `verify-ca` skips only the hostname check — useful behind a proxy or when the
  URL names an IP the certificate does not.

## Create and drop

A SQL server does not create the database on first use. On a fresh target:

```bash
soli db:create              # CREATE DATABASE via the `postgres` maintenance DB
soli db:migrate up
soli db:schema:dump         # optional snapshot of db/schema.sql
soli db:drop                # DROP DATABASE IF EXISTS
```

`db:create` connects to the `postgres` database on the same server, then
`CREATE DATABASE` the name from the URL. If it already exists (SQLSTATE `42P04`)
the command succeeds. Both accept `--connection NAME`.

## Document tables

Each collection is:

```text
table <collection>
  _key  TEXT PRIMARY KEY
  doc   JSONB NOT NULL
```

The table is created on first write. JSON operators are native (`doc->>'status'`,
`jsonb_set`, `||` merge). String equality in `.where({ "status": "open" })`
compares on the **text extract**, which is what a document index holds.

## Indexes

`index "status"` (or `soli db:indexes`) creates an expression index:

```sql
CREATE INDEX IF NOT EXISTS idx_posts_status ON posts ((doc->>'status'))
```

Multi-field and `unique:` work the same way. Reconciliation is idempotent by
name. Numbers and booleans keep JSON comparison (`10` matches a stored `10.0`)
and are **not** covered by that expression index.

Migrations can index a document field with a `doc.` prefix:

```soli
db.add_index("posts", ["doc.status"], { "unique": true })
```

`fulltext` / `vector_index` / `geo_index` are reported as skipped on SQL, not
as adapter errors.

## Background jobs

The queue lives in `_jobs` on the same connection. Claim uses
`FOR UPDATE SKIP LOCKED`, so several workers can poll without blocking each
other. On first enqueue Soli indexes `state`, `run_at`, and `priority` (plus
`next_run_at` / `enabled` for cron). See [Jobs](jobs.md).

## Schema dump

```bash
soli db:schema:dump         # writes db/schema.sql
soli db:schema:load         # recreate a fresh database from that file
```

The dump reconstructs `CREATE TABLE` from introspection (`pg_tables` + column
types) and appends `pg_indexes` definitions, plus a `-- versions:` header so
load records applied migrations without replaying every file. Defaults and
constraints that introspection does not reconstruct may be missing — prefer
replaying migrations when those matter.

## Column-aware models

```soli
class Order < Model
  connection "legacy"
  table "orders"
end
```

Introspection uses `information_schema`. Generated keys (`BIGSERIAL`,
`IDENTITY`) are left to the database on insert. Types Soli maps:

| PostgreSQL | Soli |
|------------|------|
| `int2` / `int4` / `int8` | Int |
| `float4` / `float8` | Float |
| `numeric` | Float (written as text; values beyond f64 lose precision on **read**) |
| `bool` | Bool |
| `text` / `varchar` / `citext` | String |
| `uuid` | String |
| `date` / `timestamp` / `timestamptz` | DateTime |
| `json` / `jsonb` | Hash / Array |
| `bytea`, arrays, geometry | unsupported (skipped on read; error if filtered or written) |

`ILIKE` is native. Hash `.where({ "email": { "ilike": "%@x.com" } })` compiles to
`email ILIKE $n`.

## Honest limits

- **No graph, vector, fulltext, geo, columnar, timeseries, or raw SDBQL.** Those
  stay on SoliDB. Use `Model.find_by_sql` for SQL the portable surface cannot
  express.
- **`grouped {}` read-coalescing is SoliDB-only.**
- **Composite primary keys** are refused at boot on column-aware models.
- **`encrypts` and STI** work in column mode: encrypted fields must be text
  columns; STI subclasses need a string `type` column.

The shared portable surface (hash filters, `.join`, `.having`, includes,
HABTM / `through:`) is documented under [Multiple Databases](multi-database.md#sql-document-backends).

## See also

- [MySQL](mysql.md) / [SQLite](sqlite.md)
- [Multiple Databases](multi-database.md) — named connections and the capability matrix
- [Migrations](migrations.md#on-the-sql-adapters) — portable column DDL
- [Models](models.md) — the ORM surface
