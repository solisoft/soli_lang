# Background Jobs and Cron

Soli ships a background-job and cron system that runs **inside the Soli process**. Define a handler class in `app/jobs/`, enqueue it from your controllers or models, and Soli's job engine stores it, claims it, runs it on a worker thread, retries it on failure, and fires your cron schedules.

> **Storage.** Jobs are ordinary documents in a `_jobs` collection (and `_cron_jobs` for schedules) on your default database connection, so the engine works the same on SolidB, PostgreSQL, MySQL, and SQLite. On a SQL connection the queue indexes itself on first enqueue (`state`, `run_at`, `priority`), so the claim query does not scan the table on every poll tick. Nothing calls back into your app over HTTP — no callback URL, no inbound route, no shared secret required.

## Defining a Job

Create a file under `app/jobs/`. The filename and class name follow the same convention as controllers and models — `email_job.sl` defines `class EmailJob`.

```soli
# app/jobs/welcome_email_job.sl
class WelcomeEmailJob {
  static def perform(args: Hash) {
    user = User.find(args["user_id"]);
    Mailer.send(user.email, "Welcome to the app");
  }
}
```

Every job class must define a `static def perform(args: Hash)`. That's the entry point the engine calls when the job runs.

## Enqueueing Jobs (Facade-style)

Job classes get a set of static helpers automatically — you don't need to inherit from anything:

```soli
# Enqueue. A worker claims it on the next poll and runs it.
WelcomeEmailJob.perform_later({ "user_id": 42 });

# Schedule for later.
WelcomeEmailJob.perform_in("5 minutes", { "user_id": 42 });
WelcomeEmailJob.perform_at("2026-05-01T08:00:00Z", { "user_id": 42 });

# Run it right here, right now — no queue, no worker. Useful in specs.
WelcomeEmailJob.perform_now({ "user_id": 42 });

# Pick a non-default queue: pass its name as the trailing argument.
WelcomeEmailJob.perform_later({ "user_id": 42 }, "mailers");

# Pass an options hash to set the queue *and* priority (higher runs first).
WelcomeEmailJob.perform_later({ "user_id": 42 }, { "queue": "mailers", "priority": 10 });
```

Every enqueue helper (`perform_later`, `perform_in`, `perform_at`) takes the same optional trailing argument — either a queue-name **string** or an **options hash**:

| Key           | Type   | Purpose                                            |
|---------------|--------|----------------------------------------------------|
| `queue`       | String | Queue name (defaults to `SOLI_JOBS_DEFAULT_QUEUE`) |
| `priority`    | Int    | Higher executes first                              |
| `max_retries` | Int    | Retry budget (defaults to `SOLI_JOBS_MAX_RETRIES`) |

Duration strings accept `seconds`, `minutes`, `hours`, `days`, `weeks` (and the singular/abbreviated forms — `s`, `min`, `hr`, `d`, `wk`). Numeric values are interpreted as seconds.

`perform_now` runs the handler synchronously in the calling process and returns whatever `perform` returns. It writes no queue row, so it works with no database and no worker pool — which is what makes it the right tool in a test.

## Low-level API

If you'd rather not use the per-class facade:

```soli
job_id = Job.enqueue("WelcomeEmailJob", { "user_id": 42 });
Job.enqueue_in("WelcomeEmailJob", "30 minutes", { "user_id": 42 });
Job.enqueue_at("WelcomeEmailJob", "2026-05-01T08:00:00Z", { "user_id": 42 });

# The trailing queue argument is also a string or an options hash, exactly
# like the facade helpers.
Job.enqueue("WelcomeEmailJob", { "user_id": 42 }, "mailers");
Job.enqueue("WelcomeEmailJob", { "user_id": 42 }, { "queue": "mailers", "priority": 10 });

Job.cancel(job_id);              # true when removed; raises if already running
Job.retry(job_id);               # failed/dead → pending; keeps attempts + last_error
jobs = Job.list("default");      # rows in the "default" queue (omit for all)
queue_names = Job.queues();      # queues with non-terminal work
```

`Job.cancel` only applies to work that hasn't started (`scheduled`, `pending`, `failed`). A `running` job holds a worker and a lease, so cancelling it raises rather than pretending to stop it.

`Job.retry` puts a `failed` or `dead` row back on the queue immediately. It keeps `attempts` and `last_error` so the dashboard still shows why it died. Other states raise.

## Webhook Jobs (Arbitrary URLs)

Sometimes the work you want to enqueue isn't a Soli class — it's a POST to a third-party API (Slack, Stripe, an internal service), or a webhook to some other system on a delay. The `Webhook` class enqueues jobs whose target is a URL. The engine fires the HTTP request itself, with the same retry policy as any other job.

```soli
# Fire immediately
Webhook.enqueue("https://hooks.slack.com/services/T00/B00/abc", {
  "text": "Order #1234 shipped"
});

# Delay 5 minutes
Webhook.enqueue_in(
  "https://api.example.com/order-completed",
  "5 minutes",
  { "order_id": 1234 }
);

# At a specific time
Webhook.enqueue_at(
  "https://api.example.com/daily-summary",
  "2026-05-01T08:00:00Z",
  { "report": "daily" }
);
```

The `opts` hash (last argument) accepts:

| Key            | Type    | Purpose                                                                   |
|----------------|---------|---------------------------------------------------------------------------|
| `queue`        | String  | Queue name (defaults to `SOLI_JOBS_DEFAULT_QUEUE`)                        |
| `priority`     | Int     | Higher executes first                                                     |
| `max_retries`  | Int     | Retry budget                                                              |
| `secret`       | String  | Per-job HMAC key (overrides `SOLI_WEBHOOK_SECRET`)                        |
| `headers`      | Hash    | Extra HTTP headers attached to the outgoing request                       |

```soli
Webhook.enqueue(
  "https://api.partner.test/event",
  { "kind": "user.created", "user_id": user.id },
  {
    "queue": "external",
    "priority": 10,
    "secret": getenv("PARTNER_HMAC_SECRET"),
    "headers": { "Authorization": "Bearer " + getenv("PARTNER_TOKEN") }
  }
);
```

When the job fires, Soli POSTs the payload as JSON with these headers:

- `Content-Type: application/json`
- `X-Webhook-Event: job`
- `X-Webhook-Delivery: <job-id>`
- `X-Webhook-Signature: <lowercase hex HMAC-SHA256(body, secret)>` — present when a secret is configured (per-job `secret`, else `SOLI_WEBHOOK_SECRET`)
- Plus any `headers` you supply

Non-2xx responses count as failure and are retried with the same exponential backoff as class-target jobs. `Webhook.cancel(id)` and `Webhook.list(queue)` operate on the same `_jobs` collection as `Job`.

## Cron (Recurring Jobs)

Schedule a recurring job by passing a cron expression to `Cron.schedule` or by declaring it on the class. Soli evaluates the expressions and fires the schedules itself.

### Imperative

```soli
Cron.schedule("nightly_report", Cron.daily_at("03:00"), "ReportJob", {});
Cron.schedule("warm_cache",     Cron.every("5 minutes"), "WarmCacheJob", {});
Cron.list();
Cron.update("nightly_report", { "cron_expression": "0 0 4 * * *" });
Cron.delete("nightly_report");
```

`Cron.schedule` is **idempotent**. Calling it twice with the same name updates the existing entry rather than creating a duplicate, so it's safe to call from a boot script. `Cron.update` and `Cron.delete` take the schedule **name** (the same one you passed to `schedule`).

An invalid expression is rejected by `Cron.schedule` / `Cron.update` with an error naming the expected shape — a schedule that could never fire is never stored.

### Convention (declarative)

A class can declare a `static cron`. On boot, worker 0 upserts a cron entry named after the class:

```soli
class NightlyReportJob {
  static cron: String = Cron.daily_at("03:00");

  static def perform(args: Hash) {
    Report.generate();
  }
}
```

The auto-derived cron name is the snake-case of the class (`nightly_report_job`). To remove a static-cron schedule, delete the field and call `Cron.delete(name)` once — Soli does not auto-delete to avoid surprise data loss.

### Cron expression helpers

Soli uses **six-field** cron expressions: `sec min hour day-of-month month day-of-week`. That leading seconds field is what distinguishes them from five-field Unix crontab lines — pasting a five-field line is an error, not a silently-never-firing schedule.

| Helper                                    | Cron string         |
|-------------------------------------------|---------------------|
| `Cron.every("5 minutes")`                 | `0 */5 * * * *`     |
| `Cron.every("1 hour")`                    | `0 0 * * * *`       |
| `Cron.every("2 hours")`                   | `0 0 */2 * * *`     |
| `Cron.every("1 day")`                     | `0 0 0 */1 * *`     |
| `Cron.hourly()`                           | `0 0 * * * *`       |
| `Cron.daily_at("03:00")`                  | `0 0 3 * * *`       |
| `Cron.weekly_at("monday", "09:00")`       | `0 0 9 * * Mon`     |

You can always pass a raw six-field cron string instead.

## Configuration

Set these env vars (typically in `.env`):

| Variable                  | Purpose                                                                       | Default   |
|---------------------------|-------------------------------------------------------------------------------|-----------|
| `SOLI_JOBS_DEFAULT_QUEUE` | Queue name when none is supplied                                              | `default` |
| `SOLI_JOB_WORKERS`        | Worker threads that run job code; `0` disables the engine in this process     | `1`       |
| `SOLI_JOBS_POLL_MS`       | How often the poller looks for due work (milliseconds)                        | `1000`    |
| `SOLI_JOBS_LEASE_SECS`    | Lease length; a claimed job is reclaimable this long after its last heartbeat  | `60`      |
| `SOLI_JOBS_MAX_RETRIES`   | Default retry budget per job                                                  | `3`       |
| `SOLI_JOBS_RETENTION_SECS`| How long completed rows are kept before pruning                               | `604800`  |
| `SOLI_WEBHOOK_SECRET`     | Default HMAC key for **outgoing** `Webhook.*` deliveries                      | unset     |
| `SOLI_JOB_VIEW_HELPERS`   | Set `0` to skip loading view helpers/i18n into job interpreters (saves memory) | enabled   |

The engine starts with `soli serve` when the app has jobs — `app/jobs/` exists, or a mailer is configured (so `deliver_later` works). `SOLI_JOB_WORKERS=0` disables it in that process; enqueued rows wait for a process that does run workers — typically `soli jobs` (see [Standalone worker](#standalone-worker)).

There is **no** callback URL and **no** required secret any more. `SOLI_JOBS_CALLBACK_URL`, `SOLI_JOBS_SECRET`, and `SOLI_JOBS_DATABASE` are no longer read; `SOLI_WEBHOOK_SECRET` now only signs outgoing webhook deliveries.

## How Dispatch Works

1. `WelcomeEmailJob.perform_later(args)` writes a row to `_jobs` with `state: "pending"` and `run_at: now`.
2. The poller thread claims due rows atomically — Postgres `FOR UPDATE SKIP LOCKED`, MySQL a token claim, SQLite the database write lock (`BEGIN IMMEDIATE`), SolidB an `If-Match` compare-and-swap — stamping each with `state: "running"`, a `locked_until` lease, and an incremented `attempts`.
3. Claimed jobs go to the worker pool, where a fully-loaded interpreter (models, services, mailers, templates) calls `WelcomeEmailJob.perform(args)`.
4. The worker reports the outcome: success marks the row `done`; a raised error, a panic, or a non-2xx webhook response marks it `failed` with the next retry time, or `dead` once the retry budget is spent.

Because claiming is atomic, several `soli serve` **or** `soli jobs` processes can share one database and one queue without ever running the same job twice concurrently. Cron works the same way: the poller claims a due schedule with a compare-and-swap on its `next_run_at`, so exactly one process enqueues each occurrence.

Job code never runs on a web-server worker, so a slow handler cannot delay request serving.

## Retries and Crash Recovery

- **Backoff** is exponential from 5 seconds, doubling per attempt, capped at 1 hour, with a small per-job spread so a burst of sibling failures doesn't retry in lockstep.
- **`attempts` increments at claim time**, not at completion. A worker that dies mid-job therefore counts the lost attempt.
- **Leases recover crashes.** A `running` row whose `locked_until` has passed is claimable again, so a job whose process was killed is picked up by the next poller instead of being stranded. The poller heartbeats the leases of in-flight jobs each tick, so a long job is not reclaimed while it is still running.
- **At-least-once semantics.** A job that outlives its lease (longer than `SOLI_JOBS_LEASE_SECS` without a heartbeat, e.g. a hard-frozen process) can be picked up elsewhere. **Write handlers to be idempotent**, and raise `SOLI_JOBS_LEASE_SECS` if your jobs are long-running.
- **Completed rows are pruned** after `SOLI_JOBS_RETENTION_SECS`; `failed` and `dead` rows are kept for inspection.

## Standalone worker

`soli serve` can run the poller next to HTTP. To scale job capacity separately — or to keep a long job from sharing a process with request serving — set `SOLI_JOB_WORKERS=0` on the web process and run a dedicated worker:

```bash
soli jobs                  # current directory, SOLI_JOB_WORKERS or 1 slot
soli jobs ./myapp --workers 4
soli worker ./myapp        # alias
```

The worker loads models, services, mailers, and `app/jobs/`, registers `static cron` schedules, then claims and runs work until Ctrl-C / SIGTERM. It opens no HTTP port. Several workers can share one queue; claiming is still atomic.

Inspect and operate the queue without starting HTTP:

On SolidB, `_jobs` is a privileged collection. A normal app JWT is not enough — set `SOLIDB_API_KEY` (or `SOLIDB_USERNAME` / `SOLIDB_PASSWORD` for an admin). On loopback with no credentials configured, Soli tries SolidB's bootstrap `admin` / `admin`, same as the native driver.

```bash
soli jobs list                        # every queue
soli jobs list --queue mailers
soli jobs list --state failed
soli jobs retry <id>
soli jobs cancel <id>
```

The same operations are a page at `/__soli/jobs` (linked from the dev-bar tools panel in `--dev`). Cancel pending/scheduled/failed rows; retry failed/dead ones.

In production the dashboard is off unless you set credentials. Configure **HTTP Basic**:

```bash
SOLI_JOBS_USER=ops
SOLI_JOBS_PASSWORD=a-long-random-secret
```

or a **bearer token** (`Authorization: Bearer …`):

```bash
SOLI_JOBS_TOKEN=a-long-random-secret
```

Both can be set; either method is accepted. Without either pair, `/__soli/jobs` answers `404` so the route does not advertise itself. A wrong password answers `401` with `WWW-Authenticate: Basic realm="Soli jobs"`. `--dev` stays open (no prompt). The CLI (`soli jobs list` / `retry` / `cancel`) and `Job.*` still work without the dashboard.

A LiveView that enqueues an upload job and refreshes when it finishes is walked through in [A Live Field Desk](/docs/blog/liveview-desk).

## Long-Running Jobs

Nothing special is required: every job already runs on the worker pool, off the request path, with no timeout and with retries. Size the pool with `SOLI_JOB_WORKERS` — each worker holds its own loaded interpreter (models, services, mailers, jobs), so it costs memory as well as concurrency.

`static background: Bool = true` is still accepted for compatibility but has no effect — it described the old opt-out from running jobs on a web worker, which is now the default for every job. It can be removed from your job classes.

## Testing Jobs

`perform_now` runs a handler inline with no queue and no database, which is the simplest way to test job logic:

```soli
test("welcome email job sends mail", fn() {
  WelcomeEmailJob.perform_now({ "user_id": user.id });
  assert_eq(Mailer.deliveries().length(), 1);
});
```

`soli test` runs no poller, so `perform_later` in a test writes a row that nothing claims. Assert on the enqueue (`Job.list(...)`) or call `perform_now` to exercise the handler.

## Idempotency and Registration Notes

- **Cron upsert.** `Cron.schedule(name, ...)` is keyed by `name`: same name updates, never duplicates. Only worker 0 performs `static cron` auto-registration at boot, and the row key is the schedule name, so concurrent boots converge instead of racing.
- **`_jobs` lives on the default connection.** Per-model `connection "name"` routing does not apply to the queue itself.

## Hot Reload

In `--dev` mode, editing a file under `app/jobs/` reloads the class without restarting the server, like controllers and models. A job already executing finishes on the code it started with; the next job picks up the new code.

## Worker Convention Notes

- Filename and class name must match (`email_job.sl` ↔ `EmailJob`). A mismatch is a startup error.
- `perform` must be `static` — it's invoked on the class, never on an instance.
- Job arguments round-trip through JSON; pass plain hashes/arrays/strings/numbers, not class instances.
- A handler that isn't loaded fails the job (and retries), rather than marking it done — so a rename mid-deploy recovers once the new code is live.

## Migrating from the SolidB-callback engine

Earlier releases delegated the queue, schedule, and retry policy to SolidB, which POSTed a signed webhook back to `/_jobs/run/:name`. That inbound route, `SOLI_JOBS_CALLBACK_URL`, and the required callback secret are gone. To upgrade:

1. Let SolidB's internal queue drain before deploying (in-flight jobs there are invisible to the new engine).
2. Remove `SOLI_JOBS_CALLBACK_URL` and `SOLI_JOBS_SECRET`; keep `SOLI_WEBHOOK_SECRET` only if you use `Webhook.*`.
3. Drop any firewall or ingress rule that let SolidB reach your app — the app no longer receives job traffic.
4. Your job classes, `Job.*`, `Webhook.*`, `Cron.*` calls, and `static cron` declarations need no changes.

## See Also

- [Multiple Databases](multi-database.md) — the queue works on any configured adapter.
- [Sessions](sessions.md) — same pluggable-backend pattern, different domain.
