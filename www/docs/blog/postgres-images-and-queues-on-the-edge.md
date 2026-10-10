# Postgres, images and queues on the edge

Soli 2.20.1 put a Soli app inside a Cloudflare Worker, with its models on D1.
That covers a page, a form and a table. A real app reaches for three more
things soon after: files people upload, the Postgres database it already has,
and work to do after the response. Soli 2.21 brings all three to the Worker,
and the Worker got smaller on the way: 2.94 MB compressed, under the free
plan's 3 MB.

<figure style="margin:1.5rem auto;max-width:1024px;">
  <img src="/images/blog/postgres-images-and-queues-on-the-edge.svg" width="1024" height="576" alt="A Soli app inside a Cloudflare Worker, 2.94 MB compressed. Requests and queue messages enter it. It stores uploads in R2 and serves them with Range and Cloudflare Images transforms, reads and writes Postgres through Hyperdrive with the pg driver, and sends background jobs to Cloudflare Queues, which hand them back to the Worker." style="display:block;width:100%;height:auto;border-radius:12px;border:1px solid #30363d;background:#0b0d0f;">
  <figcaption style="text-align:center;color:#8b949e;font-size:0.875rem;margin-top:0.5rem;">The same app code, with Cloudflare's storage, database pool and queues behind it.</figcaption>
</figure>

None of it asks for new code in the app. The model that declares an
attachment, the job class, the Postgres connection: they are the ones
`soli serve` runs. What changes is `wrangler.toml`.

## Uploads, images and video, kept in R2

Until now a Worker app could not take an upload at all: request bodies crossed
into WebAssembly as text, which mangled anything binary. They cross as bytes
now, multipart forms are parsed by the same parser `soli serve` uses, and an
attachment can live in [R2](https://developers.cloudflare.com/r2/), Cloudflare's
object storage:

```soli
# app/models/photo.sl
class Photo < Model
  has_one_attached("image", {
    "service": "r2",
    "content_types": ["image/jpeg", "image/png", "video/mp4"],
    "max_size": 20_000_000
  })
end
```

```soli
# config/routes.sl
uploads("photos", "image")
```

```erb
<img src="<%= photo.image_url({"w": 400, "fmt": "webp"}) %>" alt="">
```

```toml
# wrangler.toml
[[r2_buckets]]
binding = "ATTACHMENTS"
bucket_name = "my-app-files"

[images]
binding = "IMAGES"
```

The Worker reaches the bucket through its binding, so there are no keys to
keep. Under `soli serve` the same declaration talks to R2's S3-compatible API,
under the same keys: a local app and its Worker see the same files.

**Reading a file back never goes through WebAssembly.** The blob route checks
the record and sets its headers (type, disposition, `nosniff`, cache), then
hands the object to `worker.js`, which answers straight from the bucket. A
video player that seeks gets `206 Partial Content` with only the bytes it asked
for, a revalidation gets `304`, and memory stays flat whatever the file's size.
We checked each of these on workerd with a local R2: a JPEG and an MP4 came
back byte for byte, `Range: bytes=100-199` and a suffix range got their 206,
and an `If-None-Match` got its 304.

**Image transforms run on Cloudflare Images.** `?w=`, `?thumb=`, `?square=`,
`?fit=cover`, `?crop=`, `?fmt=webp` (and AVIF) become options for the
[Images binding](https://developers.cloudflare.com/images/transform-images/bindings/),
so resizing costs nothing of the Worker's CPU budget. In the local test,
`?w=300` on a 1200×800 photo answered a 300×200 WebP. Without an `[images]`
binding the stored bytes are served instead.

## A smaller Worker, because Cloudflare does the pixels

We first made the `Image` class work in the Worker too. Then we measured what
it cost: `twiggy` put the image decoders and encoders at about 595 KB of code,
and leaving them out saves 804 KB of WebAssembly, **259 KB compressed**. That
was the difference between being over and under the free plan's 3 MB limit, for
work Cloudflare Images does better.

So the edge build carries no image codecs. `Image` raises there with a pointer
to URL transforms, and an uploader's storage-time `format` / `max_width` is
skipped: the original is stored, and resized on read.

| Edge runtime, compressed | |
|---|---|
| Soli 2.20.1 | 3,120,969 bytes |
| Soli 2.21, with R2, Postgres and Queues | 2,939,515 bytes |

## Your Postgres, through Hyperdrive

A Worker has no socket a Rust database client could open, so the native
Postgres client is not in the edge build. A JavaScript driver can open one:
`pg` connects through the Worker's TCP sockets. And
[Hyperdrive](https://developers.cloudflare.com/hyperdrive/) keeps a pool of
connections next to your database for it, so a request does not pay a TLS
handshake to the database.

```bash
npx wrangler hyperdrive create my-db --connection-string="postgres://user:password@db.example.com:5432/my_app"
```

```toml
compatibility_flags = ["nodejs_compat"]

[[hyperdrive]]
binding = "HYPERDRIVE"
id = "…"
localConnectionString = "postgres://user:password@localhost:5432/my_app"

[vars]
SOLI_DB_ADAPTER = "postgres"
```

`soli edge build` sees that the app uses Postgres, bundles `pg` (about 40 KB),
runs `npm install` and turns on `nodejs_compat`. `DATABASE_URL` defaults to the
`HYPERDRIVE` binding.

The models are the native adapter's: the edge build compiles exactly the SQL
`soli serve` sends to Postgres and hands each statement to `pg`, suspending the
WebAssembly stack until it answers, as it already did for D1. Because every
statement of a request runs on one connection, **transactions work**, and so do
column-aware models on a schema you already have:

```soli
class Order < Model
  table "orders"          # your existing table, its real columns
end

Order.transaction(fn() {
  order = Order.create({"customer_id": 7, "total": 4200})
  Shipment.create({"order_id": order.id})
})
```

We ran the model suite through this path against a real Postgres 18 —
documents, deep merges, aggregates, `create_many`, commit and rollback, a
column-aware table — and the same checks end to end on workerd with
Hyperdrive's local connection and the real `pg`. No connection was left open
afterwards.

MySQL stays off the edge. Its JavaScript driver alone weighs 410 KB compressed,
more than the room left under 3 MB.

## Background jobs, on Cloudflare Queues

A Worker has no thread to poll a job table with. But Cloudflare calls a
Worker's `queue()` handler for every batch of messages a
[Queue](https://developers.cloudflare.com/queues/) receives. So on the edge,
`perform_later` sends the job — the row the native engine would have written —
as a message, and the Worker runs it with the job class you already have:

```soli
# app/jobs/record_visit_job.sl
class RecordVisitJob
  static def perform(args)
    Visit.create({"path": args["path"], "at": DateTime.utc.to_iso})
  end
end

# in a controller: answer now, record after
RecordVisitJob.perform_later({"path": req["path"]})
RecordVisitJob.perform_in("10 minutes", {"path": "/reminder"})
```

```toml
[[queues.producers]]
binding = "JOBS"
queue = "my-app-jobs"

[[queues.consumers]]
queue = "my-app-jobs"
max_retries = 100
```

A failing job is retried with the same rule and backoff as on a server, until
its `max_retries` is spent, then logged as dead. On workerd with a local queue,
two jobs wrote their rows to D1 from the consumer, and one that threw was
redelivered after 5 s, 11 s, then 23 s. The whole feature adds 8.5 KB to the
Worker.

What Queues cannot do: delay a message past 12 hours (`perform_in("1 day")`
raises), or hand messages back, so `Job.list`, `Job.cancel` and `Job.retry`
raise on the edge. Cron stays with `soli serve`; on a Worker, use a
[Cron Trigger](https://developers.cloudflare.com/workers/configuration/cron-triggers/).

## Fixed on the way

Running the model suite through a second Postgres and MySQL path turned up four
bugs, none of them edge-specific in effect:

- **On MySQL, a write inside `transaction` committed it.** Every write in a
  transaction ran `CREATE TABLE IF NOT EXISTS`, and MySQL commits before any
  `CREATE TABLE`, even one that creates nothing. A rollback undid nothing
  written before it. The adapter now checks first.
- **Postgres and MySQL sorted numbers as text**: `order("views")` put `"7"`
  after `"10"`, where SQLite and D1 sorted numerically. Numbers sort as numbers
  everywhere now; strings keep their collation.
- **`aggregate({...})` failed on column-aware models** with `column "doc" does
  not exist`. It aggregates the real columns now.
- **The Worker skipped the same-origin CSRF check** `soli serve` runs before
  routing, leaving only CSRF tokens in front of a cross-site form post. A
  cross-site `POST` gets its `403` on the edge too.

## Try it

```bash
./scripts/build-edge.sh
soli edge build . --runtime target/edge
cd dist/edge && npx wrangler dev
```

The [Cloudflare Workers guide](/docs/development-tools/edge) has the full
reference: [files](/docs/development-tools/edge#files),
[Postgres](/docs/development-tools/edge#hyperdrive) and
[jobs](/docs/development-tools/edge#queues), with what each one refuses.
