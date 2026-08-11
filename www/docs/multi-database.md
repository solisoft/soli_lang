# Multiple Databases

Soli can use **one connection for the whole app** (the default) or **several named connections** at once — including **SoliDB and Postgres/MySQL in the same process**.

- **Default:** SoliDB via `SOLIDB_*` (full Model surface: graph, vector, timeseries, raw SDBQL).
- **Whole app on SQL:** `SOLI_DB_ADAPTER=postgres|mysql` + `DATABASE_URL` (document tables).
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
| `SOLI_DB_ADAPTER` | `solidb` (default), `postgres`, or `mysql` | `solidb` |
| `DATABASE_URL` | Required for `postgres` / `mysql` | unset |
| `SOLI_DB_POOL_SIZE` | SQL pool size | `10` |

```bash
# SoliDB (default)
SOLIDB_HOST=http://localhost:6745
SOLIDB_DATABASE=myapp

# Or: whole app on Postgres document tables
SOLI_DB_ADAPTER=postgres
DATABASE_URL=postgres://user:pass@localhost:5432/myapp
```

See also [Database Configuration](database.md) and [Configuration](configuration.md).

## `config/database.toml`

Place the file under the app root (same level as `config/routes.sl`). It is loaded at `soli serve` / migrate / import after `.env`.

```toml
# config/database.toml
default = "primary"

[connections.primary]
adapter = "solidb"          # solidb | postgres | mysql
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
```

### Field reference

| Field | Adapters | Description |
|-------|----------|-------------|
| `default` | — | Connection name for models without `connection "…"` |
| `adapter` | all | `solidb`, `postgres`, or `mysql` (aliases: `pg`, `postgresql`, `mariadb`, …) |
| `host` | solidb | SoliDB base URL |
| `database` | solidb | SoliDB database name |
| `username` / `password` | solidb | Optional basic auth |
| `api_key` | solidb | Optional API key |
| `url` | postgres, mysql | Connection string (`postgres://…` or `mysql://…`) — **required** for SQL |
| `pool` | postgres, mysql | Pool size (default 10) |

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

When a connection uses `adapter = "postgres"` or `"mysql"`, Model data is stored as:

```text
table <collection>
  _key  TEXT/VARCHAR PRIMARY KEY
  doc   JSONB / JSON   -- full Soli document
```

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
| HABTM / through includes, `.having`, `.join` | ✗ SoliDB-only (through/HABTM SQL planned) |
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

- **Per-connection SoliDB hosts:** registry stores host fields; full multi-host SoliDB routing is still completing — prefer one SoliDB endpoint plus SQL secondaries for now.
- **Request-scoped roles** (read replica / `writing`/`reading`) — not v1.
- **pgvector** on document tables — SoliDB-only (or a later design).

## Compile-time features

Postgres and MySQL client code is optional at **build time** (on by default). A
binary built without `postgres` or `mysql` cannot open those adapters — boot
fails with a rebuild hint if `SOLI_DB_ADAPTER` or `database.toml` selects one
that was not compiled in.

```bash
# SoliDB only (no SQL client crates)
cargo install --path . --locked --no-default-features \
  --features embedding,llm,codegraph

# Postgres only (no MySQL)
cargo install --path . --locked --no-default-features \
  --features embedding,llm,codegraph,postgres
```

Full table: [Configuration → Slim binary](configuration.md#slim-binary-cargo-features).

## Troubleshooting

| Symptom | Check |
|---------|--------|
| `Unknown database connection "…"` | Name matches `[connections.*]` / `default` |
| `url` required for postgres/mysql | Set `url =` or expand `${…}` from `.env` |
| SQL features missing | Confirm model’s connection adapter is `postgres`/`mysql`; see matrix above |
| `not compiled into this soli binary` | Rebuild with `--features postgres` and/or `mysql` (see [Slim binary](configuration.md#slim-binary-cargo-features)) |
| Includes error across DBs | Expected — query each side separately or colocate models |

## Related

- [Database Configuration](database.md) — SoliDB env basics  
- [Models](models.md) — ORM surface  
- [Configuration](configuration.md) — all env vars, slim Cargo features  
- [Blog: Multiple Databases](/docs/blog/multi-database)  
- Design: `docs/sql-adapter-design.md`  
