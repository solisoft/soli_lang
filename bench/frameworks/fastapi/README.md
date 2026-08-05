# FastAPI benchmark app

The FastAPI column of `www/docs/benchmarks.md`, serving the same matched
workloads as the Soli, Rails, Express, AdonisJS, Laravel and Django apps.

## Shape

* **uvicorn with 16 workers** on port **5103**, matching every other stack. One
  single-threaded event loop per worker.
* **uvloop + httptools**, picked up automatically because they are installed —
  they ship in the `uvicorn[standard]` extra, which is uvicorn's documented
  production install. No flags, nothing tuned.
* **SQLAlchemy 2.0 async + asyncpg**, the canonical FastAPI data layer. FastAPI
  ships no ORM of its own, the same way Express ships no view layer and no DB
  layer.
* **Jinja2 templates** via `Jinja2Templates`, FastAPI's documented template
  option, with autoescape on.
* **Pool of 5 per worker** (`max_overflow=0`) — 80 connections, matching Puma's
  16×5, Express's `max: 5` and Django's `CONN_MAX_AGE`. The hard cap keeps the
  column bounded like the others rather than letting the event loop open a
  socket per in-flight request.
* **`async def` throughout.** A sync handler would run in Starlette's
  threadpool, which is a different concurrency model from the one FastAPI is
  chosen for.

The `posts` (50 rows) and `wposts` (800,000 rows) tables are created and seeded
by the shared harness, so nothing here creates schema — `benchapp/models.py`
declares mappings over tables that already exist, the analogue of Django's
`managed = False`.

## Two choices worth stating

**The published rows return a `Response` directly.** FastAPI's headline feature
is that `return rows` runs the value through `jsonable_encoder` — and through a
`response_model` when one is declared — before serialising it. That is framework
work no other stack in this comparison does, so the matched rows use
`JSONResponse`, which is a documented FastAPI idiom ("Return a Response
Directly"). `/json-encoded` and `/db-encoded` serve the default path and are
published as labelled reference rows, because the gap between the two is a fact
about FastAPI worth knowing rather than one to hide.

**The DB rows project without instantiating models** — `select(Post.id,
Post.title, Post.views)` through an `AsyncSession`, the SQLAlchemy analogue of
Rails' `pluck`, Soli's `pluck`, Sequelize's `raw: true`, Eloquent's `toBase()`
and Django's `.values()`. `/db-hydrated` is the reference form that does build
50 mapped objects.

One non-default in the template config: `keep_trailing_newline=True`. Jinja2
strips a single trailing newline from a template, which off the same template
file would make this page one byte shorter than Django's. With it the two are
byte-identical.

## Running

```bash
pip install --user fastapi 'uvicorn[standard]' 'sqlalchemy[asyncio]' asyncpg jinja2
./start-bench.sh                 # 16 workers on 5103, pool of 5 — the published config
WORKERS=4 PORT=5203 ./start-bench.sh
POOL_SIZE=20 ./start-bench.sh    # diagnostic only, see below
```

`start.sh` in the parent directory calls this script, so the flags have one home.

`POOL_SIZE` exists to answer the question this stack's p99 provokes, not to tune it. The
published rows use the default 5 per worker, matched to every other stack. At 20 the
measured effect was **+8–10% throughput and a 26–49% lower p99** — so the matched pool
bounds the tail but not the throughput, and most of the tail is coroutine and GIL
contention rather than connection scarcity. Restore the default before measuring anything
that goes on the results page.

## Endpoints

| Route | Workload |
|---|---|
| `GET /json` | 50 in-memory objects as JSON |
| `GET /json-encoded` | reference: the same rows through FastAPI's default return path |
| `GET /template` | the same 50 rows through a Jinja2 template |
| `GET /db` | 50 rows projected in the database, as JSON |
| `GET /db-encoded` | reference: the same read through the default return path |
| `GET /db-template` | the same read, rendered as HTML |
| `GET /db-hydrated` | reference: the form that instantiates 50 mapped objects |
| `POST/PATCH/DELETE /w` | one create / update / delete per request |

## A trap this app set, and where it is handled

uvicorn's `--workers` supervisor spawns its workers through
`multiprocessing.spawn`, so a worker's cmdline reads `python3 -c from
multiprocessing.spawn import spawn_main; ...` — **the app name does not appear
in it**. Measuring this stack by cmdline pattern the way `sweep.sh` measures
Django (`gunicorn.*benchproj`) matches the supervisor alone: 8 CPU ticks where
the real process group had 88, which would have published a CPU/req roughly ten
times too good. The workers do share the supervisor's process group, so
`sweep.sh` and `memory.sh` both measure this stack by **pgid**, and both carry a
comment saying why the obvious pattern branch is missing.
