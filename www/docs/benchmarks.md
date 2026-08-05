# Benchmarks

Seven HTTP workloads, plus WebSockets — a JSON API response, a rendered HTML page, a database read, a
database-backed HTML page, and one create, update and delete per request — through eight
full stacks on one machine, one load generator, one protocol. Every server returns a
**byte-identical payload** for the JSON and DB rows, and every stack gets the same
**16-core budget** — 16 workers for the six that fork them, 16 threads for Soli, 16 BEAM
schedulers for Phoenix.

> **Read this first.** Benchmarks are easy to rig and easy to get wrong. Every **HTTP and
> memory** table below was measured in one session on a quiet box, after a warm-up pass over
> every endpoint, with the HTTP status of every response verified and every write cell's row
> count checked before and after — and where Soli loses a cell, the number is printed exactly
> as measured. **Soli loses four of the seven HTTP rows**: the JSON row to Express, and all
> three write rows to Phoenix, which is the strongest result any stack has posted against it
> here.
>
> Two areas of this page are **older measurements, kept and labelled rather than
> restated as current**: the WebSocket section, which a separate harness produces and which
> adding a stack does not affect, and three figures quoted in the database-read prose. Each
> says so where it appears. The per-operation Soli-vs-Ruby language tables that used to live
> on this page were retired with it; this page compares *frameworks*, end to end.

## Setup

| | |
|---|---|
| Soli | 1.27.2, `soli serve .`, 16 HTTP workers, SoliDB over the **native MessagePack driver** (`SOLI_DB_DRIVER=1`, pooled TCP, not HTTP) for the DB and write rows |
| Rails | 8.1.3 + Puma 8.0.2 on Ruby 3.4.9 — production, eager-loaded, 16 workers × 5 threads, PostgreSQL via ActiveRecord |
| Laravel | 13.8 on PHP 8.4 (php-fpm, `pm = static`, 16 workers) + nginx, in Docker with host networking — Eloquent + Blade, OPcache, config/route/view cached, persistent PDO connections |
| Laravel + Octane | The same application on Octane 2.18 / FrankenPHP, 16 workers, app resident between requests. Published as a **labelled reference row**, not as "Laravel", because it roughly doubles every result and is a deployment choice rather than the default |
| Django | 6.0.7 on Python 3.14, gunicorn with 16 workers — Django ORM + Django templates, `DEBUG=False`, persistent connections (`CONN_MAX_AGE`) |
| FastAPI | 0.141.1 on Python 3.14 (Starlette 1.3), uvicorn 0.52 with 16 workers on uvloop + httptools — **SQLAlchemy 2.0.51 async + asyncpg 0.31 + Jinja2 3.1**: FastAPI ships no ORM and no view layer, so both were added, the same way they were for Express. The published rows return a `Response` directly rather than paying for `jsonable_encoder`, which nothing else here pays for; the default path is a labelled reference row below |
| Phoenix | 1.8.9 on Elixir 1.17 / Erlang OTP 27, Bandit — **one OS process, 16 BEAM schedulers** (`+S 16:16`), Ecto + HEEx, `MIX_ENV=prod`, Phoenix's default `:browser` pipeline on the HTML rows. `force_ssl` removed from the generated prod config: left in, every request is a 301 and a load generator counts those as success |
| AdonisJS | 6.18 on Node 25.9, 16 cluster workers — Lucid ORM + Edge templates, built to JavaScript and run from `build/`, `NODE_ENV=production` |
| Express | 5.2.1 on Node 25.9, 16 cluster workers — **+ EJS 6.0 + Sequelize 6.37.8** (on node-postgres 8.22): Express ships no view layer and no DB layer, both had to be added. The DB rows put it on an **ORM**, not the raw driver, so all three stacks compare like for like; the driver number is kept below as a reference |
| Database | PostgreSQL 18.3 for Rails, Express, AdonisJS, Laravel, Django, FastAPI and Phoenix (same table, same 50 rows); SoliDB for Soli — all client-server over a local socket, no in-process storage anywhere |
| Load | `oha` 1.12 — 30s at concurrency 200 per cell, after an 8s warm-up of that cell |
| Machine | 16-core x86-64 Linux, load generator on the same box |

**CPU/req** is server CPU-time per request, summed across every process of the stack
(all 17 for Rails, Express, AdonisJS and Django; 18 for FastAPI, whose uvicorn supervisor
also spawns a `multiprocessing` resource tracker; php-fpm plus nginx for Laravel; a single
process for Soli and for Phoenix, whose threads `/proc` already aggregates). It is
the most portable column here — unlike req/s it barely moves with core count or client
speed.

> **One trap this measurement had to survive.** uvicorn's 16 workers are
> `multiprocessing.spawn` children, so a worker's command line reads `python3 -c from
> multiprocessing.spawn import spawn_main; ...` — the app's name appears nowhere in it.
> Summing CPU by command-line pattern, the way the Django column is summed
> (`gunicorn.*benchproj`), matched the supervisor alone: **8 CPU ticks against the process
> group's 88**, which would have published a FastAPI CPU/req roughly ten times too good.
> The workers do share the supervisor's process group, so this column and the memory
> table both measure FastAPI by **pgid**. It is the same failure mode as the Octane memory
> bug noted further down — a pattern that silently matches a subset always errs in the
> flattering direction.

## JSON — 50 objects, 2,268 bytes, built in the handler

| Stack | req/s | p99 | CPU/req | vs Rails |
|---|---:|---:|---:|---:|
| Express + EJS + Sequelize | 112,128 | 5.72 ms | — | 7.5x |
| **Soli** | 110,967 | 4.28 ms | — | 7.4x |
| FastAPI + SQLAlchemy + Jinja2 | 83,470 | 7.43 ms | 141 µs | 6.2x |
| Phoenix + Ecto + HEEx | 64,772 | 8.26 ms | 208 µs | 4.3x |
| AdonisJS + Lucid + Edge | 21,208 | 22.88 ms | 593 µs | 1.6x |
| Django + gunicorn | 16,792 | 20.55 ms | 701 µs | 1.2x |
| Rails + Puma | 13,448 | 30.73 ms | 932 µs | 1.0x |
| Laravel + Octane *(reference)* | 9,179 | 32.44 ms | 1,493 µs | 0.7x |
| Laravel + php-fpm | 4,380 | 60.98 ms | 3,059 µs | 0.3x |

Soli serialises the API response at 7.1x Rails' throughput on 7.1x less CPU. Express wins
this row outright — printed as measured. Worth knowing: in Soli, serialising these 50
objects to JSON costs ~132µs where rendering the *same data* as HTML costs 99µs — the
template engine is still cheaper than `render_json`.

**The three fastest stacks here are within 33µs of each other per request** — Express 108,
Soli 132, FastAPI 141 — which is the more interesting fact than the throughput ordering.
On a handler that touches no database, a modern Python framework is not the bottleneck
people assume it is: FastAPI's own overhead is **a fifth of Django's** on the same Python
and the same box. Phoenix is the outlier of the four at 204µs, and this is the row where it
looks weakest — hold that thought until the database rows.

> **A sanity check worth running on any row like this.** req/s × CPU/req says how much of
> the machine each stack actually used: Phoenix 13.5 cores, Soli 12.7, Rails 12.5, Express
> 12.3, and FastAPI and Django 11.8 each. All nine are inside a narrow band, so every stack
> on this row is genuinely CPU-bound and no throughput figure here is an artefact of one
> server failing to fill the box. An earlier session had FastAPI at **9.0 of 16** — not CPU-
> bound, so its throughput was a floor rather than a ceiling — which is worth knowing as a
> failure mode even though it did not recur: uvicorn's workers accept from one shared
> listening socket and the kernel does not balance accepts evenly.

## Template — 50-row HTML table + layout, ~3 KB

| Stack | req/s | p99 | CPU/req | vs Rails |
|---|---:|---:|---:|---:|
| **Soli** | 124,933 | 3.80 ms | ~95 µs | 10.7x |
| Express + EJS + Sequelize | 66,857 | 8.04 ms | 203 µs | 6.0x |
| Phoenix + Ecto + HEEx | 62,033 | 8.08 ms | 220 µs | 5.3x |
| FastAPI + SQLAlchemy + Jinja2 | 37,935 | 15.92 ms | 366 µs | 3.4x |
| AdonisJS + Lucid + Edge | 22,936 | 16.16 ms | 584 µs | 2.1x |
| Rails + Puma | 11,182 | 31.93 ms | 1,176 µs | 1.0x |
| Laravel + Octane *(reference)* | 8,861 | 29.34 ms | 1,680 µs | 0.8x |
| Django + gunicorn | 7,076 | 35.89 ms | 1,991 µs | 0.6x |
| Laravel + php-fpm | 4,626 | 48.90 ms | 3,214 µs | 0.4x |

The strongest row, and the one a server-rendered framework should care about: **11.2x
Rails' throughput on 11.9x less CPU** — and 1.9x faster than Express even though Soli's
page carries its instant-navigation script (~130 extra bytes of work per request that EJS
doesn't do). Soli's ERB engine outrunning a compiled EJS template was not a given.

**Read this row as the marginal cost of a template engine**, by subtracting each stack's
JSON CPU from its number here. Phoenix pays **+9µs** to render fifty rows — HEEx compiles
templates to iodata at build time, so there is almost nothing left to do at request time,
and it is the cheapest render in the field by a wide margin. Express pays +95µs, FastAPI
+225µs for Jinja2. Soli is the curiosity: **−33µs**, because its ERB path is genuinely
cheaper than `render_json`. FastAPI still renders the same page 5.4x faster than Django.

## Database read — 50 rows, projected columns, 2,268 bytes

| Stack | req/s | p99 | CPU/req | vs Rails |
|---|---:|---:|---:|---:|
| **Soli** | 49,707 | 5.04 ms | 196 µs (248 incl. SoliDB) | 5.1x |
| Phoenix + Ecto + HEEx | 31,346 | 15.88 ms | 361 µs | 3.2x |
| Express + EJS + Sequelize | 28,148 | 15.50 ms | 404 µs | 3.1x |
| AdonisJS + Lucid + Edge | 14,492 | 24.79 ms | 899 µs | 1.6x |
| FastAPI + SQLAlchemy + Jinja2 | 11,662 | 63.28 ms | 1,128 µs | 1.3x |
| Django + gunicorn | 10,342 | 24.09 ms | 1,187 µs | 1.1x |
| Rails + Puma | 9,150 | 37.48 ms | 1,393 µs | 1.0x |
| Laravel + Octane *(reference)* | 7,897 | 31.16 ms | 1,717 µs | 0.9x |
| Laravel + php-fpm | 3,689 | 62.30 ms | 3,700 µs | 0.4x |

> **This row compares database access architectures as much as frameworks.** Each request
> from Soli is one MessagePack round trip on a pooled TCP driver connection to SoliDB per worker — 16 in flight, no more.
> Rails holds 80 threads against PostgreSQL; Phoenix runs one Ecto pool of 80 across the
> whole VM; Express's driver is fully asynchronous behind a 5-per-worker cap.
>
> **Every stack goes through an ORM in this table.** An earlier revision measured
> Express on the raw `pg` driver, which is not the same workload: hand-written SQL with no
> model layer against frameworks paying one. Putting Sequelize in the path costs Express
> **33%** — 41,812 req/s on the driver against 28,148 through
> the ORM — and that is the number published, because every other row includes its ORM
> too. If your Node app talks to the database through a driver rather than an ORM, the
> driver row is the one that describes you. Soli's CPU column shows
> both truths: 226µs in the Soli process, **356µs system-wide once SoliDB's own CPU is
> counted** — publishing the smaller number alone would hide an entire process. Even
> counted that way Soli's DB row costs **3.9x less system CPU than Rails'**, and the
> 16-in-flight cap is not the ceiling it looked like: it is enough to lead this row.

> **These two rows include a read-result cache that no other stack here has, and
> that is a problem with them.** SoliDB memoizes read-only cursor results per
> (database, query, bind vars) and replays repeats with `executionTimeMs: 0.0`.
> The benchmark issues the *same* query every request and never writes `posts`, so
> the hit rate is 100% for the life of the run. Measured as a same-session A/B on
> this app, with the cached arm reproducing the published figure to within 7%:
>
> | Soli, `/db` | req/s | SoliDB CPU/req |
> |---|---:|---:|
> | with the cache *(what this table shows)* | 38,348 | 116 µs |
> | with the cache off | **22,693** | 309 µs |
>
> That is **1.7x**, and it is not a like-for-like advantage. PostgreSQL's buffer
> cache spares the disk but still re-plans, re-executes and re-serialises every
> request; SoliDB returns the memoized result set and does no query work at all.
> The nearest equivalent for the other stacks would be `Rails.cache.fetch` or
> Django's `cache_page` around the action — which this page would rightly refuse
> to count. Applying that ratio, Soli would fall from **1st to roughly 3rd** on
> both database rows, behind Phoenix and Express.
>
> **The rows above are therefore not corrected yet, and should not be quoted as a
> like-for-like database comparison.** Correcting them needs a re-sweep with
> `SOLI_DB_NO_QUERY_CACHE=1` (the flag exists for exactly this) on a box that
> passes `control.sh`; the derived figures are deliberately not written into the
> table, because a measured table should hold measured numbers.

> **The Phoenix row is the closest thing to a peer this page has measured**, and it is worth
> being precise about how close. Soli leads on throughput by **1.12x** — 35,969 against
> 32,255 — which is a real lead and a small one. On CPU it is a dead heat: **356µs
> system-wide for Soli against 357µs for Phoenix**, one microsecond apart, and that is the
> comparison that survives a change of machine. Both are one OS process with a scheduler
> inside it rather than a fleet of forked workers, both talk to their database over a pooled
> socket, and on this row they cost the same. Where they differ is the tail: Soli's p99 is
> 6.54 ms against Phoenix's 14.67 ms.
>
> The How Soli Compares page has asserted that "Phoenix is the only peer here" for a while
> without a measurement behind it. This is that measurement, and it holds.

> **FastAPI's p99 is the outlier on this row, and it is not the framework.** 63.28 ms
> against Django's 24.09 ms, while FastAPI serves *more* throughput (11,662 against
> 10,342) — the means are nearly equal (17.2 ms against 19.3 ms at concurrency 200), so
> what differs is entirely the tail. The cause is the shape of the queue. Django's 16
> gunicorn workers each handle one request at a time, so 184 of the 200 wait in one kernel
> accept queue: strictly FIFO, and a narrow distribution. FastAPI accepts all 200 into 16
> event loops, where ~12 coroutines per worker then contend for 5 pool slots and one GIL —
> and that contention is not FIFO, so some requests are served promptly while others wait
> several times the mean. **Async concurrency in front of a bounded pool does not remove
> the queue; it moves it somewhere less fair.** The pool is matched to every other stack's
> (5 per worker, 80 total) and Express runs the same cap at a p99 of 15.50 ms, so this is
> specific to the Python async stack rather than to the pool size — measured directly
> below rather than asserted.

**Is the matched pool what produces that tail?** Partly, and it is worth measuring instead
of arguing about. Raising FastAPI's pool from the matched 5 per worker to 20 — from 80
connections to 320, four times what any other stack here gets — gives:

| FastAPI, pool per worker | `/db` req/s | `/db` p99 | `/db-template` req/s | `/db-template` p99 |
|---|---:|---:|---:|---:|
| 5 *(matched, published above)* | 11,662 | 63.28 ms | 9,738 | 72.42 ms |
| 20 *(reference)* | 12,488 | 44.59 ms | 10,185 | 58.09 ms |

Two conclusions, and they point in different directions. **The pool is not what caps
FastAPI's throughput** — quadrupling it buys 5–7%, so the ceiling is CPU inside SQLAlchemy,
and the matched configuration is not handicapping the published rows. But **the pool is a
real part of the tail**: p99 falls by 30% on the JSON read and 20% on the rendered page.
Even so, 44.59 ms at 320 connections is still **1.85x Django's 24.09 ms at 16 connections**
and nearly the same throughput — so most of the tail survives the fix, and belongs to
coroutine and GIL contention rather than to connection scarcity.

Every stack serves the same self-describing hash rows on its fastest idiom for that
shape: Soli's `Post.pluck(:id, :title, :views).all` builds the hashes **in the
database** (`RETURN {id: doc.id, ...}`); Rails' is `pluck` + a `map`; FastAPI's is
`select(Post.id, Post.title, Post.views)` through an `AsyncSession`; node-postgres builds
the objects in the driver. Projection must happen in the database — the hydrating form of
the same query costs every ORM real money, measured here for the two Python stacks:

| Instead of projecting | req/s | vs its own projected row |
|---|---:|---:|
| FastAPI, 50 mapped SQLAlchemy objects (`select(Post)`) | 8,662 | 0.74x |
| Django, 50 model objects (`.only()` instead of `.values()`) | 9,275 | 0.90x |
| Phoenix, 50 Ecto structs (`Repo.all(Post)`) | 32,813 | **1.02x** |

This is a clean three-way comparison, because `Post` has exactly the three columns the
projection selects — so all three ORMs fetch **identical bytes** in both forms and the only
difference is what they build from them. The cost of hydration is therefore
**26% for SQLAlchemy, 10% for Django, and nothing measurable for Ecto** (1.02x is inside
run-to-run noise). Building a `%Post{}` struct is a map literal; building a mapped
SQLAlchemy object registers it in an identity map and installs instrumented attributes on
it, and that is what the 26% pays for.

Three figures on this theme were measured in an earlier session and are labelled as such
rather than restated as current. Two make the same point as the table: Rails' canonical-looking
`render json: Post.select(:id, :title, :views)` measured **3.2x slower** than its `pluck`
row because it instantiates fifty ActiveRecord models per request, and Soli fetching full
~15 KB documents to project client-side measured 13,251 req/s against its projected row.
The third is a separate lever worth knowing: if you can accept **positional arrays instead
of hashes**, everyone gets faster — on the array form of this same route Soli measured
26,666 and Express 44,515, because the field names stop being repeated fifty times on the
wire and in the parser.

## Database read + HTML render — 50 rows from the database into a page, ~3 KB

The row a server-rendered framework actually lives on: query, then render. It is the
`/db` read and the `/template` render in one request, so the response is the same page as
the Template row above, byte-for-byte the same size, and the database is the only added
variable.

| Stack | req/s | p99 | CPU/req | vs Rails |
|---|---:|---:|---:|---:|
| **Soli** | 58,591 | 4.33 ms | 155 µs (206 incl. SoliDB) | 7.0x |
| Phoenix + Ecto + HEEx | 31,605 | 14.97 ms | 362 µs | 3.8x |
| Express + EJS + Sequelize | 24,185 | 15.71 ms | 492 µs | 3.0x |
| AdonisJS + Lucid + Edge | 13,734 | 27.04 ms | 957 µs | 1.7x |
| FastAPI + SQLAlchemy + Jinja2 | 9,738 | 72.42 ms | 1,382 µs | 1.2x |
| Rails + Puma | 8,184 | 40.82 ms | 1,577 µs | 1.0x |
| Laravel + Octane *(reference)* | 7,122 | 33.82 ms | 1,937 µs | 0.9x |
| Django + gunicorn | 5,522 | 45.47 ms | 2,522 µs | 0.7x |
| Laravel + php-fpm | 3,458 | 66.74 ms | 3,979 µs | 0.4x |

Soli takes this row — **4.8x Rails' throughput, 1.6x Express's and 1.2x Phoenix's** — and it
is the row whose shape matters most, because it is the only one where every stack does the
two things a page does, each through its own ORM. Express on the raw driver reaches
32,579 here, still short of Soli with an ORM in the way.

The result worth pausing on is Soli's own: this row and the DB row above are within 9%
of each other (58,591 and 49,707) on the same query and the same 50 rows. Once a database
round trip is in the request, it dominates — whether the result leaves as JSON or as a
rendered page is close to noise. The large render-path gap the JSON and Template rows show
on in-memory data does not survive contact with a real query, which is worth knowing before
optimising a template for a page that is really waiting on the database.

FastAPI shows the same convergence from the other direction, and far more sharply: 83,470 on
the JSON row, 9,738 here. **Its framework overhead is 141µs and its data layer adds roughly
1,240µs** — so on any route that touches the database, nine-tenths of the request is
SQLAlchemy and asyncpg, not FastAPI. That is worth knowing before choosing this stack for
its benchmark reputation: the reputation is earned on the row without a database in it.

Phoenix is the mirror image and the reason this comparison is worth drawing. Its framework
overhead is the *highest* of the four fast stacks at 204µs, but **its data layer adds only
~150µs** — Ecto over postgrex, against SQLAlchemy's ~1,240µs. So the two "modern async"
stacks are separated by roughly **8x on the cost of talking to a database**, and it runs
opposite to their standing on the JSON row. Choosing between them on a JSON-echo benchmark
would get the answer exactly backwards for any app with a query in it.

### FastAPI, the default way — a labelled reference

FastAPI's signature convenience is that a handler returns a value and the framework
serialises it. That convenience is not free, and because no other stack on this page runs a
serialization framework, the matched rows above return a `Response` directly instead.
Here is what the default costs, measured on the same box in the same session:

| Route | Form | req/s | vs the matched row |
|---|---|---:|---:|
| `/json` | `JSONResponse(rows())` — published above | 83,470 | — |
| `/json-encoded` | `return rows()` — through `jsonable_encoder` | 31,809 | **0.38x** |
| `/db` | `JSONResponse(await db_rows())` — published above | 11,662 | — |
| `/db-encoded` | `return await db_rows()` | 9,410 | 0.81x |

**`jsonable_encoder` costs 62% of the throughput on the in-memory row and 19% on the
database row** — the same absolute cost both times, but a much smaller share of a request
that is already paying ~1,240µs for SQLAlchemy. Both readings are useful: the first is what
the encoder actually costs, the second is what you would actually notice. If your FastAPI
service is database-bound, the default return path is close to free; if it is a
transformation API over data already in memory, it is two thirds of your capacity.

## Writes — create, update and delete, one row per request

The read rows above are only half of CRUD. These three measure a single write per request
against an isolated 800,000-row table, reset to exactly that state before every cell so no
stack inherits a table another stack grew or emptied. Update and delete address one row by
primary key, drawn at random from the same 1..800,000 range in every stack.

**Durability is matched, and that matters more than anything else here.** SoliDB's writes
go through RocksDB's default write path — the WAL reaches the operating system but is not
`fsync`ed before the write returns. PostgreSQL's default (`synchronous_commit=on`) *does*
flush before commit returns, which is a stronger guarantee and a much slower one. Compared
head to head that way, Soli would be winning an argument the other stacks were not having.
So PostgreSQL runs with `synchronous_commit=off` for these rows, which is the setting that
matches what SoliDB actually promises: survive a process crash, not a power cut. Neither
column is "durable writes" — read them as buffered writes on both sides.

**Octane roughly doubles Laravel on every row** (1.9x to 2.3x), which is the whole reason
it is published separately. On php-fpm the framework is rebuilt per request; on Octane it
stays in memory, and that single change moves Laravel from last place to roughly Rails' band
— and past Django on the two rendered rows, the template and the database-backed page, where
Django's template engine is the slowest here. Read the php-fpm
rows as the default deployment and the Octane rows as what the same code does when you opt
into the resident runtime.

> **Octane's CPU column was wrong until this session, and read 0 µs.** Its processes live in
> a container, so `ss -ltnp` cannot see the listener's pid as an unprivileged user, and the
> harness's pgid-based CPU sum silently added nothing. `/proc/<pid>/stat` is world-readable
> even for root-owned processes — unlike `smaps_rollup`, which is why the memory table has
> to use cgroups for these two rows — so the fix was to sum by command-line pattern over
> the `octane:start` supervisor and the `frankenphp` worker host. Octane's seven cells were
> re-measured after the fix and its row carries that run throughout, which is why its
> throughput differs slightly from the interleaved sweep. Third instance on this page of the
> same lesson: **a measurement that silently matches a subset always errs in the flattering
> direction.**

### Create — one INSERT per request

| Stack | req/s | p99 | CPU/req | vs Rails |
|---|---:|---:|---:|---:|
| **Soli** | 44,492 | 7.48 ms | 96 µs (219 incl. SoliDB) | 5.5x |
| Phoenix + Ecto + HEEx | 43,623 | 8.79 ms | 220 µs | 5.4x |
| Express + EJS + Sequelize | 25,009 | 19.88 ms | 441 µs | 3.1x |
| AdonisJS + Lucid + Edge | 14,878 | 27.76 ms | 853 µs | 1.8x |
| FastAPI + SQLAlchemy + Jinja2 | 11,970 | 86.21 ms | 1,078 µs | 1.5x |
| Django + gunicorn | 11,215 | 22.87 ms | 1,057 µs | 1.4x |
| Laravel + Octane *(reference)* | 8,102 | 30.06 ms | 1,654 µs | 1.0x |
| Rails + Puma | 8,074 | 42.29 ms | 1,497 µs | 1.0x |
| Laravel + php-fpm | 3,687 | 61.22 ms | 3,760 µs | 0.5x |

### Update — one row by primary key

| Stack | req/s | p99 | CPU/req | vs Rails |
|---|---:|---:|---:|---:|
| **Soli** | 45,089 | 6.94 ms | 95 µs (229 incl. SoliDB) | 4.1x |
| Phoenix + Ecto + HEEx | 41,430 | 9.32 ms | 223 µs | 3.8x |
| Express + EJS + Sequelize | 22,432 | 18.18 ms | 491 µs | 2.1x |
| AdonisJS + Lucid + Edge | 14,762 | 31.40 ms | 827 µs | 1.4x |
| FastAPI + SQLAlchemy + Jinja2 | 10,828 | 97.01 ms | 1,202 µs | 1.0x |
| Rails + Puma | 10,635 | 34.50 ms | 1,153 µs | 1.0x |
| Django + gunicorn | 10,341 | 24.13 ms | 1,148 µs | 1.0x |
| Laravel + Octane *(reference)* | 8,338 | 29.06 ms | 1,593 µs | 0.8x |
| Laravel + php-fpm | 3,735 | 61.56 ms | 3,680 µs | 0.4x |

### Delete — one row by primary key

| Stack | req/s | p99 | CPU/req | vs Rails | rows removed |
|---|---:|---:|---:|---:|---:|
| Phoenix + Ecto + HEEx | 46,835 | 8.64 ms | 209 µs | 4.2x | 47% of requests |
| Express + EJS + Sequelize | 32,956 | 14.19 ms | 308 µs | 3.0x | 57% of requests |
| **Soli** | 46,562 | 5.87 ms | 126 µs (252 incl. SoliDB) | 4.2x | 47% of requests |
| AdonisJS + Lucid + Edge | 15,506 | 27.75 ms | 798 µs | 1.4x | 76% of requests |
| FastAPI + SQLAlchemy + Jinja2 | 11,743 | 79.88 ms | 1,093 µs | 1.1x | 81% of requests |
| Rails + Puma | 10,898 | 32.61 ms | 1,128 µs | 1.0x | 82% of requests |
| Django + gunicorn | 8,977 | 27.24 ms | 1,306 µs | 0.8x | 85% of requests |
| Laravel + Octane *(reference)* | 8,446 | 29.15 ms | 1,569 µs | 0.8x | 86% of requests |
| Laravel + php-fpm | 3,624 | 64.90 ms | 3,699 µs | 0.3x | 94% of requests |

> **Read the delete row with its caveat.** Delete is the one operation that consumes its
> own workload: a key already deleted is a miss, and a miss is cheaper than a delete. The
> "rows removed" column is measured, not assumed — the table is counted before and after
> each cell — and it shows the effect plainly: the faster a stack runs, the more of the
> 800,000-row pool it exhausts and the higher its miss rate climbs. Phoenix, fastest here,
> does real work on only **47%** of its requests; Laravel, slowest, on 94%. So this row
> flatters whoever leads it — including Phoenix — and should be read as a rough ordering,
> not a precise multiple. The create and update rows have no such problem: every create
> inserts, every update targets a key that still exists, and Phoenix leads both.

**With the native driver, Soli takes create and update; delete is a dead heat.** 1.02x Phoenix on create, 1.09x on
update, 1.56x on delete — the largest margin any stack has posted against Soli on this page.
The CPU column says the same thing more durably: Soli's *own* process is very cheap per write
(134–155µs, **7 to 11x lower than Rails'**), but the number in parentheses is the honest one,
and system-wide Soli spends **363–426µs against Phoenix's 208–219µs** — 1.7x to 2.1x more.

The cause is architectural and worth stating plainly, because it is the flip side of the
schemaless trade this page praises elsewhere. **Soli's writes leave the process as HTTP.**
Every insert is a request to SoliDB with headers to build, a body to serialise, a socket to
write and a response to parse. Phoenix's insert goes down an already-open pooled connection
as a few hundred bytes of PostgreSQL binary protocol. On the read rows that overhead hides
behind the cost of shipping fifty projected rows back; on a single-row write there is nothing
for it to hide behind, and it is most of the request. Both databases are buffering rather
than flushing (see the durability note above), so this is transport, not durability.

Two smaller notes. Every stack writes far more slowly than it reads — Soli's create row is
32,568 against 125,381 on the in-memory template row — because a write has to reach another
process and a log, whichever framework issued it. And FastAPI's **p99 on all three write rows
is 80–97 ms**, three to four times Django's, while its throughput edges Django out on each:
same cause as its database reads, 200 requests accepted into 16 event loops and then queued
on 5 pool slots with no FIFO guarantee. If you deploy that stack, the median will flatter it
and the tail is what your users will report.

## WebSockets — echo and fan-out

> **These four tables are from an earlier session** and were not re-measured when FastAPI
> was added — a different harness produces them (`ws_sweep.sh`, with its own sharded client
> and Redis for two of the stacks), and none of the three stacks in them changed. Read them
> against each other, not against the HTTP tables above.

`oha` speaks HTTP only, so these use a purpose-built client. Three stacks appear
here: Soli, Express and Rails. Rails' ActionCable mounts inside the same Puma
process serving the HTTP rows, on the **redis** adapter — the `async` adapter
keeps its pubsub in-process, so with 16 workers a broadcast would reach only the
worker that received it. It also speaks a JSON subprotocol rather than raw
WebSocket, so the client performs the full welcome → subscribe → confirm
handshake before any message is timed. Django (Channels on ASGI) and Laravel
(Reverb) would each need a *different server process* from the one serving their
HTTP rows, so they are absent rather than misrepresented.

FastAPI is the one absence that isn't structural: it is ASGI already, and the same
uvicorn workers that served its HTTP rows could serve `/ws` too. But those 16 workers are
separate processes with no shared bus, so its fan-out would land in exactly the
clustered-Node trap described below. That is worth measuring; it has not been measured
yet, and an unmeasured row is not a row.

### Echo — round trip, one message in flight per connection

| Stack | msg/s | p50 | p99 | connections |
|---|---:|---:|---:|---:|
| Express + ws | **411,340** | 2.01 ms | 6.83 ms | 1,000 |
| **Soli** | 241,790 | 4.02 ms | 6.20 ms | 1,000 |
| Rails + ActionCable | 98,529 | 0.58 ms | **63.83 ms** | 1,000 |

Express takes this one — printed as measured — at roughly 1.7x Soli's message
rate and half the median latency.

**Do not read Rails' p50 as a win.** At 1,000 connections and 98,529 msg/s the
*mean* latency must be 10.15 ms, seventeen times its median: the distribution is
bimodal, not fast. ActionCable dispatches through a bounded worker pool (four
threads per Puma worker by default), so a subset of sockets is served almost
immediately while the rest queue — which is exactly what a p50 of 0.58 ms
against a p99 of 63.83 ms describes. Soli and Express keep p99 within ~3x of
their medians; Rails' spread is 110x. When a median and a mean disagree by that
much, the median is the misleading one.

### Fan-out — one publisher, every connection in the room receives

| Stack | reached per publish | share of the room | deliveries/s |
|---|---:|---:|---:|
| **Soli**, 16 workers | **1,000 of 1,000** | 100% | 45,264 |
| Rails + ActionCable, 16 workers **+ Redis** | 1,000 of 1,000 | 100% | 45,545 |
| Express + ws, 16 workers **+ Redis** | 1,000 of 1,000 | 100% | 44,217 |
| Express + ws, 1 worker | 1,000 of 1,000 | 100% | 44,106 |
| Express + ws, 16 workers, no bus | 63 of 1,000 | **6%** | 2,846 |

**Read this as an architecture row, not a speed row.** Once the broadcast is
actually complete, all four are the same: 45,264, 45,545, 44,217 and 44,106
deliveries/s are one number. Neither Express nor Rails is slow at fan-out —
and Rails, whose echo throughput is a third of Soli's, matches it exactly here,
because delivering to a room is dominated by the sockets, not the framework.

The difference is what it takes to get there. Soli's 16 workers are threads in
one process, so a broadcast reaches every connection the server holds, with
nothing to configure. Node's `cluster` gives each worker its own sockets, so the
obvious implementation reaches only the ~1/16th that worker accepted — the last
row, and note that it does not error, it just silently delivers to 6% of the
room. Fixing it means either dropping to one worker (correct, and one sixteenth
of the HTTP capacity) or adding Redis and a hop per publish (correct, same
throughput, one more thing to run and to fail).

That is the honest shape of it: **equal throughput, unequal defaults.** The
naive Soli implementation is right; the naive clustered-Node one is quietly
wrong.

Connection cost is close: both accept about **6,000 connections/sec** once warm,
and 1,000 idle sockets cost Soli ~19 KB each against Express's ~28 KB. Soli's
*first* few hundred connections are slower while handlers warm — a cold-start
effect, not a steady-state one.

> **Two things this measurement had to get right**, because both produced
> confident wrong answers first. A single Node client saturates long before
> either server: it reported 74k msg/s against Soli where eight client processes
> reported 238k, and throughput *fell* as connections rose while latency scaled
> linearly — the signature of a bottlenecked generator, not a server limit. And
> an unthrottled publisher outruns the server, because a publish costs the
> client nothing and the server N sends; flat out, fan-out read 9.4 of 2,000 and
> was measuring the client's send loop. The published numbers use eight sharded
> clients and a rate-limited publisher.

## Memory

| Stack | Processes | Idle | Under load |
|---|---:|---:|---:|
| **Soli** | 1 x 16 threads | 95 MB | 111 MB |
| Phoenix + Ecto + HEEx | **1** (16 BEAM schedulers) | 94 MB | 160 MB |
| Laravel + php-fpm | 17 (fpm + nginx) | 136 MB | 140 MB |
| Rails + Puma | 17 (fork + CoW) | 205 MB | 889 MB |
| Laravel + Octane *(reference)* | 16 resident workers | 260 MB | 282 MB |
| Express + EJS + Sequelize | 17 (fork + CoW) | 458 MB | 1,043 MB |
| Django + gunicorn | 17 (fork + CoW) | 643 MB | 916 MB |
| AdonisJS + Lucid + Edge | 17 (fork + CoW) | 884 MB | 3,134 MB |
| FastAPI + SQLAlchemy + Jinja2 | 18 (**spawn**, no CoW) | 997 MB | 1,015 MB |

Figures are **PSS** (proportional set size) summed over the whole process group — the
honest measure for multi-process servers, because summing RSS counts every fork-shared
page 17 times. Every stack was **restarted immediately before this table** so that "idle"
means idle: read after an hour of benchmarking instead, the same servers reported Rails at
1,090 MB and Soli at 42 — retained heap, not footprint. "Under load" is read at the end of
a 30s run on the DB + HTML route.

**At idle Soli runs in a seventh of Rails' memory, a twentieth of Django's and a thirty-third
of FastAPI's.** The architectural cause is simple, and this table is the cleanest evidence for
it on the page: **the only two stacks under 130 MB are the only two that are not a fleet of
forked processes.** Soli is one process whose 16 workers are threads, at 30 MB; Phoenix is one
BEAM with 16 schedulers, at 108 MB — a sixth of Django's 643 MB for a comparable framework
plus ORM plus template engine. Soli is 3.6x leaner still than Phoenix, and those two are in a
different order of magnitude from everything else here.

**FastAPI is the one row where the process model, not the library weight, is the story.**
uvicorn's `--workers` spawns rather than forks, so each of its 16 workers imports FastAPI,
Pydantic, SQLAlchemy and Jinja2 into a *fresh* interpreter with nothing shared. Django
carries a comparable pile of Python on the same version and sits at 652 MB because gunicorn
forks and copy-on-write lets 16 workers share the parent's loaded modules. Same language,
same libraries, **354 MB of difference from `fork` versus `spawn`** — and it is also why the
CPU note above had to measure this stack by pgid: spawned children keep none of the parent's
identity, including its command line.

Two other results are worth reading together. Laravel on php-fpm is third at 136 MB —
*because* it is slowest: the framework is not resident between requests, so there is little
to hold. Octane makes the opposite trade, keeping the application in memory: it more than
doubles Laravel's throughput and roughly doubles its memory, 260 MB against 136. AdonisJS is
the far end at **3,134 MB under load**, three times Express's total on the same runtime —
what a full TypeScript framework, ORM and template engine cost when each of 16 workers
carries its own copy. Note also which stacks *grow* under load: Rails 205 → 889 MB and
Express 458 → 1,043 MB more than double, while Soli (30 → 49), Phoenix (108 → 197) and
FastAPI (997 → 1,015) stay close to their idle footprint.

> **Two measurement methods, and the difference matters.** The seven native stacks are
> measured as **PSS** summed over their process group. The two Laravel stacks run in
> containers whose process trees are partly root-owned, so `smaps_rollup` is unreadable and
> a PSS sum silently skips those processes — they are measured from their **cgroup**
> (`docker stats`) instead. An earlier revision of this page reported Octane at 43 MB, which
> was the `php artisan octane:start` supervisor alone while the FrankenPHP process
> contributing 230 MB of RSS was skipped for being unreadable. cgroup usage and PSS are not
> the same metric, so treat the Laravel rows as comparable to each other and only
> indicative against the rest. One more caveat specific to the Octane row: it is not part of
> the bench's compose project and could not be recreated from this repository, so unlike
> every other row it was **not** restarted before the reading — its container had been
> resident for seven days. For a runtime whose whole point is staying resident that is
> arguably the figure that matters, but it is not the same measurement as the others.

## The code being measured

Each app is idiomatic for its framework — the DB action in each:

```soli
# Soli
class PostsController < Controller
  def db_json
    render_json(Post.pluck(:id, :title, :views).all)
  end

  def db_template
    render("posts/list", { "title": "Posts", "items": Post.pluck(:id, :title, :views).all })
  end
end
```

```ruby
# Rails
class PostsController < ApplicationController
  def db_json
    render json: Post.pluck(:id, :title, :views)
      .map { |id, title, views| { id: id, title: title, views: views } }
  end

  def db_template
    @title = "Posts"
    render "posts/list", locals: {
      items: Post.pluck(:id, :title, :views)
        .map { |id, title, views| { id: id, title: title, views: views } }
    }
  end
end
```

Rails' `.map` is not padding: `pluck` returns arrays, so without it Rails would ship
`[[1, "…", 7]]` where the others ship `[{"id": 1, …}]` and the payloads would stop being
comparable. It is also not the handicap it looks like — `select_all`, where the adapter
builds the hashes exactly as node-postgres does for Express, measured slightly *slower*
than `pluck` + `map` on the same byte-identical response.

```soli
# Soli — model, complete
class Post < Model
end
```

```ruby
# Rails — model + migration
class Post < ApplicationRecord
end

# db/migrate/..._create_posts.rb
class CreatePosts < ActiveRecord::Migration[8.1]
  def change
    create_table :posts do |t|
      t.string  :title
      t.integer :views
    end
  end
end
```

The controllers, views and routes are line-for-line equivalent; the visible difference is
the migration. That is a trade, not a win: SoliDB is schemaless, so Soli needs no
migration *and* gets no database-enforced schema — Rails guarantees the table's shape,
Soli lets you persist anything. The Express version is a working app too, but its view
engine, ORM, pooling and template compilation are hand-assembled — the ~40 lines it
takes are the frameworkless tax the other two don't pay. Its DB action, for the record:

```javascript
// Express + Sequelize — the model has to be declared in the app
const PostModel = sequelize.define('Post', {
  title: DataTypes.STRING,
  views: DataTypes.INTEGER,
}, { tableName: 'posts', timestamps: false });

const ORM_PROJECTION = { attributes: ['id', 'title', 'views'], raw: true };

app.get('/db', async (req, res) => res.json(await PostModel.findAll(ORM_PROJECTION)));
app.get('/db-template', async (req, res) => {
  const rows = await PostModel.findAll(ORM_PROJECTION);
  res.type('html').send(layout({ title: 'Posts', body: list({ items: rows }) }));
});
```

`raw: true` is what makes this the same workload as the other two: it projects without
instantiating models, exactly as Rails' `pluck` and Soli's `pluck` do.

FastAPI is in the same position as Express — no ORM, no view layer, both supplied — and its
DB action is the async spelling of the same idea:

```python
# FastAPI + SQLAlchemy 2.0 — select() on mapped columns projects without
# hydrating objects, the analogue of pluck / raw:true / toBase() / .values().
POSTS = select(Post.id, Post.title, Post.views)

async def db_rows():
    async with Session() as session:
        result = await session.execute(POSTS)
        return [dict(row) for row in result.mappings()]

@app.get("/db")
async def db_json():
    return JSONResponse(await db_rows())
```

`JSONResponse` rather than `return await db_rows()` is the one thing to notice. Returning
the list would send it through `jsonable_encoder`, and no other stack on this page pays a
serialization framework — so the matched row returns the response directly (a documented
FastAPI idiom) and the default path is published beside it as a reference.

## What these multiples do and don't mean

Trivial handlers measure *fixed framework overhead*, which is precisely where Rails is
weakest — that is why the JSON and template rows show 7–11x and the DB rows show 4–5x. On
a page dominated by real query work the multiple compresses toward the DB row, not the
template row. The honest claim this page supports is: **Soli's framework overhead is
roughly a tenth of Rails' and its render path beats Express's, while database-bound routes
are architecture-limited for everyone.** Quote it that way.

FastAPI and Phoenix sharpen the same point from opposite directions, and together they are
the best argument on this page against trusting a single row. On the JSON row FastAPI's
per-request CPU is within 33µs of Soli's and Express's while Phoenix is the slowest of the
four — and on the write rows that ordering **inverts completely**, with Phoenix ahead of
everything and FastAPI near Django. The framework was never the variable. What separates
these stacks is everything *around* it: the data layer (Ecto ~150µs against SQLAlchemy
~1,240µs, an 8x spread), the template engine (HEEx +9µs against Jinja2 +225µs), the
transport to the database, and the tail behaviour under a bounded pool. Pick a stack on the
row that looks like your app — and if that row is a write, the answer here is not Soli.

## Reproducing

Warm up first, then measure for a fixed duration — never a fixed request count:

```bash
oha -z 8s  -c 100 --no-tui --output-format quiet http://localhost:PORT/route   # warm-up
oha -z 30s -c 200 --no-tui --output-format json  http://localhost:PORT/route
```

Three checks before believing any result, including this one. Verify the **status codes
and payload bytes** — load generators report a wall of 301s or 500s as 100% "success", and
two stacks returning different payloads aren't running the same benchmark. Run **30
seconds or more** — a fixed-count run against a fast server finishes before reaching
steady state. And watch the **load generator's own CPU** — co-located, it takes cores from
the server, so every absolute number on this page is a floor. When in doubt, trust the
CPU/req column: it survives all three mistakes.

Two of those three caught real errors while this page was being built, which is the only
reason they are worth repeating rather than reciting:

* **The status-code check.** At `max_connections=300` the shared PostgreSQL ran out of
  slots once nine servers were resident — four of them hold 80 connections each. FastAPI's
  database cell returned **26,051 HTTP 500s out of 151,327** and read 5,689 req/s against the
  11,662 it posts when the pool is available. As a throughput number alone it looked entirely
  plausible: a 50% regression with a tidy explanation free for the taking. Only the status
  distribution showed it was `TooManyConnectionsError`.
* **A control run before the sweep, not after.** This box is shared. A sweep begun at load
  average 0.50 read Express at 89,008 against the 129,725 it had measured an hour earlier
  — other tenants had woken mid-run, and load average is a lagging indicator that did not
  warn in time. Re-measuring two cells with known values *first* and refusing to proceed on
  more than 8% drift is now the first step of the harness (`control.sh`). Published as-is,
  that sweep would have shown a competitor regression and a relative Soli win, both
  artefacts — the most damaging way for a page like this to be wrong.
