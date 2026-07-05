# SolidB Reference

The complete request & method surface for talking to **SolidB**, Soli's
database — in one place. Everything below ships with your project; the deep-dive
links point at other files in this same `docs/` folder.

SolidB is reached through three layers, from highest to lowest level:

1. **Model ORM** — `class Post < Model`. Validations, callbacks, relations,
   dirty tracking. Your first choice for everyday CRUD inside a server. It shares
   the worker's pre-configured connection. → [models.md](models.md)
2. **QueryBuilder** — the chainable object returned by `where`, `order`,
   `includes`, … Terminate a chain with `.all` / `.first` / `.count` / an
   aggregate.
3. **Raw SDBQL + the `Solidb` client** — drop down when the ORM doesn't expose a
   feature, or in scripts and migrations. Query with `@sdbql{ … }` blocks,
   `Model.where("…", binds)`, or a `Solidb(host, db)` instance.
   → [database.md](database.md)

Topic deep-dives (all local to this folder): [models.md](models.md) ·
[database.md](database.md) · [migrations.md](migrations.md) ·
[search.md](search.md) · [analytics.md](analytics.md)

> **The query language is SDBQL** (SolidB Query Language) — an
> `FOR … IN … FILTER … RETURN` dialect. A few places call it "AQL"
> interchangeably (e.g. the `dev_queries()` dev tool).

---

## 1. Connection & configuration

Models connect automatically using environment variables (typically a `.env`
file at the project root). Each worker thread keeps its own pooled connection.

| Variable | Description | Default |
|----------|-------------|---------|
| `SOLIDB_HOST` | SolidB server URL | `http://localhost:6745` |
| `SOLIDB_DATABASE` | Database name | `default` |
| `SOLIDB_USERNAME` | Auth username (optional) | none |
| `SOLIDB_PASSWORD` | Auth password (optional) | none |

`soli new myapp` seeds `SOLIDB_DATABASE` with your slugified project name and
generates the `.env`. The database is created on first use. `APP_ENV` selects a
`.env.{env}` overlay (loaded after the base `.env`). Full details, multi-env
setup, and troubleshooting: [database.md](database.md).

---

## 2. Model — static methods

Inherited from `Model` (don't override them). Full reference in
[models.md](models.md#static-methods-reference).

| Method | Description |
|--------|-------------|
| `Model.create(data)` | Insert a document. **Always returns an instance** — check `instance._errors`. |
| `Model.create_many([data, …])` | Batch insert. Returns `{ created, errors }`. |
| `Model.find(id)` | Lookup by id. **Raises `RecordNotFound` on miss → auto-404 in controllers.** |
| `Model.find_by(field, value)` | First match, or `nil`. |
| `Model.first_by(field, value)` | First match with ordering, or `nil`. |
| `Model.find_or_create_by(field, value, data?)` | Look up, or insert if absent. |
| `Model.where(hash)` | Filter — **safe for user input** (keys validated, values bound). Returns a QueryBuilder. |
| `Model.where(string, binds)` | SDBQL filter string — **developer-trusted**; bind external input via `@name`, never concatenate. |
| `Model.all` | Every record as instances. |
| `Model.update(id, data)` | Update by id. |
| `Model.upsert(id, data)` | Insert if absent, else update. |
| `Model.delete(id)` | Delete by id. |
| `Model.delete_all` | Wipe the **whole** collection (test teardown). For a filtered bulk delete use `Model.where(…).delete_all`. |
| `Model.count` | Row count. |
| `Model.reset_counters(id, relation)` | Recount a `has_many` and rewrite its [counter cache](#6-associations). |
| `Model.paginate({ page:, per: })` | Terminal — `{ "records": […], "pagination": {…} }` (see below). |
| `Model.with_deleted` / `Model.only_deleted` | Include / restrict to soft-deleted rows. |
| `Model.transaction …` | Run work atomically (see [§11](#11-transactions)). |
| `Model.<scope>` | Invoke a named `scope(...)` — returns a QueryBuilder. |

```soli
user = User.create({ "name": "Alice", "email": "alice@example.com" })
if user._errors
  # re-render the form; _errors is an array of {field, message}
  return render("users/new", { "user": user })
end

alice = User.find_by("email", "alice@example.com")   # nil if absent
admins = User.where({ "role": "admin", "active": true }).all
```

**Pagination** — `paginate({ page: 1, per: 25 })` returns:

```soli
{
  "records": [ … ],                             # instances for this page
  "pagination": { "page": 1, "per": 25,
                  "total": 100, "total_pages": 4 }
}
```

---

## 3. Model — instance methods

| Method | Description |
|--------|-------------|
| `record.save()` / `record.save(hash)` | Insert or update. Returns `true`/`false`. Hash form bulk-assigns first. |
| `record.update(hash)` | Merge attrs and persist. |
| `record.delete()` | Delete (soft-delete if the model opts in). |
| `record.restore()` | Undo a soft-delete. |
| `record.reload()` | Re-fetch from the DB; reset dirty baseline. |
| `record.increment("field", n=1)` | Atomic `+=` (CAS via `_rev`, bounded retry). |
| `record.decrement("field", n=1)` | Atomic `-=`. |
| `record.touch()` | Bump `_updated_at`. |
| `record._errors` | Array of `{ "field", "message" }` after a failed save; `nil` on success. |

**Dirty tracking** — instances know what changed since load/last-save:

```soli
user = User.find(id)
user.changed?                # false — freshly loaded
user.name = "New Name"
user.changed?                # true
user.changed                 # ["name"]  (sorted attribute names)
user.changes                 # { "name": ["Old Name", "New Name"] }
user.attribute_was("name")   # "Old Name"
user.save()
user.previous_changes        # { "name": ["Old Name", "New Name"] }
```

Tracking is value-based; mutating a nested Hash/Array *in place* is invisible —
reassign the attribute to record it.

---

## 4. QueryBuilder — chaining & terminators

`where`, `order`, `limit`, `offset`, `select`/`fields`, `join`, `includes`,
`includes_count`, `pluck`, `group_by`, `similar`, `time_bucket` all return a
chainable QueryBuilder. Terminate the chain with one of:

| Terminator | Returns |
|------------|---------|
| `.all` | Array of instances. |
| `.first` | First instance, or `nil`. |
| `.count` | Number. |
| `.exists` | Boolean. |
| `.pluck("field", …)` | Array of values (or hashes for multiple fields). |
| `.sum/avg/min/max("field")` | Numeric aggregate. |
| `.median/stddev/variance/count_distinct("field")` | Statistical aggregate (chain after the setter). |
| `.group_by(fields)` + `.aggregate(spec)` + `.having(expr, binds?)` | Grouped aggregation → array of hashes. |
| `.paginate({ page:, per: })` | `{ records, pagination }`. |
| `.delete_all` | Bulk `REMOVE` of every matching row — one statement, skips callbacks. Returns `null`. |
| `.update_all(hash)` | Bulk `UPDATE` of every matching row — one statement, skips validations/callbacks. Returns `null`. |
| `.to_query` | The generated SDBQL string (debugging). |

```soli
recent = Post
  .where({ "status": "published" })
  .order("created_at", "desc")
  .limit(20)
  .all

total_views = Post.where({ "user_id": user.id }).sum("views")

# Scoped bulk writes — no N+1 loop, one statement each:
User.where({ "active": false }).update_all({ "archived": true })
post.comments.where({ "spam": true }).delete_all
```

Full method list: [models.md](models.md#querybuilder-methods).

---

## 5. Eager loading & field selection

Avoid N+1 by pre-loading relations on the query.

```soli
posts = Post.where({ … }).includes("user", "comments").all
# posts[0].user and posts[0].comments are materialized in memory

# Count without loading the rows:
Category.includes_count("products").all      # each gets products_count

# Filtered / projected includes:
User.includes("posts", "published == @p", { "p": true }).all
User.includes({ "posts": ["title", "body"] }).all

# Project the main collection:
User.select("name", "email").all             # .fields is an alias

# Filter by related existence (no load):
User.join("posts").all                       # users that have >= 1 post
```

---

## 6. Associations

Declare relations in the class body; details in
[models.md](models.md#relationships).

```soli
class Post < Model
  belongs_to("user")                 # post.user_id (FK), post.user (instance)
  has_many("comments")               # user.comments → QueryBuilder
  has_one("featured_image")
  has_and_belongs_to_many("tags")    # M2M via a join collection
end
```

| Option | On | Effect |
|--------|----|--------|
| `class_name:` / `foreign_key:` | all | Override the related class / FK field. |
| `dependent:` | has_many, has_one | Cascade on owner delete: `"delete"` (per-row + callbacks), `"delete_all"` (bulk), `"nullify"`. |
| `through:` (+ `source:`) | has_many | Traverse an intermediate relation. |
| `counter_cache:` | belongs_to | Maintain a `<children>_count` column on the parent. |
| `polymorphic:` / `as:` | belongs_to / has_* | Belongs to any of several models (`{name}_id` + `{name}_type`). |

The `has_many` accessor is **both** enumerable (`for`, `map`, `len`) **and** a
QueryBuilder (`.where`, `.order`, `.count`, `.delete_all`). It also accepts
**writes** — the FK (and polymorphic type) are stamped automatically:

```soli
post = author.posts.create({ "title": "seeded" })   # FK auto-set; returns instance
author.posts << loose_post                           # adopt & save
author.posts << [draft_a, draft_b]                   # push several
```

---

## 7. Search — vector, fulltext, geo

Declare a search index in the class body, then query it. Full reference:
[search.md](search.md). These are **eager** (return arrays, no chaining).

```soli
class Article < Model
  vector_index "embedding", dimension: 1536, metric: "cosine"
  fulltext_index "title", "body"
end
```

| Call | Requires | Notes |
|------|----------|-------|
| `Model.similar(query, field?, k?, opts?)` | `vector_index` | ANN search; results carry `_similarity_score`. Pass a vector literal to skip embedding. |
| `Model.search(query, opts?)` | `fulltext_index` | Ranked; `_search_score`. Opts: `field`, `distance`, `limit`, `highlight`. |
| `Model.hybrid(query, opts?)` | both indexes | Fused vector + fulltext; `_hybrid_score`. |
| `Model.near(lat, lon, opts?)` | `geo_index` | Sorted by distance; `_distance` (meters). |
| `Model.within(lat, lon, radius)` | `geo_index` | Everything inside a radius (meters). |

```soli
Article.similar("query text", "embedding", 5)
Article.search("database indexing")
Store.near(48.85, 2.35, { "limit": 5 })
```

---

## 8. Analytics & columnar stores

**Grouped aggregation on document models** — full reference in
[analytics.md](analytics.md):

```soli
rows = Order
  .where({ "status": "paid" })
  .group_by(["country", "plan"])
  .aggregate({ "total": ["sum", "amount"], "n": ["count"] })
  .having("total > @min", { "min": 1000 })
  .order("total", "desc")
  .all
```

Aggregate funcs: `sum`, `avg`, `min`, `max`, `count`, `count_distinct`,
`median`, `stddev`, `variance`. (`having` is a **developer-trusted** string —
bind user values.) Window functions (`ROW_NUMBER`, `LAG`, `OVER`) are not
exposed — drop to raw SDBQL.

**Columnar stores** are a separate append-and-aggregate engine — no document
CRUD, no SDBQL `FOR`:

```soli
class PageView < Model
  columnar compression: "lz4"
  column "url", "string"
  column "country", "string", indexed: true
end

PageView.insert_rows([ { "url": "/", "country": "FR" } ])
PageView.aggregate("duration_ms", "avg", { "group_by": ["country"] })
PageView.query({ "columns": ["url"], "filter": { "column": "country", "op": "eq", "value": "FR" }, "limit": 100 })
```

---

## 9. Raw queries — the complete surface

Drop to raw SDBQL when the ORM doesn't expose a feature (window functions,
hand-tuned joins, graph traversal, DDL, blobs). **Always bind external input**
— never concatenate it into the query string.

### Four ways to issue raw SDBQL

| Mechanism | Use when | Binds via |
|-----------|----------|-----------|
| `@sdbql{ … }` block | Inside a server handler; multi-line, readable | `#{expr}` |
| `Model.where("…", binds)` | You want a raw FILTER fragment but ORM instances back | `@name` |
| `Model.transaction("…")` | A single statement, run transactionally | `@name` |
| `Solidb` client `db.query(…)` | Scripts, migrations, one-offs, full statements | `@name` |

```soli
# A. @sdbql{} block — #{expr} is BOUND as a parameter, not interpolated as text.
#    Returns raw documents (not model instances). @sdql{} is a legacy alias.
min_age = 18
users = @sdbql{
  FOR u IN users
  FILTER u.age >= #{min_age}
  SORT u.name ASC
  LIMIT 50
  RETURN u
}

# B. Model.where string form — ORM instances back, raw FILTER expression:
User.where("doc.age >= @min AND doc.role == @role",
           { "min": params["min_age"], "role": params["role"] }).all

# C. Single statement, transactionally (auto-commits):
User.transaction("FOR u IN users FILTER u.active UPDATE u WITH { seen: DATE_NOW() } IN users")

# D. Solidb client — nothing opens until the first call. In migrations a
#    pre-wired `db` is injected, so you skip the construct + auth here.
db = Solidb(env("SOLIDB_HOST"), env("SOLIDB_DATABASE"))
db.auth(env("SOLIDB_USERNAME"), env("SOLIDB_PASSWORD"))
rows = db.query("FOR u IN users FILTER u.age > @min RETURN u", { "min": 18 })
```

### The `Solidb` client — complete method reference

Every method the client exposes, grouped. Reference: [database.md](database.md#raw-queries).

**Connect & session**

| Method | Returns | Description |
|--------|---------|-------------|
| `Solidb(host, database)` | `Solidb` | Build a client. Nothing opens until the first call. |
| `db.auth(user, pass)` | — | Attach basic-auth; sent on every subsequent call. |
| `db.ping` | `String` | Server timestamp. |
| `db.connected()` | `Bool` | Whether `auth` has been called on this instance. |
| `db.close()` | — | Drop per-instance state (optional; instances are GC'd). |

**Run SDBQL**

| Method | Returns | Description |
|--------|---------|-------------|
| `db.query(sdbql, binds?)` | `Array` | Run a statement. `binds` fill `@param` placeholders (bound, never concatenated). |
| `db.explain(sdbql, binds?)` | `Hash` | Planner output without executing — confirm an index is picked up. |

**Document CRUD**

| Method | Returns | Description |
|--------|---------|-------------|
| `db.get(coll, key)` | `Hash` \| `null` | Fetch one document by `_key`. |
| `db.insert(coll, key?, doc)` | `Hash` | Create; pass `null` key to auto-generate. |
| `db.update(coll, key, doc)` | `Hash` | Patch an existing document (fails if missing). |
| `db.upsert(coll, key, doc)` | `Hash` | Update if present, else insert. |
| `db.delete(coll, key)` | `String` | Remove a document. |
| `db.list(coll)` | `Array` | List documents (capped at 100 — use `query` for more). |

**Collections**

| Method | Description |
|--------|-------------|
| `db.create_collection(name, type?)` | `type`: `"blob"` / `"edge"` / `"timeseries"` / default document. `"columnar"` raises — use `create_columnar`. |
| `db.drop_collection(name)` | Drop a collection. |
| `db.list_collections()` | Names of all (document/blob/edge/timeseries) collections. |
| `db.collection_stats(name)` | Document count, index list, storage size. |

**Columnar & timeseries admin**

| Method | Description |
|--------|-------------|
| `db.create_columnar(name, columns, options?)` | Create a columnar store; `columns` = `{name, type, nullable?, indexed?}` hashes. |
| `db.drop_columnar(name)` | Drop a columnar store. |
| `db.list_columnar()` | Names of all columnar stores. |
| `db.prune_collection(name, cutoff)` | Delete timeseries rows older than an RFC3339 cutoff. |

**Indexes**

| Method | Description |
|--------|-------------|
| `db.create_index(coll, name, fields, options?)` | `options`: `{ "unique": true }`, `{ "type": "persistent"\|"hash"\|"fulltext"\|"bloom"\|"cuckoo" }`. |
| `db.drop_index(coll, name)` | Drop an index by name. |
| `db.list_indexes(coll)` | All indexes on `coll` (including the primary index). |
| `db.create_vector_index(coll, name, field, dimension, options?)` | HNSW vector index; `options` is a metric string or `{metric, quantization}` hash. |
| `db.drop_vector_index(coll, name)` | Drop a vector index. |

**Blob storage** (on collections created with `type = "blob"`; payloads are base64)

| Method | Description |
|--------|-------------|
| `db.store_blob(coll, base64, filename, content_type)` | Store binary; returns the new blob id. |
| `db.get_blob(coll, blob_id)` | Fetch the payload as base64. |
| `db.get_blob_metadata(coll, blob_id)` | Filename, content type, size — without the body. |
| `db.delete_blob(coll, blob_id)` | Remove a blob. |

**Global one-shot helpers** — stateless, take the host (and database) each call.
For anything past a single call, use a `Solidb` instance instead.

| Function | Description |
|----------|-------------|
| `solidb_connect(addr)` | Connect and ping; returns `"Connected (ping: …)"`. |
| `solidb_ping(addr)` | Server timestamp. |
| `solidb_auth(addr, db, user, pass)` | One-off authentication check. |
| `solidb_query(addr, db, sdbql, binds?)` | Execute a query with no persistent state. |

---

## 10. SDBQL language

SDBQL (SolidB Query Language) is an ArangoDB-AQL-style dialect. Bind parameters
with `@name` for **values** and `@@name` for **collection names** — the only
safe way to pass anything dynamic.

```soli
db.query(
  "FOR doc IN @@coll FILTER doc.status == @status SORT doc.created_at DESC LIMIT @n RETURN doc",
  { "@coll": "users", "status": "active", "n": 50 }
)
```

### Read clauses

| Clause | Purpose |
|--------|---------|
| `FOR var IN collection` | Iterate documents. Also `FOR x IN [array]` and `FOR i IN 1..10` (ranges). |
| `FILTER expr` | Keep matching rows. |
| `SORT expr [ASC\|DESC], …` | Order (multiple keys allowed). |
| `LIMIT [offset,] count` | Window the result. |
| `LET name = expr` | Bind an intermediate value or a subquery. |
| `COLLECT key = expr [INTO g]` | Group. |
| `COLLECT AGGREGATE a = FUNC(expr)` | Grouped aggregation. |
| `RETURN [DISTINCT] expr` | Shape the output. |

### Write clauses

| Clause | Purpose |
|--------|---------|
| `INSERT doc INTO coll [OPTIONS {…}]` | Create a document. |
| `UPDATE key WITH doc IN coll` | Merge-patch (also `UPDATE doc IN coll` inside a `FOR`). |
| `REPLACE key WITH doc IN coll` | Overwrite the whole document. |
| `UPSERT search INSERT insertDoc UPDATE updateDoc IN coll` | Insert-or-update. |
| `REMOVE key IN coll` | Delete. |

Bulk writes combine a `FOR` with a write clause — one statement, no N+1:

```sdbql
FOR u IN users FILTER u.active == false REMOVE u IN users
FOR u IN users FILTER u.plan == @p UPDATE u WITH { archived: true } IN users
```

### Joins, subqueries & graph traversal

```sdbql
# Nested FOR = join
FOR u IN users
  FOR p IN posts FILTER p.user_id == u._key
    RETURN { user: u.name, title: p.title }

# Subquery in a LET
FOR u IN users
  LET post_count = LENGTH(FOR p IN posts FILTER p.user_id == u._key RETURN 1)
  RETURN MERGE(u, { posts: post_count })

# Graph traversal over an edge collection (OUTBOUND / INBOUND / ANY, depth min..max)
FOR v, e IN 1..3 OUTBOUND @start follows
  RETURN v
```

### Operators

- Comparison: `==` `!=` `<` `<=` `>` `>=`, `IN`, `NOT IN`
- Logical: `AND` `OR` `NOT` (`&&` `||` `!`)
- Arithmetic: `+` `-` `*` `/` `%`; ternary `cond ? a : b`; ranges `min..max`

### Common functions

Grounded in what SolidB exposes; the **full catalog is defined by the SolidB
server** — see the [SolidB docs](https://solidb.solisoft.net/docs/) for the
authoritative, exhaustive list.

| Area | Functions |
|------|-----------|
| Array / aggregate | `LENGTH` · `COUNT` · `SUM` · `AVG` · `MIN` · `MAX` · `FIRST` · `LAST` · `UNIQUE` · `FLATTEN` · `APPEND` · `CONTAINS` |
| Document | `MERGE` · `KEEP` · `UNSET` · `ATTRIBUTES` · `VALUES` |
| String | `CONCAT` · `LOWER` · `UPPER` · `TRIM` · `SUBSTRING` · `SUBSTITUTE` · `SPLIT` · `LIKE` |
| Date | `DATE_NOW` · `NOW` · `DATE_ISO8601` · `DATE_FORMAT` |
| Search / analytics | `VECTOR_SIMILARITY` · `TIME_BUCKET` · `COLLECTION_COUNT` · window `ROW_NUMBER() OVER (PARTITION BY … ORDER BY …)` |

> **Safety:** every value that came from outside must travel through a bind
> parameter (`@name` / `#{expr}`), never string concatenation. Collection names
> bind with `@@name`.

---

## 11. Transactions

`Model.transaction` runs work atomically — commit on normal return, roll back
(re-raising) on throw. Nested calls join the outermost.

```soli
# Block form (recommended). The block's value is returned.
order = Order.transaction do
  account = Account.find(account_id)   # `find` sees in-transaction state
  account.balance -= amount
  account.save()
  Order.create({ "account_id": account_id, "total": amount })
end

# Single SDBQL statement, transactionally (auto-commits):
User.transaction("FOR u IN users FILTER u.active UPDATE u WITH { seen: DATE_NOW() } IN users")

# Manual handle:
tx = User.transaction()
tx.create({ "name": "Alice" })
tx.commit()        # or tx.rollback()
```

> Cursor reads inside a block (`.where().all`, `find_by`, aggregations) see
> *committed* state. To read a row you wrote earlier in the same transaction,
> use `find` (a key lookup).

---

## 12. Migrations — collections & indexes

Migrations live in `db/migrations/` and receive the injected `db` helper. Full
reference: [migrations.md](migrations.md).

```soli
def up(db: Any)
  db.create_collection("users")
  db.create_index("users", "idx_email", ["email"], { "unique": true })
end

def down(db: Any)
  db.drop_index("users", "idx_email")
  db.drop_collection("users")
end
```

| Helper | Description |
|--------|-------------|
| `db.create_collection(name, type?)` | `"blob"` / `"edge"` / `"timeseries"` / default document. `"columnar"` raises — use `create_columnar`. |
| `db.create_columnar(name, columns, options?)` | Columnar store; `columns` = `{name, type, nullable?, indexed?}` hashes. |
| `db.drop_columnar(name)` / `db.prune_collection(name, cutoff)` | Columnar drop / timeseries retention. |
| `db.drop_collection(name)` / `db.list_collections()` / `db.collection_stats(name)` | Collection admin. |
| `db.create_index(coll, name, fields, options)` | `unique:`, `type:` (`hash` default, `persistent`, `fulltext`, `bloom`, `cuckoo`). |
| `db.create_vector_index(coll, name, field, dimension, options?)` / `db.drop_vector_index(coll, name)` | HNSW vector index. |
| `db.drop_index(coll, name)` / `db.list_indexes(coll)` | Index admin. |

Model-declared indexes (`index`, `vector_index`, …) are metadata-only —
`soli db:indexes` reconciles them, or mirror them in a migration. Seed data with
`soli db:seed` (`db/seeds.sl`).

---

## 13. Inspecting queries (`--dev`)

Under `soli serve . --dev`, `dev_queries()` returns the SDBQL/AQL issued for the
current request — each `{ "query", "bind_vars", "duration_ms" }`. Great for a
debug bar and spotting N+1s. Returns `[]` in production with zero overhead.

```erb
<% for q in dev_queries() %>
  <pre><%= q.query %> (<%= q.duration_ms %>ms)</pre>
<% end %>
```

---

## 14. Security checklist

- **User input → hash `where`.** `Model.where({ "role": params["role"] })` binds
  values and validates keys. Safe.
- **String `where` / `@sdbql` / `having` are developer-trusted.** Bind every
  external value via `@name` / `#{expr}` — **never** concatenate into the query.
- **`Model.find` raises on miss** (→ 404). Don't guard it with `if x.nil?`; use
  `find_by` / `first_by` for the "or nil" shape.
- **`db_query_raw` / `Trusted.*`** in `app/controllers/`, `app/middleware/`,
  `app/views/` are flagged by the `smell/dangerous-server-builtin` lint — keep
  raw access in models, services, scripts, and migrations.
- **Never commit `.env`** — it holds `SOLIDB_*` credentials.
