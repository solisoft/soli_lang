---
name: solidb
description: Use when working with SoliDB (the Soli framework's document database) — writing or debugging SDBQL queries (its AQL-like FOR/FILTER/RETURN language), `@sdbql{ … }` blocks, `db.query(...)` / the `Solidb` client, `Model.where("doc.… == @x", binds)` strings, Soli model queries that need raw SDBQL (window functions, graph traversal, UPSERT, bulk writes), reading `.to_query` / `dev_queries()` output, or calling the SoliDB HTTP API (`/auth/login`, `/_api/database/{db}/cursor`, collections, documents, indexes, transactions). Also for running a throwaway `solidb` instance to test queries.
---

# SoliDB and SDBQL

SoliDB is a document database (RocksDB storage, Rust) that speaks HTTP/JSON
(plus a binary MessagePack driver protocol on the same port). Queries are
written in **SDBQL**, an ArangoDB-AQL-style language: `FOR … FILTER … SORT …
LIMIT … RETURN`. The Soli framework's `Model` ORM compiles to SDBQL.

Everything below was checked against `solidb 2.4.0` and Soli 2.28.2. Where a
claim could not be checked it says **(unverified)**.

## Core concepts

- **Database → collections → documents.** A server holds databases (`_system`
  always exists). A collection holds JSON documents. Collection types:
  `document` (default), `edge` (requires `_from`/`_to`), `blob`.
- **System fields**, added by the server on every write:
  - `_key` — the primary key, unique in its collection. Generated as a
    UUIDv7 string if you don't give one (`"01a1682c-4ade-78e0-…"`). Any string
    is accepted (even `"bad key!"`).
  - `_id` — **`"<database>:<collection>/<key>"`**, e.g. `"shop:users/alice"`.
    Not `users/alice` as in ArangoDB. `FILTER u._id == "users/alice"` matches
    nothing; filter on `_key` instead.
  - `_rev` — a UUID that changes on every write (used for `If-Match` CAS).
  - `_created_at`, `_updated_at` — RFC 3339 strings.
  - `_id`/`_rev` you supply on insert are overwritten.
- **Edges** reference vertices as `"collection/key"` (`_from: "users/alice"`);
  traversal and `DOCUMENT("users/alice")` accept that short form.
- **Auth**: every `/_api/*` call needs `Authorization: Bearer <jwt>` (from
  `POST /auth/login`), HTTP Basic (`-u user:pass`), or `X-API-Key: <key>`.
  `GET /_api/health` is open.

## SDBQL essentials

```sdbql
-- comments start with -- (NOT // or /* */ — those are parse errors)
FOR u IN users                         -- collection, array, range (1..10), or subquery
  FILTER u.age >= @min AND u.active    -- several FILTERs are ANDed
  LET posts = (FOR p IN posts FILTER p.user_id == u._key RETURN p)
  SORT u.age DESC, u.name ASC
  LIMIT 20, 10                         -- offset, count  (or LIMIT count)
  RETURN { name: u.name, n: LENGTH(posts) }
```

- Keywords are case-insensitive; function names are upper-case by convention.
- Strings take `"…"` or `'…'`. Object keys: bare, quoted (`{"b-c": 1}`), or
  shorthand `{name}`. Computed keys `{[k]: 1}` are **not** supported.
- `LET` may come after `SORT`/`LIMIT` — then it runs only for surviving rows.
- `RETURN DISTINCT expr`; `x[*].field` projects over an array; `a[-1]` is the
  last element; `1..3` is an **inclusive** range `[1,2,3]`.
- Operators: `== != < <= > >=`, `IN`, `NOT IN`, `LIKE` (`%`, `_`,
  case-sensitive), `=~` / `!~` regex, `AND OR NOT` / `&& || !`, `? :`,
  `??` (null fallback), `?.` (null-safe access), `|>` (pipe into a function's
  first argument), `CASE WHEN … THEN … ELSE … END`.
- `null || "x"` → `"x"` but `null OR "x"` → `true` (`||` returns a value).
- Quantifiers: `FILTER ANY t IN u.tags SATISFIES t == "x"` and `NONE … SATISFIES`
  work; `ALL … SATISFIES` does **not** parse — use `EVERY(u.tags, t -> t == "x")`.
- Lambdas: `MAP(xs, x -> x * 2)`, `FILTER(xs, x -> x > 1)`,
  `REDUCE(xs, (acc, x) -> acc + x, 0)`, `SORT_BY`, `GROUP_BY`, `EVERY`, `SOME`.
- Read one document: `DOCUMENT("users/alice")` or `DOCUMENT("users", "alice")`
  (returns `null` when missing).

### Grouping

```sdbql
FOR o IN orders
  COLLECT country = o.country                    -- group key(s)
  AGGREGATE total = SUM(o.amount), n = COUNT()   -- per-group aggregates
  FILTER total > 100                             -- post-COLLECT filter
  SORT total DESC
  RETURN { country, total, n }

FOR o IN orders COLLECT s = o.status WITH COUNT INTO n RETURN { s, n }
FOR o IN orders COLLECT s = o.status INTO g RETURN { s, keys: g[*].o._key }
FOR o IN orders COLLECT s = o.status INTO keys = o._key RETURN { s, keys }
FOR o IN orders COLLECT AGGREGATE total = SUM(o.amount) RETURN total  -- no key
```

Window functions work in `RETURN`:
`ROW_NUMBER() OVER (PARTITION BY e.dept ORDER BY e.salary DESC)`, `RANK()`,
`SUM(x) OVER (PARTITION BY …)`.

### Writes

```sdbql
INSERT { _key: "c1", n: 0 } INTO counters RETURN NEW
INSERT @doc INTO users OPTIONS { overwriteMode: "update" }   -- ignore|replace|update|conflict
UPDATE "c1" WITH { n: 7 } IN counters RETURN { old: OLD.n, new: NEW.n }
UPDATE { _key: "c1", m: 1 } IN counters                       -- selector doubles as patch
REPLACE "c1" WITH { n: 1 } IN counters                        -- whole document
REMOVE "c1" IN counters RETURN OLD
UPSERT { _key: "c1" } INSERT { _key: "c1", n: 1 } UPDATE { n: 5 } IN counters

-- Bulk: one statement, no N+1
FOR u IN users FILTER u.active == false REMOVE u IN users
FOR u IN users FILTER u.plan == @p UPDATE u WITH { archived: true } IN users
FOR i IN 1..3 INSERT { i } INTO counters RETURN NEW._key
```

- `INTO`/`IN` are interchangeable after a write. There is no `DELETE` keyword.
- Write `OPTIONS`: `ignoreErrors`, `keepNull`, `mergeObjects`, `overwriteMode`
  (INSERT only). Unknown option keys are a parse error.
- `UPDATE` merges top-level fields but **replaces nested objects** unless
  `OPTIONS { mergeObjects: true }`. `{ f: null }` stores `null` unless
  `keepNull: false`, which removes the field.
- The response carries `inserted` / `updated` / `deleted` counters.

### Bind parameters

`@name` binds a **value** (`"bindVars": {"name": …}`). Always bind external
input; never concatenate it into the query.

**`@@collection` is not supported** (`Expected bind variable name after '@'`),
and `FOR d IN @c` with a string iterates nothing. A dynamic collection name
must be validated against a whitelist and spliced into the query text.

## Top traps (full list: references/traps.md)

1. **`FILTER` swallows row errors.** A type error inside `FILTER` (a
   function given a value it cannot take) drops the row silently. Through
   2.9.4 so did an unknown function and a missing bind variable: the query
   returned `[]`, not an error. Since 2.9.5 those two fail the query, as in
   `RETURN` (`Unknown function: X`, `Bind variable '@x' not found`).
2. **`UPSERT` only finds documents by `_key`/`_id`.** `UPSERT { email: @e } …`
   never matches anything and **inserts a duplicate every time**. Also `OLD`
   is not bound inside the `UPDATE` expression (`Variable 'OLD' not found`).
   Use a deterministic `_key`, and read the old value with `DOCUMENT()`:
   `UPSERT { _key: @k } INSERT { _key: @k, n: 1 } UPDATE { n: DOCUMENT("c/" + @k).n + 1 } IN c`
   (`OLD` *is* available in the trailing `RETURN`).
3. **`NOT a == b` is `(NOT a) == b`.** Write `a != b` or `!(a == b)`.
4. **Soli ORM failure modes differ by terminator.** An invalid SDBQL string
   raises from `.all` / `.count`, but `.first` returns `nil`, and an
   `@sdbql{ … }` block returns a **String** `"Error: Query failed: 400 …"`
   instead of raising. An unknown function inside a `where` string matches
   nothing (`[]`).
5. **Query cache ignores `batchSize`.** A repeated identical query is served
   from cache (`"cached": true`) with *all* rows and no cursor `id`. Send
   `"cache": false` when you page with cursors.
6. **`_id` includes the database** (`db:coll/key`) — see Core concepts.
7. **`SUM` of integers is a float through 2.9.4** (`SUM([1,2]) → 3.0`, and
   `-0.0` over an empty list/subquery). Since 2.9.5 integers add to an
   integer (`3`, and `0` for nothing); one float among them gives a float.
   `AVG`/`MAX` of `[]` return `null`; the other math functions return floats.
8. **Comments are `--` only.**

## Try a query (curl helper)

`scripts/sdbql.sh` (in this skill) defines two shell functions (bash/zsh):

```sh
source ~/.claude/skills/solidb/scripts/sdbql.sh
export SOLIDB_URL=http://localhost:6799 SOLIDB_DB=mydb SOLIDB_USER=admin SOLIDB_PASS=secret
# or: export SOLIDB_API_KEY=sk_…
sdbql 'FOR u IN users FILTER u.age > @a SORT u.name RETURN u.name' '{"a": 18}'
sdb_api GET /_api/databases
sdb_api POST /_api/database/mydb/collection '{"name":"users"}'
```

`sdbql` POSTs `{"query", "bindVars", "cache": false}` to
`/_api/database/$SOLIDB_DB/cursor` with Basic auth (or `X-API-Key`).

**Don't experiment on someone's real database.** Start a throwaway server:

```sh
mkdir -p /tmp/sdb && SOLIDB_ADMIN_PASSWORD=testpass123 \
  solidb --host 127.0.0.1 --port 6799 --data-dir /tmp/sdb/data --no-lua --dev > /tmp/sdb/log 2>&1 &
# user "admin" / testpass123; create a database first:
sdb_api POST /_api/database '{"name":"mydb"}'
# when done: kill the process, rm -rf /tmp/sdb
```

`--dev` keeps the block cache at 128 MB. Without it, after 2.7.3 the cache
defaults to 40% of (memory − 1 GiB), memory being the process's cgroup limit
or else the host's: 25 GB on a 62 GB host (startup log: `block_cache=25051MB
(40% of the host memory of 62.2GB less 1GB)`). Pass `--block-cache 256MB` (or
`SOLIDB_BLOCK_CACHE`) to bound a throwaway server started without `--dev`.

Without `SOLIDB_ADMIN_PASSWORD` a random admin password is generated and
written to `<data-dir>/.admin_password`. Default port is 6745. Keep
`--host 127.0.0.1`: `solidb` loads a `.env` from the working directory or a
parent, and a Soli app's `SOLIDB_HOST=http://…` there is taken as the bind
address — the server then exits with `failed to lookup address information`.

## Soli in one screen (full mapping: references/orm-mapping.md)

```soli
Post.where({ "status": "published", "views": { "gte": 10 } }).order("views", "desc").limit(5).all
# FOR doc IN posts FILTER doc.status == @status__eq_1 AND doc.views >= @views__gte_2
#   SORT doc.views DESC LIMIT 5 RETURN doc
Post.where("doc.views > @v", { "v": 6 })   # raw FILTER fragment: the loop var is `doc`
Post.count                                 # RETURN COLLECTION_COUNT("posts")
Post.find_by("slug", s)                    # FOR doc IN posts FILTER doc.slug == @val LIMIT 1 RETURN doc
print(query_builder.to_query)              # shows the SDBQL + bind vars

rows = @sdbql{ FOR p IN posts FILTER p.views >= #{min_views} RETURN p }   # #{} is bound, not spliced
```

Class → collection: snake_case plural (`BlogPost` → `blog_posts`,
`Person` → `people`, `Category` → `categories`). A loaded record's key is
`record._key` — `record.id` is only set on the instance `create` returned.

## Reference files

| File | Contents |
|------|----------|
| `references/sdbql.md` | Full syntax: clauses, operators, COLLECT, windows, JOIN, graph traversal, CTEs, write options, result shape |
| `references/functions.md` | Every function name the server accepts (probed), grouped, with verified examples |
| `references/http-api.md` | Endpoints with exact paths, bodies and responses: auth, databases, collections, documents, cursor, explain, indexes, transactions, API keys |
| `references/indexes-search.md` | Persistent/hash/unique/fulltext, geo, vector, TTL indexes; FULLTEXT / NEAR / VECTOR_SEARCH; explain |
| `references/orm-mapping.md` | What each Soli Model / QueryBuilder call sends to SoliDB (captured on the wire) |
| `references/traps.md` | Every surprising behavior found, with repro and workaround; doc discrepancies |
| `scripts/sdbql.sh` | The `sdbql` / `sdb_api` shell helpers |
