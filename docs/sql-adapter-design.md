# SQL PostgreSQL / MySQL / SQLite Adapter Design

**Status:** Phase 4 — SQL `Model.transaction`, `db:migrate --connection`, HABTM includes, and the SQLite adapter are done; `through:` includes on SQL remain.  
**Related:** comparison page; `src/db/`.

## Goals

1. Run a Soli MVC app against **PostgreSQL**, **MySQL**, or **SQLite** for the common CRUD / hash-where / list / aggregate loop.
2. Keep **SoliDB as the default full-featured backend**.
3. Publish a hard **capability matrix** — no silent half-support.

## Configuration

```bash
# Default — SoliDB
SOLIDB_HOST=http://localhost:6745
SOLIDB_DATABASE=myapp

# PostgreSQL
SOLI_DB_ADAPTER=postgres
DATABASE_URL=postgres://user:pass@localhost:5432/myapp

# MySQL / MariaDB
SOLI_DB_ADAPTER=mysql
DATABASE_URL=mysql://user:pass@localhost:3306/myapp

# SQLite — a path, not a server (`sqlite::memory:` for a throwaway database)
SOLI_DB_ADAPTER=sqlite
DATABASE_URL=sqlite://db/app.sqlite3

SOLI_DB_POOL_SIZE=10              # optional, default 10
```

Unset `SOLI_DB_ADAPTER` → `solidb`.

## Architecture (shipped)

```
Model API → QueryBuilder IR
              ├─ SoliDB: build_query() → SDBQL → HTTP/driver
              └─ SQL facade (src/db/sql.rs)
                   ├─ Postgres: JSONB tables (_key, doc)
                   ├─ MySQL:    JSON tables  (_key, doc)
                   └─ SQLite:   TEXT tables  (_key, doc) read with json1
```

- Hash-style `.where({ "field": value })` → portable SQL equalities.
- String/raw SDBQL `.where("doc…")` → **error on SQL**.
- Aggregates: `sum` / `avg` / `min` / `max` / `count` on SQL; list-stats (median/…) stay SoliDB-only.
- Multi-row `group_by` / multi-aggregate (no `.having` yet on SQL).
- `delete_all` / `update_all` (merge patch) on SQL.
- Eager `.includes` batching: `belongs_to`, `has_many`, `has_one` (+ `includes_count` for has_many).
- Migrations: `db.create_table` / `db.drop_table` (and `create_collection` alias).
- Import: `soli db:import [collection…]` copies SoliDB collections → SQL document tables.
- Version table: `_migrations`.

## Capability matrix

| Capability | SoliDB | Postgres | MySQL | SQLite |
|------------|--------|----------|-------|--------|
| CRUD, validations, callbacks | ✓ | ✓ | ✓ | ✓ |
| Hash `where` / order / limit / count / exists | ✓ | ✓ | ✓ | ✓ |
| sum / avg / min / max / count | ✓ | ✓ | ✓ | ✓ |
| `delete_all` / `update_all` (merge patch) | ✓ | ✓ | ✓ | ✓ |
| Soft-delete scope (`with_deleted` / `only_deleted`) | ✓ | ✓ | ✓ | ✓ |
| `pluck` / `select` projection | ✓ (server) | ✓ (client) | ✓ (client) | ✓ (client) |
| `increment` / `decrement` | ✓ (CAS) | ✓ (one UPDATE) | ✓ (one UPDATE) | ✓ (one UPDATE) |
| `.includes` belongs_to / has_many / has_one | ✓ | ✓ (batch) | ✓ (batch) | ✓ (batch) |
| `.includes` HABTM | ✓ | ✓ | ✓ | ✓ |
| `.includes` through / filtered | ✓ | ✗ (planned) | ✗ (planned) | ✗ (planned) |
| multi-row `group_by` + multi-agg | ✓ | ✓ | ✓ | ✓ |
| `.having` on groups | ✓ | ✗ | ✗ | ✗ |
| String SDBQL `where` | ✓ | ✗ | ✗ | ✗ |
| `.join` existence filter | ✓ | ✗ | ✗ | ✗ |
| Transactions (`Model.transaction`) | ✓ | ✓ | ✓ | ✓ (serializable only) |
| `db:migrate --connection` | ✓ (default + name) | ✓ | ✓ | ✓ |
| Graph / vector (pgvector) / columnar / timeseries | ✓ | ✗ | ✗ | ✗ |
| Auto-create table on first write | collections | ✓ | ✓ | ✓ |
| `index` declarations (`soli db:indexes`) | ✓ | ✓ (expression) | ✓ (generated column) | ✓ (expression) |
| Portable column DDL (`create_table` with columns, `add_column`, `add_index`) | n/a (schemaless) | ✓ | ✓ | ✓ |
| Per-migration `connection "name"` | ✓ | ✓ | ✓ | ✓ |
| `soli db:import` SoliDB → SQL | n/a | ✓ | ✓ | ✓ |

## Includes batching (SQL)

After the parent `SELECT`, Soli issues **one extra query per include**:

| Relation | Batch query |
|----------|-------------|
| `belongs_to` | `WHERE _key IN (…fks…)` |
| `has_many` / `has_one` | `WHERE doc->>fk IN (…parent keys…)` |
| `includes_count` (has_many) | same batch, count in memory |

Not on SQL: `through:`, polymorphic `belongs_to` child, include filters, `.join`. HABTM is supported — two batched queries (the join table, then the targets).

## Import

```bash
SOLI_DB_ADAPTER=postgres DATABASE_URL=postgres://…
SOLIDB_HOST=http://localhost:6745 SOLIDB_USERNAME=… SOLIDB_PASSWORD=…
soli db:import              # all non-_ collections
soli db:import posts users  # named only
```

Each document is upserted as `_key` + `doc`. Source is SoliDB; destination is the active SQL adapter.

## Multi-database connections

Named connections (SoliDB and/or SQL) in one app:

```toml
# config/database.toml
default = "primary"

[connections.primary]
adapter = "solidb"
host = "${SOLIDB_HOST:-http://localhost:6745}"
database = "${SOLIDB_DATABASE:-default}"

[connections.legacy]
adapter = "postgres"
url = "${LEGACY_DATABASE_URL}"
pool = 10
```

```soli
class User < Model
  # uses default ("primary")
end

class LegacyOrder < Model
  connection "legacy"
end
```

- Without `config/database.toml`, env (`SOLI_DB_ADAPTER` / `DATABASE_URL` / `SOLIDB_*`) becomes connection **`primary`** (unchanged for existing apps).
- Cross-connection `.includes` raises a clear error.
- SQL multi-pool + model `connection` DSL work today.
- `soli db:migrate up|down|status --connection NAME` targets a named connection (SQL secondaries fully supported).

## pgvector (deferred)

Optional Postgres vector search was listed for Phase 3 but is **not implemented**. Document tables store opaque JSON; a vector path needs declared dimensions, an extension install story, and a `.similar` compile path. Remains SoliDB-only until a dedicated phase.

## Phases

| Phase | Status |
|-------|--------|
| **0** Config + fail-fast | done |
| **1** Postgres CRUD + hash filters + migrations | done |
| **2** MySQL + aggregates + delete_all/update_all | done |
| **3** includes batching, group_by multi-row, import tool | **done** (pgvector deferred) |
| **M0–M1** multi-DB `database.toml` + per-model `connection` | **done** (docs: `/docs/database/multi-database`) |
| **4a** SQL `Model.transaction` (held pool connection) | **done** |
| **4b** `db:migrate --connection` | **done** |
| **4c** HABTM includes on SQL | done |
| **4d** `through:` includes on SQL | planned |
| **5** SQLite adapter (document + column mode, jobs, migrations) | **done** |
| **6** Portable column DDL in migrations + per-migration `connection` | **done** |
| **7** SQL indexes, atomic counters, classified errors, query log, column-mode parity | **done** |
