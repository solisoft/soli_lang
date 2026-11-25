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
  `{name: name}`). Computed keys `{[k]: v}`, beside plain ones
  (`{id: 1, [u._key]: u.age}`): from commit `0e02365d`, after 2.13.0 (observed
  2026-11-17 on a build of it); a key that is not a string is made one as
  `TO_STRING` does (`{[1 + 1]: "two"}` → `{"2": "two"}`), a key written
  twice keeps its last value. Through 2.13.0: `Expected field name in
  object`; use `ZIP_OBJECT(keys, values)` there.
- Identifiers in backticks are not supported.
- Statement must start with `FOR`, `LET`, `RETURN`, `INSERT`, `UPDATE`,
  `REMOVE` (also `UPSERT`, `REPLACE`, `WITH` CTEs, `CREATE`/`REFRESH
  MATERIALIZED VIEW`, and, since 3.1.0, `EXPLAIN`). Typos produce
  `Unexpected token: Identifier("RETRN"). Expected FOR, LET, RETURN, …`
  (since 3.1.0 the list reads `FOR, LET, FILTER, QUALIFY, RETURN, INSERT,
  UPDATE, or REMOVE`).

## Clauses

| Clause | Form | Notes |
|--------|------|-------|
| `FOR` | `FOR x IN coll` / `IN [array]` / `IN 1..10` / `IN (subquery)` / `IN @arr` | `FOR x IN coll OPTIONS { indexHint: "name" }` accepted. Since 3.1.0: `SAMPLE n` or `SAMPLE p PERCENT` on the source, before `OPTIONS` (see below) |
| `FILTER` | `FILTER expr` | Repeatable (ANDed). Evaluation errors inside drop the row silently; since 2.9.5 an unknown function or a missing bind variable fails the query instead |
| `QUALIFY` | `QUALIFY expr` | Since 3.1.0. Once, after the body, before `SORT`. See below |
| `EXPLAIN` | `EXPLAIN [ANALYZE] <query>` | Since 3.1.0. One plan row. See below |
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

Aggregate functions: `COUNT()` `COUNT(x)` `LENGTH(x)` `SUM` `AVG` `MIN` `MAX`
`COUNT_DISTINCT` `UNIQUE` `SORTED_UNIQUE` `COLLECT_LIST`/`PUSH` `VARIANCE`
`VARIANCE_SAMPLE` `STDDEV` `STDDEV_SAMPLE` `MEDIAN` `BIT_AND` `BIT_OR`
`BIT_XOR`. **After 2.13.0** (observed on a build of `main`, 2026-11-17) also
`PERCENTILE(x, p)` / `QUANTILE(x, p)`, `PERCENTILE(x, p, "interpolation")`
and `APPROX_COUNT_DISTINCT(x)`, with the results of the array functions of
those names over the group's values:

```sdbql
FOR x IN 1..10 COLLECT AGGREGATE p = PERCENTILE(x, 90), a = APPROX_COUNT_DISTINCT(x % 3),
  s = STDDEV(x), v = VAR_SAMP(x) RETURN [p, a, s, v]
-- [[9.0, 3.0, 2.8722813232690143, 9.166666666666666]]
```

- `p` is a number, a string or a bind variable (`PERCENTILE(o.amount, @p)`);
  an expression of the row → `400 ParseError "The aggregate PERCENTILE takes
  a constant or a bind variable after its first argument"`. A rank outside
  0–100, or no rank → `null`. `SUM(x, 2)` → `400 "The aggregate SUM takes
  one argument"`.
- **Through 2.13.0** `AGGREGATE p = PERCENTILE(x, 95)` is a parse error
  (`Expected RightParen, got Comma`): use `COLLECT k = … INTO g = x RETURN
  PERCENTILE(g, 95)`, which holds every value as a row.
- `STDDEV` / `VARIANCE` after 2.13.0 are computed from exact sums: the same
  number in any order; last digits may differ from 2.13.0 and earlier.
- A `MEDIAN` / `PERCENTILE` keeps its numbers: through 2.13.0 each counts as
  a row against `SOLIDB_MAX_INTERMEDIATE_ROWS` (a median over more than
  5,000,000 values → `500 "Query exceeded the intermediate row limit"`);
  after it, eight numbers count as one row, and several `PERCENTILE` /
  `MEDIAN` of the same expression in one COLLECT keep (and count) its
  numbers once.

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
(COUNT, SUM, AVG, MIN, MAX, COUNT_DISTINCT, BIT_*; after 2.13.0 also the
variance family, MEDIAN, PERCENTILE, APPROX_COUNT_DISTINCT) — on RocksDB since 2.8.0,
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

**`ROLLUP` and `CUBE`** (3.1.0; observed over HTTP on
2026-11-24/25 on builds of the tree released as 3.1.0). The
keyword follows the group variables and precedes `INTO` / `WITH COUNT` /
`AGGREGATE` / `OPTIONS`; written after `AGGREGATE` it is `Unexpected token:
Identifier("ROLLUP")`. Both at once, or a second copy: `400` `ROLLUP and CUBE
each appear once, after the group variables, and not together`. Neither word
is reserved (`LET rollup = 1 LET cube = 2` works).

```sdbql
FOR o IN orders
  COLLECT country = o.country, status = o.status ROLLUP
  AGGREGATE total = SUM(o.amount)
  RETURN { country, status, total, gc: GROUPING(country), gs: GROUPING(status) }
```

Orders `(US, open, 10)`, `(US, paid, 5)`, `(FR, open, 7)` return six rows,
mask then key (null sorts below every value, so detail `FR` then `US`, then
the country subtotals, then the grand total):

| country | status | total | GROUPING(country) | GROUPING(status) |
|---------|--------|-------|-------------------|------------------|
| FR | open | 7 | 0 | 0 |
| US | open | 10 | 0 | 0 |
| US | paid | 5 | 0 | 0 |
| FR | null | 7 | 0 | 1 |
| US | null | 15 | 0 | 1 |
| null | null | 22 | 1 | 1 |

- `ROLLUP` is the `n+1` prefixes. For `(a, b, c)` the masks after the detail
  (0) are 4 (only `c`), 6 (`b` and `c`), 7 (grand total): bit `i` set means
  variable `i` was rolled up. `CUBE` is every mask `1 .. 2^n-1`. The cube of
  the orders above also has `(null, open, 17)` with bits `(1, 0)` and
  `(null, paid, 5)`.
- `GROUPING(country)` is the variable's **name**, not `GROUPING(o.country)`
  (`GROUPING() takes a group variable name, not an expression`). It is not a
  normal builtin: the argument is not evaluated. Outside a `COLLECT`:
  `GROUPING() is only valid on the rows of a COLLECT`. An unknown name:
  `GROUPING(city) is not a group variable of this COLLECT`. Two arguments:
  `GROUPING() takes one group variable, as in GROUPING(country)`. All four
  are `500` `Query execution error: …`. On an ordinary `COLLECT` it is 0.
- `GROUPING` is readable by whatever follows the `COLLECT`: `FILTER`, `LET`,
  `SORT`, `QUALIFY`, `RETURN`. `FILTER GROUPING(status) == 1` keeps the
  subtotals and the grand total. Rows come out detail first; for a report
  with each subtotal under its groups and the grand total last, sort on the
  bits before the keys:
  `SORT GROUPING(country), country, GROUPING(status), status` →
  `FR open`, `FR null`, `US open`, `US paid`, `US null`, `null null`. A plain
  `SORT country, status` puts the grand total first (null sorts first).
- A rolled-up key is JSON `null`. `GROUPING` is what separates that from a
  real null: with a document whose `status` is null, `(US, null, 1)` has bit
  0 and the subtotal `(US, null, 3)` has bit 1.
- `WITH COUNT INTO n` is rolled up like an aggregate (`FR 1`, `US 2`,
  `null 3`).
- Empty input with group variables: **no rows**, no grand total. No group
  variables: the one ordinary row (`ROLLUP` adds nothing), including over
  `[]`. More than 31 group variables: `500` `ROLLUP and CUBE accept at most
  31 group variables`, even when the input is empty.
- Super-group aggregates are merged from the detail groups, in the order
  those groups were created. `COLLECT_LIST` / `PUSH` concatenate (`[10, 5]`
  for `US`, `[7, 10, 5]` for the total); `UNIQUE` keeps first-seen order and
  `SORTED_UNIQUE` sorts at the end. `INTO` members of a subtotal are those
  detail members concatenated, so a grand total holds every input row again
  — memory multiplies by the levels. Of `1` and `1.0` the earlier row's
  value is kept, as a key and in `UNIQUE`.
- From the code, unverified: shards send detail groups (mask 0) only and
  the extra rows are built after the merge; a grouped `ROLLUP` is not an
  index count; the ungrouped columnar array-fold steps aside.

A FILTER that no index serves is still a scan of the collection, but since
the build after 2.13.0 the leading FILTERs that compare fields of the loop
variable with constants or bind variables (`==` `!=` `<` `<=` `>` `>=` `IN`,
`AND` / `OR` / `NOT`) are checked on the stored bytes, and only the documents
they keep are decoded. Observed on a build of `main` after 2.13.0
(2026-11-17, solistore), 70k orders, no index on `customer`:
```text
FOR o IN orders FILTER o.customer == @c RETURN o.amount            -- 140 rows, 8.4 ms
-- explain: access_type "full_scan", documents_scanned 70000, warning
--   "optimizer: FILTER on 'orders' checked on the stored documents: only those it keeps are decoded"
FOR o IN orders FILTER UPPER(o.customer) == UPPER(@c) RETURN o.amount   -- same rows, 48.7 ms, no warning
```
A function call, a comparison of two fields or a value read from another row
keeps that FILTER, and every FILTER after it, on decoded rows: put the plain
comparisons first. On a million documents the plain form took 121 ms where
it took 808 before; with an index it is a few milliseconds, so index a field
you filter on often.

Since 2.13.0 (observed on its release build, RocksDB, 182,500 documents
with a persistent index on `d`):
- `FOR x IN c FILTER x.d >= @f COLLECT m = x.m AGGREGATE s = SUM(x.s) RETURN …`
  no longer builds a row per document the index names. Explain warning:
  "optimizer: COLLECT folded into the index read of 'c': the documents the
  index names are grouped as they are read, never held as rows (reading only
  d, m, s from the stored bytes)". 1,066 ms → 34 ms for every document,
  69 ms → 5 ms for 15,000. Only `>` / `>=` on a single-field index; rows
  keep the index's order.
- A subquery over an array in memory (`LET ds = DOCUMENT(…)` then
  `AVG((FOR x IN ds FILTER x.v[p] != null RETURN x.v[p]))`) costs about a
  microsecond a run instead of five: nesting such subqueries is fine.
- A `LET` or `RETURN` holding a subquery or `DOCUMENT()` is evaluated for
  256 rows or more on the scan threads, when it writes nothing and calls
  only pure functions and reads (not `RAND`, `NOW`, `UUID`), outside a
  transaction. Same rows, same order.
- `FOR x IN c FILTER x.d < @c REMOVE x IN c` (nothing after the REMOVE, no
  OPTIONS, no `OLD`): 90,500 documents removed in 0.30 s instead of 1.39 s.

### Window functions

```sdbql
FOR e IN emp SORT e._key
  RETURN {k: e._key, rn: ROW_NUMBER() OVER (PARTITION BY e.dept ORDER BY e.salary DESC)}
-- ann 1, bob 2, cid 2, dee 1
FOR e IN emp SORT e._key
  RETURN {k: e._key, r: RANK() OVER (ORDER BY e.salary DESC), s: SUM(e.salary) OVER (PARTITION BY e.dept)}
-- ann r1 s180.0, bob r2, cid r4 s130.0, dee r3
```

### QUALIFY, SAMPLE, EXPLAIN (3.1.0)

Not in 3.0.0 (`Unexpected token: Identifier("ROLLUP")` there). Observed over
HTTP on 2026-11-24/25 on builds of the tree released as 3.1.0; the tests are
`tests/sdbql_explain_qualify_rollup_sample_tests.rs`.
The offline parser in `sdbql-core` rejects the new syntax (unverified). None
of `qualify`, `sample`, `explain`, `rollup`, `cube` is a reserved name
(`LET qualify = 1 RETURN qualify` works).

**`QUALIFY`** — one predicate after the body, before `SORT`. It may follow a
`COLLECT` and read its variables (`COLLECT d = e.dept AGGREGATE s =
SUM(e.salary) QUALIFY s > 200`), and needs no `FOR` (`QUALIFY 1 == 1 RETURN
1`). Parse errors (`400`): `SORT` before it; a `COLLECT` or a write
(`INSERT` / `UPDATE` / `REMOVE`) after it (`Unexpected token: Insert. …`);
a second `QUALIFY` (`A query has one QUALIFY clause`). To gate a write on a
window use the `LET` form, which is older and still works:
`LET rn = ROW_NUMBER() OVER (…) FILTER rn == 1 INSERT …`.

```sdbql
FOR e IN emp
  QUALIFY ROW_NUMBER() OVER (PARTITION BY e.dept ORDER BY e.salary DESC) <= 3
  RETURN e
-- the three highest salaries per department
```

Windows in `QUALIFY`, and every window in `RETURN`, are numbered on the rows
the body produced, before the predicate and before `SORT`. The same `OVER`
call is computed once, so a number repeated in `RETURN` is the pre-filter
number: `QUALIFY ROW_NUMBER() OVER (ORDER BY e.salary DESC) > 1 RETURN
ROW_NUMBER() OVER (ORDER BY e.salary DESC)` returns 2 to 6 on six rows,
never 1. A window `LET` in the body is computed during the body and
`QUALIFY` can read it (`LET rn = … QUALIFY rn <= 2`). A *different* window
call that appears only in a `LET` after `LIMIT` is numbered on the survivors
of `QUALIFY`, before `LIMIT` cuts: five survivors and `LIMIT 3` gave ranks 5,
4, 3 (without `QUALIFY` such a `LET` already numbered the rows from before
`LIMIT`, on 2.15.0 too). A query with no `QUALIFY` still numbers `RETURN`
windows after `SORT`; that order differs only for a window with no
`ORDER BY` of its own: `ROW_NUMBER() OVER ()` behind `SORT e.salary` counts
1 to 6 in sorted order without `QUALIFY`, and in the body's order with it.

An error in the predicate fails the query (`500`) where `FILTER` drops the
row: `UPPER() == "X"` → `UPPER requires 1 argument(s)`, and
`DATE_ADD(e.dept, 1, "day") > 1` → `Invalid date string 'eng'`; both give
`[]` in a `FILTER`. A `QUALIFY` with no window is still applied
(`QUALIFY e.salary >= 80 SORT e.salary RETURN e.salary` → 80, 90, 100), and
`RETURN DISTINCT` still removes duplicates at the end. The predicate may
read fields no other clause names (`QUALIFY e.salary >= 80 RETURN e._key` →
`e1`, `e2`, `e3`), of a traversal's vertex and edge too.

A window function written directly in a `FILTER` is not an error and not a
filter: every row is dropped and the result is `[]` (on 2.15.0 too). Write
it in `QUALIFY`, or in a `LET` read by the `FILTER`.

**`SAMPLE`** — on the `FOR` source, after `SYSTEM_TIME` / `VALID_TIME`
(unverified), before `OPTIONS` (`OPTIONS {…} SAMPLE 2` is a parse error).
Before `FILTER`. The whole source is read; later clauses see only the
sample (`SAMPLE 7 COLLECT WITH COUNT INTO n` → 7). Nested `FOR`: once per
outer row. Collections, arrays, ranges and subquery sources, and the loop of
a write (`FOR d IN logs SAMPLE 2 REMOVE d IN logs` → `deleted: 2`).

```sdbql
FOR d IN events SAMPLE 100 RETURN d         -- ≤100 rows, not scan order
FOR d IN events SAMPLE 1 PERCENT RETURN d   -- each row p=0.01, scan order kept
FOR d IN events SAMPLE @n RETURN d          -- also SAMPLE @p PERCENT
FOR x IN [1, 2, 3, 4, 5] SAMPLE 2 RETURN x
```

- `SAMPLE n`: at most `n` distinct rows, not in scan order. `n >=` source
  length returns every row in scan order. `0` returns nothing. `1.0` is 1,
  and an expression is accepted (`SAMPLE 1 + 1`). `1.5`, a negative or a
  string: `500` `SAMPLE count must be a non-negative whole number, got 1.5`.
- `SAMPLE p PERCENT`: each row is kept with probability p/100, scan order
  preserved (`50 PERCENT` of 20 rows returned 13 of them once). `0 PERCENT`
  keeps nothing, `100 PERCENT` keeps every row. Outside 0..=100: `500`
  `SAMPLE percent must be a number from 0 to 100, got 101`.
- Not seeded. Do not assert which rows a count sample returns, except the
  keep-all and keep-none cases. `FOR d IN c SAMPLE 2 RETURN d` returns 2,
  not the raw whole collection. `FILTER` after `SAMPLE 0` is empty.
- The `SAMPLE()` function (search) is a different call (unverified).

**`EXPLAIN`** — one row, the plan object (`collections`, `filters`,
`let_bindings`, `sort`, `limit`, `timing`, `documents_scanned`,
`documents_returned`, `warnings`).

```sdbql
EXPLAIN FOR u IN users FILTER u.email == @e RETURN u
-- one object, plan_only: true; every access is estimated and the counts are 0.
-- Writes nothing, including EXPLAIN INSERT … (cursor inserted: 0).
-- Pass the bind variables: without @e there is no error, and the plan says
-- full_scan where it says index_lookup with it.
EXPLAIN ANALYZE FOR u IN users FILTER u.email == @e RETURN u
-- the plan with timings, not the rows. inserted/updated/deleted stay 0
EXPLAIN ANALYZE INSERT { _key: "a" } INTO t
-- the INSERT runs once. Result is the plan, not the inserted document.
-- warnings[0]: "ANALYZE ran this writing query. Timings are not measured, …".
-- warnings[1] is the plan-only line ("nothing was run…"): wrong here, the
-- write ran. The timing object is present and its numbers are 0.
-- plan_only is omitted (serialized only when true).
-- Cursor inserted/updated/deleted are the real counts.
```

`EXPLAIN (FOR …) UNION (FOR …)` is accepted; the plan lists the first
branch's collection only. `EXPLAIN WITH x AS (…) FOR …` is accepted. Parse
errors: `EXPLAIN EXPLAIN …` and `WITH … EXPLAIN FOR …` (`Unexpected token:
Identifier("EXPLAIN"). …`), `(EXPLAIN …)` as a subquery (`Expected
RightParen, got Return`). A collection that does not exist is not an error
in a plan. In a transaction (`…/transaction/{id}/query`) `EXPLAIN ANALYZE`
of a write answers the staging message with the plan as `result`. The HTTP
`POST …/explain` endpoint still does not itself apply mutations (given an
`INSERT` it stores nothing); given an `EXPLAIN …` statement it explains the
query inside. `warnings` also notes `QUALIFY`, `SAMPLE` and
`ROLLUP`/`CUBE` when the query has them.

**Fixed before 3.1.0.** Defects seen on 2026-11-24 were fixed on 2026-11-25,
before the release, and checked on a server built from that tree (`SAMPLE
3` of 20 draws each key 47 to 72 times in 400 runs).

- From the code, not run on a server: `EXPLAIN ANALYZE` of a read with
  `SAMPLE` or `QUALIFY` runs it for real; `documents_returned` and the
  total time are measured, the accesses are the planner's estimates, and
  the steps are not timed. `EXPLAIN` anywhere but the start of a statement
  (a CTE, a set-operation operand, `(EXPLAIN …) UNION …`) is a parse error.

Behaviour of the fixes, observed then:
- `/cursor` with `X-Transaction-ID` refuses a write that has `SAMPLE` or
  `QUALIFY` (400, "send it to …/transaction/{id}/query"); that endpoint
  staged 2 of 10 for `FOR d IN logs SAMPLE 2 REMOVE d IN logs`.
- `EXPLAIN CREATE MATERIALIZED VIEW …` is a 400 `ParseError` ("EXPLAIN
  applies to queries, not to CREATE STREAM or materialized-view
  statements").
- A sampled query is never cached (`cached: false` with `cache: true`), and
  a cached query is invalidated by a write to a collection only its
  `QUALIFY` reads.

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

From 2.10.4 (observed on servers built from its code, 2026-11-14):
- `SHORTEST_PATH` without `OPTIONS { weight }` searches from both ends and
  returns *a* shortest path; when several have the same length it is not
  necessarily the one earlier versions returned. Do not assert on which.
- An edge collection carries four indexes: `_edge_from_idx` (`_from`),
  `_edge_to_idx` (`_to`), `_edge_out_idx` (`[_from, _to]`) and
  `_edge_in_idx` (`[_to, _from]`), created with it (or at the first
  traversal of an older one). Do not drop them: a traversal reads where
  edges lead from the two-field ones.
- A traversal reads from memory what it read before; a write drops what
  was kept of the documents it touches (a batch: of the whole collection)
  (`SOLIDB_GRAPH_CACHE_MB` per map, default 256, `0` off).
  Results are the same either way; only timings differ between a first and
  a later run.

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
| `[*]` | `(FOR e IN emp RETURN e)[*].name` → names. A key that is not an identifier: `ds[*].v['1']` reads `'1'` in each element (after 2.13.0; through 2.13.0 it indexed the array of results → `null`, write `ds[* RETURN CURRENT.v['1']]` there). `ds[*].v[0]` still indexes the array of results |
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

`INSERT … OPTIONS {overwrite: true}` (or `overwriteMode: "replace"`) over
several rows, with no `RETURN OLD`/`NEW` and no other option, is written
in two batches after 2.13.0: the new keys as one insert, the stored keys as
one replacement, each all or none. Through 2.13.0 it went row by row and
an error left the earlier rows written.

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
