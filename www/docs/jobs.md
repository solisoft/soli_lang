# Background Jobs and Cron

Soli ships with a SolidB-backed background-job and cron system. Define a handler class in `app/jobs/`, enqueue it from your controllers or models, and SolidB takes care of scheduling, retries, and triggering the handler when it's time to run.

> **Backend.** The first version targets SolidB only. SolidB owns the queue, scheduling, and retry policy; Soli provides the language-side API and a webhook the SolidB worker calls to actually execute your code.

## Defining a Job

Create a file under `app/jobs/`. The filename and class name follow the same convention as controllers and models — `email_job.sl` defines `class EmailJob`.

```soli
# app/jobs/welcome_email_job.sl
class WelcomeEmailJob {
  static fn perform(args: Hash) {
    user = User.find(args["user_id"]);
    Mailer.send(user.email, "Welcome to the app");
  }
}
```

Every job class must define a `static fn perform(args: Hash)`. That's the entry point SolidB triggers when the job runs.

## Enqueueing Jobs (Facade-style)

Job classes get a set of static helpers automatically — you don't need to inherit from anything:

```soli
# Enqueue. SolidB picks it up and calls back to /_jobs/run/WelcomeEmailJob.
WelcomeEmailJob.perform_later({ "user_id": 42 });

# Schedule for later.
WelcomeEmailJob.perform_in("5 minutes", { "user_id": 42 });
WelcomeEmailJob.perform_at("2026-05-01T08:00:00Z", { "user_id": 42 });

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
| `max_retries` | Int    | Retry budget                                       |

Duration strings accept `seconds`, `minutes`, `hours`, `days`, `weeks` (and the singular/abbreviated forms — `s`, `min`, `hr`, `d`, `wk`). Numeric values are interpreted as seconds.

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

Job.cancel(job_id);
jobs = Job.list("default");      # jobs in the "default" queue
queue_names = Job.queues();
```

## Webhook Jobs (Arbitrary URLs)

Sometimes the work you want to enqueue isn't a Soli class — it's a POST to a third-party API (Slack, Stripe, an internal service), or a webhook to some other system on a delay. The `Webhook` class enqueues jobs whose target is a URL rather than a Soli handler. SolidB itself fires the HTTP request when the job runs.

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

When the job fires, SolidB POSTs the payload as JSON with these headers:

- `Content-Type: application/json`
- `X-Webhook-Event: job`
- `X-Webhook-Delivery: <job-id>`
- `X-Webhook-Signature: <lowercase hex HMAC-SHA256(body, secret)>` — present when a secret is configured
- Plus any `headers` you supply

Non-2xx responses count as failure and are retried with the same exponential backoff as script-target jobs. `Webhook.cancel(id)` and `Webhook.list(queue)` operate on the same underlying `_jobs` collection as `Job`.

## Cron (Recurring Jobs)

Schedule a recurring job by passing a cron expression to `Cron.schedule` or by declaring it on the class.

### Imperative

```soli
Cron.schedule("nightly_report", Cron.daily_at("03:00"), "ReportJob", {});
Cron.schedule("warm_cache",     Cron.every("5 minutes"), "WarmCacheJob", {});
Cron.list();
Cron.update(cron_id, { "schedule": "0 4 * * *" });
Cron.delete(cron_id);
```

`Cron.schedule` is **idempotent**. Calling it twice with the same name updates the existing entry rather than creating a duplicate, so it's safe to call from a boot script.

### Convention (declarative)

A class can declare a `static cron`. On boot, worker 0 upserts a cron entry named after the class:

```soli
class NightlyReportJob {
  static cron = Cron.daily_at("03:00");

  static fn perform(args: Hash) {
    Report.generate();
  }
}
```

The auto-derived cron name is the snake-case of the class (`nightly_report_job`). To remove a static-cron schedule, delete the field and call `Cron.delete(id)` once — Soli does not auto-delete to avoid surprise data loss.

### Cron expression helpers

| Helper                                    | Cron string         |
|-------------------------------------------|---------------------|
| `Cron.every("5 minutes")`                 | `*/5 * * * *`       |
| `Cron.every("1 hour")`                    | `0 * * * *`         |
| `Cron.every("2 hours")`                   | `0 */2 * * *`       |
| `Cron.every("1 day")`                     | `0 0 */1 * *`       |
| `Cron.hourly()`                           | `0 * * * *`         |
| `Cron.daily_at("03:00")`                  | `0 3 * * *`         |
| `Cron.weekly_at("monday", "09:00")`       | `0 9 * * 1`         |

You can always pass a raw cron string instead.

## Configuration

Set these env vars (typically in `.env`):

| Variable                  | Purpose                                                                 | Default                            |
|---------------------------|-------------------------------------------------------------------------|------------------------------------|
| `SOLI_JOBS_DATABASE`      | SolidB database hosting queues + cron entries                           | `SOLIDB_DATABASE` then `default`   |
| `SOLI_JOBS_DEFAULT_QUEUE` | Queue name when none is supplied                                        | `default`                          |
| `SOLI_JOBS_CALLBACK_URL`  | URL SolidB POSTs to when a Soli `Job` fires                             | `http://127.0.0.1:3000/_jobs/run`  |
| `SOLI_WEBHOOK_SECRET`     | **Required.** HMAC-SHA256 key used to sign and verify callbacks         | unset                              |
| `SOLI_JOBS_SECRET`        | Legacy alias for `SOLI_WEBHOOK_SECRET`; still accepted                  | unset                              |

The callback URL must be reachable from the SolidB server. In production set it to your Soli app's public URL plus `/_jobs/run`. Either env var alone is enough to enable signed callbacks — the dispatcher checks `SOLI_WEBHOOK_SECRET` first, then falls back to `SOLI_JOBS_SECRET`.

## Security: Signed Callbacks

The `POST /_jobs/run/:name` route dispatches to `XJob.perform(args)` on whichever class the URL names — i.e. it can call any loaded class with a static `perform`. To stop a passing client from invoking arbitrary code, every callback must carry a valid signature.

- **A webhook secret is required.** If neither `SOLI_WEBHOOK_SECRET` nor `SOLI_JOBS_SECRET` is set, Soli **does not register** the `/_jobs/run/:name` route at all. Workers log a warning at boot and SolidB callbacks will get 404s until you configure a secret.
- **Signature header.** SolidB sends `X-Webhook-Signature` (canonical) whose value is the lowercase hex HMAC-SHA256 of the raw request body, computed with the configured secret as the key. The legacy `X-Job-Signature` header is also accepted by the dispatcher for backward compatibility.
- **Constant-time check.** Soli verifies the signature with `secure_compare()` (constant-time) so callers can't probe the secret by timing 401 responses.
- **Bad signature → 401, missing class → 503, handler error → 500** — only a valid signature lets the request reach `cls.perform(args)`.

If you compute the signature yourself (e.g. for an integration test), it's:

```soli
let body = json_stringify({ "args": { "user_id": 42 } });
let sig  = hmac(body, getenv("SOLI_WEBHOOK_SECRET"));   # hex HMAC-SHA256
# POST /_jobs/run/WelcomeEmailJob with body and X-Webhook-Signature: <sig>
```

Use a long, random value for the secret (32+ bytes from a CSPRNG). Rotate it the same way you'd rotate any HMAC key — both Soli and the SolidB sender need to flip together.

## How Dispatch Works

1. `WelcomeEmailJob.perform_later(args)` calls `Job.enqueue("WelcomeEmailJob", args)`.
2. Soli POSTs the job to `/_api/database/{db}/queues/default/enqueue` on SolidB, including the configured callback URL as `webhook_url`.
3. SolidB picks up the job and POSTs `/_jobs/run/WelcomeEmailJob` on the Soli app with `{ "args": ... }` in the body, signed with `X-Webhook-Signature`.
4. Soli's built-in handler verifies the signature, looks up `WelcomeEmailJob` in the loaded class registry, and calls `WelcomeEmailJob.perform(args)`.
5. Soli replies `200 ok` on success or `500` on error. SolidB owns the retry policy.

For cron, the flow is identical except step 3 is triggered by the cron schedule rather than an explicit enqueue. For `Webhook.enqueue(...)`, steps 3–4 collapse: SolidB POSTs directly to the URL you supplied — there is no Soli-side dispatcher to invoke.

## Idempotency and Retries

- **Cron upsert.** `Cron.schedule(name, ...)` is keyed by `name`. Same name = update, never duplicate. Worker 0 is the only worker that performs cron auto-registration on boot, to avoid races between workers.
- **Job retries.** SolidB owns retry semantics. Soli's callback returns 5xx on handler error so SolidB will retry. Treat handlers as idempotent where possible.
- **`job_id` and `attempt`.** When SolidB retries, it includes an `attempt` count and `job_id` in the callback body. Read them from `req["json"]` if you need de-dup logic.

## Hot Reload

In `--dev` mode, editing a file under `app/jobs/` reloads the class without restarting the server, like controllers and models. Existing enqueued jobs continue to use the new code on their next callback.

## Worker Convention Notes

- Filename and class name must match (`email_job.sl` ↔ `EmailJob`). A mismatch is a startup error.
- `perform` must be `static` — it's invoked on the class, never on an instance.
- Job arguments round-trip through JSON via SolidB; pass plain hashes/arrays/strings/numbers, not class instances.
- A handler that's not yet loaded responds `503` to the callback so SolidB retries instead of failing.

## See Also

- [Background Jobs & Cron deep dive (blog)](/docs/blog/background-jobs-and-cron) — architecture, security model, production patterns, and real examples.
- [SolidB queues and cron API](https://solidb.solisoft.net/docs/) — the underlying server-side primitives.
- [Sessions](sessions.md) — same pluggable-backend pattern, different domain.
