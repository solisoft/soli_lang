# SoliDB / SDBQL traps

Each item was reproduced on `solidb 2.4.0` (and Soli 2.28.2 where Soli is
involved).

## Query language

1. **Errors inside `FILTER` are swallowed.** The row is dropped and the query
   succeeds. Through 2.9.4 that included the two mistakes below; observed on
   2.9.5, they fail the query (500 `Unknown
   function: NOPE_FN`, 500 `Bind variable '@min' not found…`), in a subquery
   too. A type error on one row still only drops that row. Through 2.9.4:
   ```sdbql
   FOR u IN users FILTER NOPE_FN(u.name) RETURN u      -- [] (no error)
   FOR u IN users FILTER u.age > @min RETURN u         -- [] when @min is not bound
   RETURN NOPE_FN(1)                                   -- 500 "Unknown function: NOPE_FN"
   RETURN @x                                           -- 500 "Bind variable '@x' not found…"
   ```
   When a filter "matches nothing", move the expression into `RETURN` to see
   the error.
2. **`UPSERT` finds its document by `_key`/`_id`, or (since 2.10.1) by a
   unique index.** Through 2.10.0, `UPSERT {email: @e} INSERT … UPDATE … IN
   users` inserts a new document on every run, and `OLD` inside the `UPDATE`
   expression fails (`Variable 'OLD' not found`). Since 2.10.1 (observed on
   a build of main, 2026-11-14), with a unique hash or persistent index on
   exactly the searched fields:
   ```sdbql
   UPSERT {email: @e} INSERT {email: @e, n: 1} UPDATE {n: OLD.n + 1} IN users RETURN {n: NEW.n, was: OLD.n}
   -- run 1: {"n":1,"was":null}   run 2: {"n":2,"was":1}   one document
   ```
   `OLD` is bound in the UPDATE expression, by key too (`UPSERT {_key: "k"}
   … UPDATE {n: OLD.n + 1}` → 1, then 2). Concurrent runs lose no increment
   (test suite). **Still a trap:** a search that matches no unique index
   (`UPSERT {city: "Lyon"} …` with no unique index on `city`, or part of a
   composite one) inserts every time. On 2.10.0 and earlier use a
   deterministic key and `DOCUMENT()`:
   ```sdbql
   UPSERT {_key: @k} INSERT {_key: @k, n: 1} UPDATE {n: DOCUMENT("c/" + @k).n + 1} IN c RETURN NEW.n
   ```
   or, for a non-key field, a unique index plus `INSERT … OPTIONS
   {ignoreErrors: true}` (skips the duplicate; `overwriteMode: "ignore"`
   only covers `_key` clashes and still raises the unique violation), or
   insert-if-absent:
   ```sdbql
   LET existing = FIRST(FOR s IN subs FILTER s.email == @e LIMIT 1 RETURN s)
   FOR x IN (existing == null ? [1] : []) INSERT {email: @e} INTO subs RETURN NEW.email
   ```
3. **No `@@collection` binds.** `@@c` is a parse error
   (`Expected bind variable name after '@'`); `FOR d IN @c` iterates the
   string and returns `[]`. Whitelist the name and splice it into the text.
4. **`NOT a == b` parses as `(NOT a) == b`** → usually no rows. Use `a != b`
   or `!(a == b)`.
5. **Comments are `--` only.** `//`, `/* */`, `#` are parse errors.
6. **`ALL x IN arr SATISFIES …` does not parse** (`ANY` and `NONE` do). Use
   `EVERY(arr, x -> …)`. Conversely `ANY(arr, x -> …)` as a function does not
   parse — use `SOME(…)`.
7. **`_id` contains the database:** `"shop:users/alice"`. `FILTER u._id ==
   "users/alice"` misses; filter on `_key`. (`DOCUMENT("users/alice")` and edge
   `_from`/`_to` use `coll/key`.) Through 2.9.4 a traversal started from
   `u._id` returned `[]` and from `u` failed (`Start vertex must be a
   string`); after it, `OUTBOUND u._id edges`, `OUTBOUND u edges` and
   `OUTBOUND "users/alice" edges` all work, also for `SHORTEST_PATH`, and
   edges written with `_from: a._id` are followed. `FILTER e._to == u._id`
   in a plain FOR over the edges still misses: compare with
   `CONCAT("users/", u._key)`.
8. **`UPDATE` does not deep-merge.** `UPDATE k WITH {o: {b: 2}}` on
   `{o: {a: 1}}` leaves `{o: {b: 2}}`. Add `OPTIONS {mergeObjects: true}`.
9. **`UNION` deduplicates** (`UNION([1,2],[2,3])` → `[1,2,3]`), unlike AQL.
10. **Numbers:** math/aggregates return floats (`FLOOR(4.8)` → `4.0`;
    through 2.9.4 also `SUM([1,2,3])` → `6.0` and `SUM` over nothing →
    `-0.0`; after it `6` and `0`, in the function, `COLLECT … AGGREGATE` and
    `SUM(…) OVER`, a float among the values giving a float); `1 / 0` → `null`;
    `TO_NUMBER("x")` → `0`; `1 == "1"` → `false`.
11. **`[1..10]` is `[[1,…,10]]`.** The range literal is `1..10` (inclusive).
12. **Empty arrays are truthy** in `FILTER` (`FILTER u.tags` keeps `tags: []`).
    Test `LENGTH(u.tags) > 0`.
13. **Computed object keys `{[k]: v}` are not supported.**
14. **No `DELETE` statement** — `REMOVE`.
15. **`LENGTH(users)` (a bare collection name) is an error** —
    `Variable 'users' not found`. Use `COLLECTION_COUNT("users")` or
    `LENGTH(FOR u IN users RETURN 1)`.
16. **`SHORTEST_PATH` puts the direction after the target:**
    `FOR v IN SHORTEST_PATH "a/1" TO "a/9" OUTBOUND edges RETURN v`.
17. **`FULLTEXT` on a field without a fulltext index returns `[]`**, not an
    error. `NEAR` without a geo index does raise.
18. **`SLEEP()` does not exist** (listed in SoliDB's own reference).
19. **Through 2.5.1, subqueries inside a `FOR` re-run per row in several
    shapes; bind them with a `LET` first on those versions.** 2.5.0 ran
    `FILTER x IN (FOR …)` once per row; 2.5.1 reuses a subquery only when it
    reads nothing from the row, so a nested one that reads an outer loop's
    row (`FOR col … (FOR m … FILTER m.card_id IN (FOR c … FILTER c.column_id
    == col._key …))`) still runs per inner row (7.2 s vs 12 ms hoisted, 168
    columns × 2k comments). **2.5.1 only: a subquery using `COLLECT … INTO g`
    without `KEEP` returns the first row's groups on every row** — add `KEEP`
    or hoist it. Unreleased after 2.5.1: a read-only, deterministic subquery
    in a read-only query runs once per distinct set of outer values it reads,
    its result is shared rather than copied, `x IN (subquery)` can use an
    index on `_key`/`_id`/Persistent/Hash, and an unindexed correlated
    `FILTER v.f == outer.x` hashes the collection once per query. Writing
    queries still evaluate subqueries per row.
37. **Through 2.6.4, an index on `f` drops the rows where `f` is null or
    missing** from `f < x`, `f <= x`, `f >= null` and from a `SORT f` read in
    index order (ASC, with or without LIMIT; DESC without LIMIT, or with one
    reaching past the indexed rows). Observed on 2.6.4 with `{f:1}`,
    `{f:null}`, `{}`, `{f:9}`: `FILTER d.f <= 5` → `[k1]` with the index,
    `[k1, k2, k3]` without; `SORT d.f ASC LIMIT 2` → `k1, k4` instead of
    `k2, k3`. A regular index is sparse (no entry for an all-null row) and
    SDBQL ranks null below every value. On those versions, add
    `AND d.f != null` when nulls should not match anyway, or query the null
    rows separately. Fixed after 2.6.4: such a read is only served by a
    sparse index when null cannot be in the result (another term rejects it;
    for ASC, every document is in the index), otherwise the query scans;
    create the index with `"sparse": false` to keep it indexed.
38. **Through 2.7.2, `COLLECT … LIMIT n` with no SORT and no FILTER grouped
    only the first n documents**: the LIMIT was pushed into the scan.
    `FOR d IN c COLLECT k = d.k LIMIT 2 RETURN k` returned the keys of the
    first two documents (`[null, 1]` where the groups were `[null, false, …]`),
    and `COLLECT WITH COUNT INTO n LIMIT 1` counted one document. After 2.7.2 (3,000 tickets, 2 statuses):
    `["closed","open"]` and `[3000]`. On older versions add a `SORT` after
    the COLLECT.
39. **Through 2.7.2, a full-scan COLLECT held the whole collection in
    memory** and its documents counted against `SOLIDB_MAX_INTERMEDIATE_ROWS`
    (5M): a larger collection could not be grouped, and 100k documents of
    500 B got a 1 GB container OOM-killed. Fixed after 2.7.2 (sdbql.md,
    COLLECT); on older versions filter through an index first or aggregate
    in slices.
40. **Through 2.7.2, `d["a.b"]` could miss documents in a scan that decodes
    only the fields a query reads**: it read the key `a.b` but not the path
    `a` → `b`. After 2.7.2, with `{"a.b": 1}` and `{a: {b: 1}}`,
    `FILTER d["a.b"] == 1` → both.
41. **Through 2.9.0, `UPDATE d WITH {n: d.n + 1}` lost concurrent
    updates**, with or without a transaction, at every isolation level
    (card #796): the `WITH` value was computed from the document the `FOR`
    read, then merged under the key's lock with no `_rev` check. Ten
    clients appending with `PUSH` on 8 hot keys lost 24,785 of 405,507
    committed appends in 10 s (one in sixteen). Since 2.9.1, when
    `UPDATE`/`REPLACE` names a document the query read (`UPDATE d`, `UPDATE d._key`), a row whose document moved
    on is evaluated again under the key's lock (its FILTERs, LETs, then
    WITH) and skipped if it no longer matches; a pipeline with a nested FOR,
    COLLECT or join between the read and the write fails with `409` instead.
    Inside a transaction, through 2.9.3 a moved-on document always failed
    the statement with `409` (roll back, retry); from 2.9.4, at
    `read_committed`, the write waits for the other transaction's lock (up
    to `SOLIDB_TX_LOCK_WAIT_MS`, 500 ms) and the row is evaluated again on
    what it committed, as outside a transaction; a deadlock is still a
    `409`, at once.
    `UPDATE "literal-key" WITH …` is not guarded: it names no document the
    query read. On 2.9.0 and earlier, guard a hot counter with an HTTP
    transaction over the document routes (`GET` then `PUT`, which lock the
    key; retry on 409). Through 2.9.3 the isolation level only decides
    whether the commit is fsynced.
42. **Through 2.9.3 a transaction does not protect what it only read
    (write skew), at any isolation level; from 2.9.4 only at
    `read_committed`** (begin with `repeatable_read` or `serializable` and
    the second commit gets `409`, see http-api.md). Observed on 2.9.3 at
    `serializable`: two
    doctors on call, two transactions each running `FOR d IN doctors
    FILTER d._key == @me LET n = LENGTH(FOR x IN doctors FILTER
    x.on_call == true RETURN 1) FILTER n >= 2 UPDATE d WITH {on_call:
    false} IN doctors` for a different doctor — both commit, nobody is on
    call. A query in a transaction reads committed data with no lock, and
    the commit re-checks only the documents written. To enforce a rule
    across documents, read them by key through the transaction's document
    routes (`GET …/transaction/{id}/document/…` takes a shared lock: the
    other transaction's write then gets `409`, roll back and retry), or
    write every document the decision reads. Elle (Jepsen's checker) on
    list-append histories with the #796 fix: the query path is read
    committed at every level — the same document read twice in one
    transaction can return two values — while the document routes were
    serializable. Also: `on` is reserved in
    SDBQL (`LET on = …` → "Expected variable name after LET"). As a field
    name a keyword did not parse through 2.9.4 (`RETURN {on: 1}`, `{to: x}`,
    `d.to`: quote it or use `d["to"]`), and `{sort: 1}` made a field `order`
    while `d.order` read `sort`; since 2.9.5 any keyword is a field name as
    written, in an object and after a dot.

43. **Through 2.10.0 a statement of up to 100 rows is not atomic.** Outside
    a transaction, `FOR k IN ["a","b","taken"] INSERT {_key: k} INTO c`
    with `taken` existing fails with `409` but leaves `a` and `b` stored;
    same for `UPDATE` / `REMOVE`. Above 100 rows it was already one batch,
    with looser rules: an `UPDATE` row breaking the schema or naming a
    missing document was skipped silently, a `REMOVE` of a missing key
    ignored. Since 2.10.1 (observed on a build of main, 2026-11-14): a
    plain `INSERT` / `UPDATE` / `REMOVE` after a `FOR` is one batch from
    two rows up and fails whole: the insert above leaves only `taken`;
    `FOR k IN ["taken","nope"] REMOVE k IN c` → `404 nope`, nothing
    removed. **Still row by row** (an error can leave earlier rows): any
    `OPTIONS` (`ignoreErrors` skips per row on purpose), `REPLACE`,
    `UPDATE … RETURN NEW`, `RETURN OLD` on UPDATE/REMOVE, `UPSERT`, sharded
    collections. Use a transaction for those.

## HTTP / server

20. **Query cache vs paging:** an identical repeated query returns
    `"cached": true` with every row, no cursor `id`, `batchSize` ignored.
    Send `"cache": false` to page.
21. **The document endpoint auto-creates collections**
    (`POST /document/newcoll` succeeds); SDBQL `INSERT INTO newcoll` → 404.
22. **No `PATCH`.** `PUT` merges; `?replace=true` replaces; `?upsert=true`
    creates. `replace=true` is refused inside a transaction.
23. **Vector metric spelling:** `cosine`, `euclidean`, `dot`/`dotproduct`.
    `"dotProduct"` or a typo silently creates a **cosine** index.
24. **TTL needs epoch seconds or an RFC 3339 string.** Epoch milliseconds
    never expire (read as seconds, far in the future). Through 2.10.0 a
    float (e.g. from `FLOOR(DATE_NOW()/1000)` → `….0`) never expires either;
    since 2.10.1 it expires like the integer it truncates to.
25. **Wrong-dimension vectors are accepted** on insert without error.
26. **Unknown collection `type` strings are accepted** at creation.
27. **`solidb` reads `.env` from the working directory or a parent.** A Soli
    project's `SOLIDB_HOST=http://localhost:6745` becomes the bind host and
    the server exits with `failed to lookup address information`. Start it with
    `--host 127.0.0.1`, or from another directory.
28. **Default port 6745 is probably someone's real database.** Run tests on a
    throwaway instance (`--port 6799 --data-dir /tmp/…`).

## Soli ORM

29. **Invalid SDBQL fails differently per terminator:** `.all`/`.count`
    raise; `.first` returns `nil`; `@sdbql{}` returns an `"Error: Query
    failed: …"` **String**; an unknown function in a `where` string gives `[]`.
30. **`.sum/.avg/.min/.max(field)` return `nil`** — they generate
    `RETURN SUM(doc.field)` per row. Use
    `.aggregate({"total": ["sum", "field"]}).first`.
31. **`.exists` and `.pluck` return QueryBuilders** — `.exists.first`,
    `.pluck("f").all`.
32. **`record.id` is `nil` on loaded records** — use `record._key`. On an
    instance returned by `create`, `id` is set and a later `save` persists an
    `id` field in the document.
33. **`find_or_create_by` is not atomic** (query, then insert). Use a unique
    index if duplicates matter.
34. **`{"field": nil}` in a `where` hash matches documents missing the field.**
35. **`{"like": …}` is case-sensitive on SoliDB; `{"ilike": …}` is not.**
36. **`Model.similar` needs the vector index to be named `idx_<field>`.**

## Where the docs disagree with the binary

- SoliDB `docs/SDBQL_REFERENCE.md`: lists a `DELETE` clause (no such
  statement); uses `//` comments in SDBQL examples (parse error); documents
  `UPSERT {id: 1} … UPDATE {val: OLD.val + 1}` (through 2.10.0: search by
  non-key field never matches and `OLD` is unbound in UPDATE; the example
  now searches by `_key`); shows `ALL x IN … SATISFIES` (parse
  error); lists `SLEEP` (unknown); example `PERCENTILE([1..100], 95)` passes a
  nested array; the vector-index example posts to
  `/_api/database/mydb/collection/docs/vector-index` (real route:
  `POST /_api/database/{db}/vector/{coll}`).
- Soli `www/docs/solidb-reference.md`: recommends `@@name` collection binds
  (`{"@coll": "users"}`) — SoliDB rejects them; says `.exists` returns a
  Boolean and `.pluck` an Array, and lists `.sum/avg/min/max` as numeric
  aggregates — they return QueryBuilders, and the sums are `nil`.
- Soli `www/docs/models.md`: says SoliDB `ilike` is "also LIKE — no separate
  case-folding"; it compiles to `LIKE(x, p, true)`, which is
  case-insensitive (`{"ilike": "t%"}` matched "Two"/"Three", `like` did not).
