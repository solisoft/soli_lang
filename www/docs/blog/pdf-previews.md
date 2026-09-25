# One PDF Template, Three Outputs: the Document, Its Thumbnails, and Its Preview

Soli has rendered PDFs in-process for a while: a JSON layout template, a JSON
data document, `pdf_render`, and out comes a base64 PDF. No headless browser, no
wkhtmltopdf, no Node. That covers the moment a customer clicks **Download**.

It doesn't cover the other places a document shows up. The invoice list wants a
thumbnail per row. The "review before sending" screen wants the pages inline,
not a PDF viewer in an iframe that renders differently on every browser and not
at all in some mail clients. The email that says "your invoice is attached"
would like a picture of it alongside the attachment. Each of those wants an **image** of the
document, and until now the only way to get one was to render the PDF and shell
out to poppler's `pdftoppm` to rasterise it. Soli's own documentation gallery
did exactly that.

Three builtins replace that step:

```soli
pdf_preview(template, data, options?)          # one image per page
pdf_preview_from_markdown(markdown, options?)  # the Markdown counterpart
pdf_preview_response(template, data, options?) # one page as a ready HTTP reply
```

They take the same template, the same data and the same options hash as
`pdf_render`. This post is about why the preview is trustworthy, which knobs it
has, what the image format choice is worth in bytes, and a controller that
serves the document, its thumbnail and a review page from one template.

<figure style="margin:1.5rem auto;max-width:1024px;">
  <img src="/images/blog/pdf-previews.svg" width="1024" height="576" alt="An invoice template and its data are laid out once into a LaidOutDoc. The PDF backend turns it into a .pdf for pdf_render and pdf_response; the raster backend paints it into PNG, WebP or JPEG images for pdf_preview, pdf_preview_from_markdown and pdf_preview_response. Below, bars compare page 1 of the invoice sample: 165 KB as a 150 dpi PNG against 51 KB as WebP q90, and 25 KB as a 320 px PNG thumbnail against 7.7 KB as WebP." style="display:block;width:100%;height:auto;border-radius:12px;border:1px solid #30363d;background:#0b0d0f;">
  <figcaption style="text-align:center;color:#8b949e;font-size:0.875rem;margin-top:0.5rem;">The preview is not a reading of the PDF — it is the same laid-out page, painted into pixels instead of PDF operators.</figcaption>
</figure>

## The design problem: a preview that can lie

The obvious way to preview a PDF is to render the PDF and then rasterise it with
a PDF reader. That works, and it has two costs. The first is operational: a
native dependency on every machine that renders previews, and a subprocess per
request. The second is subtler. The preview is now a *second program's reading*
of your output. When it disagrees with the viewer your customer uses, which one
is the document?

The other obvious way — render the same template to HTML and screenshot it — is
worse: now there are two layout engines, and a "check before you send" screen
that shows a line break the PDF doesn't have is a screen that trains people to
stop checking.

Soli's PDF engine was already split at the right seam. The layout pass produces
a backend-neutral draw model, a `LaidOutDoc`, and `pdf_backend.rs` described
itself as "the ONLY module that imports printpdf". So the preview is a second
backend beside it: `pdf/src/raster_backend.rs` paints that same `LaidOutDoc`
into an RGBA pixmap with tiny-skia, and SVG artwork goes through resvg. Text
advances come from the same `FontRegistry::char_advance` the layout measured
with, not from the font's own metrics table as a PDF viewer would read them —
deliberately, so the preview reproduces the exact line the document was laid
out against. What you see is where the text will be.

The module header is explicit about precedence: the PDF backend is normative,
and "when the two disagree the PDF wins, and this module is the one with the
bug."

Nothing new had to be compiled for this. `tiny-skia` and `resvg` were already in
the dependency graph under `svg2pdf`, whose `text` feature pulls them in; the
commit names them directly so the raster backend no longer hangs off someone
else's feature flag. And since the gallery script no longer needs `pdftoppm`,
`pdfinfo` or a separate cargo build, `scripts/gen_pdf_previews.sl` is now a Soli
script whose only requirement is `soli`.

## What a preview covers, and what it can't

Because the preview paints the draw model rather than PDF bytes, it covers what
the engine renders — `pdf_render` and `pdf_from_markdown` input. It does **not**
cover anything that exists only as bytes: a merged, filled or stamped PDF, the
output of `pdf_pages`, or an uploaded file. Rasterising those would need a PDF
interpreter. Preview the template before those steps.

Some render options have no raster meaning. Rather than rejecting them — which
would stop you from sharing one options hash between the PDF and its preview —
they are accepted and ignored:

| Option | In the preview |
|---|---|
| `stationery` | The letterhead is composited onto emitted PDF bytes, so it is absent. **Warns.** |
| `sign` | The signature is applied to emitted bytes; its visible appearance is missing. **Warns.** |
| `attachments`, `password`, `pdfa` | Cannot change a pixel. Silent. |

Only the two that change what you would see say anything, and each says so
**once per process**. A preview endpoint runs on every request; a warning per
thumbnail would be a log, not a warning. I ran a shared hash through
`pdf_preview` twice and got exactly one line:

```
[WARN] pdf_preview(): ignoring `stationery` — a letterhead is composited onto emitted PDF bytes, so the preview shows the content without it
```

Two smaller divergences are documented rather than fixed: colour/bitmap glyph
fonts (CBDT/sbix/COLR) are not drawn, and hairlines thinner than one device pixel
are anti-aliased rather than snapped, so very thin table rules look slightly
different at low `dpi`.

## The knobs

All three functions read the same preview options:

| Key | Default | Meaning |
|---|---|---|
| `dpi` | `96` | Scale is `dpi / 72`. A4 comes out 794×1123 at 96, 1240×1754 at 150. |
| `width` / `height` | — | Pixels. Either one overrides `dpi`; both together fit the page *inside* the box, aspect ratio kept. |
| `pages` | all | 1-based, the same selection `pdf_pages` takes: `[1, 3]` or `"1-3,7"`. |
| `format` | `"png"` | `"png"`, `"webp"` or `"jpeg"`. |
| `quality` | `90` | 1–100, for `webp` and `jpeg`. Ignored by lossless PNG. |
| `out_dir` / `prefix` | — / `"page"` | Write files and return their paths instead of base64 strings. |
| `page` | `1` | `pdf_preview_response` only — the page the response carries. |

A few behaviours worth knowing, all of which I checked against the build:

- `{"width": 320, "height": 320}` on an A4 invoice gives a **226×320** image —
  the box is a bound, not a stretch.
- Asking for a page past the end is an error, not a short array:
  `page 2 was requested, but the document has 1 page(s)`.
- With `out_dir`, one page keeps the bare prefix (`invoice-42.webp`), several get
  a page suffix (`terms-1.webp`, `terms-2.webp`). The extension follows
  `format`, and JPEG is written as `.jpg`. `prefix` must be a plain file name:
  no separators, no leading dot.
- `pdf_preview_response` refuses `pages` and `out_dir` — "the response carries
  one image" — rather than silently picking one.

### Every allocation knob is capped

`dpi` and `width` usually arrive in a query string, and a raster is
`width × height × 4` bytes. The source puts it plainly: an A4 at 600 dpi is
already a 140 MB pixmap; at 20 000 dpi it is hundreds of gigabytes. So each knob
has a ceiling, overridable by environment variable:

| Variable | Default |
|---|---|
| `SOLI_PDF_PREVIEW_MAX_DPI` | 600 |
| `SOLI_PDF_PREVIEW_MAX_DIMENSION_PX` | 8192 per axis |
| `SOLI_PDF_PREVIEW_MAX_PAGES` | 64 |
| `SOLI_PDF_PREVIEW_MAX_PIXELS` | 40 000 000 per page |

Exceeding one is an error that names the variable —
``pdf_preview(): `dpi` of 1200 exceeds the 600 cap (raise SOLI_PDF_PREVIEW_MAX_DPI)``.
The page cap counts pages actually painted, not the document's length: a
thumbnail of page 1 of a 200-page report is exactly what this is for. Pages are
laid out once and then painted and encoded one at a time, so a long document
never holds every pixmap at once.

## PNG, WebP, JPEG: what the choice is worth

A document page is flat colour and crisp type — the content PNG compresses
worst. The changelog measures the `invoice` sample (at 150 dpi, 165 KB as PNG and
51 KB as WebP q90; a 320px thumbnail 25 KB against 7.7 KB). I re-ran it against
the sample in `www/public/pdf-samples/`, page 1, and got the same numbers:

| Page 1 of `invoice` | Size | PNG | WebP q90 | WebP q80 | JPEG q90 |
|---|---|---|---|---|---|
| `dpi: 150` | 1240×1754 | 165 KB | **51 KB** | 41 KB | 159 KB |
| `dpi: 96` | 794×1123 | 93 KB | **28 KB** | — | — |
| `width: 320` | 320×453 | 25 KB | **7.7 KB** | — | — |

(The JPEG figure is my own measurement, not the changelog's.) At quality 90 the
type stays crisp — the changelog reports no visible ringing on body text at
150 dpi. JPEG is there for completeness: it has no alpha, so the page is
flattened onto the paper colour, and on this content it saves almost nothing
over PNG.

Why the WebP numbers are this good is an implementation detail that matters. The
`image` crate's own WebP encoder is lossless-only and would hand most of the
saving back. Previews go through libwebp instead — the same path
`Image.format("webp")` takes — and the raster session returns raw straight-alpha
RGBA, so the host encodes exactly once instead of decoding a PNG it just wrote.

PNG stays the default anyway: lossless and universal is the safer thing to
default to, and the docs gallery is committed as PNG. If a preview is going over
the network, `{"format": "webp", "quality": 90}` is very likely what you want.

## Tutorial: an invoice, its thumbnail, and a review page

Here is a controller that serves all three outputs from one template. Assume an
`Invoice` model with `number`, `customer_name`, `issued_on`, `lines` and
`total` fields, the template at `pdf/invoice.template.json`, and fonts in the
app's `font/` directory — the default `font_dirs`, resolved against the app root.

```soli
# config/routes.sl
get("/invoices/:id", "invoices#show")
get("/invoices/:id/pdf", "invoices#download")
get("/invoices/:id/preview", "invoices#preview")
get("/invoices/:id/thumbnail", "invoices#thumbnail")
```

The controller keeps the template and the data document in two private helpers
(methods starting with `_` are not routed), so every output is guaranteed to
come from the same inputs:

```soli
# app/controllers/invoices_controller.sl
class InvoicesController < Controller
  # The document itself.
  def download
    invoice = Invoice.find(params["id"])
    pdf_response(this._template(), this._document(invoice),
      {"filename": "invoice-#{invoice.number}.pdf"})
  end

  # One page as an image, for a preview pane.
  def preview
    invoice = Invoice.find(params["id"])
    response = pdf_preview_response(this._template(), this._document(invoice),
      {"width": 480, "format": "webp", "quality": 90})
    response["headers"]["Cache-Control"] = "private, max-age=300"
    response
  end

  # Every page inline, before the invoice goes out.
  def show
    invoice = Invoice.find(params["id"])
    pages = pdf_preview(this._template(), this._document(invoice),
      {"width": 800, "format": "webp"})
    render("invoices/show", {"invoice": invoice, "pages": pages})
  end

  def _template
    slurp("pdf/invoice.template.json")
  end

  def _document(invoice)
    json_stringify({
      "data": {
        "invoice": {"number": invoice.number, "issued": invoice.issued_on},
        "customer": {"name": invoice.customer_name},
        "items": invoice.lines,
        "totals": {"due": invoice.total}
      }
    })
  end
end
```

`pdf_preview_response` returns a plain hash — `status`, `headers` with a
`Content-Type` that follows `format` (`image/webp` here), and `body_base64`,
which the server decodes into the binary body. Because it is a hash, adding a
`Cache-Control` header is one assignment.

The review page doesn't need a second route at all. `pdf_preview` returns one
base64 string per page, which is already what a `data:` URI wants:

```erb
<%# app/views/invoices/show.html.slv %>
<h1>Invoice <%= invoice.number %></h1>

<% for page in pages %>
  <img src="data:image/webp;base64,<%= page %>" width="800" alt="Invoice page">
<% end %>

<a href="/invoices/<%= invoice.id %>/pdf">Download PDF</a>
```

What the reviewer approves is the laid-out document, not an HTML approximation
of it. At 800px WebP the sample invoice's page 1 is about 28 KB, so inlining is
reasonable for a page or two; for a long document, select what you
show with `pages` or link to the preview route instead.

### Caching the thumbnail

A thumbnail in a list is requested far more often than the invoice changes, so
it is worth rendering once. The inputs to a preview are the template and the
data — hash them and you get a cache key that invalidates itself:

```soli
  def thumbnail
    invoice = Invoice.find(params["id"])
    template = this._template()
    document = this._document(invoice)
    digest = sha256(template + document)
    file_name = "invoice-#{digest}"

    unless file_exists("public/previews/#{file_name}.webp")
      pdf_preview(template, document, {
        "pages": [1], "width": 320, "format": "webp",
        "out_dir": "public/previews", "prefix": file_name
      })
    end
    redirect("/previews/#{file_name}.webp")
  end
```

A cache hit costs a JSON build and a SHA-256 — no layout, no painting. Edit the
invoice or the template and the digest changes, so the next request renders a
fresh file; there is no invalidation step to forget. `out_dir` is resolved
through the same jail as `file_write_base64`, and `pages: [1]` means a single
file named exactly `<prefix>.webp`.

Two honest caveats. Files in `public/` are served to anyone holding the URL; a
64-hex-character digest is not guessable, but it is a capability URL, not
access control — for documents that must stay behind a login, keep the
images out of `public/` and answer them from an action that checks the user
first. And nothing deletes stale
files; a periodic sweep of `public/previews` is yours to add.

### Previews in email

The same pair works for the "your invoice is ready" mail. `pdf_render` and
`pdf_preview` both return base64, which is exactly what a mailer's
`attach_base64` takes, so the PDF and a picture of its first page can travel
together. With `template` and `document` built by the helpers above, and an
`InvoiceMailer` of your own:

```soli
first_page = pdf_preview(template, document,
  {"pages": [1], "width": 600, "format": "png"})[0]

InvoiceMailer.issued(invoice)
  .attach_base64("invoice-#{invoice.number}.pdf",
    pdf_render(template, document), "application/pdf")
  .attach_base64("invoice-#{invoice.number}.png", first_page, "image/png")
  .deliver_later
```

PNG rather than WebP here, because mail clients are a less forgiving audience
than browsers.

## Markdown gets it too

`pdf_preview_from_markdown` mirrors `pdf_from_markdown`: the same Markdown
renderer builds the template, and the raster backend paints it. (As with any
`pages` selection, `"1-2"` on a one-page document is an error.)

```soli
terms = slurp("docs/terms.md")
first_pages = pdf_preview_from_markdown(terms,
  {"pages": "1-2", "width": 600, "format": "webp"})
```

That is how the documentation gallery now previews its `markdown` sample —
`scripts/gen_pdf_previews.sl` loops over the samples, calls `pdf_preview` or
`pdf_preview_from_markdown` at 150 dpi with `pages: [1]`, and writes straight
into `www/public/images/docs/pdf/`.

## Where this leaves you

One template now answers three questions: *what is the document* (`pdf_render`,
`pdf_response`), *what does it look like at a glance* (`pdf_preview` at a
thumbnail width, WebP, cached by content digest), and *is this what I'm about to
send* (every page inline, painted from the same layout). The preview can't
drift from the PDF, because there is only one layout to drift from.

Full reference: [PDF & Factur-X Generation → Page previews](/docs/builtins/pdf#page-previews).
