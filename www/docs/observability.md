# Observability

Soli ships three production signals out of the box: **metrics**, **structured logs**, and **distributed traces**. All are opt-in so a quiet process pays nothing until you turn a channel on.

| Signal | Enable | Where it goes |
|--------|--------|---------------|
| Metrics | `SOLI_METRICS=1` | Prometheus text at `GET /_metrics` |
| Logs | `SOLI_LOG=…` (+ optional `SOLI_LOG_FORMAT=json`) | stdout / stderr |
| Traces | `SOLI_OTEL=1` or `OTEL_EXPORTER_OTLP_*` | OTLP/HTTP JSON to your collector |
| Health | always on | `GET /_health`, `GET /_ready` |

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

Scrape from Prometheus / Grafana Alloy / Datadog agent like any other text exposition endpoint. There is no auth on `/_metrics`; bind it to a private interface or front it with a proxy that restricts access.

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

## Dev vs production

| | `--dev` | Production |
|--|---------|------------|
| Dev bar (queries, flamegraph, replay) | on | off |
| Access log | always on (terminal) | `SOLI_LOG` / `SOLI_REQUEST_LOG` |
| JSON format | available | available |
| Span tree | flamegraph | OTLP when OTEL on |
| Metrics | opt-in | opt-in |
| Health endpoints | on | on |

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
