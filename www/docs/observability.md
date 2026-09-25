# Observability

Soli ships three production signals out of the box: **metrics**, **structured logs**, and **distributed traces**. All are opt-in so a quiet process pays nothing until you turn a channel on. Alongside them, **error tracking** groups every failed request into a triage page inside the app, **slow-query tracking** does the same for database queries over a threshold, and **notifications** tell someone by webhook, email or a job of your own.

| Signal | Enable | Where it goes |
|--------|--------|---------------|
| Metrics | `SOLI_METRICS=1` | Prometheus text at `GET /_metrics` |
| Logs | `SOLI_LOG=…` (+ optional `SOLI_LOG_FORMAT=json`) | stdout / stderr |
| Traces | `SOLI_OTEL=1` or `OTEL_EXPORTER_OTLP_*` | OTLP/HTTP JSON to your collector |
| Health | always on | `GET /_health`, `GET /_ready` |
| Errors | on (`SOLI_ERRORS=off` to stop) | `_soli_errors` table, shown at `/__soli/errors` |
| Slow queries | on at 200 ms (`SOLI_SLOW_QUERY_MS`, `SOLI_SLOW_QUERIES=off`) | `_soli_slow_queries` table, shown at `/__soli/slow_queries` |
| Notifications | `SOLI_NOTIFY_WEBHOOKS` / `SOLI_NOTIFY_EMAILS` / a `SoliNotificationJob` | Slack, Teams, Discord, Google Chat, any URL, email, your job |

For the full env-var table see [Configuration](configuration.md). This page is the operator guide: what each signal means, how to turn it on, and how the pieces correlate.

## Quick start

```bash
APP_ENV=production \
SOLI_METRICS=1 \
SOLI_LOG=access \
SOLI_LOG_FORMAT=json \
OTEL_EXPORTER_OTLP_ENDPOINT=http://otel-collector:4318 \
OTEL_SERVICE_NAME=myapp \
OTEL_RESOURCE_ATTRIBUTES=deployment.environment=production \
soli serve
```

Boot prints a one-line banner for each signal that is active:

```text
Using hyper async HTTP server with 2 worker threads (production default — …)
OpenTelemetry tracing enabled → http://otel-collector:4318/v1/traces (service.name=myapp)
Production logs: JSON (SOLI_LOG_FORMAT=json)
```

## Health and readiness

Always available — nothing to enable. Use them for load balancers and orchestrators:

| Endpoint | Meaning | Answers |
|----------|---------|---------|
| `GET /_health` | **Liveness** — is this process alive? | `200 ok` for as long as the server runs, *including while it shuts down* |
| `GET /_ready` | **Readiness** — should traffic be routed here? | `200 ready`, or `503 starting` / `503 draining` |

Point liveness probes at `/_health` and readiness probes at `/_ready`. On `SIGTERM` readiness fails first so the LB stops routing, in-flight requests finish, then the process exits (bounded by `SOLI_SHUTDOWN_GRACE_SECS`, default 25s). See [Configuration → Health checks](configuration.md#health-checks-and-graceful-shutdown).

## Metrics (`/_metrics`)

Collection is opt-in via `SOLI_METRICS=1` (or `true`). Until that is set, counters stay at zero and the hot path skips the per-operation clocks.

```bash
SOLI_METRICS=1 soli serve
curl -s localhost:5011/_metrics
```

Representative series (all Prometheus text format):

| Metric | Meaning |
|--------|---------|
| `soli_http_requests_total` | Requests handled |
| `soli_lexing_duration_seconds` / `_count` | Time in the lexer |
| `soli_parsing_duration_seconds` / `_count` | Time in the parser |
| `soli_vm_execution_seconds` / `_count` | Bytecode VM wall time |
| `soli_template_render_duration_seconds` / `_count` | Views, layouts, partials |
| `soli_middleware_duration_seconds` / `_count` | Middleware totals |
| `soli_db_query_duration_seconds` / `_count` | SoliDB / SolidB query time |
| `soli_vm_handler_demotions_total` | Handlers that fell back from the VM to the tree-walker (cached per worker). `SOLI_ENGINE_LOG=1` prints one line per unique handler; `SOLI_FAIL_ON_VM_DEMOTION=1` exits the process when the VM *refuses* a handler, so CI cannot ship a new refuse. The bytecode VM only runs outside `--dev`, so neither applies to `soli serve --dev` or `soli test`. |
| `soli_handler_panics_total` | Panics contained by the per-request `catch_unwind` (client got 500; worker stayed up) |

`soli_handler_panics_total` and `soli_vm_handler_demotions_total` are counted even when `SOLI_METRICS` is off — rare enough that the atomics are free, and most wanted when nobody thought to enable collection in advance.

Scrape from Prometheus / Grafana Alloy / Datadog agent like any other text exposition endpoint. Access: with `SOLI_METRICS_TOKEN` set, the endpoint wants `Authorization: Bearer <token>`. Unset, it answers only loopback and private-range peers — and **refuses (404) any request carrying `X-Forwarded-For`, `X-Real-IP` or `Forwarded`, or any request while `trust_proxy` is on**, since behind a reverse proxy every peer looks local. A deployment behind a proxy must set `SOLI_METRICS_TOKEN` and configure the scraper to send it.

## Structured logs

### Channels (`SOLI_LOG`)

Comma-separated list. Any detail channel implies `access` so the block has a request line to hang off.

| Channel | What it prints |
|---------|----------------|
| `access` | One line per request: method, path, status, handler ms (+ queue wait) |
| `query` | AQL with binds + duration (secret-looking bind *names* redacted) |
| `http` | Outgoing `HTTP.*` calls (credential-like query params redacted) |
| `kv` | SoliKV / Cache commands |
| `timing` | Middleware / view / phase breakdown |
| `all` | Everything |

Legacy: `SOLI_REQUEST_LOG=1` is an alias for `access`.

```bash
# Access only
SOLI_LOG=access soli serve

# Full per-request breakdown (noisy — prefer slow mode in prod)
SOLI_LOG=query,http,timing soli serve
```

### Slow requests (`SOLI_SLOW_REQUEST_MS`)

Emit the full detail block only when queue wait + handler time crosses a threshold. Fast requests stay silent unless you also asked for explicit channels.

```bash
SOLI_SLOW_REQUEST_MS=100 soli serve
```

### Format (`SOLI_LOG_FORMAT`)

| Value | Output |
|-------|--------|
| `text` (default) | Multi-line human blocks, historical default |
| `json` | One NDJSON object per event on stdout (errors on stderr) |

```bash
SOLI_LOG=access SOLI_LOG_FORMAT=json soli serve
```

Example access line:

```json
{
  "ts": "2026-08-09T12:00:00.123Z",
  "level": "info",
  "msg": "request",
  "method": "GET",
  "path": "/users",
  "status": 200,
  "duration_ms": 4.2,
  "total_ms": 4.2,
  "request_id": "…",
  "trace_id": "…",
  "span_id": "…"
}
```

With detail channels (or a slow hit) the same object grows nested `db` / `http` / `kv` / `timing` arrays. Production errors use `level: "error"` and `msg: "request_error"` with redacted request snapshot, stack, and env.

Ship stdout/stderr to Loki, CloudWatch, Datadog, Elastic, etc. No file rotation in-process — use your supervisor or container log driver.

## Distributed tracing (OpenTelemetry)

Soli does **not** pull in the heavyweight OTel SDK. It reuses the hierarchical span tree already built for the dev-bar flamegraph and exports it as OTLP/HTTP JSON.

### Enable

Any of:

```bash
# Local collector sidecar (defaults to http://127.0.0.1:4318/v1/traces)
SOLI_OTEL=1 soli serve

# Explicit collector
OTEL_EXPORTER_OTLP_ENDPOINT=http://otel-collector:4318 \
OTEL_SERVICE_NAME=myapp \
soli serve

# Full traces URL (overrides the base)
OTEL_EXPORTER_OTLP_TRACES_ENDPOINT=http://otel-collector:4318/v1/traces \
soli serve
```

| Variable | Role | Default |
|----------|------|---------|
| `SOLI_OTEL` | Force tracing on (`1` / `true` / `yes`) | unset |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | Collector base URL; enables tracing | unset |
| `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` | Full `/v1/traces` URL | derived from base |
| `OTEL_SERVICE_NAME` | `service.name` resource attribute | `soli` |
| `OTEL_RESOURCE_ATTRIBUTES` | Extra `key=value` pairs, comma-separated | unset |
| `OTEL_SDK_DISABLED` | Force off when `true` | unset |

### What is exported

Per request:

1. A **SERVER** root span (`GET /path`) with `http.request.method`, `url.path`, `http.response.status_code`, `soli.request_id`.
2. Nested **INTERNAL** / **CLIENT** spans for middleware, before/after actions, controller actions, views/partials/components, DB queries, and outgoing HTTP — the same tree the flamegraph shows under `--dev`.

Export is **async** on a dedicated background thread. A full queue drops batches rather than stalling web workers; the first drop and the first POST failure print a one-time warning on stderr.

### W3C Trace Context

- Inbound `traceparent` is parsed and becomes the parent of the root span.
- Every response carries `traceparent` so gateways and clients can correlate.
- When tracing is on, responses also get `X-Request-Id` (unless `X-Soli-Request-Id` was already set in `--dev`).

### Log ↔ trace joins

Turn on both:

```bash
SOLI_LOG=access SOLI_LOG_FORMAT=json SOLI_OTEL=1 soli serve
```

JSON access lines include `trace_id` and `span_id` matching the exported root span. In Grafana / Datadog / Jaeger UI, jump from a log line to the full span tree.

### Sampling

Soli always samples when tracing is enabled (flags bit `0x01`). Configure sampling, batching, and retention on the collector (Grafana Tempo, Jaeger, Datadog agent, OpenTelemetry Collector, …) rather than in the Soli process.

## Error tracking (`/__soli/errors`)

Every request that ends in a 500 is recorded, grouped, and shown at `/__soli/errors` — a self-hosted, built-in stand-in for Sentry. There is no service to sign up for and no SDK: it is on by default and writes to the app's own database (SoliDB, Postgres, MySQL or SQLite), in a `_soli_errors` table.

**Grouping.** Each failure gets a fingerprint from its message and the frame that raised it. Numbers, ids, UUIDs and quoted values are stripped from the message first, and the line number from the frame, so `Order 42 not found` and `Order 97 not found` are one group, and editing code above the bug does not start a new one. Double quotes always quote; a single quote quotes only at the start of a word, so the apostrophe in `can't` or `User's` stays part of the message rather than swallowing the words after it. File paths are stored relative to the app root, so deploying to another directory keeps the history.

**What a group holds.** Count, first and last seen, and the five newest occurrences, each with its stack, the request, the handler's local variables, and a `curl` line that replays it against a local server. Auth headers (`Authorization`, `Cookie`, `X-Api-Key` and any header whose name looks secret), secret-looking keys at any depth (`password`, `token`, `api_key`, …) and the raw request body are replaced by `[REDACTED]` before anything is stored — in the request, in the handler's locals, and in the data passed to `render()` when a template fails mid-render — the same redaction the stderr error log uses. Key names match whatever the separator, so `api-key`, `apiKey` and `x-api-key` count as `api_key`. The replay `curl` line never carries `Authorization`, `Cookie` or API-key headers, nor a secret-named query param. Redaction goes by field name: a field called `card` is not recognised as a secret, so don't put card numbers in forms you do not control.

**Triage.** A group is `open`, `resolved` or `ignored`. **resolve** moves it out of the open list; if it fails again it comes back **regressed**. **ignore** keeps counting in the background without listing it. **delete** forgets it.

**Cost.** Recording never slows the request: the sample is handed to a background writer over a bounded queue and written in one-second batches, one update per group. If errors arrive faster than they can be written, the extra samples are dropped and the page says how many. The page reports each recording fault separately: samples dropped because the queue was full, samples whose write to the database failed, and restarts of the writer — a panic inside one write is caught and counted as a failed write, and a writer that stopped anyway is started again on the next error.

**Access.** Same gate as `/__soli/jobs`. In `--dev` the page is open to the machine it runs on (loopback, local host name) and linked from the dev bar's tools panel. Anyone else — every request in production — needs credentials, and with none configured the path answers `404`:

```bash
SOLI_ERRORS_USER=ops
SOLI_ERRORS_PASSWORD=<long random string>
SOLI_ERRORS_TOKEN=<long random string>   # Authorization: Bearer … for scripts

# or one set for both /__soli/jobs and /__soli/errors
SOLI_ADMIN_USER=ops
SOLI_ADMIN_PASSWORD=<long random string>
SOLI_ADMIN_TOKEN=<long random string>
```

The triage buttons are same-origin form posts: a cross-site `POST` is refused with `403` even though the browser would attach the Basic credentials.

**Turning it off.** `SOLI_ERRORS=off` stops recording (the page still lists what is there). Under `APP_ENV=test` it is off unless `SOLI_ERRORS=on`, so spec runs do not fill the table.

**Limits.** Only HTTP request failures are recorded — job failures stay on `/__soli/jobs`, and LiveView/EUI event errors are not captured yet. To be told when something fails, see [Notifications](#notifications). Counts are exact within one process; several hosts writing the same group at the same moment can undercount it. Groups are kept until you delete them.

**Retention.** An app keeps at most **1000 groups** (plus one overflow group). Once that many exist, an occurrence whose fingerprint is not already stored is counted in a single *overflow* group instead of starting a new one, with its own message kept in the sample; groups already stored keep counting as usual. The list says when the limit has been reached. Deleting groups makes room again — resolving or ignoring them does not. Each group keeps its five newest samples and 24 hours of hourly counts.

## Slow queries (`/__soli/slow_queries`)

Every database query that takes `SOLI_SLOW_QUERY_MS` (default **200 ms**) or longer is recorded, grouped by shape, and shown at `/__soli/slow_queries`. It covers every query the ORM runs — SoliDB over HTTP or the native driver, and the Postgres, MySQL and SQLite adapters — in requests and in background jobs, and writes to the app's own database in a `_soli_slow_queries` table. On by default, like error tracking.

**Grouping.** A query's *shape* is the query with its literals taken out: numbers become `?`, a list of values becomes one `?`, whitespace collapses, and in SDBQL quoted strings become `?` too. So `FILTER u.id == 42` and `FILTER u.id == 97` are one group, and `IN (1, 2, 3)` is the same shape as `IN (7)`. In the adapters' SQL, a quoted name is an identifier or a JSON key the adapter wrote (`doc ->> 'tag'`) — values are always binds — so those are kept, and a filter on `tag` stays apart from a filter on `name`. Placeholders (`@name`, `$1`, `?`) are kept as written.

**What a group holds.** How many slow runs, their average, slowest and total time, the request or job that ran the last one (`GET /orders → orders#index`, `job ReportJob`), first and last seen, 24 hours of hourly counts, and the **five slowest runs**, each with the query as it ran, its bind values and where it came from. Bind values under a secret-looking name (`password`, `token`, …) are replaced by `[REDACTED]` and long values are cut to 200 characters; `SOLI_SLOW_QUERY_BINDS=off` stores no bind values at all. SQL binds are numbered (`$1`, `?`), so their names say nothing — turn binds off if your queries filter on personal data you do not want in the table.

**Reading the list.** Four orders: **impact** (total time spent — the default, and usually where to start), **slowest** (the single worst run), **frequent** (most slow runs) and **recent**. **delete** forgets a shape; it comes back on its next slow run.

**Cost.** A fast query pays one comparison: the query text and its binds are only read and copied once the query is over the threshold. Slow runs go to a background writer over a bounded queue and are written in one-second batches, one update per shape — the same machinery as error tracking, with the same notices when samples are dropped or a write fails. The writer's own queries, and any query on the framework's `_soli_*` tables, are never recorded.

**Access.** Same gate as `/__soli/errors`: open to this machine in `--dev` (and linked from the dev bar's tools panel), and `404` in production unless credentials are set:

```bash
SOLI_SLOW_QUERIES_USER=ops
SOLI_SLOW_QUERIES_PASSWORD=<long random string>
SOLI_SLOW_QUERIES_TOKEN=<long random string>

# or the shared set, accepted by every operator page
SOLI_ADMIN_USER=ops
SOLI_ADMIN_PASSWORD=<long random string>
```

**Turning it off.** `SOLI_SLOW_QUERIES=off` stops recording; under `APP_ENV=test` it is off unless `SOLI_SLOW_QUERIES=on`. The threshold is read once per process — change it and restart.

**Limits.** The time measured is the whole round trip seen from Soli — network, queueing in the pool, the database's own work — not the database's execution time alone. It says *which* query is slow, not why: run it with `EXPLAIN` on the database. At most **1000 shapes** are kept per app; slow runs of a new shape past that are counted on the page but not stored. Raw `db_query()` strings that inline different identifiers are different shapes.

## Notifications

Errors and slow queries can tell someone instead of waiting to be looked at. Four events are sent, each after its group is stored:

| Event | When |
|-------|------|
| `error.new` | an error whose fingerprint has never been seen |
| `error.regressed` | an error you marked **resolved** fails again |
| `error.spike` | one error group reaches `SOLI_NOTIFY_SPIKE` occurrences within a window (default `50/5m`); not sent for **ignored** groups |
| `slow_query.new` | a query shape crosses `SOLI_SLOW_QUERY_MS` for the first time |

**Where they go.** Set any of these; every event goes to all of them:

```bash
# Slack, Microsoft Teams, Discord, Google Chat or any URL — comma-separated
SOLI_NOTIFY_WEBHOOKS=https://hooks.slack.com/services/T000/B000/XXXX,https://acme.webhook.office.com/webhookb2/...

# Addresses, sent through the app's own mailer (SOLI_SMTP_*)
SOLI_NOTIFY_EMAILS=oncall@example.com,cto@example.com
SOLI_NOTIFY_FROM=alerts@example.com        # default: SOLI_SMTP_FROM

# Links in messages point here (default: https:// + the first SOLI_APP_HOSTS)
SOLI_NOTIFY_URL=https://shop.example.com
```

Webhooks are recognised by host and sent the message each product expects: Slack incoming webhooks (`hooks.slack.com`) get formatted text with a link, Microsoft Teams (`*.webhook.office.com`, and Workflows URLs on `*.logic.azure.com` / `*.powerplatform.com`) an Adaptive Card with an **Open in Soli** button, Discord and Google Chat a text message. Any other URL receives the event as JSON:

```json
{
  "event": "error.new",
  "app": "shop",
  "fingerprint": "25ea121f5ce571ea",
  "summary": "Cannot access property 'total' on null at 14:3",
  "detail": "raised in boom at app/controllers/items_controller.sl:14",
  "context": "GET /boom",
  "count": 1,
  "at": "2026-09-25T07:39:58Z",
  "url": "https://shop.example.com/__soli/errors/25ea121f5ce571ea"
}
```

with an `X-Soli-Event` header, and — when `SOLI_NOTIFY_SECRET` is set — `X-Soli-Signature`, the hex HMAC-SHA256 of the body under that secret. Webhook URLs go through the same SSRF guard as `Webhook.enqueue`; a receiver on a private address has to be allowed with `SOLI_HTTP_ALLOW_HOSTS`. In `--dev` without an SMTP host, emails land in the dev inbox at `/__soli/inbox`.

**Anything else, in Soli.** If the app has `app/jobs/soli_notification_job.sl`, every event is also enqueued to it with the same hash as the JSON above — for PagerDuty, an SMS, a ticket, or a filter of your own:

```soli
# app/jobs/soli_notification_job.sl
class SoliNotificationJob
  static def perform(event)
    return unless event["event"] == "error.spike"
    HTTP.post("https://events.pagerduty.com/v2/enqueue", {
      "routing_key": getenv("PAGERDUTY_KEY"),
      "event_action": "trigger",
      "payload": {"summary": event["summary"], "source": event["app"], "severity": "error"}
    })
  end
end
```

**Not every time.** One event per group is sent at most once per `SOLI_NOTIFY_THROTTLE` (default `15m`; `90s`, `1h` also work), so a burst of the same failure is one message. `SOLI_NOTIFY_EVENTS=error.new,error.regressed` narrows which events are sent. `SOLI_NOTIFY_SPIKE=off` turns spike detection off. `SOLI_NOTIFY_APP_NAME` names the app in messages (default: its directory name).

**Cost.** Sending happens on a per-app notifier thread with a bounded queue: a slow or unreachable webhook never delays a request or the trackers. A failed send is logged as a `[notify]` line on stderr and counted on the errors and slow-queries pages, which also say where notifications currently go.

**Limits.** Throttling and spike windows are counted per process, so several hosts can each send the same event once. A failed send is not retried (use the job for that — jobs retry). There is no per-user routing or on-call schedule: that is what the job hook, or the tool on the other end of the webhook, is for.

## Dev vs production

| | `--dev` | Production |
|--|---------|------------|
| Dev bar (queries, flamegraph, replay) | on | off |
| Access log | always on (terminal) | `SOLI_LOG` / `SOLI_REQUEST_LOG` |
| JSON format | available | available |
| Span tree | flamegraph | OTLP when OTEL on |
| Metrics | opt-in | opt-in |
| Health endpoints | on | on |
| Error tracking (`/__soli/errors`) | on, open to this machine | on, behind `SOLI_ERRORS_*` / `SOLI_ADMIN_*` |
| Slow queries (`/__soli/slow_queries`) | on, open to this machine | on, behind `SOLI_SLOW_QUERIES_*` / `SOLI_ADMIN_*` |
| Notifications | when `SOLI_NOTIFY_*` is set (emails go to the dev inbox without SMTP) | when `SOLI_NOTIFY_*` is set |

Production logging reuses the same channel buffers as the dev bar (`query`, `http`, `kv`, `timing`) without paying for hot-reload, the bar injection, or the interpreter demotion that `--dev` implies.

## Limits (honest)

- No auto-instrumentation of every third-party client library — only Soli's own request path, ORM, and `HTTP.*` client.
- OTLP export is **traces only** (not metrics or logs pipelines). Metrics stay on Prometheus `/_metrics`; logs stay on stdout.
- Outbound `traceparent` injection on every `HTTP.*` call is not yet automatic; inbound propagation and response echo are.
- No in-process sampling UI — put that on the collector.

## See also

- [Configuration](configuration.md) — full env-var reference
- [Debugging](/docs/development-tools/debugging) — dev bar, flamegraph, breakpoints
- [Deploy](deploy.md) — shipping the binary
- [How Soli Compares](/docs/getting-started/comparison) — ops posture vs Rails / Phoenix / Laravel / Django
