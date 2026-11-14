# Runs every hour on the Worker: `soli edge build` turns this `static cron`
# into a Cloudflare Cron Trigger ("0 * * * *", UTC) in wrangler.toml, and the
# Worker's scheduled() handler runs the job whose schedule it is.
class HourlyTallyJob
  static cron = Cron.hourly()

  static def perform(args)
    CronRun.create({
      "at": DateTime.utc.to_iso.substring(0, 19) + "Z",
      "check_ins": CheckIn.count
    })
    # Keep a day of runs.
    stale = CronRun.order("at", "desc").offset(24).all
    stale.each { |run| run.delete }
  end
end
