# Search: Vector, Fulltext & Geo

Models can declare **search indexes** in the class body — vector (HNSW ANN),
fulltext, geospatial, and plain secondary indexes — and query them with
`similar`, `search`, `hybrid`, `near`, and `within`.

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

### Generating embeddings (`embed` / `embed_batch`)

`.similar("text", ...)` embeds the *query* for you, but you still need to
embed the **documents** you store. `embed` is the write-side counterpart — it
returns the vector for a string so you can persist it on the record:

```soli
class Article < Model
  vector_index "embedding", dimension: 1536, metric: "cosine"

  before_save fn() {
    this.embedding = embed(this.title + "\n" + this.body)
  }
end
```

- `embed(text) -> Array<Float>` — one embedding vector.
- `embed_batch(texts) -> Array<Array<Float>>` — one request for many texts,
  returned in input order. Use it to back-fill embeddings over an existing
  collection instead of one call per row:

```soli
articles = Article.where({ "embedding": null }).all
vectors  = embed_batch(articles.map(fn(a) a.title))
articles.each_with_index(fn(article, i) {
  article.embedding = vectors[i]
  article.save()
})
```

Both use the same `SOLI_EMBEDDING_*` configuration as `.similar()` (see
[the embedding configuration](database.md)); they raise if
`SOLI_EMBEDDING_API_KEY` is unset. Keys and endpoints live in the environment,
not in app code — one place to review where text is sent.

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

## Hybrid Search (`hybrid`)

Combined vector + fulltext ranking in one call. Requires **both** a
`vector_index` and a `fulltext_index` declaration. The query text is
embedded client-side for the vector leg (same embedding env vars as
`similar`) and used raw for the fulltext leg; the database fuses the two
ranked lists server-side:

```soli
class Article < Model
  vector_index "embedding", dimension: 1536, metric: "cosine"
  fulltext_index "content"
end

results = Article.hybrid("kubernetes deployment")
results[0]._hybrid_score      # combined score (sorted descending)
results[0]._vector_score      # raw vector similarity (when that leg matched)
results[0]._text_score        # raw fulltext score (when that leg matched)
results[0]._sources           # ["vector", "fulltext"]

# Tuning: favor semantics 70/30, RRF fusion, bigger page
Article.hybrid("error 500 handler", {
  "vector_weight": 0.7, "text_weight": 0.3,
  "fusion": "rrf", "limit": 20
})

# Pass a vector literal to skip the embedding call
Article.hybrid("exact keywords", { "vector": [0.1, 0.2, ...] })
```

| Option | Description |
|--------|-------------|
| `vector` | Query vector literal — skips the client-side embedding call |
| `vector_field` | Picks the `vector_index` by field (needed when several are declared) |
| `field` | Fulltext field to search (must be covered by a `fulltext_index`; default: first declared field) |
| `vector_weight` | Weight for the vector leg (default 0.5) |
| `text_weight` | Weight for the fulltext leg (default 0.5) |
| `fusion` | `"weighted"` (default) or `"rrf"` (Reciprocal Rank Fusion) |
| `limit` | Maximum results (default 10) |

Documents matching **both** legs rank highest; documents matching only one
still appear. See the
[SolidB Hybrid Search docs](https://solidb.solisoft.net/docs/hybrid-search)
for fusion-method details and tuning guidance. The raw-SDBQL escape hatch
remains SolidB's `HYBRID_SEARCH` function via `db.query(...)` / `@sdbql{}`.

## Pipeline notes (fulltext / hybrid / geo)

`search`, `hybrid`, `near`, and `within` bypass the SDBQL query pipeline:

- They are **eager** — they return an array of model instances immediately;
  there is no chaining (`.where(...)`, `.order(...)` don't compose with
  them).
- On soft-delete models, deleted rows are dropped **client-side** after the
  index lookup — which can shrink a `limit`-ed result set.

## See Also

- [Models — Vector / Similarity Search](models.md#vector--similarity-search)
- [Analytics & Columnar Stores](analytics.md)
- [Migrations — Index Helpers](migrations.md#index-helpers)
