# Observability

Soli ships three production signals out of the box: **metrics**, **structured logs**, and **distributed traces**. All are opt-in so a quiet process pays nothing until you turn a channel on. Alongside them, an **overview** at `/__soli/` ranks what needs attention and shows app usage, **error tracking** groups every failed request into a triage page inside the app, **slow-query tracking** does the same for database queries over a threshold, and **notifications** tell someone by webhook, email or a job of your own.

| Signal | Enable | Where it goes |
|--------|--------|---------------|
| Metrics | `SOLI_METRICS=1`, `DD_AGENT_HOST` / `SOLI_STATSD`, `SOLI_OTEL_METRICS=1` | Prometheus text at `GET /_metrics`, DogStatsD to a Datadog agent, OTLP/HTTP JSON |
| Logs | `SOLI_LOG=…` (+ optional `SOLI_LOG_FORMAT=json`) | stdout / stderr |
| Traces | `SOLI_OTEL=1` or `OTEL_EXPORTER_OTLP_*` | OTLP/HTTP JSON to your collector |
| Health | always on | `GET /_health`, `GET /_ready` |
| Errors | on (`SOLI_ERRORS=off` to stop) | `_soli_errors` table, shown at `/__soli/errors` |
| Query time | on (`SOLI_QUERY_STATS=off` to stop) | `_soli_query_stats` table, shown at `/__soli/slow_queries` |
| Slow queries | on at 200 ms (`SOLI_SLOW_QUERY_MS`, `SOLI_SLOW_QUERIES=off`) | `_soli_slow_queries` table, shown at `/__soli/slow_queries` |
| Notifications | `SOLI_NOTIFY_WEBHOOKS` / `SOLI_NOTIFY_EMAILS` / a `SoliNotificationJob` / "Notify this device" | Slack, Teams, Discord, Google Chat, Campfire, any URL, email, your job, Web Push |
| Overview and usage | on (`SOLI_USAGE=off` stops the usage counts) | `/__soli/`, hourly usage in `_soli_usage` |

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

Collection is opt-in: `SOLI_METRICS=1` (or `true`), a DogStatsD target or OTLP metrics export each turn it on. Until one is set, counters stay at zero, the hot path skips the per-operation clocks, and an app metric call is validated and dropped. `/_metrics` is always served.

```bash
SOLI_METRICS=1 soli serve
curl -s localhost:5011/_metrics
```

Representative series (all Prometheus text format):

| Metric | Meaning |
|--------|---------|
| `soli_http_requests_total` | Requests handled |
| `soli_http_request_duration_seconds` | Histogram of request time, from routing to response, labelled `method` (`GET`…, anything else `OTHER`), `route` (the matched `controller#action`, `unmatched` for a 404) and `status_class` (`2xx`…). `/_metrics`, `/_health`, `/_ready` and `/up` are not counted |
| `soli_lexing_duration_seconds` | Time in the lexer |
| `soli_parsing_duration_seconds` | Time in the parser |
| `soli_vm_execution_seconds` | Bytecode VM wall time, one sample per entry into the VM (a handler, a middleware, a view, a script). Blocks, paren-less method calls and constructors run inside that entry are part of its time, not samples of their own |
| `soli_template_render_duration_seconds` | Views, layouts, partials |
| `soli_middleware_duration_seconds` | Middleware totals, per request |
| `soli_db_query_duration_seconds` | SoliDB / SolidB query time, per request |
| `soli_vm_handler_demotions_total` | Actions, controller hooks and middleware that fell back from the VM to the tree-walker (cached per worker). `SOLI_ENGINE_LOG=1` prints one line per unique handler (for a script, it prints which engine runs it and why — see [Configuration](configuration.md#script-engine)); `SOLI_FAIL_ON_VM_DEMOTION=1` exits the process when the VM *refuses* a handler, so CI cannot ship a new refuse. Views are not counted: `SOLI_ENGINE_LOG=1` also names, once each, the templates the interpreter renders and why (see [Views](views.md#which-engine-renders-a-view)). The bytecode VM only runs outside `--dev`, so neither applies to `soli serve --dev` or `soli test`. |
| `soli_handler_panics_total` | Panics contained by the per-request `catch_unwind` (client got 500; worker stayed up) |
| `soli_metrics_dropped_total` | App metric samples dropped: past a cardinality cap, or a full DogStatsD queue |

Every `*_seconds` series is a **histogram**: `_bucket{le="…"}` (cumulative), `_sum` and `_count`. The buckets are seconds — `0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1, 2.5, 5, 10` — and `SOLI_METRICS_BUCKETS="0.01,0.1,1"` replaces them (increasing, comma-separated; a bad list warns and keeps the defaults). They apply to app histograms too. A p95 in PromQL:

```promql
histogram_quantile(0.95,
  sum by (le, route) (rate(soli_http_request_duration_seconds_bucket[5m])))
```

> **Upgrading from 2.31.0 or earlier.** The duration series used to be a counter pair: the bare name was the total seconds and `_count` the count. The total is now `<name>_sum` (the bare name no longer exists), `_count` is unchanged, and `_bucket` is new. Replace `rate(soli_db_query_duration_seconds[5m])` with `rate(soli_db_query_duration_seconds_sum[5m])`.

`soli_handler_panics_total` and `soli_vm_handler_demotions_total` are counted even when `SOLI_METRICS` is off — rare enough that the atomics are free, and most wanted when nobody thought to enable collection in advance.

A handler that fails on the VM is re-run on the tree-walker, which answers the request, unless it wrote first: to the database, a job enqueued, a mail sent. The handler is demoted only when the VM was at fault:

- the VM refused it;
- the re-run answered where the VM failed (it succeeded, or raised a deliberate 404, 403, `halt` or `DomainError`), an engine divergence, also counted in `soli_vm_engine_divergences_total`, which should stay at zero.

An error both engines raise, such as a `throw`, a nil read from a missing parameter or a `JSON.parse` of a bad body, is the app's own. The request answers 500 and the handler stays on the VM, so a bad request cannot move it to the slower engine. The worker remembers it: the next requests failing with the same error are answered from the VM without a re-run, so whatever the handler did before the error (an outbound HTTP call, a cache write) runs once per request, not twice. A new error of the same handler is re-run once again, for up to eight distinct errors per handler; past that, every error of the handler is answered from the VM without a re-run, so a handler that keeps failing with new messages (one that carries request data, say) cannot double its side effects either. A handler that fails after a write, an enqueue or a mail is not re-run, since it would happen twice. Its error is the answer, and it is demoted, since nothing can tell a VM gap from the app's error. A handler that ran out of the request's time budget ([`SOLI_HANDLER_TIMEOUT_SECS`](configuration.md)) is neither re-run nor demoted: the timeout is the answer.

### App metrics

Your own counters, gauges and histograms, with the [`Metrics` class](builtins.md#metrics-class):

```soli
Metrics.increment("orders.created", tags: {"plan": order.plan})
Metrics.gauge("queue.depth", pending_jobs.length)
Metrics.histogram("cart.items", cart.items.length)
total = Metrics.time("checkout.duration", {"step": "charge"}) do
  Payments.charge(order)
end
```

They are shared by every worker and appear on `/_metrics` after the built-in series (`orders.created` is scraped as `orders_created`), go to DogStatsD and OTLP when those are set up, and are capped at 1,000 tag combinations per metric and 1,000 metric names (a sample past a cap is dropped and counted in `soli_metrics_dropped_total`). Under `soli test` they are kept even with no sink, so a spec reads them back with `Metrics.value`.

### DogStatsD (Datadog)

Set any of these and every app sample is also sent to a Datadog agent, together with each request's duration:

| Variable | Meaning |
|----------|---------|
| `SOLI_STATSD` | `host:port` (port 8125 if left out), `udp://host:port` or `unix:///path`. Wins over the two below; `SOLI_STATSD=off` turns DogStatsD off even when `DD_AGENT_HOST` is set |
| `DD_DOGSTATSD_URL` | `udp://host:port` or `unix:///var/run/datadog/dsd.socket` |
| `DD_AGENT_HOST` (+ `DD_DOGSTATSD_PORT`, default 8125) | The agent host the Datadog tracers use |
| `DD_ENV`, `DD_SERVICE`, `DD_VERSION` | Sent as `env:`, `service:`, `version:` on every line — resolved like the trace resource ([precedence](#enable)), so `OTEL_SERVICE_NAME` or `OTEL_RESOURCE_ATTRIBUTES` win when set; no tag when nothing is configured |
| `DD_TAGS` | More global tags, `team:web,region:eu` (comma- or space-separated) |

Lines follow the DogStatsD format — `orders.created:1|c|#plan:pro,env:prod,service:shop`: `increment` sends `|c`, `gauge` `|g`, `histogram` `|h`, `distribution` and `time` `|d`. Built in, one line per request: `soli.http.request.duration` (seconds, `|d`) tagged `method`, `route`, `status_class`. Workers never touch the socket: they queue each line (16,384 at most, then drop and count), and a background thread packs them into datagrams of at most 1,432 bytes (8 KB on a Unix socket). Nothing waits for, or hears back from, the agent.

On Kubernetes, the agent's host IP usually arrives as `DD_AGENT_HOST` through the downward API, so the same variable that points the tracer at the agent turns DogStatsD on.

### OTLP metrics

`SOLI_OTEL_METRICS=1` (or `OTEL_METRICS_EXPORTER=otlp`) starts a background thread that POSTs every metric — built-in and app — as OTLP/HTTP JSON every `OTEL_METRIC_EXPORT_INTERVAL` ms (default 60000). Counters are cumulative monotonic sums, gauges gauges, the `*_seconds` series and app histograms explicit-bucket histograms, all cumulative since the process started; built-in names are dotted (`soli.http.request.duration`, unit `s`; `soli.http.requests`).

| Variable | Meaning |
|----------|---------|
| `OTEL_EXPORTER_OTLP_METRICS_ENDPOINT` | Full URL. Otherwise `OTEL_EXPORTER_OTLP_ENDPOINT` + `/v1/metrics`, otherwise `http://127.0.0.1:4318/v1/metrics` |
| `OTEL_EXPORTER_OTLP_HEADERS`, `OTEL_EXPORTER_OTLP_METRICS_HEADERS` | `key=value,key2=value2`, parsed exactly as for traces and logs; the metrics list wins on the same key (case-insensitive). Never logged — put the API key here (`DD-API-KEY=…` for Datadog's OTLP intake) |
| `OTEL_METRIC_EXPORT_INTERVAL`, `OTEL_METRIC_EXPORT_TIMEOUT` | Milliseconds (defaults 60000 and 10000); without `OTEL_METRIC_EXPORT_TIMEOUT`, a set `OTEL_EXPORTER_OTLP_TIMEOUT` applies |
| `OTEL_SERVICE_NAME`, `OTEL_RESOURCE_ATTRIBUTES`, `DD_SERVICE` / `DD_ENV` / `DD_VERSION` | The resource — the very one traces and logs carry, with the same [precedence](#enable) |

`OTEL_SDK_DISABLED=true` or `SOLI_OTEL_METRICS=0` keeps it off. A failed export warns once and is retried with the next interval's (cumulative) values; a `429` is not an error.

### Scraping

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
  "trace_id": "0af7651916cd43dd8448eb211c80319c",
  "span_id": "b7ad6b7169203331",
  "service": "shop",
  "env": "production",
  "version": "1.4.2",
  "dd": {
    "trace_id": "9532127138774266268",
    "span_id": "13235353014750950193",
    "service": "shop",
    "env": "production",
    "version": "1.4.2"
  }
}
```

With detail channels (or a slow hit) the same object grows nested `db` / `http` / `kv` / `timing` arrays. Production errors use `level: "error"` and `msg: "request_error"` with redacted request snapshot, stack, env, the error class as `error_type`, and the same `trace_id` / `span_id` / `dd` correlation as the access line.

`service` / `env` / `version` (top level and inside `dd`) appear only when configured — `OTEL_SERVICE_NAME` / `DD_SERVICE`, `DD_ENV`, `DD_VERSION` or `OTEL_RESOURCE_ATTRIBUTES` (see [Datadog](#datadog)) — so an unconfigured app never overrides the service your log shipper derives from container labels. `dd` holds the Datadog form of the ids and is present whenever there is a trace or a configured tag.

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
| `OTEL_SERVICE_NAME` | `service.name` resource attribute | `DD_SERVICE`, else `soli` |
| `OTEL_RESOURCE_ATTRIBUTES` | Extra `key=value` pairs, comma-separated, percent-decoded | unset |
| `OTEL_EXPORTER_OTLP_HEADERS` | Headers on every export POST: `k=v,k2=v2`, values percent-decoded | unset |
| `OTEL_EXPORTER_OTLP_TRACES_HEADERS` | Same, traces only; a key here replaces the generic one | unset |
| `OTEL_EXPORTER_OTLP_LOGS_HEADERS` | Same, logs only | unset |
| `OTEL_EXPORTER_OTLP_METRICS_HEADERS` | Same, metrics only ([OTLP metrics](#otlp-metrics)) | unset |
| `OTEL_EXPORTER_OTLP_TIMEOUT` | Per-POST timeout, milliseconds | `2000` |
| `SOLI_OTEL_LOGS` / `OTEL_LOGS_EXPORTER=otlp` | Also export log records over OTLP ([Log export](#log-export-otlp)) | off |
| `OTEL_EXPORTER_OTLP_LOGS_ENDPOINT` | Full `/v1/logs` URL | derived from base |
| `DD_SERVICE` / `DD_ENV` / `DD_VERSION` | Datadog unified service tagging, used when the OTel equivalents are unset | unset |
| `OTEL_SDK_DISABLED` | Force off when `true` | unset |
| `OTEL_TRACES_SAMPLER` | Head sampler — see [Sampling](#sampling) | `parentbased_always_on` |
| `OTEL_TRACES_SAMPLER_ARG` | Ratio for the `*traceidratio` samplers, `0`..`1` | `1.0` |
| `SOLI_OTEL_PROPAGATE` | `0` / `false` stops adding `traceparent` to outgoing calls | on |

**Precedence.** `service.name`: `OTEL_SERVICE_NAME` > `service.name` in `OTEL_RESOURCE_ATTRIBUTES` > `DD_SERVICE` > `soli`. Environment: `deployment.environment.name` / `deployment.environment` in `OTEL_RESOURCE_ATTRIBUTES` > `DD_ENV` (exported as both keys). Version: `service.version` in `OTEL_RESOURCE_ATTRIBUTES` > `DD_VERSION`; Soli does not invent one — its own version is on `telemetry.sdk.version`.

**Headers.** Values are credentials (`dd-api-key`, `Authorization`), so they are never printed: the startup banner lists header *names* only, and a malformed entry (no `=`, an invalid name, a value holding CR/LF after decoding) is skipped with a warning naming the variable and key, not the value. `https://` endpoints work out of the box (rustls with the Mozilla root set).

### What is exported

Per request:

1. A **SERVER** root span (`GET /path`) with `http.request.method`, `url.path`, `http.response.status_code`, `soli.request_id`.
2. Nested **INTERNAL** / **CLIENT** spans for middleware, before/after actions, controller actions, views/partials/components, DB queries, and outgoing HTTP — the same tree the flamegraph shows under `--dev`. Each outgoing call is one CLIENT span.

Per background job (run on the job pool), a trace of its own — see [Jobs](#jobs).

Export is **async** on a dedicated background thread. A full queue drops batches rather than stalling web workers; the first drop and the first POST failure print a one-time warning on stderr.

### Exceptions on spans

When a request ends with an uncaught error — the ones `/__soli/errors` groups and the production error block logs — the **SERVER** root span gets status `ERROR` with the message and an OTel `exception` event:

| Attribute | Value |
|-----------|-------|
| `exception.type` | The class of a thrown instance (`PaymentError`), `Error` for a thrown string or hash; for an error Soli raises itself, a descriptive name — `TypeError`, `IndexError`, `NameError`, `ArgumentError`, `NoMethodError`, `ZeroDivisionError`, else `RuntimeError` |
| `exception.message` | The error message |
| `exception.stacktrace` | The production error log's stack, one frame per line, paths relative to the app |

The handler span the error unwound out of (the action, a before/after action or a middleware) is marked `ERROR` too. A deliberate outcome — a `find` miss's 404, `forbidden()`, `halt(…)`, a `DomainError` — is an answer, not a failure: no exception, as on `/__soli/errors`.

### Log export (OTLP)

Opt-in: `SOLI_OTEL_LOGS=1` (or the standard `OTEL_LOGS_EXPORTER=otlp`) sends the events Soli logs — the access line (when `SOLI_LOG` asks for it) and `request_error` — as OTLP/HTTP JSON LogRecords to `OTEL_EXPORTER_OTLP_LOGS_ENDPOINT`, or the base endpoint + `/v1/logs`, or `http://127.0.0.1:4318/v1/logs`.

- `msg` becomes the body, `level` the severity (`info` 9, `warn` 13, `error` 17), `trace_id` / `span_id` the record's correlation; every other field is an attribute (nested arrays and objects included).
- The resource is the traces' resource (`service.name`, environment, version).
- Records are batched (up to 512, at least once a second) on their own thread behind a bounded queue (4096): a full queue drops records with a one-time warning; requests never wait.
- stdout / stderr logging is unchanged, and export works with `SOLI_LOG_FORMAT=text` too.

### W3C Trace Context

- Inbound `traceparent` is parsed and becomes the parent of the root span.
- Every response carries `traceparent` so gateways and clients can correlate.
- When tracing is on, responses also get `X-Request-Id` (unless `X-Soli-Request-Id` was already set in `--dev`).
- **Outgoing calls carry the trace on.** Every `HTTP.*` request — `get`, `post`, `request`, `get_json`, `stream`, the parallel `get_all` / `get_all_json` / `parallel`, and `SOAP.call` — gets a `traceparent` whose parent id is the CLIENT span recorded for that call, so the downstream service's span nests under it in Datadog, Jaeger or Tempo. The inbound `tracestate` (Datadog's `dd=…` among others) is forwarded unchanged. `HTTP.stream_to`, which outlives the request, names the span that started it instead.

```soli
# Nothing to write: inside a traced request this call sends
#   traceparent: 00-<the request's trace id>-<this call's span id>-01
HTTP.get("https://inventory.internal/items/42")

# A header you set yourself is never replaced
HTTP.get(url, {"headers": {"traceparent": upstream_traceparent}})
```

Like the OTel SDKs and Datadog's tracers, the headers go to **every host** a call reaches, third-party APIs included: they carry ids and the sampling flag, nothing else. `SOLI_OTEL_PROPAGATE=0` stops adding them (export is unaffected). With tracing off nothing is added.

### Log ↔ trace joins

Turn on both:

```bash
SOLI_LOG=access SOLI_LOG_FORMAT=json SOLI_OTEL=1 soli serve
```

JSON access lines and `request_error` lines include `trace_id` and `span_id` matching the exported root span, plus `dd.trace_id` / `dd.span_id` — the lower 64 bits of each id as an unsigned decimal string, which is how Datadog joins a log to an OTel trace. In Grafana / Datadog / Jaeger UI, jump from a log line to the full span tree.

### Sampling

Head sampling follows the standard OTel variables:

| `OTEL_TRACES_SAMPLER` | Which traces are exported |
|-----------------------|---------------------------|
| `parentbased_always_on` *(default)* | the caller's decision (the inbound `traceparent`'s sampled flag); every trace that starts here |
| `parentbased_always_off` | the caller's decision; none that start here |
| `parentbased_traceidratio` | the caller's decision; `OTEL_TRACES_SAMPLER_ARG` of those that start here |
| `always_on` | all, whatever the caller said |
| `always_off` | none |
| `traceidratio` | `OTEL_TRACES_SAMPLER_ARG` of all, whatever the caller said |

```bash
# Keep 10 % of the traces this app starts; follow upstream for the rest
OTEL_TRACES_SAMPLER=parentbased_traceidratio OTEL_TRACES_SAMPLER_ARG=0.1 soli serve
```

The ratio is decided from the trace id (the rule the Go and Java SDKs use), so services sampling one trace with the same ratio agree. An unsampled request still gets ids — its log lines keep `trace_id`, its response and its outgoing calls carry `traceparent` with flags `00`, so downstream services drop it too — it is only not exported. An unknown sampler name or a ratio outside `0`..`1` prints one warning on stderr and the default applies. Tail sampling, batching and retention stay on the collector.

### Jobs

A job run on the background pool is a trace of its own: a **CONSUMER** root span named `job <ClassName>` with `messaging.system=soli`, `messaging.destination.name` (the queue), `messaging.message.id` (the job id), `soli.job.class` and `soli.job.attempt`, and, when the job fails, an ERROR status with an `exception` event (`exception.type` `Error`, the failure message) as for a request's uncaught error, and under it the same db / http / view spans a request records. It carries the same resource (`service.name`, the `DD_*` tags) as request spans, and its outgoing `HTTP.*` calls carry its `traceparent`.

A job enqueued inside a traced request (or another job) keeps that span's `traceparent` in its queue row (`traceparent`, absent on rows enqueued untraced or before this version), and its root span gets an **OTLP link** to it — not a parent, since the job may run much later, on another host. Datadog, Jaeger and Tempo show it as a linked span you can follow back to the request. Mailer `deliver_later`, notification jobs and `HTTP.stream_to` handlers are jobs too. A job's sampling is decided like a request that arrives without `traceparent`. Webhook deliveries (`Webhook.enqueue`) are not traced.

## Datadog

Soli speaks OpenTelemetry, which Datadog ingests natively. Two setups:

**Through the Datadog Agent** (recommended — the agent adds host tags and handles retries). Turn on its OTLP receiver (`DD_OTLP_CONFIG_RECEIVER_PROTOCOLS_HTTP_ENDPOINT=0.0.0.0:4318`, or `otlp_config.receiver.protocols.http.endpoint` in `datadog.yaml`), then:

```bash
OTEL_EXPORTER_OTLP_ENDPOINT=http://datadog-agent:4318 \
DD_SERVICE=shop DD_ENV=production DD_VERSION=1.4.2 \
SOLI_LOG=access SOLI_LOG_FORMAT=json \
soli serve
```

Logs go to stdout/stderr as usual; the agent's log collection (container or file) parses the JSON and joins each line to its trace through `dd.trace_id` / `dd.span_id`. Add `SOLI_OTEL_LOGS=1` instead if you would rather ship logs over OTLP (set `otlp_config.logs.enabled: true` in the agent).

**Agentless** — straight to Datadog's OTLP intake, with the API key as a header:

```bash
OTEL_EXPORTER_OTLP_TRACES_ENDPOINT=https://otlp.datadoghq.com/v1/traces \
OTEL_EXPORTER_OTLP_LOGS_ENDPOINT=https://otlp.datadoghq.com/v1/logs \
OTEL_EXPORTER_OTLP_HEADERS=dd-api-key=$DD_API_KEY \
SOLI_OTEL_LOGS=1 \
DD_SERVICE=shop DD_ENV=production DD_VERSION=1.4.2 \
soli serve
```

The URLs above are an example: use the OTLP intake endpoints Datadog documents for your site (`datadoghq.eu`, `us3.datadoghq.com`, …) and check which signals it accepts agentless. Any `https://` endpoint works.

What you get:

- **Unified service tagging** — `DD_SERVICE` / `DD_ENV` / `DD_VERSION` become `service.name` / `deployment.environment(.name)` / `service.version` on the resource (traces, logs and OTLP metrics alike), `service` / `env` / `version` on every JSON log line and as DogStatsD tags. The OTel variables win when both are set.
- **APM traces** — the per-request span tree; the root span is the service entry span.
- **Log ↔ trace correlation** — `dd.trace_id` / `dd.span_id` on JSON logs (both forms of the id are kept, so other backends still join on the hex `trace_id`).
- **Error Tracking** — the `exception` event's `exception.type` / `exception.message` / `exception.stacktrace` map to Datadog's `error.type` / `error.message` / `error.stack`, so uncaught errors group in Error Tracking by class — throw your own `Error` subclasses to get distinct groups. JSON `request_error` lines carry `error_type`; remap `error` → `error.message`, `error_type` → `error.kind` and `stack` → `error.stack` in a log pipeline if you want log-based error tracking as well.

## Overview (`/__soli/`)

`/__soli/` is the page to open first: what needs dealing with most urgently across every operator page, and how the app is being used. It is the first item of the operator pages' top bar.

**Health at a glance.** Six tiles, each a link: requests over the chosen range with the change against the range before it, the share of server errors (5xx), latency p95 (p50 underneath), open error groups (and how many are new this hour), jobs (dead in the last 24 h, how many are waiting and how long the oldest has), and slow-query shapes seen in the last 24 h (and how many are new).

**Needs attention.** One ranked list built from every source the page can read:

| Source | Critical | Warning | Info |
|--------|----------|---------|------|
| Error groups (open only) | new within the hour; came back after being resolved and seen within the hour; rising (10+ in the last hour and 3× the hourly average before) | seen in the last 24 h | open, quiet for over a day |
| Jobs | dead within the hour; the oldest due job has waited 30 min | dead in the last 24 h; failing and waiting for a retry; running past its lease; 100+ waiting or the oldest waited 5 min | |
| Slow queries (seen in 24 h, 5 at most) | | new within the hour; seen within the hour with an all-time mean of twice the threshold | the rest of the day's |
| Spam | | a spike: 20+ refused in the last hour and 5× the average before | refused in the last 24 h (`200+` when the newest 200 all fall in the day) |
| Engine | a handler panicked within the hour | a panic in the last 24 h | an older panic; a handler the VM refused (it runs on the slower interpreter); a handler the two engines disagree on — the framework's to fix, worth reporting |
| Traffic | 5xx on 5 % of the last hour's requests (10+) | 5xx on 1 % (5+); a route with a p95 of 2 s or more over 50+ requests | |
| Recording | | a tracker dropped or failed to write samples within the hour; a store could not be read | a tracker's misses, the last over an hour ago |

Within a tier, rows are ordered by a score out of 100: **40 × recency** (`1 / (1 + hours since last seen)`), **35 × growth** (`(last hour − hourly average of the 23 before) / (the two added)`, never below 0) and **25 × volume** (`log10(1 + count over 24 h) / 4`, at most 1). Rows with no hourly history to compare (a job, a queue, the last hour's traffic) score no growth. Ties go to the most recent, then alphabetically, so a refresh does not reshuffle them. Resolved and ignored error groups never appear. Each row shows its tier, its source, what it is, the detail in monospace (the message, the query, the handler), why it is listed (counts, last seen, trend) and a metric; its button is a link to the item on its page, or — for a single dead job — that page's own **Retry** form, which keeps the same-origin POST rule. Nothing on the overview changes state on a `GET`. With nothing to show, it says so.

**Usage.** Requests per hour as a bar chart (per 5 minutes for 1 h, per 6 hours for 7 d), responses by status class (2xx, 3xx, 4xx, 5xx) and the top routes — by route **pattern** (`GET /posts/:id`), never the raw path — with requests, share, p95 and 5xx rate; a p95 of a second or more and a 5xx rate of 1 % or more are highlighted. `?range=1h`, `24h` (the default) and `7d` pick the window. A reply the response cache answers (`Soli-Response-Cache`) is counted under the route its worker counted it for. The framework's own endpoints (`/__soli/*`, `/__solidev/*`, `/_health`, `/_ready`, `/_metrics`, `/up`), static files and a miss for an asset-looking path (`/favicon.ico`) are not counted. Times are UTC.

**Refreshing.** *Updated N s ago* says how old the page is; **Refresh** reloads it, **Auto-refresh** reloads it every 30 seconds (`?refresh=30`, a `<meta>` refresh — off by default).

**Access.** The same gate as the pages it summarises, and never wider: it opens for anyone who may open at least one of them (`/__soli/mobile` included — every page's logo links here), and shows each source only to a caller who may open that source's page. Credentials for `/__soli/jobs` alone show the jobs tile and the job rows, nothing else. Usage and the engine rows are the overview's own, under `SOLI_OVERVIEW_*` or the shared set:

```bash
SOLI_OVERVIEW_USER=ops
SOLI_OVERVIEW_PASSWORD=<long random string>
SOLI_OVERVIEW_TOKEN=<long random string>   # Authorization: Bearer …

# or the shared set, accepted by every operator page
SOLI_ADMIN_USER=ops
SOLI_ADMIN_PASSWORD=<long random string>
```

In `--dev` it is open to this machine (and linked from the dev bar's tools panel); in production with nothing configured `/__soli/` answers `404`. A tracker that is switched off (`SOLI_ERRORS=off`, …) is not read. The page is sent with `Cache-Control: no-store` and `X-Frame-Options: SAMEORIGIN`.

**Cost.** The page reads what the pages already read — the open error groups, the slow-query groups, the refused posts — each a bounded list, plus memory. From the queue it reads its own two bounded lists, filtered by the database: the jobs not finished yet (oldest first) and those that died in the last 24 hours (newest first), 500 at most each, and it counts the due backlog and the day's dead jobs with two `COUNT` queries, so a week of finished jobs never hides them. Usage is counted on the worker around each request: a monotonic clock read at each end, then two counter increments on a table that worker owns, behind a lock no other worker takes (about 15 ns for the counting and 65 ns with the clock reads, measured with eight threads recording at once). Each worker folds its counts into the shared table once every five minutes, and the page folds them all when it renders, so what it shows is live. The 1 h and 24 h views come from memory; when the app has a database configured (a `config/database.toml`, a SQL adapter, or `SOLIDB_*` settings), hourly totals are also written to `_soli_usage` once a minute, one update per hour of data, never one per request, and read back before the first write, so 7 d survives a restart. If the database does not answer yet, the hours are kept and the writer tries again with growing pauses (up to 30 minutes), reading the stored hours back as soon as it can. Without a database, usage stays in memory and the section says so. Stored hours are kept 15 days.

**Limits.** Usage counts what this process answered, all its workers together; the stored hours add up across processes that share a database, so with several processes the 24 h and 7 d views after a restart mix every process's past hours with this one's new hours, and the change against the range before can read as a drop. Up to 62 route patterns are tracked by name per app; past that, routes are counted together as *other routes*, and a request no route matched as *no route matched*. Percentiles come from power-of-two latency buckets, interpolated: close, not exact. The job totals are counted by the database; the per-handler groups are built from the 500 rows read. The overview does not keep its own history of backlog size, so *growing* reads from how long the oldest job has waited. `SOLI_USAGE=off` stops counting; under `APP_ENV=test` it is off unless `SOLI_USAGE=on`.

## Error tracking (`/__soli/errors`)

Every request that ends in a 500 is recorded, grouped, and shown at `/__soli/errors` — a self-hosted, built-in stand-in for Sentry. There is no service to sign up for and no SDK: it is on by default and writes to the app's own database (SoliDB, Postgres, MySQL or SQLite), in a `_soli_errors` table.

**Grouping.** Each failure gets a fingerprint from its message and the frame that raised it. Numbers, ids, UUIDs and quoted values are stripped from the message first, and the line number from the frame, so `Order 42 not found` and `Order 97 not found` are one group, and editing code above the bug does not start a new one. Double quotes always quote; a single quote quotes only at the start of a word, so the apostrophe in `can't` or `User's` stays part of the message rather than swallowing the words after it. File paths are stored relative to the app root, so deploying to another directory keeps the history.

**What a group holds.** Count, first and last seen, and the five newest occurrences, each with its stack, the request, the handler's local variables, and a `curl` line that replays it against a local server. Auth headers (`Authorization`, `Cookie`, `X-Api-Key` and any header whose name looks secret), secret-looking keys at any depth (`password`, `token`, `api_key`, …) and the raw request body are replaced by `[REDACTED]` before anything is stored — in the request, in the handler's locals, and in the data passed to `render()` when a template fails mid-render — the same redaction the stderr error log uses. Key names match whatever the separator, so `api-key`, `apiKey` and `x-api-key` count as `api_key`. The replay `curl` line never carries `Authorization`, `Cookie` or API-key headers, nor a secret-named query param. Redaction goes by field name: a field called `card` is not recognised as a secret, so don't put card numbers in forms you do not control.

**Finding one failure.** The production 500 page shows an *Error ID* — the request id, also on the `[ERROR] request_id=…` log line. Paste it into the search box at the top of the list (or open `/__soli/errors?q=<id>`) to land on its group with that occurrence opened. Only the five newest occurrences of a group are kept, so an older id is not found; the page says so, and the log line still has it. On Postgres, MySQL and SQLite the lookup reads every stored group; on SoliDB it is one filtered query.

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

## Query time and N+1 (`/__soli/slow_queries`)

A query that takes 3 ms is never slow, but run forty times a request on every request it can be most of your database time. So every query the ORM runs — the fast ones too, same coverage as [slow queries](#slow-queries-__solislow_queries) — is counted by shape, and the page's default view, **all queries**, ranks the shapes by the time they took over the **last 24 hours**:

```
301  FOR doc IN posts FILTER doc.n == @n__eq_1 LIMIT ? RETURN doc   39 ms · 91%  avg 0.1 ms  N+1 · 16×  GET /feed → home#feed
 20  FOR doc IN posts RETURN doc                                     2 ms · 4%   avg 0.1 ms
```

**What a row says.** Calls in the last 24 hours, the time they took and its share of all query time, the average, an hourly trend of time, and the **most times one request or job ran that shape** — with the request that did it (`GET /feed → home#feed`). From 10 runs in one request the row is tagged **N+1**, which is the production counterpart of the dev bar's N+1 badge and `assert_no_n_plus_one`. A row links to the shape's hour-by-hour table, and to its slow runs when it has some. No bind value is ever stored, only the shape.

**Counting.** Only runs inside a request or a job count towards the per-request figure, so a WebSocket or LiveView worker's stream of events is never read as one request running a query a thousand times. The job engine's own polling (claims, leases, cron) is not counted; the jobs it runs are. Neither is anything on the framework's `_soli_*` tables.

**Cost.** One hash of the query text and one counter update per query — about 20 ns, against a database round trip of tens of microseconds. Each worker thread keeps its own table; the text is turned into a shape only the first time a thread meets it. Totals are written once a minute, one update per shape, by the same kind of background writer as error tracking. A query that builds its values into the text instead of binding them is a new text on every call: past 2048 live texts per thread, those are counted in one *not broken down* row rather than normalised.

**Retention.** At most 1000 shapes per app; when the table is full, shapes nothing has run for 7 days are removed to make room. Each shape keeps 24 hours of hourly totals, so a fixed N+1 stops being flagged once its hours age out. `SOLI_QUERY_STATS=off` stops counting; like the other trackers it is off under `APP_ENV=test` unless `SOLI_QUERY_STATS=on`.

## Slow queries (`/__soli/slow_queries`)

Every database query that takes `SOLI_SLOW_QUERY_MS` (default **200 ms**) or longer is recorded, grouped by shape, and shown at `/__soli/slow_queries`. It covers every query the ORM runs — SoliDB over HTTP or the native driver, and the Postgres, MySQL and SQLite adapters — in requests and in background jobs, and writes to the app's own database in a `_soli_slow_queries` table. On by default, like error tracking.

**Grouping.** A query's *shape* is the query with its literals taken out: numbers become `?`, a list of values becomes one `?`, whitespace collapses, and in SDBQL quoted strings become `?` too. So `FILTER u.id == 42` and `FILTER u.id == 97` are one group, and `IN (1, 2, 3)` is the same shape as `IN (7)`. In the adapters' SQL, a quoted name is an identifier or a JSON key the adapter wrote (`doc ->> 'tag'`) — values are always binds — so those are kept, and a filter on `tag` stays apart from a filter on `name`. Placeholders (`@name`, `$1`, `?`) are kept as written.

**What a group holds.** How many slow runs, their average, slowest and total time, the request or job that ran the last one (`GET /orders → orders#index`, `job ReportJob`), first and last seen, 24 hours of hourly counts, and the **five slowest runs**, each with the query as it ran, its bind values and where it came from. Bind values under a secret-looking name (`password`, `token`, …) are replaced by `[REDACTED]` and long values are cut to 200 characters; `SOLI_SLOW_QUERY_BINDS=off` stores no bind values at all. SQL binds are numbered (`$1`, `?`), so their names say nothing — turn binds off if your queries filter on personal data you do not want in the table.

**Reading the list.** The page opens on [all queries](#query-time-and-n1-__solislow_queries); slow runs are its second tab (`?order=impact`). Four orders: **impact** (total time spent in slow runs — usually where to start), **slowest** (the single worst run), **frequent** (most slow runs) and **recent**. **delete** forgets a shape; it comes back on its next slow run.

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

## Spam (`/__soli/spam`)

The posts `spam_check(req)` refused and accepted, with the model it scores
them with; "not spam" and "spam" correct a mistake and retrain it. Same access
rules as the errors page, with `SOLI_SPAM_*` credentials; `SOLI_SPAM=off`
stops recording. See [Spam on public forms](security/spam.md).

Each post shows its learnt score where an error shows its count — red from
0.9 (the line `spam_check` refuses at), amber leaning spam, faint at 0.5 (no
opinion yet: the model is untrained, or the words are new), green leaning ham —
then its message, path and client, the reason it was refused (hover the tag for
what triggered it), and when it arrived.

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
# Slack, Microsoft Teams, Discord, Google Chat, Campfire or any URL — comma-separated
SOLI_NOTIFY_WEBHOOKS=https://hooks.slack.com/services/T000/B000/XXXX,https://acme.webhook.office.com/webhookb2/...

# Addresses, sent through the app's own mailer (SOLI_SMTP_*)
SOLI_NOTIFY_EMAILS=oncall@example.com,cto@example.com
SOLI_NOTIFY_FROM=alerts@example.com        # default: SOLI_SMTP_FROM

# Links in messages point here (default: https:// + the first SOLI_APP_HOSTS)
SOLI_NOTIFY_URL=https://shop.example.com
```

Webhooks are recognised by host and sent the message each product expects: Slack incoming webhooks (`hooks.slack.com`) get formatted text with a link, Microsoft Teams (`*.webhook.office.com`, and Workflows URLs on `*.logic.azure.com` / `*.powerplatform.com`) an Adaptive Card with an **Open in Soli** button, Discord and Google Chat a text message. Campfire (the self-hosted 37signals chat) is recognised by its bot URL's path, `https://<your campfire>/rooms/<room id>/<bot key>/messages` (Account → Bots → the room's URL), and gets a short HTML message with an **Open in Soli** link. Campfire has no `@everyone` and no groups: who is alerted is who is in the room, so post to an all-access room to reach everyone, or to a closed room holding only the on-call members to reach a group. Members who want a push for every alert set that room's notifications to *Everything*. Any other URL receives the event as JSON:

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

**Push on your own devices.** The errors page and the slow-queries page each have a **Notify this device** button. Press it in a browser (it asks for permission) and that device gets a Web Push notification for each event of the page's topic — `error.new` / `error.regressed` / `error.spike` from `/__soli/errors`, `slow_query.new` from `/__soli/slow_queries` — even with the page closed. Clicking the notification opens the error or the query. **Send a test** checks it end to end; **Stop notifying this device** unsubscribes. Nothing to configure:

- the VAPID key pair is the app's own `VAPID_PUBLIC_KEY` / `VAPID_PRIVATE_KEY` / `VAPID_SUBJECT` when set (the ones `WebPush` reads), otherwise a pair generated once and stored in the app's database (`_soli_push_keys`) — replacing it makes every browser subscribe again;
- subscriptions are stored in `_soli_push_subscriptions`, one per browser, with the topics it chose; a push service answering 404/410 (the browser dropped it) deletes it. Whether anyone is subscribed is read once and reused for 30 seconds, so a burst of new errors costs one database read; subscribing or unsubscribing on this server takes effect at once, a browser subscribed through another host of the same app within those 30 seconds;
- the subscription endpoints (`POST /__soli/push/subscribe`, `unsubscribe`, `status`, `test`) sit behind the gate of the topic's page (`SOLI_ERRORS_*`, `SOLI_SLOW_QUERIES_*` or `SOLI_ADMIN_*`) and the same-origin check; the service worker, `/__soli/push/sw.js`, is public and controls `/__soli/` only;
- Web Push needs HTTPS (or `localhost`); the same events, throttle and `SOLI_NOTIFY_EVENTS` apply as for the other destinations.

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
| Overview (`/__soli/`) | on, open to this machine | on, behind any operator page's credentials, each source shown only with its own |
| Error tracking (`/__soli/errors`) | on, open to this machine | on, behind `SOLI_ERRORS_*` / `SOLI_ADMIN_*` |
| Slow queries (`/__soli/slow_queries`) | on, open to this machine | on, behind `SOLI_SLOW_QUERIES_*` / `SOLI_ADMIN_*` |
| Notifications | when `SOLI_NOTIFY_*` is set (emails go to the dev inbox without SMTP) | when `SOLI_NOTIFY_*` is set |

**Never indexed.** Every response under `/__soli/…` and from `/_health`, `/_ready`, `/_metrics` and `/up` carries `X-Robots-Tag: noindex, nofollow` — the 401 challenge and the 404 of an unconfigured page included — so a search engine that finds the URL neither indexes it nor follows its links. A header the page set itself is kept. Nothing to configure; add the paths to your `robots.txt` too if you want crawlers not to ask at all.

Production logging reuses the same channel buffers as the dev bar (`query`, `http`, `kv`, `timing`) without paying for hot-reload, the bar injection, or the interpreter demotion that `--dev` implies.

## Limits (honest)

- No auto-instrumentation of every third-party client library — only Soli's own request path, ORM, and `HTTP.*` client.
- OTLP export covers **traces** and, opt-in, **logs** (`SOLI_OTEL_LOGS=1`) and **metrics** (`SOLI_OTEL_METRICS=1`) — HTTP/JSON only, no gRPC or protobuf. Logs always stay on stdout/stderr too, and Prometheus `/_metrics` keeps working alongside. The last metrics interval before the process exits is not flushed; DogStatsD lines still queued at exit are lost too.
- Metrics are per process: several processes (or hosts) each expose their own `/_metrics` and send their own series — aggregate in Prometheus or Datadog.
- `Metrics.time` takes its tags positionally (`Metrics.time("x", {"k": "v"}) do … end`): the bytecode VM refuses a labelled call that also carries a block.
- Head sampling only (`OTEL_TRACES_SAMPLER`); tail sampling and any sampling UI belong on the collector.
- Calls made outside Soli's `HTTP.*` client — `Apns`, `Fcm`, S3, webhook deliveries — carry no `traceparent`.
- Background jobs are traced, but their log lines have no JSON mode, so they carry no `dd` correlation.

## See also

- [Configuration](configuration.md) — full env-var reference
- [Debugging](/docs/development-tools/debugging) — dev bar, flamegraph, breakpoints
- [Deploy](deploy.md) — shipping the binary
- [How Soli Compares](/docs/getting-started/comparison) — ops posture vs Rails / Phoenix / Laravel / Django
