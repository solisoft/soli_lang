# Framework benchmark suite

The applications behind [`www/docs/benchmarks.md`](../../www/docs/benchmarks.md),
plus the harness that measures them. Every app serves the same seven matched
workloads over the same data, and every one runs **16 workers**.

| Stack | Port | Server | ORM / templates |
|---|---:|---|---|
| [Soli](soli/) | 5080 | `soli serve`, 16 worker threads | Model / ERB |
| [Rails](rails/) | 5096 | Puma, 16 workers × 5 threads | ActiveRecord / ERB |
| [Express](express/) | 5097 | Node cluster, 16 workers | Sequelize / EJS |
| [AdonisJS](adonis/) | 5102 | Node cluster, 16 workers | Lucid / Edge |
| [Laravel](laravel/) | 5098 | php-fpm 16 workers + nginx (Docker) | Eloquent / Blade |
| [Laravel Octane](laravel/) | 5100 | FrankenPHP, 16 resident workers (Docker) | Eloquent / Blade |
| [Django](django/) | 5099 | gunicorn, 16 workers | Django ORM / Django templates |
| [FastAPI](fastapi/) | 5103 | uvicorn, 16 workers (uvloop) | SQLAlchemy 2.0 async + asyncpg / Jinja2 |
| [Phoenix](phoenix/) | 5104 | Bandit, **1 OS process / 16 BEAM schedulers** | Ecto / HEEx |

**5101 is taken** — it is Octane's `--admin-port`, so a new stack starts at 5103.

Phoenix is the only stack besides Soli that is **not** 16 OS processes. The BEAM runs
one process with one scheduler thread per core (`+S 16:16` pins it) and a lightweight
process per connection, which is the same shape as Soli's 16 worker threads. That makes it
the only column that tests whether Soli's memory and fan-out advantages are architectural
or merely unique-among-forking-runtimes.

## The workloads

| Route | What it measures |
|---|---|
| `GET /json` | 50 objects built in the handler, serialised — framework overhead |
| `GET /template` | the same 50 rows through the template engine |
| `GET /db` | 50 rows projected in the database, as JSON |
| `GET /db-template` | the same read rendered as HTML — the real server-rendered page |
| `POST /w` | one INSERT per request |
| `PATCH /w` | one row updated by primary key |
| `DELETE /w` | one row deleted by primary key |

## Running

```bash
./seed.sh          # posts (50 rows) + wposts (800,000) in PostgreSQL and SoliDB
./start.sh         # bring up all seven no-build stacks, wait for health
./session.sh       # <-- USE THIS: one publishable session, guarded on both sides
```

`session.sh` runs `control → sweep → refs → restart → memory → control` and exits
non-zero if either control fails. Prefer it to running the steps by hand: it is
what makes a session publishable rather than merely finished. Individual steps
are still there for iteration:

```bash
./control.sh       # is this box comparable to the published session?
./sweep.sh         # every matched cell, back to back
./refs.sh          # the labelled reference cells, same protocol
./memory.sh        # PSS per stack — restart the stacks first, see below
```

**The closing control is the one that matters.** A pre-flight check cannot catch a
box that degrades *mid-run*, and that is precisely what spoiled two attempts here:
a sweep that opened at load average 0.50 and finished with every stack halved
(Django 3,065 against a true 7,085, Laravel 2,134 against 4,502). Nothing in the
output looked wrong — no errors, no timeouts, just uniformly lower numbers with a
plausible story available for free. Re-running the control at the end is what
turns that from a publishable-looking result into a discarded one.

**Run `control.sh` before every sweep.** It re-measures two cells whose published
values are known and fails if either has drifted more than 8%. This is not
ceremony — it exists because the failure it catches already happened: a sweep
started at load average 0.50 and read Express at 89,008 against a published
129,725 (**−31%**) and Rails at 12,072 against 16,116 (−25%), because other
tenants on this shared box had woken up mid-run. Load average is a lagging
indicator and did not warn in time; nothing else in the harness noticed either.
Published as-is, those numbers would have looked like a Soli-relative *win* and a
competitor regression, which is the most damaging way for this page to be wrong.

`memory.sh` also wants every stack **freshly restarted**: read after an hour of
sweeping, the same servers report Rails at 1,090 MB against 224 MB from a clean
start. That is retained heap, not footprint, and the column is labelled "idle".

`sweep.sh` warms each cell for 8s at c=100, then measures 30s at c=200, and
reports req/s, p99 and CPU-time per request summed over **every process** of the
stack. Write cells reset their dataset first and count rows before and after, so
the delete row's hit rate is measured rather than assumed.

AdonisJS builds to JavaScript first (`node ace build`) and runs from `build/`
via `adonis/start-bench.sh`; it needs `pg`, `luxon` and `@types/pg` installed.
It is the one stack `start.sh` does not launch, for that reason.

FastAPI needs `fastapi`, `uvicorn[standard]`, `sqlalchemy[asyncio]`, `asyncpg`
and `jinja2`; `start.sh` calls `fastapi/start-bench.sh`, which holds the flags.

Phoenix needs Elixir + Erlang/OTP and a one-off `MIX_ENV=prod mix compile` in
`phoenix/`; `start.sh` calls `phoenix/start-bench.sh`. It runs `MIX_ENV=prod`, and
`config/prod.exs` **removes** the generated `force_ssl` — left in, every request
becomes a 301 and `oha` reports a wall of them as 100% success.

Requires `oha`, `psql`, a PostgreSQL on `127.0.0.1:5433` (user/db `bench`) and a
SoliDB on `6745`. Override with `PGURL` and `SDB`.

### PostgreSQL needs `max_connections >= 500`

Not a nicety — the suite does not fit below it. Four stacks hold **80 connections
each** (Rails 16×5, Express 16×5, FastAPI 16×5, Phoenix's single VM-wide Ecto
pool), which is 320 before Django's 16, AdonisJS's pool and Laravel's persistent
PDO handles. Pools are held for as long as the server runs, so the peak is the
sum across every resident stack, not the stack under test.

At `max_connections=300` the ninth stack simply could not connect: FastAPI's
`/db` cell returned **26,051 HTTP 500s out of 151,327** with
`asyncpg.exceptions.TooManyConnectionsError`, and its throughput read 5,689
against a true 11,436 — a fabricated 50% regression that had nothing to do with
FastAPI. `sweep.sh`'s status-code check is what caught it (`!! {'200': …, '500':
…}` in the cell output); the req/s figure alone looked plausible. That check is
the only reason this did not get published.

The bench container is started with the limit on the command line, which
overrides `postgresql.auto.conf`, so raising it means recreating the container:

```bash
docker rm -f benchpg
docker run -d --name benchpg --network host \
  -e POSTGRES_USER=bench -e POSTGRES_PASSWORD=bench -e POSTGRES_DB=bench \
  -v <existing-volume>:/var/lib/postgresql \
  postgres:latest -c port=5433 -c max_connections=500
```

The data lives in an external volume and survives. Restart every stack
afterwards — their pooled connections point at a container that no longer
exists, and most pools will not notice until a request fails.

## Rules the comparison depends on

These are the things that decide the result before a single request is sent, so
they are worth stating plainly.

* **Same payload.** `/json` and `/db` return **byte-identical** responses from
  every stack (2,268 bytes). Template output differs by a few bytes per
  stack — Soli's page carries its instant-navigation script, Rails emits one
  extra newline — and those differences are noted on the page rather than
  hidden. Laravel, Django and FastAPI render the same 2,864 bytes.
* **Every stack goes through its ORM**, and each uses the form that projects
  *without* instantiating models: Rails `pluck`, Soli `pluck`, Sequelize
  `raw: true`, Eloquent `toBase()`, Django `.values()`, SQLAlchemy
  `select(Post.id, ...)`, Ecto `select: %{id: p.id, ...}`. Measuring one stack on
  a raw driver against the rest flattered it by 32%, which is why Express's
  driver figure is now published as a labelled reference instead.
* **Every stack pools its database connections.** Without it, php-fpm and
  gunicorn open a fresh connection per request — worth ~8ms on loopback, and
  enough to cost Laravel two thirds of its database throughput. That is
  measuring connection setup, not the framework. Every pool is 80 connections —
  5 per worker for the six forking stacks (FastAPI's `max_overflow=0` keeps that
  cap hard), and Ecto's single VM-wide `POOL_SIZE=80` for Phoenix, which has no
  per-worker pool to size. Django is the exception at 16, one per sync worker.
* **No stack runs a serialization framework the others don't.** FastAPI's
  published rows return a `Response` directly rather than letting
  `jsonable_encoder` walk the payload, because nothing else here pays that cost;
  the default path is published beside it as a labelled reference.
* **Durability is matched on the write rows.** SoliDB acks before `fsync`;
  PostgreSQL's default does not. PostgreSQL therefore runs with
  `synchronous_commit=off` for those cells, which is the setting that matches
  what SoliDB actually promises. Neither side is "durable writes".
* **Laravel runs on php-fpm, not Octane**, and Soli runs with the realtime
  worker split off (`SOLI_WS_WORKERS=0`) so all 16 workers serve HTTP. Both are
  choices that move the numbers a long way; both are stated rather than assumed.

## WebSockets

`./ws_sweep.sh` measures what `oha` cannot, on the three stacks that serve
sockets from the same process as their HTTP rows. Rails' ActionCable speaks a
JSON subprotocol rather than raw WebSocket, so the client negotiates
`actioncable-v1-json` and completes welcome → subscribe → confirm before timing
anything (`PROTOCOL=actioncable`).

### Echo — round trip, 1,000 connections

| Stack | msg/s | p50 | p99 |
|---|---:|---:|---:|
| Express + ws | **411,340** | 2.01 ms | 6.83 ms |
| Soli | 241,790 | 4.02 ms | 6.20 ms |
| Rails + ActionCable | 98,529 | 0.58 ms | **63.83 ms** |

Rails' p50 is not a win. At 98,529 msg/s over 1,000 connections the *mean* must
be 10.15 ms — seventeen times its median — because ActionCable dispatches
through a bounded worker pool, so some sockets are served immediately and the
rest queue. Soli and Express hold p99 within ~3x of their medians; Rails' spread
is 110x.

### Fan-out — one publisher, rate-limited

| Stack | reached per publish | deliveries/s |
|---|---:|---:|
| Soli, 16 workers | **1,000 of 1,000** | 45,264 |
| Rails + ActionCable, 16 workers + Redis | 1,000 of 1,000 | 45,545 |
| Express + ws, 16 workers + Redis | 1,000 of 1,000 | 44,217 |
| Express + ws, 1 worker | 1,000 of 1,000 | 44,106 |
| Express + ws, 16 workers, no bus | 63 of 1,000 | 2,846 |

**Equal throughput, unequal defaults.** Once the broadcast is complete all four
are one number. Soli's workers are threads in one process, so it reaches every
connection with nothing configured; clustered Node and ActionCable each need
Redis, and clustered Node without it *silently* delivers to 6% of the room
rather than erroring.

Two things this harness had to get right, both of which produced confident wrong
answers first:

* **Shard the client.** A single Node client saturates long before the servers:
  it reported 74k msg/s against Soli where eight client processes reported 238k,
  and throughput *fell* as connections rose while latency scaled linearly — the
  signature of a bottlenecked generator. `SHARDS` defaults to 8.
* **Rate-limit the publisher.** Pumping broadcasts flat out just outruns the
  server — a publish costs the client nothing and the server N sends — so the
  ratio collapses and measures the client's send loop. At a fixed rate
  (`PUBLISH_RATE`, default 50/s) it means what it says.

Django and Laravel have no WebSocket rows: Channels needs ASGI and Reverb is a
separate process, so neither would be the same server that produced their HTTP
rows. FastAPI *is* ASGI and could serve `/ws` from the same uvicorn workers, but
its 16 workers are separate processes with no shared bus, so fan-out would land
in exactly the clustered-Node trap below — worth measuring, not yet measured.

## Two traps worth knowing

### uvicorn workers are invisible to a cmdline match

`uvicorn --workers 16` spawns its workers through `multiprocessing.spawn`, so a
worker's cmdline reads `python3 -c from multiprocessing.spawn import
spawn_main; ...` — **the app name is not in it**. Measuring FastAPI the way
`sweep.sh` measures Django (`cpu_pat 'gunicorn.*benchproj'`) matches the
supervisor alone: 8 CPU ticks against the process group's 88, which would have
published a CPU/req roughly ten times too good. The workers do share the
supervisor's pgid, so `sweep.sh` and `memory.sh` measure this stack by **process
group**, and both carry a comment saying why the obvious pattern branch is
absent. `start.sh` kills it by pgid for the same reason.

This is the same failure mode as the Octane memory bug in `memory.sh`'s header:
a pattern that silently matches a subset always errs in the flattering
direction.

### Octane's CPU column read 0µs (fixed)

The mirror image of the same bug, found while adding FastAPI. `srv_cpu` had no
`octane` branch, so it fell through to `cpu_grp "$(listener 5100)"` — and
`ss -ltnp` cannot see a pid inside a container as an unprivileged user, so the
listener lookup returned empty and the sum added **nothing**. Every Octane cell
published `CPU/req 0us`.

`/proc/<pid>/stat` *is* world-readable even for root-owned processes — that is
why `cpu_pat` works for Laravel's containerised php-fpm, and why only
`smaps_rollup` (used by `memory.sh`) needs the cgroup fallback. The fix is a
pattern over both processes that matter: `octane:start` (the supervisor) and
`frankenphp run` (the worker host). Counting only the first would have
reproduced the 43 MB memory bug in the CPU column.

### The reference cells live in `refs.sh`

`sweep.sh` measures the matched cells only. The "what if you wrote it the other
way" figures the results page quotes in prose — FastAPI's default
`jsonable_encoder` return path, hydrated-vs-projected reads, Express on the raw
driver — are in `refs.sh`, on the same 8s-warm/30s-measured protocol. Run it
immediately after `sweep.sh` so a reference figure is comparable to the matched
row it sits beside.

### Soli's inline builder argument (fixed)

Soli's `db_json` binds the query builder to a local before rendering:

```soli
let rows = Post.pluck(:id, :title, :views).all
return render_json(rows)
```

That was a **workaround**, not a style preference: passing the builder inline —
`render_json(Post.pluck(...).all)` — used to evaluate it twice and send the
query twice, costing 42% of this benchmark's database throughput while
appearing to measure the framework. It is fixed in the interpreter now
(`render_json` evaluates its argument once), and the inline form is equally
fast. The local is kept here only so the published numbers match the code that
produced them.
