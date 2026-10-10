# Cloudflare Workers (edge build)

A Soli app can run inside a [Cloudflare Worker](https://developers.cloudflare.com/workers/):
the Soli interpreter is compiled to WebAssembly, your app's source ships inside
the Worker, and each request is answered by the same routing, controllers,
views and models that `soli serve` runs — next to your visitors, with no server
to keep up.

<https://cf.solisoft.net> is one: it is `examples/cloudflare-worker` in the
Soli repository, and it explains this page in pages of its own.

```bash
./scripts/build-edge.sh                      # once per Soli version → target/edge
soli edge build . --runtime target/edge      # your app → dist/edge
cd dist/edge && npx wrangler dev             # on workerd, locally
npx wrangler deploy                          # on Cloudflare
```

## How it works

- **The edge runtime** is the Soli library built for `wasm32-unknown-unknown`
  without its server, CLI and native-only builtins: about 9 MB of
  WebAssembly, 2.9 MB compressed. `scripts/build-edge.sh` builds it from the
  `edge/` crate.
- **`soli edge build`** writes a plain Wrangler project: the runtime, a small
  JavaScript host (`src/worker.js`), your app's source as `src/app.json`, and
  `public/` as the Worker's static assets.
- **At the first request** an isolate instantiates the runtime, mounts the
  app's files in memory (a Worker has no filesystem), and boots the app the way
  a `soli serve` worker does. That takes a few tens of milliseconds and happens
  once per isolate; later requests take a few milliseconds.
- **Every request** goes through `handle_request`, the function `soli serve`'s
  workers call: middleware, CSRF, routing, the action on the VM, the view and
  its layout.
- **Model queries** go to Cloudflare D1 through the Worker's binding, to
  PostgreSQL through a JavaScript driver (usually via Hyperdrive), or
  to SoliDB over HTTP(S) with `fetch`. The interpreter is synchronous and they
  are asynchronous, so the wasm stack *suspends* on each query —
  [JavaScript Promise Integration](https://v8.dev/blog/jspi)
  (`WebAssembly.Suspending` / `WebAssembly.promising`) — and resumes when the
  answer arrives. Your code does not change.

## Build the edge runtime

Once per Soli version, in the Soli source tree:

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.126 --locked
./scripts/build-edge.sh
```

The wasm-bindgen CLI must match the version `edge/Cargo.toml` pins; the script
checks and prints the install line when it does not. Nothing in the wasm build
compiles C, so no C toolchain is needed. The output, in `target/edge` (or
`$OUT`):

| File | What it is |
|------|------------|
| `soli_edge_bg.wasm` | the interpreter (opt-level `z`, fat LTO) |
| `soli_edge.js` | wasm-bindgen glue |
| `worker.js` | the Worker: boots the app, queues requests into wasm |
| `jspi.js` | the one import the stack suspends on |

## Package an app: `soli edge build`

```bash
soli edge build [folder] [--out DIR] [--runtime DIR] [--sql postgres|none]
```

| Option | Default | |
|--------|---------|---|
| `folder` | `.` | the app (it must have `app/controllers`) |
| `--out`, `-o` | `<folder>/dist/edge` | the Wrangler project to write |
| `--runtime` | `$SOLI_EDGE_RUNTIME`, then `edge/` or `../share/soli/edge` next to the `soli` binary | where `build-edge.sh` put the runtime |
| `--sql` | detected | `postgres` to bundle its driver, `none` for nothing (see [PostgreSQL](#postgresql-hyperdrive)) |

What goes where:

- `app/`, `config/` and `lib/` → `src/app.json`, mounted at `/app` in the
  Worker. Only UTF-8 text files; a binary file is skipped with a line saying so
  (binary files belong in `public/`).
- `public/` → `public/`, served as [static assets](https://developers.cloudflare.com/workers/static-assets/)
  before the Worker runs. The copy is fresh each build.
- `.env` and anything outside those directories is **not** bundled.
- `wrangler.toml` is written on the first build only. Later builds keep it, so
  your name, routes, vars and account stay.
- For an app on PostgreSQL, the driver: `src/sql-pg.js`, `pg` in
  `package.json`, and `npm install`.

Add `dist/` to the app's `.gitignore`.

## Configure

Everything an app reads with `getenv()` — the SoliDB settings, your own keys —
comes from the Worker's bindings: plain values under `[vars]` in
`wrangler.toml`, secrets with Wrangler.

```toml
[vars]
SOLIDB_HOST = "https://db.example.com"
SOLIDB_DATABASE = "my_app"
SOLIDB_USERNAME = "app"
```

```bash
npx wrangler secret put SOLIDB_PASSWORD
```

The runtime also sets, unless the Worker does:

| Variable | Value |
|----------|-------|
| `SOLI_RUNTIME` | `cloudflare-workers` |
| `SOLI_VERSION` | the Soli version the runtime was built from |

Request headers carry what Cloudflare knows about the visitor:
`cf-connecting-ip` (also the request's peer IP), `cf-ipcountry`, and
`cf-colo` — the datacenter that ran the request, which `worker.js` adds from
`request.cf.colo`.

## Deploy

```bash
npx wrangler deploy
```

For your own domain, once its zone is on the same Cloudflare account, add to
`wrangler.toml` — above the `[assets]` table, or TOML files it under it:

```toml
routes = [{ pattern = "app.example.com", custom_domain = true }]
```

A Wrangler API token needs *Workers Scripts: Edit* on the account, and
*Workers Routes: Edit* on the zone for a custom domain. Wrangler finds the
account from the token; with access to several, set `account_id` in
`wrangler.toml`.

## Databases

Models work as they do under `soli serve` — `find`, `where`, `create`,
`update`, `delete`, `count`, aggregates, `update_all` / `delete_all`,
validations, callbacks — on D1, PostgreSQL or SoliDB.

### Cloudflare D1

[D1](https://developers.cloudflare.com/d1/) is SQLite run by Cloudflare, next
to the Worker: an app with D1 needs no database server of its own.

```bash
npx wrangler d1 create my-app-db        # prints the database_id
```

```toml
[[d1_databases]]
binding = "DB"
database_name = "my-app-db"
database_id = "…"

[vars]
SOLI_DB_ADAPTER = "d1"
```

`SOLI_DB_ADAPTER = "d1"` puts models on the D1 adapter, and `DATABASE_URL`
names the binding: `d1://DB`, which is the default. Under `wrangler dev` the
database is local (`.wrangler/state`); `npx wrangler d1 execute my-app-db
--local --command "SELECT …"` reads it. In `config/database.toml`, a D1
connection is `adapter = "d1"`, `url = "d1://BINDING"`.

It is the SQLite adapter's document model — a table per model with a `_key`
and a JSON `doc` column, created on first write — and the SQLite adapter's
SQL, sent to the binding with `prepare(…).bind(…).all()`. Each statement is a
round trip, so writes use `RETURNING doc` rather than reading the row back.
What D1 does not offer:

- **Transactions.** D1 runs batches, not interactive transactions:
  `Model.transaction` raises.
- **Column-aware models** (a model that declares its columns) raise: store
  them as documents.
- **The job table and cron**: on the edge, jobs go through
  [Cloudflare Queues](#background-jobs-cloudflare-queues) instead.

### SoliDB

Over SoliDB's HTTP API. The Worker must be able to reach `SOLIDB_HOST`: a
public `https://` address. Credentials follow the usual order (JWT login with
`SOLIDB_USERNAME` / `SOLIDB_PASSWORD`, `SOLIDB_API_KEY`, then basic auth).

### PostgreSQL (Hyperdrive)

A Worker has no socket a Rust client could open, so the edge build does not
carry the native `postgres` client. It sends the SQL that adapter compiles to a
JavaScript driver in the Worker, `pg`, which connects through the Worker's TCP
sockets. [Hyperdrive](https://developers.cloudflare.com/hyperdrive/) keeps a
pool of connections next to the database for it, so a request does not pay a
TLS handshake to the database.

```bash
npx wrangler hyperdrive create my-db --connection-string="postgres://user:password@db.example.com:5432/my_app"
```

```toml
compatibility_flags = ["nodejs_compat"]   # top level, above the first table

[[hyperdrive]]
binding = "HYPERDRIVE"
id = "…"                                  # what `hyperdrive create` printed
localConnectionString = "postgres://user:password@localhost:5432/my_app"

[vars]
SOLI_DB_ADAPTER = "postgres"
```

`DATABASE_URL` defaults to `hyperdrive://HYPERDRIVE`; `hyperdrive://NAME`
names another binding, and a plain `postgres://` URL is connected to directly,
without Hyperdrive. `wrangler dev` uses `localConnectionString` (it must carry
a password).

`soli edge build` bundles the driver when the app's configuration names
Postgres — `adapter = "postgres"` in `config/database.toml`, `SOLI_DB_ADAPTER`
in `.env` or under `[vars]` — or when told with `--sql postgres` (`--sql none`
bundles nothing). It copies the driver's glue, adds `pg` to `package.json`,
runs `npm install`, and puts `nodejs_compat` in a new `wrangler.toml`; an
existing one without it earns a warning.

The models are the native adapter's: the same document tables and SQL,
column-aware models (`table "orders"`) included, and **transactions**, since
every statement of a request runs on one connection — opened at the request's
first query, closed (handed back to Hyperdrive's pool) when it ends. What stays
with `soli serve`: cron, `db:create` / `db:drop` and schema dumps (jobs run
through [Cloudflare Queues](#background-jobs-cloudflare-queues)).

Not available on the edge: **MySQL** — its JavaScript driver alone weighs
410 KB compressed, more than the room left under the free plan's 3 MB —
SQLite and the native SoliDB driver. The edge build uses D1, PostgreSQL or
SoliDB over HTTP.

## Files: uploads, images and video

Files in `public/` are Cloudflare static assets: images and video served
before the Worker runs, `Range` included.

Uploads work as under `soli serve` — a multipart form reaches the action, and
`find_uploaded_file` / `attach_<field>` take the file — and they are kept in
[R2](https://developers.cloudflare.com/r2/), Cloudflare's object storage, with
`service: "r2"`:

```soli
class Photo < Model
  has_one_attached("image", {
    "service": "r2",
    "content_types": ["image/jpeg", "image/png", "video/mp4"],
    "max_size": 20_000_000
  })
end
```

```bash
npx wrangler r2 bucket create my-app-files
```

```toml
[[r2_buckets]]
binding = "ATTACHMENTS"
bucket_name = "my-app-files"

# Image transforms (optional).
[images]
binding = "IMAGES"
```

The Worker talks to the bucket through its binding: no keys to keep.
`SOLI_ATTACHMENTS_R2_BINDING` names another binding than `ATTACHMENTS`. Under
`soli serve` the same declaration uses R2's S3-compatible API, with the `s3`
service's configuration — `S3_ENDPOINT=https://<account>.r2.cloudflarestorage.com`,
`S3_REGION=auto`, an R2 API token's `S3_ACCESS_KEY` / `S3_SECRET_KEY`, and
`SOLI_ATTACHMENTS_BUCKET` — so a local app and its Worker read the same
objects, under the same keys (`<collection>/<blob id>`).

**Reading back never crosses wasm.** The blob route (`<field>_url`) checks the
record and sets its headers, then hands the object to `worker.js`, which
answers from the bucket. A video player that seeks gets `206 Partial Content`
with just the bytes it asked for, a revalidation gets `304`, and memory stays
flat whatever the file's size.

**Image transforms** — `photo_url({"w": 400, "fmt": "webp"})`, `?thumb=`,
`?square=`, `?fit=cover`, `?crop=`, `?rot=`, `?gray=1`, `?q=` — are run by the
[Images binding](https://developers.cloudflare.com/images/transform-images/bindings/),
WebP and AVIF output included, on Cloudflare's machines rather than in the
Worker's CPU budget. Without an `[images]` binding they answer the stored
bytes. Images has no hue rotation or inversion, so `hue` and `invert` are
ignored there, and `blur`, `bright` and `contrast` are approximations of what
`soli serve` computes. Images bills transformations beyond its free monthly
allowance.

The Worker itself carries no image codecs: they would cost 260 KB of its
compressed size for work Cloudflare Images does better. On the edge the `Image`
class raises, saying so, and an uploader's storage-time transform (`format`,
`max_width`, `max_height`) is skipped: the original is stored, and URL
transforms resize it on read.

What uploads cannot do on the edge:

- **Large files.** The body arrives whole, and a file is copied a few times on
  its way into R2 (the request, wasm memory, the parsed part, its base64 form),
  against an isolate limit of 128 MB: keep `max_size` to about 20 MB.
- **Resumable (tus) uploads and `direct_upload`**, which need a disk or S3
  credentials.

## Background jobs: Cloudflare Queues

A Worker has no thread to poll a job table with, but Cloudflare runs a Worker's
`queue()` handler for each batch of messages a
[Queue](https://developers.cloudflare.com/queues/) receives. So on the edge,
`perform_later`, `perform_in`, `perform_at`, `Job.enqueue*` and
`Webhook.enqueue*` send the job — the row the native engine would insert — as a
message, and the Worker runs it: the same job classes from `app/jobs/`, the
same `static def perform(args)`, no code change.

```bash
npx wrangler queues create my-app-jobs
```

```toml
[[queues.producers]]
binding = "JOBS"
queue = "my-app-jobs"

[[queues.consumers]]
queue = "my-app-jobs"
max_retries = 100        # Soli decides retries with each job's max_retries
```

A failed job is redelivered after the native engine's backoff (5 s, 10 s,
20 s… capped at an hour) until its `max_retries` is spent, then acknowledged
and logged as dead (`npx wrangler tail` shows it). Delivery is at least once, as
with the job table: keep `perform` idempotent. All Soli queue names travel in
one Cloudflare Queue; `SOLI_QUEUE_BINDING` names another producer binding than
`JOBS`. A job runs in turn with the requests of its isolate, under the same CPU
limit.

What the edge cannot do with them:

- **Delays past 12 hours**, Cloudflare's limit: `perform_in("1 day", …)` raises.
- **Row operations** — `Job.list`, `Job.queues`, `Job.cancel`, `Job.retry` —
  raise: Cloudflare does not hand messages back. Its dashboard shows the queue.
- **Cron** (`Cron.*`, `static cron`): use a
  [Cron Trigger](https://developers.cloudflare.com/workers/configuration/cron-triggers/).

## What runs on the edge

| Runs | Stays with `soli serve` |
|------|-------------------------|
| routes, controllers, `before_action`, middleware | WebSockets, LiveView |
| views, layouts, partials, components, helpers | server-sent events, `stream`, blob streaming (answered `501`) |
| query strings, URL-encoded and multipart forms, JSON | resumable (tus) and direct uploads, file writes |
| i18n from `config/locales`; background jobs through Cloudflare Queues | cron, sending mail |
| models on D1, PostgreSQL (Hyperdrive), or SoliDB over HTTP(S) | MySQL, SQLite, the native SoliDB driver; transactions on D1 |
| the `HTTP` class (`fetch`); `HTTP.parallel*` run one after another | `--dev`: hot reload, dev bar, REPL |
| static files from `public/`; attachments on R2, with `Range` and image transforms | PDF, Office, the `Image` class, `System.run` |

A builtin that needs what a Worker lacks returns an error naming the edge
build ("… is not available on the edge (Cloudflare Workers) build").

## Limits worth knowing

- **One request at a time per isolate.** A request suspended on a query keeps
  its isolate's wasm stack; `worker.js` queues the next one behind it.
  Cloudflare spreads load across isolates, but a slow query delays the
  requests queued in the same isolate.
- **A Rust panic ends the isolate.** wasm is `panic = abort`: where `soli serve`
  answers a 500 and keeps the worker, the edge build loses the isolate, and the
  next request boots a fresh one.
- **Size.** About 2.9 MB compressed: inside the free plan's 3 MB, just. The
  Postgres driver adds about 40 KB.
- **Sessions.** The default in-memory store lives as long as an isolate; use
  the `cookie` driver (`SOLI_SESSION_DRIVER=cookie` and a 32+ character
  `SOLI_SESSION_SECRET` secret).
- **Redirects** of the `HTTP` class are followed by `fetch`; the SSRF check
  still screens every URL you pass it.

## Troubleshooting

| Symptom | Cause |
|---------|-------|
| `Soli failed to boot: …` (500 on every request) | the app did not load: the message is the boot error (syntax error, missing `app/controllers`…) |
| `501 Streaming responses are not available…` | the action streams (SSE, `stream`, `solidb_blob_response`) |
| `… is not available on the edge (Cloudflare Workers) build` | a builtin that needs a socket, a thread, a subprocess or a disk |
| `no edge runtime found` from `soli edge build` | pass `--runtime` or set `SOLI_EDGE_RUNTIME` to the directory `build-edge.sh` wrote |
| a model call hangs, then the request fails | `SOLIDB_HOST` is not reachable from Cloudflare (a LAN address, `localhost`) |
| `no D1 binding named DB` | `wrangler.toml` lacks the `[[d1_databases]]` block whose `binding` the D1 url names |
| `no Hyperdrive binding named HYPERDRIVE` | `wrangler.toml` lacks the `[[hyperdrive]]` block the connection's url names |
| `this Worker carries no postgres driver` | the build did not see the dialect: `soli edge build --sql postgres` |
| `no Queue binding named JOBS` | `wrangler.toml` lacks the `[[queues.producers]]` block for the job queue |
| `MySQL is not available on the edge build` | the app's connection is MySQL: move it to Postgres (Hyperdrive), D1 or SoliDB |
| `CSRF check failed: Origin … does not match` (403) | a POST whose Origin is another site — the same-origin gate of `soli serve`; list your other public hostnames in `SOLI_APP_HOSTS` |
| `no R2 binding named ATTACHMENTS` | `wrangler.toml` lacks the `[[r2_buckets]]` block an `r2` attachment needs |
| an image URL with `?w=` answers the full-size original | no `[images]` binding in `wrangler.toml` |
| `… is not supported on Cloudflare D1` | a transaction, a column-aware model, or the job table / cron on a D1 connection |
