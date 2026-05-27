# Sending Email with SendGrid — and Doing It from a Background Job

Email delivery is one of those features that quietly turns a synchronous request into a slow one. The user submits a signup form, your controller posts to SendGrid's API, the API takes 300 ms on a good day and several seconds on a bad one — and the user waits. The fix is not "make HTTP faster"; the fix is to take the work off the request path entirely.

This post walks through two things together: building a thin SendGrid wrapper as an `app/services/` class, then handing the delivery off to a SolidB-backed background job so the controller returns immediately. By the end you will have a `SendGrid.send_mail(...)` you can call from anywhere, an `EmailJob` you can enqueue with one line, and a clear mental model of why those two pieces belong on opposite sides of the queue.

<figure style="margin:1.5rem auto;max-width:1024px;">
  <img src="/images/blog/sendgrid-jobs-flow.jpg" width="1024" height="576" alt="Email delivery flow in Soli: controller enqueues EmailJob via perform_later to SolidB queue; background job later executes and calls SendGrid API while the user request has already completed." style="display:block;width:100%;height:auto;border-radius:12px;border:1px solid #30363d;background:#0b0d0f;">
  <figcaption style="text-align:center;color:#8b949e;font-size:0.875rem;margin-top:0.5rem;">Keep the user request fast. Move the slow email work into a background job.</figcaption>
</figure>

## The Shape of the Solution

Three files and a route do the whole job:

```
app/services/sendgrid.sl             # thin wrapper around the v3 Messages API
app/jobs/email_job.sl                # static perform — receives args, calls SendGrid
app/controllers/users_controller.sl  # enqueues EmailJob.perform_later(...)
```

The controller never touches SendGrid. It validates input, persists the user, enqueues an email, and returns. The job runs out-of-band on a worker triggered by SolidB and is the only piece that talks to api.sendgrid.com.

## Step 1: A Thin SendGrid Wrapper

Wrap the SendGrid v3 Messages API once, in a place you can reuse from any job, controller, or one-off script. Drop it in `app/services/` — Soli autoloads every `.sl` file under that directory at boot, so the class is available everywhere without an `import`.

```soli
# app/services/sendgrid.sl
class SendGrid
  SEND_URL = "https://api.sendgrid.com/v3/mail/send"

  static def send_mail(to, subject, body, from = nil)
    api_key = getenv("SENDGRID_API_KEY")
    if api_key == nil or api_key == ""
      raise("SENDGRID_API_KEY is not set")
    end

    sender = from
    if sender == nil or sender == ""
      sender = getenv("SENDGRID_FROM") || "no-reply@example.com"
    end

    payload = {
      "personalizations": [{"to": [{"email": to}]}],
      "from": {"email": sender},
      "subject": subject,
      "content": [{"type": "text/plain", "value": body}]
    }

    response = HTTP.post_json(SEND_URL, payload, {
      "headers": {
        "Authorization": "Bearer #{api_key}",
        "Content-Type": "application/json"
      }
    })

    # SendGrid returns 202 Accepted on success, with an empty body.
    if response["status"] >= 200 and response["status"] < 300
      return {"ok": true, "status": response["status"]}
    end

    {"ok": false, "status": response["status"], "body": response["body"]}
  end
end
```

A few things to notice:

- **`getenv("SENDGRID_API_KEY")` is read inside `send_mail`, not at module load.** That means changing the key in `.env` and hot-reloading does the right thing in dev. It also means each call pays for one cheap env lookup — trivial compared to the HTTP round trip.
- **`HTTP.post_json` does the JSON serialization and `Content-Type` for us.** We still set `Authorization` explicitly because that's the API contract.
- **A 202 with an empty body is the success case.** Treating any 2xx as `ok: true` keeps the wrapper boring; the caller decides what to do with non-2xx.
- **No retry logic in the library.** Retries belong to the queue, not to the library. SolidB will re-fire the job if the handler raises or returns a 5xx — letting the wrapper stay thin.

Because `app/services/` is autoloaded, you call `SendGrid.send_mail(...)` from any controller, job, or script without an `import` line.

## Step 2: The Email Job

Now the asynchronous half. Soli's background-job system loads every class under `app/jobs/`, registers a static `perform(args)` entry point, and gives you `perform_later`, `perform_now`, `perform_in`, and `perform_at` for free. The full mechanics live in [/docs/jobs](/docs/jobs); the short version is:

```soli
# app/jobs/email_job.sl
class EmailJob
  static def perform(args)
    to      = args["to"]
    subject = args["subject"]
    body    = args["body"]
    from    = args["from"]

    result = SendGrid.send_mail(to, subject, body, from)

    if !result["ok"]
      # Raising re-surfaces a 5xx back to SolidB, which schedules a retry.
      raise("SendGrid rejected #{to}: #{result["status"]} #{result["body"]}")
    end

    result
  end
end
```

Two design choices worth flagging:

1. **`args` is a plain hash.** Job arguments round-trip through JSON via SolidB's queue, so primitive types only — no model instances, no closures, no dates as `Time` objects. Pass IDs and strings; rehydrate inside `perform` if you need the record.
2. **Failure is communicated by raising.** Soli's `/_jobs/run/:name` route turns an uncaught exception into a `500`. SolidB sees the 500, increments `attempt`, and re-enqueues per its retry policy. There's no separate "tell the queue I failed" call — the absence of a normal return is the signal.

## Step 3: Enqueue from a Controller

This is the line the user actually waits for:

```soli
# app/controllers/users_controller.sl
class UsersController
  def create
    user = User.create(params["user"])
    if user._errors
      return {"status": 422, "json": {"errors": user._errors}}
    end

    EmailJob.perform_later({
      "to":      user.email,
      "subject": "Welcome to Acme",
      "body":    "Hi #{user.name}, your account is ready."
    })

    redirect("/users/#{user.id}")
  end
end
```

`perform_later` calls `Job.enqueue("EmailJob", args)` under the hood, which POSTs the job to SolidB's `/_api/database/{db}/queues/{queue}/enqueue` endpoint. SolidB acks the enqueue in a few milliseconds and the controller returns. Whatever SendGrid is doing — and however many seconds it spends doing it — happens out of sight on a worker that gets called back by SolidB later.

Want to delay the send instead? Same API, different verb:

```soli
EmailJob.perform_in("2 minutes", {"to": user.email, "subject": "...", "body": "..."})
EmailJob.perform_at("2026-06-01T08:00:00Z", {"to": user.email, "subject": "...", "body": "..."})
```

Want to run it inline for a test? `EmailJob.perform_now({...})` skips the queue entirely and calls `perform` in-process.

## Step 4: Why SolidB Owns the Queue

The job system uses SolidB for three things that would otherwise be three different pieces of infrastructure:

- **Durable storage of pending work.** When SolidB acks the enqueue, the job survives a Soli restart, a crash, and a redeploy. The user does not have to retry signup just because you shipped a hotfix.
- **Scheduling.** `perform_in` and `perform_at` are queue inserts with a `not_before` timestamp. SolidB releases them when their time arrives — no second Cron daemon, no `at` job, no `setTimeout`.
- **Retries.** A 5xx from the callback tells SolidB to re-fire after a backoff. You declare the retry policy on the SolidB side; Soli's contract is "return a status code." This is exactly the boundary you want: the language runtime doesn't pretend to know how many times to retry the API.

Configure it with three env vars (`SOLI_JOBS_DATABASE`, `SOLI_JOBS_CALLBACK_URL`, `SOLI_JOBS_SECRET`) and the system bootstraps itself. The full table — including the HMAC-SHA256 signing scheme that protects the `/_jobs/run/:name` route from being called by anything other than SolidB — is in [/docs/jobs#configuration](/docs/jobs#configuration).

## Why This Pattern Holds Up

Two boundaries make this clean:

- **The library doesn't know about the queue.** `SendGrid.send_mail` is a synchronous function. It works in a controller, a script, a `soli console` session, or a test. Pulling SendGrid into a separate file like this means a future "send a password reset" feature wires up the same wrapper into a `PasswordResetJob` without copying API code.
- **The controller doesn't know about SendGrid.** It enqueues an email by *intent* (to, subject, body), not by transport. If you swap SendGrid for SES tomorrow, you touch `app/services/sendgrid.sl` and `EmailJob`. The controller and the SignupForm don't care.

That separation is what lets the request return in 10 ms while the email still goes out a second later — the user gets their redirect, the email shows up, and you never had to put SendGrid's latency into your SLO.

## What's Next

A few directions to extend this:

- **Templates.** Replace the plain-text body with SendGrid dynamic templates — pass `"template_id"` and `"dynamic_template_data"` instead of `"content"` in the payload. The wrapper grows by ~10 lines and your controllers stop building HTML strings.
- **Per-queue routing.** `EmailJob.set(queue: "mailers").perform_later(...)` puts welcome emails and transactional alerts on different SolidB queues so a backlog in one doesn't starve the other.
- **Idempotency.** SolidB hands you `args["job_id"]` and `args["attempt"]` on retries. Stamp the user record with `welcome_email_sent_at` after a successful send and short-circuit on attempt > 1 if the field is already set.

The wrapper, the job, and the queue are intentionally small. Adding any of the above is changing one of the three files — not rewiring the architecture.
