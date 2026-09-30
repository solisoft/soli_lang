# Soli benchmark harness

Load-tests a running Soli app with [`oha`](https://github.com/hatoo/oha) across
four workloads and produces reports + a live dashboard at `/`.

## Workloads

| Group  | Endpoint                     | What it measures                              |
|--------|------------------------------|-----------------------------------------------|
| `json` | `GET /bench/json`            | Single-object JSON serialization (no DB/view) |
| `json` | `GET /bench/json_many?n=100` | Array JSON serialization                      |
| `view` | `GET /bench/view?n=50`       | Server-side template rendering (no DB)        |
| `page` | `GET /bench/page?step=1..4`  | Realistic page: layout + nested partials + DB |
| `db`   | `GET /bench/db_one`          | DB read, one row                              |
| `db`   | `GET /bench/db_list?n=100`   | DB read, N rows (DB-side `LIMIT`)             |
| `crud` | `POST /bench/crud`           | Create (write path)                           |
| `crud` | `GET /bench/crud/:id`        | Read by key                                   |
| `crud` | `PUT /bench/crud/:id`        | Update                                        |

### The `page` workload — simulating a real app

`GET /bench/page?step=N` renders a full page through the `application` **layout**,
composed of **nested partials** (`page → _section → _item`, and `_stats → _stat`),
backed by **DB calls**. Complexity ramps over four steps so you can measure the
cost of a page growing more "real":

| Step | DB queries | Sections rendered                         |
|------|-----------|--------------------------------------------|
| 1    | 1         | Products                                   |
| 2    | 2         | + Recent                                   |
| 3    | 3         | + Featured                                 |
| 4    | 4         | + Summary (aggregated stats)               |

The runner benchmarks each step separately (`page_1`…`page_4`) so the report
shows throughput dropping and latency climbing as the page does more work.

## Prerequisites

- `oha`, `jq`, `curl` on `PATH`
- The app running. The simplest way:

  ```bash
  soli serve .          # listens on http://localhost:5011 by default
  ```

## Run

```bash
./benchmarks/run_bench.sh                 # 10s per workload, 50 connections
./benchmarks/run_bench.sh -z 30s -c 100   # longer / heavier
BASE_URL=http://localhost:5011 ./benchmarks/run_bench.sh
```

Environment knobs: `BASE_URL`, `DURATION`, `CONNECTIONS`, `SEED_ROWS`.

The script seeds the `widgets` collection, runs one `oha` pass per workload,
then writes three artifacts per run under `benchmarks/results/<timestamp>/`:

- `raw/<workload>.json` — raw `oha` JSON output
- `report.md` — Markdown summary table
- `report.html` — standalone styled report (open in a browser)
- `summary.json` — machine-readable summary

It also `POST`s `summary.json` to `/bench/results`, which stores the run in the
DB so the **home page (`/`) always shows the latest run**.

`benchmarks/results/latest` symlinks the most recent run.

## Updating results & history

To record a new run, just run the script again — nothing else:

```bash
./benchmarks/run_bench.sh
```

Every run is kept, in two places:

- **On disk**: one timestamped folder per run under `benchmarks/results/`.
- **In the app DB**: one `BenchmarkRun` row per run.

The dashboard at `/` shows the most recent run in detail **plus a full run
history**. Click any past run in the history table (or visit `/?run=<key>`) to
view that run's numbers. The newest run is highlighted as *current*.

## Regenerate reports without re-running the load test

```bash
./benchmarks/gen_report.sh benchmarks/results/<timestamp> \
    http://localhost:5011 10s 50 <timestamp> <<'WL'
json|json|Single JSON object|GET|/bench/json
...
WL
```

(`run_bench.sh` calls this for you; you only need it to re-render an old run.)

## Tests

The spec lives at `tests/controllers/bench_controller_spec.sl` and covers every
action and edge case. It needs the `soli test` in-process server to boot
(`test_server_start()`); on a host where that server cannot bind, run the
verification against a live `soli serve .` instead (the harness does exactly
this). DB connectivity for tests is configured in `tests/.env.test`.
