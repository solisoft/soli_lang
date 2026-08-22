# Database adapters (`src/db/`)

Soli apps talk to data in two ways:

1. **SoliDB** (HTTP document DB) — default, full ORM (graph, vector, AQL). Client code lives under `interpreter/builtins/model/` and `solidb_http.rs`.
2. **SQL adapters** — Postgres, MySQL, SQLite when `SOLI_DB_ADAPTER` or `config/database.toml` says so. That is **this** crate module.

## Layout

| File | Role |
|---|---|
| `mod.rs` | Re-exports, feature gates |
| `adapter.rs` | `Adapter` enum, `parse_adapter`, URL parsing |
| `registry.rs` | Named connections (`primary`, …) |
| `sql.rs` | Runtime SQL execution (query, exec, transactions) |
| `sql_compile.rs` | Portable SELECT/WHERE from the ORM query builder |
| `sql_columns_compile.rs` | Column-mode (`table "orders"`) |
| `hash_filter.rs` | Hash-style `.where({ age: 18 })` → SQL |
| `columns.rs` / `introspect.rs` | Schema introspection at boot |
| `ddl.rs` / `schema_dump.rs` | Migrations / `db:schema:dump` |
| `postgres.rs` / `mysql.rs` / `sqlite.rs` | Driver-specific |
| `tls.rs` | rustls, `sslmode` ladder |
| `caps.rs` | What each backend can do |
| `trace.rs` | Dev query log |

Cargo features: `postgres`, `mysql`, `sqlite` (on by default). `--no-default-features` builds a SoliDB-only binary.

## Types

### `Adapter`

`Postgres` | `Mysql` | `Sqlite` | (SoliDB is “not an Adapter”, it is the default when unset).

`parse_adapter(s)` reads `SOLI_DB_ADAPTER` / toml.

### `ConnectionSpec` / `ConnectionRegistry`

| Method | Role |
|---|---|
| `is_sql()` | SQL vs SoliDB HTTP |
| `label()` | Human name for logs |
| `get(name)` | Named connection |
| `default_spec()` | `primary` |
| `resolve(name)` | Error if missing |
| `names()` | All connection names |

`init_from_app_path(folder)` loads `config/database.toml` or env. `with_connection` runs a closure on a spec.

### TLS (`tls.rs`)

libpq ladder: `disable` / `prefer` (default) / `require` / `verify-ca` / `verify-full`. MySQL spellings map to the same rungs. rustls + ring; no OpenSSL.

## How a `User.where({ active: true }).all()` becomes SQL

1. Soli calls a native method on the Model class (`builtins/model/`).
2. That builds a query-builder object (`Value::QueryBuilder`).
3. `.all()` asks `src/db/sql_compile.rs` (or SoliDB AQL) for SQL + bind params.
4. `sql.rs` runs it on the pool.
5. Rows become `Instance`s of the model class.

**Never concatenate user strings into SQL.** Bind params only. Field names go through `validate_field_name` on the model side.

## Column mode

`table "orders"` on a class: introspect columns at boot (`columns.rs`). Composite primary keys are **not** supported (documented ceiling).

## What is *not* in `src/db/`

Associations, validations, callbacks, `grouped()` coalescing — those stay in `interpreter/builtins/model/`. `src/db/` is the portable SQL compiler + drivers.
