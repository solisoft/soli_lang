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
| `PUT /_api/auth/password` | `{"current_password","new_password"}` | (unverified) |

With an API key, `CURRENT_USER()` returns `"api-key:<id>"`.

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

## Documents

| Method & path | Notes | Response |
|---------------|-------|----------|
| `POST /_api/database/{db}/document/{coll}` | Body = document; `_key` optional (UUIDv7 generated). **Creates the collection if missing.** | Full stored doc incl. `_key`, `_id` (`"db:coll/key"`), `_rev`, `_created_at`, `_updated_at`. Existing `_key` → `409 ConflictError` |
| `GET /_api/database/{db}/document/{coll}/{key}` | | Doc, or `404 {"code":404,"error":"<key>","type":"DocumentNotFound"}` |
| `PUT /_api/database/{db}/document/{coll}/{key}` | **Merge** top-level fields (a `null` value stores `null`) | Updated doc; missing key → `404` |
| `PUT …/{key}?replace=true` | Replace the whole document | Updated doc |
| `PUT …/{key}?upsert=true` | Create when missing | Doc |
| `PUT …/{key}` + `If-Match: <_rev>` | Optimistic CAS | Mismatch → `409 "Document 'k' has been modified. Expected revision …"` |
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
  version reads. Its writes are staged; its reads see committed data, not the
  transaction's own writes — except, from 2.9.4, a document read by key
  (`FILTER d._key == @k`, `DOCUMENT()`), which reads as staged.
- **Isolation levels** (`isolationLevel`: `read_committed` default,
  `repeatable_read`, `serializable`, `read_uncommitted`). Through 2.9.3 the
  level only decides whether the commit is fsynced. After 2.9.3,
  `repeatable_read` and `serializable` (alike) record what the transaction's
  queries read and check it at commit: a document read by key must still be
  at that revision (or still absent); a collection read by scan or index
  must not have been written at all since. Otherwise commit → `409`
  `TransactionConflict` ("Read conflict: …"), nothing written: roll back and
  retry. Re-reading a changed key fails that statement with `409` at once.
  Columnar and sharded collections cannot be read at these levels.
- **Locks.** Through 2.9.3 a document lock held by another transaction is
  refused at once with `409`. After 2.9.3 it is waited for, up to
  `SOLIDB_TX_LOCK_WAIT_MS` (500 ms; `0` = refuse at once); a deadlock is
  refused at once; then `409` ("… (waited 500 ms)" / "… (deadlock: …)").
  Writes at `repeatable_read` / `serializable` do not wait.
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

## Other route families (not exercised)

Blobs (`/_api/blob/{db}/{coll}` upload/download, chunked uploads), cluster
(`/_api/cluster/*`), triggers, Lua scripts and services (`/api/{db}/{service}`,
off with `--no-lua`), SQL (`POST /_api/database/{db}/sql`), columnar stores
(`/_api/database/{db}/columnar…`), graph RAG, AI agents, sync, changefeed
websocket (`/_api/ws/changefeed`), `GET /metrics`.
