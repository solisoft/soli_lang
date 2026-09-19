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

# ============================================================================
# PNG page previews. `iVBORw0KGgo` is the base64 encoding of the PNG magic
# bytes, the way "JVBERi" stands in for "%PDF-" above.
# ============================================================================

let pdf_template_2p = [[{
  "fonts": ["titillium"],
  "content": [
    { "type": "paragraph", "value": "Page one" },
    { "type": "page_break" },
    { "type": "paragraph", "value": "Page two" }
  ]
}]];

describe("PDF page previews", fn() {
    test("pdf_preview returns one base64 PNG per page", fn() {
        let pages = pdf_preview(pdf_template, pdf_data);
        assert_eq(pages.length(), 1);
        assert(pages[0].starts_with("iVBORw0KGgo"));
    });

    test("every page of a multi-page document comes back", fn() {
        let pages = pdf_preview(pdf_template_2p, pdf_data);
        assert_eq(pages.length(), 2);
        assert(pages[1].starts_with("iVBORw0KGgo"));
    });

    test("pages selects a subset, as a list or a range string", fn() {
        assert_eq(pdf_preview(pdf_template_2p, pdf_data, {"pages": [2]}).length(), 1);
        assert_eq(pdf_preview(pdf_template_2p, pdf_data, {"pages": "1-2"}).length(), 2);
    });

    test("the default dpi is 96, so A4 is about 794px wide", fn() {
        let page = pdf_preview(pdf_template, pdf_data)[0];
        let width = Image.from_buffer(page).width;
        assert(width > 780 && width < 810);
    });

    test("dpi scales the output", fn() {
        let small = Image.from_buffer(pdf_preview(pdf_template, pdf_data, {"dpi": 96})[0]).width;
        let big = Image.from_buffer(pdf_preview(pdf_template, pdf_data, {"dpi": 192})[0]).width;
        assert(big > small * 19 / 10);
    });

    test("width wins over dpi", fn() {
        let page = pdf_preview(pdf_template, pdf_data, {"dpi": 600, "width": 400})[0];
        assert_eq(Image.from_buffer(page).width, 400);
    });

    test("height alone drives the scale", fn() {
        let page = pdf_preview(pdf_template, pdf_data, {"height": 500})[0];
        assert_eq(Image.from_buffer(page).height, 500);
    });

    test("markdown previews too", fn() {
        let pages = pdf_preview_from_markdown("# Title\n\nSome body text.");
        assert(pages.length() >= 1);
        assert(pages[0].starts_with("iVBORw0KGgo"));
    });

    test("pdf_preview_response is a ready image/png response", fn() {
        let res = pdf_preview_response(pdf_template, pdf_data, {"width": 320});
        assert_eq(res["status"], 200);
        assert_eq(res["headers"]["Content-Type"], "image/png");
        assert(res["body_base64"].starts_with("iVBORw0KGgo"));
    });

    test("pdf_preview_response picks the page asked for", fn() {
        let one = pdf_preview_response(pdf_template_2p, pdf_data, {"page": 1})["body_base64"];
        let two = pdf_preview_response(pdf_template_2p, pdf_data, {"page": 2})["body_base64"];
        assert(one != two);
    });

    test("pdf_preview_response refuses options that cannot mean anything", fn() {
        let a = pdf_preview_response(pdf_template, pdf_data, {"pages": [1]}) rescue "REJECTED";
        assert_eq(a, "REJECTED");
        let b = pdf_preview_response(pdf_template, pdf_data, {"out_dir": "tmp"}) rescue "REJECTED";
        assert_eq(b, "REJECTED");
        let c = pdf_preview_response(pdf_template_2p, pdf_data, {"page": 9}) rescue "REJECTED";
        assert_eq(c, "REJECTED");
    });

    test("out_dir writes files and returns their paths", fn() {
        let paths = pdf_preview(pdf_template_2p, pdf_data,
            {"width": 200, "out_dir": "tmp/preview-spec", "prefix": "doc"});
        assert_eq(paths.length(), 2);
        assert(paths[0].ends_with("doc-1.png"));
        assert(File.exists("tmp/preview-spec/doc-1.png"));
        assert_eq(Image.new(paths[0]).width, 200);
    });

    test("a single page keeps the bare prefix as its name", fn() {
        let paths = pdf_preview(pdf_template, pdf_data,
            {"width": 120, "out_dir": "tmp/preview-spec", "prefix": "solo"});
        assert_eq(paths.length(), 1);
        assert(paths[0].ends_with("solo.png"));
    });

    test("a prefix cannot smuggle a path out of the directory", fn() {
        let result = pdf_preview(pdf_template, pdf_data,
            {"out_dir": "tmp/preview-spec", "prefix": "../escape"}) rescue "REJECTED";
        assert_eq(result, "REJECTED");
    });

    test("the size knobs are capped rather than allocated", fn() {
        assert_eq(pdf_preview(pdf_template, pdf_data, {"dpi": 100000}) rescue "REJECTED", "REJECTED");
        assert_eq(pdf_preview(pdf_template, pdf_data, {"width": 999999}) rescue "REJECTED", "REJECTED");
        assert_eq(pdf_preview(pdf_template, pdf_data, {"pages": "1-100000"}) rescue "REJECTED", "REJECTED");
    });

    test("the page cap counts pages painted, not the document's length", fn() {
        # A long document must still be previewable one page at a time —
        # a thumbnail of page 1 of a long report is the whole point.
        let many = [[{
          "fonts": ["titillium"],
          "content": [
            { "type": "paragraph", "value": "a" }, { "type": "page_break" },
            { "type": "paragraph", "value": "b" }, { "type": "page_break" },
            { "type": "paragraph", "value": "c" }
          ]
        }]];
        let one = pdf_preview(many, pdf_data, {"width": 120, "pages": [1]});
        assert_eq(one.length(), 1);
    });

    test("asking for a page past the end is an error", fn() {
        let result = pdf_preview(pdf_template, pdf_data, {"pages": [9]}) rescue "REJECTED";
        assert_eq(result, "REJECTED");
    });

    test("format webp is much smaller than png, and both decode", fn() {
        let png  = pdf_preview(pdf_template, pdf_data, {"width": 600})[0];
        let webp = pdf_preview(pdf_template, pdf_data, {"width": 600, "format": "webp"})[0];
        assert(webp.length() < png.length());
        # Both must be real images of the size asked for.
        assert_eq(Image.from_buffer(png).width, 600);
        assert_eq(Image.from_buffer(webp).width, 600);
    });

    test("quality trades size for fidelity", fn() {
        let high = pdf_preview(pdf_template, pdf_data, {"width": 600, "format": "webp", "quality": 95})[0];
        let low  = pdf_preview(pdf_template, pdf_data, {"width": 600, "format": "webp", "quality": 40})[0];
        assert(low.length() < high.length());
    });

    test("an unknown format is refused, and so is a silly quality", fn() {
        assert_eq(pdf_preview(pdf_template, pdf_data, {"format": "gif"}) rescue "REJECTED", "REJECTED");
        assert_eq(pdf_preview(pdf_template, pdf_data, {"quality": 0}) rescue "REJECTED", "REJECTED");
    });

    test("the written file takes the format's extension", fn() {
        let paths = pdf_preview(pdf_template, pdf_data,
            {"width": 200, "format": "webp", "out_dir": "tmp/preview-spec", "prefix": "shot"});
        assert(paths[0].ends_with("shot.webp"));
        assert(file_exists("tmp/preview-spec/shot.webp"));
    });

    test("pdf_preview_response carries the matching content type", fn() {
        let res = pdf_preview_response(pdf_template, pdf_data, {"format": "webp", "width": 320});
        assert_eq(res["headers"]["Content-Type"], "image/webp");
    });

    test("PDF-only options are ignored, not fatal — one hash drives both", fn() {
        let pages = pdf_preview(pdf_template, pdf_data,
            {"pdfa": true, "password": "x", "stationery": "does-not-exist.pdf"});
        assert(pages[0].starts_with("iVBORw0KGgo"));
    });
});
