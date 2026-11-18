# SoliDB HTTP API

Paths come from `src/server/routes.rs` in the SoliDB source; every request and
response shown was run against `solidb 2.4.0`. `{db}` is a database name.
JSON in, JSON out (`Content-Type: application/json`).

## Authentication

| Method | How |
|--------|-----|
| JWT | `POST /auth/login` `{"username","password"}` → `{"token": "eyJ…"}`; send `Authorization: Bearer <token>` |
| Basic | `curl -u admin:secret …` on any `/_api/*` route |
| API key | `X-API-Key: sk_…` (create below) |

- No credentials → `401`. `GET /_api/health` needs none:
  `{"status":"ok","timestamp":"…"}`.
- First start creates user `admin`; its password is `SOLIDB_ADMIN_PASSWORD`
  if set, otherwise random and written to `<data-dir>/.admin_password`.

```sh
curl -s -X POST localhost:6799/auth/login -H 'Content-Type: application/json' \
  -d '{"username":"admin","password":"testpass123"}'
# {"token":"eyJ0eXAiOiJKV1Qi…"}
```

API keys and identity:

| Method & path | Body | Response |
|---------------|------|----------|
| `POST /_api/auth/api-keys` | `{"name":"ci","roles":["admin"]}` (optional `scoped_databases`) | `{"id","name","key":"sk_…","created_at","roles","scoped_databases"}` — `key` is shown only here |
| `GET /_api/auth/api-keys` | | `{"keys":[{id,name,created_at,roles,scoped_databases}]}` |
| `DELETE /_api/auth/api-keys/{key_id}` | | `200` |
| `GET /_api/auth/me` | | `{"username":"admin","roles":["admin"],"permissions":[…]}` |
| `PUT /_api/auth/password` | `{"current_password","new_password"}` — the caller's own | (unverified); an admin resetting someone else's: see *Users and roles* below |

With an API key, `CURRENT_USER()` returns `"api-key:<id>"`.

### Users and roles (observed on a build of `main` after 2.12.1, commit `3c851677`, 2026-11-17)

Unreleased at that date: 2.12.1 has neither the `solidb user` command nor
`PUT /_api/auth/users/{u}/password`.

| Method & path | Body | Response |
|---------------|------|----------|
| `PUT /_api/auth/users/{username}/password` | `{"password":"…"}` — the new one; the caller needs Admin on every database (a global `admin`), and does not give the old one | `204`, empty. Under 12 characters → `400 "Password must be at least 12 characters"`; unknown user → `404 "User 'x' not found"`; an editor or an admin of one database → `403 "…insufficient permissions for Admin on global"`; the field named anything else → `422` |

- After a reset the old password is refused at once (Basic and login); a
  JWT issued before it keeps working until it expires.
- A role may be limited to one database: `/_api/auth/me` then shows
  `"roles":["admin@shop"]` and
  `"permissions":[{"action":"admin","scope":"database","database":"shop"}]`.

The `solidb user` command wraps those endpoints (no curl, no password in
argv):

```sh
export SOLIDB_URL=http://localhost:6745 SOLIDB_PASSWORD=…   # the admin's; or --password-file F, or a prompt
echo 'a-long-password' | solidb user create shop_app --role admin --database shop --password-stdin
# User 'shop_app' created
solidb user list            # USERNAME  ROLES  CREATED      (--json: {"users":[{username,roles,created_at}]})
solidb user roles shop_app  # ROLE  DATABASE  ASSIGNED BY  ASSIGNED AT   (--json: [{id,role,database,username,assigned_by,assigned_at}])
solidb user grant ed viewer --database other    # Role 'viewer' granted to 'ed' on database 'other'
solidb user revoke ed viewer --database other
echo 'another-password' | solidb user passwd shop_app --password-stdin   # Password of 'shop_app' changed
solidb user delete ed       # also removes its roles
```

- Global options: `-s/--server` (default `http://localhost:6745`),
  `-u/--username` to log in as (default `admin`), `--password-file`,
  `-k/--api-key` (an `sk_…` key or a JWT, env `SOLIDB_API_KEY`). Without
  `--password-stdin` the new password is prompted for, twice.
  `passwd` with no username changes the caller's own.
- `--role` without `--database` is for every database. Roles: `admin`,
  `editor`, `viewer`, or a custom one. An unknown role is an error and the
  user is not kept: `Error: User 'bad' was not kept: assigning role
  'nosuchrole' failed: nosuchrole (HTTP 404)`.
- Exit codes: `0`; `3` not found (user or role); `4` already exists
  (`User 'shop_app' already exists (HTTP 409)`), so a provisioning script
  can accept it; `5` login or permission refused; `1` anything else (a
  password under 12 characters).
- `solidb` loads a `.env` from the current directory: run from an
  application's folder, `SOLIDB_PASSWORD` may be the application's, and
  the error then says so (`Login as 'admin' with the password from
  SOLIDB_PASSWORD failed: Invalid credentials (HTTP 401)`).
- `solidb user roles nobody` prints `No roles assigned to 'nobody'` and
  exits 0, whether or not the user exists.

### Who may write `_jobs` (same build)

`_jobs` (Soli's job store) is readable with Read and writable only by an
admin. `POST …/document/_jobs`, by caller:

| Caller | Result |
|--------|--------|
| global `admin` | `201`, any database |
| admin of that database (`admin@shop` writing `shop`) | `201` — through 2.12.1 `403`; also inside a transaction (`…/transaction/{id}/document/_jobs`) and through import |
| admin of another database | `403 "…insufficient permissions for Write on other"` |
| `editor` | `403 "Access denied: '_jobs' is executed by the server and is writable only with an Admin credential"` |

- **Through SDBQL too, from commit `64791a78`** (unreleased; observed
  2026-11-17 on a build of it): as an admin of
  that one database, `INSERT {…} INTO _jobs` and `UPDATE … IN _jobs` on
  `/cursor`, `INSERT INTO _jobs …` on `/sql` and a query inside a
  transaction (`…/transaction/{id}/query`) all write. On `3c851677`
  itself and through 2.12.1 they are `403` with the editor's message and
  only a global admin's query writes `_jobs`: use the document endpoint
  there. An editor's query stays `403`.
- **From a Lua service script too, from commit `86074294`**
  (unreleased; observed 2026-11-17 on a build of it): a script writes as
  its caller, so one called by `admin@shop` in `shop` writes `_jobs` by
  `db:collection("_jobs"):insert({…})` and by
  `db:query("INSERT {…} INTO _jobs")`. Through 2.13.0 both are `403`
  with the editor's message, whatever the docs said. The same script
  called by that admin in another database, or by an editor, is `403`;
  `_scripts` and `_env` stay `403` from a script as from the API.
- That is all such an admin gains in a query: `ROW_POLICY("c", "…")` is
  still `403 "…changes a row policy and requires admin"` for anyone but a
  global admin.
- SDBQL does not create `_jobs`: a query on a database that has none is
  `404`; the document endpoint creates it.
- `_env` and the other credential collections stay `403` for every
  caller, `_scripts` and the like stay unwritable by name
  (`"…is managed by the server and is not writable through this API"`).

## Databases

| Method & path | Body | Response |
|---------------|------|----------|
| `POST /_api/database` | `{"name":"shop"}` | `{"name":"shop","status":"created"}` |
| `GET /_api/databases` | | `{"databases":["_system","shop"]}` |
| `DELETE /_api/database/{name}` | | `204` |

## Collections

| Method & path | Body / notes | Response |
|---------------|--------------|----------|
| `POST /_api/database/{db}/collection` | `{"name":"users"}`, optional `"type": "edge"\|"blob"` (default `document`) | `{"name":"users","status":"created"}`; duplicate → `409 CollectionAlreadyExists`. An unknown `type` string is accepted silently |
| `GET /_api/database/{db}/collection` | | `{"collections":[{"name","count","type","stats":{…},"versioning","autoIndex"}]}` |
| `GET /_api/database/{db}/collection/{name}/count` | | `{"count":3}` |
| `GET /_api/database/{db}/collection/{name}/stats` | | `{"document_count", "disk_usage":{…}, "sharding":{…}, "cluster":{…}, …}` |
| `PUT /_api/database/{db}/collection/{name}/truncate` | | `{"collection","database","deleted":1,"status":"truncated"}` |
| `DELETE /_api/database/{db}/collection/{name}` | | `204` |

Other collection routes exist (`/compact`, `/recount`, `/repair`, `/prune`,
`/properties`, `/export`, `/import`, `/schema`) — not exercised here.

### Field rules (build after 2.13.0)

What a collection asks of a top-level field; checked on every write (document
API, SDBQL, driver, transactions, import) and, unlike a JSON schema, without
slowing the driver's writes. Observed on a build of `main` after 2.13.0
(2026-11-17):

```text
PUT /_api/database/{db}/collection/{name}/rules      (replaces the rules; Write)
{"rules":[{"field":"name","required":true,"type":"string"},
          {"field":"price","type":"number","min":0,"max":1000},
          {"field":"status","in":["open","paid"]}]}
→ 200 {"rules":[…as stored: min/max come back as floats, 0.0…]}
GET    …/rules → 200 {"rules":[…]}        ({"rules":[]} when none)
DELETE …/rules → 204
```
- Keys of a rule: `field` (top-level, no dot, no leading `_`), `required`
  (present and not null), `type` (`string` `number` `integer` `bool` `object`
  `array`), `min` / `max` (numbers), `in` (strings, numbers, booleans). A
  field left out or null passes everything but `required`.
- A write that breaks one: `400 {"code":400,"error":"Schema validation
  failed: 'price' must be at least 0","type":"InvalidDocument"}` — the same
  through SDBQL (`UPDATE "a" WITH {price: 2000} IN goods` → `'price' must be
  at most 1000`). Nothing is written, a whole batch included.
- A rule that asks for nothing → `400 The rule on 'x' asks for nothing`; an
  unknown key in a rule → `422` (body does not deserialize).
- Documents already stored are not checked; an update of one is.
- Rules and the schema (`/schema`) are replicated and sent to a joining
  node from the same build on (checked by tests, not observed on a live
  cluster); a replicated write is not checked against them. `solidb-dump`
  writes both; `solidb-restore` declares them after the data.
- A JSON schema (`/schema`) is checked in a transaction and in an import
  with `mode=upsert` from the same build on; before, both stored what a
  plain insert refused.

## Documents

| Method & path | Notes | Response |
|---------------|-------|----------|
| `POST /_api/database/{db}/document/{coll}` | Body = document; `_key` optional (UUIDv7 generated). **Creates the collection if missing.** | Full stored doc incl. `_key`, `_id` (`"db:coll/key"`), `_rev`, `_created_at`, `_updated_at`. Existing `_key` → `409 ConflictError` |
| `GET /_api/database/{db}/document/{coll}/{key}` | | Doc, or `404 {"code":404,"error":"<key>","type":"DocumentNotFound"}` |
| `PUT /_api/database/{db}/document/{coll}/{key}` | **Merge** top-level fields (a `null` value stores `null`) | Updated doc; missing key → `404` |
| `PUT …/{key}?replace=true` | Replace the whole document | Updated doc |
| `PUT …/{key}?upsert=true` | Create when missing | Doc |
| `PUT …/{key}` + `If-Match: <_rev>` | Optimistic CAS | Mismatch → `409 "Document 'k' has been modified. Expected revision …"`. On a sharded collection: honoured after 2.13.0 for an update (`400 "If-Match is not supported on sharded collections"` through 2.13.0), still `400` with `?replace=true` |
| `DELETE /_api/database/{db}/document/{coll}/{key}` | | `204` |

There is no `PATCH` route (`405`). `…/document/{coll}/_batch` is an internal
shard endpoint (requires `X-Shard-Direct` and cluster credentials) — use an
SDBQL `FOR d IN @docs INSERT d INTO coll` for bulk inserts.

## Queries (cursor)

`POST /_api/database/{db}/cursor`

```json
{"query": "FOR u IN users FILTER u.age > @a RETURN u", "bindVars": {"a": 18},
 "batchSize": 1000, "cache": true}
```

`bindVars` (alias `bind_vars`), `batchSize` (alias `batch_size`, default
1000) and `cache` (default `true`) are optional.

```json
{"result":[1,2],"count":5,"has_more":true,"id":"01a16832-…","cached":false,
 "executionTimeMs":0.04,"inserted":0,"updated":0,"deleted":0}
```

- Next batch: `PUT /_api/cursor/{id}` → `{"result":[3,4],"count":2,"has_more":true,"id":…}`;
  after the last batch the cursor is gone (`404 "Cursor not found or expired"`).
- Drop early: `DELETE /_api/cursor/{id}` → `204`.
- **Cache:** a second identical query answers from the query cache with
  `"cached": true`, **all rows, no `id`, `batchSize` ignored**. Pass
  `"cache": false` when paging.
- A database that does not exist is not an error for a query that touches no
  collection (`RETURN 1` works on `/_api/database/nodb/cursor`).

Errors: `400 ParseError`, `500 ExecutionError` (unknown function, missing bind
var, arity), `404 CollectionNotFound`, `409 ConflictError`,
`400 InvalidDocument` (unique-index violation).

### Explain

`POST /_api/database/{db}/explain` with the same body → planner info without
side effects of interest:

```json
{"collections":[{"name":"people","variable":"p","access_type":"index_lookup",
  "index_used":"idx_age","index_type":"Persistent"}],
 "filters":[…], "documents_scanned":4, "documents_returned":4, "warnings":[]}
```

`access_type` is `full_scan` when no index applies. After 2.7.2 `filters`
also lists the FILTER that runs after an index read, with
`documents_before`/`documents_after`, and `documents_returned` counts the
rows that survive it (and a COLLECT's groups); through 2.7.2 that was
`filters: []` and every row the index returned.

## Indexes

| Method & path | Body | Response |
|---------------|------|----------|
| `POST /_api/database/{db}/index/{coll}` | `{"name":"idx_age","fields":["age"]}`; `"field":"x"` also accepted; `"type"`: `persistent` (default; `skiplist`/`btree` are aliases), `hash`, `fulltext`, `bloom`, `cuckoo`; `"unique": true`; fulltext: `min_length`, `fold_accents` | `{"name","field","fields","type":"Persistent","unique":false,"status":"created"}`; unknown type → `400 "Unknown index type: geo"` |
| `GET /_api/database/{db}/index/{coll}` | | `{"indexes":[{"name","fields","index_type","unique","unique_values","indexed_documents"}]}` |
| `DELETE /_api/database/{db}/index/{coll}/{name}` | | `204` |
| `PUT /_api/database/{db}/index/{coll}/rebuild` | | (unverified) |
| `POST /_api/database/{db}/index/{coll}/check` | | since 2.10.1; read-only, needs Read. `{"documents":3,"consistent":true,"indexes":[{"name","index_type","fields","unique","entries":3,"missing":{"count":0,"sample":[]},"stale":{…},"duplicates":{…}}],"not_checked":[…geo, fulltext, ttl, vector names]}` (observed on a build of main, 2026-11-14). Driver command `check_indexes` |
| `POST /_api/database/{db}/geo/{coll}` | `{"name":"geo_loc","field":"loc"}` | `{"name","field","type":"geo","status":"created"}` |
| `GET /_api/database/{db}/geo/{coll}` | | `{"indexes":[{"name","field","precision":6,"indexed_documents"}]}` |
| `POST /_api/database/{db}/geo/{coll}/{field}/near` | `{"lat":48.86,"lon":2.33,"limit":5}` | `{"results":[{"document":{…},"distance":559.97}]}` (meters, nearest first) |
| `POST /_api/database/{db}/geo/{coll}/{field}/within` | `{"lat","lon","radius":1000}` | same shape |
| `POST /_api/database/{db}/vector/{coll}` | `{"name":"emb_idx","field":"emb","dimension":3,"metric":"cosine"}` (optional `m`, `ef_construction`, `quantization`) | `{"name","field","dimension","metric","quantization":"none","type":"vector","status":"created","indexed_vectors":3,…}` |
| `GET /_api/database/{db}/vector/{coll}` | | `{"indexes":[{name,field,dimension,metric,m:16,ef_construction:200,…}]}` |
| `POST /_api/database/{db}/vector/{coll}/{index}/search` | `{"vector":[1,0,0],"limit":2}` | `{"results":[{"doc_key":"a","score":1.0,"document":{…}}],"count":2}` |
| `POST /_api/database/{db}/ttl/{coll}` | `{"name":"ttl_exp","field":"expires_at","expire_after_seconds":0}` | `{"name","field","expire_after_seconds","type":"ttl","status":"created"}` |
| `GET /_api/database/{db}/ttl/{coll}` | | `{"indexes":[…]}` |

### Column blocks of a time series collection (from 2.11.0; observed on a build of `bench/timeseries`, 2026-11-15)

A `timeseries` collection can keep the fields its aggregations read as
arrays, beside its documents. Not on 2.10.4 or earlier: the routes do not
exist there.

| Method & path | Body | Response |
|---|---|---|
| `PUT /_api/database/{db}/collection/{name}/blocks` | `{"time":"ts","fields":["host","cpu"]}` — `time` holds integer milliseconds; fields are top-level, strings or numbers | `{"time","fields","blocks":0,"opaque_blocks":0,"sealed_documents":0,"documents":0}`. A collection that is not `timeseries` → `400 BadRequest "… is not a timeseries collection"`; the time among the fields, a dotted or `_` field → `400` |
| `GET …/blocks` | | the same status; no definition → `400 "'m' keeps no column blocks"` |
| `POST …/blocks/seal` | | `{"sealed":1, …status}`: seals every document not in a block, the newest included. After a bulk load, a restart or a large delete |
| `DELETE …/blocks` | | `204`; the documents are untouched |

- Sealing happens by itself as documents arrive (1,024 at a time, the
  newest 512 left for later), so right after 3,000 inserts the status shows
  `"sealed_documents":2488`; `…/blocks/seal` brings it to 3,000.
- A query is answered from the blocks only in this shape:
  `FOR p IN c FILTER <time compared with numbers, AND declared fields == strings> COLLECT [k = p.field | k = TIME_BUCKET(p.time, "5m")] AGGREGATE <fn>(p.field) | COUNT()`.
  EXPLAIN then warns `COLLECT folded from the column blocks of 'm'`. Any
  other FILTER (`p.cpu > 3`, `OR`, `!=`), a `TIME_BUCKET` with options or a
  week/month interval, an `INTO`, a SORT/LIMIT before the COLLECT: the
  ordinary path, same answers. Results are identical either way
  (`AVG` 12.25 over mixed `12` / `12.5` values observed on both).
- **Row queries read fewer documents** (after 2.13.0; observed 2026-11-17
  on a build of it, a million points, 500 series): `FOR p IN c FILTER
  p.<time> == @t RETURN p`, a span `>= @a AND < @b`, or either with
  `p.series == @s`, read only the blocks that can hold such a point —
  1,453 ms → 15.5 ms for one instant, 1,491 → 45 ms for a series over an
  hour, 0.4 ms for a series that does not exist. Same rows, same order,
  any other FILTER still applies. EXPLAIN warns `read by its column
  blocks` and counts only the documents read. `FILTER p.series == @s` alone is NOT
  helped (861 ms, every block holds the series): keep an index on the
  series field for that. Through
  2.13.0 every row query without an index reads the whole collection.
- One more shape, over the driver only (observed on a build of `main`,
  2026-11-16): `FOR p IN c FILTER p.series == @s SORT p.<time> DESC LIMIT 1
  RETURN <only declared fields and the time>` reads the newest row of the
  series from the blocks — when `sealed_documents == documents`, `series`
  is the only string field declared, and no other point of the series has
  that instant. Otherwise the `[series, time]` index answers, same result;
  that index is still required (the planner starts from it).
- `opaque_blocks` counts ranges where a declared field is missing or null
  or the time is not an integer: read as documents. Integers and floats in
  one field are fine.
- A delete or a point with an old key throws away the block it falls in;
  until the next insert or `…/blocks/seal` the query runs the ordinary way.
- Node-local: neither the definition nor the blocks are replicated or
  dumped.

### Columnar collections (from 2.11.0 unless said otherwise; observed on a build of `main` after 2.10.4, commit `eeec4093`, 2026-11-15)

A table with a fixed list of columns, stored as arrays, for aggregations
over many rows. Not a document collection: it has its own routes, no `_key`
(a row has a UUIDv7 id the insert returns), and the document routes answer
`404 CollectionNotFound` for its name. It shows in the collection list as
`_columnar_<name>`.

| Method & path | Body | Response |
|---|---|---|
| `POST /_api/database/{db}/columnar` | `{"name":"m","columns":[{"name":"host","type":"string"},{"name":"cpu","type":"float64"}],"compression":"lz4"}` — types `int64`, `float64`, `string`, `bool`, `timestamp`, `json`; compression `lz4` (default) or `none` | `{"status":"created","name":"m","columns":2}`; the name taken → `409 CollectionAlreadyExists` |
| `GET /_api/database/{db}/columnar` | | `{"collections":[{name, columns, compression, row_count, created_at}], "count"}` |
| `GET …/columnar/{c}` | | `{name, columns, compression, row_count, created_at, last_updated_at, stats}` |
| `DELETE …/columnar/{c}` | | `{"status":"deleted","name"}`; absent → `404` |
| `POST …/columnar/{c}/insert` | `{"rows":[{"host":"a","cpu":1.5}, …]}` | `{"status":"ok","inserted":3,"ids":[…]}` |
| `POST …/columnar/{c}/aggregate` | `{"column":"cpu","operation":"SUM","group_by":["host"],"filter":{"column":"day","op":"GTE","value":1764547200000}}` — `SUM`, `AVG`, `COUNT`, `MIN`, `MAX`, `COUNT_DISTINCT`; `group_by` and `filter` optional. `filter` is the one `/query` takes, **from 2.12.1 only: through 2.12.0 the field is ignored without an error and every row is aggregated** — use SDBQL `FILTER … COLLECT` there. Also through 2.12.0 `group_by` reads row by row (2 minutes for 3.65 M rows; 0.3 s from 2.12.1). No `STDDEV` / `MEDIAN` / `PERCENTILE` / `APPROX_COUNT_DISTINCT` here (→ `400`, by decision, card #959): they are SDBQL aggregates | `{"result":7.0}`; with `group_by`: `{"result":null,"groups":[{"host":"a","_agg":4.0}, …]}` — a group key that reads as a number comes back a float (`"metric_id":42.0`) |
| `POST …/columnar/{c}/query` | `{"columns":["host","cpu"],"filter":{"column":"host","op":"EQ","value":"a"},"limit":10}` — one comparison: `EQ NE GT GTE LT LTE IN` or the symbols. **On a `timestamp` column, through 2.12.1: `GT GTE LT LTE` return no row when the values are date strings, and `EQ` matches only the exact text inserted** — store epoch integers and compare with integers there. After 2.12.1 the column is compared by instant (ISO 8601 text, any offset, or seconds / milliseconds), and a range against a non-date → `400` | `{"result":[{"cpu":1.5,"host":"a"}],"count":1}`; an unknown column → `400` |
| `POST …/columnar/{c}/index` | `{"column":"host","index_type":"bitmap"}` — `sorted` (default), `hash`, `bitmap`, `bloom` (`minmax` existed through 2.11.0; from 2.12.0 → `400 "The minmax index type was removed…"`, and a stored one is dropped when its collection is loaded) | `{"status":"created","column","index_type"}` once built; already indexed → `400`. From 2.12.1 (observed on a build of it, 2026-11-16): `?wait=false` → `202 {"status":"building","job","column","index_type","rows_done","rows_total","started_at"}`, built on a server thread and resumed after a restart; `?rebuild=true` builds it again over the rows as they are (200, or 202 with `wait=false`) |
| `GET …/columnar/{c}/index/{column}` (from 2.12.1) | | while building: the `202` body with `rows_done` moving (`status` `building`, or `failed` + `error`); then `{"status":"created","column","index_type","created_at","rows","size_bytes"}` (+ `job` if built in the background by this process); neither → `404` |
| `GET …/columnar/{c}/indexes` · `DELETE …/columnar/{c}/index/{column}` | | `{"indexes":[{column, index_type, created_at}],"count"}` — from 2.12.1 each index also has `rows` (how many rows it answers for) and `size_bytes` (after 2.13.0 also `collection_rows` and `missing`, the rows inserted since its build: rebuild with `POST …/index?rebuild=true` when it grows; after `2ecd4982` an insert that leaves an index with under half of the rows, and 4,096 or more outside it, starts that rebuild itself in the background, on the node that took the insert — from the test suite), and the body a `builds` array of unfinished background builds · `{"status":"deleted","column"}` (from 2.12.1 also calls off a running build) |

- **Deleting rows** (from 2.11.0; observed on a build of `main`,
  2026-11-15; through 2.10.4 these routes do not exist and the only way to
  remove a row is to drop the collection):
  `DELETE …/columnar/{c}/row/{id}` → `{"status":"deleted","id"}`, absent →
  `404 DocumentNotFound`; `POST …/columnar/{c}/delete` with `{"ids":[…]}`
  or `{"filter": …}` (one comparison, or `{"and":[…]}` / `{"or":[…]}` of
  them; not both, not neither → `400`) → `{"status":"ok","deleted":1}`,
  unknown ids passed over; `PUT …/columnar/{c}/truncate` (Admin) →
  `{"status":"truncated","name"}`, columns kept. Delete many rows in one
  call: a block is rewritten once per call. All three replicate. A delete
  by filter of 1.81 M rows out of 3.65 M: 9.5 s on 2.12.0, 0.15 s from
  2.12.1.
- **What a columnar index buys** (observed 2026-11-16, 3.65 M rows):
  little. 7,300 rows of one value of an int column: 6.6 ms with a sorted
  index, 8.3 ms without; an index on a column that follows insertion
  order (a day, a sequence) brings nothing, block bounds already skip. An
  index covers the rows present when it was built (`rows` in
  `GET …/indexes`); later rows are compared in the arrays, a delete in
  the middle cuts it to the rows before. From 2.12.1 two sorted indexes
  on those rows take 7.6 MB (56 MB on 2.12.0) and build in 1.2 s.
- **Retention** (from 2.12.0; observed on a build of `main`,
  2026-11-16): `PUT …/columnar/{c}/retention` with `{"seconds":2592000}`
  (Admin; `{"seconds":null}` keeps every row; `0` → `400`) →
  `{"name":"m","retention_secs":2592000}`. Rows older than that, counted
  from when each was inserted, are removed by a background pass once a
  minute; `POST …/columnar/{c}/prune` does it now →
  `{"status":"ok","deleted":6000}`. `GET …/columnar/{c}` shows
  `retention_secs`. Whole blocks go without being rewritten. Replicated as
  a setting; each node removes its own rows. On 2.12.0 an index on a
  column is not used after a removal until recreated (filters still
  answer right); from 2.12.1 it goes on answering, as after a
  `POST …/delete` that removes exactly the oldest rows.
  Through 2.11.0 the routes do not exist.
- **Compacting** (from 2.12.0; observed on a build of `main`,
  2026-11-16): `POST …/columnar/{c}/compact` →
  `{"status":"ok","blocks_before":200,"blocks_after":16}` (a million rows
  loaded 5,000 at a time, one second). Merges neighbouring small blocks
  (each insert of ≥ 4,096 rows is its own block; a block holds up to
  65,536). Rows, order, query answers and indexes unchanged; on demand
  only; not replicated. Worth running after a load in small batches or a
  large delete.
- **Columns change after creation** (from 2.12.0; observed on a
  build of `main`, 2026-11-16, 200,000 rows; all Admin, all replicated):
  `POST …/columnar/{c}/columns` with `{"name":"region","type":"string"}` →
  `201 {"status":"added","column":"region"}` (existing rows read `null`
  in it; always nullable; an existing name → `409`);
  `PUT …/columnar/{c}/columns/{column}` with `{"name":"value"}` →
  `{"status":"renamed","column":"value","was":"v"}` (values kept; a row
  naming the old column → `400 unknown column`);
  `DELETE …/columnar/{c}/columns/{column}` → `{"status":"dropped",…}`
  (values gone for good; unknown → `400`; the last column → `400`). A
  column with an index → `409 "… drop the index first"`. Through 2.11.0
  the columns are fixed at creation.
- No update of a row in place (delete and insert again).
- **Row policies**: on 2.13.0 none applies (whoever may read the database
  reads every row): `ROW_POLICY("metrics", …)` answers `404
  CollectionNotFound`, and on `_columnar_metrics` it is stored and never
  applied (observed 2026-11-17). On builds of `main` from `ba99b3cf` to
  `bec7192a` both are refused (`'metrics' is a columnar collection: row
  policies do not apply to it…`). After `bec7192a` (card #955, from the
  test suite, not observed on a server): `ROW_POLICY("metrics",
  "doc.tenant == CURRENT_USER")` is kept and applied to every query that
  reads the collection, admins exempt, with the answers a document
  collection gives; such a query builds rows (slower: no array fold);
  `POST …/columnar/{c}/query` and `…/aggregate` (and the driver's) answer
  `403 "a row policy is declared on the columnar collection …"` to a
  caller it binds. Not checked on `INSERT INTO`.
- Change events: through 2.13.0 (and builds up to `ea48f8ea`) a changefeed
  or a live query does not see columnar writes. After it (card #956; the
  event from the test suite, the WebSocket path not observed): one event
  per insert, delete or truncate, `{"type":"insert","key":"<first id>",
  "data":{"rows":5000,"first":"<id>","last":"<id>"}}`, on the node that
  took the write; it does not carry the rows.
- No triggers, no foreign keys (on it or towards it): from the build after
  2.13.0 both answer `400 '<name>' is a columnar collection: …`; on 2.13.0
  they answer as if the collection did not exist.
- **The schema is checked from 2.11.0** (observed on a build of `main`,
  2026-11-15). An insert is refused whole with `400`, naming the row:
  `row 0: unknown column 'extra'`, `row 1: column 'cpu' is not nullable and
  has no value`, `row 1: must be an object, got an integer`, `… is float64,
  got a string`. Columns are `nullable: false` unless declared
  `"nullable": true`, so declare the optional ones. An integer in a
  `float64` column is stored as a float (`3` comes back `3.0`); `timestamp`
  takes a string or an integer; `json` anything. An unknown type name →
  `400 "Unknown column type: decimal. Supported: int64, float64, string,
  bool, timestamp, json"`. `"indexed": true` in a column definition is
  ignored: use `POST …/index`.
  Through 2.10.4 nothing is checked: `{"host":7,"cpu":"x","extra":true}`
  into `host: string, cpu: float64` is accepted, a row that is not an
  object is skipped without an error, and an unknown type name silently
  creates a `string` column — validate before inserting there.
- `/query` from 2.11.0 (the router's own test on a build of `main`,
  2026-11-15): `{"columns":["v"], "filter": <comparison | {"and":[…]} |
  {"or":[…]}>, "order_by":"v DESC", "offset":3, "limit":3}`; `columns` left
  out = all; `order_by` on a column that is not returned → `400`. Through
  2.10.4 it takes one comparison and `limit` only: `{"and":[…]}` is refused
  (`missing field column`), `order_by` and `offset` are ignored without an
  error.
- **A `FOR` that is not a `COLLECT` is slow through 2.11.0**: every row
  and every column is built before the FILTER or LIMIT (about 1 s per
  query on a million rows of six columns, whatever it asks). From 2.12.0
  (observed on a build of `main`, 2026-11-16, same data): only the columns
  read are built; `FILTER r.col == const` / `< <= > >=` on an integer
  column, `==` on a string or boolean column, with literals or bind
  variables, joined by AND / OR, are answered from the column or its index
  (`LIMIT 10` 0.6 ms). Also from 2.12.0 (observed 2026-11-16,
  card #893) the comparison runs in the arrays of the blocks and a block
  of integers its min/max rule out is skipped, no index needed: one row
  by value 0.6 ms, 100 rows of a range (`r.v >= a AND r.v < b`) 0.8 ms,
  a float range (`r.ms >= 1000 AND r.ms < 1010`, also pushed now) 0.6 ms,
  on a million rows. A count whose whole FILTER is such comparisons
  (`FOR r IN m FILTER r.v >= a AND r.v < b COLLECT WITH COUNT INTO n
  RETURN n`, `RETURN LENGTH(FOR r IN m FILTER … RETURN 1)`) builds no
  row: 500,000 rows counted in 1.2 ms (726 ms the build before); EXPLAIN says
  `columnar_count`. Still row by row: a float `==`, a string range,
  `!=`, a constant of another kind than the column (`r.int_col == 5.0`),
  a filter naming an outer `FOR`'s variable, `SORT … LIMIT`.
- **SDBQL reads it and, from 2.11.0, inserts into it** (observed on a
  build of `main`, 2026-11-15). `FOR r IN m …` works as on a document
  collection. `INSERT {host: "a", v: 1} INTO m` and
  `FOR e IN events INSERT {host: e.host, v: e.ms} INTO m` insert every row
  of the query in one call (`"inserted": 6000` in the response; schema
  errors as above). No `OPTIONS`, no `RETURN NEW`, not in a transaction:
  each `501 OperationNotSupported`. `UPDATE` / `UPSERT` / `REMOVE` → `501
  "'m' is a columnar collection: SDBQL can INSERT into it, but not …"`.
  Through 2.10.4 every SDBQL write → `404 CollectionNotFound`: use
  `POST …/insert`. `LENGTH(m)` → `Variable 'm' not found`.
- **The aggregate shape is folded from the arrays** (fastest; through
  2.11.0 the only fast one): `FOR r IN c [FILTER …] COLLECT [k = r.col]
  AGGREGATE <COUNT|SUM|AVG|MIN|MAX>(r.col) …`. Through 2.11.0 the FILTERs
  had to be `==` with a string, number or boolean and `<` `<=` `>` `>=`
  with a number, joined by `AND`, and a block holding a null went row by
  row. From 2.12.0 (observed 2026-11-16, card #892, a million rows) the
  arrays also take `!=`, `IN [..]`, `OR`, string and boolean ranges,
  `== null` / `!= null`, the constant on the left, columns with nulls,
  and a key `TIME_BUCKET(r.ts, "1h")` (fixed interval, integer column):
  `FILTER r.status IN [404, 500] COLLECT h = r.host …` 1,118 ms → 3 ms,
  an hourly `TIME_BUCKET` 1,067 ms → 10 ms. Through 2.12.0 these still
  go row by row (about a second per million): `NOT`, `NOT IN`, a
  comparison of two columns, a key or aggregate that is another
  expression (`SUM(r.a * r.b)`), `COUNT_DISTINCT`, `TIME_BUCKET` of
  weeks or months or of a string column. From 2.12.1
  (observed 2026-11-16 on a build of `main`, a million rows, same
  answers as 2.12.0): all of those are answered from the arrays too —
  `SUM(r.price * r.qty)` by host 1,269 ms → 6 ms, `NOT IN` 2.3 ms, two
  columns compared 3.1 ms, `COUNT_DISTINCT(r.user)` by status 11 ms,
  `COLLECT k = UPPER(r.host)` 27 ms, `DATE_TRUNC(r.ts, 'hour')` 136 ms,
  monthly `TIME_BUCKET` 264 ms. Still from rows there: a key or argument
  that is a `json` column of mixed kinds, a key mixing integers and
  floats (`r.bytes / 100`), a non-pure function (`RAND()`), aggregates
  other than COUNT/SUM/AVG/MIN/MAX/COUNT_DISTINCT. After 2.13.0
  (observed 2026-11-17 on a build of `main`, 2,000,000 rows grouped by a
  string column of 50 values, card #950) `STDDEV` / `VARIANCE`, `MEDIAN`,
  `PERCENTILE(r.col, p)` and `APPROX_COUNT_DISTINCT(r.col)` are answered
  from the arrays too: `STDDEV` 1,768 → 8 ms, `MEDIAN` 1,773 → 13 ms,
  `PERCENTILE(r.amount, 95)` 12 ms, `APPROX_COUNT_DISTINCT(r.user)` 22 ms
  (exact `COUNT_DISTINCT(r.user)`: 394 ms on 2.13.0 and at commit
  `70fb3de3`, 49 ms after it, card #952). Three percentiles and a median
  of one column in one COLLECT: 22 ms (its numbers are kept once).
  `BIT_AND` / `BIT_OR` / `BIT_XOR` come from the arrays too, one commit
  later. `UNIQUE`, `SORTED_UNIQUE`, `COLLECT_LIST` and `COLLECT … INTO`
  still build rows (about 1.7 s there). Window functions build every
  row too (`ROW_NUMBER() OVER (PARTITION BY r.country ORDER BY r.amount
  DESC)`: 4.4 s on 2.13.0), except, after `ebbd4c01` (card #953,
  observed 2026-11-17), the top-N shape written exactly `FOR r IN c LET
  n = ROW_NUMBER() OVER (PARTITION BY <columns> ORDER BY <columns>)
  FILTER n <= 3 …` (`<`, `== 1` too): 62 ms; `RANK()` stays at 4.4 s. Results are
  the same either way. Through 2.11.0 columnar indexes serve `/query` and
  `/aggregate` only; from 2.12.0 a pushed-down SDBQL FILTER uses a hash,
  sorted, bitmap or bloom index for `==` / `IN`. A range against a number
  never uses an index (a sorted one is not consulted for it: the
  blocks' own bounds do better, 0.8 ms against 485 ms with a sorted index
  before). EXPLAIN of a row-returning `FOR` over a columnar collection
  (observed 2026-11-16): `"access_type": "columnar_scan"`, `"index_used":
  "host (hash)"` when an equality used one, and a warning ``"`m` is
  columnar: 3 of its 6 columns are built, for the rows its FILTER on
  `host`, `v` keeps, found in the column arrays and the index on host
  (hash)"``. The folded aggregate shape reports `columnar_scan` too,
  with `"optimizer: COLLECT folded from the blocks of the columnar
  collection …"`.
- **Transactions**: read refused at `repeatable_read` / `serializable`
  (`501 OperationNotSupported "… use read_committed"`). A columnar write
  is never part of a transaction. Observed 2026-11-17 on a build of
  `main` at `4a664066` (after 2.13.0): `…/transaction/{id}/query` with
  `INSERT … INTO m` → `501`; `…/transaction/{id}/document/m` → `404
  CollectionNotFound`; **`POST …/columnar/m/insert` with the header
  `X-Transaction-ID` was carried out at once (row visible before the
  commit, kept after a rollback), on that build and 2.13.0**. From the
  next commit it, the row deletes and the truncate answer `501
  "inserting into a columnar collection is not supported in a
  transaction"` (from the test suite).
- **Names**: through 2.13.0 and `4a664066` a document collection or a
  materialized view could be created under a columnar collection's name,
  and the reverse (`CREATE MATERIALIZED VIEW m AS FOR r IN m RETURN r` →
  200; queries kept reading the columnar `m`). From the next commit:
  `409 CollectionAlreadyExists` both ways (test suite). A view *over* a
  columnar collection works (`CREATE MATERIALIZED VIEW mv AS FOR r IN m
  COLLECT …`, `REFRESH` re-runs it; its rows are documents).
- **Replication** (from the code and its tests, not observed on a cluster):
  create, insert, drop and indexes are logged for the peers from this build
  on, over HTTP and the driver; before it only HTTP inserts were, so a peer
  never had the collection. Upgrade every node before creating a columnar
  index. A node that joins receives each columnar collection whole in the
  full sync, when both nodes run this build.
- **Creating an index is slow through 2.11.0**: quadratic in the rows
  (0.8 s for 20,000, 11.7 s for 80,000, over a minute for 200,000, the
  collection's writes waiting meanwhile). From 2.12.0 (observed on a
  build of `main`, 2026-11-16): 17 ms for 80,000 rows, 0.2 s for a hash
  index on a million rows of five values, 2.1 s for a sorted index on a
  million distinct values.
- **An index covers the rows present when it was created.** Through 2.10.4
  that made answers wrong: with a `sorted`, `hash`, `bitmap` or `bloom`
  index on `s`, `/query` and `/aggregate` filtered on `s` missed every row
  inserted afterwards (`minmax` and no index answered right). From 2.11.0
  (observed on a build of `main`, 2026-11-15, all five types: insert two
  rows, index, insert two more, both `s == "a"` rows and the new `s == "c"`
  row come back) the rows after those the index holds are compared
  directly, so the answer is the unindexed one. The index still only speeds
  up the rows it holds: create it after loading, and drop and recreate it
  after a large load. One made by 2.10.4 or earlier is not used until
  recreated. On 2.10.4 or earlier: recreate the index after every insert,
  or do without. Dropping an index now clears the column's `index_type`.

### Sharded columnar collections (from 2.15.0; observed on a two-node build of `main`, 2026-11-18, card #899)

`POST …/columnar` with `"num_shards":4,"shard_key":"host","replication_factor":1`
(the key must be a column, else `400`) → `{"status":"created",…}`; the
shards appear as columnar collections `m_s0`…`m_s3` spread over the nodes;
`m` itself lists `row_count: 0`. `POST …/columnar/m/insert` and SDBQL
`INSERT … INTO m` route rows to their shards from any node; `FOR r IN m …`
answers over every shard from any node (a `COLLECT` of COUNT / SUM /
AVG / MIN / MAX is computed on each shard and only its groups travel;
other queries read every shard's rows; results equal a single
collection's); `DELETE …/columnar/m` drops the
shards everywhere. Refused on `m` for now (`"'m' is sharded: … is not
supported on it yet"`): deletes, truncate, columns, indexes, retention,
compact, tags, branches, `/query`, `/aggregate`.

### Exports as Parquet and Arrow (from 2.15.0; observed on a build of `main`, 2026-11-18, card #980)

`GET …/collection/{name}/export?format=parquet` → one zstd Parquet file
(`application/vnd.apache.parquet`), `?format=arrow` → an Arrow IPC stream;
no `format` (or `jsonl`) → JSON lines as before; anything else → `400
"Unknown export format …: jsonl, parquet or arrow"`. `GET …/columnar/{c}/export`
(Parquet by default) writes `_id` then the schema's columns, `timestamp`
columns as `timestamp[ms, UTC]`. Columns of a document collection: `_key`
first, no `_id` / `_rev`, `_created_at` / `_updated_at` as timestamps, ints
→ `int64`, ints and floats → `float64`, a vector-indexed field →
`fixed_size_list<float>[dim]`, a list of numbers → `list<double>`, mixed
kinds / objects / other lists → JSON text. Read back with
`pyarrow.parquet.read_table`, `pa.ipc.open_stream`. Sharded and blob
collections → `400` (JSON lines only). A server built without the
`export-arrow` feature answers `400`.

### Tags of a columnar collection (from 2.15.0; observed on a build of `main`, 2026-11-18, card #981)

A name for a columnar collection's rows as they are now; queries read them
later as they were, and a restore brings them back. Local to the node: not
replicated, not dumped.

| Method & path | Body | Response |
|---|---|---|
| `POST …/columnar/{c}/tags` | `{"tag":"q3"}` — 1 to 128 of `[A-Za-z0-9_.-]` | `201 {"tag":"q3","created_at":…,"row_count":3,"blocks":1}`; taken twice → `409` |
| `GET …/columnar/{c}/tags` | | `{"tags":[{tag, created_at, row_count, blocks}]}` |
| `POST …/columnar/{c}/tags/{tag}/restore` | — Admin; refused while the node replicates | `{"status":"restored","rows":3}`; rows and schema become the tag's, indexes answer for no row until rebuilt |
| `DELETE …/columnar/{c}/tags/{tag}` | — Admin | `{"status":"dropped"}` |

```sdbql
FOR r IN m OPTIONS {tag: "q3"} COLLECT AGGREGATE s = SUM(r.v) RETURN s   -- [6] while m sums to 16
```

**Branches** (same build): `POST …/columnar/{c}/branches {"branch":"trial","from":"q3"}`
(no `from`: a tag `~trial` is taken for it) → `201 {"status":"created","name":"trial","rows":3}`;
`GET …/columnar/{c}/branches` → `{"branches":["trial"]}`. The branch is a
columnar collection read and written as any (`FOR r IN trial`), starting
with the tag's rows; its writes and the parent's never meet. `GET
…/columnar/trial` shows `"branch_of":{"collection":"m","tag":"~trial","implicit":true}`.
Dropping the parent while a branch exists → `409 "'m' has branches (trial):
drop them first"`; dropping the branch drops its implicit tag. A branch has
no tag or branch of its own, its columns cannot be dropped or renamed, and
it is not replicated.

`tag` takes a literal string. One query reads a collection at one tag or
as it is: `FOR a IN m OPTIONS {tag:"q3"} FOR b IN m …` → `400 "A query reads
'm' at one tag, or as it is, not both"`. On a document collection → `400`;
an unknown tag → `404`. No index is used at a tag.

### Columnar collections from 2.11.0 (observed on builds of `main`, 2026-11-15)

Same endpoints, driver commands and results as before; what changed is
underneath. Rows are stored in blocks of typed arrays instead of one key
per cell, and a collection written by 2.10.4 or earlier is converted the
first time it is loaded.

- `FOR r IN c FILTER <column compared with a constant, AND …> COLLECT k = r.column … AGGREGATE COUNT() | SUM | AVG | MIN | MAX (r.column)`
  is computed on the arrays (2.4 s → about 1 ms for a filtered sum over a
  million rows). `!=`, `OR`, a function in a key or a `RETURN` of rows
  still build every row first: same answers, slow on a large collection.
- A float inserted a few rows at a time is read back exactly; through
  2.10.4 some values lost their last digit (`93.33333333333333` came back
  `93.33333333333331`).
- The blocks are held in memory once read (about 140 bytes a row for six
  columns), with no ceiling yet.

### Foreign keys (since 2.10.0; observed on builds of its branch, 2026-11-13)

A child collection declares that a top-level field holds the `_key` of a
document of a parent collection of the same database, or null. Not before
2.10.0: the endpoints do not exist there and nothing checks a reference.
The HTTP and SDBQL answers below were seen on a live server; `set_null`,
`restrict`, the driver and transactions are from the test suite
(`tests/foreign_key_tests.rs`).

| Request | Body | Answer |
|---|---|---|
| `POST /_api/database/{db}/collection/{coll}/foreign-keys` | `{"field":"customer","references":"customers","on_delete":"restrict"\|"cascade"\|"set_null","name":"…"}` (`on_delete` defaults to `restrict`, `name` to `fk_<field>`) | `201 {"name":"fk_customer","field":"customer","references":"customers","on_delete":"cascade","index":"fk_customer"}`; stored documents that break it → `409 "Foreign key violation: 1 document(s) of 'orders' refer through 'customer' to a document that is not in 'customers' (first keys: o0)"`, nothing kept |
| `POST …/foreign-keys/check` | `{"field","references"}` | `{"count":1,"sample":[{"_key":"o0","customer":"zed"}]}` — what a declaration would be refused for |
| `GET …/foreign-keys` | | `{"collection","foreign_keys":[…],"referenced_by":[{"collection":"orders","name",…}]}` |
| `DELETE …/foreign-keys/{name}` | | `204`; the index stays |

- A write whose field names a missing parent → `409 "Foreign key violation:
  'customer' in orders refers to 'zed', which is not a document of
  'customers'"`, from the document API, SDBQL `INSERT`/`UPDATE`, the driver
  and a transaction's commit alike. Null or absent is accepted; a value that
  is not a string is refused.
- Deleting a parent (`REMOVE "ann" IN customers`, `DELETE …/document/…`):
  `restrict` → `409`; `cascade` deletes the children in the same write (the
  query still reports `"deleted":1`); `set_null` nulls their field.
- The parent collection cannot be dropped (`409 "Collection 'customers' is
  referenced by the foreign key 'fk_customer' of 'orders': drop that foreign
  key first"`), nor truncated while the child holds documents (`409`), and
  the key's index cannot be dropped (`409`).
- The index is a persistent or hash index on that single field, reused if it
  exists, else created as `fk_<field>`.
- `"by": "id"`: the field holds the parent's `_id`, `customers/ann` or
  `shop:customers/ann`. Default for `_from` / `_to` on an edge collection:
  `{"field":"_from","references":"customers","on_delete":"cascade"}` →
  `{"name":"fk_from","field":"_from","by":"id",…,"index":"_edge_from_idx"}`;
  an edge to a missing vertex → `409 "Foreign key violation: '_to' in
  follows refers to 'zed', which is not a document of 'customers'"`.
  `set_null` is refused on `_from` / `_to`.
- Import into a collection that has a key: checked document by document
  (a child before its parent → the batch is refused, `"failed"` in the
  answer). After 2.13.0 (from the test suite): `POST
  …/collection/{c}/import?foreign_keys=defer` (Admin of the database)
  stores the documents, then answers `{"status":"imported","count":2,
  "failed":0,"orphans":[{"foreign_key":"fk_customer","field":"customer",
  "references":"customers","count":2,"sample":[…]}]}`; nothing is taken
  back. Combines with `mode=upsert`.
- Both ends at once (build after 2.13.0): `POST …/foreign-keys/ends`
  `{"from":"people","to":"firms","on_delete":"cascade"}` → `201
  {"from":{…"fk_from"…},"to":{…"fk_to"…}}`; both or neither (checked by
  tests, 2026-11-17).
- An end over several vertex collections (after 2.13.0, from the test
  suite): `POST …/foreign-keys` `{"field":"_to","references":["firms",
  "people"],"on_delete":"cascade"}` → `201 {"foreign_keys":[{"name":
  "fk_to_firms",…,"also":["people"]},{"name":"fk_to_people",…}]}`, all or
  none; with `name`, or `"by":"key"` → `400`. An edge to another
  collection → `409`. Any field holding an `_id` (`"by":"id"`) can take a
  list.
- Not on sharded collections (`501`). On a replicating node the key's
  definition and the cascaded writes are replicated; a replicated write is
  not checked again.
- `solidb-dump` writes the keys (`"_type":"foreign_key"` records) and
  `solidb-restore` declares them after the documents, also with `--drop`
  (seen: restore into a new database, over itself with `--drop`, and with
  `--overwrite`).
- Driver commands `list_foreign_keys`, `create_foreign_key`,
  `delete_foreign_key`, `check_foreign_key`; the Rust client has methods of
  those names. No other client library has them yet.
- The Soli ORM's `belongs_to` / `dependent:` do not declare one: they still
  work in the application.

The `DELETE …/{name}` forms exist for geo, vector and ttl indexes too.
Details and query-side usage: indexes-search.md.

### Counter fields (from 2.12.0; observed on a build of `main`, 2026-11-16, one node and two nodes)

A `document` or `edge` collection declares that a field is numbered by the database:
every insert gets the next number, never twice the same and with no gap (a
refused insert, a rollback or a refused commit takes none). Not on 2.11.0
or earlier: the routes answer 404.

| Request | Body | Response |
|---|---|---|
| `POST /_api/database/{db}/collection/{coll}/counters` | `{"field":"number","start":41,"reset":"day"\|"week"\|"month"\|"year","timezone":"Europe/Paris","prefix":"INV-{YYYY}-{MM}-","suffix":"","padding":4,"durable":true}` (only `field` is required; `start` 1, `timezone` UTC, `durable` true) | `201 {"field":"number","start":41,…,"node":null,"index":"counter_number","state":{"epoch":…,"date":null,"next":41}}` |
| `GET …/counters` | | `{"collection","counters":[{…,"state":{"epoch","date":"2026-11-16","next":1289}}]}` |
| `PUT …/counters/{field}/next` | `{"next":1288}` | the record; a number a document already holds → `409` |
| `PUT …/counters/{field}/node` | `{"node":"n2"}`, or `{"force":true}` | the record. A handover is asked of the node that numbers; `force` is asked of the node that takes the counter when the numbering node is lost, and goes on after the highest number that node holds (a number given to a document not yet replicated is given again) |
| `DELETE …/counters/{field}` | | `204`; the field and its unique index stay |

- Listing is Read; every change is **Admin**.
- Value: an integer with no prefix, suffix or padding (`41`), else a string
  (`"INV-2026-11-0041"`). Placeholders: `{YYYY}` `{YY}` `{MM}` `{DD}`
  `{WW}`. With `reset`, prefix/suffix must name the period, else `400 "A
  counter that restarts needs its period in the prefix or suffix ({YYYY}
  and {MM}): its numbers would repeat otherwise"`. After a reset the count
  starts at 1; `start` is used once.
- `INSERT {…} INTO c RETURN NEW.number` and `FOR i IN 1..3 INSERT … RETURN
  NEW.number` return the numbers (`["INV-2026-11-0042",…]`); the document
  API's response holds the field; the driver's `insert` / `bulk_insert`
  are numbered too.
- The client does not set it: an insert with the field → `400 "'number' is
  a counter of 'invoices': the database assigns it, leave it out"` (null is
  the same as absent); an update that changes it → `400 "… it cannot be
  changed"`. A replace that leaves it out keeps the number.
- **In a transaction the number is given at commit**: `RETURN NEW` and the
  transaction's own reads show the document without the field. Read it
  back after the commit.
- Inserts into such a collection are stored one at a time, and with
  `durable` (the default) each waits for an fsync, shared by the inserts
  waiting together (both engines; with `SOLIDB_DURABILITY=full` on
  solistore every insert waits for it whatever `durable` says, ~430 / 1,600 /
  3,500 a second from 1 / 8 / 32 clients on a build after 2.13.0, where 2.13.0
  stayed at ~450/s): measured over HTTP on a workstation
  (NVMe), single inserts ran at ~410/s from 1 client, ~1,700/s from 8 and
  ~3,500/s from 32, against ~6,500/s and ~10,000/s with `durable:false`.
  Batch (`FOR … INSERT`, bulk insert, one transaction) when loading many:
  one write, one fsync.
- Declaring on a collection that already holds numbers: refused `409` when
  two documents hold the same value or one holds the first number; give
  `start`, or set `next` afterwards. Documents without the field are left
  alone.
- An import in upsert mode / a merge by key keeps the number of a document
  it merges into and numbers one it creates; neither may carry the field
  (`POST …/collection/{c}/import?mode=upsert` with `-F file=@docs.jsonl` →
  `{"status":"imported","count":3,"failed":0}`; a refused document →
  `"failed":1,"error":"… the database assigns it, leave it out"`. Through
  2.11.0 the answer has no `error`, and the last line of a file with no
  final newline is dropped without being counted).
- Cluster: one node numbers (`node`, the one that got the declaration; it
  must run with `--node-id`). Elsewhere an insert → `409 "The counter
  'number' of 'invoices' is numbered by node 'n1': insert there"`. Hand it
  over with `PUT …/node` on the node that numbers. Not on sharded
  collections.
- Driver: `list_counters`, `create_counter {counter: {…}}`,
  `delete_counter`, `set_counter_next`, `move_counter`.
- `solidb-dump` / `solidb-restore` carry the counter and where it stands.

## Transactions

| Method & path | Body | Response |
|---------------|------|----------|
| `POST /_api/database/{db}/transaction/begin` | `{}` or `{"isolationLevel":"read_committed"}` | `{"id":"tx:1792749567411353000","isolationLevel":"ReadCommitted","status":"active"}` |
| `POST …/transaction/{tx_id}/document/{coll}` | document | staged doc |
| `GET` / `PUT` / `DELETE …/transaction/{tx_id}/document/{coll}/{key}` | | sees the transaction's own writes |
| `POST …/transaction/{tx_id}/query` | `{"query", "bind_vars"}` (`bindVars` too after 2.9.0) | `{"message":"1 operation(s) staged in transaction. Commit to apply changes.","mutationCount":1,"result":[…]}`; a read-only query: `{"result":[…]}` |
| `POST …/transaction/{tx_id}/commit` | | `{"id","status":"committed"}` |
| `POST …/transaction/{tx_id}/rollback` | | `{"id","status":"aborted"}` |

- `{tx_id}` is the full `"tx:…"` string from `begin`.
- Writes are invisible outside the transaction until commit (a plain `GET`
  answers `404`), and discarded on rollback.
- Alternative: send the normal document routes with header
  `X-Transaction-ID: tx:…` — the write is staged in that transaction.
- `PUT …?replace=true` is refused inside a transaction.
- **Up to 2.9.0 the query endpoint reads only `bind_vars`**: `bindVars` (the
  `/cursor` spelling) is dropped without an error, and an unbound variable in a
  `FILTER` matches nothing — a filtered `UPDATE` sent that way updates nothing
  and reports success (observed on 2.9.0). Send `bind_vars`, which every
  version reads. Its writes are staged. Its reads: through 2.9.3,
  committed data only; from 2.9.4 through 2.10.1, a document read by key
  (`FILTER d._key == @k`, `DOCUMENT()`) reads as staged and nothing else
  does (an `INSERT` then `FOR d IN c FILTER d.f == @x` misses the new
  document); after 2.10.1, scans, indexed FILTERs, SORT/LIMIT, counts,
  COLLECT, joins and subqueries see the transaction's writes too — and a
  collection the transaction wrote to is scanned, not read by index, until
  the commit. `FULLTEXT`, vector/geo searches, graph traversals and `AS OF`
  still read committed data.
- **Isolation levels** (`isolationLevel`: `read_committed` default,
  `repeatable_read`, `serializable`, `read_uncommitted`). Through 2.9.3 the
  level only decides whether the commit is fsynced. After 2.9.3,
  `repeatable_read` and `serializable` (alike) record what the transaction's
  queries read and check it at commit: a document read by key must still be
  at that revision (or still absent); a collection read by scan or index
  must not have been written at all since. Otherwise commit → `409`
  `TransactionConflict` ("Read conflict: …"), nothing written: roll back and
  retry. Re-reading a changed key fails that statement with `409` at once.
  **On a cluster, through 2.13.0 the writes of a transaction are not
  replicated** (stored on the node that took the commit only; fixed in the
  build after, checked by tests 2026-11-17).
  Sharded collections cannot be read at these levels. A columnar
  collection can from the build after 2.13.0 (refused on 2.13.0): the
  commit answers `409` when any row of it was inserted or deleted since
  the read (checked by tests on a build of `main`, 2026-11-17).
- **Locks.** Through 2.9.3 a document lock held by another transaction is
  refused at once with `409`. After 2.9.3 it is waited for, up to
  `SOLIDB_TX_LOCK_WAIT_MS` (500 ms; `0` = refuse at once); a deadlock is
  refused at once; then `409` ("… (waited 500 ms)" / "… (deadlock: …)").
  Writes at `repeatable_read` / `serializable` do not wait.
  Through 2.10.1 a waiter could be overtaken by later arrivals until its
  wait ran out (a 409 under a steady stream of writers to one document);
  after 2.10.1, arrivals queue behind a transaction that has waited a
  quarter of the wait.
  Write contended documents in one order (ascending key) in every
  transaction: then none deadlocks, each waits its turn, none gets a `409`.
- **Driver (MessagePack) transactions** are per connection: a write is staged
  only when sent as `{"cmd": "transaction_command", "tx_id", "command": {…}}`
  on the connection that began it. The SDKs did not do that up to 2.9.0 —
  `begin_transaction` then `insert` wrote at once and survived a rollback
  (all seven SDKs; fixed after 2.9.0, Elixir through
  `SoliDB.Client.in_transaction/2` / `transaction/3`).
- **A query that writes inside a driver transaction**: refused up to 2.9.1
  ("a query that writes cannot run inside a transaction"). Since 2.9.2 it is
  staged like `…/transaction/{tx}/query`, revision guard included (a stale
  `UPDATE` → an error containing "conflict": roll back, retry). End a
  conditional write with `RETURN 1` and read the rows to know whether it
  matched — the driver sends no `mutationCount`. Over the driver a five-step
  transfer runs about twice as fast as over HTTP (bank benchmark,
  2026-11-09).
- **A whole transaction in one driver request (since 2.9.3)**:
  `{"cmd": "run_transaction", "database", "steps": [{"command": {…}, "require_rows": bool}]}`
  — begin, steps, commit on the server. Answers `{"committed": true,
  "results": [...]}` or `{"committed": false, "stopped_at": i}` (step `i`
  had `require_rows` and returned no row: rolled back). A failing step rolls
  back with an error naming it ("step 2: …"); a commit-time refusal (e.g. a
  duplicate key) does not name one. Rust: `client.run_transaction(db,
  vec![TransactionStep::query(…).require_rows(), …])`. Refused while a
  `begin_transaction` is open on the same client.

## Constraints on a cluster

A unique index and a foreign key are checked by the node that takes the
write; replication applies it elsewhere unchecked. Two nodes accepting
conflicting writes at once leave a duplicate or an orphan on every node
(by design, decided 2026-11-17). Find them with `POST
/_api/database/{db}/index/{coll}/check` (`duplicates`) and `POST
…/collection/{coll}/foreign-keys/check`. After 2.13.0 (from the test suite,
not observed on a server): `SOLIDB_CONSTRAINT_CHECK_SECS=3600` runs both on
a schedule and reports `solidb_fk_orphans{database,collection}` and
`solidb_unique_duplicates{database,collection}` on `/metrics`, plus a
warning in the log. Nothing is repaired.

## Backups and point-in-time restore

Observed 2026-11-17 on a build of `main` after 2.13.0 (throwaway server,
solistore). The checkpoint (`POST /_api/backup`) exists since 2.x; the log
position, the log archive and the replay are after 2.13.0 (card #852).

| Method & path | Body | Response |
|---------------|------|----------|
| `POST /_api/backup` (global Admin) | `{"path":"cp"}` — relative to `SOLIDB_BACKUP_ROOT` (default `<data-dir>/backups`), must not exist | `{"status":"created","path":"/…/backups/cp","scope":"instance","log_position":{"sequence":11,"node_id":"…","log_active":true,"taken_at_ms":1794946970927},"note":"…"}`; the checkpoint also holds a `SYNC_LOG_POSITION` file with that object |
| `POST /_api/backup/log` (global Admin) | `{"path":"cp.log","after":11}` — entries after that sequence | `{"status":"created","path","after":11,"entries":10,"first_sequence":12,"last_sequence":21,"first_timestamp":…,"last_timestamp":…,"complete":true}`; `complete:false` when entries between `after` and the oldest one kept were pruned; an existing file → `400`; log not written (`SOLIDB_SYNC_LOG_KEEP_SECS` unset, no peer) → `400` |

- Keep the history: start the server with `SOLIDB_SYNC_LOG_KEEP_SECS=604800`
  (the log is then written from the start and kept that long; one more log
  write per document write).
- Restore to an instant: copy the checkpoint directory, then
  `solidb --data-dir <copy> --replay-log <archive> --replay-until
  2026-11-17T14:02:00Z --replay-only` (RFC 3339 or epoch milliseconds;
  without `--replay-until` the whole archive; without `--replay-only` the
  server then serves). Log line: `Point-in-time restore: 7 entries replayed
  … (0 already in the checkpoint, 3 after the instant asked for)`. Observed:
  an update, a delete and a collection created before the instant are there;
  writes after it are not. The position file is renamed
  `SYNC_LOG_POSITION.replayed`; a second `--replay-log` on that directory is
  refused.
- Not restored this way: sharded collections, writes made while the log was
  not kept. Not observed: a restore on RocksDB, transactions, columnar rows
  and blobs in the replay.

## Other route families (not exercised)

Blobs (`/_api/blob/{db}/{coll}` upload/download, chunked uploads), cluster
(`/_api/cluster/*`), triggers, Lua scripts and services (`/api/{db}/{service}`,
off with `--no-lua`), SQL (`POST /_api/database/{db}/sql`), graph RAG, AI agents, sync, changefeed
websocket (`/_api/ws/changefeed`), `GET /metrics`.
