# Benchmarks

Seven HTTP workloads, plus WebSockets — a JSON API response, a rendered HTML page, a database read, a
database-backed HTML page, and one create, update and delete per request — through six
full stacks on one machine, one load generator, one protocol. Every server returns a
**byte-identical payload** for the JSON and DB rows, and every stack runs **16 workers**.

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
| Laravel | 13.8 on PHP 8.4 (php-fpm, `pm = static`, 16 workers) + nginx, in Docker with host networking — Eloquent + Blade, OPcache, config/route/view cached, persistent PDO connections |
| Laravel + Octane | The same application on Octane 2.18 / FrankenPHP, 16 workers, app resident between requests. Published as a **labelled reference row**, not as "Laravel", because it roughly doubles every result and is a deployment choice rather than the default |
| Django | 6.0.7 on Python 3.14, gunicorn with 16 workers — Django ORM + Django templates, `DEBUG=False`, persistent connections (`CONN_MAX_AGE`) |
| AdonisJS | 6.18 on Node 25.9, 16 cluster workers — Lucid ORM + Edge templates, built to JavaScript and run from `build/`, `NODE_ENV=production` |
| Express | 5.2.1 on Node 25.9, 16 cluster workers — **+ EJS 6.0 + Sequelize 6.37.8** (on node-postgres 8.22): Express ships no view layer and no DB layer, both had to be added. The DB rows put it on an **ORM**, not the raw driver, so all three stacks compare like for like; the driver number is kept below as a reference |
| Database | PostgreSQL 18.3 for Rails, Express, AdonisJS, Laravel and Django (same table, same 50 rows); SoliDB for Soli — all client-server over a local socket, no in-process storage anywhere |
| Load | `oha` 1.12 — 30s at concurrency 200 per cell, after an 8s warm-up of that cell |
| Machine | 16-core x86-64 Linux, load generator on the same box |

**CPU/req** is server CPU-time per request, summed across every process of the stack
(all 17 for Rails, Express, AdonisJS and Django; php-fpm plus nginx for Laravel). It is the most
portable column here — unlike req/s it
barely moves with core count or client speed.

## JSON — 50 objects, 2,268 bytes, built in the handler

| Stack | req/s | p99 | CPU/req | vs Rails |
|---|---:|---:|---:|---:|
| Express + EJS + Sequelize | 110,182 | 6.63 ms | 110 µs | 8.1x |
| **Soli** | 70,398 | 6.44 ms | 191 µs | 5.2x |
| AdonisJS + Lucid + Edge | 22,461 | 17.96 ms | 619 µs | 1.7x |
| Django + gunicorn | 17,952 | 15.53 ms | 677 µs | 1.3x |
| Rails + Puma | 13,535 | 28.58 ms | 941 µs | 1.0x |
| Laravel + php-fpm | 5,108 | 46.18 ms | 2,838 µs | 0.4x |
| Laravel + Octane *(reference)* | 10,854 | 26.24 ms | 1,349 µs | 0.8x |

Soli serialises the API response at 5.2x Rails' throughput on 4.9x less CPU. Express wins
this row outright — printed as measured. Worth knowing: in Soli, serialising these 50
objects to JSON costs ~191µs where rendering the *same data* as HTML costs 98µs — the
template engine is currently cheaper than `render_json`.

## Template — 50-row HTML table + layout, ~3 KB

| Stack | req/s | p99 | CPU/req | vs Rails |
|---|---:|---:|---:|---:|
| **Soli** | 124,314 | 4.27 ms | 98 µs | 11.6x |
| Express + EJS + Sequelize | 65,762 | 8.32 ms | 203 µs | 6.1x |
| AdonisJS + Lucid + Edge | 22,309 | 16.75 ms | 576 µs | 2.1x |
| Rails + Puma | 10,724 | 35.91 ms | 1,205 µs | 1.0x |
| Django + gunicorn | 7,086 | 34.55 ms | 2,011 µs | 0.7x |
| Laravel + php-fpm | 4,801 | 49.38 ms | 3,062 µs | 0.4x |
| Laravel + Octane *(reference)* | 9,275 | 28.56 ms | 1,597 µs | 0.9x |

The strongest row, and the one a server-rendered framework should care about: **11.6x
Rails' throughput on 12.3x less CPU** — and 1.9x faster than Express even though Soli's
page carries its instant-navigation script (~130 extra bytes of work per request that EJS
doesn't do). Soli's ERB engine outrunning a compiled EJS template was not a given.

## Database read — 50 rows, projected columns, 2,268 bytes

| Stack | req/s | p99 | CPU/req | vs Rails |
|---|---:|---:|---:|---:|
| **Soli** | 35,472 | 6.93 ms | 221 µs (339 incl. SoliDB) | 3.9x |
| Express + EJS + Sequelize | 26,689 | 15.67 ms | 425 µs | 2.9x |
| AdonisJS + Lucid + Edge | 13,958 | 26.69 ms | 928 µs | 1.5x |
| Django + gunicorn | 9,842 | 25.12 ms | 1,236 µs | 1.1x |
| Rails + Puma | 9,122 | 38.00 ms | 1,351 µs | 1.0x |
| Laravel + php-fpm | 3,773 | 61.23 ms | 3,569 µs | 0.4x |
| Laravel + Octane *(reference)* | 8,256 | 30.02 ms | 1,639 µs | 0.9x |

> **This row compares database access architectures as much as frameworks.** Each request
> from Soli is one blocking HTTP round trip to SoliDB per worker — 16 in flight, no more.
> Rails holds 80 threads against PostgreSQL; Express's driver is fully asynchronous,
> effectively unbounded — which is most of why it still leads here.
>
> **Every stack goes through an ORM in this table.** An earlier revision measured
> Express on the raw `pg` driver, which is not the same workload: hand-written SQL with no
> model layer against two frameworks paying one. Putting Sequelize in the path costs
> Express **34%** — 42,818 req/s on the driver against 28,388 through the ORM — and that
> is the number published, because Soli's and Rails' rows include their ORMs too. If your
> Node app talks to the database through a driver rather than an ORM, the driver row is
> the one that describes you. Soli's CPU column shows
> both truths: 221µs in the Soli process, **339µs system-wide once SoliDB's own CPU is
> counted** — publishing the smaller number alone would hide an entire process. Even
> counted that way Soli's DB row costs **4.0x less system CPU than Rails'**, and the
> 16-in-flight cap is not the ceiling it looked like: it is enough to lead this row.

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
| **Soli** | 38,616 | 6.53 ms | 187 µs (304 incl. SoliDB) | 4.9x |
| Express + EJS + Sequelize | 23,008 | 16.12 ms | 513 µs | 2.9x |
| AdonisJS + Lucid + Edge | 13,667 | 26.65 ms | 955 µs | 1.7x |
| Rails + Puma | 7,840 | 43.44 ms | 1,577 µs | 1.0x |
| Django + gunicorn | 5,438 | 44.85 ms | 2,545 µs | 0.7x |
| Laravel + php-fpm | 3,548 | 63.55 ms | 3,822 µs | 0.5x |
| Laravel + Octane *(reference)* | 7,328 | 33.75 ms | 1,868 µs | 0.9x |

Soli takes this row — **4.9x Rails' throughput and 1.7x Express's** — and it is the row
whose shape matters most, because it is the only one where every stack does the two things
a page does, each through its own ORM. Express on the raw driver reaches 32,416 here,
still short of Soli with an ORM in the way.

The result worth pausing on is Soli's own: this row and the JSON row above are within 9%
of each other (38,616 and 35,472) on the same query and the same 50 rows. Once a database
round trip is in the request, it dominates — whether the result leaves as JSON or as a
rendered page is close to noise. The large render-path gap the JSON and Template rows show
on in-memory data does not survive contact with a real query, which is worth knowing before
optimising a template for a page that is really waiting on the database.

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

**Octane roughly doubles Laravel on every row** (1.8x to 2.1x), which is the whole reason
it is published separately. On php-fpm the framework is rebuilt per request; on Octane it
stays in memory, and that single change moves Laravel from last place into the same band as
Rails and Django — it beats Django on the template, database-backed page and delete rows.
Read the php-fpm rows as the default deployment and the Octane rows as what the same code
does when you opt into the resident runtime.

### Create — one INSERT per request

| Stack | req/s | p99 | CPU/req | vs Rails |
|---|---:|---:|---:|---:|
| **Soli** | 32,607 | 8.99 ms | 132 µs (344 incl. SoliDB) | 3.9x |
| Express + EJS + Sequelize | 22,816 | 19.67 ms | 487 µs | 2.7x |
| AdonisJS + Lucid + Edge | 14,372 | 26.95 ms | 881 µs | 1.7x |
| Django + gunicorn | 10,949 | 22.83 ms | 1,077 µs | 1.3x |
| Rails + Puma | 8,320 | 46.55 ms | 1,494 µs | 1.0x |
| Laravel + php-fpm | 3,726 | 61.47 ms | 3,659 µs | 0.4x |
| Laravel + Octane *(reference)* | 8,383 | 30.05 ms | 1,599 µs | 1.0x |

### Update — one row by primary key

| Stack | req/s | p99 | CPU/req | vs Rails |
|---|---:|---:|---:|---:|
| **Soli** | 31,693 | 8.73 ms | 131 µs (359 incl. SoliDB) | 3.0x |
| Express + EJS + Sequelize | 21,856 | 18.50 ms | 503 µs | 2.1x |
| AdonisJS + Lucid + Edge | 14,940 | 26.74 ms | 817 µs | 1.4x |
| Rails + Puma | 10,438 | 42.26 ms | 1,173 µs | 1.0x |
| Django + gunicorn | 10,112 | 24.93 ms | 1,168 µs | 1.0x |
| Laravel + php-fpm | 3,811 | 60.21 ms | 3,570 µs | 0.4x |
| Laravel + Octane *(reference)* | 8,566 | 28.57 ms | 1,545 µs | 0.8x |

### Delete — one row by primary key

| Stack | req/s | p99 | CPU/req | rows removed | vs Rails |
|---|---:|---:|---:|---:|---:|
| Express + EJS + Sequelize | 30,910 | 16.40 ms | 324 µs | 2.9x | 59% of requests |
| **Soli** | 29,588 | 8.99 ms | 150 µs (399 incl. SoliDB) | 2.7x | 60% of requests |
| AdonisJS + Lucid + Edge | 14,994 | 27.11 ms | 832 µs | 1.4x | 77% of requests |
| Rails + Puma | 10,837 | 35.40 ms | 1,151 µs | 1.0x | 82% of requests |
| Django + gunicorn | 8,843 | 28.81 ms | 1,316 µs | 0.8x | 85% of requests |
| Laravel + php-fpm | 3,869 | 59.10 ms | 3,534 µs | 0.4x | 93% of requests |
| Laravel + Octane *(reference)* | 8,725 | 28.07 ms | 1,525 µs | 0.8x | 88% of requests |

> **Read the delete row with its caveat.** Delete is the one operation that consumes its
> own workload: a key already deleted is a miss, and a miss is cheaper than a delete. The
> "rows removed" column is measured, not assumed — the table is counted before and after
> each cell — and it shows the effect plainly: the faster a stack runs, the more of the
> 800,000-row pool it exhausts and the higher its miss rate climbs. Rails, at a third of
> the throughput, does the largest share of real deletes. So this row understates the gap
> to Rails and should be read as a rough ordering, not a precise multiple. The create and
> update rows have no such problem: every create inserts, and every update targets a key
> that still exists.

Two things stand out across all three. Soli's own CPU per write is **8 to 10x lower than
Rails'** and roughly a third of Express's, but the system-wide figure — the number in
parentheses, which counts SoliDB's process too — is where the honest comparison sits, and
even there Soli leads. And every stack writes far more slowly than it reads: Soli's create
row is 32,607 against 124,314 on the in-memory template row, because a write has to reach
another process and a log, whichever framework issued it.

## WebSockets — echo and fan-out

`oha` speaks HTTP only, so these use a purpose-built client. Three stacks appear
here: Soli, Express and Rails. Rails' ActionCable mounts inside the same Puma
process serving the HTTP rows, on the **redis** adapter — the `async` adapter
keeps its pubsub in-process, so with 16 workers a broadcast would reach only the
worker that received it. It also speaks a JSON subprotocol rather than raw
WebSocket, so the client performs the full welcome → subscribe → confirm
handshake before any message is timed. Django (Channels on ASGI) and Laravel
(Reverb) would each need a *different server process* from the one serving their
HTTP rows, so they are absent rather than misrepresented.

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
| **Soli** | 1 x 16 threads | 50 MB | 70 MB |
| Laravel + php-fpm | 17 (fpm + nginx) | 84 MB | 84 MB |
| Laravel + Octane *(reference)* | 16 resident workers | 43 MB | 43 MB |
| Rails + Puma | 17 (fork + CoW) | 255 MB | 924 MB |
| Express + EJS + Sequelize | 17 (fork + CoW) | 442 MB | 1,055 MB |
| Django + gunicorn | 17 (fork + CoW) | 648 MB | 921 MB |
| AdonisJS + Lucid + Edge | 17 (fork + CoW) | 2,815 MB | 3,068 MB |

Figures are **PSS** (proportional set size) summed over the whole process group — the
honest measure for multi-process servers, because summing RSS counts every fork-shared
page 17 times. "Under load" is read at the end of a 30s run on the DB + HTML route. The
architectural cause is simple: Soli is one process whose 16 workers are threads; the others
fork 16 processes with a heap apiece.

**At idle Soli runs in a fifth of Rails' memory and a thirteenth of Django's.** Two results
cut against the throughput order and are worth reading together. Laravel is nearly as lean
as Soli (84 MB, flat under load) *because* it is slowest — php-fpm keeps no application
resident, so there is nothing to hold; and Octane, which keeps the app resident and doubles
Laravel's throughput, uses **less** still at 43 MB, because 16 warm workers cost less than
rebuilding the framework on every request. AdonisJS is the other end: **2,815 MB idle**,
roughly 197 MB per worker against Express's 65 MB, which is what a full TypeScript
framework, ORM and template engine cost when each of 16 workers carries its own copy.

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

## What these multiples do and don't mean

Trivial handlers measure *fixed framework overhead*, which is precisely where Rails is
weakest — that is why the JSON and template rows show 5–11x and the DB rows show 4–5x. On
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
