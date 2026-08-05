# Phoenix benchmark app

The Phoenix column of `www/docs/benchmarks.md`, serving the same matched
workloads as the Soli, Rails, Express, AdonisJS, Laravel, Django and FastAPI
apps. Generated with `mix phx.new --no-assets --no-live --no-gettext
--no-mailer --no-dashboard`, then given the bench controller.

## Why this column exists

Until it was added, the docs site asserted that **"Phoenix is the only peer
here"** on the How Soli Compares page with no measurement behind it. That was
the only competitive claim on the site that had never been run.

It is also the only stack besides Soli that is **not** 16 OS processes. The BEAM
is one process with one scheduler thread per core and a lightweight process per
connection; Rails, Express, AdonisJS, Django, FastAPI and php-fpm all fork 16
processes with a heap apiece. So this is the column that tests whether Soli's
memory and fan-out advantages are *architectural* or merely
unique-among-forking-runtimes.

## Shape

* **Bandit** (Phoenix 1.8's default server) on port **5104**, `MIX_ENV=prod`.
* **`+S 16:16`** — scheduler count pinned to 16, the same core budget the other
  stacks get from 16 workers. Pinned rather than inherited so the match is
  explicit.
* **Ecto + PostgreSQL**, `POOL_SIZE=80` — matching Puma's 16×5 and the 5-per-worker
  pools of Express and FastAPI. Ecto holds one pool for the whole VM, so this is
  set once rather than per worker.
* **HEEx templates**, Phoenix's default engine.
* **Phoenix's default `:browser` pipeline** on the HTML routes — session, CSRF,
  secure headers, untouched. That is the analogue of Django running its full
  default middleware chain and Rails running ActionController. The write routes
  use `:api` for the same reason Django's are `@csrf_exempt`.

The `posts` (50 rows) and `wposts` (800,000 rows) tables are created and seeded
by the shared harness, so there are **no migrations here** — `lib/benchweb/post.ex`
declares a mapping over a table that already exists, the analogue of Django's
`managed = False`.

## Two things that had to be fixed, both silent

**`force_ssl` is removed from `config/prod.exs`.** Phoenix generates it with a
localhost exclude list, but any mismatch turns every request into a **301** — and
`oha` reports a wall of 301s as 100% "success". The results page warns about
exactly this failure mode; it would have been self-inflicted here.

**Both layouts have to be disabled, not one.** `render(conn, :list, layout: false)`
only turns off the inner app layout. The router's `put_root_layout` is separate,
and leaving it on wrapped this document inside Phoenix's root layout — a second
`<!DOCTYPE html>` nested inside `<body>`, at 3,492 bytes against Django's 2,864.
It renders in a browser without complaint, so only the byte count caught it. The
controller now calls both `put_root_layout(html: false)` and
`put_layout(html: false)`.

## Payload parity

`/json` and `/db` are **byte-identical** to every other stack (2,268 bytes,
verified by SHA-256 against Django's). That was not automatic: Elixir maps have no
insertion order, so key order comes from Erlang term order over the keys — and
`:id < :title < :views` happens to be the order the other stacks emit. If the
fields were renamed, the payload would silently reorder and stop matching.

The HTML rows render 2,863 bytes against Django's 2,864: HEEx strips a template's
trailing newline. One byte, inside the per-stack template variance the results
page already documents (Soli 3,030 with its instant-nav script, Rails 2,865,
Express 2,916, AdonisJS 2,863).

## Running

```bash
MIX_ENV=prod mix compile      # once
./start-bench.sh              # 16 schedulers on 5104, pool of 80
PORT=5204 SCHEDULERS=4 ./start-bench.sh
```

`start.sh` in the parent directory calls this script, so the flags have one home.

## Endpoints

| Route | Workload |
|---|---|
| `GET /json` | 50 in-memory maps as JSON |
| `GET /template` | the same 50 rows through HEEx |
| `GET /db` | 50 rows projected in the database, as JSON |
| `GET /db-template` | the same read, rendered as HTML |
| `GET /db-hydrated` | reference: the form that materialises 50 Ecto structs |
| `POST/PATCH/DELETE /w` | one create / update / delete per request |

## Not measured yet

**Phoenix Channels.** This is the strongest remaining gap in the suite: Channels
run in the same BEAM as the HTTP rows and Phoenix PubSub needs no Redis, which is
the same claim Soli's fan-out row makes. Measuring it needs a `phoenix` protocol
mode in `ws_bench.js` (the `actioncable` mode is the precedent — Channels also
speak a JSON envelope with a join handshake). Until then the WebSocket section
has Soli as the sole example of its own thesis.
