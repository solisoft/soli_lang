# SDBQL syntax reference

Every example here was run against `solidb 2.4.0`; results are shown as the
`result` array the cursor endpoint returns. Test data: `emp` =
`{ann: eng/100/tags[a,b]/boss null, bob: eng/80/tags[b]/boss ann, cid: ops/60/tags[]/boss ann, dee: ops/70/no tags/boss cid}`,
`orders` = `{o1: ann/10, o2: ann/5, o3: bob/7}`, edge collection `knows` =
`ann→bob, ann→cid, cid→dee`.

## Lexical

- Keywords are case-insensitive (`for e in emp filter … return …` works).
- Comments: `-- to end of line` only. `//`, `/* */` and `#` are parse errors.
- Strings: `"double"` or `'single'`. Numbers, `true`/`false`/`null`.
- Arrays `[1, 2]`, objects `{a: 1, "b-c": 2, name}` (shorthand `{name}` =
  `{name: name}`). Computed keys `{[k]: v}` → `Expected field name in object`.
- Identifiers in backticks are not supported.
- Statement must start with `FOR`, `LET`, `RETURN`, `INSERT`, `UPDATE`,
  `REMOVE` (also `UPSERT`, `REPLACE`, `WITH` CTEs, `CREATE`/`REFRESH
  MATERIALIZED VIEW`). Typos produce
  `Unexpected token: Identifier("RETRN"). Expected FOR, LET, RETURN, …`.

## Clauses

| Clause | Form | Notes |
|--------|------|-------|
| `FOR` | `FOR x IN coll` / `IN [array]` / `IN 1..10` / `IN (subquery)` / `IN @arr` | `FOR x IN coll OPTIONS { indexHint: "name" }` accepted |
| `FILTER` | `FILTER expr` | Repeatable (ANDed). Evaluation errors inside drop the row silently; since 2.9.5 an unknown function or a missing bind variable fails the query instead |
| `LET` | `LET x = expr` | Also after `SORT`/`LIMIT` (then computed only for surviving rows; not visible to earlier clauses) |
| `SORT` | `SORT e1 [ASC\|DESC], e2 …` | Stable. Any expression |
| `LIMIT` | `LIMIT n` / `LIMIT offset, n` | Bind vars allowed: `LIMIT @o, @n` |
| `COLLECT` | see below | |
| `RETURN` | `RETURN expr` / `RETURN DISTINCT expr` | A query with only writes and no RETURN gives `result: []` |
| `JOIN` | `[LEFT\|RIGHT\|FULL [OUTER]] JOIN coll ON cond` | Binds `coll` to the **array** of matches per row |
| `WITH` (CTE) | `WITH name AS (subquery) FOR t IN name …` | `WITH RECURSIVE name AS (anchor UNION ALL step)` |
| Materialized view | `CREATE MATERIALIZED VIEW mv AS <query>` / `REFRESH MATERIALIZED VIEW mv` | Stored as a collection `mv` you can `FOR` over; refresh re-runs the query |
| Writes | `INSERT` / `UPDATE` / `REPLACE` / `REMOVE` / `UPSERT` | see Writes |

```sdbql
FOR e IN emp FILTER e.dept == "eng" SORT e.salary DESC RETURN e.name
-- ["Ann","Bob"]
FOR e IN emp SORT e.dept ASC, e.salary DESC LIMIT 1, 2 RETURN e.name
-- ["Bob","Dee"]
FOR e IN emp SORT e.salary LIMIT 2 LET bonus = e.salary * 0.1 RETURN {k: e._key, bonus}
-- [{"bonus":6.0,"k":"cid"},{"bonus":7.0,"k":"dee"}]
FOR e IN emp LET total = SUM(FOR o IN orders FILTER o.emp == e._key RETURN o.total)
  SORT e._key RETURN {k: e._key, total}
-- ann 15.0, bob 7.0, cid -0.0, dee -0.0   (through 2.9.4; after it, integers
--   add to an integer and nothing to 0: ann 15, bob 7, cid 0, dee 0)
WITH top AS (FOR e IN emp FILTER e.salary > 75 RETURN e) FOR t IN top SORT t._key RETURN t._key
-- ["ann","bob"]
WITH RECURSIVE nums AS (RETURN 1 UNION ALL FOR n IN nums FILTER n < 3 RETURN n + 1) FOR n IN nums RETURN n
-- [1,2,3]
FOR x IN (FOR u IN users RETURN u.age) SORT x RETURN x
```

### COLLECT

Sections after the group keys may appear in any order: `INTO`, `WITH COUNT
INTO`, `AGGREGATE`, then optional `OPTIONS { method: "hash"|"sorted" }`.

```sdbql
FOR e IN emp COLLECT dept = e.dept WITH COUNT INTO n SORT dept RETURN {dept, n}
-- [{"dept":"eng","n":2},{"dept":"ops","n":2}]
FOR e IN emp COLLECT dept = e.dept
  AGGREGATE total = SUM(e.salary), avg = AVG(e.salary), mx = MAX(e.salary), c = COUNT()
  SORT dept RETURN {dept, total, avg, mx, c}
-- eng: total 180.0, avg 90.0, mx 100, c 2
FOR e IN emp COLLECT dept = e.dept INTO g SORT dept RETURN {dept, names: g[*].e.name}
-- g holds {e: <row>} objects (one key per FOR/LET variable in scope)
FOR e IN emp COLLECT dept = e.dept INTO names = e.name SORT dept RETURN {dept, names}
-- [{"dept":"eng","names":["Ann","Bob"]}, …]
FOR e IN emp COLLECT d = e.dept INTO g KEEP e SORT d RETURN {d, n: LENGTH(g)}
FOR e IN emp COLLECT AGGREGATE total = SUM(e.salary), n = COUNT() RETURN {total, n}
-- [{"n":4,"total":310.0}]
FOR e IN emp COLLECT WITH COUNT INTO n RETURN n          -- [4]
FOR e IN emp FILTER e.salary > 60 COLLECT d = e.dept AGGREGATE s = SUM(e.salary)
  FILTER s > 100 RETURN {d, s}                           -- HAVING = FILTER after COLLECT
-- [{"d":"eng","s":180.0}]
```

**A full-scan COLLECT streams** (after 2.7.2): `FOR x IN c [FILTER …|LET …]*
COLLECT …` folds each document into its group as it is read and decodes only
the fields the clauses use. Memory is the groups (plus what `INTO` and list
aggregates keep); the documents read do not count against
`SOLIDB_MAX_INTERMEDIATE_ROWS`. Through 2.7.2 the whole collection was
materialised first: 1M orders of 500 B took 6.2 s and +5.7 GB (0.92 s and
no growth in 2.7.3), a collection over the ceiling could not be aggregated,
and 100k documents got a 1 GB container OOM-killed. A FILTER served by an
index, columnar, sharded and search-view sources, and `SYSTEM_TIME`/valid-time
loops keep the old path. `COLLECT … LIMIT n` limits groups (traps.md #38).

Since 2.8.0, when the group keys and aggregate arguments are fields of the
loop variable and each FILTER compares a field with a constant or a bind
variable, they are read straight from the stored bytes (no document is
decoded); a collection of 65,536 documents or more is also split into key
ranges read in parallel at one snapshot, for aggregates that merge exactly
(COUNT, SUM, AVG, MIN, MAX, COUNT_DISTINCT, BIT_*) — on RocksDB since 2.8.0,
and on solistore since 2.9.0 (2.8.0 splits only on RocksDB).
Observed on 2.8.0 builds (RocksDB) and a 2.9.0 build (solistore), 70k orders:
```text
FOR o IN orders COLLECT s = o.status AGGREGATE n = COUNT(), total = SUM(o.amount) RETURN {s, n, total}
-- explain warning (both engines): "optimizer: COLLECT folded into the scan of 'orders': documents
--   are grouped as they are read, never held as rows (reading only amount, status from the stored
--   bytes; 2 key ranges read in parallel)"
FOR o IN orders FILTER o.amount > 5 COLLECT s = UPPER(o.status) WITH COUNT INTO n RETURN {s, n}
-- a function in the key: "(decoding only amount, status)"
```
`SOLIDB_SCAN_THREADS` caps the threads (default: the CPUs the process may
use; `1` turns parallel scans off). SUM/AVG are exact (functions.md).

### Window functions

```sdbql
FOR e IN emp SORT e._key
  RETURN {k: e._key, rn: ROW_NUMBER() OVER (PARTITION BY e.dept ORDER BY e.salary DESC)}
-- ann 1, bob 2, cid 2, dee 1
FOR e IN emp SORT e._key
  RETURN {k: e._key, r: RANK() OVER (ORDER BY e.salary DESC), s: SUM(e.salary) OVER (PARTITION BY e.dept)}
-- ann r1 s180.0, bob r2, cid r4 s130.0, dee r3
```

### Joins

Nested `FOR` is the classic join; `JOIN` groups matches into an array.

```sdbql
FOR e IN emp FOR o IN orders FILTER o.emp == e._key SORT o._key RETURN [e.name, o.total]
-- [["Ann",10],["Ann",5],["Bob",7]]
FOR e IN emp JOIN orders ON e._key == orders.emp SORT e._key RETURN {k: e._key, n: LENGTH(orders)}
-- ann 2, bob 1           (inner: rows without matches dropped)
FOR e IN emp LEFT JOIN orders ON e._key == orders.emp SORT e._key RETURN {k: e._key, n: LENGTH(orders)}
-- ann 2, bob 1, cid 0, dee 0
```

### Graph traversal

Edges store `_from`/`_to` as `"collection/key"`. Start vertices use the same form.

```sdbql
FOR v IN 1..2 OUTBOUND "emp/ann" knows SORT v._key RETURN v._key        -- ["bob","cid","dee"]
FOR v, e IN 1..1 INBOUND "emp/dee" knows RETURN [v._key, e._from]       -- [["cid","emp/cid"]]
FOR v IN 1..3 ANY "emp/bob" knows SORT v._key RETURN DISTINCT v._key    -- ["ann","cid","dee"]
FOR v, e, p IN 2..2 OUTBOUND "emp/ann" knows RETURN p.vertices[*]._key  -- [["ann","cid","dee"]]
FOR v IN SHORTEST_PATH "emp/ann" TO "emp/dee" OUTBOUND knows RETURN v._key
-- ["dee"]   (direction comes AFTER the target; AQL's `OUTBOUND SHORTEST_PATH` is a parse error)
```

## Operators and semantics

| Operator | Verified behavior |
|----------|-------------------|
| `==`, `!=` | Type-strict: `1 == "1"` → `false` |
| `< <= > >=` | Cross-type order `null < false < 0 < "" < [] < {}`; strings compare as text (`"10" < "9"` → `true`; ISO dates compare correctly) |
| `IN`, `NOT IN` | `"b" IN e.tags`; `e.dept IN ["ops"]` |
| `LIKE` | `e.name LIKE "A%"`; case-sensitive. Case-insensitive: `LIKE(e.name, "a%", true)` |
| `=~`, `!~` | Regex: `e.name =~ "^[AB]"` |
| `AND OR NOT`, `&& \|\| !` | `null \|\| "fallback"` → `"fallback"`, `0 \|\| 5` → `5`; `null OR "fallback"` → `true` |
| `NOT` precedence | `NOT e.dept == "eng"` is `(NOT e.dept) == "eng"` → matches nothing. Use `!(…)` or `!=` |
| `? :` | `5 > 3 ? "y" : "n"` |
| `??` | `null ?? "d"` → `"d"`; `DOCUMENT("emp/dee").tags ?? []` → `[]` |
| `?.` | `DOCUMENT("emp/nobody")?.name` → `null` |
| `+ - * / %` | `7 / 2` → `3.5`; `7 % 3` → `1`; `1 / 0` → `null`; `"a" + "b"` → `"ab"` |
| `\|>` | `"hello" \|> UPPER() \|> REVERSE()` → `"OLLEH"` |
| `[*]` | `(FOR e IN emp RETURN e)[*].name` → names |
| `[i]` | `[10,20,30][1]` → `20`, `[-1]` → `30` |
| `a..b` | inclusive: `1..3` → `[1,2,3]` (`[1..3]` is a nested array `[[1,2,3]]`) |
| `CASE` | `CASE WHEN 1 > 2 THEN "a" ELSE "b" END` → `"b"` |
| Quantifiers | `FILTER ANY t IN e.tags SATISFIES t == "a"`, `FILTER NONE t IN e.tags SATISFIES t == "b"`. `ALL … SATISFIES` → parse error; use `EVERY(e.tags, t -> t == "b")` |

Truthiness in `FILTER`: a missing field / `null` is false; an empty array
`[]` is **true** (`FILTER e.tags` keeps `cid`, whose tags are `[]`).

`FILTER e.tags == null` matches documents *without* the field.

## Writes

```sdbql
INSERT {_key: "c1", n: 0} INTO counters RETURN NEW          -- NEW = stored doc incl. system fields
INSERT {_key: "c1", n: 0} INTO counters                     -- 409 "Document with _key 'c1' already exists"
INSERT {_key: "c1", n: 99} INTO counters OPTIONS {overwriteMode: "ignore"} RETURN NEW.n   -- [null], unchanged
INSERT {_key: "c1", n: 5} INTO counters OPTIONS {overwriteMode: "update"} RETURN NEW.n    -- [5]  (merge)
INSERT {_key: "c1", n: 6} INTO counters OPTIONS {overwriteMode: "replace"}               -- whole doc
INSERT {_key: "c1"} INTO counters OPTIONS {overwriteMode: "conflict"}                    -- 409 (the default)
UPDATE "c1" WITH {n: 7} IN counters RETURN {old: OLD.n, new: NEW.n}   -- [{"new":7,"old":6}]
UPDATE {_key: "c1", m: 1} IN counters RETURN NEW.m                    -- selector is also the patch
FOR c IN counters FILTER c._key == "c1" UPDATE c WITH {n: c.n + 1} IN counters RETURN NEW.n
REPLACE "c1" WITH {n: 1} IN counters RETURN NEW                       -- other fields gone
REMOVE "c2" IN counters RETURN OLD.n
REMOVE {_key: "c1"} IN counters
FOR c IN counters REMOVE c IN counters RETURN OLD._key
UPDATE "nope" WITH {n: 1} IN counters                        -- error "nope" (DocumentNotFound)
UPDATE "nope" WITH {n: 1} IN counters OPTIONS {ignoreErrors: true}   -- [] no error
```

Options per statement: `ignoreErrors`, `keepNull`, `mergeObjects`, and
`overwriteMode` / legacy `overwrite: true` on INSERT. AQL no-ops
(`waitForSync`, `ignoreRevs`, `exclusive`, …) are accepted and ignored.
Anything else: `Unknown INSERT option 'bogus'. Supported: …`. Option values
must be literals.

- **Nested objects:** `UPDATE … WITH {o: {b: 2}}` on `{o: {a: 1}}` gives
  `{o: {b: 2}}`. With `OPTIONS {mergeObjects: true}` → `{o: {a: 1, b: 2}}`.
  (AQL defaults to merging; SoliDB does not.)
- **Nulls:** `UPDATE … WITH {m: null}` keeps `m: null`; with
  `OPTIONS {keepNull: false}` the field is removed.
- **UPSERT** looks the document up **only by `_key` / `_id`** (the search
  value may also be a bare key string). Any other search object never matches,
  so the INSERT branch runs every time (duplicates). `OLD` is not bound while
  the UPDATE expression is evaluated (`Variable 'OLD' not found`) but is bound
  for the `RETURN`. Counter pattern that works:
  ```sdbql
  UPSERT {_key: @k} INSERT {_key: @k, n: 1} UPDATE {n: DOCUMENT("c/" + @k).n + 1} IN c RETURN NEW.n
  -- 1, then 2, …
  ```
  `UPSERT … REPLACE {…} IN coll` replaces instead of merging.
- `INSERT` into a missing collection → 404 `CollectionNotFound` (the HTTP
  document endpoint, by contrast, auto-creates the collection).
- Edge collections reject documents without `_from` (`Edge document must have a _from field`).
- There is no `DELETE` statement (parse error) — use `REMOVE`.

## Bind variables

```sdbql
FOR e IN emp FILTER e.salary >= @min RETURN e.name          -- bindVars {"min": 80}
FOR e IN emp LIMIT @o, @n RETURN e._key                     -- {"o": 1, "n": 1}
FOR x IN @arr RETURN x * 2                                  -- {"arr": [1, 2]} → [2, 4]
INSERT @doc INTO users OPTIONS {overwriteMode: "update"}    -- {"doc": {...}}
```

- Missing bind var in `RETURN`: error `Bind variable '@x' not found. Did you
  forget to pass it in bindVars?`. Missing in `FILTER`: no error, zero rows.
- `@@coll` collection binds do not exist (parse error). `FOR d IN @c` with
  `c = "users"` iterates the string and returns `[]`.
- Unused bind vars are accepted.

## Cursor result shape

```json
{"result": [...], "count": 5, "has_more": true, "id": "01a1…", "cached": false,
 "executionTimeMs": 0.04, "inserted": 0, "updated": 0, "deleted": 0}
```

`count` is the total on the first batch and the batch length on later ones.
Errors: `{"code": 400, "error": "…", "type": "ParseError"}` (syntax),
`{"code": 500, "type": "ExecutionError"}` (runtime), `404 CollectionNotFound`,
`409 ConflictError`. See http-api.md for paging.
