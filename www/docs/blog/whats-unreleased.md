# What's on `main` since v1.29.0

v1.29.0 shipped on 9 August. Everything below is already on `main` and will
land in the next tag. This is a tour of that cycle — not a dump of the
changelog. The exhaustive list lives on the
[changelog](/docs/getting-started/changelog#unreleased).

The short version: **SQL is a real backend**, **jobs no longer phone home**,
**LiveView stopped leaking and leaking memory**, and **auth stopped helping
attackers**. Plus `unless … end` is a real statement, and you can take
Stripe payments without a generator.

<figure style="margin:1.5rem auto;max-width:1024px;">
  <img src="/images/blog/whats-unreleased.svg" width="1024" height="576" alt="Unreleased cycle since v1.29.0: SQL adapters, in-process jobs, LiveView rooms, auth hardening, and unless/end." style="display:block;width:100%;height:auto;border-radius:12px;border:1px solid #30363d;background:#0b0d0f;">
  <figcaption style="text-align:center;color:#8b949e;font-size:0.875rem;margin-top:0.5rem;">Four backends, one Model surface. One process owns the queue. LiveView rooms share a board. Auth no longer times the miss path.</figcaption>
</figure>

## SQL is no longer a sketch

You can point Soli at Postgres, MySQL, or a SQLite file and keep the same
`Model` calls. Document tables (`_key` + `doc`) work everywhere. So do
**column-aware** models: `table "orders"` maps onto a schema you already have.

```soli
class Order < Model
  table "orders"
  belongs_to("account")
  has_many("line_items")
  encrypts(:tax_id)
end

class Admin < User
  # STI: same table, string `type` column
end
```

What that buys you on this cycle:

- **Hash `.where` is a real vocabulary** — `{ "total": { "gt": 100 } }`,
  `{ "id": [1, 2, 3] }`, `{ "email": { "ilike": "%@x.com" } }`,
  `{ "or": [{ "state": "draft" }, { "state": "open" }] }`. Equality-only
  hashes (and a silent `{ "gt": 10 }` → `==`) are gone.
- **`.includes`, HABTM, `through:`, `.join` (EXISTS), `.having`** on SQL.
  Three parents with `includes("books")` is two queries, not four.
- **Atomic `increment` / `decrement` and counter caches** — 200 of 200
  concurrent bumps on SQLite; the old read-modify-write kept 53.
- **Constraint violations become field errors** on every adapter, not
  "sqlite column insert row: UNIQUE constraint failed…".
- **`encrypts` and STI on real columns.** Missing text/`type` columns fail
  at boot, not at 2 a.m.
- **Migrations can create column tables** (`pk`, `decimal(10,2)`,
  `timestamps: true`) and declare `connection "analytics"`.
- **`soli db:create` / `db:drop` / `db:schema:dump` / `db:schema:load`.**
- **The job queue indexes itself** on first enqueue. Document `index
  "status"` is real DDL the planner can use.
- **The dev bar, `dev_queries()`, and N+1 detection work on SQL.** They
  were SoliDB-only; the N+1 guard was blind on three of four backends.
- **SQLite is first-class** — same Model surface, WAL + busy timeout +
  foreign keys, `BEGIN IMMEDIATE` transactions, jobs via the write lock.
  One writer at a time; `DECIMAL` is REAL affinity. Honest caveats, not
  a pretend Postgres.

Still refused on column mode: composite primary keys.

Pages: [Multiple Databases](/docs/database/multi-database),
[Postgres](/docs/database/postgres), [MySQL](/docs/database/mysql),
[SQLite](/docs/database/sqlite), [Migrations](/docs/database/migrations).

## Jobs live in your process

The queue is a `_jobs` collection on the default connection. A poller
claims due work (Postgres `SKIP LOCKED`, MySQL a token, SQLite the write
lock, SolidB `If-Match`). Workers run handlers. Soli owns retries,
leases, and cron.

There is no `POST /_jobs/run/:name`, no callback URL, no secret Stripe
(or SolidB) must use to reach you. Let the old SolidB queue drain before
you upgrade.

```soli
WelcomeEmailJob.perform_later({ "user_id": user.id })
WelcomeEmailJob.perform_now({ "user_id": user.id })   # tests: no queue

Cron.daily_at("08:00", "DigestJob")
```

Ops:

- `soli jobs` / `soli worker` — claim and run with no HTTP listener
  (`SOLI_JOB_WORKERS=0` on `soli serve`).
- `soli jobs list` / `retry` / `cancel`.
- [`/__soli/jobs`](/__soli/jobs) — same UI. Open in `--dev`. In
  production, set `SOLI_JOBS_USER` + `SOLI_JOBS_PASSWORD` and/or
  `SOLI_JOBS_TOKEN`. Unconfigured production is a 404.

See [Jobs](/docs/builtins/jobs).

## LiveView grew up

The Field Desk tutorial — [A Live Field Desk](/docs/blog/liveview-desk) —
is the demo. Under it, the cycle closed a lot of holes.

**Features you can use**

- Nested `live_component` shares parent assigns; `send_update` runs a
  child's `event == "update"` when `router_live` exists.
- `data-live-room="name"` — every tab joins `room:name:component`. A
  public board is one instance, not one per cookie-less socket.
- Chunked `POST /live/upload` for files over 256 KiB.
- `soli-patch`, `soli-live`, debounce/throttle, hooks, click-away,
  `soli-disable-with`, JS commands (`add_class`, `focus`, `navigate`…).
- `has_one_attached` / `has_many_attached` — disk, S3, or SolidB; a
  LiveView upload hash attaches as-is.

**What stopped being a liability**

- Closed views are released (2-minute grace, then reaped). They used to
  live for the process and keep waking on every subscribed write.
- Frames of one instance are serialized — a tick and a click no longer
  last-writer-wins each other's state.
- Events only hit the sending socket's instance.
- A raising handler reports an error; it does not increment the demo
  counter.
- Uploads are session-bound, capped (including **in-progress** chunks:
  4 per session, 32 / 64 MiB global, 2-minute idle).
- A reconnect re-arms ticks.
- Render errors no longer leak absolute paths, and are escaped.

## `unless … end` is a statement

It used to exist only as postfix, then as a desugared `if !cond` that
`soli fmt` turned into something else. It is `StmtKind::Unless` now.

```soli
unless ["up", "late", "overdue"].includes?(order.status)
  order.errors.add("status", "invalid")
end

unless cart.empty?
  checkout()
else
  redirect("/cart")
end

return cached unless cached.nil?
```

`else` is allowed. `elsif` is not. Formatter keeps `unless`. See
[Control flow](/docs/language/control-flow#kw-unless).

## Auth that does not help the attacker

- Sign-in **misses spend the same Argon2 work** as hits. Lockout copy
  waits until the password verifies. One failure message.
- **Per-IP throttle** on sign-in, sign-up, and reset (`429`). Shared
  budget: `AUTH_ATTEMPTS_PER_IP` / `AUTH_IP_WINDOW_SECONDS` (15 / 5 min).
  An existing lockout is not re-stamped. Keys on the peer unless
  `enable_trust_proxy()`.
- **`User#password_error`** owns the policy (`AUTH_MIN_PASSWORD_LENGTH`,
  default 12). Reset cannot install a password registration would refuse,
  and it drops the remember-me digest.
- **`SOLI_FORCE_SECURE_COOKIES` is the whole jar**, not just `session_id`.
  `same_site: "None"` implies `Secure`.
- **CSRF `/_` exemption is gone.** Only framework endpoints are skipped.
  Your `POST /_admin/wipe` is gated. Use `skip_csrf` when you mean it
  (Stripe webhooks: [the Stripe post](/docs/blog/stripe-checkout)).
- **`auth_base_url` reads `APP_BASE_URL`**, not `http://localhost:5011`.
- **`rate_limiter_from_ip(req, limit, window_seconds)` works** — it was
  arity-2 and rejected the documented third argument.
- **`cargo audit --deny warnings`** is a CI gate.

`soli generate oauth github|google` is on this cycle too: state CSRF,
find-or-create, session login.

## Faster where it is paid on every request

Hash overwrite no longer clones the key. `req["params"]["id"]` is one
VM opcode. Request plumbing skips work when there are no helpers, no
named routes, no middleware clock. That is the path every MVC action
walks.

## Operations

- `SOLI_LOG_FORMAT=json` — one NDJSON object per request.
- `SOLI_OTEL=1` — W3C `traceparent`, the same span tree as the dev-bar
  flamegraph, OTLP/HTTP off the request thread.
- Production default workers: **2**, not one-per-core.
- Slim Cargo features: drop `postgres` / `mysql` / `sqlite` / `paseto`
  you do not need.

See [Observability](/docs/development-tools/observability) and
[Keeping memory low](/docs/getting-started/configuration#keeping-memory-low).

## Docs that match the binary

This cycle also *removed* fiction: `assert_equal`, `soli run file.sl`,
`--no-dev`, `Math.floor`, DateTime `.add_months()`, five `/docs` URLs
that served JSON. The testing vocabulary, `soli file.sl`, and adapter
env vars on the configuration page are what the process actually does.

New writing: [Stripe Checkout](/docs/blog/stripe-checkout),
[Field Desk](/docs/blog/liveview-desk), dedicated SQL adapter pages.

## If you are upgrading from 1.29

1. Drain SolidB's old job queue before you rely on in-process jobs.
2. Re-read CSRF if you had routes under `/_`.
3. Set `SOLI_JOBS_USER` / `PASSWORD` or `SOLI_JOBS_TOKEN` if you want
   `/__soli/jobs` in production.
4. Set `APP_BASE_URL` so auth mail does not point at localhost.
5. Column-mode models: add a string `type` column before you declare STI;
   only encrypt **text** columns.

The changelog page is the ledger. This post is the map.
