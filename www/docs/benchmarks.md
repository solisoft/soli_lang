# Benchmarks

Four matched workloads — a JSON API response, a rendered HTML page, a database read, and
a database-backed HTML page — through three full stacks on one machine, one load generator, one protocol. Every server
returns a **byte-identical payload** for the JSON and DB rows, and every stack runs
**16 workers**.

> **Read this first.** Benchmarks are easy to rig and easy to get wrong. Everything below
> was measured in one session on a quiet box, after a warm-up pass over every endpoint,
> with the HTTP status of every response verified — and where Soli loses a cell, the number
> is printed exactly as measured. The per-operation Soli-vs-Ruby language tables that used
> to live on this page were retired with it; this page compares *frameworks*, end to end.

## Setup

| | |
|---|---|
| Soli | 1.25.0, `soli serve .`, 16 HTTP workers, SoliDB (loopback HTTP) for the DB row |
| Rails | 8.1.3 + Puma 8.0.2 on Ruby 3.4.9 — production, eager-loaded, 16 workers × 5 threads, PostgreSQL via ActiveRecord |
| Express | 5.2.1 on Node 25.9, 16 cluster workers — **+ EJS 6.0 + Sequelize 6.37.8** (on node-postgres 8.22): Express ships no view layer and no DB layer, both had to be added. The DB rows put it on an **ORM**, not the raw driver, so all three stacks compare like for like; the driver number is kept below as a reference |
| Database | PostgreSQL 18.3 for Rails and Express (same table, same 50 rows); SoliDB for Soli — all client-server over a local socket, no in-process storage anywhere |
| Load | `oha` 1.12 — 30s at concurrency 200 per cell, after warming all nine endpoints |
| Machine | 16-core x86-64 Linux, load generator on the same box |

**CPU/req** is server CPU-time per request, summed across every process of the stack
(all 17 for Rails and Express). It is the most portable column here — unlike req/s it
barely moves with core count or client speed.

## JSON — 50 objects, 2,268 bytes, built in the handler

| Stack | req/s | p99 | CPU/req | vs Rails |
|---|---:|---:|---:|---:|
| Express + EJS + pg | 122,526 | 5.04 ms | 102 µs | 8.2x |
| **Soli** | 75,866 | 6.15 ms | 180 µs | **5.1x** |
| Rails + Puma | 14,954 | 25.77 ms | 852 µs | 1.0x |

Soli serialises the API response at 5.1x Rails' throughput on 4.7x less CPU. Express wins
this row outright — printed as measured. Worth knowing: in Soli, serialising these 50
objects to JSON costs ~180µs where rendering the *same data* as HTML costs 89µs — the
template engine is currently cheaper than `render_json`.

## Template — 50-row HTML table + layout, ~3 KB

| Stack | req/s | p99 | CPU/req | vs Rails |
|---|---:|---:|---:|---:|
| **Soli** | **137,924** | 3.72 ms | 89 µs | **11.3x** |
| Express + EJS + pg | 74,217 | 7.36 ms | 182 µs | 6.1x |
| Rails + Puma | 12,176 | 32.63 ms | 1,110 µs | 1.0x |

The strongest row, and the one a server-rendered framework should care about: **11.3x
Rails' throughput on 12.5x less CPU** — and 1.9x faster than Express even though Soli's
page carries its instant-navigation script (~130 extra bytes of work per request that EJS
doesn't do). Soli's ERB engine outrunning a compiled EJS template was not a given.

## Database read — 50 rows, projected columns, 2,268 bytes

| Stack | req/s | p99 | CPU/req | vs Rails |
|---|---:|---:|---:|---:|
| Express + EJS + Sequelize | 28,388 | 15.08 ms | 397 µs | 3.0x |
| **Soli** | 22,769 | 11.00 ms | 306 µs (522 incl. SoliDB) | **2.4x** |
| Rails + Puma | 9,594 | 38.83 ms | 1,291 µs | 1.0x |

> **This row compares database access architectures as much as frameworks.** Each request
> from Soli is one blocking HTTP round trip to SoliDB per worker — 16 in flight, no more.
> Rails holds 80 threads against PostgreSQL; Express's driver is fully asynchronous,
> effectively unbounded — which is most of why it still leads here.
>
> **All three stacks go through an ORM in this table.** An earlier revision measured
> Express on the raw `pg` driver, which is not the same workload: hand-written SQL with no
> model layer against two frameworks paying one. Putting Sequelize in the path costs
> Express **34%** — 42,818 req/s on the driver against 28,388 through the ORM — and that
> is the number published, because Soli's and Rails' rows include their ORMs too. If your
> Node app talks to the database through a driver rather than an ORM, the driver row is
> the one that describes you. Soli's CPU column shows
> both truths: 306µs in the Soli process, **522µs system-wide once SoliDB's own CPU is
> counted** — publishing the smaller number alone would hide an entire process. Even so,
> Soli's DB row costs 2.5x less system CPU than Rails'; the remaining gap to Express is
> the 16-in-flight cap, not the machine.

Every stack serves the same self-describing hash rows on its fastest idiom for that
shape: Soli's `Post.pluck(:id, :title, :views).all` builds the hashes **in the
database** (`RETURN {id: doc.id, ...}`); Rails' is `pluck` + a `map` — the canonical-looking
`render json: Post.select(:id, :title, :views)` measured **3.2x slower** (3,481 req/s),
because it instantiates fifty ActiveRecord models per request; node-postgres builds the
objects in the driver. Two lessons worth taking home: projection must happen in the
database (Soli fetching full ~15 KB documents and projecting client-side measured 13,251
req/s), and if you can accept positional arrays instead of hashes, everyone gets faster —
Soli measured 26,666 and Express 44,515 on the array form of the same route.

## Database read + HTML render — 50 rows from the database into a page, ~3 KB

The row a server-rendered framework actually lives on: query, then render. It is the
`/db` read and the `/template` render in one request, so the response is the same page as
the Template row above, byte-for-byte the same size, and the database is the only added
variable.

| Stack | req/s | p99 | CPU/req | vs Rails |
|---|---:|---:|---:|---:|
| **Soli** | **40,034** | 6.49 ms | 182 µs (301 incl. SoliDB) | **4.9x** |
| Express + EJS + Sequelize | 23,580 | 16.26 ms | 503 µs | 2.9x |
| Rails + Puma | 8,155 | 41.95 ms | 1,509 µs | 1.0x |

Soli takes this row — **4.9x Rails' throughput and 1.7x Express's** — and it is the row
whose shape matters most, because it is the only one where every stack does the two things
a page does, each through its own ORM. Express's asynchronous driver still wins the raw DB
read above; once the same rows have to go through an ORM *and* become HTML, that lead is
gone. On the raw driver Express reaches 32,754 here, still short of Soli.

The result worth pausing on is Soli's own: **the database-backed HTML page is nearly
twice the throughput of the database-backed JSON response** (40,034 vs 22,769) on the same
query and the same 50 rows. Nothing about the database changed — `render_json` is simply
more expensive than the template engine, the same asymmetry the JSON row notes. If you are
building pages rather than an API, the cheaper path is the one you were going to write
anyway.

## Memory

| Stack | Processes | Idle | Under load |
|---|---:|---:|---:|
| **Soli** | 1 × 16 threads | **48 MB** | **72 MB** |
| Rails + Puma | 17 (fork + CoW) | 195 MB | 918 MB |
| Express + EJS + pg | 17 (fork + CoW) | 342 MB | 908 MB |

Figures are **PSS** (proportional set size) summed over the whole process group — the
honest measure for multi-process servers, because summing RSS counts every fork-shared
page 17 times (an idle Rails' RSS sum reads 1.9 GB against a real 195 MB). "Under load" is read at
the end of a 30s run on the DB + HTML route. The architectural cause is simple: Soli is one
process whose 16 workers are threads; Rails and Express each fork 16 processes with a heap
apiece. **At idle Soli runs in a quarter of Rails' memory; under load, in a thirteenth.** Soli's
own figures grew this release (28 → 48 MB idle): each worker now drives DB I/O on its own
reactor with its own connection pool, which is what bought the throughput above.

## The code being measured

Both apps are idiomatic and the same size — the DB action in each:

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

## What these multiples do and don't mean

Trivial handlers measure *fixed framework overhead*, which is precisely where Rails is
weakest — that is why the JSON and template rows show 5–11x and the DB row shows 2.4x. On
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
