# Soli on Cloudflare Workers

The site at <https://cf.solisoft.net>: a small Soli app that explains how to
run Soli inside a Cloudflare Worker — and runs there. Its `/d1` page keeps
its data in Cloudflare D1.

```bash
soli serve .                                   # as usual, on your machine
soli edge build . --runtime ../../target/edge  # the Worker, in dist/edge
cd dist/edge && npx wrangler dev               # on workerd, locally
```

`../../target/edge` is what `scripts/build-edge.sh` produces in the Soli
source tree.

## Deploying cf.solisoft.net

`wrangler.toml` here is the site's deploy config — the Worker's name, the
account that owns solisoft.net, the custom domain, the D1 database, the
observability settings. `soli edge build` copies it into `dist/edge` on every
build, so a clone deploys the site as it is:

```bash
soli edge build . --runtime ../../target/edge
cd dist/edge && npx wrangler deploy   # after `npx wrangler login`, or with CLOUDFLARE_API_TOKEN
```

To run your own copy, change `name`, `account_id`, `routes` and the D1
`database_id` first. The guide is the app itself (`/guide`), and the full reference is
`www/docs/edge.md`.

## The D1 page

`/d1` stores check-ins in a `CheckIn` model. On the Worker, models run on D1;
create the database once and bind it in `dist/edge/wrangler.toml`:

```bash
npx wrangler d1 create soli-cf-demo          # prints the database_id
```

```toml
[[d1_databases]]
binding = "DB"
database_name = "soli-cf-demo"
database_id = "…"

[vars]
SOLI_DB_ADAPTER = "d1"
```

The table is created on the first write. Locally, the same model runs on
SQLite (`.env` is not bundled into the Worker):

```bash
SOLI_DB_ADAPTER=sqlite DATABASE_URL=sqlite://db/demo.sqlite3 soli serve .
```
