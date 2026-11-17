# LiveView handlers: `router_live("tally", "live#tally")` in config/routes.sl.
# Each takes {event, params, state} and returns the new state — on the edge
# exactly as under `soli serve`.

# The shared board on /live.
def tally(event_data)
  state = event_data["state"]
  count = state["count"] || 0
  clicks = state["clicks"] || 0

  changes = match event_data["event"] {
    "increment" => {"count": count + 1, "clicks": clicks + 1},
    "decrement" => {"count": count - 1, "clicks": clicks + 1},
    "reset" => {"count": 0, "clicks": clicks + 1},
    _ => {"count": count, "clicks": clicks}
  }
  state.merge(changes)
end

# The invoice editor on /invoices/:id. Each invoice is a room (its key), so
# every screen open on it shares one instance; the page's InvoiceLoad hook
# sends `load` with the key once the socket is up. Every change is written to
# D1, then the totals are recomputed by the PDF engine itself.
def invoice_editor(event_data)
  state = event_data["state"]
  params = event_data["params"] || {}
  event = event_data["event"]
  return _invoice_load(state, params["key"].to_s) if event == "load"
  # Until `load`, the template shows the hook that sends it.
  return state.merge({"doc": nil, "missing": state["missing"] || false}) if state["doc"].nil?

  doc = state["doc"]
  changed = match event {
    "party" => _invoice_set_party(doc, params),
    "detail" => _invoice_set_detail(doc, params),
    "line" => _invoice_set_line(doc, params),
    "add_line" => _invoice_add_line(doc),
    "remove_line" => _invoice_remove_line(doc, params["index"].to_s.to_i),
    _ => false
  }
  return state unless changed

  doc["rev"] = (doc["rev"] || 0) + 1
  Invoice.find(state["key"]).update({
    "seller": doc["seller"], "buyer": doc["buyer"], "lines": doc["lines"],
    "number": doc["number"], "payment_terms": doc["payment_terms"], "rev": doc["rev"]
  })
  state.merge(_invoice_view(state["key"], doc))
end

def _invoice_load(state, key)
  return state if state["key"] == key && state["doc"].present?

  invoice = Invoice.find(key) rescue nil
  return state.merge({"missing": true}) if invoice.nil?

  _invoice_view(key, invoice.to_h)
end

# What the template reads: the document, the engine's totals, the revision
# the preview image is drawn for.
def _invoice_view(key, doc)
  data = pdf_invoice_data(Invoice.typed(doc))
  {
    "key": key, "doc": doc, "rev": doc["rev"],
    "items": data["items"], "total": data["total"], "missing": false,
    "vat_rates": Invoice::VAT_RATES,
    "party_fields": [["name", "Name"], ["address_line", "Address"], ["postcode", "Postcode"], ["city", "City"],
      ["country", "Country (ISO)"], ["vat_id", "VAT number"]]
  }
end

def _invoice_set_party(doc, params)
  side = params["side"] == "seller" ? "seller" : "buyer"
  field = params["name"].to_s
  limit = Invoice::PARTY_FIELDS[field]
  return false if limit.nil? || (side == "seller" && field == "vat_id")

  value = params["value"].to_s.trim
  value = value.upcase if field == "country"
  doc[side][field] = value.substring(0, limit)
  true
end

def _invoice_set_detail(doc, params)
  limits = {"number": 30, "payment_terms": 60}
  field = params["name"].to_s
  return false if limits[field].nil?

  doc[field] = params["value"].to_s.trim.substring(0, limits[field])
  true
end

def _invoice_set_line(doc, params)
  index = params["index"].to_s.to_i
  field = params["name"].to_s
  return false unless Invoice::LINE_FIELDS.includes?(field)
  return false if index < 0 || index >= doc["lines"].length

  value = params["value"].to_s.trim
  value = value.substring(0, 80) if field == "name"
  value = "20" if field == "vat_rate" && !Invoice::VAT_RATES.includes?(value)
  doc["lines"][index][field] = value
  true
end

def _invoice_add_line(doc)
  return false if doc["lines"].length >= Invoice::MAX_LINES

  doc["lines"].push({"name": "New line", "quantity": "1", "unit_price": "100", "vat_rate": "20"})
  true
end

def _invoice_remove_line(doc, index)
  return false if doc["lines"].length <= 1 || index < 0 || index >= doc["lines"].length

  doc["lines"] = doc["lines"].take(index) + doc["lines"].drop(index + 1)
  true
end
