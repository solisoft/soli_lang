# PDF builtins — pdf_render / pdf_response / pdf_facturx_from_invoice, the
# `pdfa` option and its incompatibilities, and the PNG/WebP page previews.
# Templates name the "titillium" font, found relative to the repo root: run
# this spec from there.

PDF_TEMPLATE = """{
  "fonts": ["titillium"],
  "content": [
    { "type": "paragraph", "value": "Invoice ${invoice.number}" }
  ]
}"""

TWO_PAGE_TEMPLATE = """{
  "fonts": ["titillium"],
  "content": [
    { "type": "paragraph", "value": "Page one" },
    { "type": "page_break" },
    { "type": "paragraph", "value": "Page two" }
  ]
}"""

PDF_DATA = """{ "data": { "invoice": { "number": "F-42" } } }"""

PDF_INVOICE = """{
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
}"""

# "JVBERi" is the base64 encoding of "%PDF-"; "iVBORw0KGgo" that of the PNG
# magic bytes.
PDF_MAGIC = "JVBERi"
PNG_MAGIC = "iVBORw0KGgo"

PREVIEW_DIR = "/tmp/soli-pdf-builtins-spec-previews"

describe("PDF builtins") do
  describe("pdf_render") do
    test("returns base64 PDF bytes") do
      pdf = pdf_render(PDF_TEMPLATE, PDF_DATA)
      assert_gt(pdf.length, 1000)
      assert(pdf.starts_with(PDF_MAGIC))
    end

    test("accepts the pdfa option") do
      assert(pdf_render(PDF_TEMPLATE, PDF_DATA, {"pdfa": true}).starts_with(PDF_MAGIC))
    end

    test("pdfa is incompatible with password protection") do
      assert_raises("`pdfa` is incompatible with password protection") do
        pdf_render(PDF_TEMPLATE, PDF_DATA, {"pdfa": true, "password": "x"})
      end
    end
  end

  describe("pdf_response") do
    test("wraps the PDF as a ready attachment response") do
      response = pdf_response(PDF_TEMPLATE, PDF_DATA, {"filename": "test.pdf"})
      assert_eq(response.keys, ["status", "headers", "body_base64"])
      assert_eq(response["status"], 200)
      assert_eq(response["headers"], {
        "Content-Type": "application/pdf",
        "Content-Disposition": "attachment; filename=\"test.pdf\""
      })
      assert(response["body_base64"].starts_with(PDF_MAGIC))
    end
  end

  describe("pdf_facturx_from_invoice") do
    test("renders an enriched invoice") do
      assert(pdf_facturx_from_invoice(PDF_TEMPLATE, PDF_INVOICE).starts_with(PDF_MAGIC))
    end

    test("rejects the pdfa option, which Factur-X already implies") do
      assert_raises("PDF/A is implied by Factur-X; drop the `pdfa` option") do
        pdf_facturx_from_invoice(PDF_TEMPLATE, PDF_INVOICE, {"pdfa": true})
      end
    end

    test("an allowance with neither amount nor percent is an invalid invoice") do
      bad_invoice = """{
        "number": "F-1", "issue_date": "2026-07-01", "currency": "EUR",
        "seller": { "name": "A", "country": "FR" },
        "buyer": { "name": "B", "country": "FR" },
        "lines": [ { "name": "W", "unit_price": 100 } ],
        "allowances": [ { "reason": "broken" } ]
      }"""
      assert_raises("allowances entry \"broken\" must set exactly one of \"amount\" or \"percent\"") do
        pdf_facturx_from_invoice(PDF_TEMPLATE, bad_invoice)
      end
    end
  end
end

describe("PDF page previews") do
  after_each() do
    System.run_sync(["rm", "-rf", PREVIEW_DIR])
  end

  describe("pdf_preview") do
    test("returns one base64 PNG per page") do
      pages = pdf_preview(PDF_TEMPLATE, PDF_DATA)
      assert_eq(pages.length, 1)
      assert(pages[0].starts_with(PNG_MAGIC))
    end

    test("every page of a multi-page document comes back") do
      pages = pdf_preview(TWO_PAGE_TEMPLATE, PDF_DATA)
      assert_eq(pages.length, 2)
      assert(pages[1].starts_with(PNG_MAGIC))
      assert_ne(pages[0], pages[1])
    end

    test("pages selects a subset, as a list or a range string") do
      assert_eq(pdf_preview(TWO_PAGE_TEMPLATE, PDF_DATA, {"pages": [2]}).length, 1)
      assert_eq(pdf_preview(TWO_PAGE_TEMPLATE, PDF_DATA, {"pages": "1-2"}).length, 2)
    end

    test("PDF-only options are ignored, not fatal — one hash drives both") do
      options = {"pdfa": true, "password": "x", "stationery": "does-not-exist.pdf"}
      pages = pdf_preview(PDF_TEMPLATE, PDF_DATA, options)
      assert(pages[0].starts_with(PNG_MAGIC))
    end

    test("markdown previews too") do
      pages = pdf_preview_from_markdown("# Title\n\nSome body text.")
      assert_eq(pages.length, 1)
      assert(pages[0].starts_with(PNG_MAGIC))
    end
  end

  describe("size") do
    test("the default dpi is 96, so an A4 page is 794x1123") do
      image = Image.from_buffer(pdf_preview(PDF_TEMPLATE, PDF_DATA)[0])
      assert_eq(image.width, 794)
      assert_eq(image.height, 1123)
    end

    test("dpi scales the output") do
      big = Image.from_buffer(pdf_preview(PDF_TEMPLATE, PDF_DATA, {"dpi": 192})[0])
      assert_eq(big.width, 1587)
    end

    test("width wins over dpi") do
      page = pdf_preview(PDF_TEMPLATE, PDF_DATA, {"dpi": 600, "width": 400})[0]
      assert_eq(Image.from_buffer(page).width, 400)
    end

    test("height alone drives the scale and keeps the aspect ratio") do
      image = Image.from_buffer(pdf_preview(PDF_TEMPLATE, PDF_DATA, {"height": 500})[0])
      assert_eq(image.height, 500)
      assert_eq(image.width, 354)
    end
  end

  describe("caps and refusals") do
    test("a dpi over the cap is refused rather than allocated") do
      assert_raises("`dpi` of 100000 exceeds the 600 cap") do
        pdf_preview(PDF_TEMPLATE, PDF_DATA, {"dpi": 100000})
      end
    end

    test("a width over the cap is refused") do
      assert_raises("`width` of 999999px exceeds the 8192px cap") do
        pdf_preview(PDF_TEMPLATE, PDF_DATA, {"width": 999999})
      end
    end

    test("a page range over the cap is refused") do
      assert_raises("`pages` selects 100000 pages, over the 64 cap") do
        pdf_preview(PDF_TEMPLATE, PDF_DATA, {"pages": "1-100000"})
      end
    end

    test("the page cap counts pages painted, not the document's length") do
      # A long document must still be previewable one page at a time — a
      # thumbnail of page 1 of a long report is the whole point.
      three_pages = """{
        "fonts": ["titillium"],
        "content": [
          { "type": "paragraph", "value": "a" }, { "type": "page_break" },
          { "type": "paragraph", "value": "b" }, { "type": "page_break" },
          { "type": "paragraph", "value": "c" }
        ]
      }"""
      assert_eq(pdf_preview(three_pages, PDF_DATA, {"width": 120, "pages": [1]}).length, 1)
    end

    test("asking for a page past the end is an error") do
      assert_raises("page 9 was requested, but the document has 1 page(s)") do
        pdf_preview(PDF_TEMPLATE, PDF_DATA, {"pages": [9]})
      end
    end

    test("an unknown format is refused") do
      assert_raises("unknown `format` \"gif\"") do
        pdf_preview(PDF_TEMPLATE, PDF_DATA, {"format": "gif"})
      end
    end

    test("a quality outside 1-100 is refused") do
      assert_raises("`quality` must be 1-100, got 0") do
        pdf_preview(PDF_TEMPLATE, PDF_DATA, {"quality": 0})
      end
    end
  end

  describe("formats") do
    test("webp is smaller than png, and both decode at the size asked for") do
      png = pdf_preview(PDF_TEMPLATE, PDF_DATA, {"width": 600})[0]
      webp = pdf_preview(PDF_TEMPLATE, PDF_DATA, {"width": 600, "format": "webp"})[0]
      assert_lt(webp.length, png.length)
      assert_eq(Image.from_buffer(png).width, 600)
      assert_eq(Image.from_buffer(webp).width, 600)
    end

    test("jpeg decodes at the size asked for") do
      jpeg = pdf_preview(PDF_TEMPLATE, PDF_DATA, {"width": 300, "format": "jpeg"})[0]
      assert_eq(Image.from_buffer(jpeg).width, 300)
    end

    test("quality trades size for fidelity") do
      high = pdf_preview(PDF_TEMPLATE, PDF_DATA, {"width": 600, "format": "webp", "quality": 95})[0]
      low = pdf_preview(PDF_TEMPLATE, PDF_DATA, {"width": 600, "format": "webp", "quality": 40})[0]
      assert_lt(low.length, high.length)
    end
  end

  describe("out_dir") do
    test("writes one file per page and returns their paths") do
      paths = pdf_preview(TWO_PAGE_TEMPLATE, PDF_DATA, {"width": 200, "out_dir": PREVIEW_DIR, "prefix": "doc"})
      assert_eq(paths, ["#{PREVIEW_DIR}/doc-1.png", "#{PREVIEW_DIR}/doc-2.png"])
      assert(File.exists("#{PREVIEW_DIR}/doc-2.png"))
      assert_eq(Image.new(paths[0]).width, 200)
    end

    test("a single page keeps the bare prefix as its name") do
      paths = pdf_preview(PDF_TEMPLATE, PDF_DATA, {"width": 120, "out_dir": PREVIEW_DIR, "prefix": "solo"})
      assert_eq(paths, ["#{PREVIEW_DIR}/solo.png"])
    end

    test("the written file takes the format's extension") do
      options = {"width": 200, "format": "webp", "out_dir": PREVIEW_DIR, "prefix": "shot"}
      paths = pdf_preview(PDF_TEMPLATE, PDF_DATA, options)
      assert_eq(paths, ["#{PREVIEW_DIR}/shot.webp"])
      assert(file_exists("#{PREVIEW_DIR}/shot.webp"))
    end

    test("a prefix cannot smuggle a path out of the directory") do
      assert_raises("`prefix` must be a plain file name without path separators, got \"../escape\"") do
        pdf_preview(PDF_TEMPLATE, PDF_DATA, {"out_dir": PREVIEW_DIR, "prefix": "../escape"})
      end
    end
  end

  describe("pdf_preview_response") do
    test("is a ready image/png response") do
      response = pdf_preview_response(PDF_TEMPLATE, PDF_DATA, {"width": 320})
      assert_eq(response["status"], 200)
      assert_eq(response["headers"]["Content-Type"], "image/png")
      assert(response["body_base64"].starts_with(PNG_MAGIC))
    end

    test("picks the page asked for") do
      first = pdf_preview_response(TWO_PAGE_TEMPLATE, PDF_DATA, {"page": 1})["body_base64"]
      second = pdf_preview_response(TWO_PAGE_TEMPLATE, PDF_DATA, {"page": 2})["body_base64"]
      assert_ne(first, second)
      assert_eq(second, pdf_preview(TWO_PAGE_TEMPLATE, PDF_DATA, {"pages": [2]})[0])
    end

    test("carries the content type of the format") do
      response = pdf_preview_response(PDF_TEMPLATE, PDF_DATA, {"format": "webp", "width": 320})
      assert_eq(response["headers"]["Content-Type"], "image/webp")
    end

    test("refuses `pages`: the response carries one image") do
      assert_raises("use `page` (a single page) rather than `pages`") do
        pdf_preview_response(PDF_TEMPLATE, PDF_DATA, {"pages": [1]})
      end
    end

    test("refuses `out_dir`: the response carries the image") do
      assert_raises("`out_dir` is meaningless here") do
        pdf_preview_response(PDF_TEMPLATE, PDF_DATA, {"out_dir": "tmp"})
      end
    end

    test("refuses a page past the end") do
      assert_raises("page 9 is past the end of the document (2 page(s))") do
        pdf_preview_response(TWO_PAGE_TEMPLATE, PDF_DATA, {"page": 9})
      end
    end
  end
end
