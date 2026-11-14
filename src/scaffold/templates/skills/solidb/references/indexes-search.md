# Indexes and search

All index types below were created on `solidb 2.4.0` and queried. HTTP paths
are in http-api.md.

## Secondary indexes: persistent, hash, unique, compound

```sh
sdb_api POST /_api/database/mydb/index/people '{"name":"idx_age","fields":["age"]}'                 # persistent (default)
sdb_api POST /_api/database/mydb/index/people '{"name":"uq_email","field":"email","unique":true}'
sdb_api POST /_api/database/mydb/index/people '{"name":"h_city","fields":["city"],"type":"hash"}'
sdb_api POST /_api/database/mydb/index/people '{"name":"idx_city_age","fields":["city","age"]}'     # compound
```

- Types accepted: `persistent` (aliases `skiplist`, `btree`), `hash`,
  `fulltext`, `bloom`, `cuckoo`. `geo`, `vector` and `ttl` have their own
  endpoints (`"type":"geo"` here → `400 Unknown index type: geo`).
- A unique violation — from SDBQL `INSERT` or the document endpoint — is
  `400 InvalidDocument`: `Unique constraint violated: fields '["email"]' with
  value [String("u1@x.io")] already exists in index 'uq_email'`.
- The planner picks indexes on its own; check with `explain`:
  ```sh
  sdb_api POST /_api/database/mydb/explain '{"query":"FOR p IN people FILTER p.age == 5 RETURN p"}'
  # "access_type":"index_lookup","index_used":"idx_age"
  sdb_api POST /_api/database/mydb/explain '{"query":"FOR p IN people FILTER p.city == \"Paris\" AND p.age == 4 RETURN p"}'
  # "index_used":"idx_city_age"
  ```
  Without a usable index: `"access_type":"full_scan","index_used":null`.
- A composite index serves leading fields fixed by `==` or `IN`, then one
  range on the next field (2.6.3+; before that only `==` on *every* field,
  so Soli's `_jobs` claim read every pending row through `idx_state`):
  ```sh
  # index idx_jobs_claim on ["state","run_at","priority"]
  sdb_api POST /_api/database/mydb/explain '{"query":"FOR j IN jobs FILTER j.state IN [\"pending\", \"failed\"] AND j.run_at <= \"2026-11-05\" RETURN j"}'
  # "access_type":"index_lookup","index_used":"idx_jobs_claim"
  ```
- From 2.6.4 (observed on 2.6.4, index `[owner_id, status]`, no single-field
  index on `owner_id`):
  - its **first field alone** is served: `t.owner_id == "u1"` → `idx_owner_status`
    (a single-field index on that field, when there is one, is used first);
    counts too: `… COLLECT WITH COUNT INTO n` → `"access_type":"index_count"`.
  - **`!=` / `NOT IN` on the field after the fixed ones** is read as ranges
    around the value: `t.owner_id == "u1" AND t.status != "Completed"` →
    `idx_owner_status`, and rows with `status` null/missing still come back.
    `t.status != "Completed"` alone (nothing fixed before) is still a
    `full_scan`: a single-field index holds no nulls.
  - an **`IN` list holding null** on a fixed field: index `[opp, acct]`,
    `t.opp IN [null, ""] AND t.acct IN ["a1"]` → `idx_opp_acct`. Refused
    (scan) when some combination is all null: `t.opp IN [null, ""]` alone.
  - an **`OR` whose every branch has an index** is their union:
    `t.a == 1 OR t.b == 2` → `"index_used":"idx_a | idx_b"`. One branch
    without an index → `full_scan`, and (after 2.6.4) `explain` says which:
    `"warnings": ["optimizer: OR scanned, no index serves the branch
    \`t.seq == 2\` (an index on \`seq\` would serve it)"]`. EXPLAIN's
    `filters[].expression` is SDBQL text (`t.a == 1 OR t.seq == 2`); before,
    a comparison printed as `BinaryOp { … }`.
  Falls back when every fixed value is `null` or a value is an array/object.
- **Indexes are sparse by default**: a document whose index fields are all
  null or missing gets no entry, which is why `== null`, `IN [null, …]` and
  `!=` with nothing fixed before them scan. Create the index with
  `"sparse": false` to index every document (observed after 2.6.4):
  ```sh
  sdb_api POST /_api/database/mydb/index/cases '{"name":"idx_status","fields":["status"],"type":"persistent","sparse":false}'
  # {"name":"idx_status",…,"sparse":false,"status":"created"}; GET …/index/cases lists "sparse"
  ```
  Then, with `{status:"Open"}`, `{status:"Closed"}`, `{status:null}`, `{}`:
  `c.status != "Closed"` → `idx_status`, `[c1, c3, c4]` (the missing one
  included); `c.status == null` → `[c3, c4]`; `c.status IN [null, "Open"]`
  and `c.status <= "Open"` are index reads too. Several nulls never break a
  unique non-sparse index. An existing index cannot be switched: drop and
  recreate it. The driver protocol's CreateIndex has no such option.
  A sparse index serves `f < x` / `f <= x` only when another term on `f`
  rejects null (`f >= 1 AND f <= 5`, `f <= 5 AND f != null`), and an
  index-ordered `SORT f` only when every document has an entry or (after
  2.7.2) the FILTER bounds `f` and rejects null; else the query scans
  (traps.md #37 for the wrong results before that fix). Observed after 2.7.2,
  3,000 tickets: `t.amount < 20 SORT t.amount DESC LIMIT 3` → `full_scan`
  (null < 20, so the missing ones could match).
- `explain` **runs the query** (it times each step). After 2.7.1,
  `{"query": …, "analyze": false}` plans it without running or reading a
  document: `"plan_only": true`, every access `"estimated": true`, timings
  and counts 0 — use it on a slow or writing query.
- **Several usable indexes: the cheapest is read** (after 2.7.1; before, the
  first that applied — equalities before ranges, and only a range in the
  FILTER's *first* conjunct). The planner counts each candidate's index
  entries and reads the fewest; equal counts keep the old order. Observed,
  3,000 tickets, 2,990 `open`, `amount` 0…2999, hash `idx_status`,
  persistent `idx_amount`:
  ```sh
  sdb_api POST /_api/database/mydb/explain '{"query":"FOR t IN tickets FILTER t.status == \"open\" AND t.amount >= 2995 RETURN t"}'
  # "access_type":"index_lookup","index_used":"idx_amount","estimated_rows":5
  ```
  `t.status == "closed" AND t.amount >= 100` → `idx_status`, 10;
  `t.amount >= 10 AND t.amount < 20` → one interval, 10 (through 2.7.2: the
  `< 20` side, 20 — see the next entry);
  `t.owner_id IN [...] AND (t.opp == "o3" OR t.amount >= 2998)` → the OR's
  union `idx_opp | idx_amount`, 3, not the 2,000 team rows.
  `estimated_rows` is absent when only one index could serve (an
  `index_sort` read, after 2.7.2, always gives the entries it walks); `analyze:
  false` reports the same choice (no count for a subquery whose FILTER uses
  the outer row). `OPTIONS { indexHint }` skips the
  comparison.
- **A two-sided range is one interval, and `SORT f … LIMIT` over it reads in
  index order** (after 2.7.2). Through 2.7.2, `f >= lo AND f < hi` on a
  single-field index (the driver's and `POST …/index` indexes are sparse)
  read every entry on one side of one bound and checked the other per row:
  a one-day window of 1M orders read ~500k documents, 2.2 s, and 32 such
  queries at once got the server OOM-killed in 16 GB. Now the bounds merge
  (any operator, any order, field on either side) and the read stops at
  `hi`. With `SORT f ASC|DESC LIMIT n` the interval is walked from the right
  end and stops at `n` rows that pass the rest of the FILTER (0.1 ms). Same
  3,000 tickets (`amount` 0…2999, persistent `idx_amount`):
  ```sh
  sdb_api POST /_api/database/mydb/explain '{"query":"FOR t IN tickets FILTER t.amount >= 100 AND t.amount < 200 SORT t.amount DESC LIMIT 3 RETURN t._key"}'
  # "access_type":"index_sort","index_used":"idx_amount","estimated_rows":100,
  # "documents_scanned":3, "sort":{…,"index":"idx_amount"},
  # "warnings":["optimizer: rows read in SORT order from index 'idx_amount'"]
  # the query → ["t199","t198","t197"]
  ```
  The walk is taken only when no other index read of the FILTER counts fewer
  entries; it falls back to read-and-sort for several sort keys, a hint, a
  sparse range that admits null, or an array/object met in the range. TTL,
  Bloom and Cuckoo indexes still read one bound. A range with `LIMIT` and no
  `SORT` reads the whole interval.
- **EXPLAIN shows the FILTER that runs after an index read** (after 2.7.2):
  `t.amount >= 0 AND t.amount < 20 AND t.status == "open"` → `filters:
  [{…, "documents_before":20, "documents_after":10}]`, `documents_returned`
  10. Through 2.7.2 it said `filters: []` and counted every row the index
  returned (20919 for a window of 144 rows).
- **EXPLAIN follows `indexHint`** (after 2.7.1; before, it named the
  planner's own pick whatever the hint). Same tickets, `t.status == "open"
  AND t.amount >= 2995`, observed on both `explain` and `analyze: false`:
  `OPTIONS { indexHint: "idx_status" }` → `"index_used":"idx_status"`, no
  `estimated_rows`; `indexHint: "idx_owner"` (cannot serve) → `idx_amount`
  plus the warning ``"`tickets`: indexHint [idx_owner] cannot serve the
  FILTER; the planner's own choice is used"``; with `forceIndexHint: true`
  EXPLAIN fails like the query: `could not use index hint to serve query:
  none of [idx_owner] on 'tickets' can serve the FILTER (forceIndexHint is
  set)`.
- **Trigram index for `LIKE "%term%"`** (after 2.6.4): `"type":"trigram"`,
  one field, not unique (else `400 BadRequest` "A trigram index covers
  exactly one field and cannot be unique"):
  ```sh
  sdb_api POST /_api/database/mydb/index/accounts '{"name":"trg_name","type":"trigram","field":"name"}'
  ```
  Serves `LIKE(a.name, "%dupont%", true)` (→ `"index_used":"trg_name"`,
  matching "Dupont SA" and "DUPONT et fils") and `a.name LIKE "%Dupont%"`;
  a pattern with no literal run of 3 (`"%du%"`) scans, and it never serves
  `==`, ranges or SORT. **2.7.0 only:** a term with a common trigram was
  slower through the index than without (`%client 1234%` on 200k "Client N
  SARL": 220 ms vs 192 ms); after 2.7.0 the rarest trigram is read first
  (3.7 ms), and a term in over a quarter of the rows (`%sarl%`) scans. An `OR` of LIKEs on trigram-indexed fields is a
  union. Without it, every `LIKE "%…%"` (Soli's `.search` / `ilike`) scans.
- `explain` (2.6.4) also lists subquery loops after the query's own, with
  `"subquery":"LET o"` (or `"RETURN"`, `"FILTER"`, nested `"LET a > RETURN"`),
  explained against the first row that reaches them. When none does (empty
  collection, FILTER matching nothing), after 2.6.4 they are planned from
  their FILTER's shape without running — outer values as placeholders —
  with `"estimated": true` (a Soli `includes` on `_key` shows
  `"index_used":"primary key"`); on 2.6.4 only a `warnings` entry. `access_type` can be `index_sort` (rows read in SORT order to the
  LIMIT; `sort.index` names it — after 2.7.2 also for a range on a
  single-field index, with no sort step), `index_count`, and a `_key` read reports
  `"index_used":"primary key"`. Before 2.6.4 those showed as `full_scan`.
  After 2.7.2 a full-scan COLLECT reports `"documents_returned"` as its
  groups, plus `"optimizer: COLLECT folded into the scan of 'tickets':
  documents are grouped as they are read, never held as rows (decoding only
  amount, status)"`; after 2.7.3 a typed read says `(reading only amount,
  status from the stored bytes)`, with `; N key ranges read in parallel` on
  RocksDB (sdbql.md, COLLECT).
- `FOR p IN people OPTIONS { indexHint: "name" }` is accepted (a hint naming
  no index still runs the query). From 2.6.3 a hint may name a composite
  index: it is used once the FILTER fixes its first field. With
  `forceIndexHint: true` and a FILTER the index cannot serve (e.g. only
  `j.priority == 1`): `500 ExecutionError` "could not use index hint to
  serve query: none of [idx_jobs_claim] on 'jobs' can serve the FILTER".

## Fulltext

```sh
sdb_api POST /_api/database/mydb/index/people '{"name":"ft_bio","fields":["bio"],"type":"fulltext"}'
# options: "min_length" (default 3), "fold_accents": true (accent-insensitive)
```

```sdbql
FOR h IN FULLTEXT("people", "bio", "coffee") LIMIT 2
  RETURN {k: h.doc._key, s: h.score, m: h.matched}
-- [{"k":"p1","m":["coffee"],"s":0.00249…}, …]
RETURN LENGTH(FULLTEXT("people", "bio", "databases coffee", {limit: 5}))   -- [5]
```

- Hits are `{doc, score, matched, field, spans}`; 4th argument is a distance
  (number) or options `{distance, limit, phrase}`.
- A distance does **not** make a misspelled query word match by itself:
  `FULLTEXT("people", "bio", "cofee", 1)` returned `[]` here.
- **No fulltext index on that field → `[]`, no error.**
- `SNIPPET(hit, width)` / `HIGHLIGHT(text, terms)` exist for display
  (names probed only).

## Geo

Points are stored as `{lat, lon}` objects (`loc: {lat: 48.8584, lon: 2.2945}`).

```sh
sdb_api POST /_api/database/mydb/geo/places '{"name":"geo_loc","field":"loc"}'
```

```sdbql
FOR p IN NEAR("places", {lat: 48.86, lon: 2.33}, 5000, "loc") RETURN p.name
-- ["Louvre","Eiffel"]   (radius in meters, nearest first)
FOR p IN places SORT GEO_DISTANCE(p.loc, {lat: 48.86, lon: 2.33})
  RETURN {n: p.name, d: ROUND(GEO_DISTANCE(p.loc, {lat: 48.86, lon: 2.33}))}
-- [{"d":560.0,"n":"Louvre"},{"d":2603.0,"n":"Eiffel"}]   (no index needed, full scan)
RETURN ROUND(DISTANCE(48.8584, 2.2945, 48.8606, 2.3376))   -- [3162.0]
RETURN GEO_POINT(48.8, 2.3)   -- [{"coordinates":[2.3,48.8],"type":"Point"}]  (GeoJSON is lon,lat)
```

`NEAR` on a field without a geo index raises
`NEAR: no geo index on field 'loc' of collection 'people'`. The HTTP
`/geo/{coll}/{field}/near` and `/within` endpoints return
`{"results":[{"document", "distance"}]}`.

## Vector (HNSW)

```sh
sdb_api POST /_api/database/mydb/vector/docs '{"name":"emb_idx","field":"emb","dimension":3,"metric":"cosine"}'
# metric: cosine (default) | euclidean | dot (or dotproduct, lower-case). Anything else —
# including "dotProduct" — silently becomes cosine. Optional m (16), ef_construction (200), quantization
```

```sdbql
FOR r IN VECTOR_SEARCH("docs", "emb_idx", [1,0,0], 2) RETURN {k: r.doc._key, s: r.score}
-- [{"k":"a","s":1.0},{"k":"b","s":0.9938…}]
FOR r IN VECTOR_SEARCH("docs", "emb_idx", [1,0,0], 2, {filter: {tenant: "x"}}) RETURN r.doc._key
-- ["a","c"]   (filter = equality on fields, applied with over-fetch)
FOR d IN docs SORT VECTOR_SIMILARITY(d.emb, [1,0,0]) DESC LIMIT 2 RETURN d._key   -- exact scan
RETURN VECTOR_DISTANCE([0,0], [3,4], "euclidean")   -- [5.0]
```

A document whose vector has the wrong dimension is still inserted (no error).
HTTP search: `POST /vector/{coll}/{index}/search {"vector":[…],"limit":2}`.

## TTL

```sh
sdb_api POST /_api/database/mydb/ttl/sessions '{"name":"ttl_exp","field":"expires_at","expire_after_seconds":0}'
```

A document is removed once `field + expire_after_seconds <= now`, by a
sweeper that runs every 60 s. Observed:

| `expires_at` value | Removed? |
|--------------------|----------|
| integer epoch **seconds** in the past (`1792749600`) | yes, within a minute |
| RFC 3339 string in the past (`"2020-01-01T00:00:00Z"`) | yes |
| epoch **milliseconds** (`DATE_NOW() - 10000`) | no — read as seconds, i.e. far future |
| float seconds (`FLOOR(DATE_NOW()/1000) - 10` → `1792749549.0`) | through 2.10.0: no, floats are ignored; after 2.10.0: yes, within a minute (truncated to the second; observed on a build of main, 2026-11-14) |

So from SDBQL write a string: `expires_at: DATE_ISO8601(DATE_NOW() + 3600 * 1000)`
(verified: a past `DATE_ISO8601(…)` value was swept, a future one kept), or
send an integer from the client.

## Soli model declarations

`vector_index`, `fulltext_index`, `geo_index`, `index` in a Soli model class
are metadata; Soli creates them at dev-server boot or via `soli db:indexes` /
migrations (per Soli docs, unverified here). The ORM looks a vector index up
by the name **`idx_<field>`** — `Doc.similar(vec, "emb", 2)` called
`POST /vector/docs/idx_emb/search` and failed with
`Vector Index 'idx_emb' not found` against an index named otherwise. See
orm-mapping.md for what `Model.search` / `near` / `within` send.
