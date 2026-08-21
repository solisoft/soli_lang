# MySQL

MySQL (and MariaDB) is the SQL adapter for an existing MySQL estate, a shared
hosting default, or a warehouse already on MySQL. The Model surface is the same
as [Postgres](postgres.md) and [SQLite](sqlite.md) — hash `.where`,
associations, aggregates, migrations, jobs — stored either as `_key` + JSON
`doc` tables or as a [column-aware](multi-database.md#column-aware-models-existing-databases)
mapping onto a real schema.

SoliDB remains the full stack (graph, vector, timeseries, raw SDBQL). Use MySQL
when the data already lives there, or as a secondary connection beside SoliDB
via [`config/database.toml`](multi-database.md).

## Connect

Whole app on MySQL:

```bash
SOLI_DB_ADAPTER=mysql
DATABASE_URL=mysql://user:pass@localhost:3306/myapp
# optional
SOLI_DB_POOL_SIZE=10
```

Aliases: `mysql`, `mariadb`.

A named connection:

```toml
# config/database.toml
default = "primary"

[connections.warehouse]
adapter = "mysql"
url = "${WAREHOUSE_DATABASE_URL}"
pool = 5
```

```soli
class FactSale < Model
  connection "warehouse"
end
```

The `mysql` Cargo feature is on by default. A binary built without it refuses
to open a MySQL connection at boot and tells you to rebuild.

## URL

```text
mysql://USER:PASSWORD@HOST:3306/DATABASE
```

Placeholders in raw SQL are `?` — see [`Model.find_by_sql`](multi-database.md#raw-sql-escape-hatch).
Identifiers are backtick-quoted.

## TLS

The client speaks TLS — rustls with the `ring` provider, compiled in, so there
is no system OpenSSL to install. The modes are MySQL's; libpq's spellings
(`prefer`, `require`, `verify-ca`, `verify-full`) parse to the same rungs, so a
URL copied from either ecosystem works.

| `ssl-mode` | Encrypts | Verifies the chain | Verifies the hostname |
|------------|----------|--------------------|-----------------------|
| `DISABLED` | no | — | — |
| `PREFERRED` **(default)** | when the server supports it | no | no |
| `REQUIRED` | yes | no | no |
| `VERIFY_CA` | yes | yes | no |
| `VERIFY_IDENTITY` | yes | yes | yes |

```bash
# managed MySQL — the mode to use
DATABASE_URL=mysql://user:pass@db.example.com:3306/myapp?ssl-mode=VERIFY_IDENTITY

# verify against the server's own CA (MySQL writes `ca.pem` into its data dir)
DATABASE_URL=mysql://user:pass@db.internal:3306/myapp?ssl-mode=VERIFY_CA&ssl-ca=/var/lib/mysql/ca.pem
```

- **`PREFERRED` is the default.** The driver has no opportunistic mode, so Soli
  probes once with TLS when the pool opens and falls back to cleartext if the
  server declines. Anything stronger never downgrades.
- **`REQUIRED` and up connect over TCP**, even for a `localhost` URL. The driver
  skips TLS on a Unix socket outright, so preferring the socket would satisfy
  `REQUIRED` with a cleartext connection — the promise has to be real.
- MySQL's auto-generated server certificate is **self-signed**: `REQUIRED`
  accepts it (it encrypts without identifying), while `VERIFY_CA` needs
  `ssl-ca` pointing at that server's `ca.pem`.
- `ssl-ca` **replaces** the built-in Mozilla roots rather than adding to them,
  and a CA file supplied with a mode that would not consult it is refused, not
  silently ignored.
- A mandatory mode fails at boot naming the reason, e.g. `connection "primary"
  asked for ssl-mode=verify-full: invalid peer certificate: UnknownIssuer`.

## Create and drop

A SQL server does not create the database on first use. On a fresh target:

```bash
soli db:create              # CREATE DATABASE IF NOT EXISTS (db-less connection)
soli db:migrate up
soli db:schema:dump
soli db:drop                # DROP DATABASE IF EXISTS
```

`db:create` connects to the server **without** selecting a database (the target
may not exist yet), then `CREATE DATABASE IF NOT EXISTS`. Both accept
`--connection NAME`.

## Document tables

Each collection is:

```text
table <collection>
  _key  VARCHAR(255) PRIMARY KEY
  doc   JSON NOT NULL
```

The table is created on first write. JSON operators are `JSON_EXTRACT` /
`JSON_UNQUOTE` / `JSON_SET` / `JSON_MERGE_PATCH`. String equality in
`.where({ "status": "open" })` compares on the **text extract**.

MySQL has **no `RETURNING`**. An atomic increment writes with `JSON_SET` then
reads the new value on the same connection — still one statement's atomicity
for the write itself.

## Indexes

MySQL cannot index a JSON extract directly. `index "status"` (or
`soli db:indexes`) therefore:

1. Adds a generated `STORED` column for the extract (skipped if it already
   exists — MySQL has no `IF NOT EXISTS` on `ALTER TABLE … ADD COLUMN`).
2. Creates an index on that column (skipped if the index name already exists).

Multi-field and `unique:` work the same way. Numbers and booleans keep JSON
comparison and are not covered by that generated-column index.

Migrations can index a document field with a `doc.` prefix:

```soli
db.add_index("posts", ["doc.status"], { "unique": true })
```

`fulltext` / `vector_index` / `geo_index` are reported as skipped on SQL.

## Background jobs

The queue lives in `_jobs` on the same connection. MySQL has no
`SKIP LOCKED` in the form Soli uses on Postgres, so a claim writes a unique
token into `locked_by` and then selects the rows that hold that token.
Several workers can poll; a row belongs to whoever wrote the token first. On
first enqueue Soli indexes `state`, `run_at`, and `priority`. See [Jobs](jobs.md).

## Schema dump

```bash
soli db:schema:dump         # writes db/schema.sql
soli db:schema:load         # recreate a fresh database from that file
```

The dump is `SHOW TABLES` + `SHOW CREATE TABLE` for each, plus a
`-- versions:` header so load records applied migrations without replaying
every file. `SHOW CREATE` is the server's own SQL, so defaults, keys, and
engine options come through.

## Column-aware models

```soli
class Order < Model
  connection "warehouse"
  table "orders"
end
```

Introspection uses `information_schema`. `AUTO_INCREMENT` keys are left to the
database on insert. Types Soli maps:

| MySQL | Soli |
|-------|------|
| `smallint` / `mediumint` / `int` / `bigint` | Int |
| `float` / `double` | Float |
| `decimal` | Float (written as text; values beyond f64 lose precision on **read**) |
| `tinyint(1)`, `boolean`, `bit(1)` | Bool |
| `char` / `varchar` / `text` / `enum` / `set` | String |
| `date` / `datetime` / `timestamp` | DateTime, interpreted as **UTC** (MySQL stores no offset) |
| `json` | Hash / Array |
| `blob`, geometry, unsigned `bigint` | unsupported (skipped on read; error if filtered or written) |

There is no native `ILIKE`. Hash `.where({ "email": { "ilike": "%@x.com" } })`
compiles to `LOWER(email) LIKE LOWER(?)`.

Migrations emit foreign keys at **table** level: MySQL parses an inline
`REFERENCES` and then ignores it. `CREATE INDEX` has no `IF NOT EXISTS`;
`DROP INDEX` needs the table name.

## Honest limits

- **No timezone-aware timestamp.** `datetime` / `timestamp` values are UTC.
- **A Unix socket is never encrypted.** `ssl-mode=REQUIRED` and up therefore
  force TCP; a socket-only server is reachable in cleartext modes only.
- **No graph, vector, fulltext, geo, columnar, timeseries, or raw SDBQL.** Those
  stay on SoliDB. Use `Model.find_by_sql` for SQL the portable surface cannot
  express.
- **`grouped {}` read-coalescing is SoliDB-only.**
- **Composite primary keys** are refused at boot on column-aware models.
- **`encrypts` and STI** work in column mode: encrypted fields must be text
  columns; STI subclasses need a string `type` column.
- A reserved name (`_migrations`, `_jobs`, `_cron_jobs`) cannot be created,
  dropped, or renamed by a migration.

The shared portable surface (hash filters, `.join`, `.having`, includes,
HABTM / `through:`) is documented under [Multiple Databases](multi-database.md#sql-document-backends).

## See also

- [PostgreSQL](postgres.md) / [SQLite](sqlite.md)
- [Multiple Databases](multi-database.md) — named connections and the capability matrix
- [Migrations](migrations.md#on-the-sql-adapters) — portable column DDL
- [Models](models.md) — the ORM surface
