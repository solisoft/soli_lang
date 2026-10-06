# Offline builtins with no spec of their own: the PDF toolkit (markdown,
# layout_map, pages, merge, stamp, sign/verify, fill, Factur-X, attachments),
# Geo math, Xml.get_element_by_id, JSON.parse_jsonp, json_script,
# Duration.humanize, Money.mul and Array.compact_blank.
#
# Signatures use an embedded self-signed P-256 certificate (valid until 2126).

const SIGN_CERT = "-----BEGIN CERTIFICATE-----\n" +
  "MIIBfjCCASWgAwIBAgIUV1BpK9LDesww0yOe2TPFPxwIbTUwCgYIKoZIzj0EAwIw\n" +
  "FDESMBAGA1UEAwwJc29saS10ZXN0MCAXDTI2MDgyMzA4MDUzMFoYDzIxMjYwNzMw\n" +
  "MDgwNTMwWjAUMRIwEAYDVQQDDAlzb2xpLXRlc3QwWTATBgcqhkjOPQIBBggqhkjO\n" +
  "PQMBBwNCAATpVcfPwcKM6U0LPHCIQPr3jB9dQNtWGNpn+/kb1s+Zrm/S5tNzkgNQ\n" +
  "ZVKjI3mdX6+wKozcWKHa96di9ZcPaFgTo1MwUTAdBgNVHQ4EFgQU560h2h98rkH+\n" +
  "wMHVyUhnHUfH/LswHwYDVR0jBBgwFoAU560h2h98rkH+wMHVyUhnHUfH/LswDwYD\n" +
  "VR0TAQH/BAUwAwEB/zAKBggqhkjOPQQDAgNHADBEAiBzIzk8odmDf95j0V7VpM3k\n" +
  "l/m6spsNjL+BaUWgMfykfwIgSqlDBCgFbDJZEri0hIenkDErIWZYoHJXWkMGgYUc\n" +
  "cC4=\n" +
  "-----END CERTIFICATE-----\n"
const SIGN_KEY = "-----BEGIN PRIVATE KEY-----\n" +
  "MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgg9sXBVkpEjxF6s4C\n" +
  "06eYwdZ5XwJh7T0UcJuFpNYWThWhRANCAATpVcfPwcKM6U0LPHCIQPr3jB9dQNtW\n" +
  "GNpn+/kb1s+Zrm/S5tNzkgNQZVKjI3mdX6+wKozcWKHa96di9ZcPaFgT\n" +
  "-----END PRIVATE KEY-----\n"

# A hand-built one-page PDF with a single AcroForm text field named "fullname".
const FORM_PDF_B64 = "JVBERi0xLjQKMSAwIG9iago8PCAvVHlwZSAvQ2F0YWxvZyAvUGFnZXMgMiAwIFIgL0Fjcm9Gb3JtIDw8IC9G" +
  "aWVsZHMgWzUgMCBSXSA+PiA+PgplbmRvYmoKMiAwIG9iago8PCAvVHlwZSAvUGFnZXMgL0tpZHMgWzMgMCBS" +
  "XSAvQ291bnQgMSA+PgplbmRvYmoKMyAwIG9iago8PCAvVHlwZSAvUGFnZSAvUGFyZW50IDIgMCBSIC9NZWRp" +
  "YUJveCBbMCAwIDIwMCAxMDBdIC9SZXNvdXJjZXMgPDwgL0ZvbnQgPDwgL0hlbHYgNiAwIFIgPj4gPj4gL0Nv" +
  "bnRlbnRzIDQgMCBSID4+CmVuZG9iago0IDAgb2JqCjw8IC9MZW5ndGggMzYgPj4Kc3RyZWFtCkJUIC9IZWx2" +
  "IDEyIFRmIDIwIDgwIFRkIChGb3JtKSBUaiBFVAplbmRzdHJlYW0KZW5kb2JqCjUgMCBvYmoKPDwgL1R5cGUg" +
  "L0Fubm90IC9TdWJ0eXBlIC9XaWRnZXQgL0ZUIC9UeCAvVCAoZnVsbG5hbWUpIC9SZWN0IFsxMCAxMCAxOTAg" +
  "NDBdIC9QIDMgMCBSIC9EQSAoL0hlbHYgMTIgVGYgMCBnKSA+PgplbmRvYmoKNiAwIG9iago8PCAvVHlwZSAv" +
  "Rm9udCAvU3VidHlwZSAvVHlwZTEgL0Jhc2VGb250IC9IZWx2ZXRpY2EgPj4KZW5kb2JqCnhyZWYKMCA3CjAw" +
  "MDAwMDAwMDAgNjU1MzUgZiAKMDAwMDAwMDAwOSAwMDAwMCBuIAowMDAwMDAwMDkwIDAwMDAwIG4gCjAwMDAw" +
  "MDAxNDcgMDAwMDAgbiAKMDAwMDAwMDI3NSAwMDAwMCBuIAowMDAwMDAwMzYxIDAwMDAwIG4gCjAwMDAwMDA0" +
  "ODYgMDAwMDAgbiAKdHJhaWxlcgo8PCAvU2l6ZSA3IC9Sb290IDEgMCBSID4+CnN0YXJ0eHJlZgo1NTYKJSVF" +
  "T0YK"

const ATTACHMENT_PATH = "tests/fixtures/_offline_misc_att.txt"
# Triple quotes keep #{name} for the PDF engine to fill in.
const PARAGRAPH_TEMPLATE = """{"fonts": [], "content": [{"type": "paragraph", "value": "Hello #{name}"}]}"""

# pdf_pages raises on a page the document does not have.
def page_count(pdf)
  (1..21).filter { |page| (pdf_pages(pdf, [page]) rescue nil).present? }.length
end

def signed_pdf
  pdf_sign(pdf_from_markdown("# Contract\n\nSigned below."), {
    "cert": SIGN_CERT,
    "key": SIGN_KEY,
    "reason": "spec",
    "location": "Paris"
  })
end

describe("PDF toolkit") do
  describe("generation") do
    test("pdf_from_markdown renders prose into a one-page base64 PDF") do
      pdf = pdf_from_markdown("# Quarterly Report\n\nRevenue is **up**.\n\n- one\n- two")
      assert(pdf.starts_with("JVBERi"), "base64 of %PDF-")
      assert_gt(pdf.length, 1000)
      assert_eq(page_count(pdf), 1)
    end

    test("pdf_from_markdown takes theme options") do
      plain = pdf_from_markdown("# Tinted\n\nBody text.")
      tinted = pdf_from_markdown("# Tinted\n\nBody text.", {"headingColor": "1a5fb4"})
      assert(tinted.starts_with("JVBERi"))
      assert_ne(tinted, plain)
    end

    test("pdf_layout_map reports where every element landed") do
      boxes = pdf_layout_map(PARAGRAPH_TEMPLATE, "{\"name\": \"World\"}")
      assert_eq(boxes.length, 1)
      box = boxes[0]
      assert_eq(box["page"], 0)
      assert_eq(box["kind"], "paragraph")
      assert_eq(box["path"], "content.0")
      assert_gt(box["x"], 0)
      assert_gt(box["w"], 400)
      assert_gt(box["h"], 10)
    end
  end

  describe("page surgery") do
    test("pdf_pages keeps a range selection") do
      merged = pdf_merge([pdf_from_markdown("# One"), pdf_from_markdown("# Two")])
      assert_eq(page_count(pdf_pages(merged, "1-2")), 2)
      assert_eq(page_count(pdf_pages(merged, "2")), 1)
    end

    test("pdf_pages accepts an array of page numbers") do
      merged = pdf_merge([pdf_from_markdown("# One"), pdf_from_markdown("# Two")])
      assert_eq(page_count(pdf_pages(merged, [2])), 1)
    end

    test("pdf_pages rejects out-of-range selections") do
      assert_raises("pages: no valid page numbers selected") do
        pdf_pages(pdf_from_markdown("# Solo page"), "99")
      end
    end

    test("pdf_merge concatenates the pages of every document") do
      first = pdf_from_markdown("# First")
      merged = pdf_merge([first, pdf_from_markdown("# Second")])
      assert_eq(page_count(merged), 2)
      assert_eq(page_count(pdf_merge([merged, first])), 3)
    end

    test("pdf_merge refuses an empty list and a non-array") do
      assert_raises("merge: no PDFs given") do
        pdf_merge([])
      end
      assert_raises("pdf_merge() expects an array of PDFs (paths or base64)") do
        pdf_merge("not a list")
      end
    end

    test("pdf_stamp draws a watermark and keeps the pages") do
      pdf = pdf_from_markdown("# Confidential")
      stamped = pdf_stamp(pdf, "DRAFT", {"opacity": 0.3, "rotation": 45})
      assert(stamped.starts_with("JVBERi"))
      assert_ne(stamped, pdf)
      assert_eq(page_count(stamped), 1)
    end
  end

  describe("digital signatures") do
    test("pdf_sign embeds a verifiable PAdES signature") do
      signatures = pdf_verify(signed_pdf())
      assert_eq(signatures.length, 1)
      signature = signatures[0]
      assert_eq(signature["field"], "Signature1")
      assert_eq(signature["valid"], true)
      assert_eq(signature["covers_document"], true)
      assert_eq(signature["signer"], "soli-test")
      assert_eq(signature["reason"], "spec")
    end

    test("changing a signed PDF invalidates the signature") do
      signatures = pdf_verify(pdf_stamp(signed_pdf(), "TAMPERED", {}))
      assert_eq(signatures.length, 1)
      assert_eq(signatures[0]["valid"], false)
      assert_eq(signatures[0]["covers_document"], false)
    end

    test("pdf_verify finds no signatures in an unsigned PDF") do
      assert_eq(pdf_verify(pdf_from_markdown("# Unsigned")), [])
    end

    test("pdf_sign rejects a missing key") do
      assert_raises("sign: `key` (PEM string or path) is required") do
        pdf_sign(pdf_from_markdown("# No key"), {"cert": SIGN_CERT})
      end
    end
  end

  describe("forms and e-invoices") do
    after_each() do
      File.delete(ATTACHMENT_PATH) if File.exists(ATTACHMENT_PATH)
    end

    test("pdf_fill fills AcroForm fields by name") do
      filled = pdf_fill(FORM_PDF_B64, {"fullname": "Olivier Bonnaure"})
      assert(filled.starts_with("JVBERi"))
      assert_ne(filled, FORM_PDF_B64)
      assert_eq(page_count(filled), 1)
    end

    test("pdf_fill flattens when asked") do
      filled = pdf_fill(FORM_PDF_B64, {"fullname": "OB"})
      flattened = pdf_fill(FORM_PDF_B64, {"fullname": "OB"}, {"flatten": true})
      assert(flattened.starts_with("JVBERi"))
      assert_ne(flattened, filled)
    end

    test("pdf_fill rejects a PDF without an AcroForm") do
      assert_raises("the PDF has no AcroForm (no fillable fields)") do
        pdf_fill(pdf_from_markdown("# No fields"), {"fullname": "x"})
      end
    end

    test("pdf_facturx embeds the invoice XML and pdf_extract_facturx reads it back") do
      cii = "<CrossIndustryInvoice><ExchangedDocument><ID>FX-1</ID></ExchangedDocument></CrossIndustryInvoice>"
      template = "{\"fonts\": [], \"content\": [{\"type\": \"paragraph\", \"value\": \"Invoice FX-1\"}]}"
      pdf = pdf_facturx(template, "{}", cii)
      assert(pdf.starts_with("JVBERi"))
      assert_contains(pdf_extract_facturx(pdf), cii)
    end

    test("pdf_extract_facturx returns nil without an e-invoice payload") do
      assert_null(pdf_extract_facturx(pdf_from_markdown("# Plain")))
    end

    test("pdf_attachments lists nothing on a plain document") do
      assert_eq(pdf_attachments(pdf_from_markdown("# Plain")), [])
    end

    test("pdf_attachments round-trips an embedded file") do
      File.write(ATTACHMENT_PATH, "attachment payload")
      template = "{\"fonts\": [], \"content\": [{\"type\": \"paragraph\", \"value\": \"With attachment\"}]}"
      pdf = pdf_render(template, "{}", {"attachments": [{"path": ATTACHMENT_PATH, "name": "note.txt"}]})
      attachments = pdf_attachments(pdf)
      assert_eq(attachments.length, 1)
      assert_eq(attachments[0]["name"], "note.txt")
      assert_eq(attachments[0]["size"], 18)
      assert_eq(Base64.decode(attachments[0]["base64"]), "attachment payload")
    end
  end
end

describe("Geo static methods") do
  test("distance matches a known great-circle pair, in metres") do
    paris_to_london = Geo.distance(48.8566, 2.3522, 51.5074, -0.1278)
    assert_lt((paris_to_london - 343556.5).abs, 1.0)
  end

  test("distance from a point to itself is zero") do
    assert_eq(Geo.distance(48.8566, 2.3522, 48.8566, 2.3522), 0)
  end

  test("bearing is clockwise from north") do
    assert_lt(Geo.bearing(0.0, 0.0, 1.0, 0.0).abs, 0.001)
    assert_lt((Geo.bearing(0.0, 0.0, 0.0, 1.0) - 90.0).abs, 0.001)
    assert_lt((Geo.bearing(0.0, 0.0, -1.0, 0.0) - 180.0).abs, 0.001)
    assert_lt((Geo.bearing(0.0, 0.0, 0.0, -1.0) - 270.0).abs, 0.001)
  end

  test("bounding_box encloses its radius on every axis") do
    box = Geo.bounding_box(48.8566, 2.3522, 5000)
    lat_5km = 5000.0 / 111320.0
    assert_eq(box.keys.sort, ["max_lat", "max_lng", "min_lat", "min_lng"])
    assert(box["max_lat"] >= 48.8566 + lat_5km)
    assert(box["min_lat"] <= 48.8566 - lat_5km)
    assert_lt(box["min_lng"], 2.3522)
    assert_gt(box["max_lng"], 2.3522)
  end

  test("geohash matches the reference encoding") do
    assert_eq(Geo.geohash(57.64911, 10.40744, 11), "u4pruydqqvj")
  end

  test("geohash defaults to 9 characters") do
    assert_eq(Geo.geohash(48.8566, 2.3522), "u09tvw0f6")
  end

  test("geohash_decode recovers the cell centre within its error bars") do
    decoded = Geo.geohash_decode("u4pruydqqvj")
    assert(decoded["lat_error"] < 0.00001)
    assert((decoded["lat"] - 57.64911).abs <= decoded["lat_error"])
    assert((decoded["lng"] - 10.40744).abs <= decoded["lng_error"])
  end
end

describe("Xml.get_element_by_id") do
  test("extracts a standalone fragment by ID attribute") do
    xml = "<envelope><signed ID=\"payload\"><value>hi</value></signed></envelope>"
    assert_eq(Xml.get_element_by_id(xml, "payload"), "<signed ID=\"payload\"><value>hi</value></signed>")
  end

  test("raises when no element carries the id") do
    assert_raises("Xml.get_element_by_id(): no element with Id 'nope' found") do
      Xml.get_element_by_id("<root/>", "nope")
    end
  end
end

describe("JSON.parse_jsonp") do
  test("unwraps a callback-padded payload") do
    assert_eq(JSON.parse_jsonp("angular.callbacks._0({\"a\": 1, \"b\": [true]});"), {"a": 1, "b": [true]})
  end

  test("tolerates the /**/ sniffing guard and whitespace") do
    assert_eq(JSON.parse_jsonp("/**/cb( {\"ok\": true} )"), {"ok": true})
  end

  test("rejects input without a call wrapper") do
    assert_raises("not a JSONP response: no '(' found") do
      JSON.parse_jsonp("{\"plain\": 1}")
    end
  end

  test("rejects a wrapped payload that is not JSON") do
    assert_raises("Expected string key") do
      JSON.parse_jsonp("cb({bad json})")
    end
  end
end

describe("json_script") do
  test("serializes a value as JSON safe to inline in a <script>") do
    assert_eq(json_script({"a": "</script><b>"}), "{\"a\":\"\\u003c/script\\u003e\\u003cb\\u003e\"}")
  end

  test("escapes HTML comment openers and ampersands") do
    assert_eq(json_script([1, "<!--"]), "[1,\"\\u003c!--\"]")
    assert_eq(json_script("a&b"), "\"a\\u0026b\"")
  end

  test("round-trips through JSON.parse") do
    data = {"html": "<b>&</b>", "n": 1}
    assert_eq(JSON.parse(json_script(data)), data)
  end
end

describe("Duration.humanize") do
  # humanize takes an optional locale, so it is called with parentheses: see
  # the pending test at the end of this describe.
  test("names a whole unit") do
    assert_eq(Duration.of_seconds(7200).humanize(), "2 hours")
    assert_eq(Duration.of_seconds(45).humanize(), "45 seconds")
    assert_eq(Duration.of_seconds(86400 * 3).humanize(), "3 days")
  end

  test("uses the singular for one") do
    assert_eq(Duration.of_seconds(1).humanize(), "1 second")
    assert_eq(Duration.of_seconds(0).humanize(), "0 seconds")
  end

  test("combines a primary and a secondary unit") do
    assert_eq(Duration.of_minutes(90).humanize(), "1 hour 30 minutes")
    assert_eq(Duration.of_seconds(3661).humanize(), "1 hour 1 minute")
  end

  test("names the magnitude of a negative duration") do
    assert_eq(Duration.of_seconds(-90).humanize(), "1 minute 30 seconds")
  end

  test("falls back to English for a locale without translations") do
    assert_eq(Duration.of_seconds(120).humanize("fr"), "2 minutes")
  end

  test("refuses a locale that is not a string") do
    assert_raises("Duration.humanize() locale must be a string") do
      Duration.of_seconds(5).humanize(3)
    end
  end

  test("is called without parentheses like any zero-argument method") do
    pending("bug: a native method with optional arguments is not called without parens (.humanize gives a Function)")
    assert_eq(Duration.of_seconds(7200).humanize, "2 hours")
  end
end

describe("Money.mul") do
  test("scales a money value by an integer factor exactly") do
    total = Money.mul(Money.new("49.99", "EUR"), 3)
    assert_eq(Money.format(total, {"symbol": false}), "149.97 EUR")
  end

  test("has no binary-float drift") do
    total = Money.mul(Money.new("0.10", "EUR"), 3)
    assert_eq(Money.format(total, {"symbol": false}), "0.30 EUR")
  end

  test("scales by a float factor") do
    total = Money.mul(Money.new("10.00", "EUR"), 1.5)
    assert_eq(Money.format(total, {"symbol": false}), "15.00 EUR")
  end

  test("keeps the currency of the operand") do
    total = Money.mul(Money.new(10, "JPY"), 5)
    assert_eq(Money.compare(total, Money.new(50, "JPY")), 0)
    assert_eq(Money.format(total), "¥50")
  end

  test("rejects a money factor") do
    assert_raises("Money.mul() factor expects Int/Float, got hash") do
      Money.mul(Money.new(10, "EUR"), Money.new(2, "EUR"))
    end
  end
end

describe("Array.compact_blank") do
  test("drops nils, empty strings, arrays and hashes") do
    assert_eq([1, nil, "", [], {}, "x"].compact_blank, [1, "x"])
  end

  test("keeps falsy-but-not-blank values like 0 and false") do
    assert_eq([0, false, nil].compact_blank, [0, false])
  end

  test("returns an empty array when everything is blank") do
    assert_eq(["", [], {}].compact_blank, [])
  end

  test("drops whitespace-only strings, which .blank? calls blank") do
    pending("bug: compact_blank keeps \"  \" although \"  \".blank? is true")
    assert_eq(["  ", "x"].compact_blank, ["x"])
  end
end
