const MAX_CHECK_INS_PER_MINUTE = 30

class CheckInsController < Controller
  # GET /d1 — check-ins stored in Cloudflare D1, by datacenter
  def index(req)
    @title = "D1"
    @description = "Check in from the edge: a Soli model on Cloudflare D1, written and read by the Worker that " +
      "answers, with the code that does it."
    @runtime = RuntimeInfo.of(req)
    started = clock
    # Two statements, two round trips to D1: the total is summed from the
    # per-datacenter counts rather than asked for separately.
    per_colo = CheckIn.group_by("colo").all.sort_by { |row| -row["n"] }
    @total = per_colo.reduce(fn(sum, row) sum + row["n"], 0)
    @colos = per_colo.length
    @by_colo = per_colo.take(12)
    @latest = CheckIn.order("at", "desc").limit(10).all
    @db_ms = ((clock - started) * 1000).round(1)
    @checked_in = params["checked_in"] == "1"
    @busy = params["busy"] == "1"
  end

  # POST /d1/check-in — stores the datacenter and country that served this request
  def create(req)
    # A public form with no login: cap the writes for everyone at once, so a
    # script cannot fill D1 (30 a minute stays under D1's free daily quota).
    return redirect("/d1?busy=1") if @_recent_check_ins >= MAX_CHECK_INS_PER_MINUTE

    runtime = RuntimeInfo.of(req)
    CheckIn.create({
      "colo": runtime["colo"],
      "country": runtime["country"],
      "at": DateTime.utc.to_iso.substring(0, 19) + "Z"
    })
    redirect("/d1?checked_in=1")
  end

  private

  def _recent_check_ins
    cutoff = DateTime.from_unix(DateTime.utc.to_unix - 60).to_iso.substring(0, 19) + "Z"
    CheckIn.where({"at": {"gte": cutoff}}).count
  end
end
