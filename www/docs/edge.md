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

## Start an app for Workers: `soli new --edge`

```bash
soli new my_app --edge
cd my_app && soli edge dev       # on workerd, http://localhost:8787
```

The usual application, set up to deploy as a Worker:

- **`wrangler.toml`** beside `app/` — the Worker's name, its static assets,
  models on D1 (`SOLI_DB_ADAPTER = "d1"`), sessions in the `cookie` driver,
  and the R2, Images, Queues and Hyperdrive blocks ready to uncomment.
  `soli edge build` copies it in on every build: commit it.
- **`.env`** for `soli serve` on your machine: the same models on SQLite (D1
  is SQLite) and the same cookie sessions, with a generated
  `SOLI_SESSION_SECRET`.
- **`.dev.vars`** — the same secret for `soli edge dev` (wrangler's local-only
  variables); on Cloudflare it is set with `npx wrangler secret put`.
- `.gitignore` entries for `dist/edge/`, `.wrangler/` and `.dev.vars`, and a
  README and `CLAUDE.md` that say where the app runs and how to deploy it.

## How it works

- **The edge runtime** is the Soli library built for `wasm32-unknown-unknown`
  without its server, CLI and native-only builtins: about 11.7 MB of
  WebAssembly, 3.6 MB gzipped. `scripts/build-edge.sh` builds it from the
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
  your name, routes, vars and account stay. An app that keeps its **own**
  `wrangler.toml` next to `app/` has it copied in on every build instead:
  commit it, and the deploy config lives with the code (nothing in it is a
  secret; tokens come from `wrangler login` or `CLOUDFLARE_API_TOKEN`).
- For an app on PostgreSQL, the driver: `src/sql-pg.js`, `pg` in
  `package.json`, and `npm install`.

Add `dist/` to the app's `.gitignore`.

## Develop on workerd: `soli edge dev`

```bash
soli edge dev [folder] [--port 8787] [--runtime DIR] [--sql postgres|none]
```

Packages the app into `dist/edge`, runs it with `wrangler dev --local` — the
runtime Cloudflare runs, with local D1, R2, KV and Queues stored under
`dist/edge/.wrangler/state` — and repackages on every change under `app/`,
`config/`, `lib/`, `public/` or to `wrangler.toml`; wrangler reloads the
Worker and the open page. The app's `.dev.vars` (wrangler's local-only
variables) is copied in.

`wrangler dev` hands the Worker the production host of `routes` as `Host`,
so `soli edge dev` adds the addresses the page is really opened from to
`SOLI_APP_HOSTS` — `localhost:<port>`, `127.0.0.1:<port>`, and the hosts
listed in the app's `.env` or passed by soli-proxy — or forms would fail the
same-origin check locally.

Behind **soli-proxy** it is a site like the others (`sites/<name>` linking to
the app, with an `app.infos`):

```toml
name = "cf.solisoft.test"
domain = "cf.solisoft.test"
start_script = "env SOLI_EDGE_RUNTIME=/path/to/edge-runtime soli edge dev . --port $PORT"
health_check = "/"
```

`https://cf.solisoft.test` then shows the page the Worker renders.

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
`request.cf.colo`. When the edge sent no `cf-ipcountry` (IP geolocation off on
the zone, or `wrangler dev`, which fills `request.cf` but adds no `cf-*`
header), `worker.js` adds it from `request.cf.country`.

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
round trip, so writes use `RETURNING doc` rather than reading the row back, and
the reads inside a [`grouped(fn() { … })`](models.md#coalescing-reads-grouped)
block go to D1 together in one `batch()`: one round trip for the block.
What D1 does not offer:

- **Transactions.** D1 runs batches, not interactive transactions:
  `Model.transaction` raises.
- **Column-aware models** (a model that declares its columns) raise: store
  them as documents.
- **The job table and cron**: on the edge, jobs go through
  [Cloudflare Queues](#background-jobs-cloudflare-queues) and `static cron`
  through [Cron Triggers](#cron-cloudflare-cron-triggers) instead.

One D1 database per site: a connection `url = "d1://SITE_{tenant}"` sends each
tenant's queries to its own binding (`switch_tenant("acme")` → `SITE_acme`),
declared in `wrangler.toml` with one `[[d1_databases]]` block per site. Bindings
are fixed at deploy, so a new site means a new binding and a redeploy — for many
sites, use [SQLite in Durable Objects](#sqlite-in-durable-objects-a-database-per-tenant)
instead. See [One database per tenant](multi-database.md#one-database-per-tenant).

### SQLite in Durable Objects: a database per tenant

A [Durable Object](https://developers.cloudflare.com/durable-objects/) with
SQLite storage is a small database of its own, created the first time it is
named. Soli's D1 adapter reaches one with a `do://` url: the binding, then the
object's name.

```toml
# config/database.toml
default = "primary"

[connections.primary]            # accounts, sites — on D1
adapter = "d1"
url = "d1://DB"

[connections.site]               # each site's data — its own Durable Object
adapter = "d1"
url = "do://SITES/{tenant}"
```

```toml
# wrangler.toml
[[durable_objects.bindings]]
name = "SITES"
class_name = "SoliSqlObject"

[[migrations]]
tag = "v1"
new_sqlite_classes = ["SoliSqlObject"]
```

`SoliSqlObject` ships in every Worker `soli edge build` packages: it runs the
statements it is sent on its own storage and answers like D1, so models,
`grouped` (one RPC per block, run in one storage transaction), and everything
else the D1 adapter does work unchanged. With
[`switch_tenant(site.db_key)`](multi-database.md#one-database-per-tenant) in a
middleware, `do://SITES/{tenant}` gives every site a database of its own —
unlike a D1 binding per site, nothing is declared per site and nothing is
redeployed when one is added. A fixed name (`do://SITES/main`) makes a single
object-backed database.

The same limits as D1 apply (no `Model.transaction`, no column-aware models), a
Durable Object holds up to 10 GB, and its object name is limited to letters,
digits, `_` and `-`. Under `soli edge dev` the objects are local, in
`dist/edge/.wrangler/state`.

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
with `soli serve`: `db:create` / `db:drop` and schema dumps (jobs run through
[Cloudflare Queues](#background-jobs-cloudflare-queues), `static cron` through
[Cron Triggers](#cron-cloudflare-cron-triggers)).

Not available on the edge: **MySQL** (no driver glue yet), SQLite and the
native SoliDB driver. The edge build uses D1, PostgreSQL or
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

The Worker itself carries no image codecs: they would add 260 KB (gzipped)
to it for work Cloudflare Images does better. On the edge the `Image`
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
- **Cron**: `static cron` runs on Cron Triggers (next section); `Cron.schedule`
  and the other `Cron.*` calls that write the cron table do not run.

## Cron: Cloudflare Cron Triggers

A job class's `static cron` runs on the edge with no change: Cloudflare calls
the Worker's `scheduled()` handler at each time
[`[triggers] crons`](https://developers.cloudflare.com/workers/configuration/cron-triggers/)
in `wrangler.toml` lists, and the Worker runs the jobs whose schedule it is.

```soli
# app/jobs/nightly_report_job.sl
class NightlyReportJob
  static cron = Cron.daily_at("03:15")

  static def perform(args)
    Report.generate
  end
end
```

`soli edge build` reads the `static cron` declarations of `app/jobs/` and
writes the triggers into the `wrangler.toml` it generates; an existing
`wrangler.toml` gets a warning that lists the ones it lacks:

```toml
[triggers]
crons = ["15 3 * * *"]
```

- **Five fields, UTC, to the minute.** Soli's expressions have a seconds field
  first (`0 15 3 * * *`); a trigger drops it. A schedule that needs seconds
  (`Cron.every("30 seconds")`, `"30 * * * * *"`) fails the build, naming its
  file.
- **Read from the source.** A string literal or a `Cron.*` helper called with
  literals (`daily_at`, `hourly`, `every`, `weekly_at`). A schedule computed
  when the app runs (a constant) is a warning: add its trigger by hand.
- **Several jobs on one schedule share a trigger** and all run, one after the
  other, with no tenant. One failing does not stop the others; it fails the
  invocation, so Cloudflare's dashboard shows it, and `npx wrangler tail` its
  error. There are no retries: a job that must not be lost can enqueue its
  work with `perform_later` (Cloudflare Queues retry).
- **Not available:** `Cron.schedule` / `update` / `delete` and `Cron.list`,
  which write and read the cron table: a trigger is fixed when the Worker is
  deployed.
- **Count.** Cloudflare allows 5 Cron Triggers per account on the Workers Free
  plan, 250 on Paid; the build warns past 5.
- **Locally**, `npx wrangler dev --test-scheduled` and
  `curl "http://127.0.0.1:8787/__scheduled?cron=15+3+*+*+*"` fire a trigger.

## Mail: over HTTP

A Worker cannot open a connection to a mail server, so `delivery_method:
"smtp"` raises there, saying so. Mail goes out over HTTP instead: the
[`http` delivery method](mailer.md#over-http) POSTs each message to a
provider's API — as JSON, or reshaped by a
[provider class](blog/mail-over-http-provider-classes.md) (Brevo, Resend,
Postmark, SendGrid, Mailgun) — through the Worker's `fetch`.

```soli
# config/application.sl
Mailer.configure({"delivery_method": "http", "request": "BrevoMail",
                  "api_key": getenv("BREVO_API_KEY"), "from": "Acme <noreply@example.com>"})
```

Keep the key a secret (`npx wrangler secret put BREVO_API_KEY`). Your mailers do
not change: `deliver_now` sends from the request, and `deliver_later` becomes a
[Cloudflare Queues](#background-jobs-cloudflare-queues) message whose job sends
it, retried like any job — without a `JOBS` binding it sends at once.
Attachments travel base64 in the request, so keep them well under the
~20 MB a request body allows.

## LiveView: sockets in Durable Objects

A [LiveView](liveview.md) runs on the edge unchanged: the same template, the
same `router_live` handler, the same `/live/client.js`. A Worker cannot keep a
socket between requests, so each LiveView instance gets a
[Durable Object](https://developers.cloudflare.com/durable-objects/):

- The Worker sends `/live/socket/<component>` to the object of that instance —
  the visitor's session's, or the room's for a component `live_rooms`
  declared (an undeclared `?room=` shares nothing, as under `soli serve`).
- The object accepts the socket with the
  [hibernation API](https://developers.cloudflare.com/durable-objects/best-practices/websockets/)
  and keeps the instance — its state and last markup — in its own storage. It
  can be evicted between events; the next event wakes it, reads the instance
  back, runs one frame (handler, render, diff) and stores the result. Every
  socket of the instance gets the patch.
- `soli edge build` adds the binding to the `wrangler.toml` it writes when
  `config/routes.sl` declares `router_live`; an existing `wrangler.toml` gets a
  warning naming it:

```toml
[[durable_objects.bindings]]
name = "SOLI_LIVE"
class_name = "SoliLiveObject"

[[migrations]]
tag = "soli-live-v1"
new_sqlite_classes = ["SoliLiveObject"]
```

Measured on `wrangler dev --local` (workerd) with a counter: a click takes
about 1 ms inside wasm and 8–10 ms round trip; the first socket of a fresh
isolate gets its render in 65–170 ms (boot included). The instance survives a
restart of the whole process. LiveView adds about 205 KB to the runtime.

Not carried on the edge yet: `tick` intervals, live queries
(`Model.live_where`), `live_update` from a controller or a job — they need a
process that stays up — LiveView uploads (`soli-upload`), and EUI. Other
WebSocket routes and server-sent events still answer `501`.

## What runs on the edge

| Runs | Stays with `soli serve` |
|------|-------------------------|
| routes, controllers, `before_action`, middleware; [LiveView](#liveview-sockets-in-durable-objects) | other WebSockets; LiveView `tick`, live queries, `live_update` |
| views, layouts, partials, components, helpers | server-sent events, `stream`, blob streaming (answered `501`) |
| query strings, URL-encoded and multipart forms, JSON | resumable (tus) and direct uploads, file writes |
| i18n from `config/locales`; background jobs through Cloudflare Queues; `static cron` through Cron Triggers | `Cron.schedule` and the cron table, SMTP (mail goes [over HTTP](#mail-over-http)) |
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
- **Size.** About 11.7 MB uncompressed (3.6 MB gzipped), against Workers'
  64 MiB limit on the uncompressed bundle, the same on the free and paid
  plans; there is no compressed limit any more. The Workers limit that still
  applies is startup time: 1 second. The Postgres driver adds about 40 KB.
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
