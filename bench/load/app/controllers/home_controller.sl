# Home controller - handles the root routes

class HomeController < Controller
    # GET / — dashboard. Shows one run in detail plus the full run history.
    # `?run=<key>` selects a historical run; otherwise the latest is shown.
    def index
        @runs = BenchmarkRun.history().all()

        let selected = null
        selected = BenchmarkRun.find_by("_key", params["run"]) unless params["run"].blank?
        selected ||= @runs.first

        @run = selected
        @workloads = []
        @workloads = selected.workloads unless selected.nil?

        # Derive peak/total from each run's workloads so the history table is
        # populated even for runs stored before those fields existed.
        @history = @runs.map do |run| this._summarize(run) end
        @title = "Soli Benchmark"
    end

    # Reduce one run to the figures the history table shows.
    def _summarize(run)
        let workloads = run.workloads ?? []
        let peak = 0
        let total = 0
        for entry in workloads
            let rps = (entry["rps"] ?? "0").to_i()
            peak = rps if rps > peak
            total = total + (entry["requests"] ?? "0").to_i()
        end

        return {
            "key": run._key,
            "stamp": run.stamp,
            "peak_rps": str(peak),
            "total_requests": str(total),
            "duration": run.duration,
            "connections": run.connections
        }
    end

    # GET /health
    def health
        {
            "status": 200,
            "headers": {"Content-Type": "application/json"},
            "body": "{\"status\":\"ok\"}"
        }
    end
end
