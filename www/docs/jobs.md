# Background Jobs and Cron

Soli ships with a SolidB-backed background-job and cron system. Define a handler class in `app/jobs/`, enqueue it from your controllers or models, and SolidB takes care of scheduling, retries, and triggering the handler when it's time to run.

> **Backend.** The first version targets SolidB only. SolidB owns the queue, scheduling, and retry policy; Soli provides the language-side API and a webhook the SolidB worker calls to actually execute your code.

## Defining a Job

Create a file under `app/jobs/`. The filename and class name follow the same convention as controllers and models — `email_job.sl` defines `class EmailJob`.

```soli
// app/jobs/welcome_email_job.sl
class WelcomeEmailJob {
    static fn perform(args: Hash) {
        let user = User.find(args["user_id"]);
        Mailer.send(user.email, "Welcome to the app");
    }
}
```

Every job class must define a `static fn perform(args: Hash)`. That's the entry point SolidB triggers when the job runs.

## Enqueueing Jobs (Facade-style)

Job classes get a set of static helpers automatically — you don't need to inherit from anything:

```soli
// Run inline, in the current process. No queue, no callback.
WelcomeEmailJob.perform_now({ "user_id": 42 });

// Enqueue. SolidB picks it up and calls back to /_jobs/run/WelcomeEmailJob.
WelcomeEmailJob.perform_later({ "user_id": 42 });

// Schedule for later.
WelcomeEmailJob.perform_in("5 minutes", { "user_id": 42 });
WelcomeEmailJob.perform_at("2026-05-01T08:00:00Z", { "user_id": 42 });

// Pick a non-default queue.
WelcomeEmailJob.set(queue: "mailers").perform_later({ "user_id": 42 });
```

Duration strings accept `seconds`, `minutes`, `hours`, `days`, `weeks` (and the singular/abbreviated forms — `s`, `min`, `hr`, `d`, `wk`). Numeric values are interpreted as seconds.

## Low-level API

If you'd rather not use the per-class facade:

```soli
let job_id = Job.enqueue("WelcomeEmailJob", { "user_id": 42 });
Job.enqueue_in("WelcomeEmailJob", "30 minutes", { "user_id": 42 });
Job.enqueue_at("WelcomeEmailJob", "2026-05-01T08:00:00Z", { "user_id": 42 });
Job.cancel(job_id);
let jobs = Job.list("default");      // jobs in the "default" queue
let queue_names = Job.queues();
```

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

| Variable                  | Purpose                                       | Default                            |
|---------------------------|-----------------------------------------------|------------------------------------|
| `SOLI_JOBS_DATABASE`      | SolidB database hosting queues + cron entries | `SOLIDB_DATABASE` then `default`   |
| `SOLI_JOBS_DEFAULT_QUEUE` | Queue name when none is supplied              | `default`                          |
| `SOLI_JOBS_CALLBACK_URL`  | URL SolidB POSTs to when a job fires          | `http://127.0.0.1:3000/_jobs/run`  |
| `SOLI_JOBS_ALLOW_REMOTE`  | Allow non-localhost callbacks                 | `false`                            |
| `SOLI_JOBS_SECRET`        | HMAC secret (when remote allowed)             | unset                              |

The callback URL must be reachable from the SolidB server. In production set it to your Soli app's public URL plus `/_jobs/run`.

## How Dispatch Works

1. `WelcomeEmailJob.perform_later(args)` calls `Job.enqueue("WelcomeEmailJob", args)`.
2. Soli POSTs the job to `/_api/database/{db}/queues/default/enqueue` on SolidB, including the configured callback URL.
3. SolidB picks up the job and POSTs `/_jobs/run/WelcomeEmailJob` on the Soli app with `{ "args": ... }` in the body.
4. Soli's built-in handler looks up `WelcomeEmailJob` in the loaded class registry and calls `WelcomeEmailJob.perform(args)`.
5. Soli replies `200 ok` on success or `500` on error. SolidB owns the retry policy.

For cron, the flow is identical except step 3 is triggered by the cron schedule rather than an explicit enqueue.

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

- [SolidB queues and cron API](https://solidb.solisoft.net/docs/) — the underlying server-side primitives.
- [Sessions](sessions.md) — same pluggable-backend pattern, different domain.
