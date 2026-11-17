# The invoices of /invoices are a public demo: each lives a day. It shares
# HourlyTallyJob's Cron Trigger ("0 * * * *", UTC): one trigger runs both.
class InvoiceCleanupJob
  static cron = Cron.hourly()

  static def perform(args)
    cutoff = DateTime.from_unix(DateTime.utc.to_unix - 86400).to_iso.substring(0, 19) + "Z"
    Invoice.where({"created_at": {"lt": cutoff}}).all.each { |invoice| invoice.delete }
  end
end
