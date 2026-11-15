# The audited model, driven over HTTP by tests/audit_generator_test.rs.
class InvoicesController < Controller
  # POST /members
  def member
    render_json({"key": Member.create({"email": params["email"]})._key})
  end

  # POST /invoices
  def create
    invoice = Invoice.create({
      "number": params["number"],
      "total": params["total"],
      "iban": params["iban"],
      "secret_note": params["secret_note"],
      "views": 0
    })
    render_json({"key": invoice._key})
  end

  # POST /invoices/:id/update
  def update
    invoice = Invoice.find(params["id"])
    invoice.update(permit(params, {"total": true, "iban": true, "secret_note": true, "views": true}))
    render_json({"audits": invoice.audits.length})
  end

  # POST /invoices/:id/delete
  def destroy
    Invoice.find(params["id"]).delete
    render_json({"ok": true})
  end

  # GET /invoices/:id/audits
  def audits
    rows = Audit.where({"record_key": params["id"]}).order("at", "asc").all
    render_json(rows.map { |row| row.to_h })
  end

  # GET /audit/verify
  def verify
    render_json({"broken": Audit.verify, "count": Audit.count})
  end

  # POST /audit/tamper — what someone with database access would do
  def tamper
    row = Audit.find_by("seq", params["seq"])
    row.update({"actor_label": "nobody@example.com"})
    render_json({"ok": true})
  end

  # GET /audit/actor
  def actor
    member = Member.find_by("email", params["email"])
    render_json({
      "count": Audit.by_actor(member).length,
      "period": Audit.between(0, DateTime.utc.to_unix + 10).length,
      "before": Audit.between(0, 10).length
    })
  end

  # POST /audit/purge
  def purge
    render_json({"purged": Audit.purge_before(DateTime.utc.to_unix + 10), "left": Audit.count})
  end
end
