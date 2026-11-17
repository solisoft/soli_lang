# An invoice edited live on /invoices/:id. A public demo: no account, the
# unguessable key is the only way in, and InvoiceCleanupJob deletes invoices
# a day old. On the Worker it lives in D1, as a document.
class Invoice < Model
  const MAX_LINES = 12
  const VAT_RATES = ["0", "5.5", "10", "20"]
  const CURRENCIES = ["EUR"]
  # What a visitor may change, and how long each field may be.
  const PARTY_FIELDS = {"name": 60, "address_line": 80, "postcode": 12, "city": 40, "country": 2, "vat_id": 20}
  const LINE_FIELDS = ["name", "quantity", "unit_price", "vat_rate"]

  # A fresh draft: a fictional studio billing the visitor, three lines.
  static def draft(colo)
    today = DateTime.utc
    {
      "number": "EDGE-" + today.to_iso.substring(0, 10).replace("-", "") + "-" + colo.to_s.upcase,
      "issue_date": today.to_iso.substring(0, 10),
      "due_date": DateTime.from_unix(today.to_unix + 30 * 86400).to_iso.substring(0, 10),
      "currency": "EUR",
      "payment_terms": "30 days net",
      "note": "Demo invoice drawn by a Cloudflare Worker in #{colo}. Not a request for payment.",
      "seller": {
        "name": "Atelier Nimbus", "address_line": "12 rue des Nuages", "postcode": "13001",
        "city": "Marseille", "country": "FR", "vat_id": "FR00123456789",
        "iban": "FR7630006000011234567890189", "bic": "AGRIFRPP"
      },
      "buyer": {
        "name": "Your company", "address_line": "1 Main Street", "postcode": "75002",
        "city": "Paris", "country": "FR", "vat_id": ""
      },
      "lines": [
        {"name": "Edge deployment, 3 regions", "quantity": "1", "unit_price": "1200", "vat_rate": "20"},
        {"name": "LiveView workshop (day)", "quantity": "2", "unit_price": "650", "vat_rate": "20"},
        {"name": "Technical book", "quantity": "3", "unit_price": "39.90", "vat_rate": "5.5"}
      ],
      "rev": 1,
      "created_at": today.to_iso.substring(0, 19) + "Z"
    }
  end

  # The typed invoice `pdf_facturx_from_invoice` and `pdf_invoice_data` read,
  # from a stored document: lines with a blank name are left out.
  static def typed(doc)
    lines = doc["lines"]
      .filter { |line| line["name"].to_s.trim.present? }
      .map { |line| {
        "name": line["name"].to_s.trim,
        "quantity": Invoice.number_or(line["quantity"], "1").to_f,
        "unit_price": Invoice.number_or(line["unit_price"], "0"),
        "vat_rate": Invoice.number_or(line["vat_rate"], "20").to_f
      } }
    {
      "number": doc["number"], "issue_date": doc["issue_date"], "due_date": doc["due_date"],
      "currency": doc["currency"], "payment_terms": doc["payment_terms"], "note": doc["note"],
      "seller": doc["seller"], "buyer": Invoice.party(doc["buyer"]), "lines": lines
    }.to_json
  end

  # A blank VAT number is no VAT number.
  static def party(fields)
    fields.filter { |key, value| key != "vat_id" || value.to_s.trim.present? }
  end

  # `text` when it reads as a number, else `fallback`.
  static def number_or(text, fallback)
    value = text.to_s.trim.replace(",", ".")
    Regex.matches("^[0-9]{1,9}(\\.[0-9]{1,2})?$", value) ? value : fallback
  end
end
