# ============================================================================
# Offline-capable builtins miscellany: the PDF toolkit (render/markdown/pages/
# merge/stamp/sign/verify/fill/facturx/attachments/layout_map), Image methods
# not covered by image_spec.sl, Geo math, and assorted class methods
# (Xml.get_element_by_id, JSON.parse_jsonp, Duration.humanize, Money.mul,
# Markdown.to_safe_html/to_spans, Array.compact_blank).
#
# Everything here runs fully offline: PDFs are rendered from templates,
# signatures use an embedded self-signed P-256 certificate (valid until 2126),
# and the fillable-form fixture is a hand-built minimal AcroForm PDF embedded
# as base64.
# ============================================================================

const SIGN_CERT = "-----BEGIN CERTIFICATE-----\nMIIBfjCCASWgAwIBAgIUV1BpK9LDesww0yOe2TPFPxwIbTUwCgYIKoZIzj0EAwIw\nFDESMBAGA1UEAwwJc29saS10ZXN0MCAXDTI2MDgyMzA4MDUzMFoYDzIxMjYwNzMw\nMDgwNTMwWjAUMRIwEAYDVQQDDAlzb2xpLXRlc3QwWTATBgcqhkjOPQIBBggqhkjO\nPQMBBwNCAATpVcfPwcKM6U0LPHCIQPr3jB9dQNtWGNpn+/kb1s+Zrm/S5tNzkgNQ\nZVKjI3mdX6+wKozcWKHa96di9ZcPaFgTo1MwUTAdBgNVHQ4EFgQU560h2h98rkH+\nwMHVyUhnHUfH/LswHwYDVR0jBBgwFoAU560h2h98rkH+wMHVyUhnHUfH/LswDwYD\nVR0TAQH/BAUwAwEB/zAKBggqhkjOPQQDAgNHADBEAiBzIzk8odmDf95j0V7VpM3k\nl/m6spsNjL+BaUWgMfykfwIgSqlDBCgFbDJZEri0hIenkDErIWZYoHJXWkMGgYUc\ncC4=\n-----END CERTIFICATE-----\n";
const SIGN_KEY = "-----BEGIN PRIVATE KEY-----\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgg9sXBVkpEjxF6s4C\n06eYwdZ5XwJh7T0UcJuFpNYWThWhRANCAATpVcfPwcKM6U0LPHCIQPr3jB9dQNtW\nGNpn+/kb1s+Zrm/S5tNzkgNQZVKjI3mdX6+wKozcWKHa96di9ZcPaFgT\n-----END PRIVATE KEY-----\n";

# A hand-built one-page PDF with a single AcroForm text field named
# "fullname" (see the header comment).
const FORM_PDF_B64 = "JVBERi0xLjQKMSAwIG9iago8PCAvVHlwZSAvQ2F0YWxvZyAvUGFnZXMgMiAwIFIgL0Fjcm9Gb3JtIDw8IC9GaWVsZHMgWzUgMCBSXSA+PiA+PgplbmRvYmoKMiAwIG9iago8PCAvVHlwZSAvUGFnZXMgL0tpZHMgWzMgMCBSXSAvQ291bnQgMSA+PgplbmRvYmoKMyAwIG9iago8PCAvVHlwZSAvUGFnZSAvUGFyZW50IDIgMCBSIC9NZWRpYUJveCBbMCAwIDIwMCAxMDBdIC9SZXNvdXJjZXMgPDwgL0ZvbnQgPDwgL0hlbHYgNiAwIFIgPj4gPj4gL0NvbnRlbnRzIDQgMCBSID4+CmVuZG9iago0IDAgb2JqCjw8IC9MZW5ndGggMzYgPj4Kc3RyZWFtCkJUIC9IZWx2IDEyIFRmIDIwIDgwIFRkIChGb3JtKSBUaiBFVAplbmRzdHJlYW0KZW5kb2JqCjUgMCBvYmoKPDwgL1R5cGUgL0Fubm90IC9TdWJ0eXBlIC9XaWRnZXQgL0ZUIC9UeCAvVCAoZnVsbG5hbWUpIC9SZWN0IFsxMCAxMCAxOTAgNDBdIC9QIDMgMCBSIC9EQSAoL0hlbHYgMTIgVGYgMCBnKSA+PgplbmRvYmoKNiAwIG9iago8PCAvVHlwZSAvRm9udCAvU3VidHlwZSAvVHlwZTEgL0Jhc2VGb250IC9IZWx2ZXRpY2EgPj4KZW5kb2JqCnhyZWYKMCA3CjAwMDAwMDAwMDAgNjU1MzUgZiAKMDAwMDAwMDAwOSAwMDAwMCBuIAowMDAwMDAwMDkwIDAwMDAwIG4gCjAwMDAwMDAxNDcgMDAwMDAgbiAKMDAwMDAwMDI3NSAwMDAwMCBuIAowMDAwMDAwMzYxIDAwMDAwIG4gCjAwMDAwMDA0ODYgMDAwMDAgbiAKdHJhaWxlcgo8PCAvU2l6ZSA3IC9Sb290IDEgMCBSID4+CnN0YXJ0eHJlZgo1NTYKJSVFT0YK";

# A generated 40x30 RGB PNG (the shared tests/fixtures/test.png is 1x1, too
# small for meaningful crop/rotate assertions).
const TEST_PNG_B64 = "iVBORw0KGgoAAAANSUhEUgAAACgAAAAeCAIAAADRv8uKAAAAVUlEQVR4nGNgYOMRkpBT0TIws3Hy8AuJSkjLKaqoa+maMG3OohXrtuw6cOzMpRv3nryiurpRi0ctHrV41OJRi0ctHrV41OJRi0ctHrV41OJRiwevxQDOb22rjHJEuQAAAABJRU5ErkJggg==";

describe("PDF toolkit: generation", fn() {
    test("pdf_from_markdown renders prose into a base64 PDF", fn() {
        let pdf = pdf_from_markdown("# Quarterly Report\n\nRevenue is **up**.\n\n- one\n- two");
        assert(pdf.starts_with("JVBERi"));
        assert(pdf.length() > 1000);
    });

    test("pdf_from_markdown accepts theme options", fn() {
        let pdf = pdf_from_markdown("# Tinted\n\nBody text.", {"headingColor": "1a5fb4"});
        assert(pdf.starts_with("JVBERi"));
    });

    test("pdf_layout_map reports where every element landed", fn() {
        let template = [[{"fonts": [], "content": [{"type": "paragraph", "value": "Hello #{name}"}]}]];
        let boxes = pdf_layout_map(template, "{\"name\": \"World\"}");
        assert(len(boxes) >= 1);
        let first = boxes[0];
        assert_eq(first["page"], 0);
        assert_eq(first["kind"], "paragraph");
        assert_eq(first["path"], "content.0");
        assert(first["w"] > 0);
        assert(first["h"] > 0);
    });
});

describe("PDF toolkit: page surgery", fn() {
    test("pdf_pages keeps a range selection", fn() {
        let pdf = pdf_from_markdown("# Solo page");
        let kept = pdf_pages(pdf, "1");
        assert(kept.starts_with("JVBERi"));
    });

    test("pdf_pages accepts an array of page numbers", fn() {
        let pdf = pdf_from_markdown("# Solo page");
        let kept = pdf_pages(pdf, [1]);
        assert(kept.starts_with("JVBERi"));
    });

    test("pdf_pages rejects out-of-range selections", fn() {
        let pdf = pdf_from_markdown("# Solo page");
        let result = pdf_pages(pdf, "99") rescue "REJECTED";
        assert_eq(result, "REJECTED");
    });

    test("pdf_merge concatenates documents", fn() {
        let a = pdf_from_markdown("# First");
        let b = pdf_from_markdown("# Second");
        let merged = pdf_merge([a, b]);
        assert(merged.starts_with("JVBERi"));
        assert(merged != a);
        assert(merged != b);
    });

    test("pdf_stamp draws a watermark onto every page", fn() {
        let pdf = pdf_from_markdown("# Confidential");
        let stamped = pdf_stamp(pdf, "DRAFT", {"opacity": 0.3, "rotation": 45});
        assert(stamped.starts_with("JVBERi"));
        assert(stamped != pdf);
    });
});

describe("PDF toolkit: digital signatures", fn() {
    test("pdf_sign embeds a verifiable PAdES signature", fn() {
        let pdf = pdf_from_markdown("# Contract\n\nSigned below.");
        let signed = pdf_sign(pdf, {
            "cert": SIGN_CERT,
            "key": SIGN_KEY,
            "reason": "spec",
            "location": "Paris"
        });
        assert(signed.starts_with("JVBERi"));

        let sigs = pdf_verify(signed);
        assert_eq(len(sigs), 1);
        assert_eq(sigs[0]["valid"], true);
        assert_eq(sigs[0]["covers_document"], true);
        assert_eq(sigs[0]["reason"], "spec");
    });

    test("pdf_verify finds no signatures in an unsigned PDF", fn() {
        let pdf = pdf_from_markdown("# Unsigned");
        assert_eq(len(pdf_verify(pdf)), 0);
    });

    test("pdf_sign rejects a missing key", fn() {
        let pdf = pdf_from_markdown "# No key";
        let result = pdf_sign(pdf, {"cert": SIGN_CERT}) rescue "REJECTED";
        assert_eq(result, "REJECTED");
    });
});

describe("PDF toolkit: forms and e-invoices", fn() {
    test("pdf_fill fills AcroForm fields by name", fn() {
        let filled = pdf_fill(FORM_PDF_B64, {"fullname": "Olivier Bonnaure"});
        assert(filled.starts_with("JVBERi"));
    });

    test("pdf_fill flattens when asked", fn() {
        let filled = pdf_fill(FORM_PDF_B64, {"fullname": "OB"}, {"flatten": true});
        assert(filled.starts_with("JVBERi"));
    });

    test("pdf_fill rejects a PDF without an AcroForm", fn() {
        let plain = pdf_from_markdown("# No fields");
        let result = pdf_fill(plain, {"fullname": "x"}) rescue "REJECTED";
        assert_eq(result, "REJECTED");
    });

    test("pdf_facturx embeds the invoice XML and extract reads it back", fn() {
        let cii = "<CrossIndustryInvoice><ExchangedDocument><ID>FX-1</ID></ExchangedDocument></CrossIndustryInvoice>";
        let template = "{\"fonts\": [], \"content\": [{\"type\": \"paragraph\", \"value\": \"Invoice FX-1\"}]}";
        let pdf = pdf_facturx(template, "{}", cii);
        assert(pdf.starts_with("JVBERi"));

        let extracted = pdf_extract_facturx(pdf);
        assert(extracted.contains("CrossIndustryInvoice"));
        assert(extracted.contains("FX-1"));
    });

    test("pdf_extract_facturx returns null without an e-invoice payload", fn() {
        let plain = pdf_from_markdown("# Plain");
        assert_null(pdf_extract_facturx(plain));
    });

    test("pdf_attachments lists nothing on a plain document", fn() {
        let plain = pdf_from_markdown("# Plain");
        assert_eq(len(pdf_attachments(plain)), 0);
    });

    test("pdf_attachments round-trips an embedded file", fn() {
        File.write("tests/fixtures/_offline_misc_att.txt", "attachment payload");
        let template = "{\"fonts\": [], \"content\": [{\"type\": \"paragraph\", \"value\": \"With attachment\"}]}";
        let pdf = pdf_render(template, "{}", {
            "attachments": [{"path": "tests/fixtures/_offline_misc_att.txt", "name": "note.txt"}]
        });
        let atts = pdf_attachments(pdf);
        assert_eq(len(atts), 1);
        assert_eq(atts[0]["name"], "note.txt");
        assert_eq(atts[0]["size"], 18);
        assert_eq(Base64.decode(atts[0]["base64"]), "attachment payload");
        File.delete("tests/fixtures/_offline_misc_att.txt");
    });
});

describe("Image: methods beyond image_spec", fn() {
    test("from_buffer decodes a base64 buffer round-trip", fn() {
        let img = Image.from_buffer(TEST_PNG_B64);
        assert_eq(img.width, 40);
        assert_eq(img.height, 30);
        let copy = Image.from_buffer(img.to_buffer());
        assert(copy.width == img.width);
        assert(copy.height == img.height);
    });

    test("crop extracts the requested rectangle", fn() {
        let img = Image.from_buffer(TEST_PNG_B64);
        let cropped = img.crop(0, 0, 10, 20);
        assert_eq(cropped.width, 10);
        assert_eq(cropped.height, 20);
    });

    test("hue_rotate shifts colors without changing dimensions", fn() {
        let img = Image.from_buffer(TEST_PNG_B64);
        let shifted = img.hue_rotate(90);
        assert(shifted.width == img.width);
        assert(shifted.height == img.height);
    });

    test("quality adjusts JPEG encoding quality", fn() {
        let img = Image.from_buffer(TEST_PNG_B64).quality(40);
        assert(img.to_buffer().length() > 0);
    });

    test("rotate180 preserves dimensions", fn() {
        let img = Image.from_buffer(TEST_PNG_B64);
        let rotated = img.rotate180();
        assert(rotated.width == 40);
        assert(rotated.height == 30);
    });

    test("rotate270 swaps width and height", fn() {
        let img = Image.from_buffer(TEST_PNG_B64);
        let rotated = img.rotate270();
        assert(rotated.width == 30);
        assert(rotated.height == 40);
    });

    test("to_file writes the encoded image to disk", fn() {
        let out = "tests/fixtures/_offline_misc_out.png";
        let ok = Image.from_buffer(TEST_PNG_B64).grayscale().to_file(out);
        assert_eq(ok, true);
        assert(File.exists(out));
        let reloaded = Image.new(out);
        assert(reloaded.width > 0);
        File.delete(out);
    });
});

describe("Geo static methods", fn() {
    test("distance matches a known great-circle pair", fn() {
        let metres = Geo.distance(48.8566, 2.3522, 51.5074, -0.1278);
        assert((metres - 343500.0).abs() < 2000.0);
        assert_eq(Geo.distance(48.8566, 2.3522, 48.8566, 2.3522), 0);
    });

    test("bearing is clockwise from north", fn() {
        assert((Geo.bearing(0.0, 0.0, 1.0, 0.0)).abs() < 0.001);
        assert((Geo.bearing(0.0, 0.0, 0.0, 1.0) - 90.0).abs() < 0.001);
        assert((Geo.bearing(0.0, 0.0, -1.0, 0.0) - 180.0).abs() < 0.001);
        assert((Geo.bearing(0.0, 0.0, 0.0, -1.0) - 270.0).abs() < 0.001);
    });

    test("bounding_box encloses its radius on every axis", fn() {
        let box_hash = Geo.bounding_box(48.8566, 2.3522, 5000);
        let lat_5km = 5000.0 / 111320.0;
        assert(box_hash["max_lat"] >= 48.8566 + lat_5km);
        assert(box_hash["min_lat"] <= 48.8566 - lat_5km);
        assert(box_hash["min_lng"] < box_hash["max_lng"]);
        assert(box_hash["min_lat"] < box_hash["max_lat"]);
    });

    test("geohash matches the reference encoding", fn() {
        assert_eq(Geo.geohash(57.64911, 10.40744, 11), "u4pruydqqvj");
        assert_eq(Geo.geohash(48.8566, 2.3522).length(), 9);
    });

    test("geohash_decode recovers the cell centre within its error bars", fn() {
        let decoded = Geo.geohash_decode("u4pruydqqvj");
        assert((decoded["lat"] - 57.64911).abs() <= decoded["lat_error"]);
        assert((decoded["lng"] - 10.40744).abs() <= decoded["lng_error"]);
    });
});

describe("Xml.get_element_by_id", fn() {
    test("extracts a standalone fragment by ID attribute", fn() {
        let xml = [[<envelope><signed ID="payload"><value>hi</value></signed></envelope>]];
        let fragment = Xml.get_element_by_id(xml, "payload");
        assert(fragment.contains("<value>hi</value>"));
        assert(fragment.contains("ID=\"payload\""));
    });

    test("raises when no element carries the id", fn() {
        let result = Xml.get_element_by_id("<root/>", "nope") rescue "MISSING";
        assert_eq(result, "MISSING");
    });
});

describe("JSON.parse_jsonp", fn() {
    test("unwraps a callback-padded payload", fn() {
        let parsed = JSON.parse_jsonp("angular.callbacks._0({\"a\": 1, \"b\": [true]});");
        assert_eq(parsed["a"], 1);
        assert_eq(parsed["b"][0], true);
    });

    test("tolerates the /**/ sniffing guard and whitespace", fn() {
        let parsed = JSON.parse_jsonp("/**/cb( {\"ok\": true} )");
        assert_eq(parsed["ok"], true);
    });

    test("rejects input without a call wrapper", fn() {
        let result = JSON.parse_jsonp("{\"plain\": 1}") rescue "BAD";
        assert_eq(result, "BAD");
    });
});

describe("Duration.humanize", fn() {
    test("humanizes whole units", fn() {
        assert(Duration.of_seconds(7200).humanize().contains("2 hours"));
        assert(Duration.of_minutes(90).humanize().contains("1 hour"));
        assert(Duration.of_seconds(45).humanize().contains("45 seconds"));
    });

    test("combines primary and secondary units", fn() {
        let spoken = Duration.of_seconds(3661).humanize();
        assert(spoken.contains("1 hour"));
        assert(spoken.contains("minute"));
    });

    test("honors an explicit locale argument", fn() {
        let spoken = Duration.of_seconds(120).humanize("fr");
        assert(spoken.contains("2"));
    });
});

describe("Money.mul", fn() {
    test("scales a money value by an integer factor exactly", fn() {
        let total = Money.mul(Money.new("49.99", "EUR"), 3);
        assert_eq(Money.format(total, {"symbol": false}), "149.97 EUR");
    });

    test("keeps the currency of the operand", fn() {
        let total = Money.mul(Money.new(10, "JPY"), 5);
        assert_eq(Money.compare(total, Money.new(50, "JPY")), 0);
    });

    test("rejects a money factor", fn() {
        let result = Money.mul(Money.new(10, "EUR"), Money.new(2, "EUR")) rescue "REJECTED";
        assert_eq(result, "REJECTED");
    });
});

describe("Markdown.to_safe_html and Markdown.to_spans", fn() {
    test("to_safe_html escapes raw HTML while keeping markdown formatting", fn() {
        let html = Markdown.to_safe_html("Hello <script>alert(1)</script>");
        assert(!html.contains("<script>"));
        assert(html.contains("&lt;script&gt;"));
    });

    test("to_safe_html neutralizes javascript: links", fn() {
        let html = Markdown.to_safe_html("[click](javascript:alert(1))");
        assert(!html.contains("javascript:"));
    });

    test("to_spans maps inline markdown onto PDF span hashes", fn() {
        let spans = Markdown.to_spans("plain **bold** *italic* `code`");
        assert_eq(len(spans), 6);
        assert_eq(spans[0]["text"], "plain ");
        assert_eq(spans[1]["text"], "bold");
        assert_eq(spans[1]["fontWeight"], "bold");
        assert_eq(spans[1]["italic"], null);
        assert_eq(spans[3]["italic"], true);
        assert_eq(spans[5]["mono"], true);
        assert_eq(spans[5]["text"], "code");
    });

    test("to_spans attaches link targets", fn() {
        let spans = Markdown.to_spans("[docs](https://example.com)");
        assert_eq(len(spans), 1);
        assert_eq(spans[0]["link"], "https://example.com");
        assert_eq(spans[0]["text"], "docs");
    });
});

describe("Array.compact_blank", fn() {
    test("drops nils, empty strings, arrays and hashes", fn() {
        assert_eq([1, null, "", [], {}, "x"].compact_blank(), [1, "x"]);
    });

    test("keeps falsy-but-not-blank values like 0 and false", fn() {
        assert_eq([0, false, null].compact_blank(), [0, false]);
    });

    test("returns an empty array when everything is blank", fn() {
        assert_eq(["", [], {}].compact_blank(), []);
    });
});
