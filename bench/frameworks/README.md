# Framework benchmark suite

The five applications behind [`www/docs/benchmarks.md`](../../www/docs/benchmarks.md),
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
./start.sh         # bring up all five stacks, wait for health
./sweep.sh         # all 35 cells, back to back
./memory.sh        # PSS per stack, idle and under load
```

`sweep.sh` warms each cell for 8s at c=100, then measures 30s at c=200, and
reports req/s, p99 and CPU-time per request summed over **every process** of the
stack. Write cells reset their dataset first and count rows before and after, so
the delete row's hit rate is measured rather than assumed.

AdonisJS builds to JavaScript first (`node ace build`) and runs from `build/`
via `adonis/start-bench.sh`; it needs `pg`, `luxon` and `@types/pg` installed.

Requires `oha`, `psql`, a PostgreSQL on `127.0.0.1:5433` (user/db `bench`) and a
SoliDB on `6745`. Override with `PGURL` and `SDB`.

## Rules the comparison depends on

These are the things that decide the result before a single request is sent, so
they are worth stating plainly.

* **Same payload.** `/json` and `/db` return **byte-identical** responses from
  all five stacks (2,268 bytes). Template output differs by a few bytes per
  stack — Soli's page carries its instant-navigation script, Rails emits one
  extra newline — and those differences are noted on the page rather than
  hidden.
* **Every stack goes through its ORM**, and each uses the form that projects
  *without* instantiating models: Rails `pluck`, Soli `pluck`, Sequelize
  `raw: true`, Eloquent `toBase()`, Django `.values()`. Measuring one stack on a
  raw driver against four ORMs flattered it by 34%, which is why Express's
  driver figure is now published as a labelled reference instead.
* **Every stack pools its database connections.** Without it, php-fpm and
  gunicorn open a fresh connection per request — worth ~8ms on loopback, and
  enough to cost Laravel two thirds of its database throughput. That is
  measuring connection setup, not the framework.
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
rows.

## A trap worth knowing (fixed)

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
