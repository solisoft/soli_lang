# Framework benchmark suite

The five applications behind [`www/docs/benchmarks.md`](../../www/docs/benchmarks.md),
plus the harness that measures them. Every app serves the same seven matched
workloads over the same data, and every one runs **16 workers**.

| Stack | Port | Server | ORM / templates |
|---|---:|---|---|
| [Soli](soli/) | 5080 | `soli serve`, 16 worker threads | Model / ERB |
| [Rails](rails/) | 5096 | Puma, 16 workers × 5 threads | ActiveRecord / ERB |
| [Express](express/) | 5097 | Node cluster, 16 workers | Sequelize / EJS |
| [Laravel](laravel/) | 5098 | php-fpm 16 workers + nginx (Docker) | Eloquent / Blade |
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

`./ws_sweep.sh` measures three things `oha` cannot, on the two stacks that serve
sockets from the same process as their HTTP rows:

| Cell | Soli | Express |
|---|---:|---:|
| echo, 1,000 conns | 232,327 msg/s (p50 4.18 ms) | **407,310 msg/s** (p50 2.08 ms) |
| room fan-out | **1,000 / publish — 100% of the room** | 63 / publish — 6% of the room |

Express wins the round trip. The room row is the one that matters
architecturally: Soli's workers are threads in one process, so a broadcast
reaches every connection, while Node's cluster gives each of its 16 workers its
own sockets — a broadcast reaches only the ~1/16th that worker accepted. Making
Express's fan-out real needs a shared bus (Redis); the number above is what the
obvious implementation actually does.

Two things this harness had to get right, both of which produced wrong answers
first:

* **Shard the client.** A single Node client saturates long before the servers:
  it reported 74k msg/s against Soli where eight client processes reported 238k,
  and throughput *fell* as connections rose while latency scaled linearly — the
  signature of a bottlenecked generator. `SHARDS` defaults to 8.
* **Rate-limit the publisher.** Pumping broadcasts flat out just outruns the
  server — a publish costs the client nothing and the server N sends — so the
  fan-out ratio collapses and measures the client's send loop. At a fixed rate
  (`PUBLISH_RATE`, default 50/s) the ratio means what it says.

Rails, Django and Laravel are not here: ActionCable needs Redis, Channels needs
ASGI and Laravel needs Reverb, each a different server process from the one
serving their HTTP rows.

## A trap worth knowing

Soli's `db_json` binds the query builder to a local before rendering:

```soli
let rows = Post.pluck(:id, :title, :views).all
return render_json(rows)
```

Passing the builder inline — `render_json(Post.pluck(...).all)` — evaluates it
twice and sends the query twice. It cost this benchmark 42% of its database
throughput and was silently measuring a bug rather than the framework.
