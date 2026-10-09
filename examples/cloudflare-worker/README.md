# Soli on Cloudflare Workers

The site at <https://cf.solisoft.net>: a small Soli app, with no database,
that explains how to run Soli inside a Cloudflare Worker — and runs there.

```bash
soli serve .                                   # as usual, on your machine
soli edge build . --runtime ../../target/edge  # the Worker, in dist/edge
cd dist/edge && npx wrangler dev               # on workerd, locally
```

`../../target/edge` is what `scripts/build-edge.sh` produces in the Soli
source tree. The guide is the app itself (`/guide`), and the full reference is
`www/docs/edge.md`.
