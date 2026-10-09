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
  WebAssembly, 3 MB compressed. `scripts/build-edge.sh` builds it from the
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
- **Model queries** go to Cloudflare D1 through the Worker's binding, or to
  SoliDB over HTTP(S) with `fetch`. The interpreter is synchronous and both are
  asynchronous, so the wasm stack *suspends* on each query —
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
soli edge build [folder] [--out DIR] [--runtime DIR]
```

| Option | Default | |
|--------|---------|---|
| `folder` | `.` | the app (it must have `app/controllers`) |
| `--out`, `-o` | `<folder>/dist/edge` | the Wrangler project to write |
| `--runtime` | `$SOLI_EDGE_RUNTIME`, then `edge/` or `../share/soli/edge` next to the `soli` binary | where `build-edge.sh` put the runtime |

What goes where:

- `app/`, `config/` and `lib/` → `src/app.json`, mounted at `/app` in the
  Worker. Only UTF-8 text files; a binary file is skipped with a line saying so
  (binary files belong in `public/`).
- `public/` → `public/`, served as [static assets](https://developers.cloudflare.com/workers/static-assets/)
  before the Worker runs. The copy is fresh each build.
- `.env` and anything outside those directories is **not** bundled.
- `wrangler.toml` is written on the first build only. Later builds keep it, so
  your name, routes, vars and account stay.

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
validations, callbacks — on either of two backends.

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
- **Jobs and cron**, which have no worker on the edge anyway.

### SoliDB

Over SoliDB's HTTP API. The Worker must be able to reach `SOLIDB_HOST`: a
public `https://` address. Credentials follow the usual order (JWT login with
`SOLIDB_USERNAME` / `SOLIDB_PASSWORD`, `SOLIDB_API_KEY`, then basic auth).

Not available on the edge: PostgreSQL, MySQL and SQLite (native drivers), and
the native SoliDB driver — the edge build uses D1 or SoliDB over HTTP.

## What runs on the edge

| Runs | Stays with `soli serve` |
|------|-------------------------|
| routes, controllers, `before_action`, middleware | WebSockets, LiveView |
| views, layouts, partials, components, helpers | server-sent events, `stream`, blob streaming (answered `501`) |
| query strings, URL-encoded forms, JSON | multipart uploads, file writes |
| i18n from `config/locales` | background jobs, cron, sending mail |
| models on D1, or on SoliDB over HTTP(S) | PostgreSQL, MySQL, SQLite, the native SoliDB driver; transactions on D1 |
| the `HTTP` class (`fetch`); `HTTP.parallel*` run one after another | `--dev`: hot reload, dev bar, REPL |
| static files from `public/` | PDF, Office, lossy WebP, `System.run` |

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
- **Size.** About 3 MB compressed: inside the paid plan's limit, at the edge of
  the free plan's 3 MB.
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
| `… is not supported on Cloudflare D1` | a transaction, a column-aware model, jobs or cron on a D1 connection |
