# Search: Vector, Fulltext & Geo

Models can declare **search indexes** in the class body — vector (HNSW ANN),
fulltext, geospatial, and plain secondary indexes — and query them with
`similar`, `search`, `near`, and `within`.

## Index DSL

```soli
class Article < Model
  vector_index "embedding", dimension: 1536, metric: "cosine"
  fulltext_index "title", "body"
  index "email", unique: true
end

class Store < Model
  geo_index "location"    # field holds { "lat": ..., "lon": ... }
end
```

| Declaration | Description |
|-------------|-------------|
| `vector_index field, dimension:, metric:` | HNSW vector index for ANN search on an embedding field. Optional: `m:`, `ef_construction:`, `quantization:`, `name:`. |
| `fulltext_index field, ...` | Fulltext index over one or more fields; powers `Model.search`. |
| `geo_index field` | Geospatial index; the field holds a `{ "lat": ..., "lon": ... }` hash. Powers `near` / `within`. |
| `index field_or_fields, options?` | Secondary index. A field name or an array (compound). Options: `unique:`, `type:` (`"persistent"` default, or `"hash"` / `"fulltext"` / `"bloom"` / `"cuckoo"`), `name:` (defaults to `idx_<collection>_<fields>`). |

## How indexes get created (sync strategy)

Declarations are **metadata-only at load** — declaring an index doesn't talk
to the database. In dev, the server ensures the declared indexes exist at
boot. In production, run `soli db:indexes [folder]` (new CLI command) or
create them in migrations — migrations remain the recommended production DDL
path; `soli db:indexes` is the DSL reconciler:

```bash
soli db:indexes           # reconcile declared indexes for the current app
soli db:indexes ./myapp   # or point at a project folder
```

There's also an internal `__sync_model_indexes()` builtin for scripts and
tests.

Migration-side equivalents:

- `db.create_index(collection, name, fields, { "unique": ..., "type": ... })`
  for secondary and fulltext indexes (`type: "fulltext"`).
- `db.create_vector_index(collection, name, field, dimension, options)` /
  `db.drop_vector_index(collection, name)` for vector indexes — `options` is
  a metric string (`"cosine"`) or a hash with `metric` and `quantization`.
- Geo indexes have **no migration helper yet** — dev boot or
  `soli db:indexes` creates them.

See [Migrations](migrations.md#index-helpers).

## Vector Search (`similar`)

With a `vector_index` declared on the field, `.similar()` **pushes the search
down** to the database's HNSW index (approximate nearest neighbor):

```soli
Article.similar("query text", "embedding", 5)      # embeds client-side, then ANN search
Article.similar([0.1, 0.2, ...], "embedding", 5)   # vector literal — no embedding call

# Chained filters: ANN candidates first, then your filter
Article.where({ "published": true }).similar("q", "embedding", 5)

# Escape hatch: force the old exact client-side cosine path
Article.similar("q", "embedding", 5, { "exact": true })
```

- Text queries are embedded client-side (requires `SOLI_EMBEDDING_API_KEY`;
  see [Models — Vector / Similarity Search](models.md#vector--similarity-search)
  for the embedding env vars). Pass a vector literal to skip the embedding
  call entirely.
- Results carry a `_similarity_score` field.
- Without a `vector_index` declaration, `.similar()` behaves exactly as
  before — the historical client-side cosine path is unchanged.

### ANN honesty notes

- HNSW results are **approximate** — ordering can differ from exact cosine
  similarity, especially among close scores.
- With chained filters, the database returns ANN *candidates* first (4×k,
  capped at 400) and your filters are applied **after** candidate selection —
  so fewer than `k` rows may come back.
- `{ "exact": true }` is the escape hatch: exact client-side cosine over the
  filtered rows, at fetch-everything cost.

## Fulltext Search (`search`)

Requires a `fulltext_index` covering the field(s). Results are ranked;
each carries `_search_score`:

```soli
results = Article.search("database indexing")
results[0]._search_score

# Fuzzy, field-scoped, highlighted
results = Article.search("phne", { "field": "title", "distance": 1, "limit": 5, "highlight": true })
results[0]._highlighted
```

| Option | Description |
|--------|-------------|
| `field` | Restrict the search to one indexed field |
| `distance` | Fuzzy matching: maximum edit distance |
| `limit` | Maximum number of results |
| `highlight` | Adds a `_highlighted` field with match markup |

## Geo Search (`near` / `within`)

Requires a `geo_index`. `near` sorts by distance and adds `_distance`
(meters); `within` returns everything inside a radius (meters):

```soli
nearby = Store.near(48.85, 2.35, { "limit": 5 })   # each result has ._distance
inside = Store.within(48.85, 2.35, 2000.0)         # radius: 2 km
```

## Pipeline notes (fulltext / geo)

`search`, `near`, and `within` bypass the SDBQL query pipeline:

- They are **eager** — they return an array of model instances immediately;
  there is no chaining (`.where(...)`, `.order(...)` don't compose with
  them).
- On soft-delete models, deleted rows are dropped **client-side** after the
  index lookup — which can shrink a `limit`-ed result set.

## Hybrid Search (not exposed yet)

Combined vector + fulltext ranking is **not** exposed through the ORM — the
database's HTTP endpoint for it is still a stub. The raw-SDBQL escape hatch
is SolidB's `HYBRID_SEARCH` function via `db.query(...)` / `@sdbql{}`; see
the [SolidB Hybrid Search docs](https://solidb.solisoft.net/docs/hybrid-search)
for its signature and tuning.

## See Also

- [Models — Vector / Similarity Search](models.md#vector--similarity-search)
- [Analytics & Columnar Stores](analytics.md)
- [Migrations — Index Helpers](migrations.md#index-helpers)
