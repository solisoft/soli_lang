# The layout every invoice of /invoices is drawn with, as the JSON
# `pdf_facturx_from_invoice` and `pdf_preview` take. It reads the names the
# typed-invoice route fills — `invoice.*`, `company.*` (the seller),
# `customer.*`, `items[]`, `total.*`, `payment.*` — so the preview
# (`pdf_invoice_data`) and the Factur-X PDF print the same figures.
class InvoiceDocument
  const INK = "1B3A6B"
  const ACCENT = "F6821F"
  const RULE = "E3E8EF"
  const NO_BORDER = {"top": "false", "bottom": "false", "left": "false", "right": "false"}
  const BOTTOM_RULE = {"top": "false", "bottom": "true", "left": "false", "right": "false"}

  static def template
    {
      "fonts": ["titillium"],
      "options": {"header_height": 0},
      "content": [
        InvoiceDocument._masthead,
        {"type": "move", "y": 10},
        {"type": "hr", "color": INK, "thickness": 1.6},
        {"type": "move", "y": 16},
        InvoiceDocument._parties,
        {"type": "move", "y": 14},
        InvoiceDocument._dates,
        {"type": "move", "y": 16},
        InvoiceDocument._lines,
        {"type": "move", "y": 12},
        InvoiceDocument._totals,
        {"type": "move", "y": 18},
        InvoiceDocument._payment,
        {"type": "paragraph", "value": "${infos.text}",
          "options": {"fontSize": 9, "color": "475569", "lineHeight": 1.4}}
      ],
      "footer": [
        {"type": "hr", "color": RULE},
        {"type": "paragraph",
          "value": "${company.name} · VAT ${company.vat_number} · Factur-X EN 16931 · page #PAGE# of #TOTAL_PAGE#",
          "options": {"fontSize": 7.5, "color": "6B7280", "alignment": "center"}}
      ]
    }
  end

  static def _cell(lines, width, alignment = "left")
    {"content": lines, "width": width, "alignment": alignment, "valign": "top", "borderSides": NO_BORDER}
  end

  static def _text(value, size = 9, weight = "normal")
    {"type": "text", "value": value, "fontSize": size, "fontWeight": weight}
  end

  static def _masthead
    {
      "type": "table", "header_columns": [], "options": {"padding_x": 0, "padding_y": 0},
      "rows": [[
        InvoiceDocument._cell([
          InvoiceDocument._text("${company.name}", 18, "bold"),
          InvoiceDocument._text("${company.address} · ${company.zipcode} ${company.city}", 9)
        ], 300),
        InvoiceDocument._cell([
          InvoiceDocument._text("${invoice.type_label}", 22, "bold"),
          InvoiceDocument._text("${invoice.number}", 11)
        ], 181, "right")
      ]]
    }
  end

  static def _parties
    {
      "type": "table", "header_columns": [], "options": {"padding_x": 0, "padding_y": 0},
      "rows": [[
        InvoiceDocument._cell([
          InvoiceDocument._text("FROM", 7.5, "bold"),
          InvoiceDocument._text("${company.name}", 10.5, "bold"),
          InvoiceDocument._text("${company.address}"),
          InvoiceDocument._text("${company.zipcode} ${company.city}, ${company.country}"),
          InvoiceDocument._text("VAT ${company.vat_number}")
        ], 240),
        InvoiceDocument._cell([
          InvoiceDocument._text("BILL TO", 7.5, "bold"),
          InvoiceDocument._text("${customer.company}", 10.5, "bold"),
          InvoiceDocument._text("${customer.address}"),
          InvoiceDocument._text("${customer.zipcode} ${customer.city}, ${customer.country}"),
          InvoiceDocument._text("${customer.vat_number}")
        ], 241)
      ]]
    }
  end

  static def _dates
    heading = fn(label, width) { {"text": label, "width": width, "fontSize": 7.5, "fontWeight": "bold",
      "alignment": "left", "borderSides": NO_BORDER} }
    value = fn(text, width, weight) { {"text": text, "width": width, "fontSize": 10, "fontWeight": weight,
      "alignment": "left", "borderSides": NO_BORDER} }
    {
      "type": "table", "options": {"padding_x": 0, "padding_y": 2},
      "header_columns": [heading("ISSUE DATE", 120), heading("DUE DATE", 120), heading("TERMS", 241)],
      "rows": [[
        value("${invoice.created_at}", 120, "normal"),
        value("${invoice.due_date}", 120, "bold"),
        value("${invoice.payment_terms}", 241, "normal")
      ]]
    }
  end

  static def _lines
    heading = fn(label, width, alignment) { {"text": label, "width": width, "fontSize": 8, "fontWeight": "bold",
      "alignment": alignment, "borderSides": BOTTOM_RULE, "borderColor": INK} }
    cell = fn(text, width, alignment) { {"text": text, "width": width, "fontSize": 10, "alignment": alignment,
      "borderSides": BOTTOM_RULE, "borderColor": RULE} }
    {
      "type": "table", "data": "items", "options": {"padding_x": 6, "padding_y": 7},
      "header_columns": [
        heading("DESCRIPTION", 241, "left"), heading("QTY", 50, "center"),
        heading("UNIT PRICE", 90, "right"), heading("AMOUNT", 100, "right")
      ],
      "rows": [[
        cell("${name}", 241, "left"), cell("${qty}", 50, "center"),
        cell("${amount}", 90, "right"), cell("${total_amount}", 100, "right")
      ]]
    }
  end

  static def _totals
    row = fn(label, value, size, weight, top) {
      sides = {"top": top, "bottom": "false", "left": "false", "right": "false"}
      [
        {"text": "", "width": 241, "borderSides": NO_BORDER},
        {"text": label, "width": 140, "fontSize": size, "fontWeight": weight, "alignment": "right",
          "borderSides": sides, "borderColor": INK},
        {"text": value, "width": 100, "fontSize": size, "fontWeight": weight, "alignment": "right",
          "borderSides": sides, "borderColor": INK}
      ]
    }
    {
      "type": "table", "header_columns": [], "options": {"padding_x": 6, "padding_y": 5},
      "rows": [
        row("Total excl. VAT", "${total.taxable}", 10, "normal", "false"),
        row("VAT", "${total.vat}", 10, "normal", "false"),
        row("Amount due", "${total.due_amount}", 14, "bold", "true")
      ]
    }
  end

  static def _payment
    {"type": "box", "fill": "F7F9FC", "border": "D7E0EC", "borderWidth": 0.8, "radius": 4, "padding": 14,
      "gap": 14, "content": [
        {"type": "paragraph", "value": "Pay by bank transfer",
          "options": {"fontSize": 11, "fontWeight": "bold", "color": INK}},
        {"type": "paragraph", "value": "IBAN ${payment.iban} · BIC ${payment.bic} · reference ${payment.remittance}",
          "options": {"fontSize": 9, "spacing": 6}},
        {"type": "qr", "kind": "epc", "name": "${payment.name}", "iban": "${payment.iban}", "bic": "${payment.bic}",
          "amount": "${payment.amount}", "currency": "${payment.currency}",
          "remittance": "${payment.remittance}", "width": 86},
        {"type": "move", "y": 92},
        {"type": "paragraph", "value": "Scan with a European banking app: the transfer is filled in.",
          "options": {"fontSize": 8, "color": "6B7280"}}
      ]}
  end
end
