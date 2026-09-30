# BenchmarkRun model — one stored row per `run_bench.sh` execution.
# The `workloads` field holds the full per-workload result array (a list of
# hashes) so the home dashboard can render the latest run straight from the DB.
# Collection: benchmark_runs

class BenchmarkRun < Model
    validates("stamp", { "presence": true })

    # Most-recent run first, capped to one row.
    scope("latest", fn() { this.order("_created_at", "desc").limit(1) })

    # Full run history, newest first.
    scope("history", fn() { this.order("_created_at", "desc") })
end
