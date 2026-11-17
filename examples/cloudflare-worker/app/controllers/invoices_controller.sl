# Invoices created in the last ten minutes, for everyone: a public form with no
# login, so writes to D1 are capped.
const MAX_INVOICES_PER_TEN_MINUTES = 40
# Twice the width the page shows the preview at, for sharp text.
const INVOICE_PREVIEW_WIDTH = 760

class InvoicesController < Controller
  # GET /invoices — what the demo does, and the latest invoices
  def index(req)
    @title = "Live invoicing"
    @description = "Edit an invoice live from a Cloudflare Worker: a Soli LiveView on a Durable Object, D1, " +
      "a page preview redrawn at every change and a Factur-X PDF/A-3 with a scan-to-pay QR code."
    @runtime = RuntimeInfo.of(req)
    @latest = Invoice.order("created_at", "desc").limit(8).all
    @busy = params["busy"] == "1"
  end

  # POST /invoices — a new draft, then its editor
  def create(req)
    return redirect("/invoices?busy=1") if @_recent_invoices >= MAX_INVOICES_PER_TEN_MINUTES

    invoice = Invoice.create(Invoice.draft(RuntimeInfo.of(req)["colo"]))
    redirect("/invoices/#{invoice._key}")
  end

  # GET /invoices/:id — the live editor
  def show(req)
    @invoice = Invoice.find(params["id"])
    @title = "Invoice #{@invoice.number}"
    @description = "A live invoice drawn by a Cloudflare Worker: edit it, share the link, download its Factur-X PDF."
    @runtime = RuntimeInfo.of(req)
  end

  # GET /invoices/:id/pdf — the Factur-X PDF/A-3b, rendered in the Worker
  def pdf(req)
    invoice = Invoice.find(params["id"])
    typed = Invoice.typed(invoice.to_h)
    body = pdf_facturx_from_invoice(InvoiceDocument.template.to_json, typed,
      {"profile": "en16931", "title": "Invoice #{invoice.number}", "author": "Soli on Cloudflare Workers"})
    {
      "status": 200,
      "headers": {
        "Content-Type": "application/pdf",
        "Content-Disposition": "inline; filename=\"#{invoice.number}.pdf\"",
        "Cache-Control": "no-store"
      },
      "body_base64": body
    }
  end

  # GET /invoices/:id/preview — page 1 as a PNG, redrawn for every revision
  def preview(req)
    invoice = Invoice.find(params["id"])
    data = pdf_invoice_data(Invoice.typed(invoice.to_h))
    response = pdf_preview_response(InvoiceDocument.template.to_json, {"data": data}.to_json,
      {"width": INVOICE_PREVIEW_WIDTH})
    response["headers"]["Cache-Control"] = "public, max-age=86400"
    response
  end

  private

  def _recent_invoices
    cutoff = DateTime.from_unix(DateTime.utc.to_unix - 600).to_iso.substring(0, 19) + "Z"
    Invoice.where({"created_at": {"gte": cutoff}}).count
  end
end
