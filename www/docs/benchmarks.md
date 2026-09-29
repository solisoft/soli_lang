# Benchmarks

Seven HTTP workloads — a JSON API response, a rendered HTML page, a database read, a
database-backed HTML page, and one create, update and delete per request — through eight
full stacks and one reference configuration, on one machine, with one load generator and one
protocol. Every server returns a **byte-identical payload** for the JSON and DB rows, and every
stack gets the same **16-thread budget**: 16 workers for the six stacks that fork them, 16
threads for Soli, 16 BEAM schedulers for Phoenix.

> **Read this first.** Every HTTP and memory figure on this page was measured on
> 2026-09-28, on one quiet 32-thread machine where each stack, the load generator and the
> databases ran on **cores of their own**. Every sweep opens and closes with a control run
> that refuses to publish if the box drifted by more than 8%, the HTTP status of every
> response was checked, and every write cell counted its table before and after. Where a
> stack beats Soli, the number is printed exactly as measured — **Express wins the WebSocket
> echo by 1.9x**, Phoenix edges Soli on the update row, Express's raw driver out-reads it
> without its query cache, and one Laravel cell carries 50 HTTP 500s that are counted, not
> hidden.
>
> This session **replaces** every earlier table. The machine, every runtime and every
> framework version changed, so no figure below is comparable to one quoted from a previous
> revision of this page. Three WebSocket fan-out rows that need Redis are older, and say so.

## How these numbers were produced

Throughput figures mean nothing without the machine and the conditions behind them, so here
they are in full. **Every req/s on this page is what one stack does with 16 hardware threads
of this particular machine, with the load generator on the same box and every connection over
loopback.** They are not what a production deployment behind a network, TLS and a real mix of
queries would see; the CPU/req column is the figure that travels best to other hardware.

### The machine

| | |
|---|---|
| CPU | AMD Ryzen 9 9950X — 16 cores / 32 threads (SMT on), two 8-core CCDs with 32 MB of L3 each, boost up to 5.76 GHz, `performance` governor, boost enabled |
| Memory | 128 GB DDR5-5600 (4 × 32 GB) |
| Storage | 2 × Samsung PM9A3 NVMe in RAID 1 (PostgreSQL's and SoliDB's data both live here) |
| System | Rocky Linux 10.2, kernel 6.12, bare metal (no virtualisation), `somaxconn` 4096 |
| Network | none: client, servers and databases talk over **loopback**, plain HTTP/1.1 with keep-alive, no TLS |

### How the machine was divided

| Role | Logical CPUs | Physical cores |
|---|---|---|
| The stack under test (16 workers / threads / schedulers) | 0–7, 16–23 | cores 0–7 — the whole first CCD, SMT siblings included |
| `oha`, the load generator | 8–11, 24–27 | 4 cores of the second CCD |
| PostgreSQL 18.6 and SoliDB 2.0.2 | 12–15, 28–31 | the other 4 cores of the second CCD |

Pinning was set with `taskset` and verified on every process, including the ones that rootless
Podman starts outside the launching shell (php-fpm, nginx, FrankenPHP, PostgreSQL). It means a
fast server cannot steal cycles from its own client, and the databases never compete with the
stack for a core. It also means every query crosses from one CCD to the other, as it would to
a database on another socket — a little slower than sharing a cache, and the same for every
stack. Only one stack was loaded at a time; the others sat idle on the same cores.

The client was checked rather than assumed to be out of the way: giving `oha` eight cores
instead of four moved neither of the two fastest cells (Soli `/template` 496,897 → 493,459,
Express `/json` 313,766 → 314,428 — both inside run-to-run noise). With four cores, `oha`
itself is not the bottleneck anywhere on this page.

### What each request does

* **The same request, over and over.** Each cell sends one route for 30 s. Reads return the
  same 50 rows every time, so every cache in the path — PostgreSQL's buffer cache, SoliDB's
  block cache and, where noted, SoliDB's query-result cache and Soli's response cache — is
  hot. A real app's mix of different queries and pages is slower for every stack.
* **Small payloads.** 2,268 bytes of JSON or about 3 KB of HTML. Larger pages shift time into
  serialisation and the kernel.
* **Concurrency 200**, after an 8 s warm-up at 100. Latency figures are the p99 of the 30 s
  measurement.
* **Writes against a buffered log** on both databases (see Writes): durable to a process crash,
  not to a power cut.

### Soli's settings

The binary is Soli 2.6.5 plus the unreleased response-cache work on `main`, built with the
shipped `dist` profile (fat LTO, one codegen unit), run as
`SOLI_WS_WORKERS=0 SOLI_DB_DRIVER=1 soli serve . --port 5080 --workers 16`:

* `--workers 16` — 16 HTTP worker threads, the same budget as every other stack.
* `SOLI_WS_WORKERS=0` — no worker reserved for the realtime split, so all 16 serve HTTP.
* `SOLI_DB_DRIVER=1` — models reach SoliDB over the native MessagePack driver (pooled TCP)
  rather than its HTTP API.
* SoliDB's **query-result cache is on**, which is its default; the uncached rows set
  `SOLI_DB_NO_QUERY_CACHE=1`. Soli's **response cache is on**, which is its default and cannot
  be switched off (see the Template row).
* SoliDB 2.0.2 (commit `25dd515b`, not yet tagged) ran as a separate process with default
  settings on its own data directory.

Every other stack runs in its production mode, with the adjustments listed in Setup — each one
there to make the comparison like for like (a matched pool, a matched serialiser, a matched
durability level), not to favour a stack.

### The protocol

Every published sweep opened and closed with `control.sh`, which re-measures two known cells
(Express `/json`, Soli `/template`) and refuses the run on more than 8% drift. The main sweep
closed at −5.2% and +0.3%; every follow-up pass was bracketed the same way. Every response's
HTTP status was recorded, the JSON and DB payloads were compared byte for byte across all
nine stacks before measuring, and every write cell counted its table before and after. The
host's other workloads — a local LLM service among them — were stopped for the whole run.

## Setup

| | |
|---|---|
| Soli | 2.6.5 plus the unreleased response-cache work on `main` (built with fat LTO, one codegen unit — the shipped `dist` profile), `soli serve .`, 16 HTTP workers, `SOLI_WS_WORKERS=0`; SoliDB **2.0.2** (commit `25dd515b`, not yet tagged) over the **native MessagePack driver** (`SOLI_DB_DRIVER=1`, pooled TCP, not HTTP) for the DB and write rows |
| Rails | 8.1.4 + Puma 8.0.2 on **Ruby 4.0.7** — production, eager-loaded, 16 workers × 5 threads, PostgreSQL via ActiveRecord |
| Laravel | 13.33 on **PHP 8.5.11** (php-fpm, `pm = static`, 16 workers) + nginx (16 workers), in a container with host networking — Eloquent + Blade, OPcache, config/route/view cached, persistent PDO connections, `composer install --no-dev -o` |
| Laravel + Octane | The same application on Octane 2.20 / FrankenPHP 1.12.7, 16 workers, app resident between requests. A **labelled reference row**, not "Laravel": it is a deployment choice that roughly doubles every result |
| Django | 6.1.1 on **Python 3.14.7**, gunicorn 26.2 with 16 workers — Django ORM + Django templates, `DEBUG=False`, persistent connections (`CONN_MAX_AGE`) |
| FastAPI | 0.141.1 on Python 3.14.7 (Starlette 1.7), uvicorn 0.54 with 16 workers on uvloop + httptools — **SQLAlchemy 2.1.1 async + asyncpg 0.31 + Jinja2 3.1.6**: FastAPI ships no ORM and no view layer, so both were added, as for Express. The matched rows return a `Response` directly rather than paying for `jsonable_encoder`, which nothing else here pays for; the default path is a reference row |
| Phoenix | 1.8.15 on **Elixir 1.20.4 / Erlang OTP 29**, Bandit 1.12.5 — **one OS process, 16 BEAM schedulers** (`+S 16:16`), Ecto SQL 3.14 + HEEx, `MIX_ENV=prod`, Phoenix's default `:browser` pipeline on the HTML rows, `force_ssl` removed (left in, every request is a 301 that a load generator counts as success) |
| AdonisJS | **7.5.2** on **Node 26.10**, 16 cluster workers — Lucid 22.4 + Edge 6.5, built to JavaScript and run from `build/`, `NODE_ENV=production`, the same web-kit middleware and providers as its 6.x predecessor (session, shield, static, vite) |
| Express | 5.2.1 on Node 26.10, 16 cluster workers — **+ EJS 6.0 + Sequelize 6.37.8** (on node-postgres 8.23): Express ships no view layer and no DB layer, so both were added. The DB rows go through the **ORM**, not the raw driver; the driver figures are reference rows |
| Database | PostgreSQL **18.6** (`max_connections=500`, `synchronous_commit=off` — see Writes) for the seven PostgreSQL stacks, same table, same 50 rows; SoliDB 2.0.2 for Soli. Both client-server over loopback, no in-process storage anywhere |
| Load | `oha` 1.16 — 30 s at concurrency 200 per cell, after an 8 s warm-up of that cell at concurrency 100 |
| Machine | See *How these numbers were produced* above |

**CPU/req** is server CPU time per request, summed across every process of the stack (all 17
for Rails, Express, AdonisJS and Django; 18 for FastAPI, whose uvicorn supervisor also spawns a
`multiprocessing` resource tracker; php-fpm plus nginx for Laravel; a single process for Soli
and for Phoenix, whose threads `/proc` already aggregates). For Soli the figure in parentheses
adds SoliDB's own CPU for the same requests; PostgreSQL's CPU is **not** added to the other
stacks, so the parenthesised figure is the stricter of the two. CPU/req is the most portable
column here — unlike req/s it barely moves with core count or client speed.

> **One trap this measurement has to survive.** uvicorn's 16 workers are
> `multiprocessing.spawn` children, so a worker's command line reads `python3 -c from
> multiprocessing.spawn import spawn_main; ...` — the app's name appears nowhere in it.
> Summing CPU by command-line pattern, the way Django's is summed (`gunicorn.*benchproj`),
> matches the supervisor alone and once published a FastAPI CPU/req roughly ten times too
> good. The workers share the supervisor's process group, so FastAPI is measured by
> **pgid**. A pattern that silently matches a subset always errs in the flattering direction.

## JSON — 50 objects, 2,268 bytes, built in the handler

| Stack | req/s | p99 | CPU/req | vs Rails |
|---|---:|---:|---:|---:|
| **Soli** | 495,563 | 0.99 ms | 26 µs | 11.4x |
| Express + EJS + Sequelize | 306,503 | 1.25 ms | 47 µs | 7.1x |
| FastAPI + SQLAlchemy + Jinja2 | 176,502 | 2.16 ms | 85 µs | 4.1x |
| Phoenix + Ecto + HEEx | 106,584 | 4.82 ms | 140 µs | 2.5x |
| AdonisJS + Lucid + Edge | 67,484 | 5.87 ms | 228 µs | 1.6x |
| Rails + Puma | 43,369 | 24.28 ms | 326 µs | 1.0x |
| Django + gunicorn | 21,555 | 22.26 ms | 327 µs | 0.5x |
| Laravel + Octane *(reference)* | 15,322 | 19.00 ms | 1,010 µs | 0.4x |
| Laravel + php-fpm | 8,140 | 25.48 ms | 1,883 µs | 0.2x |

Framework overhead only — no database, no template. Soli serves the API response at
**11.4x Rails' throughput on 12x less CPU**, and at 1.6x Express, the fastest of the others.
The CPU column orders the field the same way the throughput does: Soli 26 µs, Express 47,
FastAPI 85, Phoenix 140. On a handler this small, the differences are the cost of the request
path itself — routing, the response object, the header set, the write — and that is exactly
what this row isolates.

## Template — 50-row HTML table + layout, ~3 KB

| Stack | req/s | p99 | CPU/req | vs Rails |
|---|---:|---:|---:|---:|
| **Soli** | 496,129 | 1.03 ms | 26 µs | 15.1x |
| Express + EJS + Sequelize | 149,354 | 1.76 ms | 101 µs | 4.5x |
| Phoenix + Ecto + HEEx | 93,973 | 2.59 ms | 162 µs | 2.9x |
| FastAPI + SQLAlchemy + Jinja2 | 71,775 | 5.37 ms | 218 µs | 2.2x |
| AdonisJS + Lucid + Edge | 65,695 | 5.42 ms | 234 µs | 2.0x |
| Rails + Puma | 32,909 | 27.29 ms | 433 µs | 1.0x |
| Laravel + Octane *(reference)* | 13,618 | 19.84 ms | 1,143 µs | 0.4x |
| Django + gunicorn | 13,134 | 16.15 ms | 1,183 µs | 0.4x |
| Laravel + php-fpm | 7,613 | 27.20 ms | 2,017 µs | 0.2x |

Soli serves the page at **15.1x Rails** and **3.3x Express** — but this row does not measure
Soli's template engine, and it would be misleading to read it that way. Soli keeps a
**response cache** that no other stack here has: a page whose output depends only on the data
passed to `render()` — this one, and the default `soli new` home page — is kept per worker as
the *finished* response (body, injected scripts, ETag), and a request that renders the same
template with the same data gets it back without rendering at all. That is why the row costs
the same 26 µs as the JSON row. It was checked directly: adding a 2,000-iteration loop to this
view left the throughput unchanged.

The cache is real and on by default, so the row is what a Soli app gets for a page like this.
It does not apply to a request that carries a session, or to a layout that calls
`csrf_meta_tag()`, so pages with per-user content take the uncached path, which this page does
**not** measure. A Soli variant of this route that defeats the cache is the one missing cell;
until it exists, compare template *engines* with the other stacks' rows among themselves.

Soli's page is 3,030 bytes against 2,863–2,916 for the rest: it carries the injected
instant-navigation `<script>` tag. The other size differences are a trailing newline here and
there, noted rather than normalised away.

## Database read — 50 rows, projected columns, 2,268 bytes

| Stack | req/s | p99 | CPU/req | vs Rails |
|---|---:|---:|---:|---:|
| **Soli** | 294,002 | 1.18 ms | 36 µs (46 incl. SoliDB) | 9.5x |
| Soli, SoliDB query cache off *(reference)* | 112,180 | 2.12 ms | 36 µs (100 incl. SoliDB) | 3.6x |
| Express + EJS + Sequelize | 94,755 | 3.35 ms | 156 µs | 3.1x |
| Phoenix + Ecto + HEEx | 67,843 | 4.86 ms | 220 µs | 2.2x |
| AdonisJS + Lucid + Edge | 44,781 | 7.91 ms | 341 µs | 1.5x |
| Rails + Puma | 30,841 | 13.36 ms | 494 µs | 1.0x |
| FastAPI + SQLAlchemy + Jinja2 | 23,958 | 21.87 ms | 640 µs | 0.8x |
| Django + gunicorn | 19,060 | 21.73 ms | 525 µs | 0.6x |
| Laravel + Octane *(reference)* | 13,055 | 17.35 ms | 1,126 µs | 0.4x |
| Laravel + php-fpm | 6,537 | 32.23 ms | 2,215 µs | 0.2x |

Two Soli rows, because SoliDB does something no other database here does. It **memoizes
read-only query results** per (database, query, bind variables) and replays a repeat with no
query work at all; this benchmark issues the same query every request and never writes
`posts`, so the hit rate is 100%. PostgreSQL's buffer cache spares the disk but still plans,
executes and serialises every request. The main Soli row is what an app gets by default; the
reference row turns the memoization off with `SOLI_DB_NO_QUERY_CACHE=1` and is the
like-for-like database comparison.

**Without the cache, Soli still leads the ORM rows**: 112,180 req/s against Express's 94,755
and Phoenix's 67,843, and **3.6x Rails**. Express on the raw `pg` driver, with no ORM in the
path, reads faster still — 160,230 (reference cells below). Every other row here pays its ORM,
and Soli's pays its model layer.

**SoliDB 2.0.2 is what made that true.** On 2.0.1, measured earlier the same day on the same
box, the uncached row read **38,086 req/s** — behind Phoenix, Express and AdonisJS — with
SoliDB spending **184 µs** of CPU per request against Soli's own 35. That measurement is why
2.0.2 exists: it cut SoliDB's CPU on this query to **64 µs** (−65%) and tripled the row. With
the cache on, 2.0.2 moved the read from 238,308 to 294,002 req/s (SoliDB 25 → 10 µs).

Every Soli row that touches the database — both read rows here, the page render and the three
writes — is from that 2.0.2 run, bracketed by its own controls. Those controls read **+4.2%
and +6.4%** against the session's calibration: the box was a little faster than during the
main sweep, which can flatter these Soli rows by a few percent against the other stacks. The
JSON and template rows touch no database and are from the main sweep, with everyone else.

> **The two rows are not the same query path, and the cached one is not a like-for-like
> result.** A cached read is a key lookup and a copy; the nearest equivalent for the other
> stacks would be `Rails.cache.fetch` or Django's `cache_page` around the action, which this
> page would rightly refuse to count. Quote the uncached row when comparing databases, and the
> cached row when describing what a Soli app does out of the box on repeated identical reads.

Every stack serves the same self-describing hash rows through its fastest idiom for that
shape: Soli's `Post.pluck(:id, :title, :views).all` builds the hashes **in the database**;
Rails' is `pluck` + `map`; Sequelize uses `raw: true`, Eloquent `toBase()`, Django `.values()`,
SQLAlchemy `select(Post.id, Post.title, Post.views)`, Ecto a `select` map. Projection has to
happen in the database — the hydrating form of the same query costs every ORM but one:

| Instead of projecting | req/s | vs its own projected row |
|---|---:|---:|
| FastAPI, 50 mapped SQLAlchemy objects (`select(Post)`) | 13,732 | 0.57x |
| Django, 50 model objects (`.only()` instead of `.values()`) | 17,901 | 0.94x |
| Phoenix, 50 Ecto structs (`Repo.all(Post)`) | 64,399 | 0.95x |

`Post` has exactly the three columns the projection selects, so each ORM fetches identical
bytes in both forms and the only difference is what it builds from them. Building mapped
SQLAlchemy objects — identity map, instrumented attributes — costs FastAPI **43%** of this
row; a Django model or an Ecto struct costs 5–6%. FastAPI's p99 on the hydrated read is
100 ms.

## Database read + HTML render — 50 rows from the database into a page, ~3 KB

The row a server-rendered framework lives on: query, then render — the `/db` read and the
`/template` render in one request, so the page is the Template row's page and the database is
the only added variable.

| Stack | req/s | p99 | CPU/req | vs Rails |
|---|---:|---:|---:|---:|
| **Soli** | 292,318 | 1.12 ms | 37 µs (47 incl. SoliDB) | 11.7x |
| Soli, SoliDB query cache off *(reference)* | 112,465 | 1.99 ms | 37 µs (101 incl. SoliDB) | 4.5x |
| Phoenix + Ecto + HEEx | 72,263 | 4.40 ms | 208 µs | 2.9x |
| Express + EJS + Sequelize | 70,143 | 4.04 ms | 214 µs | 2.8x |
| AdonisJS + Lucid + Edge | 42,707 | 8.22 ms | 358 µs | 1.7x |
| Rails + Puma | 25,024 | 15.92 ms | 616 µs | 1.0x |
| FastAPI + SQLAlchemy + Jinja2 | 19,497 | 27.48 ms | 791 µs | 0.8x |
| Laravel + Octane *(reference)* | 11,759 | 19.04 ms | 1,259 µs | 0.5x |
| Django + gunicorn | 10,896 | 19.72 ms | 1,390 µs | 0.4x |
| Laravel + php-fpm | 6,174 | 34.08 ms | 2,352 µs | 0.2x |

The Template row's caveat applies here too: with the query cache off, Soli runs the query on
every request, but the page rendered from its result can still come out of the response cache,
because the result is the same each time. The uncached row therefore measures the database
path in full and the render path at its best.

Soli takes the row with or without its query cache — **4.5x Rails' throughput uncached**,
ahead of Phoenix (72,263) and Express (70,143). What the render costs once a query is in the
request differs sharply by stack: **nothing for Soli** — its uncached page and its uncached
JSON read are the same 112k req/s — and nothing for Phoenix, whose page is slightly *faster*
than its JSON read, but **26% for Express** (94,755 → 70,143) and 45% for Django.

## Writes — create, update and delete, one row per request

Each of these measures one write per request against an isolated table, reset to exactly its
seeded state before every cell so no stack inherits a table another stack grew or emptied.
Update and delete address one row by primary key, drawn at random from the whole table in
every stack.

**Durability is matched.** SoliDB's writes go through RocksDB's default path — the WAL reaches
the operating system but is not `fsync`ed before the write returns. PostgreSQL's default
(`synchronous_commit=on`) flushes before commit returns, a stronger and much slower guarantee.
So PostgreSQL runs with `synchronous_commit=off`, the setting that matches what SoliDB
promises: survive a process crash, not a power cut. Neither column is "durable writes".

### Create — one INSERT per request

| Stack | req/s | p99 | CPU/req | vs Rails |
|---|---:|---:|---:|---:|
| **Soli** | 135,103 | 1.78 ms | 22 µs (60 incl. SoliDB) | 4.3x |
| Phoenix + Ecto + HEEx | 124,305 | 3.65 ms | 114 µs | 3.9x |
| Express + EJS + Sequelize | 91,537 | 3.45 ms | 162 µs | 2.9x |
| AdonisJS + Lucid + Edge | 53,772 | 6.83 ms | 282 µs | 1.7x |
| Rails + Puma | 31,778 | 12.92 ms | 473 µs | 1.0x |
| FastAPI + SQLAlchemy + Jinja2 | 29,035 | 20.69 ms | 526 µs | 0.9x |
| Django + gunicorn | 19,957 | 22.19 ms | 459 µs | 0.6x |
| Laravel + Octane *(reference)* | 13,175 | 16.88 ms | 1,105 µs | 0.4x |
| Laravel + php-fpm | 6,372 | 33.06 ms | 2,260 µs | 0.2x |

Laravel's php-fpm cell returned **50 HTTP 500s out of 190,961 responses** (0.03%). Neither
php-fpm, nginx nor Laravel logged a cause; the figure is printed with the failures counted
rather than re-run until clean. The delete run on the same stack, later, returned none.

### Update — one row by primary key

| Stack | req/s | p99 | CPU/req | vs Rails |
|---|---:|---:|---:|---:|
| Phoenix + Ecto + HEEx | 123,792 | 3.57 ms | 114 µs | 3.3x |
| **Soli** | 120,551 | 2.30 ms | 22 µs (74 incl. SoliDB) | 3.2x |
| Express + EJS + Sequelize | 85,882 | 3.83 ms | 173 µs | 2.3x |
| AdonisJS + Lucid + Edge | 58,444 | 6.36 ms | 259 µs | 1.6x |
| Rails + Puma | 37,502 | 12.29 ms | 394 µs | 1.0x |
| FastAPI + SQLAlchemy + Jinja2 | 27,576 | 20.27 ms | 555 µs | 0.7x |
| Django + gunicorn | 19,475 | 22.47 ms | 481 µs | 0.5x |
| Laravel + Octane *(reference)* | 13,616 | 16.54 ms | 1,049 µs | 0.4x |
| Laravel + php-fpm | 6,538 | 32.15 ms | 2,180 µs | 0.2x |

### Delete — one row by primary key, on a 20-million-row table

| Stack | req/s | p99 | CPU/req | vs Rails | rows removed |
|---|---:|---:|---:|---:|---:|
| **Soli** | 144,982 | 1.97 ms | 25 µs (69 incl. SoliDB) | 3.8x | 90% of requests |
| Phoenix + Ecto + HEEx | 82,431 | 28.99 ms | 117 µs | 2.1x | 94% of requests |
| Express + EJS + Sequelize | 74,373 | 26.21 ms | 104 µs | 1.9x | 95% of requests |
| AdonisJS + Lucid + Edge | 51,683 | 16.50 ms | 274 µs | 1.3x | 96% of requests |
| Rails + Puma | 38,586 | 12.68 ms | 356 µs | 1.0x | 97% of requests |
| FastAPI + SQLAlchemy + Jinja2 | 30,031 | 18.84 ms | 501 µs | 0.8x | 98% of requests |
| Django + gunicorn | 18,523 | 22.44 ms | 543 µs | 0.5x | 99% of requests |
| Laravel + Octane *(reference)* | 13,641 | 16.57 ms | 1,034 µs | 0.4x | 99% of requests |
| Laravel + php-fpm | 6,579 | 32.11 ms | 2,154 µs | 0.2x | 100% of requests |

**This row had to be re-measured, and why is the most useful thing on this page.** The first
sweep ran delete, as every earlier session had, against the 800,000-row table the create and
update rows use. Delete is the one operation that consumes its workload: a key already deleted
is a miss, and a miss is far cheaper than a delete. On this faster machine the fast stacks
exhausted that table within seconds. Express posted 144,612 req/s having actually removed a
row on **18%** of its requests, Phoenix 132,241 on 20% — together they had emptied most of the
table and spent the rest of the cell deleting nothing. Published, that row would have ranked
stacks by how quickly they ran out of work.

A key drawn at random from *N* rows reaches *N*·(1 − e^(−k/*N*)) distinct rows after *k*
requests. At the ~4.3 million requests the fastest stack makes in 30 s, 800,000 rows gives it
an 18% hit rate; **20 million rows gives 90%** — Soli, the fastest, measured exactly that — and
every PostgreSQL stack stayed at 94% or above.
The table size is now a harness knob (`WPOOL`, read by every app at boot), and this row was
re-measured on all nine stacks with it; the create and update rows, whose every request does
real work at any table size, were not. The "rows removed" column is still measured, not
assumed, so the remaining few percent of misses stay visible.

**Soli and Phoenix are the two write stacks here**, and they trade places: Soli leads create
(135,103 against 124,305, 4.3x Rails), **Phoenix leads update** (123,792 against 120,551 —
by 2.7%, inside the spread of two runs), and on delete Soli leads clearly: 144,982 req/s against Phoenix's 82,431, 3.8x Rails. Soli's rows-removed figure is the lowest in the table (90%) precisely because it made the most requests, so compare real deletes per second instead — about **130,000 for Soli, 77,000 for Phoenix, 71,000 for Express** — and the lead holds at 1.7x Phoenix.  What separates them is CPU. Soli's own process spends 22 µs per write
against Phoenix's 114, but SoliDB adds 38–52 µs to that, and PostgreSQL's CPU is not counted
for Phoenix at all. The honest reading is that the two are **peers on writes**, and that both
are in a different class from everything else here.

Every stack writes more slowly than it reads — Soli's create row is a quarter of its template
row — because a write has to reach another process and a log, whichever framework issued it.

## Reference cells

The "what if you wrote it the other way" figures, measured in the same session with the same
protocol, so each is comparable to the matched row it sits beside.

| Cell | req/s | p99 | matched row |
|---|---:|---:|---:|
| Express `/db` on the raw `pg` driver, no Sequelize | 160,230 | 2.12 ms | 94,755 |
| Express `/db-template` on the raw driver | 101,485 | 3.00 ms | 70,143 |
| FastAPI `/json` through the default `jsonable_encoder` path | 53,740 | 7.76 ms | 176,502 |
| FastAPI `/db` through `jsonable_encoder` | 18,110 | 30.17 ms | 23,958 |
| FastAPI `/db`, 50 hydrated SQLAlchemy objects | 13,732 | 100.24 ms | 23,958 |
| Django `/db`, 50 hydrated model objects | 17,901 | 22.08 ms | 19,060 |
| Phoenix `/db`, 50 hydrated Ecto structs | 64,399 | 5.10 ms | 67,843 |

Two of these are worth reading as findings in their own right. **Sequelize costs Express 41% of
its database read** — the raw driver row is 1.7x the ORM row — which is why the ORM row is
the one published: every other stack here pays an ORM too. And **FastAPI's default return
path costs it 70% of its JSON row**: returning the list sends it through `jsonable_encoder`, a
serialisation framework nothing else here runs, so the matched row returns a `Response`
directly — a documented FastAPI idiom — and the default is printed here instead.

## WebSockets — echo and fan-out

`oha` speaks HTTP only, so these use a purpose-built client (`ws_bench.js`), run as **eight
sharded processes** on the client cores (8–15 and 24–31 — all eight free cores, because a
single Node process saturates long before either server). Both servers ran on the same 16
threads as their HTTP rows, from the same processes. The pass was bracketed by the HTTP
control at both ends.

Two stacks are measured here: Soli and Express, whose `ws` handlers live in the same
processes as their HTTP rows. Rails' ActionCable needs Redis and its own JSON subprotocol and
was not re-measured on this machine; Django (Channels) and Laravel (Reverb) would need a
*different server process* from the one serving their HTTP rows, so they are absent rather
than misrepresented.

> **Soli's WebSocket abuse limits were switched off for this section, and they would
> otherwise have decided it.** By default Soli refuses more than **64 sockets from one peer
> address** (`SOLI_WS_MAX_CONNECTIONS_PER_IP`) and closes any socket that sustains more than
> **100 frames per second** beyond a burst of 200 (`SOLI_WS_MAX_MESSAGES_PER_SEC`,
> `SOLI_WS_MESSAGE_BURST`). A benchmark client breaks both at once: every socket comes from
> 127.0.0.1, and an echo client sends as fast as it can. With the defaults, Soli held 64 of
> 1,000 connections and echoed **899 msg/s** — a measure of the limits, not of the server.
> Express enforces neither, so both Soli limits were set to `0` here. In production, behind a
> load balancer that forwards real client addresses, keep them.

### Echo — round trip, one message in flight per connection

| Stack | msg/s | p50 | p99 | connections |
|---|---:|---:|---:|---:|
| Express + ws | **1,163,734** | 0.80 ms | 1.75 ms | 1,000 |
| **Soli** | 620,447 | 1.51 ms | 3.52 ms | 1,000 |

**Express takes this row outright, at 1.9x Soli's message rate and half its latency.** It is
the one workload on this page where the architecture of the server, rather than its
per-request cost, decides the result, and a profile of Soli under this load shows where.
For every echoed message Soli's socket task reads the frame and hands it to a worker thread
(where the `ws#echo` handler runs); the worker hands the reply back through the connection
registry to the socket's separate writer task, which sends it. Express's `ws` reads, runs the
handler and writes in one function on one thread. In a profile of the same workload (taken on
a separate 16-thread machine, so the proportions carry over and the absolute numbers do not),
about 70% of Soli's samples are on the I/O side of that round trip — the `send` system call
alone is 29%, which Express pays too — while the Soli handler itself, across every worker
thread, is about 30%. Cutting the second
hand-off (letting the socket task write the handler's reply to its own sender directly) is
where the gap would close; batching the worker's drain of the queue, already tried, gained
1.7%.

Neither number is limited by the client. In an earlier pass — whose closing control failed,
so its figures are not published — the eight shards on four cores instead of eight left Soli
unchanged (621,809) and held Express back (915,586); four client cores are not enough for
Express, eight are.

### Fan-out — one publisher, every connection in the room receives

| Stack | reached per publish | share of the room | deliveries/s |
|---|---:|---:|---:|
| **Soli**, 16 workers | **1,000 of 1,000** | 100% | 46,685 |
| Express + ws, 16 workers, no bus | 64 of 1,000 | **6%** | 2,944 |

The publisher is rate-limited (50 publishes/s), so a complete broadcast reads the same for
every stack — 1,000 connections × 50 × the test's duty cycle — and this row is about
completeness, not speed. **Soli's 16 workers are threads in one process**, so a broadcast
reaches every connection the server holds with nothing configured. Node's `cluster` gives each
worker its own sockets, so the obvious implementation reaches only the ~1/16th the publishing
worker accepted — and it does not error, it silently delivers to 6% of the room.

Measured in an earlier session on a different machine, and not repeated here: Express with a
Redis bus, Express on one worker, and Rails + ActionCable on Redis all reached 1,000 of 1,000
at the same ~45,000 deliveries/s as Soli. Equal throughput, unequal defaults: the naive Soli
implementation is right; the naive clustered-Node one is quietly wrong, and fixing it means
Redis and a hop per publish, or one worker.

### Connections

| Stack | 1,000 connections opened | connect rate | memory held |
|---|---:|---:|---:|
| **Soli** | 1,000 (0 failed) | 651/s cold, 9,009/s warm | 126 → 152 MB (~27 KB per socket) |
| Express + ws | 1,000 (0 failed) | 9,804/s | ~905 MB total, flat |

Once warm, both accept about 9,000–10,000 connections per second. **Soli's first burst of
connections after a start is fourteen times slower** (651/s), and a second burst on the same
server runs at 9,009/s: a one-time warm-up cost on the WebSocket route, not a steady-state
one — but a real one for a server that has just restarted under load, and the second thing on
this route worth fixing. Soli holds 1,000 idle sockets in 26 MB of additional memory; Express's
16 workers already occupy about 900 MB before the first socket opens.

## Memory

| Stack | Processes | Idle | Under load |
|---|---:|---:|---:|
| **Soli** | 1 × 16 threads | **97 MB** | **126 MB** |
| Phoenix + Ecto + HEEx | **1** (16 BEAM schedulers) | 101 MB | 170 MB |
| Laravel + php-fpm | 17 (fpm + nginx) | 104 MB | 146 MB |
| Rails + Puma | 17 (fork + CoW) | 221 MB | 1,157 MB |
| Laravel + Octane *(reference)* | 16 resident workers | 331 MB | 335 MB |
| Django + gunicorn | 17 (fork + CoW) | 624 MB | 704 MB |
| Express + EJS + Sequelize | 17 (fork + CoW) | 668 MB | 1,599 MB |
| AdonisJS + Lucid + Edge | 17 (fork + CoW) | 898 MB | 2,467 MB |
| FastAPI + SQLAlchemy + Jinja2 | 18 (**spawn**, no CoW) | 1,013 MB | 1,044 MB |

Figures are **PSS** (proportional set size) summed over the whole process group — the honest
measure for multi-process servers, because summing RSS counts every fork-shared page 17
times. Every stack was **restarted immediately before this table**, and the restart was
verified by process id; "under load" is read at the end of a 30 s run on the DB + HTML route.

**The only three stacks near 100 MB are the only three that are not a fleet of large forked
processes** — Soli's 16 workers are threads in one process, Phoenix is one BEAM, and php-fpm
keeps almost nothing resident between requests because it rebuilds the framework for each
one. Of those, only Soli and Phoenix keep their application loaded and stay small under
load: Soli grows by 29 MB, Phoenix by 69. Rails more than quintuples under load (221 → 1,157
MB), Express more than doubles, and AdonisJS reaches 2.5 GB. FastAPI is the row where the
process model, not the libraries, is the story: uvicorn's workers are **spawned**, so each of
16 imports FastAPI, Pydantic, SQLAlchemy and Jinja2 into a fresh interpreter with nothing
shared — Django carries a comparable pile of Python and sits 389 MB lower because gunicorn
forks and copy-on-write shares the parent's modules.

> **Two measurement methods.** The seven native stacks are measured as PSS. The two Laravel
> stacks run in containers whose processes belong to another user namespace, so
> `smaps_rollup` is unreadable and a PSS sum would silently skip them — they are measured from
> their **cgroup** (`podman stats`) instead. cgroup usage and PSS are not the same metric;
> read the Laravel rows against each other and only as indicative against the rest.

> **This table was taken twice, and the first one was wrong.** The session's own memory pass
> reported Soli at 151 MB. The harness restarts stacks before measuring memory, but it found
> Soli's process by the binary name `soli` — and the benchmark binary was named something
> else, so the old process survived, the new one lost the port, and the "idle" reading was a
> server that had been under load for hours. It is the same retained-heap trap this section
> warns about for Rails, caught by comparing process ids across the restart. The harness now
> kills every stack by the port it listens on, and the table above is from a verified cold
> restart.

## What these multiples do and don't mean

Trivial handlers measure fixed framework overhead, which is where the gaps are widest: 11x
Rails on JSON, and 15x on the template, where Soli's response cache is also at work. On routes dominated by real database work the multiple
compresses — 3.6x on the uncached read, 3–4x on the writes — because the database, the
transport to it and the ORM take over the request, whatever framework issued it. The claim
this page supports is: **Soli's framework overhead is a small fraction of every other stack's
here, it leads the database rows even with its query cache off, and on writes Phoenix is its
peer — ahead on update, behind on create.**
Quote it that way — and quote the uncached database row, not the cached one, when the
question is databases.

Pick a stack on the row that looks like your app. The spreads that matter are rarely the
framework's: they are the data layer (Ecto at ~115 µs per write against SQLAlchemy at
~500–550), the template engine (Jinja2 adds ~130 µs over FastAPI's JSON row), the transport to
the database, and the tail under a bounded pool — FastAPI's p99 on every database row is
20–30 ms, where Soli's is 1–2.

## Reproducing

Everything above comes from the harness in `bench/frameworks/` — nine apps, one launcher, one
sweep:

```bash
./seed.sh          # posts (50 rows) + wposts in PostgreSQL and SoliDB
./start.sh         # every stack, pinned to SERVER_CPUS when set
./session.sh       # control -> sweep -> references -> control -> restart -> memory
```

On a box other than the one a session was calibrated on, `lib.sh` holds the knobs:
`SERVER_CPUS` and `CLIENT_CPUS` pin the stacks and `oha`, `SDB_PORT` points at a dedicated
SoliDB, `WPOOL` sizes the write table (20,000,000 for the delete row above), and
`WORKLOADS`/`WRITES` narrow a sweep to the rows being re-measured. Laravel runs under Docker
Compose, or under rootless Podman with the `*.podman.yml` overlays.

Three checks before believing any result, including this one:

* **Verify status codes and payload bytes.** Load generators report a wall of 301s or 500s as
  100% "success", and two stacks returning different payloads are not running the same
  benchmark. This session's seed script produced a SoliDB read table without the `id` field the
  Soli app reads; every Soli `/db` response came back with `"id": null` in string-key order —
  2,377 bytes where every other stack sent 2,268 — and only the byte comparison caught it.
* **Bracket every sweep with a control run.** `control.sh` re-measures two known cells before
  *and after* and refuses a drift of more than 8%. Load average is a lagging indicator; a box
  can degrade mid-run and produce uniformly lower, plausible-looking numbers.
* **Make sure the workload survives the run.** A delete benchmark that empties its own table
  measures misses; a memory reading after a "restart" that did not happen measures retained
  heap. Both happened while this page was being built, and both looked like results.
