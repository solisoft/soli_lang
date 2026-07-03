# ============================================================================
# PDF builtins — pdf_render / pdf_response / pdf_facturx_from_invoice, the
# `pdfa` option and its incompatibilities. Fixtures live at module level: the
# test runner executes each test closure outside the describe body's scope.
# ============================================================================

let pdf_template = [[{
  "fonts": ["titillium"],
  "content": [
    { "type": "paragraph", "value": "Invoice ${invoice.number}" }
  ]
}]];

let pdf_data = [[{ "data": { "invoice": { "number": "F-42" } } }]];

let pdf_invoice = [[{
  "number": "F-42",
  "issue_date": "2026-07-01",
  "due_date": "2026-08-01",
  "currency": "EUR",
  "seller": { "name": "ACME", "address_line": "1 rue de la Paix", "postcode": "75000",
              "city": "Paris", "country": "FR", "vat_id": "FRXX999999999" },
  "buyer": { "name": "Bob", "address_line": "2 avenue du Test", "postcode": "69000",
             "city": "Lyon", "country": "FR" },
  "lines": [ { "name": "Widget", "quantity": 2, "unit_price": 100, "vat_rate": 20.0 } ],
  "allowances": [ { "reason": "Volume discount", "percent": 10, "vat_rate": 20.0 } ],
  "charges": [ { "reason": "Shipping", "amount": "20.00", "vat_rate": 20.0 } ],
  "payment_terms": "30 days net"
}]];

describe("PDF builtins", fn() {
    test("pdf_render returns base64 PDF bytes", fn() {
        let pdf = pdf_render(pdf_template, pdf_data);
        assert(pdf.length() > 1000);
        # "JVBERi" is the base64 encoding of "%PDF-".
        assert(pdf.starts_with("JVBERi"));
    });

    test("pdf_render accepts the pdfa option", fn() {
        let pdf = pdf_render(pdf_template, pdf_data, {"pdfa": true});
        assert(pdf.starts_with("JVBERi"));
    });

    test("pdfa is incompatible with password protection", fn() {
        let result = pdf_render(pdf_template, pdf_data, {"pdfa": true, "password": "x"}) rescue "REJECTED";
        assert_eq(result, "REJECTED");
    });

    test("pdf_response wraps the PDF as a ready response", fn() {
        let response = pdf_response(pdf_template, pdf_data, {"filename": "test.pdf"});
        assert_eq(response["status"], 200);
        assert_eq(response["headers"]["Content-Type"], "application/pdf");
        assert(response["headers"]["Content-Disposition"].contains("test.pdf"));
        assert(response["body_base64"].starts_with("JVBERi"));
    });

    test("pdf_facturx_from_invoice renders an enriched invoice", fn() {
        let pdf = pdf_facturx_from_invoice(pdf_template, pdf_invoice);
        assert(pdf.starts_with("JVBERi"));
    });

    test("pdf_facturx_from_invoice rejects the pdfa option", fn() {
        let result = pdf_facturx_from_invoice(pdf_template, pdf_invoice, {"pdfa": true}) rescue "REJECTED";
        assert_eq(result, "REJECTED");
    });

    test("an invalid invoice is rejected", fn() {
        let bad = [[{
          "number": "F-1", "issue_date": "2026-07-01", "currency": "EUR",
          "seller": { "name": "A", "country": "FR" },
          "buyer": { "name": "B", "country": "FR" },
          "lines": [ { "name": "W", "unit_price": 100 } ],
          "allowances": [ { "reason": "broken" } ]
        }]];
        let result = pdf_facturx_from_invoice(pdf_template, bad) rescue "REJECTED";
        assert_eq(result, "REJECTED");
    });
});
