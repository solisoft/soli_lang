# Observability

Soli ships three production signals out of the box: **metrics**, **structured logs**, and **distributed traces**. All are opt-in so a quiet process pays nothing until you turn a channel on. Alongside them, **error tracking** groups every failed request into a triage page inside the app.

| Signal | Enable | Where it goes |
|--------|--------|---------------|
| Metrics | `SOLI_METRICS=1` | Prometheus text at `GET /_metrics` |
| Logs | `SOLI_LOG=…` (+ optional `SOLI_LOG_FORMAT=json`) | stdout / stderr |
| Traces | `SOLI_OTEL=1` or `OTEL_EXPORTER_OTLP_*` | OTLP/HTTP JSON to your collector |
| Health | always on | `GET /_health`, `GET /_ready` |
| Errors | on (`SOLI_ERRORS=off` to stop) | `_soli_errors` table, shown at `/__soli/errors` |

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

**Limits.** Only HTTP request failures are recorded — job failures stay on `/__soli/jobs`, and LiveView/EUI event errors are not captured yet. There are no alerts or notifications. Counts are exact within one process; several hosts writing the same group at the same moment can undercount it. Groups are kept until you delete them.

**Retention.** An app keeps at most **1000 groups** (plus one overflow group). Once that many exist, an occurrence whose fingerprint is not already stored is counted in a single *overflow* group instead of starting a new one, with its own message kept in the sample; groups already stored keep counting as usual. The list says when the limit has been reached. Deleting groups makes room again — resolving or ignoring them does not. Each group keeps its five newest samples and 24 hours of hourly counts.

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
