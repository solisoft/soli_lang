# Background Jobs & Cron in Soli: Queues, Secure Callbacks, Retries, and Recurring Work Without the Ops Tax

Sending a welcome email, generating a nightly report, or processing a CSV import inside the same HTTP request that the user is waiting on is one of the fastest ways to make a web app feel slow and brittle. The user submits the form, your controller calls an external API, the network hiccups for 1.2 seconds (or 8 seconds on a bad day), and the progress spinner spins while the user wonders if the page is broken.

The right answer is not "make the external service faster." The right answer is to take the work completely off the request path — and to do it with a primitive that is as boring and reliable as the rest of your stack.

Soli ships a complete background job and cron system backed by SolidB. You define a plain class with a `static def perform(args)`, call `MyJob.perform_later(...)` (or `perform_in`, `perform_at`), and SolidB handles durable queuing, scheduling, retries, and delivery via signed HTTP callbacks back into your application. No Redis, no Sidekiq, no separate worker daemon to package, deploy, and babysit — just the database you were already running for your primary data.

This post walks through the architecture, the practical APIs (both the beautiful facade and the low-level escape hatches), the security model that makes the callback route safe, real production patterns (idempotency, deduplication using `job_id` + `attempt`, graceful degradation with the new postfix `rescue` operator), cron and recurring work, hot reload during development, how the dev bar makes the entire async flow visible, and a concrete starter you can copy-paste tonight.

<figure style="margin:1.5rem auto;max-width:1024px;">
  <img src="/images/blog/background-jobs-cron.jpg" width="1024" height="576" alt="Soli background jobs architecture diagram: a controller enqueues work via perform_later into SolidB, which later delivers a signed HTTP callback (X-Job-Signature) to the Soli server that invokes your Job.perform handler. No extra daemons required." style="display:block;width:100%;height:auto;border-radius:12px;border:1px solid #30363d;background:#0b0d0f;">
  <figcaption style="text-align:center;color:#8b949e;font-size:0.875rem;margin-top:0.5rem;">The complete flow: fast enqueue → durable scheduling in SolidB → signed callback execution in normal Soli code.</figcaption>
</figure>

## The Shape of the Solution

Three pieces work together. None of them is exotic.

1. **SolidB** is the durable queue and scheduler. It stores jobs and cron entries as ordinary documents, handles delay, cron evaluation, retry/backoff, and the "call this URL when ready" contract.
2. **Your Soli application** owns the business logic. You drop handler classes in `app/jobs/`. At boot Soli discovers every `*_job.sl` file, loads the class, and automatically injects the ergonomic `perform_later` / `perform_in` / `set(queue: ...)` methods. It also registers one special route (`POST /_jobs/run/:name`) that SolidB will call later.
3. **A signed webhook callback** is the bridge. When SolidB decides a job should run, it POSTs a JSON payload (containing `args`, plus `job_id` and `attempt` on retries) to the callback URL you configured. Soli verifies an HMAC-SHA256 signature using a shared secret, looks up the class by name, and invokes `perform`. Success (200) or failure (5xx) tells SolidB what to do next.

The controller that enqueues the job returns in a few milliseconds. The actual work — and any number of retries — happens later, on whatever schedule and retry policy SolidB applies.

A realistic end-to-end welcome-email flow needs only three files plus the one-time configuration of `SOLI_JOBS_SECRET`:

```
app/services/mailer.sl
app/jobs/welcome_email_job.sl
app/controllers/users_controller.sl
```

We'll build the full version, add a recurring nightly report job declared entirely on the class, and then go deep on the parts that actually matter when you have 50 jobs and a production incident at 3 a.m.

## Architecture Flow (What Actually Happens)

Here is the concrete sequence for a delayed or immediate job:

```
Controller
   |
   +--> WelcomeEmailJob.perform_later({ user_id: 123 })
            |
            +--> Job.enqueue(...)  (normal SolidB HTTP call)
                     |
                     v
                 SolidB queue (durable, with retry policy)
                     |
                     | (5 minutes later, or immediately)
                     v
                 SolidB worker decides to fire the job
                     |
                     +--> POST https://yourapp.example.com/_jobs/run/WelcomeEmailJob
                              Headers: X-Job-Signature: <hmac-of-body>
                              Body: {"args": {"user_id": 123}, "job_id": "...", "attempt": 1}
                     |
                     v
                 Your Soli app (any worker)
                     |
                     +--> __soli_jobs_run (the built-in prelude)
                              - check SOLI_JOBS_SECRET
                              - constant-time secure_compare on the signature
                              - lookup class by name via __soli_get_class
                              - call WelcomeEmailJob.perform(args)
                     |
                     +--> 200 (success) or 500 (handler raised) returned to SolidB
```

Cron follows the identical callback path; only the trigger is different (time-based instead of explicit enqueue).

The beauty of the design is that the "worker" that executes your job code is just your normal Soli web server. There is no separate process image, no special job-only deployment artifact, and no new language runtime to keep patched.

## Defining a Job Handler

Job files live in `app/jobs/` and follow the same naming convention as everything else:

- `welcome_email_job.sl` → `class WelcomeEmailJob`
- `import_products_job.sl` → `class ImportProductsJob`

```soli
# app/jobs/welcome_email_job.sl
class WelcomeEmailJob {
    static def perform(args: Hash) {
        user_id = args["user_id"]
        user = User.find(user_id)

        Mailer.send_welcome(user)
    }
}
```

`perform` must be `static`. Arguments are plain data that survived a JSON round-trip through SolidB. Never pass model instances, closures, or `Time` objects — pass IDs and primitive values, then rehydrate inside the handler.

At load time Soli injects a set of ergonomic static methods onto every job class (unless you have already defined your own with the same name):

- `perform_now(args)` — execute immediately in the current process. Excellent for development, one-off scripts, and tests.
- `perform_later(args, queue?)` — the 90% case. Enqueue for async execution on the default (or named) queue.
- `perform_in(duration, args, queue?)` — schedule relative to now (`"5 minutes"`, `900`, `"2 hours"`).
- `perform_at(iso8601_string, args, queue?)` — schedule for an absolute UTC time.
- `set(queue: "mailers")` — returns a tiny builder so you can write `WelcomeEmailJob.set(queue: "mailers").perform_later(...)`.

You can also stay at the lower level with the global `Job` and `Cron` classes:

```soli
id = Job.enqueue("WelcomeEmailJob", { "user_id": 42 })
Job.enqueue_in("WelcomeEmailJob", "30 minutes", { "user_id": 42 })
Job.enqueue_at("WelcomeEmailJob", "2026-06-01T08:00:00Z", { "user_id": 42 })
Job.cancel(id)
queued = Job.list("default")
queues = Job.queues()
```

## A Real Service Wrapper + Job (SendGrid Example)

Most teams already have thin service objects. Turning one into a background job is mechanical.

First, the reusable mailer (lives in `app/services/` so it is autoloaded everywhere):

```soli
# app/services/mailer.sl
class Mailer {
    static SEND_URL = "https://api.sendgrid.com/v3/mail/send"

    static def send_welcome(user) {
        api_key = getenv("SENDGRID_API_KEY")
        if api_key == null or api_key == "" {
            raise("SENDGRID_API_KEY is not set")
        }

        from = getenv("SENDGRID_FROM") || "no-reply@example.com"

        payload = {
            "personalizations": [{"to": [{"email": user.email}]}],
            "from": {"email": from},
            "subject": "Welcome to Acme",
            "content": [{"type": "text/plain", "value": "Hi " + user.name + ", your account is ready."}]
        }

        response = HTTP.post_json(Mailer.SEND_URL, payload, {
            "headers": {
                "Authorization": "Bearer " + api_key,
                "Content-Type": "application/json"
            }
        })

        if response["status"] >= 200 and response["status"] < 300 {
            return {"ok": true, "status": response["status"]}
        }

        {"ok": false, "status": response["status"], "body": response["body"]}
    }
}
```

Now the job that actually calls it:

```soli
# app/jobs/welcome_email_job.sl
class WelcomeEmailJob {
    static def perform(args: Hash) {
        user_id = args["user_id"]
        user = User.find(user_id)

        result = Mailer.send_welcome(user)

        if !result["ok"] {
            # Raising (or returning 5xx from the callback) tells SolidB to retry.
            raise("SendGrid rejected " + user.email + ": " + str(result["status"]))
        }

        result
    }
}
```

Note the deliberate thinness: no retry logic inside the job. Retries are the queue's job. The handler's only responsibility is "do the work or tell the queue you failed."

## Enqueuing from Controllers (and Everywhere Else)

The user-visible moment stays fast:

```soli
# app/controllers/users_controller.sl
class UsersController {
    def create(req) {
        params = req["json"]
        user = User.create(params["user"])

        if user._errors {
            return {"status": 422, "body": json_stringify({"errors": user._errors})}
        }

        WelcomeEmailJob.perform_later({ "user_id": user.id })

        redirect("/welcome")
    }
}
```

You can call the same methods from models (after-save callbacks), from other jobs, from the REPL, or from a CLI script. The facade methods are just ordinary static methods on the class.

## The Security Model — Signed Callbacks Done Right

The route `POST /_jobs/run/:name` is intentionally dangerous: by design it can invoke any class that has a `perform` method. Therefore it must be extremely well protected.

If `SOLI_JOBS_SECRET` is unset, Soli never registers the route at all (worker 0 prints a loud warning). When the secret *is* present, every single callback must carry a valid signature.

The actual dispatcher (a small prelude injected into every worker) is worth reading in full because it is the security contract:

```soli
def __soli_jobs_run(req) {
    secret = getenv("SOLI_JOBS_SECRET")
    if secret == null or secret == "" {
        return {"status": 503, "body": "Job dispatcher disabled: SOLI_JOBS_SECRET not set"}
    }

    provided_sig = req["headers"]["x-job-signature"] ?? ""
    raw_body = req["body"] ?? ""
    expected_sig = hmac(raw_body, secret)

    if !secure_compare(provided_sig, expected_sig) {
        return {"status": 401, "body": "Invalid signature"}
    }

    name = req["params"]["name"]
    cls = __soli_get_class(name)
    if cls == null {
        return {"status": 503, "body": "Job class not loaded: " + str(name)}
    }

    payload = req["json"]
    job_args = payload != null ? (payload["args"] ?? {}) : {}

    try {
        cls.perform(job_args)
        return {"status": 200, "body": "ok"}
    } catch err {
        print("Job " + str(name) + " failed: " + str(err))
        return {"status": 500, "body": "job error: " + str(err)}
    }
}
```

Important details that the tests explicitly protect:

- Header lookup is lowercase (`x-job-signature`) because the HTTP library normalizes header names.
- `secure_compare` performs a constant-time comparison so an attacker cannot use timing to guess the secret.
- The body that is signed is the *raw* request body (before any parsing).
- A missing class returns 503 (SolidB will retry later — useful during deploys).
- Any exception inside `perform` becomes a 500 → SolidB retries.

SolidB (or any client you write yourself for testing) must compute the identical HMAC-SHA256 hex digest of the exact bytes it sends and place it in the `X-Job-Signature` header. Soli's `Job.enqueue` family does this for you automatically when talking to SolidB.

## Delayed Execution and the Duration Parser

The duration language used by both `perform_in` and several `Cron.*` helpers is intentionally small and obvious:

```soli
ReportJob.perform_in("5 minutes", { "kind": "daily" });
CleanupJob.perform_in(3600, { "scope": "sessions" });     # raw seconds also work
BillingJob.perform_at("2026-06-01T02:00:00Z", { "month": "2026-05" });
```

Supported units (singular, plural, and abbreviations all work):

- seconds: `s`, `sec`, `secs`, `second`, `seconds`
- minutes: `m`, `min`, `mins`, `minute`, `minutes`
- hours: `h`, `hr`, `hrs`, `hour`, `hours`
- days: `d`, `day`, `days`
- weeks: `w`, `wk`, `week`, `weeks`

The parser is implemented in a few dozen lines in the job builtin and is covered by the test suite.

## Cron — Recurring Work Without a Separate Scheduler

You have two equivalent styles.

### Imperative registration (anywhere)

```soli
Cron.schedule("nightly_report", Cron.daily_at("03:00"), "NightlyReportJob", {});
Cron.schedule("warm_cache", Cron.every("5 minutes"), "WarmCacheJob", { "scope": "all" });
```

`Cron.schedule` is idempotent keyed by the human name you choose. Calling it twice with the same name updates the schedule rather than creating a duplicate. Perfect for boot-time or migration scripts.

### Declarative (on the job class)

```soli
# app/jobs/nightly_report_job.sl
class NightlyReportJob {
    static cron = Cron.daily_at("03:30")

    static def perform(args: Hash) {
        date = args["date"] ?? datetime_now().format("%Y-%m-%d")
        # ... build and store the report ...
    }
}
```

On boot, *only worker 0* walks the loaded job classes, reads any `static cron` field, and upserts the corresponding SolidB cron entry using the snake-cased class name as the schedule name (`nightly_report_job`). This guarantees a single source of truth without race conditions between workers.

The expression helpers produce standard five-field cron strings:

| Expression                              | Resulting cron     |
|-----------------------------------------|--------------------|
| `Cron.every("5 minutes")`               | `*/5 * * * *`      |
| `Cron.every("2 hours")`                 | `0 */2 * * *`      |
| `Cron.hourly()`                         | `0 * * * *`        |
| `Cron.daily_at("03:00")`                | `0 3 * * *`        |
| `Cron.weekly_at("monday", "09:00")`     | `0 9 * * 1`        |
| `Cron.weekly_at("fri", "17:30")`        | `30 17 * * 5`      |

You can always pass a raw cron string if the helpers don't cover your case.

## Production Patterns

### Idempotency Using job_id + attempt

When SolidB retries a job it includes `job_id` and `attempt` in the callback payload (visible as `req["json"]["job_id"]` and `req["json"]["attempt"]` inside your `perform` if you look at the full payload instead of just `args`).

A common pattern:

```soli
static def perform(args) {
    full = ...   # the top-level JSON from the callback
    job_id = full["job_id"]
    attempt = full["attempt"] ?? 1

    if already_handled(job_id) {
        return   # safe re-delivery
    }

    # do the work
    mark_handled(job_id)
}
```

External services that support idempotency keys should receive the `job_id`.

### Using the Postfix `rescue` Operator

The new `expr rescue fallback` syntax is delightful inside jobs:

```soli
api_key = getenv("STRIPE_SECRET") rescue null
if api_key == null {
    print("Stripe disabled in this environment — job is a no-op")
    return
}

result = risky_call() rescue {"success": false, "error": "network"}
```

It keeps the main line readable while making the fallback explicit and local.

### Observability — The Dev Bar Sees Both Halves

Because an enqueue is just an ordinary outbound HTTP call from your app to SolidB, and the later execution is an ordinary inbound request, the dev bar shows the complete picture:

- In the original user request you see the `Job.enqueue` (or `perform_later`) span with its latency.
- Minutes or hours later you see a completely separate request trace for the callback itself, including whatever database work or downstream HTTP calls the job performed.

This is vastly more useful than "something happened in a background queue somewhere."

### Hot Reload During Development

Editing any file under `app/jobs/` while the server is running with `--dev` causes that class to be reloaded. The next callback that arrives for that job name executes the new code. No restart, no redeploy, no lost state in the queue.

## Configuration Reference

| Variable                    | Meaning                                                                 | Default                              |
|-----------------------------|-------------------------------------------------------------------------|--------------------------------------|
| `SOLI_JOBS_DATABASE`        | SolidB database used for queues and cron                                | `SOLIDB_DATABASE` or `default`       |
| `SOLI_JOBS_DEFAULT_QUEUE`   | Queue name when none supplied                                           | `default`                            |
| `SOLI_JOBS_CALLBACK_URL`    | Base URL SolidB will POST to (must be reachable from the SolidB host)   | `http://127.0.0.1:3000/_jobs/run`    |
| `SOLI_JOBS_SECRET`          | **Required in production.** Long random value used for HMAC signatures. | unset (the route is not registered)  |

In development the localhost default is usually fine. In production set the callback URL to a publicly (or SolidB-reachable) address and ensure the secret is strong and rotated like any other signing key.

## Comparison with the Traditional Approach

Teams coming from Rails + Sidekiq or Laravel + queues are used to a separate daemon, a separate Redis/SQS connection, separate retry semantics, and a separate deployment story. The operational surface area is real.

Soli + SolidB collapses that surface:

- One database technology to operate.
- One set of credentials and one connection pool.
- Job handlers are normal Soli code — they see the same models, the same dev bar, the same everything.
- The "worker" is your existing web server processes.

The conscious trade-off is that SolidB is now on the critical path for both synchronous requests and background work. For the vast majority of applications that were already using SolidB as the primary store, this is a net simplification, not an added risk.

## Testing Jobs

`perform_now` makes unit-testing the handler trivial:

```soli
test("welcome email job sends the right thing", fn() {
    user = Factory.create("user")
    WelcomeEmailJob.perform_now({ "user_id": user.id })
    # assert on whatever your mailer records or on the HTTP mock
})
```

For full integration tests that go through the queue you can still use the normal test server + `with_session`, but most teams find that exercising `perform_now` (plus a few manual end-to-end enqueues against a test SolidB) is sufficient and dramatically faster.

## A Complete, Copy-Pasteable Starter

A recurring report job declared entirely on the class, plus an ad-hoc entry point:

```soli
# app/jobs/nightly_report_job.sl
class NightlyReportJob {
    static cron = Cron.daily_at("03:30")

    static def perform(args: Hash) {
        date = args["date"] ?? datetime_now().format("%Y-%m-%d")

        # In a real handler you would look at the full callback payload
        # for job_id / attempt and implement a cheap dedup check here.

        rows = DB.query("SELECT ... FROM orders WHERE date = @d", { "d": date })
        csv = to_csv(rows)

        Storage.put("reports/" + date + ".csv", csv)
    }
}
```

Manual trigger from anywhere:

```soli
NightlyReportJob.perform_later({ "date": "2026-05-20" })
# or
NightlyReportJob.perform_in("10 minutes", { "date": "2026-05-20" })
```

## Where to Go Next

- The canonical reference: [/docs/jobs](/docs/jobs) — exhaustive method tables and configuration.
- The SendGrid email + background job post — a complete, thin service wrapper turned into a production job.
- The event-streaming post — shows the job system being used as a durable consumer for an external `es` topic.

Background work stops being a source of dread the moment the framework gives you a boring, observable, signed primitive and gets out of the way. In Soli that primitive is literally just another class with a `perform` method.

Pick the one job that has been the biggest source of latency or flakiness in your current stack. Move it into `app/jobs/`, call `perform_later` from the controller, watch both halves light up in the dev bar, and enjoy the sudden feeling that your request handlers are fast again.

Your users — and the person who gets paged at 3 a.m. — will thank you.
