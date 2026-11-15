//! What `soli new --edge` adds: an application that deploys to Cloudflare
//! Workers. Its `wrangler.toml` sits beside `app/`, so `soli edge build`
//! copies it into the Worker on every build and the deploy config is
//! committed with the code.

/// The `compatibility_date` Soli writes into a new `wrangler.toml`: the
/// Workers runtime behaviour the edge build is tested against (it is the
/// date of the deployed demo, `examples/cloudflare-worker`).
///
/// A fixed date, not today's. Wrangler refuses a date newer than the workerd
/// it ships with, so "today" made a fresh app fail on any wrangler older than
/// the day it was generated — including the version CI pins. Raise it when
/// the runtime needs something newer, together with that wrangler pin.
pub const COMPATIBILITY_DATE: &str = "2026-10-09";

/// `wrangler.toml`: the Worker, its static assets, models on D1, sessions in
/// the cookie driver, and the other bindings commented out.
pub fn wrangler_toml(worker: &str, db_name: &str, today: &str) -> String {
    format!(
        r#"# The Cloudflare Worker this app deploys as. `soli edge build` and
# `soli edge dev` copy this file into dist/edge on every build: edit it here,
# commit it. Nothing in it is a secret — tokens come from `npx wrangler login`
# or CLOUDFLARE_API_TOKEN, secrets from `npx wrangler secret put NAME`.
name = "{worker}"
main = "src/worker.js"
compatibility_date = "{today}"
# The account to deploy to, when your token reaches several:
# account_id = "…"

# A custom domain, once its zone is on the same Cloudflare account:
# routes = [{{ pattern = "app.example.com", custom_domain = true }}]

# public/ is served before the Worker runs.
[assets]
directory = "./public"

# Configuration the app reads with getenv(). Secrets do not go here:
#   npx wrangler secret put SOLI_SESSION_SECRET   # 32+ characters
[vars]
SOLI_DB_ADAPTER = "d1"
# The in-memory session store lives as long as an isolate; sessions travel in
# an encrypted cookie instead (needs SOLI_SESSION_SECRET).
SOLI_SESSION_DRIVER = "cookie"
SOLI_CSRF_TOKENS = "require"

# Models on Cloudflare D1. `soli edge dev` runs a local copy; before the first
# deploy, `npx wrangler d1 create {db_name}` prints the id to put here.
[[d1_databases]]
binding = "DB"
database_name = "{db_name}"
database_id = "00000000-0000-0000-0000-000000000000"

# Attachments on R2 (`service: "r2"`; `npx wrangler r2 bucket create {worker}-files`).
# [[r2_buckets]]
# binding = "ATTACHMENTS"
# bucket_name = "{worker}-files"

# Image transforms on R2 attachment URLs (`?w=300&fmt=webp`).
# [images]
# binding = "IMAGES"

# One SQLite database per site, each in a Durable Object: a connection in
# config/database.toml with adapter = "d1", url = "do://SITES/{{tenant}}" (see
# switch_tenant). Objects are created by naming them: nothing per site here.
# [[durable_objects.bindings]]
# name = "SITES"
# class_name = "SoliSqlObject"
# [[migrations]]
# tag = "v1"
# new_sqlite_classes = ["SoliSqlObject"]

# Background jobs (`perform_later`) through Cloudflare Queues
# (`npx wrangler queues create {worker}-jobs`).
# [[queues.producers]]
# binding = "JOBS"
# queue = "{worker}-jobs"
# [[queues.consumers]]
# queue = "{worker}-jobs"
# max_retries = 100

# Postgres through Hyperdrive instead of D1: set SOLI_DB_ADAPTER = "postgres"
# above, add compatibility_flags = ["nodejs_compat"] at the top, and
# [[hyperdrive]]
# binding = "HYPERDRIVE"
# id = "…"
# localConnectionString = "postgres://user:password@localhost:5432/{db_name}"
"#
    )
}

/// `.env`, for `soli serve` on this machine: the same models on SQLite (D1 is
/// SQLite), the same session driver.
pub fn env_file(db_name: &str, session_secret: &str) -> String {
    format!(
        r#"# This app deploys to Cloudflare Workers (wrangler.toml). This file is for
# `soli serve` and `soli edge dev` on your machine; the Worker never sees it.

# Models: D1 on the Worker, the same SQLite document tables locally.
SOLI_DB_ADAPTER=sqlite
DATABASE_URL=sqlite://db/{db_name}.sqlite3

# Sessions in an encrypted cookie, as on the Worker.
SOLI_SESSION_DRIVER=cookie
SOLI_SESSION_SECRET={session_secret}

# Require a valid per-form CSRF token on browser form posts.
SOLI_CSRF_TOKENS=require

# Public hostnames this app is opened under locally besides localhost — a
# soli-proxy site, for instance. `soli edge dev` passes them to the Worker.
# SOLI_APP_HOSTS=myapp.solisoft.test
"#
    )
}

/// `.dev.vars`: wrangler's local-only variables, for `soli edge dev`. The
/// deployed Worker's secret is set with `wrangler secret put`.
pub fn dev_vars(session_secret: &str) -> String {
    format!(
        "# Local secrets for `soli edge dev` (wrangler dev). Never committed; on Cloudflare\n\
         # set them with `npx wrangler secret put SOLI_SESSION_SECRET`.\n\
         SOLI_SESSION_SECRET=\"{session_secret}\"\n"
    )
}

/// Appended to `.gitignore`.
pub const GITIGNORE_EXTRA: &str = r#"
# Cloudflare Workers: the packaged Worker, wrangler's local state (D1, R2…),
# its npm drivers, and local secrets.
/dist/edge/
.wrangler/
.dev.vars
"#;

/// Appended to the root `CLAUDE.md`, so an agent working in the app knows
/// where it runs.
pub const CLAUDE_MD_EXTRA: &str = r#"

## This app runs on Cloudflare Workers

It was created with `soli new --edge`. `wrangler.toml` (beside `app/`) is the
deploy config; `soli edge dev` runs the app on workerd with local D1/R2/Queues,
`soli edge build` packages it into `dist/edge`, `npx wrangler deploy` ships it.

- Models are on D1 (SQLite) on the Worker and on SQLite under `soli serve`.
- `.env` is never bundled: the Worker reads `[vars]` in `wrangler.toml` and
  secrets set with `npx wrangler secret put`.
- Not available on the edge: WebSockets/LiveView, SSE/streaming, the `Image`
  class, sending mail, cron, MySQL/SQLite drivers. Background jobs go through
  Cloudflare Queues, uploads through R2 (`service: "r2"`). Reference:
  docs/edge.md.
"#;

/// `README.md` of an edge app.
pub fn readme(name: &str, db_name: &str) -> String {
    format!(
        r#"# {name}

A Soli application that runs on Cloudflare Workers.

## Develop

```bash
soli edge dev            # the app on workerd, http://localhost:8787, rebuilt as you edit
```

It runs the runtime Cloudflare runs, with a local D1 database under
`dist/edge/.wrangler/state`. `soli serve . --dev` works too, on SQLite
(`.env`). `soli edge dev` needs the edge runtime: pass `--runtime <dir>` or set
`SOLI_EDGE_RUNTIME` to the folder `scripts/build-edge.sh` wrote in the Soli
source tree.

## Deploy

```bash
npx wrangler login
npx wrangler d1 create {db_name}               # put the id in wrangler.toml
npx wrangler secret put SOLI_SESSION_SECRET    # 32+ characters
soli edge build
cd dist/edge && npx wrangler deploy
```

`wrangler.toml` is the deploy config: commit it. Uncomment its R2, Images,
Queues or Hyperdrive blocks when the app needs them; what each one does, and
what stays with `soli serve`, is in `docs/edge.md`.
"#
    )
}
