# Benchmarks

Three matched workloads — a JSON API response, a rendered HTML page, a database read —
through three full stacks on one machine, one load generator, one protocol. Every server
returns a **byte-identical payload** for the JSON and DB rows, and every stack runs
**16 workers**.

> **Read this first.** Benchmarks are easy to rig and easy to get wrong. Everything below
> was measured in one session on a quiet box, after a warm-up pass over all nine endpoints,
> with the HTTP status of every response verified — and where Soli loses a cell, the number
> is printed exactly as measured. The per-operation Soli-vs-Ruby language tables that used
> to live on this page were retired with it; this page compares *frameworks*, end to end.

## Setup

| | |
|---|---|
| Soli | 1.24.1, `soli serve .`, 16 HTTP workers, SoliDB (loopback HTTP) for the DB row |
| Rails | 8.1.3 + Puma 8.0.2 on Ruby 3.4.9 — production, eager-loaded, 16 workers × 5 threads, PostgreSQL via ActiveRecord |
| Express | 5.2.1 on Node 25.9, 16 cluster workers — **+ EJS 6.0 + node-postgres 8.22**: Express ships no view layer and no DB layer, both had to be added |
| Database | PostgreSQL 18.3 for Rails and Express (same table, same 50 rows); SoliDB for Soli — all client-server over a local socket, no in-process storage anywhere |
| Load | `oha` 1.12 — 30s at concurrency 200 per cell, after warming all nine endpoints |
| Machine | 16-core x86-64 Linux, load generator on the same box |

**CPU/req** is server CPU-time per request, summed across every process of the stack
(all 17 for Rails and Express). It is the most portable column here — unlike req/s it
barely moves with core count or client speed.

## JSON — 50 objects, 2,268 bytes, built in the handler

| Stack | req/s | p99 | CPU/req | vs Rails |
|---|---:|---:|---:|---:|
| Express + EJS + pg | 113,502 | 6.01 ms | 109 µs | 8.2x |
| **Soli** | 71,011 | 6.43 ms | 192 µs | **5.1x** |
| Rails + Puma | 13,870 | 27.26 ms | 904 µs | 1.0x |

Soli serialises the API response at 5.1x Rails' throughput on 4.7x less CPU. Express wins
this row outright — printed as measured. Worth knowing: in Soli, serialising these 50
objects to JSON costs ~192µs where rendering the *same data* as HTML costs 97µs — the
template engine is currently cheaper than `render_json`.

## Template — 50-row HTML table + layout, ~3 KB

| Stack | req/s | p99 | CPU/req | vs Rails |
|---|---:|---:|---:|---:|
| **Soli** | **123,570** | 3.99 ms | 99 µs | **11.2x** |
| Express + EJS + pg | 67,821 | 7.94 ms | 201 µs | 6.1x |
| Rails + Puma | 11,032 | 33.55 ms | 1,156 µs | 1.0x |

The strongest row, and the one a server-rendered framework should care about: **11.2x
Rails' throughput on 11.7x less CPU** — and 1.8x faster than Express even though Soli's
page carries its instant-navigation script (~130 extra bytes of work per request that EJS
doesn't do). Soli's ERB engine outrunning a compiled EJS template was not a given.

## Database read — 50 rows, projected columns, 2,268 bytes

| Stack | req/s | p99 | CPU/req | vs Rails |
|---|---:|---:|---:|---:|
| Express + EJS + pg | 40,684 | 17.22 ms | 221 µs | 3.7x |
| **Soli** | 21,182 | 29.55 ms | 313 µs (531 incl. SoliDB) | **1.9x** |
| Rails + Puma | 11,131 | 33.94 ms | 1,194 µs | 1.0x |

> **This row compares database access architectures as much as frameworks.** Each request
> from Soli is one blocking HTTP round trip to SoliDB per worker — 16 in flight, no more.
> Rails holds 80 threads against PostgreSQL; Express's driver is fully asynchronous,
> effectively unbounded — which is most of why it dominates here. Soli's CPU column shows
> both truths: 313µs in the Soli process, **531µs system-wide once SoliDB's own CPU is
> counted** — publishing the smaller number alone would hide an entire process. Even so,
> Soli's DB row costs 2.2x less system CPU than Rails'; the remaining gap to Express is
> the 16-in-flight cap, not the machine.

Every stack serves the same self-describing hash rows on its fastest idiom for that
shape: Soli's `Post.pluck("id", "title", "views").all()` builds the hashes **in the
database** (`RETURN {id: doc.id, ...}`); Rails' is `pluck` + a `map` — the canonical-looking
`render json: Post.select(:id, :title, :views)` measured **3.2x slower** (3,481 req/s),
because it instantiates fifty ActiveRecord models per request; node-postgres builds the
objects in the driver. Two lessons worth taking home: projection must happen in the
database (Soli fetching full ~15 KB documents and projecting client-side measured 13,251
req/s), and if you can accept positional arrays instead of hashes, everyone gets faster —
Soli measured 26,666 and Express 44,515 on the array form of the same route.

## Memory

| Stack | Processes | Idle | Under load |
|---|---:|---:|---:|
| **Soli** | 1 × 16 threads | **28 MB** | **38 MB** |
| Rails + Puma | 17 (fork + CoW) | 224 MB | 863 MB |
| Express + EJS + pg | 17 (fork + CoW) | 340 MB | 801 MB |

Figures are **PSS** (proportional set size) summed over the whole process group — the
honest measure for multi-process servers, because summing RSS counts every fork-shared
page 17 times (an idle Rails' RSS sum reads 1.4 GB against a real 224 MB). "Under load" is read at
the end of a 30s run on the DB route. The architectural cause is simple: Soli is one
process whose 16 workers are threads; Rails and Express each fork 16 processes with a heap
apiece. **At idle Soli runs in an eighth of Rails' memory; under load, in a twenty-second.**

## The code being measured

Both apps are idiomatic and the same size — the DB action in each:

```soli
# Soli
class PostsController < Controller
  def db_json
    render_json(Post.pluck("id", "title", "views").all())
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
end
```

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
engine, DB driver, pooling and template compilation are hand-assembled — the ~40 lines it
takes are the frameworkless tax the other two don't pay.

## What these multiples do and don't mean

Trivial handlers measure *fixed framework overhead*, which is precisely where Rails is
weakest — that is why the JSON and template rows show 5–11x and the DB row shows 1.9x. On
a page dominated by real query work the multiple compresses toward the DB row, not the
template row. The honest claim this page supports is: **Soli's framework overhead is
roughly a tenth of Rails' and its render path beats Express's, while database-bound routes
are architecture-limited for everyone.** Quote it that way.

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
