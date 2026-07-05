# Analytics & Columnar Stores

Soli gives you two complementary tools for analytical workloads, both backed
by SolidB:

- **Rich aggregation on document models** — grouped, multi-aggregate queries
  with `group_by` / `aggregate` / `having`, plus statistical terminals
  (`median`, `stddev`, `variance`, `count_distinct`).
- **Columnar stores** — a separate column-oriented storage engine for
  high-volume append-and-aggregate data (page views, events, telemetry).

## Rich Aggregation (Document Models)

### Grouped, multi-aggregate queries

Group on one or more fields with `group_by` and compute several aggregates at
once with `aggregate`:

```soli
rows = Order
  .where({ "status": "paid" })
  .group_by(["country", "plan"])
  .aggregate({ "total": ["sum", "amount"], "avg_age": ["avg", "age"], "n": ["count"] })
  .having("total > @min", { "min": 1000 })
  .order("total", "desc")
  .limit(20)
  .all
# => [{ "country": "FR", "plan": "pro", "total": 5300, "avg_age": 34.2, "n": 12 }, ...]
```

- `group_by` takes a field name or an **array** of field names. Each result
  row is a plain hash carrying the group fields plus your aggregate aliases.
- The `aggregate` spec maps `alias: [func, field]` — or `alias: ["count"]`
  for a plain row count.
- In grouped mode, `order` must name a **group field or an aggregate alias**;
  `limit` / `offset` compose as usual.

Available aggregate functions:

| Function | Meaning |
|----------|---------|
| `sum` / `avg` / `min` / `max` | The classics, over a field |
| `count` | Row count per group (no field argument) |
| `count_distinct` | Number of distinct values of a field |
| `median` | Median of a field |
| `stddev` | Standard deviation of a field |
| `variance` | Variance of a field |

> `PERCENTILE` is **not** available — SolidB has no such aggregate function.
> `median` covers p50; other percentiles need application-side math over the
> raw rows.

### One-argument `group_by`: implicit count

Without an `aggregate` spec you get a count per group, aliased `n`:

```soli
User.group_by("role").all
# => [{ "role": "admin", "n": 3 }, { "role": "member", "n": 240 }]
```

### Ungrouped `aggregate`: one row

`aggregate` without `group_by` collapses the whole match into a single row —
chain `.first`:

```soli
totals = Order.aggregate({ "total": ["sum", "amount"], "n": ["count"] }).first
totals.total   # => 812350
totals.n       # => 1204
```

### Statistical terminals

`median` / `stddev` / `variance` / `count_distinct` work like `sum` / `avg` —
set the aggregation and chain `.first`:

```soli
Order.median("amount").first
Order.where({ "status": "paid" }).stddev("amount").first
Order.variance("amount").first
Order.count_distinct("customer_id").first
```

### Filtering groups with `having`

`having(expr, binds?)` filters **after** the COLLECT, over the bare aggregate
aliases and group fields (no `doc.` prefix):

```soli
Order
  .group_by("country")
  .aggregate({ "total": ["sum", "amount"], "n": ["count"] })
  .having("total > @min AND n >= @orders", { "min": 1000, "orders": 5 })
  .all
```

> **Security:** like the string form of `where`, the `having` string is
> **developer-trusted** — it is spliced into the query. Never build it from
> user input; user-supplied values belong in the bind-vars hash.

### Soft delete

The new grouped queries (`group_by` + `aggregate`) respect the model's
soft-delete scope — deleted rows are excluded by default, and
`with_deleted` / `only_deleted` compose. One honest inconsistency: the legacy
three-argument form below **never** applied the soft-delete filter, and still
doesn't (kept unchanged for backward compatibility).

### Legacy three-argument form

`group_by(field, func, agg_field)` is unchanged and returns the historical
`[{group, result}]` shape:

```soli
User.group_by("country", "sum", "balance").all
# => [{ "group": "US", "result": 1000 }, { "group": "FR", "result": 500 }]
```

### Window functions: raw-SDBQL escape hatch

`ROW_NUMBER()` / `LAG()` / `OVER (...)` are **not** exposed through the
QueryBuilder. Drop down to a raw query (`db.query` in a script/migration, or
an `@sdbql{}` block) when you need them:

```soli
db = Solidb(env("SOLIDB_HOST"), env("SOLIDB_DATABASE"))
rows = db.query("
  FOR o IN orders
    RETURN { country: o.country, amount: o.amount,
             rank: ROW_NUMBER() OVER (PARTITION BY o.country ORDER BY o.amount DESC) }
")
```

See the SolidB documentation for the full window-function syntax.

## Columnar Models

Columnar stores are a **separate storage engine** inside SolidB: data is laid
out by column rather than by document, which makes appends cheap and large
scans/aggregations fast. They are *not* document collections — they live
behind their own HTTP API, are **not** visible to SDBQL `FOR` loops, and have
no document CRUD.

Declare one with the `columnar` DSL and typed `column` declarations:

```soli
class PageView < Model
  columnar compression: "lz4"        # optional options; bare `columnar` works
  column "url", "string"
  column "visited_at", "timestamp"
  column "duration_ms", "int", nullable: true
  column "country", "string", indexed: true
end
```

- `columnar` marks the model as a columnar store. Options: `compression:` —
  `"lz4"` (the default) or `"none"`.
- `column name, type` declares a typed column. `nullable: true` allows nils;
  `indexed: true` creates the default (sorted) column index.

Column types (aliases accepted):

| Type | Accepted spellings |
|------|--------------------|
| integer | `int`, `int64`, `integer`, `bigint` |
| float | `float`, `float64`, `double`, `number` |
| string | `string`, `text`, `varchar` |
| boolean | `bool`, `boolean` |
| timestamp | `timestamp`, `datetime`, `date` |
| json | `json`, `object`, `array` |

### Inserting rows (`insert_rows`)

```soli
PageView.insert_rows([
  { "url": "/", "visited_at": "2026-07-05T10:00:00Z", "duration_ms": 12, "country": "FR" },
  { "url": "/pricing", "visited_at": "2026-07-05T10:00:03Z", "duration_ms": 48, "country": "DE" }
])
# => { "inserted": 2, "ids": [...] }
```

In dev the store is auto-created (with your declared columns) on first use,
like document collections.

### Aggregating

`aggregate(field, op, options?)` returns a scalar, or one row per group with
a `group_by` option:

```soli
PageView.aggregate("duration_ms", "avg")                               # => 27.4 (scalar)
PageView.aggregate("duration_ms", "avg", { "group_by": ["country"] })
# => [{ "country": "FR", "value": 12.0 }, { "country": "DE", "value": 48.0 }]
PageView.count
```

Ops: `count` / `sum` / `avg` / `min` / `max` / `count_distinct`.

> Known cosmetic quirk: grouped **string** keys may come back JSON-quoted
> (e.g. `"\"FR\""` instead of `"FR"`) — this happens server-side. Strip the
> quotes client-side if it matters for display.

### Querying rows (`query`)

```soli
PageView.query({
  "columns": ["url", "duration_ms"],
  "filter": { "column": "country", "op": "eq", "value": "FR" },
  "limit": 100
})
```

The query endpoint is deliberately minimal — state of play:

- At most **one** filter. Ops: `eq` / `ne` / `gt` / `gte` / `lt` / `lte` /
  `in`.
- **No sort** — order the returned rows client-side if you need to.
- `columns` projects; `limit` caps the row count.

For anything richer, aggregate server-side (above) or export into a document
collection.

### Column indexes

```soli
PageView.add_column_index("country", "bitmap")
PageView.column_indexes
PageView.drop_column_index("country")
PageView.columnar_stats
```

Index kinds: `sorted` (the default) | `hash` | `bitmap` | `minmax` | `bloom`.
As a rule of thumb: `sorted` for ranges and equality, `hash` for pure
equality, `bitmap` for low-cardinality columns (country, status), `minmax`
for block pruning on range scans, `bloom` for fast negative membership tests.

`columnar_stats` returns store-level statistics (row counts, on-disk layout).

### No document API

Columnar models deliberately raise on the document API:

```soli
PageView.find(id)
# => raises: "PageView.find: PageView is a columnar model;
#    columnar stores have no document API."
```

No `_key`, `find`, `save`, `where`, validations, callbacks, or relations —
and no SDBQL `FOR` over the store. `insert_rows`, `aggregate`, `query`,
`count`, and the index/stats helpers above are the whole API.

### Migrations

Use the dedicated helpers — `columns` is an array of
`{ "name": ..., "type": ..., "nullable"?: bool, "indexed"?: bool }` hashes,
and `options` accepts `{ "compression": "lz4" | "none" }`:

```soli
def up(db: Any)
  db.create_columnar("page_views", [
    { "name": "url", "type": "string" },
    { "name": "visited_at", "type": "timestamp" },
    { "name": "duration_ms", "type": "int", "nullable": true },
    { "name": "country", "type": "string", "indexed": true }
  ], { "compression": "lz4" })
end

def down(db: Any)
  db.drop_columnar("page_views")
end
```

> `db.create_collection(name, "columnar")` now **raises** — it used to
> silently create a mislabeled *document* collection, which was never a real
> columnar store. Use `db.create_columnar` instead. See
> [Migrations](migrations.md).

## See Also

- [Models — Aggregations](models.md#aggregations) — the quick reference
- [Search: Vector, Fulltext & Geo](search.md) — the other half of this
  release
- [Migrations](migrations.md) — `create_columnar`, `create_index` types
